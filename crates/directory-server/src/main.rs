use std::net::SocketAddr;
use std::path::PathBuf;

use directory_server::admin::{AdminState, build_admin_router};
use directory_server::{AppState, build_public_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db_path: PathBuf = std::env::var("DIRECTORY_DB_PATH")
        .unwrap_or_else(|_| "directory.sqlite3".to_string())
        .into();
    let public_addr: SocketAddr = std::env::var("DIRECTORY_PUBLIC_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;
    let admin_addr: SocketAddr = std::env::var("DIRECTORY_ADMIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8090".to_string())
        .parse()?;
    let admin_token = std::env::var("DIRECTORY_ADMIN_TOKEN").map_err(|_| {
        anyhow::anyhow!(
            "DIRECTORY_ADMIN_TOKEN must be set — the admin purge endpoint refuses to start without it"
        )
    })?;

    if !admin_addr.ip().is_loopback() {
        tracing::warn!(
            %admin_addr,
            "admin endpoint is not bound to loopback — the purge endpoint is reachable over the \
             network; make sure it's protected (firewall/VPN/SSH tunnel)"
        );
    }

    let state = AppState::open(db_path)?;
    let public_router = build_public_router(state.clone());
    let admin_router = build_admin_router(AdminState {
        app: state,
        token: admin_token,
    });

    let public_listener = tokio::net::TcpListener::bind(public_addr).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;

    tracing::info!(%public_addr, "public directory API listening");
    tracing::info!(%admin_addr, "admin API listening");

    let public_server =
        axum::serve(public_listener, public_router).with_graceful_shutdown(shutdown_signal());
    let admin_server =
        axum::serve(admin_listener, admin_router).with_graceful_shutdown(shutdown_signal());

    tokio::try_join!(public_server, admin_server)?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
