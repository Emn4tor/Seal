use std::collections::HashMap;

use crypto_session::{
    AttachmentPayload, DirectEnvelope, DirectPayload, GroupEnvelope, GroupPayload, MegolmManager,
    OlmManager, SessionKey,
};
use futures::StreamExt;
use identity::Identity;
use libp2p::identity::Keypair;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, autonat, gossipsub, request_response};
use libp2p_stream::Control;
use net::{ChatBehaviour, ChatBehaviourEvent, ChatRequest, ChatResponse};

use crate::contact::Contact;
use crate::events::{ChatEvent, NetworkStatus};

fn group_topic(group_id: &str) -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new(format!("/p2p-chat/group/{group_id}/1"))
}

/// Orchestrates identity, the P2P transport, and the Olm/Megolm session
/// managers into one usable node. This is the layer a Tauri command/event
/// wrapper (Phase 5) sits on top of; nothing here is UI-specific.
pub struct ChatNode {
    pub identity: Identity,
    swarm: libp2p::Swarm<ChatBehaviour>,
    olm: OlmManager,
    megolm: MegolmManager,
    contacts: HashMap<String, Contact>,
}

impl ChatNode {
    /// Fresh random libp2p transport identity every call — fine for tests
    /// (which don't care about surviving a restart) but *not* what the real
    /// app should use for an existing account; see `with_keypair`.
    pub fn new(identity: Identity) -> anyhow::Result<Self> {
        Self::with_keypair(identity, Keypair::generate_ed25519())
    }

    /// Like `new`, but with an explicit libp2p transport keypair rather
    /// than a freshly generated one. `AppService::load_or_create` uses
    /// this with a keypair persisted via `storage`'s `p2p_identity` table
    /// so this account's PeerId stays the same across restarts — without
    /// that, every launch would mint a new PeerId, silently stranding
    /// anyone who cached the old one as a contact before this restart.
    pub fn with_keypair(identity: Identity, keypair: Keypair) -> anyhow::Result<Self> {
        let swarm = net::build_swarm(keypair)?;
        Ok(Self {
            identity,
            swarm,
            olm: OlmManager::new(),
            megolm: MegolmManager::new(),
            contacts: HashMap::new(),
        })
    }

    /// A fresh handle for opening/accepting raw voice streams — cheap to
    /// clone, see `net::voice_protocol`. Not tied to any particular call;
    /// `voice::VoiceCallState` gets one of these each time it starts.
    pub fn voice_control(&self) -> Control {
        self.swarm.behaviour().stream.new_control()
    }

    /// Registers a peer's known address(es) without dialing — lets a later
    /// dial-by-peer-id (including `libp2p-stream`'s implicit dial inside
    /// `Control::open_stream`) succeed. Used for voice-channel participants,
    /// who aren't necessarily an existing `Contact` (that requires a 1:1
    /// Olm session, which a fellow group/voice member may never have set
    /// up with us).
    pub fn register_peer_address(&mut self, peer_id: PeerId, addrs: &[Multiaddr]) {
        for addr in addrs {
            self.swarm.add_peer_address(peer_id, addr.clone());
        }
    }

    /// Broadcasts a voice-channel join/leave announcement over the group's
    /// existing Megolm session and gossipsub topic — same transport as a
    /// chat message, just a different payload variant. See
    /// `GroupPayload::VoicePresence` for why this needs to be re-sent on a
    /// heartbeat rather than just once.
    pub fn send_voice_presence(
        &mut self,
        group_id: &str,
        channel_id: &str,
        joined: bool,
    ) -> anyhow::Result<()> {
        let payload = GroupPayload::VoicePresence {
            channel_id: channel_id.to_string(),
            joined,
        };
        let envelope = self.megolm.encrypt(
            group_id,
            &self.identity.user_id(),
            &bincode::serialize(&payload)?,
        )?;
        let bytes = bincode::serialize(&envelope)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(group_topic(group_id), bytes)?;
        Ok(())
    }

    /// Broadcasts "this group's channel list changed" over the group's
    /// existing Megolm session and gossipsub topic — same transport as a
    /// chat message, just a different payload variant. See
    /// `GroupPayload::ChannelsChanged` for why members who miss this need
    /// another way to catch up.
    pub fn send_channels_changed(&mut self, group_id: &str) -> anyhow::Result<()> {
        let payload = GroupPayload::ChannelsChanged;
        let envelope = self.megolm.encrypt(
            group_id,
            &self.identity.user_id(),
            &bincode::serialize(&payload)?,
        )?;
        let bytes = bincode::serialize(&envelope)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(group_topic(group_id), bytes)?;
        Ok(())
    }

