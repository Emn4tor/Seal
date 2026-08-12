use rusqlite::{OptionalExtension, params};

use crate::error::StorageError;
use crate::store::LocalStore;

impl LocalStore {
    /// How far a manual sync with `peer_device_id` has gotten — `0` if
    /// never synced, so the first sync naturally gathers full history.
    pub fn load_sync_cursor(&self, peer_device_id: &str) -> Result<i64, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_synced_at FROM sync_state WHERE peer_device_id = ?1",
                params![peer_device_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn save_sync_cursor(
        &self,
        peer_device_id: &str,
        last_synced_at: i64,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO sync_state (peer_device_id, last_synced_at)
             VALUES (?1, ?2)
             ON CONFLICT(peer_device_id) DO UPDATE SET last_synced_at = excluded.last_synced_at",
            params![peer_device_id, last_synced_at],
        )?;
        Ok(())
    }
}
