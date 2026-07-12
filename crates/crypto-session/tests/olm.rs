use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crypto_session::OlmManager;
use identity::Identity;

#[test]
fn olm_1to1_round_trip_is_bidirectional() {
    let mut alice = Identity::generate();
    let mut bob = Identity::generate();

    // Bob publishes a one-time key, as he would to the directory server.
    bob.account_mut().generate_one_time_keys(1);
    let bob_otk = *bob.account().one_time_keys().values().next().unwrap();
    let bob_otk_b64 = STANDARD.encode(bob_otk.as_bytes());
    let bob_identity_key = bob.curve25519_public_base64();
    let alice_identity_key = alice.curve25519_public_base64();

    let mut alice_olm = OlmManager::new();
    alice_olm
        .start_outbound(&alice, &bob_identity_key, &bob_otk_b64)
        .expect("alice starts outbound session");

    let envelope = alice_olm
        .encrypt(&alice, &bob_identity_key, b"hello bob")
        .expect("alice encrypts first message");
    assert_eq!(envelope.sender_user_id, alice.user_id());
    assert_eq!(envelope.sender_curve25519_key, alice_identity_key);

    // Bob has no prior session — this pre-key message must transparently
    // establish one.
    let mut bob_olm = OlmManager::new();
    assert!(!bob_olm.has_session_with(&alice_identity_key));
    let plaintext = bob_olm
        .decrypt(&mut bob, &envelope)
        .expect("bob decrypts and establishes inbound session");
    assert_eq!(plaintext, b"hello bob");
    assert!(bob_olm.has_session_with(&alice_identity_key));

    // Bob replies; Alice's already-established outbound session must
    // decrypt it directly, no new session needed.
    let reply = bob_olm
        .encrypt(&bob, &alice_identity_key, b"hi alice")
        .expect("bob encrypts reply");
    let reply_plaintext = alice_olm
        .decrypt(&mut alice, &reply)
        .expect("alice decrypts reply with existing session");
    assert_eq!(reply_plaintext, b"hi alice");
}

#[test]
fn decrypting_without_a_session_and_without_a_prekey_fails() {
    let mut bob = Identity::generate();
    let mut bob_olm = OlmManager::new();

    let bogus = crypto_session::DirectEnvelope {
        sender_user_id: "nobody".to_string(),
        sender_curve25519_key: STANDARD.encode([9u8; 32]),
        message_type: 1, // "Normal", not a pre-key — can't bootstrap a session
        ciphertext: vec![1, 2, 3],
    };

    assert!(bob_olm.decrypt(&mut bob, &bogus).is_err());
}
