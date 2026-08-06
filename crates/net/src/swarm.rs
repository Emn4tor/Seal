use libp2p::identity::Keypair;
use libp2p::{Swarm, SwarmBuilder, noise, yamux};

use crate::behaviour::{ChatBehaviour, build_behaviour};

/// Builds a fully-wired swarm: QUIC primary / TCP+Noise+Yamux fallback
/// transport, plus a relay-client transport for NAT traversal via `dcutr`.
/// `keypair` is the libp2p transport identity — deliberately separate from
/// the vodozemac chat identity (see the `identity` crate).
///
/// `.with_dns()` matters more here than it looks: the directory's relay
/// is advertised as a `/dns4/<host>/...` multiaddr, not a raw IP. Without
/// a DNS-aware transport, resolving it silently never makes progress
/// until the caller's timeout gives up, indistinguishable from "the relay
/// is unreachable." Every other dial in this app uses raw `/ip4/...`
/// addresses with no DNS component, which is why direct/LAN messaging
/// worked fine while the relay path never did.
pub fn build_swarm(keypair: Keypair) -> anyhow::Result<Swarm<ChatBehaviour>> {
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            Default::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(build_behaviour)?
        .build();
    Ok(swarm)
}

pub fn build_swarm_with_new_identity() -> anyhow::Result<Swarm<ChatBehaviour>> {
    build_swarm(Keypair::generate_ed25519())
}