    pub fn local_peer_id(&self) -> PeerId {
        *self.swarm.local_peer_id()
    }

    pub fn listen_on(&mut self, addr: Multiaddr) -> anyhow::Result<()> {
        self.swarm.listen_on(addr)?;
        Ok(())
    }

    pub fn dial(&mut self, addr: Multiaddr) -> anyhow::Result<()> {
        self.swarm.dial(addr)?;
        Ok(())
    }

    pub async fn wait_for_listen_addr(&mut self) -> Multiaddr {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = self.swarm.select_next_some().await {
                return address;
            }
        }
    }

    /// Like `wait_for_listen_addr`, but collects every address the swarm
    /// announces within a short window after the first one arrives.
    /// Binding `0.0.0.0` emits one `NewListenAddr` per network interface
    /// (Wi-Fi, Ethernet, a VPN adapter, ...), in unpredictable order, so a
    /// caller that needs every real LAN-reachable address — not just
    /// whichever interface happened to enumerate first — should use this
    /// instead. Always returns at least one address.
    pub async fn wait_for_listen_addrs(&mut self, settle: std::time::Duration) -> Vec<Multiaddr> {
        let mut addrs = Vec::new();
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = self.swarm.select_next_some().await {
                addrs.push(address);
                break;
            }
        }
        let deadline = tokio::time::sleep(settle);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    if let SwarmEvent::NewListenAddr { address, .. } = event {
                        addrs.push(address);
                    }
                }
                _ = &mut deadline => break,
            }
        }
        addrs
    }

    /// Requests a circuit reservation on `relay_addr` (a
    /// `/…/p2p/<relay-peer-id>` multiaddr, as returned by the directory
    /// server's `/v1/relay-info`) so we're reachable even when nobody can
    /// dial us directly. `dcutr` (already in `ChatBehaviour`) then attempts
    /// to upgrade any resulting connection to a direct one automatically —
    /// nothing else here has to drive that part.
    ///
    /// Best-effort by design: callers should treat a failure/timeout as "no
    /// relay available right now" and fall back to whatever addresses
    /// already work (LAN reachability predates this and doesn't depend on
    /// it). Called once at startup, before the main event loop begins
    /// driving the swarm — same pattern as `wait_for_listen_addrs`, and for
    /// the same reason: nothing else is polling `self.swarm` concurrently
    /// yet, so an exclusive wait loop here is safe.
    pub async fn reserve_relay_circuit(
        &mut self,
        relay_addr: Multiaddr,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Multiaddr> {
        let circuit_addr = relay_addr.with(Protocol::P2pCircuit);
        self.swarm.listen_on(circuit_addr)?;

        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. }
                            if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) =>
                        {
                            // `address` here (as reported by the relay-client
                            // transport) already ends in `/p2p/<local-peer-id>`
                            // — appending it again produced a malformed
                            // multiaddr (`.../p2p-circuit/p2p/<id>/p2p/<id>`)
                            // that every dial through it then rejected with
                            // `MalformedMultiaddr`. Confirmed by tracing the
                            // raw event: reservations were succeeding, but
                            // literally nobody could ever dial the address
                            // this returned.
                            return Ok(address);
                        }
                        SwarmEvent::ListenerClosed { addresses, reason: Err(e), .. }
                            if addresses.iter().any(|a| {
                                a.iter().any(|p| matches!(p, Protocol::P2pCircuit))
                            }) =>
                        {
                            anyhow::bail!("relay circuit listener closed: {e}");
                        }
                        _ => {}
                    }
                }
                _ = &mut deadline => {
                    anyhow::bail!("timed out waiting for the relay to grant a circuit reservation");
                }
            }
        }
    }

    pub fn add_contact(
        &mut self,
        user_id: &str,
        curve25519_key_b64: &str,
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
    ) {
        self.contacts.insert(
            user_id.to_string(),
            Contact {
                user_id: user_id.to_string(),
                curve25519_key: curve25519_key_b64.to_string(),
                peer_id,
                addrs,
            },
        );
    }

    fn contact(&self, user_id: &str) -> anyhow::Result<Contact> {
        self.contacts
            .get(user_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown contact: {user_id}"))
    }

    pub fn has_contact(&self, user_id: &str) -> bool {
        self.contacts.contains_key(user_id)
    }

    /// Whether we currently have a live libp2p connection to this contact.
    /// `false` for a contact we've never dialed yet, same as one whose
    /// connection has since dropped (e.g. their app restarted, picking a
    /// fresh ephemeral listen port — see `ensure_connected_contact`'s doc
    /// comment) — either way, the cached `Contact::addrs` might no longer
    /// be dialable and are worth refreshing before relying on them again.
    pub fn is_connected_to(&self, user_id: &str) -> bool {
        self.contacts
            .get(user_id)
            .is_some_and(|c| self.swarm.is_connected(&c.peer_id))
    }

    /// Drops the in-memory transport contact (peer id, known addresses,
    /// Curve25519 key) — the Olm session, if any, is left alone in `olm`'s
    /// own manager rather than torn down here, since removing a contact
    /// doesn't need to invalidate a session that's already established.
    pub fn remove_contact(&mut self, user_id: &str) {
        self.contacts.remove(user_id);
    }

    /// Whether an Olm session with this contact already exists — callers
    /// use this to decide whether claiming a fresh one-time key is
    /// actually necessary before sending.
    pub fn has_direct_session(&self, peer_user_id: &str) -> bool {
        self.contacts
            .get(peer_user_id)
            .is_some_and(|c| self.olm.has_session_with(&c.curve25519_key))
    }

    /// Starts an Olm session with a contact using a one-time key claimed out
    /// of band (from the directory server's `/otk/claim` endpoint) — a
    /// no-op if a session already exists.
    pub fn ensure_outbound_session(
        &mut self,
        peer_user_id: &str,
        peer_one_time_key_b64: &str,
    ) -> anyhow::Result<()> {
        let contact = self.contact(peer_user_id)?;
        if !self.olm.has_session_with(&contact.curve25519_key) {
            self.olm.start_outbound(
                &self.identity,
                &contact.curve25519_key,
                peer_one_time_key_b64,
            )?;
        }
        Ok(())
    }

    pub fn send_direct_message(
        &mut self,
        peer_user_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        let contact = self.contact(peer_user_id)?;
        let payload = DirectPayload::Chat {
            body: body.to_string(),
            attachment,
        };
        let envelope = self.olm.encrypt(
            &self.identity,
            &contact.curve25519_key,
            &bincode::serialize(&payload)?,
        )?;
        self.send_envelope(&contact, &envelope)
    }

    fn send_envelope(
        &mut self,
        contact: &Contact,
        envelope: &DirectEnvelope,
    ) -> anyhow::Result<()> {
        // bincode, not serde_json: this wraps `ciphertext: Vec<u8>` (and, via
        // `DirectPayload`, potentially a whole attachment's bytes once
        // decrypted) — JSON has no binary type, so it would explode a byte
        // vector into a comma-separated array of decimal numbers (4-5x
        // larger, and enough to blow past transport size limits for
        // anything but tiny text messages). bincode encodes `Vec<u8>`
        // compactly. Purely an internal wire encoding between peers running
        // the same code, so there's no cross-version compatibility concern.
        let bytes = bincode::serialize(envelope)?;
        self.swarm
            .behaviour_mut()
            .request_response
            .send_request_with_addresses(
                &contact.peer_id,
                ChatRequest { payload: bytes },
                contact.addrs.clone(),
            );
        Ok(())
    }

    /// Creates a fresh outbound Megolm session for `group_id` and subscribes
    /// to its gossipsub topic so we receive other members' messages.
    pub fn create_group(&mut self, group_id: &str) {
        self.megolm.rotate_outbound(group_id);
        let _ = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&group_topic(group_id));
    }

    pub fn join_group_topic(&mut self, group_id: &str) {
        let _ = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&group_topic(group_id));
    }

    /// Whether we already have our own outbound Megolm session for this
    /// group — i.e. whether we can actually send anything to it (not just
    /// receive), regardless of whether we created it or joined later.
    pub fn has_outbound_group_session(&self, group_id: &str) -> bool {
        self.megolm.has_outbound(group_id)
    }

    /// Shares the group's *current* outbound session key with one member,
    /// 1:1-Olm-encrypted — never sent through any server.
    pub fn share_group_key(&mut self, group_id: &str, member_user_id: &str) -> anyhow::Result<()> {
        let key = self
            .megolm
            .current_session_key(group_id)
            .ok_or_else(|| anyhow::anyhow!("no outbound session for group {group_id}"))?;
        let contact = self.contact(member_user_id)?;
        let payload = DirectPayload::GroupKeyShare {
            group_id: group_id.to_string(),
            session_key_bytes: key.to_bytes(),
        };
        let envelope = self.olm.encrypt(
            &self.identity,
            &contact.curve25519_key,
            &bincode::serialize(&payload)?,
        )?;
        self.send_envelope(&contact, &envelope)
    }

    /// Asks `owner_user_id` to (re-)send a group's key — see `DirectPayload::
    /// GroupKeyRequest`'s doc comment. `owner_user_id` must already be a
    /// contact (`AppService::request_missing_group_key` ensures that
    /// before calling this).
    pub fn request_group_key(&mut self, group_id: &str, owner_user_id: &str) -> anyhow::Result<()> {
        let contact = self.contact(owner_user_id)?;
        let payload = DirectPayload::GroupKeyRequest {
            group_id: group_id.to_string(),
        };
        let envelope = self.olm.encrypt(
            &self.identity,
            &contact.curve25519_key,
            &bincode::serialize(&payload)?,
        )?;
        self.send_envelope(&contact, &envelope)
    }

    /// Rotates to a fresh session key and re-shares it with the given
    /// remaining members — call this on member removal so the removed
    /// member can't decrypt anything encrypted afterwards.
    pub fn rotate_group_key(
        &mut self,
        group_id: &str,
        remaining_member_ids: &[String],
    ) -> anyhow::Result<()> {
        self.megolm.rotate_outbound(group_id);
        for member in remaining_member_ids {
            self.share_group_key(group_id, member)?;
        }
        Ok(())
    }

    pub fn send_group_message(
        &mut self,
        group_id: &str,
        channel_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        let payload = GroupPayload::Chat {
            channel_id: channel_id.to_string(),
            body: body.to_string(),
            attachment,
        };
        let envelope = self.megolm.encrypt(
            group_id,
            &self.identity.user_id(),
            &bincode::serialize(&payload)?,
        )?;
        let bytes = bincode::serialize(&envelope)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(group_topic(group_id), bytes)?;
        Ok(())
    }

    /// Drives the swarm until a chat-relevant event occurs. Callers should
    /// loop on this — it's the async equivalent of an event stream and is
    /// what a Tauri background task would forward to the frontend.
    pub async fn next_event(&mut self) -> ChatEvent {
        loop {
            let event = self.swarm.select_next_some().await;
            if let Some(chat_event) = self.handle_swarm_event(event) {
                return chat_event;
            }
        }
    }

    fn handle_swarm_event(&mut self, event: SwarmEvent<ChatBehaviourEvent>) -> Option<ChatEvent> {
        match event {
            SwarmEvent::Behaviour(ChatBehaviourEvent::RequestResponse(event)) => {
                self.handle_request_response_event(event)
            }
            SwarmEvent::Behaviour(ChatBehaviourEvent::Gossipsub(event)) => {
                self.handle_gossipsub_event(event)
            }
            SwarmEvent::Behaviour(ChatBehaviourEvent::Autonat(autonat::Event::StatusChanged {
                new,
                ..
            })) => Some(ChatEvent::NetworkStatus(match new {
                autonat::NatStatus::Public(_) => NetworkStatus::Public,
                autonat::NatStatus::Private => NetworkStatus::Private,
                autonat::NatStatus::Unknown => NetworkStatus::Unknown,
            })),
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                Some(ChatEvent::Connected(peer_id))
            }
            _ => None,
        }
    }

    fn handle_request_response_event(
        &mut self,
        event: request_response::Event<ChatRequest, ChatResponse>,
    ) -> Option<ChatEvent> {
        match event {
            request_response::Event::Message { peer, message, .. } => {
                self.handle_request_response_message(peer, message)
            }
            // Previously silently dropped: a dial/send that never reached
            // the peer (unreachable address, connection refused, timeout, …)
            // produced no error and no event, so a failed direct message or
            // group-key-share looked identical to a successful one from the
            // sender's side. This is the other half of what actually makes
            // that failure visible — the addresses now including a relay
            // candidate (see `ChatNode::reserve_relay_circuit`) is what
            // makes it less *frequent*.
            request_response::Event::OutboundFailure { peer, error, .. } => {
                let peer_user_id = self.contact_user_id_for_peer(&peer);
                tracing::warn!(
                    peer = %peer,
                    peer_user_id = ?peer_user_id,
                    error = %error,
                    "direct message delivery failed"
                );
                Some(ChatEvent::MessageSendFailed {
                    peer_user_id,
                    reason: error.to_string(),
                })
            }
            _ => None,
        }
    }

    fn contact_user_id_for_peer(&self, peer_id: &PeerId) -> Option<String> {
        self.contacts
            .iter()
            .find(|(_, c)| c.peer_id == *peer_id)
            .map(|(user_id, _)| user_id.clone())
    }

    fn handle_request_response_message(
        &mut self,
        peer: PeerId,
        message: request_response::Message<ChatRequest, ChatResponse>,
    ) -> Option<ChatEvent> {
        match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                let result = self.handle_direct_request(&request);
                let ack = result.is_ok();
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, ChatResponse { ack });

                match result {
                    Ok(event) => event,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to process inbound direct message");
                        None
                    }
                }
            }
            // Previously ignored entirely: the transport delivered the
            // message fine, but the *recipient* couldn't decrypt it (most
            // commonly because they restarted and lost the in-memory-only
            // Olm session we still had cached for them — see `OlmManager`'s
            // doc comment) and sent `ack: false` back to say so. Nothing
            // ever looked at that ack, so this failure mode was just as
            // silent as a transport-level one: the message vanished with
            // no error on either side. Dropping our own cached session
            // here means the *next* attempt claims a fresh one-time key
            // and starts a session the recipient — who has no session
            // state at all to conflict with — can actually accept.
            request_response::Message::Response { response, .. } if !response.ack => {
                let peer_user_id = self.contact_user_id_for_peer(&peer);
                if let Some(contact) = self.contacts.values().find(|c| c.peer_id == peer) {
                    self.olm.forget_session(&contact.curve25519_key);
                }
                tracing::warn!(
                    peer = %peer,
                    peer_user_id = ?peer_user_id,
                    "peer rejected a direct message (failed to decrypt) — session reset for retry"
                );
                Some(ChatEvent::MessageSendFailed {
                    peer_user_id,
                    reason: "the recipient couldn't decrypt this message".to_string(),
                })
            }
            request_response::Message::Response { .. } => None,
        }
    }

    fn handle_direct_request(
        &mut self,
        request: &ChatRequest,
    ) -> anyhow::Result<Option<ChatEvent>> {
        let envelope: DirectEnvelope = bincode::deserialize(&request.payload)?;
        let plaintext = self.olm.decrypt(&mut self.identity, &envelope)?;
        let payload: DirectPayload = bincode::deserialize(&plaintext)?;

        match payload {
            DirectPayload::Chat { body, attachment } => Ok(Some(ChatEvent::DirectMessage {
                from: envelope.sender_user_id,
                body,
                attachment,
            })),
            DirectPayload::GroupKeyShare {
                group_id,
                session_key_bytes,
            } => {
                let key = SessionKey::from_bytes(&session_key_bytes)
                    .map_err(|e| anyhow::anyhow!("invalid session key: {e}"))?;
                self.megolm
                    .insert_inbound(&group_id, &envelope.sender_user_id, &key);
                Ok(Some(ChatEvent::GroupKeyReceived {
                    group_id,
                    from: envelope.sender_user_id,
                }))
            }
            DirectPayload::GroupKeyRequest { group_id } => Ok(Some(ChatEvent::GroupKeyRequested {
                group_id,
                from: envelope.sender_user_id,
            })),
        }
    }

    fn handle_gossipsub_event(&mut self, event: gossipsub::Event) -> Option<ChatEvent> {
        match event {
            gossipsub::Event::Message { message, .. } => {
                let envelope: GroupEnvelope = bincode::deserialize(&message.data).ok()?;
                let plaintext = self.megolm.decrypt(&envelope).ok()?;
                let payload: GroupPayload = bincode::deserialize(&plaintext).ok()?;
                match payload {
                    GroupPayload::Chat {
                        channel_id,
                        body,
                        attachment,
                    } => Some(ChatEvent::GroupMessage {
                        group_id: envelope.group_id,
                        channel_id,
                        from: envelope.sender_user_id,
                        body,
                        attachment,
                    }),
                    GroupPayload::VoicePresence { channel_id, joined } => {
                        Some(ChatEvent::VoicePresence {
                            group_id: envelope.group_id,
                            channel_id,
                            from: envelope.sender_user_id,
                            joined,
                        })
                    }
                    GroupPayload::ChannelsChanged => Some(ChatEvent::GroupChannelsChanged {
                        group_id: envelope.group_id,
                    }),
                }
            }
            gossipsub::Event::Subscribed { peer_id, topic } => Some(ChatEvent::GossipSubscribed {
                peer_id,
                topic: topic.to_string(),
            }),
            _ => None,
        }
    }
}
