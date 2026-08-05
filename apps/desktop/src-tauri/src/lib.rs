mod account_manager;
mod accounts;
mod actor;
mod commands;
mod dto;
mod embedded_directory;
mod server_config;

use std::path::PathBuf;

use account_manager::AccountManager;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WindowEvent};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Filesystem locations resolved once at startup and needed before any
/// backend is running. `shared_data_dir` is where `server.json` and
/// `accounts.json` live; each account's own data lives under
/// `shared_data_dir/accounts/<account_id>/` (see the `accounts` module) —
/// there's no longer a single static "the" profile dir, since which account
/// is active isn't decided until after this is set up.
pub struct AppPaths {
    pub shared_data_dir: PathBuf,
}

/// Sets up logging to both stdout and a daily-rotating file under
/// `<app_data_dir>/logs/` — on by default at `info`, not only when
/// `RUST_LOG` happens to be set. Debugging a real report used to mean
/// asking someone to relaunch with `RUST_LOG=debug` and capture their
/// terminal, which two `npm run tauri dev` windows at once (see
/// `apps/desktop/scripts/tauri.mjs`) makes awkward — a file that's always
/// being written means the logs from whatever just happened are already on
/// disk. `RUST_LOG`, when set, still wins and applies to both outputs:
/// `EnvFilter::try_from_default_env` only falls back to the `info` default
/// when it's absent.
fn init_logging(
    shared_data_dir: &std::path::Path,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let env_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };

    let log_dir = shared_data_dir.join("logs");
    if std::fs::create_dir_all(&log_dir).is_err() {
        // A logging setup problem shouldn't stop the app from starting —
        // fall back to stdout only.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(env_filter())
            .try_init();
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(&log_dir, "seal.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let registered = tracing_subscriber::registry()
        .with(env_filter())
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking),
        )
        .try_init()
        .is_ok();

    if registered {
        tracing::info!(log_dir = %log_dir.display(), "logging to files in this directory (rotated daily)");
    }
    Some(guard)
}

/// Shows and focuses the main window — used by the tray icon menu's "Open
/// Seal" item (shown on both left- and right-click) and macOS's
/// Dock-icon-click ("reopen") event, both of which need to undo the same
/// hide-to-tray state.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let shared_data_dir = app.path().app_data_dir()?;
            // `app_data_dir()` only resolves the conventional path — it
            // doesn't create it. Nothing else is guaranteed to before the
            // first write (e.g. saving `server.json`), so do it here.
            std::fs::create_dir_all(&shared_data_dir)?;

            // Leaked deliberately: the guard needs to outlive the whole
            // process so buffered log lines actually get flushed to disk,
            // and there's no natural owner for it once `setup` returns and
            // `.build().run(...)` takes over.
            if let Some(guard) = init_logging(&shared_data_dir) {
                Box::leak(Box::new(guard));
            }

            app.manage(AppPaths { shared_data_dir });
            app.manage(AccountManager::new());

            // No backend and no account is started here anymore — the
            // frontend calls `get_saved_server_url`/`get_official_server_url`
            // then `start_backend`, then `resolve_boot_account` to decide
            // which account (if any) to load automatically.

            // Closing the main window hides it instead of quitting — Seal
            // keeps running in the tray/menu bar so messages still arrive
            // and notify. The app only actually exits via the tray menu's
            // "Quit Seal" or the OS's own quit shortcut (Cmd+Q), neither of
            // which goes through this window-level event.
            if let Some(window) = app.get_webview_window("main") {
                let window_to_hide = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_to_hide.hide();
                    }
                });
            }

            let show_item = MenuItem::with_id(app, "show", "Open Seal", true, None::<&str>)?;
            let toggle_mic_item =
                MenuItem::with_id(app, "toggle_mic", "Toggle Mic Mute", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Seal", true, None::<&str>)?;
            let tray_menu = Menu::with_items(
                app,
                &[
                    &show_item,
                    &toggle_mic_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_item,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                // Left-click showing the same menu as right-click (rather
                // than the old behavior of left-click reopening the window
                // directly) is the whole point — `Menu::with_items`+`.menu()`
                // already makes the OS show it on right-click for free, this
                // is what actually needed to change. "Open Seal" in the menu
                // covers the direct-reopen case that used to be left-click's
                // own hardcoded behavior.
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "toggle_mic" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Ok(handle) = app.state::<AccountManager>().current().await {
                                let _ = handle.toggle_mic_muted().await;
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

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
            commands::remove_contact,
            commands::list_contacts,
            commands::send_direct_message,
            commands::list_messages,
            commands::create_group,
            commands::invite_to_group,
            commands::remove_member_from_group,
            commands::leave_group,
            commands::create_channel,
            commands::send_group_message,
            commands::list_groups,
            commands::refresh_group,
            commands::join_voice_channel,
            commands::leave_voice_channel,
            commands::call_contact,
            commands::accept_call,
            commands::decline_call,
            commands::end_call,
            commands::list_input_devices,
            commands::list_output_devices,
            commands::set_voice_changer_enabled,
            commands::set_mic_muted,
            commands::get_mic_muted,
            commands::get_voice_participants,
            commands::get_channel_voice_participants,
            commands::set_mic_threshold_db,
            commands::set_hear_self,
            commands::get_voice_speaking_participants,
            commands::panic_purge,
            commands::pick_attachment,
            commands::get_image_exif,
            commands::save_attachment,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // macOS-specific: clicking the Dock icon while every window is
            // hidden (closed to tray) doesn't automatically reopen one —
            // this is the event that fires instead, so honor it the same
            // way as the tray menu's "Open Seal".
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app_handle);
            }
        });
}
