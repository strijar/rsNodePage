//! Builds and sends `nomadnetwork.node` announces. Mirrors the pattern used
//! by rsLXMF for its control/propagation destinations: hand-assemble the
//! wire packet (header + announce payload) and push it straight onto the
//! transport actor's outbound channel.

use bytes::Bytes;
use rns_identity::announce::AnnounceData;
use rns_identity::identity::Identity;
use rns_transport::messages::{OutboundRequest, TransportMessage};
use tokio::sync::mpsc;

pub const APP_NAME: &str = "nomadnetwork.node";

pub fn send_announce(
    tx: &mpsc::Sender<TransportMessage>,
    identity: &Identity,
    destination_hash: [u8; 16],
    display_name: &str,
) {
    match build_announce_packet(identity, destination_hash, display_name) {
        Ok(raw) => {
            let _ = tx.try_send(TransportMessage::Outbound(OutboundRequest {
                raw: Bytes::from(raw),
                destination_hash,
            }));
        }
        Err(e) => tracing::warn!("nomad: failed to build announce: {e}"),
    }
}

fn build_announce_packet(
    identity: &Identity,
    destination_hash: [u8; 16],
    display_name: &str,
) -> Result<Vec<u8>, String> {
    let announce = AnnounceData::create(identity, APP_NAME, Some(display_name.as_bytes()), None)
        .map_err(|e| format!("{e}"))?;
    let payload = announce.pack();

    let flags = rns_wire::flags::PacketFlags {
        header_type: rns_wire::flags::HeaderType::Header1,
        context_flag: false,
        transport_type: rns_wire::flags::TransportType::Broadcast,
        destination_type: rns_wire::flags::DestinationType::Single,
        packet_type: rns_wire::flags::PacketType::Announce,
    };
    let header = rns_wire::header::PacketHeader {
        flags,
        hops: 0,
        transport_id: None,
        destination_hash,
        context: rns_wire::context::PacketContext::None,
    };

    let mut raw = header.pack();
    raw.extend_from_slice(&payload);
    Ok(raw)
}
