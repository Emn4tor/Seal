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
///
/// On macOS this is backed by `keychain::macos`, gated behind Touch ID
/// (falling back to the device password); everywhere else it's the plain
/// cross-platform `keyring` crate, unbiometric. See that module's doc
/// comment for why macOS gets its own path instead of going through
/// `keyring` like the others.
pub struct Keychain {
    #[cfg(not(target_os = "macos"))]
    entry: keyring::Entry,
    #[cfg(target_os = "macos")]
    service: String,
    #[cfg(target_os = "macos")]
    account: String,
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
    #[cfg(not(target_os = "macos"))]
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        let entry = keyring::Entry::new(service, username)?;
        Ok(Self { entry })
    }

    #[cfg(target_os = "macos")]
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        Ok(Self {
            service: service.to_string(),
            account: username.to_string(),
        })
    }

    /// Returns the existing KEK, or generates and stores a new random one on
    /// first run. On macOS, reading (or, on first run, the OS confirming
    /// the newly-created item's protection) prompts for Touch ID/password —
    /// this is a blocking, interactive OS call, so callers on an async
    /// runtime should run it via `spawn_blocking` rather than inline.
    #[cfg(not(target_os = "macos"))]
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

    #[cfg(target_os = "macos")]
    pub fn load_or_create_kek(&self) -> Result<[u8; 32], IdentityError> {
        match macos::get(&self.service, &self.account)? {
            Some(existing) => decode_kek(&String::from_utf8_lossy(&existing)),
            None => {
                let mut kek = [0u8; 32];
                OsRng.fill_bytes(&mut kek);
                macos::set(
                    &self.service,
                    &self.account,
                    STANDARD.encode(kek).as_bytes(),
                )?;
                Ok(kek)
            }
        }
    }

    /// Crypto-shred: irrecoverably deletes the KEK. Idempotent — deleting an
    /// already-absent entry is not an error, since purge should never fail
    /// just because it (or part of it) already ran.
    #[cfg(not(target_os = "macos"))]
    pub fn delete_kek(&self) -> Result<(), IdentityError> {
        match self.entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn delete_kek(&self) -> Result<(), IdentityError> {
        macos::delete(&self.service, &self.account)?;
        Ok(())
    }
}

/// The macOS-specific keychain backend, used instead of the cross-platform
/// `keyring` crate. `keyring`'s macOS store goes through the legacy,
/// deprecated `SecKeychainAddGenericPassword` API, which has no way to
/// attach a `SecAccessControl` — there's no path to Touch ID through it.
/// This uses `security-framework`'s modern `SecItemAdd`/`SecItemCopyMatching`
/// wrappers instead (the same "regular login keychain" as `keyring`, not
/// the sandboxed "Protected Data" store — that one needs an entitlement and
/// provisioning profile this app doesn't have).
#[cfg(target_os = "macos")]
mod macos {
    use security_framework::base::Error;
    use security_framework::passwords::{
        AccessControlOptions, PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    };
    use security_framework_sys::base::errSecItemNotFound;

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, Error> {
        match generic_password(PasswordOptions::new_generic_password(service, account)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == errSecItemNotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Apple's SecBase.h `errSecMissingEntitlement` — not in this crate's
    /// (small, curated) constant list, so hardcoded here. Hit when the
    /// calling binary isn't signed with an identity the OS trusts enough to
    /// grant `SecAccessControl` — i.e. any unsigned build, which is this
    /// repo's default (`scripts/build-mac-dmg.sh`'s build is unsigned
    /// unless you configure your own signing identity; a plain `cargo
    /// build`/`cargo tauri dev` binary is never signed at all).
    const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

    pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<(), Error> {
        let mut options = PasswordOptions::new_generic_password(service, account);
        // Touch ID when available, falling back to the device login
        // password (`USER_PRESENCE`, not `BIOMETRY_*` alone) — this app
        // needs to keep working on Macs with no biometric hardware at all,
        // not just ones with Touch ID.
        options.set_access_control_options(AccessControlOptions::USER_PRESENCE);
        match set_generic_password_options(secret, options) {
            // Falls back to an unprotected item rather than making account
            // creation/login fail outright on an unsigned build — keeps
            // dev builds working exactly as they did before Touch ID
            // protection existed. Once the app is properly code-signed,
            // this stops triggering and the KEK gets the real gate.
            Err(e) if e.code() == ERR_SEC_MISSING_ENTITLEMENT => {
                let options = PasswordOptions::new_generic_password(service, account);
                set_generic_password_options(secret, options)
            }
            other => other,
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<(), Error> {
        match delete_generic_password_options(PasswordOptions::new_generic_password(
            service, account,
        )) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == errSecItemNotFound => Ok(()),
            Err(e) => Err(e),
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
