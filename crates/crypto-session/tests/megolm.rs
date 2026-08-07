use crypto_session::MegolmManager;

#[test]
fn megolm_group_round_trip() {
    let mut alice_megolm = MegolmManager::new();
    let session_key = alice_megolm.rotate_outbound("group-1");

    let envelope = alice_megolm
        .encrypt("group-1", "alice-id", b"hello group")
        .expect("alice encrypts a group message");

    let mut bob_megolm = MegolmManager::new();
    bob_megolm.insert_inbound("group-1", "alice-id", &session_key);

    let plaintext = bob_megolm
        .decrypt(&envelope)
        .expect("bob decrypts with the shared session key");
    assert_eq!(plaintext, b"hello group");
}

#[test]
fn decrypt_without_the_session_key_fails() {
    let mut alice_megolm = MegolmManager::new();
    alice_megolm.rotate_outbound("group-1");
    let envelope = alice_megolm
        .encrypt("group-1", "alice-id", b"secret")
        .unwrap();

    // Bob never received the key share.
    let mut bob_megolm = MegolmManager::new();
    assert!(bob_megolm.decrypt(&envelope).is_err());
}

#[test]
fn a_replayed_message_is_rejected() {
    let mut alice_megolm = MegolmManager::new();
    let session_key = alice_megolm.rotate_outbound("group-1");
    let envelope = alice_megolm
        .encrypt("group-1", "alice-id", b"hello group")
        .expect("alice encrypts a group message");

    let mut bob_megolm = MegolmManager::new();
    bob_megolm.insert_inbound("group-1", "alice-id", &session_key);

    bob_megolm
        .decrypt(&envelope)
        .expect("bob decrypts the message the first time");
    let replayed = bob_megolm.decrypt(&envelope);
    assert!(
        replayed.is_err(),
        "the same ciphertext decrypted twice should be rejected as a replay"
    );
}

#[test]
fn out_of_order_but_not_replayed_messages_still_decrypt() {
    let mut alice_megolm = MegolmManager::new();
    let session_key = alice_megolm.rotate_outbound("group-1");
    let first = alice_megolm
        .encrypt("group-1", "alice-id", b"first")
        .unwrap();
    let second = alice_megolm
        .encrypt("group-1", "alice-id", b"second")
        .unwrap();

    let mut bob_megolm = MegolmManager::new();
    bob_megolm.insert_inbound("group-1", "alice-id", &session_key);

    let second_plaintext = bob_megolm
        .decrypt(&second)
        .expect("a later message can arrive and decrypt before an earlier one");
    assert_eq!(second_plaintext, b"second");

    let first_plaintext = bob_megolm.decrypt(&first);
    assert!(
        first_plaintext.is_err(),
        "an earlier index arriving after a later one has already been seen looks like a \
         replay and is rejected, a deliberate tradeoff for a hard replay window over \
         tolerating reordering"
    );
}

#[test]
fn rotating_outbound_produces_a_new_session_key() {
    let mut mgr = MegolmManager::new();
    let key1 = mgr.rotate_outbound("group-1");
    let key2 = mgr.rotate_outbound("group-1");
    assert_ne!(key1.to_bytes(), key2.to_bytes());
}

#[test]
fn removed_member_cannot_decrypt_messages_sent_after_rotation() {
    let mut alice_megolm = MegolmManager::new();
    let key_before_removal = alice_megolm.rotate_outbound("group-1");

    let mut removed_member_megolm = MegolmManager::new();
    removed_member_megolm.insert_inbound("group-1", "alice-id", &key_before_removal);

    // Alice removes a member and rotates — the removed member's session key
    // is now stale.
    alice_megolm.rotate_outbound("group-1");
    let envelope_after_rotation = alice_megolm
        .encrypt(
            "group-1",
            "alice-id",
            b"the removed member should not read this",
        )
        .unwrap();

    assert!(
        removed_member_megolm
            .decrypt(&envelope_after_rotation)
            .is_err()
    );
}
