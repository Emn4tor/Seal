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

/// In-memory cache of active Olm sessions, one per peer, keyed by the
/// peer's base64 Curve25519 identity key. Persisting sessions to the local
/// encrypted store (so ratchet state survives restarts) is the caller's
/// (`core`) responsibility — this manager only holds what's currently
/// loaded.
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

    pub fn insert(&mut self, peer_curve25519_b64: &str, session: Session) {
        self.sessions
            .insert(peer_curve25519_b64.to_string(), session);
    }

    pub fn get(&self, peer_curve25519_b64: &str) -> Option<&Session> {
        self.sessions.get(peer_curve25519_b64)
    }

    /// Starts a brand-new outbound session with a peer we've never messaged
    /// before, using their identity key and a freshly claimed one-time key
    /// (from `GET /v1/users/:id/otk/claim` on the directory server).
    pub fn start_outbound(
        &mut self,
        my_identity: &Identity,
        peer_curve25519_b64: &str,
        peer_one_time_key_b64: &str,
    ) -> Result<(), CryptoError> {
        let peer_identity_key = decode_curve25519(peer_curve25519_b64)?;
        let peer_otk = decode_curve25519(peer_one_time_key_b64)?;
        let session = my_identity.account().create_outbound_session(
            SessionConfig::version_1(),
            peer_identity_key,
            peer_otk,
        )?;
        self.sessions
            .insert(peer_curve25519_b64.to_string(), session);
        Ok(())
    }

    /// Encrypts `plaintext` for `peer_curve25519_b64` into the outer wire
    /// envelope, ready to hand to the transport layer. Requires an existing
    /// session (via `start_outbound`, or one implicitly created by a
    /// previously received pre-key message).
    pub fn encrypt(
        &mut self,
        my_identity: &Identity,
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
            sender_user_id: my_identity.user_id(),
            sender_curve25519_key: my_identity.curve25519_public_base64(),
            message_type: message_type as u8,
            ciphertext,
        })
    }

    /// Decrypts an inbound envelope, transparently establishing a new
    /// session if this is a pre-key (session-opening) message. Returns the
    /// decrypted plaintext.
    pub fn decrypt(
        &mut self,
        my_identity: &mut Identity,
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
        let result = my_identity.account_mut().create_inbound_session(
            SessionConfig::version_1(),
            peer_identity_key,
            pre_key_message,
        )?;
        self.sessions.insert(peer_key, result.session);
        Ok(result.plaintext)
    }
}
