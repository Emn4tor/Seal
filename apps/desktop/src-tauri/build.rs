fn main() {
    // `official_server_url` bakes this in via `option_env!` — without this
    // line, Cargo won't rebuild when the env var changes.
    println!("cargo:rerun-if-env-changed=SEAL_DEFAULT_DIRECTORY_URL");
    tauri_build::build()
}
