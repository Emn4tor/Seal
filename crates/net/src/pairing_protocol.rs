//! Device-linking (QR pairing) protocol, separate from `DIRECT_PROTOCOL`
//! since it must work before any Olm session exists. Wraps ephemeral
//! X25519-ECDH on top of Noise, same defense-in-depth as Olm/Megolm.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

pub const PAIRING_PROTOCOL: &str = "/p2p-chat/pairing/1";

/// How long a pairing token stays valid after minting — bounds a
/// leaked/photographed QR code's exploit window.
pub const PAIRING_TOKEN_TTL_SECS: u64 = 120;

/// Sent by the joining device after scanning the QR code: proves it saw
/// `token`, presents the fresh device identity it wants certified, and its
/// ephemeral X25519 key to complete the ECDH for `PairingResponse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub token: String,
    pub ephemeral_pubkey: [u8; 32],
    pub device_id: String,
    pub device_ed25519_key: String,
    pub device_curve25519_key: String,
}

/// `encrypted_payload` is a nonce-prepended XChaCha20-Poly1305 ciphertext
/// of a bincode `PairingPayload`, `None` when `error` explains why. Two
/// `Option`s rather than a `Result` enum to stay `cbor`-codec-friendly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingResponse {
    pub encrypted_payload: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Plaintext inside `PairingResponse::encrypted_payload`. Deliberately
/// includes the master private key so every device can re-sign directory
/// registration; exposure is bounded by ECDH, a single-use token, and QR access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    pub master_identity_pickle_json: String,
    pub display_name: String,
    pub cert: wire_proto::DeviceCertificate,
    pub bootstrap: PairingBootstrap,
}

/// What the joining device needs to message existing contacts right away.
/// Deliberately excludes groups: `AppService::discover_missing_groups`
/// already recovers those independently on resume.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PairingBootstrap {
    pub contacts: Vec<PairingContact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingContact {
    pub user_id: String,
    pub display_name: String,
    pub ed25519_key: String,
    pub curve25519_key: String,
    pub verified: bool,
    pub devices: Vec<wire_proto::DeviceCertificate>,
}

/// Wraps `x25519_dalek::EphemeralSecret` so callers outside this crate
/// don't need `x25519-dalek` as a direct dependency just to name the type.
pub struct PairingEphemeralSecret(EphemeralSecret);

/// Generates a fresh ephemeral X25519 keypair for one side of a pairing
/// exchange — a new one every `start_pairing`/QR-scan, never reused, so a
/// completed (or abandoned) pairing attempt can't be replayed.
pub fn generate_ephemeral_keypair() -> (PairingEphemeralSecret, [u8; 32]) {
    let secret = EphemeralSecret::random_from_rng(OsRng);
    let public = X25519PublicKey::from(&secret);
    (PairingEphemeralSecret(secret), *public.as_bytes())
}

/// Completes the ECDH — consumes `my_secret` since an `EphemeralSecret` is
/// only ever meant to produce one shared secret.
pub fn derive_shared_key(
    my_secret: PairingEphemeralSecret,
    their_pubkey_bytes: [u8; 32],
) -> [u8; 32] {
    let their_pubkey = X25519PublicKey::from(their_pubkey_bytes);
    *my_secret.0.diffie_hellman(&their_pubkey).as_bytes()
}

const NONCE_LEN: usize = 24;

/// Same nonce-prepended XChaCha20-Poly1305 shape as `storage::crypto`'s
/// `encrypt_blob`/`decrypt_blob`, not shared code — this crate doesn't
/// depend on `storage`, just the same well-understood pattern.
pub fn encrypt_payload(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
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

pub fn decrypt_payload(key: &[u8; 32], data: &[u8]) -> anyhow::Result<Vec<u8>> {
    if data.len() < NONCE_LEN {
        anyhow::bail!("pairing ciphertext too short");
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ciphertext).map_err(|_| {
        anyhow::anyhow!("failed to decrypt pairing response (wrong key or corrupted data)")
    })
}
