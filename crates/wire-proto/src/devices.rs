use serde::{Deserialize, Serialize};

use crate::signing::join;

/// A device's own Ed25519/Curve25519 identity, distinct from the account's
/// master key. `signature` is the master key's signature over the other
/// fields, proof the device was actually linked and not self-declared.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCertificate {
    pub device_id: String,
    pub device_ed25519_key: String,
    pub device_curve25519_key: String,
    pub master_ed25519_key: String,
    pub signature: String,
}

impl DeviceCertificate {
    pub const DOMAIN: &'static str = "device-certificate/v1";

    /// What `master_ed25519_key` must have signed for `signature` to be
    /// valid. Deliberately excludes `signature` itself.
    pub fn signing_bytes(&self) -> Vec<u8> {
        join(
            Self::DOMAIN,
            &[
                &self.device_id,
                &self.device_ed25519_key,
                &self.device_curve25519_key,
                &self.master_ed25519_key,
            ],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterDeviceRequest {
    pub user_id: String,
    pub cert: DeviceCertificate,
    pub timestamp: i64,
    pub nonce: String,
    /// Signs this whole request with the account's master key, same
    /// write-authorization every signed directory endpoint requires.
    /// Separate from `cert.signature`, which answers a different question.
    pub signature: String,
}

impl RegisterDeviceRequest {
    pub const DOMAIN: &'static str = "register-device/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.user_id,
                &self.cert.device_id,
                &self.cert.device_ed25519_key,
                &self.cert.device_curve25519_key,
                &self.cert.master_ed25519_key,
                &self.cert.signature,
                &ts,
                &self.nonce,
            ],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListResponse {
    pub devices: Vec<DeviceCertificate>,
}
