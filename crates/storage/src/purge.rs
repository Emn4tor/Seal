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
///
/// Every deletion below is still attempted even if an earlier one failed,
/// same reasoning as the keychain/database split above: a purge that gave
/// up after the first failure could leave the other files behind, again
/// defeating the point. A missing file is not a failure (nothing to
/// delete is the expected case for e.g. `-wal`/`-shm` with no pending
/// write-ahead data); anything else, such as a file still open elsewhere,
/// is returned as a real error instead of being silently swallowed, so a
/// caller reporting "purge succeeded" back to the user is actually true.
pub fn panic_purge(db_path: &Path, keychain: &Keychain) -> Result<(), StorageError> {
    let kek_result = keychain.delete_kek();
    let mut first_file_error = None;
    for suffix in ["", "-wal", "-shm"] {
        let mut path = db_path.as_os_str().to_owned();
        path.push(suffix);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
            && first_file_error.is_none()
        {
            first_file_error = Some(e);
        }
    }
    kek_result?;
    if let Some(e) = first_file_error {
        return Err(e.into());
    }
    Ok(())
}
