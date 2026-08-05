use std::path::Path;

use identity::Keychain;

use crate::error::StorageError;

/// The local "panic purge": crypto-shreds the KEK first — making every
/// encrypted column (identity/session pickles, message bodies) permanently
/// undecipherable no matter what happens to the file afterwards — then
/// deletes the database file and its WAL/SHM siblings as a courtesy so
/// nothing lingers on disk either.
///
/// The database deletion always runs, even if the keychain step fails: a
/// purge that bailed out early (KEK delete `?`-propagated) used to leave
/// the database fully intact on any keychain error, which defeats the
/// entire point of "delete everything" — the actual data on disk, not just
/// the key, is what the caller asked to get rid of. Known-benign keychain
/// errors (an OS-level ownership mismatch on an unsigned dev build that's
/// been rebuilt since the KEK was created — see
/// `identity::keychain::macos::delete`'s doc comment) are already absorbed
/// there; this is a second layer of defense for anything unanticipated,
/// since "the database is gone" matters more than "the keychain step
/// reported success."
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
