use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::VerifyingKey;
use identity::{Identity, Keychain};

#[test]
fn user_id_is_derived_from_ed25519_key_and_stable_across_calls() {
    let identity = Identity::generate();
    let id_a = identity.user_id();
    let id_b = identity.user_id();
    assert_eq!(id_a, id_b);
    // 16 bytes of SHA-256, hex-encoded.
    assert_eq!(id_a.len(), 32);
    assert!(id_a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn pickle_round_trip_preserves_identity_and_signing_capability() {
    let identity = Identity::generate();
    let user_id = identity.user_id();
    let ed_key = identity.ed25519_public_base64();

    let pickled = identity.pickle_to_json().expect("pickle to json");
    let restored = Identity::from_pickle_json(&pickled).expect("restore from pickle");

    assert_eq!(restored.user_id(), user_id);
    assert_eq!(restored.ed25519_public_base64(), ed_key);

    // The restored account must still hold the private key material, not
    // just public identifiers — prove it by signing and verifying.
    let sig = restored.sign(b"hello from the restored account");
    assert!(verify_with_dalek(
        &ed_key,
        b"hello from the restored account",
        &sig
    ));
}

/// Cross-crate contract check: a signature produced by `Identity::sign` must
/// be verifiable via plain `ed25519-dalek` decoding standard base64 — the
/// exact path `directory-server::auth::verify_signature` takes. If
/// `vodozemac`'s own base64 variant ever leaked through here instead, this
/// would fail even though everything *within* this crate looks fine.
#[test]
fn signature_is_directory_server_compatible() {
    let identity = Identity::generate();
    let message = b"register-user/v1\x1fsome-signing-bytes";
    let signature = identity.sign(message);

    assert!(verify_with_dalek(
        &identity.ed25519_public_base64(),
        message,
        &signature
    ));
}

fn verify_with_dalek(pubkey_b64: &str, message: &[u8], signature_b64: &str) -> bool {
    let pubkey_bytes = STANDARD.decode(pubkey_b64).unwrap();
    let pubkey = VerifyingKey::from_bytes(&pubkey_bytes.try_into().unwrap()).unwrap();
    let sig_bytes = STANDARD.decode(signature_b64).unwrap();
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes.try_into().unwrap());
    pubkey.verify_strict(message, &sig).is_ok()
}

/// RAII guard so the keychain entry this test creates is always removed,
/// even if an assertion panics partway through.
struct TestKeychainGuard(Keychain);

impl Drop for TestKeychainGuard {
    fn drop(&mut self) {
        let _ = self.0.delete_kek();
    }
}

// Runs unmodified on macOS despite the KEK now being Touch ID-protected
// there: `cargo test` produces an unsigned binary, which can't be granted
// `SecAccessControl` at all, so this exercises the fallback path in
// `identity::keychain::macos::set`, not a live biometric prompt.
//
// Ignored on Linux for this test specifically, not the crate/CI wiring in
// general: `ci.yml` stands up a real D-Bus Secret Service and every other
// integration test that resolves a KEK through `Keychain` passes reliably
// there. Only this test's tight create/delete/recreate loop has proven to
// reliably hit `NoStorageAccess(NoResult)` even with a startup grace
// period — gnome-keyring-daemon in headless CI isn't fast/robust enough
// for that specific rapid churn. Real coverage still exists via macOS and
// Windows, both of which run it unmodified.
#[cfg_attr(
    target_os = "linux",
    ignore = "gnome-keyring-daemon in headless CI hasn't proven reliable for this test's rapid create/delete/recreate cycle specifically — see this comment"
)]
#[test]
fn keychain_kek_is_stable_then_crypto_shredded_on_delete() {
    let unique = format!("p2p-chat-test-{}", uuid_like());
    let keychain = Keychain::new(&unique, "kek").expect("open test keychain entry");
    let guard = TestKeychainGuard(keychain);

    let first = guard.0.load_or_create_kek().expect("create kek");
    let second = guard.0.load_or_create_kek().expect("load kek again");
    assert_eq!(first, second, "KEK must be stable across loads");

    guard.0.delete_kek().expect("delete kek");
    let after_delete = guard.0.load_or_create_kek().expect("recreate kek");
    assert_ne!(
        first, after_delete,
        "a fresh KEK must be generated after crypto-shredding the old one"
    );
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}-{:x}", std::process::id())
}
