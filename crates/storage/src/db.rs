use std::path::Path;

use rusqlite::Connection;

use crate::error::StorageError;

const SCHEMA: &str = include_str!("schema.sql");

/// Opens (creating if necessary) the local database at `db_path`. The file
/// itself is plain SQLite; sensitive columns are encrypted individually at
/// the application layer with the keychain-backed KEK (see `crypto.rs`).
pub fn open(db_path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    migrate_add_device_id_column(&conn)?;
    migrate_add_device_olm_pickle_column(&conn)?;
    migrate_add_message_id_column(&conn)?;
    Ok(conn)
}

/// Adds `p2p_identity.device_id` for installs predating multi-device
/// support (`CREATE TABLE IF NOT EXISTS` doesn't touch existing tables).
/// Left `NULL`; `AppService::load_or_create` backfills a real value.
fn migrate_add_device_id_column(conn: &Connection) -> Result<(), StorageError> {
    let has_column = conn
        .prepare("SELECT 1 FROM pragma_table_info('p2p_identity') WHERE name = 'device_id'")?
        .exists([])?;
    if !has_column {
        conn.execute("ALTER TABLE p2p_identity ADD COLUMN device_id TEXT", [])?;
    }
    Ok(())
}

/// Same backfill-on-next-launch pattern as `migrate_add_device_id_column`,
/// for `p2p_identity.device_olm_pickle_blob`.
fn migrate_add_device_olm_pickle_column(conn: &Connection) -> Result<(), StorageError> {
    let has_column = conn
        .prepare(
            "SELECT 1 FROM pragma_table_info('p2p_identity') WHERE name = 'device_olm_pickle_blob'",
        )?
        .exists([])?;
    if !has_column {
        conn.execute(
            "ALTER TABLE p2p_identity ADD COLUMN device_olm_pickle_blob BLOB",
            [],
        )?;
    }
    Ok(())
}

/// Same backfill pattern, for `messages.message_id`; the unique index is
/// created here too since it can only succeed once the column exists.
fn migrate_add_message_id_column(conn: &Connection) -> Result<(), StorageError> {
    let has_column = conn
        .prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = 'message_id'")?
        .exists([])?;
    if !has_column {
        conn.execute("ALTER TABLE messages ADD COLUMN message_id TEXT", [])?;
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_message_id ON messages (message_id)",
        [],
    )?;
    Ok(())
}
