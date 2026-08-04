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
}
