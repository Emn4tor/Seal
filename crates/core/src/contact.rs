use libp2p::{Multiaddr, PeerId};

/// One of a contact's devices: its own Curve25519 key and network address.
/// The first device's key matches the account's master identity key; a
/// device added later via QR pairing gets its own.
#[derive(Debug, Clone)]
pub struct DeviceContact {
    pub device_id: String,
    pub curve25519_key: String,
    pub peer_id: PeerId,
    /// Known dialable addresses — passed to `send_request_with_addresses`
    /// so request-response can establish a connection itself if needed,
    /// rather than depending on some *other* dial (e.g. a bare-multiaddr
    /// one that isn't tied to this peer_id ahead of time) having already
    /// completed.
    pub addrs: Vec<Multiaddr>,
}

/// A known account we can message, potentially across more than one
/// currently-reachable device. Zero devices means it can't be sent to right now.
#[derive(Debug, Clone)]
pub struct Contact {
    pub user_id: String,
    pub devices: Vec<DeviceContact>,
}
