use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use vodozemac::olm::{Account, AccountPickle};

use crate::error::IdentityError;

/// A user's long-term chat identity: an Olm `Account` (Ed25519 signing key +
/// Curve25519 agreement key), independent of any libp2p transport identity.
pub struct Identity {
    account: Account,
}

impl Identity {
    pub fn generate() -> Self {
        Self {
            account: Account::new(),
        }
    }

    pub fn from_pickle(pickle: AccountPickle) -> Self {
        Self {
            account: Account::from_pickle(pickle),
        }
    }

    pub fn pickle(&self) -> AccountPickle {
        self.account.pickle()
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    pub fn account_mut(&mut self) -> &mut Account {
        &mut self.account
    }

    pub fn ed25519_public_base64(&self) -> String {
        STANDARD.encode(self.account.identity_keys().ed25519.as_bytes())
    }

    pub fn curve25519_public_base64(&self) -> String {
        STANDARD.encode(self.account.identity_keys().curve25519.as_bytes())
    }

    /// The user's public identity, derived from their Ed25519 identity key —
    /// this is the same fingerprint the directory server verifies signed
    /// requests against.
    pub fn user_id(&self) -> String {
        wire_proto::user_id_from_ed25519(self.account.identity_keys().ed25519.as_bytes())
    }

    /// Signs `message`, returning a standard-base64-encoded signature —
    /// deliberately *not* `vodozemac`'s own `to_base64()` (which uses an
    /// unpadded matrix-spec variant), so the output is byte-for-byte what
    /// `directory-server`'s `auth::verify_signature` expects to decode.
    pub fn sign(&self, message: &[u8]) -> String {
        STANDARD.encode(self.account.sign(message).to_bytes())
    }

    /// Client-side counterpart to `sign`. Takes standard-base64 for both
    /// arguments, matching `sign`'s encoding — *not* vodozemac's own
    /// unpadded `to_base64()`, which silently fails to decode if mixed.
    pub fn verify(
        pubkey_b64: &str,
        message: &[u8],
        signature_b64: &str,
    ) -> Result<(), IdentityError> {
        let pubkey_bytes: [u8; 32] = STANDARD
            .decode(pubkey_b64)
            .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))?
            .try_into()
            .map_err(|_| {
                IdentityError::InvalidKeyMaterial("ed25519 key must be 32 bytes".into())
            })?;
        let sig_bytes = STANDARD
            .decode(signature_b64)
            .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))?;
        let pubkey = vodozemac::Ed25519PublicKey::from_slice(&pubkey_bytes)
            .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))?;
        let sig = vodozemac::Ed25519Signature::from_slice(&sig_bytes)
            .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))?;
        pubkey
            .verify(message, &sig)
            .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))
    }
}

impl Identity {
    pub fn pickle_to_json(&self) -> Result<String, IdentityError> {
        Ok(serde_json::to_string(&self.pickle())?)
    }

    pub fn from_pickle_json(json: &str) -> Result<Self, IdentityError> {
        let pickle: AccountPickle = serde_json::from_str(json)?;
        Ok(Self::from_pickle(pickle))
    }
}
