use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::error::IdentityError;

const SERVICE: &str = "p2p-chat";
const KEK_USERNAME: &str = "local-encryption-key";

/// Holds the single key-encryption-key (KEK) that wraps the local SQLCipher
/// database, stored in the OS keychain rather than on disk. Deleting it is
/// what makes the local panic-purge instant and irrecoverable: without it,
/// the encrypted database is permanently unreadable ciphertext, regardless
/// of whether the file itself is ever deleted.
pub struct Keychain {
    entry: keyring::Entry,
}

impl Keychain {
    /// The app's real KEK entry, scoped to a specific app-data directory
    /// rather than one process-wide fixed name. In normal use there's
    /// exactly one data directory per OS user account, so this behaves
    /// like a single stable per-device identity — restarting the app with
    /// the same data dir reliably finds the same KEK. It also means
    /// multiple independent instances (e.g. two `AppService`s in the same
    /// test process, each with their own temp data dir) never collide on
    /// one keychain entry, which would otherwise mean two different local
    /// identities decrypting their local databases with the *same* key.
    pub fn for_app_data_dir(data_dir: &std::path::Path) -> Result<Self, IdentityError> {
        let digest = Sha256::digest(data_dir.to_string_lossy().as_bytes());
        let username = format!("{KEK_USERNAME}-{}", hex::encode(&digest[..8]));
        Self::new(SERVICE, &username)
    }

    /// Opens an arbitrary keychain entry — used directly by tests so they
    /// exercise the real OS keychain without touching the app's actual KEK.
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        let entry = keyring::Entry::new(service, username)?;
        Ok(Self { entry })
    }

    /// Returns the existing KEK, or generates and stores a new random one on
    /// first run.
    pub fn load_or_create_kek(&self) -> Result<[u8; 32], IdentityError> {
        match self.entry.get_password() {
            Ok(existing) => decode_kek(&existing),
            Err(keyring::Error::NoEntry) => {
                let mut kek = [0u8; 32];
                OsRng.fill_bytes(&mut kek);
                self.entry.set_password(&STANDARD.encode(kek))?;
                Ok(kek)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Crypto-shred: irrecoverably deletes the KEK. Idempotent — deleting an
    /// already-absent entry is not an error, since purge should never fail
    /// just because it (or part of it) already ran.
    pub fn delete_kek(&self) -> Result<(), IdentityError> {
        match self.entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn decode_kek(b64: &str) -> Result<[u8; 32], IdentityError> {
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| IdentityError::InvalidKeyMaterial(e.to_string()))?;
    let mut arr = [0u8; 32];
    if bytes.len() != 32 {
        return Err(IdentityError::InvalidKeyMaterial(
            "stored KEK is not 32 bytes".into(),
        ));
    }
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Best-effort scrub of a KEK buffer once it's no longer needed (e.g. after
/// deriving the SQLCipher key from it). Not a substitute for deleting the
/// keychain entry itself.
pub fn zeroize_kek(kek: &mut [u8; 32]) {
    kek.zeroize();
}
