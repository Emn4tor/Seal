mod account_manager;
mod accounts;
mod actor;
mod commands;
mod dto;
mod embedded_directory;
mod server_config;

use std::path::PathBuf;

use account_manager::AccountManager;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::WindowEvent;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Filesystem locations resolved once at startup. `shared_data_dir` is
/// where `accounts.json` lives; each account's own data lives under
/// `shared_data_dir/accounts/<account_id>/` (see the `accounts` module).
pub struct AppPaths {
    pub shared_data_dir: PathBuf,
}

/// Sets up logging to stdout and a daily-rotating file under
/// `<app_data_dir>/logs/`, on by default at `info`. `RUST_LOG`, when set,
/// still wins for both outputs.
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
/// hide-to-tray state. Desktop-only: no tray on mobile.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Builds the tray icon/menu and wires the close button to hide the
/// window instead of quitting. Desktop-only: mobile owns its own app
/// lifecycle (backgrounding, not a close event).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn setup_desktop_tray(app: &tauri::App) -> tauri::Result<()> {
    // Closing the main window hides it instead of quitting — Seal keeps
    // running in the tray so it can still receive and notify.
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

    // `default_window_icon()` can be `None` with no icon resource bundled;
    // degrade to a plain tray icon rather than failing to launch.
    let mut tray_builder = TrayIconBuilder::new()
        .menu(&tray_menu)
        // Left-click shows the same menu as right-click now, instead of
        // reopening directly; "Open Seal" in the menu covers that case.
        .show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }
    tray_builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "toggle_mic" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    match app.state::<AccountManager>().current().await {
                        Ok(handle) => {
                            let _ = handle.toggle_mic_muted().await;
                        }
                        // No account loaded, so no call to mute;
                        // show the window instead of doing nothing.
                        Err(_) => show_main_window(&app),
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // Process/updater plugins are desktop-only; mobile updates go through
    // the App/Play Store instead.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_process::init());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    // Push-to-talk is a global shortcut; mobile has no such API. An
    // on-screen hold-to-talk control is later work.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());

    // Launch-at-login is a desktop OS concept; mobile apps don't autostart
    // themselves.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));

    let builder = builder.plugin(tauri_plugin_notification::init());

    // QR-pairing's camera scanner — the upstream plugin only ships an
    // Android/iOS implementation, no desktop backend.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
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
            app.manage(accounts::AccountsFileLock::default());

            // No account loaded here; the frontend calls
            // `resolve_boot_account` then `create_account`/`resume_account`.

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            setup_desktop_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_official_server_url,
            commands::is_mobile,
            commands::resolve_boot_account,
            commands::list_accounts,
            commands::create_account,
            commands::resume_account,
            commands::set_account_directory_server,
            commands::join_via_pairing,
            commands::rename_account,
            commands::remove_account,
            commands::start_pairing,
            commands::list_my_devices,
            commands::sync_with_device,
            commands::add_contact,
            commands::remove_contact,
            commands::block_contact,
            commands::unblock_contact,
            commands::is_contact_blocked,
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
            commands::set_share_online_status,
            commands::get_contacts_online_status,
            commands::panic_purge,
            commands::pick_attachment,
            commands::get_image_exif,
            commands::save_attachment,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // macOS-specific: clicking the Dock icon while every window is
            // hidden (closed to tray) doesn't automatically reopen one —
            // this is the event that fires instead, so honor it the same
            // way as the tray menu's "Open Seal". Both closure params are
            // prefixed `_` since they're otherwise completely unused on
            // every other platform, once this `#[cfg]` block compiles out —
            // `-D warnings` in CI turns that into a hard error, not just a
            // local warning.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main_window(_app_handle);
            }
        });
}
