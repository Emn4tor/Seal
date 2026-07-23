use std::path::PathBuf;

use rand::distributions::Alphanumeric;
use rand::Rng;

/// Fixed local ports for the embedded directory server. Deliberately fixed
/// (not ephemeral) so that a *second* instance of this app running on the
/// same machine finds the *first* instance's server already bound and just
/// reuses it, rather than each instance spinning up its own — matching the
/// real deployment topology of "one shared directory, many clients."
const PUBLIC_PORT: u16 = 47100;
const ADMIN_PORT: u16 = 47101;

/// Starts an embedded directory-server bound to loopback, or — if the port
/// is already taken, meaning another local instance is already hosting one
/// — does nothing and just returns the URL to reuse it.
pub async fn ensure_running(shared_data_dir: PathBuf) -> String {
    let base_url = format!("http://127.0.0.1:{PUBLIC_PORT}");

    let public_listener = match tokio::net::TcpListener::bind(("127.0.0.1", PUBLIC_PORT)).await {
        Ok(listener) => listener,
        Err(_) => {
            tracing::info!("directory server already running locally; reusing it");
            return base_url;
        }
    };
    let admin_listener = match tokio::net::TcpListener::bind(("127.0.0.1", ADMIN_PORT)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(error = %e, "public directory port was free but the admin port wasn't");
            return base_url;
        }
    };

    let db_path = shared_data_dir
        .join("directory-server")
        .join("directory.sqlite3");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let state = match directory_server::AppState::open(db_path) {
        Ok(state) => state,
        Err(e) => {
            tracing::error!(error = %e, "failed to open embedded directory-server database");
            return base_url;
        }
    };

    let public_router = directory_server::build_public_router(state.clone());
    // A fresh random admin token per process: nothing in this app's own UI
    // exposes the directory-wide purge (that's an operator action via the
    // `directory-admin` CLI against a real deployment) — this embedded
    // instance existing at all is purely a same-machine testing/dev
    // convenience, not the intended production topology.
    let admin_token: String = rand::thread_rng()
        .sample_iter(Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let admin_router =
        directory_server::admin::build_admin_router(directory_server::admin::AdminState {
            app: state,
            token: admin_token,
        });

    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(public_listener, public_router).await {
            tracing::error!(error = %e, "embedded directory-server public listener stopped");
        }
    });
    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(admin_listener, admin_router).await {
            tracing::error!(error = %e, "embedded directory-server admin listener stopped");
        }
    });

    tracing::info!(%base_url, "embedded directory server started");
    base_url
}
