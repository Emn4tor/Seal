use std::path::Path;

use identity::Keychain;

use crate::error::StorageError;

/// The local "panic purge": crypto-shreds the KEK first, making every
/// encrypted column permanently undecipherable regardless of what happens
/// to the file afterward, then deletes the database file and its WAL/SHM
/// siblings as a courtesy so nothing lingers on disk either.
///
/// The database deletion always runs, even if the keychain step fails: a
/// purge that bailed out early used to leave the database fully intact on
/// any keychain error, defeating the point of "delete everything." Known-
/// benign keychain errors are already absorbed in
/// `identity::keychain::macos::delete`; this is a second layer of defense
/// for anything unanticipated.
///
/// Callers must close/drop any open [`crate::LocalStore`] connection to
/// `db_path` before calling this.
pub fn panic_purge(db_path: &Path, keychain: &Keychain) -> Result<(), StorageError> {
    let kek_result = keychain.delete_kek();
    for suffix in ["", "-wal", "-shm"] {
        let mut path = db_path.as_os_str().to_owned();
        path.push(suffix);
        let _ = std::fs::remove_file(path);
    }
    kek_result?;
    Ok(())
}
