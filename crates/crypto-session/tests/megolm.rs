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
