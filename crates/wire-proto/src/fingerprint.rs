use sha2::{Digest, Sha256};

/// Derives a user's public identity from their Ed25519 identity public key.
///
/// Truncated to 16 bytes (128 bits) of the SHA-256 digest: collision
/// resistance at that width is already far beyond what a user-facing ID
/// space needs, and it keeps the displayed/typed ID shorter.
pub fn user_id_from_ed25519(pubkey_bytes: &[u8]) -> String {
    let digest = Sha256::digest(pubkey_bytes);
    hex::encode(&digest[..16])
}
