use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::crypto::{decrypt_blob, encrypt_blob};
use crate::error::StorageError;
use crate::store::LocalStore;

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub message_id: String,
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

/// On-disk shape of `body_blob`'s decrypted plaintext, kept separate from
/// the public `StoredMessage`/`StoredAttachment`.
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
    ///
    /// `INSERT OR IGNORE` on `message_id` rather than a plain `INSERT`:
    /// this is what makes re-running a manual sync safe to repeat — an
    /// already-stored message is silently skipped, not duplicated.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_message(
        &self,
        message_id: &str,
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
            "INSERT OR IGNORE INTO messages (message_id, conversation_id, sender_user_id, body_blob, sent_at, delivered)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![message_id, conversation_id, sender_user_id, body_blob, sent_at],
        )?;
        Ok(())
    }

    pub fn list_messages(&self, conversation_id: &str) -> Result<Vec<StoredMessage>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id, sender_user_id, body_blob, sent_at FROM messages
             WHERE conversation_id = ?1 ORDER BY sent_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![conversation_id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // One tampered/bit-rotted row used to fail the whole conversation's
        // history via a short-circuiting `collect`, even though every other
        // row was perfectly fine. Best-effort instead: skip and log just
        // the bad one, same "degrade, don't fail outright" philosophy as
        // this crate's other local-data reads.
        let messages = rows
            .into_iter()
            .filter_map(|(message_id, sender_user_id, body_blob, sent_at)| {
                match Self::decode_message(
                    &self.kek,
                    message_id,
                    conversation_id,
                    &sender_user_id,
                    &body_blob,
                    sent_at,
                ) {
                    Ok(message) => Some(message),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            conversation_id,
                            sent_at,
                            "skipping a message that failed to decode, rest of the conversation still loads"
                        );
                        None
                    }
                }
            })
            .collect();
        Ok(messages)
    }

    /// Every message across every conversation sent or received after
    /// `since`, for `AppService::sync_with_device`. Rows predating
    /// `message_id` (`NULL`) are skipped: there's no stable id to sync them under.
    pub fn list_messages_since(&self, since: i64) -> Result<Vec<StoredMessage>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id, conversation_id, sender_user_id, body_blob, sent_at FROM messages
             WHERE sent_at > ?1 AND message_id IS NOT NULL ORDER BY sent_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![since], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let messages = rows
            .into_iter()
            .filter_map(
                |(message_id, conversation_id, sender_user_id, body_blob, sent_at)| {
                    match Self::decode_message(
                        &self.kek,
                        Some(message_id),
                        &conversation_id,
                        &sender_user_id,
                        &body_blob,
                        sent_at,
                    ) {
                        Ok(message) => Some(message),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                conversation_id,
                                sent_at,
                                "skipping a message that failed to decode while gathering sync candidates"
                            );
                            None
                        }
                    }
                },
            )
            .collect();
        Ok(messages)
    }

    fn decode_message(
        kek: &[u8; 32],
        message_id: Option<String>,
        conversation_id: &str,
        sender_user_id: &str,
        body_blob: &[u8],
        sent_at: i64,
    ) -> Result<StoredMessage, StorageError> {
        let json_bytes = decrypt_blob(kek, body_blob)?;
        let on_disk: MessageOnDisk = serde_json::from_slice(&json_bytes)
            .map_err(|e| StorageError::Crypto(format!("stored message was not valid: {e}")))?;
        let attachment = on_disk
            .attachment
            .map(|a| -> Result<StoredAttachment, StorageError> {
                let data = STANDARD.decode(&a.data_base64).map_err(|e| {
                    StorageError::Crypto(format!("stored attachment base64 was invalid: {e}"))
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
            // Pre-multi-device rows have no message_id (see
            // `list_messages_since`'s doc comment) — `list_messages` still
            // needs to display them, just without a real sync-able id.
            message_id: message_id.unwrap_or_default(),
            conversation_id: conversation_id.to_string(),
            sender_user_id: sender_user_id.to_string(),
            body: on_disk.body,
            attachment,
            sent_at,
        })
    }
}
