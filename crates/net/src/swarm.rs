use libp2p::identity::Keypair;
use libp2p::{Swarm, SwarmBuilder, noise, yamux};

use crate::behaviour::{ChatBehaviour, build_behaviour};

/// Builds a fully-wired swarm: QUIC/TCP transport plus a relay-client for
/// NAT traversal. Uses `with_dns_config` with hardcoded resolvers, not
/// `.with_dns()` — that reads `/etc/resolv.conf`, inaccessible on sandboxed iOS/Android.
pub fn build_swarm(keypair: Keypair) -> anyhow::Result<Swarm<ChatBehaviour>> {
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            Default::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns_config(
            libp2p::dns::ResolverConfig::default(),
            libp2p::dns::ResolverOpts::default(),
        )
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(build_behaviour)?
        .build();
    Ok(swarm)
}

pub fn build_swarm_with_new_identity() -> anyhow::Result<Swarm<ChatBehaviour>> {
    build_swarm(Keypair::generate_ed25519())
}
