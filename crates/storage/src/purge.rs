use std::path::Path;

use identity::Keychain;

use crate::error::StorageError;

/// The local "panic purge": crypto-shreds the KEK first — making every
/// encrypted column (identity/session pickles, message bodies) permanently
/// undecipherable no matter what happens to the file afterwards — then
/// deletes the database file and its WAL/SHM siblings as a courtesy so
/// nothing lingers on disk either.
///
/// Callers must close/drop any open [`crate::LocalStore`] connection to
/// `db_path` before calling this.
pub fn panic_purge(db_path: &Path, keychain: &Keychain) -> Result<(), StorageError> {
    keychain.delete_kek()?;
    for suffix in ["", "-wal", "-shm"] {
        let mut path = db_path.as_os_str().to_owned();
        path.push(suffix);
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
