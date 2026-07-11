use storage::{LocalStore, StoredAttachment};

#[test]
fn contacts_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&dir.path().join("db.sqlite3"), [1u8; 32]).unwrap();

    store
        .upsert_contact("user-1", "alice", "ed25519-a", "curve-a")
        .unwrap();
    store
        .upsert_contact("user-2", "bob", "ed25519-b", "curve-b")
        .unwrap();
    // Upsert updates in place rather than duplicating.
    store
        .upsert_contact("user-1", "alice renamed", "ed25519-a", "curve-a")
        .unwrap();

    let contacts = store.list_contacts().unwrap();
    assert_eq!(contacts.len(), 2);
    let alice = contacts.iter().find(|c| c.user_id == "user-1").unwrap();
    assert_eq!(alice.display_name, "alice renamed");
    assert!(!alice.verified);
}

#[test]
fn messages_round_trip_and_are_encrypted_at_rest() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite3");
    let store = LocalStore::open(&db_path, [2u8; 32]).unwrap();

    store
        .insert_message("conv-1", "alice", "hello", None, 100)
        .unwrap();
    store
        .insert_message("conv-1", "bob", "hi back", None, 101)
        .unwrap();
    store
        .insert_message("conv-2", "alice", "unrelated", None, 100)
        .unwrap();

    let conv1 = store.list_messages("conv-1").unwrap();
    assert_eq!(conv1.len(), 2);
    assert_eq!(conv1[0].sender_user_id, "alice");
    assert_eq!(conv1[0].body, "hello");
    assert_eq!(conv1[1].body, "hi back");

    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = raw
        .query_row(
            "SELECT body_blob FROM messages WHERE conversation_id = 'conv-1' ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("hello"));
}

#[test]
fn message_with_attachment_round_trips_and_stays_encrypted_at_rest() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite3");
    let store = LocalStore::open(&db_path, [4u8; 32]).unwrap();

    let attachment = StoredAttachment {
        filename: "cat.png".to_string(),
        mime_type: "image/png".to_string(),
        exif_stripped: true,
        data: vec![0u8, 1, 2, 3, 255, 254, 253, 252],
    };
    store
        .insert_message("conv-1", "alice", "look at this", Some(&attachment), 100)
        .unwrap();
    // A plain-text message alongside it, to confirm the two don't interfere.
    store
        .insert_message("conv-1", "bob", "nice", None, 101)
        .unwrap();

    let messages = store.list_messages("conv-1").unwrap();
    assert_eq!(messages.len(), 2);

    let with_attachment = &messages[0];
    assert_eq!(with_attachment.body, "look at this");
    let stored = with_attachment
        .attachment
        .as_ref()
        .expect("attachment should round-trip");
    assert_eq!(stored.filename, "cat.png");
    assert_eq!(stored.mime_type, "image/png");
    assert!(stored.exif_stripped);
    assert_eq!(stored.data, vec![0u8, 1, 2, 3, 255, 254, 253, 252]);

    assert!(messages[1].attachment.is_none());

    // The raw on-disk blob must never contain the attachment's plaintext
    // bytes or filename — same guarantee as the plain-text case above,
    // extended to cover the new attachment path.
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = raw
        .query_row(
            "SELECT body_blob FROM messages WHERE conversation_id = 'conv-1' ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("cat.png"));
}

#[test]
fn groups_round_trip_replaces_membership_on_upsert() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(&dir.path().join("db.sqlite3"), [3u8; 32]).unwrap();

    store
        .upsert_group(
            "group-1",
            "my group",
            1,
            &[("owner-id".to_string(), "owner".to_string())],
            &[(
                "chan-general".to_string(),
                "general".to_string(),
                "text".to_string(),
                0,
            )],
        )
        .unwrap();

    store
        .upsert_group(
            "group-1",
            "my group",
            2,
            &[
                ("owner-id".to_string(), "owner".to_string()),
                ("member-id".to_string(), "member".to_string()),
            ],
            &[
                (
                    "chan-general".to_string(),
                    "general".to_string(),
                    "text".to_string(),
                    0,
                ),
                (
                    "chan-voice".to_string(),
                    "hangout".to_string(),
                    "voice".to_string(),
                    1,
                ),
            ],
        )
        .unwrap();

    let groups = store.list_groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].roster_version, 2);
    assert_eq!(groups[0].members.len(), 2);
    assert_eq!(groups[0].channels.len(), 2);
    assert_eq!(groups[0].channels[0].name, "general");
    assert_eq!(groups[0].channels[1].name, "hangout");
    assert_eq!(groups[0].channels[1].kind, "voice");
}
