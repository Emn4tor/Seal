use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::error::IdentityError;

const SERVICE: &str = "p2p-chat";
const KEK_USERNAME: &str = "local-encryption-key";

/// The key-encryption-key (KEK) that protects sensitive local database
/// columns, stored in the OS keychain. Deleting it makes panic-purge
/// instant: the ciphertext it protected becomes permanently unreadable.
pub struct Keychain {
    #[cfg(all(not(target_os = "macos"), not(target_os = "ios")))]
    entry: keyring::Entry,
    // `keyring::Entry` (the `v1` wrapper) hardcodes NoDefaultStore on iOS
    // regardless of what `ensure_ios_credential_store` registers below —
    // `keyring_core::Entry` has no such gate.
    #[cfg(target_os = "ios")]
    entry: keyring_core::Entry,
    #[cfg(target_os = "macos")]
    service: String,
    #[cfg(target_os = "macos")]
    account: String,
}

impl Keychain {
    /// Scoped to a data dir's hash so multiple local `AppService`s (e.g. in
    /// tests) never collide on one keychain entry.
    pub fn for_app_data_dir(data_dir: &std::path::Path) -> Result<Self, IdentityError> {
        let digest = Sha256::digest(data_dir.to_string_lossy().as_bytes());
        let username = format!("{KEK_USERNAME}-{}", hex::encode(&digest[..8]));
        Self::new(SERVICE, &username)
    }

    /// Also used directly by tests, against a throwaway service name.
    #[cfg(all(not(target_os = "macos"), not(target_os = "ios")))]
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        let entry = keyring::Entry::new(service, username)?;
        Ok(Self { entry })
    }

    #[cfg(target_os = "ios")]
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        ensure_ios_credential_store();
        let entry = keyring_core::Entry::new(service, username)?;
        Ok(Self { entry })
    }

    #[cfg(target_os = "macos")]
    pub fn new(service: &str, username: &str) -> Result<Self, IdentityError> {
        Ok(Self {
            service: service.to_string(),
            account: username.to_string(),
        })
    }

    /// Returns the existing KEK, or mints one on first run. On macOS this
    /// can block on a Touch ID/password prompt — run via `spawn_blocking`.
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

    /// Crypto-shred. Idempotent: deleting an absent entry isn't an error.
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

/// Registers iOS's "Protected Data" store as `keyring_core`'s default,
/// once per process. Without this every `keyring_core::Entry::new` fails
/// with NoDefaultStore.
#[cfg(target_os = "ios")]
fn ensure_ios_credential_store() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Ok(store) = apple_native_keyring_store::protected::Store::new() {
            keyring_core::set_default_store(store);
        }
    });
}

/// Uses `security-framework` directly instead of `keyring`, whose macOS
/// backend can't attach a `SecAccessControl` (no Touch ID gating).
#[cfg(target_os = "macos")]
mod macos {
    use security_framework::base::Error;
    use security_framework::passwords::{
        AccessControlOptions, PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    };
    use security_framework_sys::base::errSecItemNotFound;

    /// errSecInvalidOwnerEdit: deleting an item created under a different
    /// code signature (e.g. every unsigned dev rebuild). Treated as
    /// already-gone since purge shouldn't fail over this.
    const ERR_SEC_INVALID_OWNER_EDIT: i32 = -25244;

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, Error> {
        match generic_password(PasswordOptions::new_generic_password(service, account)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == errSecItemNotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// errSecMissingEntitlement: an unsigned binary can't get
    /// `SecAccessControl` at all (this repo's default build).
    const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

    /// errSecAuthFailed: the other error code macOS uses for the same
    /// unsigned-binary problem, seen to vary by OS version.
    const ERR_SEC_AUTH_FAILED: i32 = -25293;

    pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<(), Error> {
        let mut options = PasswordOptions::new_generic_password(service, account);
        // USER_PRESENCE covers Touch ID and password-only Macs alike.
        options.set_access_control_options(AccessControlOptions::USER_PRESENCE);
        match set_generic_password_options(secret, options) {
            // Falls back to an unprotected item on unsigned builds rather
            // than failing account creation outright.
            Err(e)
                if e.code() == ERR_SEC_MISSING_ENTITLEMENT || e.code() == ERR_SEC_AUTH_FAILED =>
            {
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
            Err(e) if e.code() == ERR_SEC_INVALID_OWNER_EDIT => Ok(()),
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

/// Best-effort scrub after use. Not a substitute for deleting the entry.
pub fn zeroize_kek(kek: &mut [u8; 32]) {
    kek.zeroize();
}
