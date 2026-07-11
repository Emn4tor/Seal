use rusqlite::{OptionalExtension, params};

use crate::crypto::{decrypt_blob, encrypt_blob};
use crate::error::StorageError;
use crate::store::LocalStore;

pub struct StoredIdentity {
    pub user_id: String,
    pub display_name: String,
    pub pickle_json: String,
}

impl LocalStore {
    pub fn save_identity(
        &self,
        user_id: &str,
        display_name: &str,
        pickle_json: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        let pickle_blob = encrypt_blob(&self.kek, pickle_json.as_bytes());
        self.conn.execute(
            "INSERT INTO identity (id, user_id, display_name, pickle_blob, created_at)
             VALUES (0, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                user_id = excluded.user_id,
                display_name = excluded.display_name,
                pickle_blob = excluded.pickle_blob",
            params![user_id, display_name, pickle_blob, now],
        )?;
        Ok(())
    }

    pub fn load_identity(&self) -> Result<Option<StoredIdentity>, StorageError> {
        let row = self
            .conn
            .query_row(
                "SELECT user_id, display_name, pickle_blob FROM identity WHERE id = 0",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;

        let Some((user_id, display_name, pickle_blob)) = row else {
            return Ok(None);
        };
        let pickle_bytes = decrypt_blob(&self.kek, &pickle_blob)?;
        let pickle_json = String::from_utf8(pickle_bytes)
            .map_err(|e| StorageError::Crypto(format!("stored pickle was not valid utf-8: {e}")))?;
        Ok(Some(StoredIdentity {
            user_id,
            display_name,
            pickle_json,
        }))
    }
}
