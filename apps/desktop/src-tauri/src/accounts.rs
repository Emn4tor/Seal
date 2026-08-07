use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// One identity that lives on this device. `account_id` is a local,
/// arbitrary folder name chosen at creation time, deliberately *not*
/// `user_id`, since the user_id is only known once `AppService::load_or_create`
/// has actually generated the identity, and this needs to be decided first
/// so a data dir exists to generate it into.
#[derive(Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    pub account_id: String,
    pub user_id: String,
    pub display_name: String,
    pub created_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct AccountsFile {
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<AccountEntry>,
}

/// What to do at startup, decided here (not in the frontend) since the one
/// env var that can short-circuit it (`P2P_CHAT_PROFILE`) is a backend
/// concept. See each variant for when it applies.
pub enum BootDecision {
    /// No accounts exist on this device yet.
    NeedsFirstAccount,
    /// Accounts exist but none is unambiguously "the one" (e.g. the active
    /// account was just removed) let user picj new one
    NeedsPicker(Vec<AccountEntry>),
    /// Load this account with no prompt at all, the common case, and what
    /// makes launching Seal again "just log you in."
    Resume(AccountEntry),
    /// `P2P_CHAT_PROFILE` was set but didn't match an existing account's
    /// display name, create one non-interactively with that name.
    CreateWithName(String),
}

fn index_path(shared_data_dir: &Path) -> PathBuf {
    shared_data_dir.join("accounts.json")
}

pub fn load(shared_data_dir: &Path) -> AccountsFile {
    std::fs::read_to_string(index_path(shared_data_dir))
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Guards every `load` -> mutate -> `save` sequence against another Tauri
/// command doing the same thing at the same time. Without this, two
/// commands racing (e.g. a doubly-fired "create account" click before the
/// button disables) could each load the same pre-mutation snapshot, and
/// whichever saves last silently overwrites the other's change, even
/// though both changes exist on disk (a fresh keychain entry, an identity
/// directory), just not both recorded in `accounts.json`. One process-wide
/// lock is enough since it's this one JSON file for this one app process,
/// not a resource shared across processes.
#[derive(Default)]
pub struct AccountsFileLock(std::sync::Mutex<()>);

/// Runs `f` against the current `AccountsFile`, then persists whatever `f`
/// left it as. The only way any command in this module should read or
/// write `accounts.json`, so every such sequence is implicitly serialized
/// through `lock`.
pub fn update<T>(
    shared_data_dir: &Path,
    lock: &AccountsFileLock,
    f: impl FnOnce(&mut AccountsFile) -> T,
) -> std::io::Result<T> {
    let _guard = lock
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut file = load(shared_data_dir);
    let result = f(&mut file);
    save(shared_data_dir, &file)?;
    Ok(result)
}

pub fn save(shared_data_dir: &Path, file: &AccountsFile) -> std::io::Result<()> {
    std::fs::create_dir_all(shared_data_dir)?;
    let data = serde_json::to_string_pretty(file)?;
    std::fs::write(index_path(shared_data_dir), data)
}

pub fn account_dir(shared_data_dir: &Path, account_id: &str) -> PathBuf {
    shared_data_dir.join("accounts").join(account_id)
}

pub fn new_account_id() -> String {
    rand::thread_rng()
        .sample_iter(Alphanumeric)
        .take(16)
        .map(char::from)
        .collect()
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

pub fn resolve_boot(shared_data_dir: &Path) -> BootDecision {
    let file = load(shared_data_dir);

    if let Ok(profile) = std::env::var("P2P_CHAT_PROFILE") {
        return match file
            .accounts
            .iter()
            .find(|a| a.display_name.eq_ignore_ascii_case(&profile))
        {
            Some(existing) => BootDecision::Resume(existing.clone()),
            None => BootDecision::CreateWithName(profile),
        };
    }

    if file.accounts.is_empty() {
        return BootDecision::NeedsFirstAccount;
    }

    if let Some(active) = file
        .active_account_id
        .as_ref()
        .and_then(|id| file.accounts.iter().find(|a| &a.account_id == id))
    {
        return BootDecision::Resume(active.clone());
    }

    BootDecision::NeedsPicker(file.accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(account_id: &str, display_name: &str) -> AccountEntry {
        AccountEntry {
            account_id: account_id.to_string(),
            user_id: format!("user-{account_id}"),
            display_name: display_name.to_string(),
            created_at: 0,
        }
    }

    // These tests all set/remove P2P_CHAT_PROFILE, which is process-global
    // state, run serially against distinct temp dirs isn't enough on its
    // own, so each test cleans up after itself and none run concurrently by
    // virtue of being in the same file with default test harness settings...
    // to be safe, use a mutex so env var mutation never races across tests.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn no_accounts_needs_first_account() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("P2P_CHAT_PROFILE");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            resolve_boot(dir.path()),
            BootDecision::NeedsFirstAccount
        ));
    }

    #[test]
    fn active_account_resumes_silently() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("P2P_CHAT_PROFILE");
        }
        let dir = tempfile::tempdir().unwrap();
        let file = AccountsFile {
            active_account_id: Some("a1".to_string()),
            accounts: vec![entry("a1", "alice"), entry("a2", "bob")],
        };
        save(dir.path(), &file).unwrap();

        match resolve_boot(dir.path()) {
            BootDecision::Resume(a) => assert_eq!(a.account_id, "a1"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn no_active_account_needs_picker() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("P2P_CHAT_PROFILE");
        }
        let dir = tempfile::tempdir().unwrap();
        let file = AccountsFile {
            active_account_id: None,
            accounts: vec![entry("a1", "alice")],
        };
        save(dir.path(), &file).unwrap();

        match resolve_boot(dir.path()) {
            BootDecision::NeedsPicker(accounts) => assert_eq!(accounts.len(), 1),
            _ => panic!("expected NeedsPicker"),
        }
    }

    #[test]
    fn profile_env_var_matches_existing_account_by_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = AccountsFile {
            active_account_id: None,
            accounts: vec![entry("a1", "alice"), entry("a2", "bob")],
        };
        save(dir.path(), &file).unwrap();

        unsafe {
            std::env::set_var("P2P_CHAT_PROFILE", "bob");
        }
        let result = resolve_boot(dir.path());
        unsafe {
            std::env::remove_var("P2P_CHAT_PROFILE");
        }
        match result {
            BootDecision::Resume(a) => assert_eq!(a.account_id, "a2"),
            _ => panic!("expected Resume"),
        }
    }

    #[test]
    fn profile_env_var_with_no_match_creates_with_that_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();

        unsafe {
            std::env::set_var("P2P_CHAT_PROFILE", "carol");
        }
        let result = resolve_boot(dir.path());
        unsafe {
            std::env::remove_var("P2P_CHAT_PROFILE");
        }
        match result {
            BootDecision::CreateWithName(name) => assert_eq!(name, "carol"),
            _ => panic!("expected CreateWithName"),
        }
    }
}
