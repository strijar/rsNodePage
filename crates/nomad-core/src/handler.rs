//! Wires a [`PageIndex`] into the `RequestOutcome` closure that
//! `LinkManager::set_request_handler_ex` expects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use rns_runtime::link_manager::RequestOutcome;

use crate::pages::{Content, PageIndex, read_content};
use crate::script;

/// Below this size a response is sent as a single reply packet; at or above
/// it we hand it to the resource transfer path so it gets chunked. Matches
/// `rns_wire::constants::ENCRYPTED_MDU` (431 bytes) with a small safety
/// margin for the request/response msgpack envelope itself.
const INLINE_REPLY_LIMIT: usize = 380;

/// Builds the closure passed to `LinkManager::set_request_handler_ex`.
///
/// `link_identities` is the map `LinkManager::link_identities_handle()`
/// returns — it's populated once a client identifies on the link, which is
/// how we know *who* is asking, for the optional allow-list (and for the
/// `remote_identity` variable exposed to `.rhai` pages).
pub fn build_request_handler(
    index: Arc<RwLock<PageIndex>>,
    allowed_identities: Vec<[u8; 16]>,
    link_identities: Arc<Mutex<HashMap<[u8; 16], [u8; 16]>>>,
) -> impl Fn([u8; 16], [u8; 16], Vec<u8>) -> RequestOutcome + Send + 'static {
    let engine = Arc::new(script::build_engine());

    move |link_id, path_hash, data| {
        let remote_identity = link_identities
            .lock()
            .ok()
            .and_then(|ids| ids.get(&link_id).copied());

        if !allowed_identities.is_empty() {
            match remote_identity {
                Some(id) if allowed_identities.contains(&id) => {}
                _ => {
                    tracing::warn!(
                        link_id = hex::encode(link_id),
                        "nomad: denying request from unidentified/unlisted identity"
                    );
                    return RequestOutcome::Drop;
                }
            }
        }

        let entry = {
            let guard = match index.read() {
                Ok(g) => g,
                Err(_) => return RequestOutcome::Drop,
            };
            guard.get(&path_hash).cloned()
        };

        let Some(entry) = entry else {
            tracing::warn!(path_hash = hex::encode(path_hash), "nomad: unknown path");
            return RequestOutcome::Drop;
        };

        let bytes = if let Content::Script(script_path) = &entry.content {
            let remote_identity_hex = remote_identity.map(hex::encode);
            match script::run(&engine, script_path, &data, remote_identity_hex.as_deref()) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(path = %entry.request_path, error = %e, "nomad: script error");
                    script::error_page(&entry.request_path, &e)
                }
            }
        } else {
            match read_content(&entry.content) {
                Some(bytes) => bytes,
                None => {
                    tracing::warn!(path = %entry.request_path, "nomad: failed to read content");
                    return RequestOutcome::Drop;
                }
            }
        };

        tracing::info!(path = %entry.request_path, bytes = bytes.len(), "nomad: serving request");

        if bytes.len() < INLINE_REPLY_LIMIT {
            RequestOutcome::Reply(bytes)
        } else {
            let metadata = entry.is_file.then(|| {
                let name = entry
                    .request_path
                    .rsplit('/')
                    .next()
                    .unwrap_or("file")
                    .to_string();
                let mut buf = Vec::new();
                let value =
                    rmpv::Value::Map(vec![(rmpv::Value::from("name"), rmpv::Value::from(name))]);
                let _ = rmpv::encode::write_value(&mut buf, &value);
                buf
            });
            RequestOutcome::ReplyWithResource {
                ack: Vec::new(),
                data: bytes,
                metadata,
                auto_compress: true,
            }
        }
    }
}
