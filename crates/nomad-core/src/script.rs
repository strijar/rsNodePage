//! Runs `.rhai` scripts as dynamic NomadNet pages.
//!
//! The script is evaluated fresh on every request (no shared state between
//! requests) with two variables in scope:
//!
//! - `fields` — a Rhai object map of the submitted form field values (empty
//!   if the request carried no data). NomadNet clients send field values as
//!   a msgpack map; we decode it and expose it as plain strings.
//! - `remote_identity` — the requester's identity hash as a lowercase hex
//!   string, or `""` if the link hasn't identified.
//!
//! The script's final expression is the response body (as a string). Errors
//! are turned into a small Micron page rather than dropping the request
//! silently, since this is meant for a single operator debugging their own
//! pages, not a multi-tenant service.

use std::path::Path;

use rhai::{Dynamic, Engine, Map, Scope};

/// Builds the shared script engine. One `Engine` is reused across requests
/// (safe: `eval_file_with_scope` takes `&self`, and the `sync` feature makes
/// `Engine` `Send + Sync`); only the `Scope` is per-request.
///
/// The limits below are the actual safety boundary here, since a script runs
/// on every incoming, unauthenticated RNS request: they bound a script to a
/// fixed number of operations so an infinite loop or runaway recursion can't
/// hang the node or become a remote DoS.
pub fn build_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(500_000)
        .set_max_expr_depths(64, 32)
        .set_max_string_size(1_000)
        .set_max_array_size(1_000)
        .set_max_map_size(1_000);
    engine
}

/// Runs `path` and returns the rendered page bytes, or a human-readable
/// error (returned to the requester as a small Micron page — see module
/// docs for why that's an acceptable tradeoff here).
pub fn run(
    engine: &Engine,
    path: &Path,
    request_data: &[u8],
    remote_identity_hex: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut scope = Scope::new();
    scope.push("fields", decode_fields(request_data));
    scope.push(
        "remote_identity",
        remote_identity_hex.unwrap_or("").to_string(),
    );

    engine
        .eval_file_with_scope::<String>(&mut scope, path.to_path_buf())
        .map(String::into_bytes)
        .map_err(|e| e.to_string())
}

/// Renders a script error as a minimal Micron page.
pub fn error_page(request_path: &str, message: &str) -> Vec<u8> {
    format!(">Script error\n\n`[{request_path}`]\n\n{message}\n").into_bytes()
}

/// Best-effort decode of the client's request payload into a Rhai map.
///
/// NomadNet form submissions arrive as a msgpack map of `{field_name:
/// value}`; values are usually byte strings. Anything that isn't a map (no
/// fields submitted, or a client sending something unexpected) yields an
/// empty map rather than an error — dynamic pages should treat missing
/// fields as normal, not fatal.
fn decode_fields(data: &[u8]) -> Map {
    let mut out = Map::new();

    if data.is_empty() {
        return out;
    }

    let Ok(value) = rmpv::decode::read_value(&mut &data[..]) else {
        return out;
    };

    let rmpv::Value::Map(pairs) = value else {
        return out;
    };

    for (key, value) in pairs {
        let Some(key) = key.as_str() else { continue };
        let clean_key = key.strip_prefix("field_").unwrap_or(key);

        out.insert(clean_key.into(), rmpv_to_dynamic(&value));
    }
    out
}

fn rmpv_to_dynamic(value: &rmpv::Value) -> Dynamic {
    match value {
        rmpv::Value::Nil => Dynamic::UNIT,
        rmpv::Value::Boolean(b) => Dynamic::from(*b),
        rmpv::Value::Integer(i) => i
            .as_i64()
            .map(Dynamic::from)
            .unwrap_or_else(|| Dynamic::from(i.to_string())),
        rmpv::Value::F32(f) => Dynamic::from(*f as f64),
        rmpv::Value::F64(f) => Dynamic::from(*f),
        rmpv::Value::String(s) => Dynamic::from(s.as_str().unwrap_or_default().to_string()),
        // Field values are most often raw bytes (bytes-of-utf8-text on the
        // Python/NomadNet side); decode lossily rather than dropping them.
        rmpv::Value::Binary(bytes) => Dynamic::from(String::from_utf8_lossy(bytes).into_owned()),
        other => Dynamic::from(other.to_string()),
    }
}
