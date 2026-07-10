use serde::{Deserialize, Serialize};

use crate::signing::join;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterUserRequest {
    pub user_id: String,
    pub display_name: String,
    pub ed25519_key: String,
    pub curve25519_key: String,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl RegisterUserRequest {
    pub const DOMAIN: &'static str = "register-user/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.user_id,
                &self.display_name,
                &self.ed25519_key,
                &self.curve25519_key,
                &ts,
                &self.nonce,
            ],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub user_id: String,
    pub display_name: String,
    pub ed25519_key: String,
    pub curve25519_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneTimeKeyEntry {
    pub key_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadOtkRequest {
    pub user_id: String,
    pub keys: Vec<OneTimeKeyEntry>,
    pub fallback_key: Option<OneTimeKeyEntry>,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl UploadOtkRequest {
    pub const DOMAIN: &'static str = "upload-otk/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut keys_repr: Vec<String> = self
            .keys
            .iter()
            .map(|k| format!("{}:{}", k.key_id, k.public_key))
            .collect();
        keys_repr.sort_unstable();
        let keys_joined = keys_repr.join(",");
        let fallback_repr = self
            .fallback_key
            .as_ref()
            .map(|k| format!("{}:{}", k.key_id, k.public_key))
            .unwrap_or_default();
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.user_id,
                &keys_joined,
                &fallback_repr,
                &ts,
                &self.nonce,
            ],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedOtk {
    pub key_id: String,
    pub public_key: String,
    pub is_fallback: bool,
}
