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
    pub fn upsert_contact(
        &self,
        user_id: &str,
        display_name: &str,
        ed25519_key: &str,
        curve25519_key: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO contacts (user_id, display_name, ed25519_key, curve25519_key, verified)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(user_id) DO UPDATE SET
                display_name = excluded.display_name,
                ed25519_key = excluded.ed25519_key,
                curve25519_key = excluded.curve25519_key",
            params![user_id, display_name, ed25519_key, curve25519_key],
        )?;
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
