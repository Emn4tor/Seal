use serde::{Deserialize, Serialize};

use crate::signing::join;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdateRequest {
    pub user_id: String,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub relay_addrs: Vec<String>,
    pub ttl_secs: u64,
    pub share_online_status: bool,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl PresenceUpdateRequest {
    pub const DOMAIN: &'static str = "presence-update/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let multi = self.multiaddrs.join(",");
        let relay = self.relay_addrs.join(",");
        let ttl = self.ttl_secs.to_string();
        let share = if self.share_online_status { "1" } else { "0" };
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.user_id,
                &self.peer_id,
                &multi,
                &relay,
                &ttl,
                share,
                &ts,
                &self.nonce,
            ],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceRecord {
    pub user_id: String,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub relay_addrs: Vec<String>,
    pub expires_at: i64,
    pub share_online_status: bool,
}

/// The directory server's own libp2p relay identity, if it's running one —
/// public, unauthenticated info (equivalent to an SSH host key fingerprint):
/// just enough for a client to dial the relay and request a circuit
/// reservation. Never includes anything about *other* users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInfoResponse {
    pub peer_id: String,
    /// Full dialable multiaddr, already including the `/p2p/<peer_id>`
    /// suffix — e.g. `/dns4/seal.emn4tor.de/tcp/4001/p2p/12D3Koo...`.
    pub multiaddr: String,
}
