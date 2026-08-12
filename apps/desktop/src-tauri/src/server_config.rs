/// Sentinel value (not a real URL) meaning "run the bundled local directory
/// server" rather than pointing at an external one. Kept as a plain string
/// so it can cross the Tauri IPC boundary as the same `String` type a real
/// URL would use, instead of needing a tagged enum on both sides.
pub const EMBEDDED_SENTINEL: &str = "embedded";

/// The compiled-in "Seal" network, if this build has one configured.
/// Resolution order: `P2P_CHAT_DIRECTORY_URL` (runtime override), then
/// `SEAL_DEFAULT_DIRECTORY_URL` (baked in via `apps/desktop/.cargo/config.toml`).
pub fn official_server_url() -> Option<String> {
    if let Ok(runtime_override) = std::env::var("P2P_CHAT_DIRECTORY_URL") {
        return Some(runtime_override);
    }
    option_env!("SEAL_DEFAULT_DIRECTORY_URL").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests run in parallel threads sharing process-global env vars.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        // Only passes when run with CWD outside apps/desktop (e.g. `cargo
        // test --workspace` from repo root, as CI does).
        assert_eq!(official_server_url(), None);
    }
}
