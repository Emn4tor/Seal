use std::collections::HashMap;

use crypto_session::{
    AttachmentPayload, DirectEnvelope, DirectPayload, GroupEnvelope, GroupPayload, MegolmManager,
    OlmManager, SessionKey, SyncMessage,
};
use futures::StreamExt;
use identity::Identity;
use libp2p::identity::Keypair;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, autonat, gossipsub, request_response};
use libp2p_stream::Control;
use net::{
    ChatBehaviour, ChatBehaviourEvent, ChatRequest, ChatResponse, PairingEphemeralSecret,
    PairingPayload, PairingRequest, PairingResponse,
};

use crate::contact::{Contact, DeviceContact};
use crate::events::{ChatEvent, NetworkStatus};

fn group_topic(group_id: &str) -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new(format!("/p2p-chat/group/{group_id}/1"))
}

/// Bookkeeping for one in-flight call's fanned-out device sends — the call
/// is only reported failed once every device has resolved without success.
struct CallSendTally {
    outstanding: usize,
    any_success: bool,
}

struct PendingCallSend {
    call_id: String,
    peer_user_id: String,
}

/// What resolving one device's call-signaling send means for the call as a
/// whole — see `ChatNode::resolve_call_send`.
enum CallSendResolution {
    /// This request id wasn't a tracked call-signaling send at all — the
    /// caller should fall back to its generic handling.
    NotTracked,
    /// Tracked, but either still waiting on other devices, or resolved
    /// without every device failing — no event to surface either way.
    NoEvent,
    /// Every device this call was fanned out to has now failed.
    Failed {
        call_id: String,
        peer_user_id: String,
    },
}

/// Inviting side's state between minting a pairing offer and a matching
/// request arriving or the token expiring. Single-slot, like `pending_call`.
struct PendingPairingOffer {
    token: String,
    ephemeral_secret: PairingEphemeralSecret,
    expires_at: std::time::Instant,
}

/// Held between validating an inbound `PairingRequest` and `AppService`
/// supplying the certificate + snapshot to send back (`ChatNode` has no
/// direct access to `storage::LocalStore`).
struct PendingPairingResponse {
    channel: request_response::ResponseChannel<PairingResponse>,
    shared_key: [u8; 32],
}

/// The joining/phone side's state between sending a `PairingRequest` and
/// getting the matching (encrypted) `PairingResponse` back.
struct PendingPairingJoin {
    shared_key: [u8; 32],
}

/// Orchestrates identity, the P2P transport, and the Olm/Megolm session
/// managers into one usable node. This is the layer a Tauri command/event
/// wrapper (Phase 5) sits on top of; nothing here is UI-specific.
pub struct ChatNode {
    pub identity: Identity,
    /// This device's own identifier among the account's other devices —
    /// stamped on every outbound `DirectEnvelope` as `sender_device_id`.
    device_id: String,
    /// This device's own Olm account, separate from `identity` (the
    /// account's master signing identity) even on the very first device —
    /// see `schema.sql`'s `device_olm_pickle_blob` doc comment.
    pub device_identity: Identity,
    swarm: libp2p::Swarm<ChatBehaviour>,
    olm: OlmManager,
    megolm: MegolmManager,
    contacts: HashMap<String, Contact>,
    /// Tracks the outbound request id for each device a `CallInvite`/
    /// `CallAccept` was fanned out to, so a failure can be reported as
    /// `ChatEvent::CallFailed` rather than the generic `MessageSendFailed`.
    pending_call_sends: HashMap<request_response::OutboundRequestId, PendingCallSend>,
    /// Per-`call_id` tally of how many fanned-out device sends are still
    /// unresolved — see `CallSendTally` and `resolve_call_send`.
    pending_call_outstanding: HashMap<String, CallSendTally>,
    /// This device's own outstanding pairing offer (it's inviting a new
    /// device), if any — see `PendingPairingOffer`.
    pending_pairing_offer: Option<PendingPairingOffer>,
    /// Inbound pairing requests awaiting a response from `AppService`,
    /// keyed by an incrementing id (`ChatEvent::PairingRequested::response_id`).
    pending_pairing_responses: HashMap<u64, PendingPairingResponse>,
    next_pairing_response_id: u64,
    /// This device's own outstanding pairing attempt (it's joining another
    /// account's device), if any — see `PendingPairingJoin`.
    pending_pairing_join: Option<PendingPairingJoin>,
}

