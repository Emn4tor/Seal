use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Sentinel value (not a real URL) meaning "run the bundled local directory
/// server" rather than pointing at an external one. Kept as a plain string
/// so it can cross the Tauri IPC boundary as the same `String` type a real
/// URL would use, instead of needing a tagged enum on both sides.
pub const EMBEDDED_SENTINEL: &str = "embedded";

#[derive(Serialize, Deserialize)]
struct ServerConfigFile {
    directory_url: String,
}

fn config_path(shared_data_dir: &Path) -> PathBuf {
    shared_data_dir.join("server.json")
}

/// The server the user picked on a previous run, if any — read on startup
/// so returning users aren't asked again.
pub fn load_saved(shared_data_dir: &Path) -> Option<String> {
    let data = std::fs::read_to_string(config_path(shared_data_dir)).ok()?;
    let parsed: ServerConfigFile = serde_json::from_str(&data).ok()?;
    Some(parsed.directory_url)
}

pub fn save(shared_data_dir: &Path, directory_url: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(shared_data_dir)?;
    let data = serde_json::to_string_pretty(&ServerConfigFile {
        directory_url: directory_url.to_string(),
    })?;
    std::fs::write(config_path(shared_data_dir), data)
}

/// The compiled-in "Seal" network, if this build actually has one
/// configured — genuinely `None` rather than falling back to the embedded
/// sentinel, so the frontend can tell "no official server set up yet"
/// apart from "deliberately chose the local test server."
///
/// Resolution order:
/// 1. `P2P_CHAT_DIRECTORY_URL` — a runtime override for local dev/testing.
/// 2. `SEAL_DEFAULT_DIRECTORY_URL` — a build-time value, baked in when
///    compiling a real release. Unset in ordinary dev builds.
pub fn official_server_url() -> Option<String> {
    if let Ok(runtime_override) = std::env::var("P2P_CHAT_DIRECTORY_URL") {
        return Some(runtime_override);
    }
    option_env!("SEAL_DEFAULT_DIRECTORY_URL").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `cargo test` runs tests in parallel threads within one process, and
    // env vars are process-global — without this, the two tests below race
    // on `P2P_CHAT_DIRECTORY_URL` and fail intermittently (same hazard as
    // the `P2P_CHAT_PROFILE` tests in `accounts.rs`).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn save_creates_missing_data_dir() {
        // Regression test: on a fresh install, Tauri's app_data_dir() is
        // only a resolved path, not a directory that exists yet. `save`
        // used to `fs::write` straight into it and fail with ENOENT the
        // first time anyone picked a server.
        let dir =
            std::env::temp_dir().join(format!("seal-server-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.exists());

        save(&dir, "https://directory.example.com").expect("save should create the dir itself");
        assert_eq!(
            load_saved(&dir).as_deref(),
            Some("https://directory.example.com")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn official_server_url_runtime_override_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("P2P_CHAT_DIRECTORY_URL", "https://override.example.com");
        }
        let result = official_server_url();
        unsafe {
            std::env::remove_var("P2P_CHAT_DIRECTORY_URL");
        }
        assert_eq!(result.as_deref(), Some("https://override.example.com"));
    }

    #[test]
    fn official_server_url_is_none_without_any_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("P2P_CHAT_DIRECTORY_URL");
        }
        // This repo's own build doesn't bake in SEAL_DEFAULT_DIRECTORY_URL,
        // so with no runtime override either, there's genuinely no official
        // server configured — that's the whole point of this function.
        assert_eq!(official_server_url(), None);
    }
}
