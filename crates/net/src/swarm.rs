use libp2p::identity::Keypair;
use libp2p::{Swarm, SwarmBuilder, noise, yamux};

use crate::behaviour::{ChatBehaviour, build_behaviour};

/// Builds a fully-wired swarm: QUIC primary / TCP+Noise+Yamux fallback
/// transport, plus a relay-client transport for NAT traversal via `dcutr`.
/// `keypair` is the libp2p transport identity — deliberately separate from
/// the vodozemac chat identity (see the `identity` crate).
///
/// `.with_dns()` matters more here than it looks: the directory server's
/// relay is advertised as a `/dns4/<host>/tcp/<port>/p2p/<peer-id>`
/// multiaddr (`DIRECTORY_RELAY_EXTERNAL_MULTIADDR`), not a raw IP, since
/// the host might not have a stable IP forever. Without a DNS-aware
/// transport in the stack, nothing can ever resolve that address — dialing
/// or listening on it (`ChatNode::reserve_relay_circuit`) doesn't fail
/// loudly, it just silently never makes progress until the caller's own
/// timeout gives up, which looked identical to "the relay is unreachable"
/// from every angle except a packet capture on the relay's own port
/// showing zero incoming traffic at all. Every *other* dial in this app
/// (contacts, by their presence-advertised address) already uses raw
/// `/ip4/.../tcp/...` addresses with no DNS component, which is exactly
/// why direct/LAN messaging worked fine while the relay path never did.
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
