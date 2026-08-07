use std::collections::HashMap;

use vodozemac::megolm::{
    GroupSession, InboundGroupSession, MegolmMessage, SessionConfig, SessionKey,
};

use crate::envelope::GroupEnvelope;
use crate::error::CryptoError;

/// In-memory cache of Megolm group sessions: one outbound (our own sending
/// ratchet) per group we're a member of, plus one inbound session per
/// `(group, sender)` pair we've received a key share for.
#[derive(Default)]
pub struct MegolmManager {
    outbound: HashMap<String, GroupSession>,
    inbound: HashMap<(String, String, String), InboundGroupSession>,
    /// Highest `message_index` successfully decrypted per `(group, sender,
    /// session)`. A Megolm inbound session can derive the key for any
    /// index on demand and doesn't track which ones it already handed
    /// out, so replay protection has to live here: reject anything at or
    /// below the highest index already seen for that session.
    highest_seen_index: HashMap<(String, String, String), u32>,
}

impl MegolmManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or replaces/rotates) the outbound group session for
    /// `group_id`, returning the session key to share with current members.
    /// Called on group creation, and again on every membership removal —
    /// rotating retires the old session key so a removed member can't
    /// decrypt anything encrypted after they lost access.
    pub fn rotate_outbound(&mut self, group_id: &str) -> SessionKey {
        let session = GroupSession::new(SessionConfig::version_1());
        let key = session.session_key();
        self.outbound.insert(group_id.to_string(), session);
        key
    }

    pub fn has_outbound(&self, group_id: &str) -> bool {
        self.outbound.contains_key(group_id)
    }

    /// Returns the *current* outbound session key for `group_id` without
    /// rotating it — used to share the same key with multiple members of a
    /// group one at a time (e.g. when it's first created).
    pub fn current_session_key(&self, group_id: &str) -> Option<SessionKey> {
        self.outbound.get(group_id).map(|s| s.session_key())
    }

    /// Encrypts a group message with the current outbound session for
    /// `group_id`, producing the wire envelope to publish on the group's
    /// gossipsub topic.
    pub fn encrypt(
        &mut self,
        group_id: &str,
        sender_user_id: &str,
        plaintext: &[u8],
    ) -> Result<GroupEnvelope, CryptoError> {
        let session = self
            .outbound
            .get_mut(group_id)
            .ok_or(CryptoError::UnknownGroupSession)?;
        let session_id = session.session_id();
        let message = session.encrypt(plaintext);
        Ok(GroupEnvelope {
            group_id: group_id.to_string(),
            sender_user_id: sender_user_id.to_string(),
            session_id,
            ciphertext: message.to_bytes(),
        })
    }

    /// Registers a member's session key (received via a 1:1
    /// `DirectPayload::GroupKeyShare`) so their future messages in this
    /// group can be decrypted. Returns the session id for bookkeeping.
    pub fn insert_inbound(
        &mut self,
        group_id: &str,
        sender_user_id: &str,
        session_key: &SessionKey,
    ) -> String {
        let session = InboundGroupSession::new(session_key, SessionConfig::version_1());
        let session_id = session.session_id();
        self.inbound.insert(
            (
                group_id.to_string(),
                sender_user_id.to_string(),
                session_id.clone(),
            ),
            session,
        );
        session_id
    }

    pub fn decrypt(&mut self, envelope: &GroupEnvelope) -> Result<Vec<u8>, CryptoError> {
        let key = (
            envelope.group_id.clone(),
            envelope.sender_user_id.clone(),
            envelope.session_id.clone(),
        );
        let session = self
            .inbound
            .get_mut(&key)
            .ok_or(CryptoError::UnknownGroupSession)?;
        let message = MegolmMessage::from_bytes(&envelope.ciphertext)
            .map_err(|e| CryptoError::Decode(e.to_string()))?;
        let decrypted = session.decrypt(&message)?;

        if let Some(&highest) = self.highest_seen_index.get(&key)
            && decrypted.message_index <= highest
        {
            return Err(CryptoError::ReplayedGroupMessage);
        }
        self.highest_seen_index.insert(key, decrypted.message_index);

        Ok(decrypted.plaintext)
    }
}
