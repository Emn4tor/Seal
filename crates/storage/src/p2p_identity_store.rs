use rusqlite::{OptionalExtension, params};

use crate::crypto::{decrypt_blob, encrypt_blob};
use crate::error::StorageError;
use crate::store::LocalStore;

impl LocalStore {
    /// Persists the raw (protobuf-encoded) libp2p keypair so this account's
    /// PeerId survives restarts — see the `p2p_identity` table's doc
    /// comment in `schema.sql` for why an unstable PeerId is a real bug,
    /// not just cosmetic. Encrypted at rest the same way the chat identity
    /// pickle is (`identity_store`): not because this specific key protects
    /// message content — it's a pure transport-layer signing key — but
    /// there's no reason to leave it in the clear either.
    pub fn save_p2p_keypair(&self, keypair_bytes: &[u8], now: i64) -> Result<(), StorageError> {
        let blob = encrypt_blob(&self.kek, keypair_bytes);
        self.conn.execute(
            "INSERT INTO p2p_identity (id, keypair_blob, created_at)
             VALUES (0, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET keypair_blob = excluded.keypair_blob",
            params![blob, now],
        )?;
        Ok(())
    }

    pub fn load_p2p_keypair(&self) -> Result<Option<Vec<u8>>, StorageError> {
        let row = self
            .conn
            .query_row(
                "SELECT keypair_blob FROM p2p_identity WHERE id = 0",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let Some(blob) = row else {
            return Ok(None);
        };
        decrypt_blob(&self.kek, &blob).map(Some)
    }

    /// This device's own random identifier — `None` until `save_device_id`
    /// has been called at least once. Not encrypted: it's just a routing
    /// label already visible to the directory server and to contacts.
    pub fn load_device_id(&self) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT device_id FROM p2p_identity WHERE id = 0",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Requires a prior `save_p2p_keypair` call — `WHERE id = 0` updates
    /// zero rows silently rather than erroring if there's no row yet.
    pub fn save_device_id(&self, device_id: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE p2p_identity SET device_id = ?1 WHERE id = 0",
            params![device_id],
        )?;
        Ok(())
    }

    /// This device's own Olm account pickle, deliberately separate from
    /// the account's master identity pickle (see `schema.sql`'s doc on
    /// `device_olm_pickle_blob`). Same sequencing requirement as `save_device_id`.
    pub fn save_device_olm_pickle(&self, pickle_json: &str) -> Result<(), StorageError> {
        let blob = encrypt_blob(&self.kek, pickle_json.as_bytes());
        self.conn.execute(
            "UPDATE p2p_identity SET device_olm_pickle_blob = ?1 WHERE id = 0",
            params![blob],
        )?;
        Ok(())
    }

    pub fn load_device_olm_pickle(&self) -> Result<Option<String>, StorageError> {
        let row = self
            .conn
            .query_row(
                "SELECT device_olm_pickle_blob FROM p2p_identity WHERE id = 0",
                [],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten();
        let Some(blob) = row else {
            return Ok(None);
        };
        let bytes = decrypt_blob(&self.kek, &blob)?;
        Ok(Some(String::from_utf8(bytes).map_err(|e| {
            StorageError::Crypto(format!("device olm pickle was not valid utf-8: {e}"))
        })?))
    }
}
