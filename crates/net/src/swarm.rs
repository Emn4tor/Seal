use libp2p::identity::Keypair;
use libp2p::{Swarm, SwarmBuilder, noise, yamux};

use crate::behaviour::{ChatBehaviour, build_behaviour};

/// Builds a fully-wired swarm: QUIC primary / TCP+Noise+Yamux fallback
/// transport, plus a relay-client transport for NAT traversal via `dcutr`.
/// `keypair` is the libp2p transport identity — deliberately separate from
/// the vodozemac chat identity (see the `identity` crate).
pub fn build_swarm(keypair: Keypair) -> anyhow::Result<Swarm<ChatBehaviour>> {
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            Default::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(build_behaviour)?
        .build();
    Ok(swarm)
}

pub fn build_swarm_with_new_identity() -> anyhow::Result<Swarm<ChatBehaviour>> {
    build_swarm(Keypair::generate_ed25519())
}
