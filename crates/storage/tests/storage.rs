use std::time::{SystemTime, UNIX_EPOCH};

use identity::Keychain;
use storage::LocalStore;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn save_and_load_identity_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("local.sqlite3");
    let kek = [42u8; 32];

    let store = LocalStore::open(&db_path, kek).unwrap();
    store
        .save_identity("user-1", "alice", "some-pickle-json", now())
        .unwrap();

    let loaded = store.load_identity().unwrap().expect("identity present");
    assert_eq!(loaded.user_id, "user-1");
    assert_eq!(loaded.display_name, "alice");
    assert_eq!(loaded.pickle_json, "some-pickle-json");
}

#[test]
fn identity_pickle_is_not_stored_in_cleartext() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("local.sqlite3");
    let kek = [11u8; 32];
    let marker = "MARKER_PLAINTEXT_a1b2c3d4e5f6";

    let store = LocalStore::open(&db_path, kek).unwrap();
    store
        .save_identity("user-1", "alice", marker, now())
        .unwrap();

    // Read the raw blob directly via a fresh, independent connection —
    // proving the on-disk bytes never contain the plaintext, not just that
    // the high-level API round-trips correctly.
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = raw
        .query_row("SELECT pickle_blob FROM identity WHERE id = 0", [], |row| {
            row.get(0)
        })
        .unwrap();
    let blob_str = String::from_utf8_lossy(&blob);
    assert!(
        !blob_str.contains(marker),
        "plaintext marker leaked into the on-disk blob"
    );

    // But the legitimate API, holding the right KEK, still recovers it.
    let loaded = store.load_identity().unwrap().unwrap();
    assert_eq!(loaded.pickle_json, marker);
}

#[test]
fn wrong_kek_fails_to_decrypt_instead_of_returning_garbage() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("local.sqlite3");

    {
        let store = LocalStore::open(&db_path, [1u8; 32]).unwrap();
        store
            .save_identity("user-1", "alice", "secret-pickle", now())
            .unwrap();
    }

    let store_wrong_key = LocalStore::open(&db_path, [2u8; 32]).unwrap();
    let result = store_wrong_key.load_identity();
    assert!(
        result.is_err(),
        "decrypting with the wrong KEK must fail, not silently return wrong data"
    );
}

/// RAII guard so the keychain entry this test creates is always removed,
/// even if an assertion panics partway through.
struct TestKeychainGuard(Keychain);

impl Drop for TestKeychainGuard {
    fn drop(&mut self) {
        let _ = self.0.delete_kek();
    }
}

#[test]
fn panic_purge_deletes_db_files_and_crypto_shreds_the_kek() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("local.sqlite3");
    let service = format!("p2p-chat-test-{}", unique_suffix());
    let keychain = Keychain::new(&service, "kek").unwrap();
    let guard = TestKeychainGuard(keychain);

    let kek = guard.0.load_or_create_kek().unwrap();
    {
        let store = LocalStore::open(&db_path, kek).unwrap();
        store
            .save_identity("user-1", "alice", "some-pickle-json", now())
            .unwrap();
    } // connection closed before purge, as required

    assert!(db_path.exists());

    storage::panic_purge(&db_path, &guard.0).unwrap();

    assert!(!db_path.exists(), "db file must be deleted by purge");
    let new_kek = guard.0.load_or_create_kek().unwrap();
    assert_ne!(
        kek, new_kek,
        "a fresh KEK must be generated after crypto-shredding the old one"
    );
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}-{:x}", std::process::id())
}