impl ChatNode {
    /// Fresh random transport identity and device_id every call — fine for
    /// tests, *not* for a real existing account; see `with_keypair`.
    pub fn new(identity: Identity) -> anyhow::Result<Self> {
        Self::with_keypair(
            identity,
            Keypair::generate_ed25519(),
            uuid::Uuid::new_v4().to_string(),
            Identity::generate(),
        )
    }

    /// Like `new`, but with explicit persisted values so this account's
    /// PeerId and device identity stay the same across restarts — without
    /// that, every launch would strand contacts who cached the old PeerId.
    pub fn with_keypair(
        identity: Identity,
        keypair: Keypair,
        device_id: String,
        device_identity: Identity,
    ) -> anyhow::Result<Self> {
        let swarm = net::build_swarm(keypair)?;
        Ok(Self {
            identity,
            device_id,
            device_identity,
            swarm,
            olm: OlmManager::new(),
            megolm: MegolmManager::new(),
            contacts: HashMap::new(),
            pending_call_sends: HashMap::new(),
            pending_call_outstanding: HashMap::new(),
            pending_pairing_offer: None,
            pending_pairing_responses: HashMap::new(),
            next_pairing_response_id: 0,
            pending_pairing_join: None,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
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

    /// Requests a circuit reservation on `relay_addr` so we're reachable
    /// even when nobody can dial us directly; `dcutr` then attempts to
    /// upgrade any resulting connection to a direct one automatically.
    ///
    /// Best-effort: callers should treat a failure/timeout as "no relay
    /// available" and fall back to addresses that already work. Called
    /// once at startup, before the event loop starts polling `self.swarm`
    /// (same pattern as `wait_for_listen_addrs`, for the same reason).
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

    /// Replaces `user_id`'s entire set of known reachable devices — always
    /// fetched and verified as a whole, never patched incrementally.
    pub fn add_contact(&mut self, user_id: &str, devices: Vec<DeviceContact>) {
        self.contacts.insert(
            user_id.to_string(),
            Contact {
                user_id: user_id.to_string(),
                devices,
            },
        );
    }

    fn contact(&self, user_id: &str) -> anyhow::Result<Contact> {
        self.contacts
            .get(user_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown contact: {user_id}"))
    }

    /// A contact's currently-known devices, or empty if we don't know them
    /// at all — used by callers that need to loop per-device (ensuring a
    /// session with each, claiming device-scoped one-time keys, ...).
    pub fn contact_devices(&self, user_id: &str) -> Vec<DeviceContact> {
        self.contacts
            .get(user_id)
            .map(|c| c.devices.clone())
            .unwrap_or_default()
    }

    pub fn has_contact(&self, user_id: &str) -> bool {
        self.contacts.contains_key(user_id)
    }

    /// Whether we have a live libp2p connection to *any* of this contact's
    /// devices. `false` also for a dropped connection whose cached address
    /// may no longer be dialable — see `ensure_connected_contact`.
    pub fn is_connected_to(&self, user_id: &str) -> bool {
        self.contacts.get(user_id).is_some_and(|c| {
            c.devices
                .iter()
                .any(|d| self.swarm.is_connected(&d.peer_id))
        })
    }

    /// Drops the in-memory transport contact (device list) — Olm sessions
    /// with its devices are left alone; removing a contact doesn't invalidate them.
    pub fn remove_contact(&mut self, user_id: &str) {
        self.contacts.remove(user_id);
    }

    /// Whether an Olm session with one specific device of this contact
    /// already exists — used to decide if claiming a fresh one-time key
    /// for that device is necessary.
    pub fn has_direct_session_with_device(&self, peer_user_id: &str, device_id: &str) -> bool {
        self.contacts
            .get(peer_user_id)
            .and_then(|c| c.devices.iter().find(|d| d.device_id == device_id))
            .is_some_and(|d| self.olm.has_session_with(&d.curve25519_key))
    }

    /// Starts an Olm session with one specific device of a contact, using a
    /// one-time key claimed out of band for that device — a no-op if a
    /// session with it already exists.
    pub fn ensure_outbound_session(
        &mut self,
        peer_user_id: &str,
        device_id: &str,
        peer_one_time_key_b64: &str,
    ) -> anyhow::Result<()> {
        let contact = self.contact(peer_user_id)?;
        let device = contact
            .devices
            .iter()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| {
                anyhow::anyhow!("unknown device {device_id} for contact {peer_user_id}")
            })?;
        if !self.olm.has_session_with(&device.curve25519_key) {
            self.olm.start_outbound(
                &self.device_identity,
                &device.curve25519_key,
                peer_one_time_key_b64,
            )?;
        }
        Ok(())
    }

    pub fn send_direct_message(
        &mut self,
        peer_user_id: &str,
        message_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        self.encrypt_and_send_direct(
            peer_user_id,
            &DirectPayload::Chat {
                message_id: message_id.to_string(),
                body: body.to_string(),
                attachment,
            },
        )?;
        Ok(())
    }

    /// Shared by every 1:1-Olm-encrypted send: encrypts `payload` once per
    /// reachable device and hands each to the transport. Returns one
    /// request id per device, so callers like call signaling can correlate failures.
    fn encrypt_and_send_direct(
        &mut self,
        peer_user_id: &str,
        payload: &DirectPayload,
    ) -> anyhow::Result<Vec<request_response::OutboundRequestId>> {
        let contact = self.contact(peer_user_id)?;
        if contact.devices.is_empty() {
            anyhow::bail!("contact {peer_user_id} has no currently reachable devices");
        }
        let plaintext = bincode::serialize(payload)?;
        let my_curve25519_key = self.device_identity.curve25519_public_base64();
        let mut request_ids = Vec::with_capacity(contact.devices.len());
        for device in &contact.devices {
            let envelope = self.olm.encrypt(
                &self.identity.user_id(),
                &self.device_id,
                &my_curve25519_key,
                &device.curve25519_key,
                &plaintext,
            )?;
            request_ids.push(self.send_envelope(device, &envelope)?);
        }
        Ok(request_ids)
    }

    /// Like `encrypt_and_send_direct`, but targets exactly one device
    /// instead of fanning out — what manual sync needs.
    fn encrypt_and_send_direct_to_device(
        &mut self,
        peer_user_id: &str,
        device_id: &str,
        payload: &DirectPayload,
    ) -> anyhow::Result<request_response::OutboundRequestId> {
        let contact = self.contact(peer_user_id)?;
        let device = contact
            .devices
            .iter()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| {
                anyhow::anyhow!("unknown device {device_id} for contact {peer_user_id}")
            })?
            .clone();
        let plaintext = bincode::serialize(payload)?;
        let my_curve25519_key = self.device_identity.curve25519_public_base64();
        let envelope = self.olm.encrypt(
            &self.identity.user_id(),
            &self.device_id,
            &my_curve25519_key,
            &device.curve25519_key,
            &plaintext,
        )?;
        self.send_envelope(&device, &envelope)
    }

    /// Kicks off a manual sync with `peer_device_id`. `AppService::
    /// sync_with_device` must already have registered "myself" as an
    /// in-memory contact with that device present before calling this.
    pub fn send_sync_request(
        &mut self,
        peer_device_id: &str,
        since: i64,
        messages: Vec<SyncMessage>,
    ) -> anyhow::Result<()> {
        let my_user_id = self.identity.user_id();
        self.encrypt_and_send_direct_to_device(
            &my_user_id,
            peer_device_id,
            &DirectPayload::SyncRequest { since, messages },
        )?;
        Ok(())
    }

    /// Answers a sync request with this device's own delta — see
    /// `AppService::handle_sync_requested`.
    pub fn send_sync_response(
        &mut self,
        peer_device_id: &str,
        messages: Vec<SyncMessage>,
    ) -> anyhow::Result<()> {
        let my_user_id = self.identity.user_id();
        self.encrypt_and_send_direct_to_device(
            &my_user_id,
            peer_device_id,
            &DirectPayload::SyncResponse { messages },
        )?;
        Ok(())
    }

    /// "Rings" `peer_user_id`, fanning out to every reachable device. The
    /// call only fails once *every* device has failed — one unreachable
    /// device must not fail a call still ringing on another; see `resolve_call_send`.
    pub fn send_call_invite(&mut self, peer_user_id: &str, call_id: &str) -> anyhow::Result<()> {
        let request_ids = self.encrypt_and_send_direct(
            peer_user_id,
            &DirectPayload::CallInvite {
                call_id: call_id.to_string(),
            },
        )?;
        self.track_call_sends(call_id, peer_user_id, request_ids);
        Ok(())
    }

    /// Same fan-out tracking as `send_call_invite`, for the callee's side:
    /// our acceptance failing to reach every one of the caller's devices
    /// means the call can't proceed either.
    pub fn send_call_accept(&mut self, peer_user_id: &str, call_id: &str) -> anyhow::Result<()> {
        let request_ids = self.encrypt_and_send_direct(
            peer_user_id,
            &DirectPayload::CallAccept {
                call_id: call_id.to_string(),
            },
        )?;
        self.track_call_sends(call_id, peer_user_id, request_ids);
        Ok(())
    }

    fn track_call_sends(
        &mut self,
        call_id: &str,
        peer_user_id: &str,
        request_ids: Vec<request_response::OutboundRequestId>,
    ) {
        self.pending_call_outstanding.insert(
            call_id.to_string(),
            CallSendTally {
                outstanding: request_ids.len(),
                any_success: false,
            },
        );
        for request_id in request_ids {
            self.pending_call_sends.insert(
                request_id,
                PendingCallSend {
                    call_id: call_id.to_string(),
                    peer_user_id: peer_user_id.to_string(),
                },
            );
        }
    }

    /// Records that one device's fanned-out call-signaling send has
    /// resolved (delivered or failed), and reports what that means for the
    /// call as a whole — see `CallSendResolution`.
    fn resolve_call_send(
        &mut self,
        request_id: request_response::OutboundRequestId,
        succeeded: bool,
    ) -> CallSendResolution {
        let Some(PendingCallSend {
            call_id,
            peer_user_id,
        }) = self.pending_call_sends.remove(&request_id)
        else {
            return CallSendResolution::NotTracked;
        };
        let Some(tally) = self.pending_call_outstanding.get_mut(&call_id) else {
            return CallSendResolution::NoEvent;
        };
        if succeeded {
            tally.any_success = true;
        }
        tally.outstanding = tally.outstanding.saturating_sub(1);
        if tally.outstanding > 0 {
            return CallSendResolution::NoEvent;
        }
        let failed = !tally.any_success;
        self.pending_call_outstanding.remove(&call_id);
        if failed {
            CallSendResolution::Failed {
                call_id,
                peer_user_id,
            }
        } else {
            CallSendResolution::NoEvent
        }
    }

    pub fn send_call_decline(&mut self, peer_user_id: &str, call_id: &str) -> anyhow::Result<()> {
        self.encrypt_and_send_direct(
            peer_user_id,
            &DirectPayload::CallDecline {
                call_id: call_id.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn send_call_end(&mut self, peer_user_id: &str, call_id: &str) -> anyhow::Result<()> {
        self.encrypt_and_send_direct(
            peer_user_id,
            &DirectPayload::CallEnd {
                call_id: call_id.to_string(),
            },
        )?;
        Ok(())
    }

    /// Tells every device of `peer_user_id` *except* `answered_device_id`
    /// that this call is over, so they stop ringing. Best-effort — a
    /// failure just means it rings a little longer on its own timeout.
    pub fn send_call_end_to_other_devices(
        &mut self,
        peer_user_id: &str,
        call_id: &str,
        answered_device_id: &str,
    ) -> anyhow::Result<()> {
        let contact = self.contact(peer_user_id)?;
        let plaintext = bincode::serialize(&DirectPayload::CallEnd {
            call_id: call_id.to_string(),
        })?;
        let my_user_id = self.identity.user_id();
        let my_curve25519_key = self.device_identity.curve25519_public_base64();
        for device in contact
            .devices
            .iter()
            .filter(|d| d.device_id != answered_device_id)
        {
            match self.olm.encrypt(
                &my_user_id,
                &self.device_id,
                &my_curve25519_key,
                &device.curve25519_key,
                &plaintext,
            ) {
                Ok(envelope) => {
                    let _ = self.send_envelope(device, &envelope);
                }
                Err(e) => tracing::warn!(
                    error = %e,
                    device_id = %device.device_id,
                    "failed to notify a sibling device that a call was answered elsewhere"
                ),
            }
        }
        Ok(())
    }

    fn send_envelope(
        &mut self,
        device: &DeviceContact,
        envelope: &DirectEnvelope,
    ) -> anyhow::Result<request_response::OutboundRequestId> {
        // bincode, not serde_json: this wraps `ciphertext: Vec<u8>` (and, via
        // `DirectPayload`, potentially a whole attachment's bytes once
        // decrypted) — JSON has no binary type, so it would explode a byte
        // vector into a comma-separated array of decimal numbers (4-5x
        // larger, and enough to blow past transport size limits for
        // anything but tiny text messages). bincode encodes `Vec<u8>`
        // compactly. Purely an internal wire encoding between peers running
        // the same code, so there's no cross-version compatibility concern.
        let bytes = bincode::serialize(envelope)?;
        let request_id = self
            .swarm
            .behaviour_mut()
            .request_response
            .send_request_with_addresses(
                &device.peer_id,
                ChatRequest { payload: bytes },
                device.addrs.clone(),
            );
        Ok(request_id)
    }

    /// Starts offering to pair a new device: mints a one-time token and
    /// ephemeral X25519 keypair, replacing any unfinished previous offer.
    pub fn start_pairing_offer(&mut self) -> (String, [u8; 32]) {
        let token = uuid::Uuid::new_v4().to_string();
        let (ephemeral_secret, ephemeral_pubkey) = net::generate_ephemeral_keypair();
        self.pending_pairing_offer = Some(PendingPairingOffer {
            token: token.clone(),
            ephemeral_secret,
            expires_at: std::time::Instant::now()
                + std::time::Duration::from_secs(net::PAIRING_TOKEN_TTL_SECS),
        });
        (token, ephemeral_pubkey)
    }

    /// Sends this response to a pairing request previously surfaced as
    /// `ChatEvent::PairingRequested`. Fails if `response_id` has already
    /// been answered or the offer it belonged to is gone.
    pub fn respond_to_pairing(
        &mut self,
        response_id: u64,
        payload: &PairingPayload,
    ) -> anyhow::Result<()> {
        let ctx = self
            .pending_pairing_responses
            .remove(&response_id)
            .ok_or_else(|| anyhow::anyhow!("no pending pairing response with id {response_id}"))?;
        let plaintext = bincode::serialize(payload)?;
        let encrypted = net::encrypt_payload(&ctx.shared_key, &plaintext);
        let _ = self.swarm.behaviour_mut().pairing.send_response(
            ctx.channel,
            PairingResponse {
                encrypted_payload: Some(encrypted),
                error: None,
            },
        );
        Ok(())
    }

    /// The joining/phone side: dials the inviting device and sends a
    /// `PairingRequest` proving `token` was actually scanned. Result
    /// arrives later as `ChatEvent::PairingCompleted`/`PairingFailed`.
    #[allow(clippy::too_many_arguments)]
    pub fn request_pairing(
        &mut self,
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
        token: String,
        their_ephemeral_pubkey: [u8; 32],
        my_device_id: String,
        my_device_ed25519_key: String,
        my_device_curve25519_key: String,
    ) {
        let (my_ephemeral_secret, my_ephemeral_pubkey) = net::generate_ephemeral_keypair();
        let shared_key = net::derive_shared_key(my_ephemeral_secret, their_ephemeral_pubkey);
        self.pending_pairing_join = Some(PendingPairingJoin { shared_key });
        self.swarm
            .behaviour_mut()
            .pairing
            .send_request_with_addresses(
                &peer_id,
                PairingRequest {
                    token,
                    ephemeral_pubkey: my_ephemeral_pubkey,
                    device_id: my_device_id,
                    device_ed25519_key: my_device_ed25519_key,
                    device_curve25519_key: my_device_curve25519_key,
                },
                addrs,
            );
    }

    fn handle_pairing_event(
        &mut self,
        event: request_response::Event<PairingRequest, PairingResponse>,
    ) -> Option<ChatEvent> {
        match event {
            request_response::Event::Message {
                message:
                    request_response::Message::Request {
                        request, channel, ..
                    },
                ..
            } => {
                let offer = self.pending_pairing_offer.take()?;
                if offer.expires_at < std::time::Instant::now() || offer.token != request.token {
                    let _ = self.swarm.behaviour_mut().pairing.send_response(
                        channel,
                        PairingResponse {
                            encrypted_payload: None,
                            error: Some("pairing token expired or already used".to_string()),
                        },
                    );
                    return None;
                }
                let shared_key =
                    net::derive_shared_key(offer.ephemeral_secret, request.ephemeral_pubkey);
                let response_id = self.next_pairing_response_id;
                self.next_pairing_response_id += 1;
                self.pending_pairing_responses.insert(
                    response_id,
                    PendingPairingResponse {
                        channel,
                        shared_key,
                    },
                );
                Some(ChatEvent::PairingRequested {
                    response_id,
                    device_id: request.device_id,
                    device_ed25519_key: request.device_ed25519_key,
                    device_curve25519_key: request.device_curve25519_key,
                })
            }
            request_response::Event::Message {
                message: request_response::Message::Response { response, .. },
                ..
            } => {
                let join = self.pending_pairing_join.take()?;
                if let Some(error) = response.error {
                    return Some(ChatEvent::PairingFailed(error));
                }
                let Some(encrypted) = response.encrypted_payload else {
                    return Some(ChatEvent::PairingFailed(
                        "pairing response had neither a payload nor an error".to_string(),
                    ));
                };
                match net::decrypt_payload(&join.shared_key, &encrypted) {
                    Ok(plaintext) => match bincode::deserialize::<PairingPayload>(&plaintext) {
                        Ok(payload) => Some(ChatEvent::PairingCompleted(Box::new(payload))),
                        Err(e) => Some(ChatEvent::PairingFailed(format!(
                            "pairing response was malformed: {e}"
                        ))),
                    },
                    Err(e) => Some(ChatEvent::PairingFailed(e.to_string())),
                }
            }
            request_response::Event::OutboundFailure { error, .. } => {
                self.pending_pairing_join = None;
                Some(ChatEvent::PairingFailed(format!(
                    "failed to reach the inviting device: {error}"
                )))
            }
            _ => None,
        }
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

    /// The other half of `join_group_topic`. Call this on leaving a
    /// group, or its gossipsub subscription (and so message delivery)
    /// stays live forever, silently resurrecting an orphaned conversation
    /// locally with nothing left pointing at it.
    pub fn leave_group_topic(&mut self, group_id: &str) {
        let _ = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .unsubscribe(&group_topic(group_id));
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
        self.encrypt_and_send_direct(
            member_user_id,
            &DirectPayload::GroupKeyShare {
                group_id: group_id.to_string(),
                session_key_bytes: key.to_bytes(),
            },
        )?;
        Ok(())
    }

    /// Asks `owner_user_id` to (re-)send a group's key — see `DirectPayload::
    /// GroupKeyRequest`'s doc comment. `owner_user_id` must already be a
    /// contact (`AppService::request_missing_group_key` ensures that
    /// before calling this).
    pub fn request_group_key(&mut self, group_id: &str, owner_user_id: &str) -> anyhow::Result<()> {
        self.encrypt_and_send_direct(
            owner_user_id,
            &DirectPayload::GroupKeyRequest {
                group_id: group_id.to_string(),
            },
        )?;
        Ok(())
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
        message_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        let payload = GroupPayload::Chat {
            message_id: message_id.to_string(),
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
            SwarmEvent::Behaviour(ChatBehaviourEvent::Pairing(event)) => {
                self.handle_pairing_event(event)
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
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => {
                match self.resolve_call_send(request_id, false) {
                    CallSendResolution::Failed {
                        call_id,
                        peer_user_id: call_peer_user_id,
                    } => {
                        tracing::warn!(
                            peer = %peer,
                            peer_user_id = %call_peer_user_id,
                            call_id,
                            error = %error,
                            "call signaling delivery failed to every device — most likely the peer isn't online on any of them"
                        );
                        return Some(ChatEvent::CallFailed {
                            peer_user_id: call_peer_user_id,
                            call_id,
                            reason: error.to_string(),
                        });
                    }
                    CallSendResolution::NoEvent => return None,
                    CallSendResolution::NotTracked => {}
                }
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
            .find(|(_, c)| c.devices.iter().any(|d| d.peer_id == *peer_id))
            .map(|(user_id, _)| user_id.clone())
    }

    fn device_curve25519_for_peer(&self, peer_id: &PeerId) -> Option<String> {
        self.contacts
            .values()
            .find_map(|c| c.devices.iter().find(|d| d.peer_id == *peer_id))
            .map(|d| d.curve25519_key.clone())
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
            request_response::Message::Response {
                request_id,
                response,
            } if !response.ack => {
                // Forget only the specific device's session that rejected
                // this, not every device of this contact — a sibling
                // device's session (if any) may still be perfectly healthy.
                if let Some(curve_key) = self.device_curve25519_for_peer(&peer) {
                    self.olm.forget_session(&curve_key);
                }
                match self.resolve_call_send(request_id, false) {
                    CallSendResolution::Failed {
                        call_id,
                        peer_user_id: call_peer_user_id,
                    } => {
                        tracing::warn!(
                            peer = %peer,
                            peer_user_id = %call_peer_user_id,
                            call_id,
                            "every device rejected call signaling (failed to decrypt) — sessions reset for retry"
                        );
                        return Some(ChatEvent::CallFailed {
                            peer_user_id: call_peer_user_id,
                            call_id,
                            reason: "the recipient couldn't decrypt this".to_string(),
                        });
                    }
                    CallSendResolution::NoEvent => return None,
                    CallSendResolution::NotTracked => {}
                }
                let peer_user_id = self.contact_user_id_for_peer(&peer);
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
            request_response::Message::Response { request_id, .. } => {
                // A successful ack for one device never itself surfaces an
                // event; the real signal is a subsequent CallAccept/CallEnd.
                self.resolve_call_send(request_id, true);
                None
            }
        }
    }

    fn handle_direct_request(
        &mut self,
        request: &ChatRequest,
    ) -> anyhow::Result<Option<ChatEvent>> {
        let envelope: DirectEnvelope = bincode::deserialize(&request.payload)?;
        let plaintext = self.olm.decrypt(&mut self.device_identity, &envelope)?;
        let payload: DirectPayload = bincode::deserialize(&plaintext)?;

        match payload {
            DirectPayload::Chat {
                message_id,
                body,
                attachment,
            } => Ok(Some(ChatEvent::DirectMessage {
                message_id,
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
            DirectPayload::CallInvite { call_id } => Ok(Some(ChatEvent::CallInvited {
                from: envelope.sender_user_id,
                call_id,
            })),
            DirectPayload::CallAccept { call_id } => Ok(Some(ChatEvent::CallAccepted {
                from: envelope.sender_user_id,
                from_device_id: envelope.sender_device_id,
                call_id,
            })),
            DirectPayload::CallDecline { call_id } => Ok(Some(ChatEvent::CallDeclined {
                from: envelope.sender_user_id,
                call_id,
            })),
            DirectPayload::CallEnd { call_id } => Ok(Some(ChatEvent::CallEnded {
                from: envelope.sender_user_id,
                call_id,
            })),
            DirectPayload::SyncRequest { since, messages } => Ok(Some(ChatEvent::SyncRequested {
                from_device_id: envelope.sender_device_id,
                since,
                messages,
            })),
            DirectPayload::SyncResponse { messages } => Ok(Some(ChatEvent::SyncCompleted {
                device_id: envelope.sender_device_id,
                messages,
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
                        message_id,
                        channel_id,
                        body,
                        attachment,
                    } => Some(ChatEvent::GroupMessage {
                        message_id,
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
