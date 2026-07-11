//! Field-level encryption for sensitive columns, keyed by the OS-keychain KEK.
//!
//! The database file itself is plain SQLite (see `db.rs`) — individual
//! sensitive values (identity/session pickles, message bodies) are encrypted
//! as opaque BLOBs with XChaCha20-Poly1305 before being written. Losing the
//! KEK (the local panic-purge deletes it from the keychain) makes every one
//! of these blobs permanently undecipherable, which is what makes purge a
//! real crypto-shred rather than just a file deletion.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use rand::rngs::OsRng;

use crate::error::StorageError;

const NONCE_LEN: usize = 24;

pub fn encrypt_blob(kek: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(kek));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .expect("in-memory AEAD encryption cannot fail");

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

pub fn decrypt_blob(kek: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, StorageError> {
    if data.len() < NONCE_LEN {
        return Err(StorageError::Crypto("ciphertext too short".into()));
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(kek));
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| StorageError::Crypto("decryption failed (wrong key or corrupted data)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let kek = [7u8; 32];
        let ciphertext = encrypt_blob(&kek, b"hello world");
        let plaintext = decrypt_blob(&kek, &ciphertext).unwrap();
        assert_eq!(plaintext, b"hello world");
    }

    #[test]
    fn wrong_key_fails() {
        let ciphertext = encrypt_blob(&[1u8; 32], b"secret");
        assert!(decrypt_blob(&[2u8; 32], &ciphertext).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut ciphertext = encrypt_blob(&[3u8; 32], b"secret");
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 0xFF;
        assert!(decrypt_blob(&[3u8; 32], &ciphertext).is_err());
    }
}
