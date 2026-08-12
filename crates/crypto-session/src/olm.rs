use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use identity::Identity;
use vodozemac::Curve25519PublicKey;
use vodozemac::olm::{OlmMessage, Session, SessionConfig};

use crate::envelope::DirectEnvelope;
use crate::error::CryptoError;

pub fn decode_curve25519(b64: &str) -> Result<Curve25519PublicKey, CryptoError> {
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CryptoError::InvalidKey("curve25519 key must be 32 bytes".into()))?;
    Ok(Curve25519PublicKey::from_bytes(arr))
}

pub fn encode_curve25519(key: &Curve25519PublicKey) -> String {
    STANDARD.encode(key.as_bytes())
}

/// In-memory cache of active Olm sessions, keyed by peer Curve25519 key.
/// Persisting to the encrypted store is the caller's (`core`) job.
#[derive(Default)]
pub struct OlmManager {
    sessions: HashMap<String, Session>,
}

impl OlmManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_session_with(&self, peer_curve25519_b64: &str) -> bool {
        self.sessions.contains_key(peer_curve25519_b64)
    }

    /// Drops a cached session so the next send starts a fresh one (claiming
    /// a new one-time key) instead of reusing one the peer has rejected.
    /// Sessions aren't persisted across restarts (see this struct's own doc
    /// comment) — the peer's process restarting always leaves it unable to
    /// decrypt anything encrypted under the session we still have cached
    /// for them, which is exactly what a `ChatResponse { ack: false }` (see
    /// `ChatNode::handle_request_response_message`) means. Without this,
    /// every subsequent send keeps reusing the same now-dead session,
    /// keeps getting rejected, forever, with no way to recover short of the
    /// user manually removing and re-adding the contact.
    pub fn forget_session(&mut self, peer_curve25519_b64: &str) {
        self.sessions.remove(peer_curve25519_b64);
    }

    pub fn insert(&mut self, peer_curve25519_b64: &str, session: Session) {
        self.sessions
            .insert(peer_curve25519_b64.to_string(), session);
    }

    pub fn get(&self, peer_curve25519_b64: &str) -> Option<&Session> {
        self.sessions.get(peer_curve25519_b64)
    }

    /// Starts a brand-new outbound session with a peer we've never messaged
    /// before. `my_device_identity` must be this device's own Olm account,
    /// not the master identity — reusing it collides two devices onto one slot.
    pub fn start_outbound(
        &mut self,
        my_device_identity: &Identity,
        peer_curve25519_b64: &str,
        peer_one_time_key_b64: &str,
    ) -> Result<(), CryptoError> {
        let peer_identity_key = decode_curve25519(peer_curve25519_b64)?;
        let peer_otk = decode_curve25519(peer_one_time_key_b64)?;
        let session = my_device_identity.account().create_outbound_session(
            SessionConfig::version_1(),
            peer_identity_key,
            peer_otk,
        )?;
        self.sessions
            .insert(peer_curve25519_b64.to_string(), session);
        Ok(())
    }

    /// Encrypts `plaintext` for `peer_curve25519_b64`. The three `my_*`
    /// fields are only stamped on the envelope as routing hints (see
    /// `DirectEnvelope`'s doc comment); encryption itself needs just the session.
    pub fn encrypt(
        &mut self,
        my_user_id: &str,
        my_device_id: &str,
        my_curve25519_key_b64: &str,
        peer_curve25519_b64: &str,
        plaintext: &[u8],
    ) -> Result<DirectEnvelope, CryptoError> {
        let session = self
            .sessions
            .get_mut(peer_curve25519_b64)
            .ok_or(CryptoError::NoSession)?;
        let message = session.encrypt(plaintext)?;
        let (message_type, ciphertext) = message.to_parts();
        Ok(DirectEnvelope {
            sender_user_id: my_user_id.to_string(),
            sender_curve25519_key: my_curve25519_key_b64.to_string(),
            sender_device_id: my_device_id.to_string(),
            message_type: message_type as u8,
            ciphertext,
        })
    }

    /// Decrypts an inbound envelope, transparently establishing a new
    /// session for a pre-key message. `my_device_identity` must be this
    /// device's own Olm account, same reason as `start_outbound`.
    pub fn decrypt(
        &mut self,
        my_device_identity: &mut Identity,
        envelope: &DirectEnvelope,
    ) -> Result<Vec<u8>, CryptoError> {
        let peer_key = envelope.sender_curve25519_key.clone();
        let message = OlmMessage::from_parts(envelope.message_type as usize, &envelope.ciphertext)
            .map_err(|e| CryptoError::Decode(e.to_string()))?;

        if let Some(session) = self.sessions.get_mut(&peer_key) {
            return Ok(session.decrypt(&message)?);
        }

        // No existing session: this must be a pre-key message establishing a
        // brand-new one.
        let OlmMessage::PreKey(pre_key_message) = &message else {
            return Err(CryptoError::NotAPreKeyMessage);
        };
        let peer_identity_key = decode_curve25519(&peer_key)?;
        let result = my_device_identity.account_mut().create_inbound_session(
            SessionConfig::version_1(),
            peer_identity_key,
            pre_key_message,
        )?;
        self.sessions.insert(peer_key, result.session);
        Ok(result.plaintext)
    }
}
