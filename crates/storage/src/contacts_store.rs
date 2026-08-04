use rusqlite::params;

use crate::error::StorageError;
use crate::store::LocalStore;

#[derive(Debug, Clone)]
pub struct StoredContact {
    pub user_id: String,
    pub display_name: String,
    pub ed25519_key: String,
    pub curve25519_key: String,
    pub verified: bool,
}

impl LocalStore {
    /// `last_seen_at` is the caller's current time (matching the rest of
    /// this crate's convention, e.g. `message_store::insert_message`'s
    /// `sent_at` — timestamps are supplied, not computed here), updated
    /// every time this contact's info is synced from the directory (see
    /// `AppService::add_contact_by_user_id`) rather than only on first
    /// insert, so it actually tracks the most recent sync, not just the
    /// first one.
    pub fn upsert_contact(
        &self,
        user_id: &str,
        display_name: &str,
        ed25519_key: &str,
        curve25519_key: &str,
        last_seen_at: i64,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO contacts (user_id, display_name, ed25519_key, curve25519_key, verified, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)
             ON CONFLICT(user_id) DO UPDATE SET
                display_name = excluded.display_name,
                ed25519_key = excluded.ed25519_key,
                curve25519_key = excluded.curve25519_key,
                last_seen_at = excluded.last_seen_at",
            params![user_id, display_name, ed25519_key, curve25519_key, last_seen_at],
        )?;
        Ok(())
    }

    pub fn remove_contact(&self, user_id: &str) -> Result<(), StorageError> {
        self.conn
            .execute("DELETE FROM contacts WHERE user_id = ?1", params![user_id])?;
        Ok(())
    }

    pub fn list_contacts(&self) -> Result<Vec<StoredContact>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT user_id, display_name, ed25519_key, curve25519_key, verified FROM contacts
             ORDER BY display_name ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StoredContact {
                    user_id: row.get(0)?,
                    display_name: row.get(1)?,
                    ed25519_key: row.get(2)?,
                    curve25519_key: row.get(3)?,
                    verified: row.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
