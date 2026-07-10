#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("invalid key material: {0}")]
    InvalidKeyMaterial(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
