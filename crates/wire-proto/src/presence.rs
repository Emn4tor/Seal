use serde::{Deserialize, Serialize};

use crate::signing::join;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdateRequest {
    pub user_id: String,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub relay_addrs: Vec<String>,
    pub ttl_secs: u64,
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
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.user_id,
                &self.peer_id,
                &multi,
                &relay,
                &ttl,
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
}
