mod account_manager;
mod accounts;
mod actor;
mod commands;
mod dto;
mod embedded_directory;
mod server_config;

use std::path::PathBuf;

use account_manager::AccountManager;
use tauri::Manager;

/// Filesystem locations resolved once at startup and needed before any
/// backend is running. `shared_data_dir` is where `server.json` and
/// `accounts.json` live; each account's own data lives under
/// `shared_data_dir/accounts/<account_id>/` (see the `accounts` module) —
/// there's no longer a single static "the" profile dir, since which account
/// is active isn't decided until after this is set up.
pub struct AppPaths {
    pub shared_data_dir: PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let shared_data_dir = app.path().app_data_dir()?;
            // `app_data_dir()` only resolves the conventional path — it
            // doesn't create it. Nothing else is guaranteed to before the
            // first write (e.g. saving `server.json`), so do it here.
            std::fs::create_dir_all(&shared_data_dir)?;

            app.manage(AppPaths { shared_data_dir });
            app.manage(AccountManager::new());

            // No backend and no account is started here anymore — the
            // frontend calls `get_saved_server_url`/`get_official_server_url`
            // then `start_backend`, then `resolve_boot_account` to decide
            // which account (if any) to load automatically.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_official_server_url,
            commands::get_saved_server_url,
            commands::start_backend,
            commands::save_server_url_for_next_launch,
            commands::resolve_boot_account,
            commands::list_accounts,
            commands::create_account,
            commands::resume_account,
            commands::rename_account,
            commands::remove_account,
            commands::add_contact,
            commands::list_contacts,
            commands::send_direct_message,
            commands::list_messages,
            commands::create_group,
            commands::invite_to_group,
            commands::create_channel,
            commands::send_group_message,
            commands::list_groups,
            commands::join_voice_channel,
            commands::leave_voice_channel,
            commands::set_voice_changer_enabled,
            commands::set_mic_muted,
            commands::get_voice_participants,
            commands::set_mic_threshold_db,
            commands::set_hear_self,
            commands::get_voice_speaking_participants,
            commands::panic_purge,
            commands::pick_attachment,
            commands::get_image_exif,
            commands::save_attachment,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
