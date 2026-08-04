use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use directory_server::admin::{AdminState, build_admin_router};
use directory_server::{AppState, build_public_router};
use wire_proto::RelayInfoResponse;

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

    // NAT-traversal relay (Circuit Relay v2 + dcutr): a client behind a NAT
    // that can't be dialed directly reserves a circuit here, another client
    // connects to it through that circuit, and `dcutr` (client-side, in
    // both peers) then tries to upgrade to a direct connection — once that
    // succeeds, this process is out of the conversation's path entirely. See
    // `relay.rs` for why it's a separate identity from anything HTTP/chat.
    let relay_key_path: PathBuf = std::env::var("DIRECTORY_RELAY_KEY_PATH")
        .unwrap_or_else(|_| "relay-identity.key".to_string())
        .into();
    let relay_bind_addr: SocketAddr = std::env::var("DIRECTORY_RELAY_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:4001".to_string())
        .parse()?;
    // e.g. "/dns4/seal.emn4tor.de/tcp/4001" — the base multiaddr clients
    // should dial, *without* the trailing `/p2p/<peer-id>` (this appends
    // that itself, since only this process knows the peer id ahead of a
    // request). Left unset, the relay still runs (useful for same-host/dev
    // testing) but isn't advertised: better to advertise nothing than an
    // address nobody outside this machine can actually reach.
    let relay_external_base = std::env::var("DIRECTORY_RELAY_EXTERNAL_MULTIADDR").ok();

    let relay_keypair = directory_server::relay::load_or_create_keypair(&relay_key_path)?;
    let relay_peer_id = relay_keypair.public().to_peer_id();
    let relay_info = relay_external_base.map(|base| RelayInfoResponse {
        peer_id: relay_peer_id.to_string(),
        multiaddr: format!("{base}/p2p/{relay_peer_id}"),
    });
    if relay_info.is_none() {
        tracing::warn!(
            "DIRECTORY_RELAY_EXTERNAL_MULTIADDR not set — the relay will run but won't be \
             advertised via /v1/relay-info; clients behind a NAT won't be able to reach each \
             other until it's configured"
        );
    }

    tokio::spawn({
        let relay_bind_addr = relay_bind_addr;
        async move {
            if let Err(e) = directory_server::relay::run_relay(relay_keypair, relay_bind_addr).await {
                tracing::error!(error = %e, "relay swarm exited");
            }
        }
    });

    let state = AppState::open(db_path)?.with_relay_info(relay_info);

    // Belt-and-suspenders alongside the opportunistic sweep on every
    // presence write (see `db::sweep_expired_presence`): keeps a fully idle
    // server (nobody currently heartbeating) from letting old presence rows
    // — a peer_id <-> IP/relay-circuit history — just sit in the database
    // indefinitely once their TTL lapses.
    tokio::spawn({
        let state = state.clone();
        async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(300));
            loop {
                ticker.tick().await;
                let now = directory_server::state::now_secs();
                if let Err(e) = state
                    .with_conn(move |conn| directory_server::db::sweep_expired_presence(conn, now))
                    .await
                {
                    tracing::warn!(error = %e, "periodic presence sweep failed");
                }
            }
        }
    });

    let public_router = build_public_router(state.clone());
    let admin_router = build_admin_router(AdminState {
        app: state,
        token: admin_token,
    });

    let public_listener = tokio::net::TcpListener::bind(public_addr).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;

    tracing::info!(%public_addr, "public directory API listening");
    tracing::info!(%admin_addr, "admin API listening");
    tracing::info!(%relay_bind_addr, %relay_peer_id, "relay listening");

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
