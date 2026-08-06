use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use futures::StreamExt;
use libp2p::identity::Keypair;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{Multiaddr, SwarmBuilder, identify, noise, relay, yamux};

const IDENTIFY_PROTOCOL_VERSION: &str = "/p2p-chat/relay-id/1";

/// Server side of NAT traversal only — brokers a circuit reservation for a
/// peer nobody can dial directly, relays opaque (already end-to-end
/// encrypted) bytes between two peer ids while a connection uses that
/// circuit, and otherwise never looks at what's flowing through it. No chat
/// protocols, no gossipsub, no request/response: this process has no way to
/// decrypt or even parse anything it relays. `identify` is included because
/// `relay::client` on the other end relies on it to learn its own
/// externally-dialable `/p2p-circuit` address once a reservation is granted.
#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    identify: identify::Behaviour,
}

/// Loads the relay's persisted libp2p identity, generating and saving a
/// fresh one on first run. This has to survive restarts: every client that
/// has ever reserved a circuit through this relay cached a multiaddr
/// containing this peer id, and a changed id would silently strand all of
/// them until they re-fetch `/v1/relay-info`. Deliberately separate from any
/// chat/HTTP identity this server has — this key only ever signs libp2p
/// transport handshakes, nothing about a specific user or message.
pub fn load_or_create_keypair(path: &Path) -> anyhow::Result<Keypair> {
    if let Ok(bytes) = std::fs::read(path) {
        return Keypair::from_protobuf_encoding(&bytes)
            .map_err(|e| anyhow::anyhow!("stored relay identity at {path:?} is corrupt: {e}"));
    }
    let keypair = Keypair::generate_ed25519();
    let encoded = keypair.to_protobuf_encoding()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(&encoded)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(keypair)
}

/// Runs the relay swarm until the process exits (or the first fatal
/// error — callers should treat that as fatal to the whole process).
/// Purely in-memory: nothing here is written to disk, so there's nothing
/// for a seized/leaked copy of the database to contain (see `todo.md`).
///
/// `external_addr`, when given, is the same base address advertised via
/// `/v1/relay-info` (e.g. `/dns4/seal.emn4tor.de/tcp/4001`). Advertising it
/// over HTTP is only half the story: `relay::Behaviour` fills a RESERVE
/// response's `addrs` from the swarm's own confirmed external addresses,
/// which is empty by default. Without registering this address on the
/// swarm too, every reservation comes back with no usable address, and
/// the client rejects it (`NoAddressesInReservation`) even though the
/// relay otherwise looked reachable.
pub async fn run_relay(
    keypair: Keypair,
    listen_addr: SocketAddr,
    external_addr: Option<Multiaddr>,
) -> anyhow::Result<()> {
    let local_peer_id = keypair.public().to_peer_id();
    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            Default::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|kp| RelayBehaviour {
            relay: relay::Behaviour::new(local_peer_id, relay::Config::default()),
            identify: identify::Behaviour::new(identify::Config::new(
                IDENTIFY_PROTOCOL_VERSION.to_string(),
                kp.public(),
            )),
        })?
        .build();

    let multiaddr: Multiaddr = format!("/ip4/{}/tcp/{}", listen_addr.ip(), listen_addr.port())
        .parse()
        .expect("SocketAddr always converts to a valid /ip4|ip6/.../tcp/... multiaddr");
    swarm.listen_on(multiaddr)?;

    if let Some(addr) = external_addr.clone() {
        swarm.add_external_address(addr);
    } else {
        tracing::warn!(
            "no external relay address configured — reservations will fail with \
             NoAddressesInReservation; set DIRECTORY_RELAY_EXTERNAL_MULTIADDR"
        );
    }

    tracing::info!(%local_peer_id, %listen_addr, ?external_addr, "relay listening");

    loop {
        // Nothing here needs the events themselves — `relay::Behaviour`
        // handles reservation/circuit bookkeeping internally. This just
        // has to keep polling the swarm to drive it.
        swarm.select_next_some().await;
    }
}
