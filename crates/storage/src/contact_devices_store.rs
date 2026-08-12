use rusqlite::params;

use crate::error::StorageError;
use crate::store::LocalStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContactDevice {
    pub device_id: String,
    pub device_ed25519_key: String,
    pub device_curve25519_key: String,
    pub cert_signature: String,
}

impl LocalStore {
    /// Replaces `user_id`'s entire device list with `devices` — the list is
    /// always fetched and verified as a whole, never patched incrementally,
    /// so a device removed from the account should disappear locally too.
    pub fn replace_contact_devices(
        &self,
        user_id: &str,
        devices: &[StoredContactDevice],
        now: i64,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM contact_devices WHERE user_id = ?1",
            params![user_id],
        )?;
        for device in devices {
            self.conn.execute(
                "INSERT INTO contact_devices (user_id, device_id, device_ed25519_key, device_curve25519_key, cert_signature, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user_id,
                    device.device_id,
                    device.device_ed25519_key,
                    device.device_curve25519_key,
                    device.cert_signature,
                    now
                ],
            )?;
        }
        Ok(())
    }

    pub fn list_contact_devices(
        &self,
        user_id: &str,
    ) -> Result<Vec<StoredContactDevice>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, device_ed25519_key, device_curve25519_key, cert_signature
             FROM contact_devices WHERE user_id = ?1 ORDER BY added_at ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(StoredContactDevice {
                    device_id: row.get(0)?,
                    device_ed25519_key: row.get(1)?,
                    device_curve25519_key: row.get(2)?,
                    cert_signature: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn remove_contact_devices(&self, user_id: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM contact_devices WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }
}
