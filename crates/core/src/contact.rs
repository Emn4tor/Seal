use libp2p::{Multiaddr, PeerId};

/// A known peer we can message.
#[derive(Debug, Clone)]
pub struct Contact {
    pub user_id: String,
    pub curve25519_key: String,
    pub peer_id: PeerId,
    /// Known dialable addresses — passed to `send_request_with_addresses`
    /// so request-response can establish a connection itself if needed,
    /// rather than depending on some *other* dial (e.g. a bare-multiaddr
    /// one that isn't tied to this peer_id ahead of time) having already
    /// completed.
    pub addrs: Vec<Multiaddr>,
}
