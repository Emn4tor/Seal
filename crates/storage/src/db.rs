use std::path::Path;

use rusqlite::Connection;

use crate::error::StorageError;

const SCHEMA: &str = include_str!("schema.sql");

/// Opens (creating if necessary) the local database at `db_path`.
///
/// This file itself is plain SQLite — sensitive columns (identity/session
/// pickles, message bodies) are encrypted individually at the application
/// layer with the OS-keychain-backed KEK (see `crypto.rs`), rather than the
/// whole file being encrypted by a database extension.
pub fn open(db_path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}
