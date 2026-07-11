use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::crypto::{decrypt_blob, encrypt_blob};
use crate::error::StorageError;
use crate::store::LocalStore;

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub conversation_id: String,
    pub sender_user_id: String,
    pub body: String,
    pub attachment: Option<StoredAttachment>,
    pub sent_at: i64,
}

#[derive(Debug, Clone)]
pub struct StoredAttachment {
    pub filename: String,
    pub mime_type: String,
    pub exif_stripped: bool,
    pub data: Vec<u8>,
}

/// The on-disk shape of `body_blob`'s plaintext, once decrypted — kept
/// separate from `StoredMessage`/`StoredAttachment` (the public API) so
/// this file is the only place that needs to know the blob is JSON with
/// the attachment bytes base64-encoded. JSON has no binary type, so the
/// bytes need *some* text encoding to stay inside a JSON value; unlike the
/// P2P wire format (see `crates/core/src/node.rs`, which switched to
/// bincode for exactly this reason), the size/perf cost of base64 doesn't
/// matter here — this is written and read once, locally, never
/// retransmitted.
#[derive(Serialize, Deserialize)]
struct AttachmentOnDisk {
    filename: String,
    mime_type: String,
    exif_stripped: bool,
    data_base64: String,
}

#[derive(Serialize, Deserialize)]
struct MessageOnDisk {
    body: String,
    #[serde(default)]
    attachment: Option<AttachmentOnDisk>,
}

impl LocalStore {
    /// `conversation_id` is a DM's peer user_id or a group's group_id —
    /// the caller decides the convention, this table doesn't care.
    pub fn insert_message(
        &self,
        conversation_id: &str,
        sender_user_id: &str,
        body: &str,
        attachment: Option<&StoredAttachment>,
        sent_at: i64,
    ) -> Result<(), StorageError> {
        let on_disk = MessageOnDisk {
            body: body.to_string(),
            attachment: attachment.map(|a| AttachmentOnDisk {
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
                exif_stripped: a.exif_stripped,
                data_base64: STANDARD.encode(&a.data),
            }),
        };
        let json = serde_json::to_vec(&on_disk)
            .map_err(|e| StorageError::Crypto(format!("failed to encode message: {e}")))?;
        let body_blob = encrypt_blob(&self.kek, &json);
        self.conn.execute(
            "INSERT INTO messages (conversation_id, sender_user_id, body_blob, sent_at, delivered)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![conversation_id, sender_user_id, body_blob, sent_at],
        )?;
        Ok(())
    }

    pub fn list_messages(&self, conversation_id: &str) -> Result<Vec<StoredMessage>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT sender_user_id, body_blob, sent_at FROM messages
             WHERE conversation_id = ?1 ORDER BY sent_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![conversation_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.into_iter()
            .map(|(sender_user_id, body_blob, sent_at)| {
                let json_bytes = decrypt_blob(&self.kek, &body_blob)?;
                let on_disk: MessageOnDisk = serde_json::from_slice(&json_bytes).map_err(|e| {
                    StorageError::Crypto(format!("stored message was not valid: {e}"))
                })?;
                let attachment = on_disk
                    .attachment
                    .map(|a| -> Result<StoredAttachment, StorageError> {
                        let data = STANDARD.decode(&a.data_base64).map_err(|e| {
                            StorageError::Crypto(format!(
                                "stored attachment base64 was invalid: {e}"
                            ))
                        })?;
                        Ok(StoredAttachment {
                            filename: a.filename,
                            mime_type: a.mime_type,
                            exif_stripped: a.exif_stripped,
                            data,
                        })
                    })
                    .transpose()?;
                Ok(StoredMessage {
                    conversation_id: conversation_id.to_string(),
                    sender_user_id,
                    body: on_disk.body,
                    attachment,
                    sent_at,
                })
            })
            .collect()
    }
}
