use serde::{Deserialize, Serialize};

/// The libp2p-level request-response protocol id for 1:1 direct messages.
/// Payloads are opaque bytes — Olm ciphertext once Phase 4 wires in
/// end-to-end encryption; plain bytes for now while only the transport is
/// being exercised.
pub const DIRECT_PROTOCOL: &str = "/p2p-chat/direct/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub ack: bool,
}
