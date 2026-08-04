#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    /// The macOS-only path (`keychain::macos`) bypasses `keyring` to attach
    /// Touch ID protection to the KEK — see that module for why. Includes,
    /// among other things, the user cancelling a Touch ID/password prompt.
    #[cfg(target_os = "macos")]
    #[error("keychain error: {0}")]
    MacosKeychain(#[from] security_framework::base::Error),
    #[error("invalid key material: {0}")]
    InvalidKeyMaterial(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
