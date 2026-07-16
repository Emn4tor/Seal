use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crypto_session::AttachmentPayload;
use identity::{Identity, Keychain};
use libp2p::{Multiaddr, PeerId};
use net::DirectoryClient;
use storage::{LocalStore, StoredAttachment};
use wire_proto::{ChannelKind, ChannelRecord, GroupMember, GroupRecord, OneTimeKeyEntry};

use crate::events::ChatEvent;
use crate::node::ChatNode;
use crate::voice::{self, VoiceCallState};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

/// Attachments are capped at the application level (Discord's long-standing
/// default), enforced here so an oversized file never reaches the P2P
/// transport at all, regardless of what the transport's own size limits
/// happen to be configured to (see `net::build_behaviour`).
pub const MAX_ATTACHMENT_SIZE: usize = 25 * 1024 * 1024;

fn check_attachment_size(attachment: Option<&AttachmentPayload>) -> anyhow::Result<()> {
    if let Some(a) = attachment
        && a.data.len() > MAX_ATTACHMENT_SIZE
    {
        anyhow::bail!(
            "attachment is too large ({} MB, max {} MB)",
            a.data.len() / (1024 * 1024),
            MAX_ATTACHMENT_SIZE / (1024 * 1024)
        );
    }
    Ok(())
}

fn to_stored_attachment(a: &AttachmentPayload) -> StoredAttachment {
    StoredAttachment {
        filename: a.filename.clone(),
        mime_type: a.mime_type.clone(),
        exif_stripped: a.exif_stripped,
        data: a.data.clone(),
    }
}

/// How many one-time keys to keep published. A simplification worth being
/// explicit about: this always tops up with a fresh batch on startup rather
/// than tracking exactly how many are still unclaimed: fine at this
/// project's scale, but a long-running production deployment would want to
/// only top up when running low (`Account::stored_one_time_key_count`).
const ONE_TIME_KEY_BATCH: usize = 10;

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub channel_id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub position: i64,
}

impl From<ChannelRecord> for ChannelInfo {
    fn from(c: ChannelRecord) -> Self {
        Self {
            channel_id: c.channel_id,
            name: c.name,
            kind: c.kind,
            position: c.position,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub group_id: String,
    pub name: String,
    pub roster_version: u64,
    pub members: Vec<GroupMember>,
    pub channels: Vec<ChannelInfo>,
}

impl From<GroupRecord> for GroupInfo {
    fn from(r: GroupRecord) -> Self {
        Self {
            group_id: r.group_id,
            name: r.name,
            roster_version: r.roster_version,
            members: r.members,
            channels: r.channels.into_iter().map(ChannelInfo::from).collect(),
        }
    }
}

/// Ties identity, local encrypted storage, the directory rendezvous client,
/// and the P2P chat node into the single service a Tauri command layer (or
/// any other frontend) calls into. Nothing here is UI-specific.
pub struct AppService {
    pub node: ChatNode,
    directory: DirectoryClient,
    store: LocalStore,
    display_name: String,
    voice_call: Option<VoiceCallState>,
    voice_call_last_heartbeat: Option<std::time::Instant>,
}

impl AppService {
    /// `display_name` is only used when creating a genuinely new identity:
    /// once one is stored, its saved display name always wins over whatever
    /// is passed here, so re-asserting registration on startup can never
    /// silently rename an existing account (a caller that wants to rename
    /// deliberately should use [`Self::rename`] instead).
    pub async fn load_or_create(
        data_dir: PathBuf,
        directory_url: String,
        display_name: Option<String>,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let keychain = Keychain::for_app_data_dir(&data_dir)?;
        let kek = keychain.load_or_create_kek()?;
        let store = LocalStore::open(&data_dir.join("local.sqlite3"), kek)?;

        let stored = store.load_identity()?;
        let (mut identity, display_name) = match stored {
            Some(stored) => (
                Identity::from_pickle_json(&stored.pickle_json)?,
                stored.display_name,
            ),
            None => {
                let display_name = display_name
                    .ok_or_else(|| anyhow::anyhow!("a new account needs a display name"))?;
                (Identity::generate(), display_name)
            }
        };

        let directory = DirectoryClient::new(directory_url);

        // Registration is idempotent and cheap: always re-assert on
        // startup, since the directory server may have been purged since
        // we last ran.
        directory.register(&identity, &display_name).await?;

        let otk_result = identity
            .account_mut()
            .generate_one_time_keys(ONE_TIME_KEY_BATCH);
        let _ = otk_result; // discarded (evicted) keys, if any; nothing to clean up locally
        let keys: Vec<OneTimeKeyEntry> = identity
            .account()
            .one_time_keys()
            .into_iter()
            .map(|(id, key)| OneTimeKeyEntry {
                key_id: id.to_base64(),
                public_key: STANDARD.encode(key.as_bytes()),
            })
            .collect();
        identity.account_mut().generate_fallback_key();
        let fallback = identity
            .account()
            .fallback_key()
            .into_iter()
            .next()
            .map(|(id, key)| OneTimeKeyEntry {
                key_id: id.to_base64(),
                public_key: STANDARD.encode(key.as_bytes()),
            });
        directory
            .upload_one_time_keys(&identity, keys, fallback)
            .await?;
        identity.account_mut().mark_keys_as_published();

        store.save_identity(
            &identity.user_id(),
            &display_name,
            &identity.pickle_to_json()?,
            now(),
        )?;

        // Loopback for now, not 0.0.0.0: binding all interfaces means the
        // swarm can emit a `NewListenAddr` per interface in an
        // unpredictable order. Fine for same-machine use (including this
        // crate's own tests); real LAN/WAN reachability needs the
        // relay/autonat path this workspace already has wired at the
        // transport layer, layered in properly once there's a UI to
        // surface connectivity state through.
        //
        // TCP only for now, not QUIC: this is the first place in the
        // workspace that would actually bind a QUIC listener (earlier
        // phases only ever listened on TCP), and it needs its own look
        // before relying on it.
        let mut node = ChatNode::new(identity)?;
        node.listen_on(Multiaddr::from_str("/ip4/127.0.0.1/tcp/0")?)?;

        // One-shot presence announce at startup (not a recurring heartbeat
        // yet; a known follow-up). A 300s TTL is the server's own cap, so
        // presence naturally expires if the app runs longer than that
        // without restarting; contacts just won't find a fresh address
        // until it's renewed.
        let addr = node.wait_for_listen_addr().await;
        directory
            .put_presence(
                &node.identity,
                &node.local_peer_id().to_string(),
                vec![addr.to_string()],
                vec![],
                300,
            )
            .await?;

        // Re-subscribe to groups we're already in so we keep receiving
        // messages; our own ability to *send* in a group we created before
        // restarting currently needs a fresh outbound session (see the
        // module-level note on session persistence being a known gap).
        for group in store.list_groups()? {
            node.join_group_topic(&group.group_id);
        }

        Ok(Self {
            node,
            directory,
            store,
            display_name,
            voice_call: None,
            voice_call_last_heartbeat: None,
        })
    }

    /// Changes this account's display name: re-registers with the directory
    /// (an upsert, see `directory-server`'s `insert_user`) and re-saves
    /// locally, so it's what a later `load_or_create` finds. The only
    /// deliberate path a display name should ever change through.
    pub async fn rename(&mut self, new_display_name: String) -> anyhow::Result<()> {
        self.directory
            .register(&self.node.identity, &new_display_name)
            .await?;
        self.store.save_identity(
            &self.node.identity.user_id(),
            &new_display_name,
            &self.node.identity.pickle_to_json()?,
            now(),
        )?;
        self.display_name = new_display_name;
        Ok(())
    }

    pub fn user_id(&self) -> String {
        self.node.identity.user_id()
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn list_contacts(&self) -> anyhow::Result<Vec<storage::StoredContact>> {
        Ok(self.store.list_contacts()?)
    }

    pub fn list_groups(&self) -> anyhow::Result<Vec<storage::StoredGroup>> {
        Ok(self.store.list_groups()?)
    }

    pub fn list_messages(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Vec<storage::StoredMessage>> {
        Ok(self.store.list_messages(conversation_id)?)
    }

    /// Looks a user up on the directory, caches their public identity
    /// locally, and (if they're currently online) connects to them. Contact
    /// metadata (name/keys) is persisted; the network connection itself is
    /// re-established fresh every time it's needed, since a peer's address
    /// is ephemeral session data, not identity data.
    pub async fn add_contact_by_user_id(&mut self, user_id: &str) -> anyhow::Result<()> {
        let user = self.directory.get_user(user_id).await?;
        self.store.upsert_contact(
            &user.user_id,
            &user.display_name,
            &user.ed25519_key,
            &user.curve25519_key,
        )?;

        let presence = self.directory.get_presence(user_id).await?;
        let peer_id = PeerId::from_str(&presence.peer_id)
            .map_err(|e| anyhow::anyhow!("contact published an invalid peer id: {e}"))?;
        let addrs: Vec<Multiaddr> = presence
            .multiaddrs
            .iter()
            .filter_map(|addr| addr.parse::<Multiaddr>().ok())
            .collect();
        // Deliberately not also calling `node.dial(...)` here: request-response
        // dials lazily off `Contact::addrs` when a message is actually sent
        // (via `send_request_with_addresses`). A second, independent dial to
        // the same peer racing that one is exactly what caused a hang here
        // during development: simultaneous-connect tie-breaking closed one
        // of the two connections, and the queued request never made it onto
        // the surviving one.
        self.node
            .add_contact(&user.user_id, &user.curve25519_key, peer_id, addrs);
        Ok(())
    }

    async fn ensure_connected_contact(&mut self, user_id: &str) -> anyhow::Result<()> {
        if !self.node.has_contact(user_id) {
            self.add_contact_by_user_id(user_id).await?;
        }
        Ok(())
    }

    async fn ensure_direct_session(&mut self, peer_user_id: &str) -> anyhow::Result<()> {
        if self.node.has_direct_session(peer_user_id) {
            return Ok(());
        }
        let otk = self.directory.claim_one_time_key(peer_user_id).await?;
        self.node
            .ensure_outbound_session(peer_user_id, &otk.public_key)?;
        Ok(())
    }

    pub async fn send_direct_message(
        &mut self,
        peer_user_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        check_attachment_size(attachment.as_ref())?;
        self.ensure_connected_contact(peer_user_id).await?;
        self.ensure_direct_session(peer_user_id).await?;
        self.node
            .send_direct_message(peer_user_id, body, attachment.clone())?;
        self.store.insert_message(
            peer_user_id,
            &self.user_id(),
            body,
            attachment.as_ref().map(to_stored_attachment).as_ref(),
            now(),
        )?;
        Ok(())
    }

    pub async fn create_group(&mut self, name: &str) -> anyhow::Result<GroupInfo> {
        let group_id = uuid::Uuid::new_v4().to_string();
        let record = self
            .directory
            .create_group(&self.node.identity, &group_id, name)
            .await?;
        self.node.create_group(&group_id);
        self.persist_group(&record)?;
        Ok(record.into())
    }

    pub async fn invite_to_group(
        &mut self,
        group_id: &str,
        member_user_id: &str,
    ) -> anyhow::Result<GroupInfo> {
        let current = self.directory.get_group(group_id).await?;
        let updated = self
            .directory
            .update_roster(
                &self.node.identity,
                group_id,
                vec![member_user_id.to_string()],
                vec![],
                current.roster_version,
            )
            .await?;
        self.persist_group(&updated)?;

        self.ensure_connected_contact(member_user_id).await?;
        self.ensure_direct_session(member_user_id).await?;
        self.node.share_group_key(group_id, member_user_id)?;
        Ok(updated.into())
    }

    pub async fn send_group_message(
        &mut self,
        group_id: &str,
        channel_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        check_attachment_size(attachment.as_ref())?;
        self.node
            .send_group_message(group_id, channel_id, body, attachment.clone())?;
        let conversation_id = format!("{group_id}:{channel_id}");
        self.store.insert_message(
            &conversation_id,
            &self.user_id(),
            body,
            attachment.as_ref().map(to_stored_attachment).as_ref(),
            now(),
        )?;
        Ok(())
    }

    /// Owner-only (enforced server-side, see
    /// `directory-server`'s `create_channel` route). Every channel is
    /// visible to every group member in this phase (no per-channel
    /// permissions), so this doesn't touch Megolm/gossipsub at all; it
    /// only adds a routable id, persisted like any other group metadata.
    pub async fn create_channel(
        &mut self,
        group_id: &str,
        name: &str,
        kind: ChannelKind,
    ) -> anyhow::Result<GroupInfo> {
        let channel_id = uuid::Uuid::new_v4().to_string();
        self.directory
            .create_channel(&self.node.identity, group_id, &channel_id, name, kind)
            .await?;
        // Re-fetch the whole group so the new channel is persisted
        // alongside everything else, with server-assigned position intact,
        // and the caller gets back the same shape `list_groups` uses.
        let group = self.directory.get_group(group_id).await?;
        self.persist_group(&group)?;
        Ok(group.into())
    }

    /// Call this once we learn we've been added to a group (e.g. on
    /// receiving `ChatEvent::GroupKeyReceived`): fetches the roster from
    /// the directory and subscribes to the group's gossipsub topic so we
    /// actually receive its messages.
    pub async fn accept_group_invite(&mut self, group_id: &str) -> anyhow::Result<GroupInfo> {
        let record = self.directory.get_group(group_id).await?;
        self.persist_group(&record)?;
        // Always safe to repeat: this fires on *every* `GroupKeyReceived`
        // for this group (e.g. once per fellow member's key-share below,
        // not just "the" invite), and subscribing when already subscribed
        // is a no-op.
        self.node.join_group_topic(group_id);

        // Establishing our own outbound session (and sharing it with
        // everyone else) must happen exactly once, not on every
        // `GroupKeyReceived`, otherwise each member's own key-share would
        // trigger the recipient to rotate *their* key in response, which
        // would trigger the same on the other side, forever. Without this
        // guard, only the group's original creator would ever be able to
        // send anything at all (`Megolm::encrypt` fails with "unknown
        // group session" for anyone who only ever received an inbound
        // key): that isn't a real group chat, it just went unnoticed
        // until voice presence needed *every* participant to broadcast,
        // not only the owner.
        if !self.node.has_outbound_group_session(group_id) {
            self.node.create_group(group_id);

            let my_id = self.user_id();
            for member in &record.members {
                if member.user_id == my_id {
                    continue;
                }
                if let Err(e) = self.ensure_connected_contact(&member.user_id).await {
                    tracing::warn!(error = %e, member = %member.user_id, "failed to connect to a fellow group member");
                    continue;
                }
                if let Err(e) = self.ensure_direct_session(&member.user_id).await {
                    tracing::warn!(error = %e, member = %member.user_id, "failed to establish a session with a fellow group member");
                    continue;
                }
                if let Err(e) = self.node.share_group_key(group_id, &member.user_id) {
                    tracing::warn!(error = %e, member = %member.user_id, "failed to share our group key with a fellow member");
                }
            }
        }

        Ok(record.into())
    }

    fn persist_group(&self, record: &GroupRecord) -> anyhow::Result<()> {
        let members: Vec<(String, String)> = record
            .members
            .iter()
            .map(|m| (m.user_id.clone(), m.role.as_str().to_string()))
            .collect();
        let channels: Vec<(String, String, String, i64)> = record
            .channels
            .iter()
            .map(|c| {
                (
                    c.channel_id.clone(),
                    c.name.clone(),
                    c.kind.as_str().to_string(),
                    c.position,
                )
            })
            .collect();
        self.store.upsert_group(
            &record.group_id,
            &record.name,
            record.roster_version,
            &members,
            &channels,
        )?;
        Ok(())
    }

    /// Drives the node and persists messages as they arrive. Callers loop
    /// on this; it's what a Tauri background task forwards to the
    /// frontend as window events.
    pub async fn next_event(&mut self) -> ChatEvent {
        loop {
            let event = self.node.next_event().await;
            match event {
                ChatEvent::DirectMessage {
                    ref from,
                    ref body,
                    ref attachment,
                } => {
                    let _ = self.store.insert_message(
                        from,
                        from,
                        body,
                        attachment.as_ref().map(to_stored_attachment).as_ref(),
                        now(),
                    );
                    // A message arriving from someone we haven't looked up
                    // yet (they messaged us first): fetch their public
                    // identity so they show up as a contact instead of a
                    // bare user_id.
                    if !self.node.has_contact(from)
                        && let Err(e) = self.add_contact_by_user_id(&from.clone()).await
                    {
                        tracing::warn!(error = %e, from = %from, "failed to look up an unknown sender");
                    }
                    return event;
                }
                ChatEvent::GroupMessage {
                    ref group_id,
                    ref channel_id,
                    ref from,
                    ref body,
                    ref attachment,
                } => {
                    let conversation_id = format!("{group_id}:{channel_id}");
                    let _ = self.store.insert_message(
                        &conversation_id,
                        from,
                        body,
                        attachment.as_ref().map(to_stored_attachment).as_ref(),
                        now(),
                    );
                    return event;
                }
                ChatEvent::GroupKeyReceived { ref group_id, .. } => {
                    // We've just been handed the ability to decrypt this
                    // group: fetch its roster and subscribe so messages
                    // actually arrive.
                    if let Err(e) = self.accept_group_invite(&group_id.clone()).await {
                        tracing::warn!(error = %e, group_id = %group_id, "failed to accept a group invite");
                    }
                    return event;
                }
                ChatEvent::VoicePresence {
                    group_id,
                    channel_id,
                    from,
                    joined,
                } => {
                    if let Some(translated) =
                        self.handle_voice_presence(group_id, channel_id, from, joined).await
                    {
                        return translated;
                    }
                    // Irrelevant to any call we're currently in, not worth
                    // surfacing, loop around for the next real event.
                }
                other => return other,
            }
        }
    }

    /// Reacts to another member's voice-channel join/leave announcement:
    /// updates the active call's participant set (if we're actually in
    /// that channel's call) and, on a fresh join, resolves and dials them.
    /// Returns `None` when the announcement isn't relevant to us right now
    /// (wrong channel, no active call, or our own echoed announcement).
    async fn handle_voice_presence(
        &mut self,
        group_id: String,
        channel_id: String,
        from: String,
        joined: bool,
    ) -> Option<ChatEvent> {
        if from == self.user_id() {
            return None;
        }
        let relevant = self
            .voice_call
            .as_ref()
            .is_some_and(|c| c.group_id == group_id && c.channel_id == channel_id);
        if !relevant {
            return None;
        }

        let changed = self.voice_call.as_ref()?.note_presence(&from, joined);

        if joined && let Err(e) = self.connect_voice_peer(&from).await {
            tracing::warn!(error = %e, user_id = %from, "failed to connect to a voice-channel participant");
        }

        if !changed {
            return None;
        }
        let user_ids = self.voice_call.as_ref()?.participants();
        Some(ChatEvent::VoiceParticipantsChanged {
            group_id,
            channel_id,
            user_ids,
        })
    }

    /// Resolves a voice-channel participant's current network address via
    /// the directory (the same lookup `add_contact_by_user_id` does, just
    /// without persisting them as a contact: a fellow voice participant
    /// isn't necessarily someone we've 1:1-messaged) and opens a stream to
    /// them if we're the initiating side of the pair.
    async fn connect_voice_peer(&mut self, user_id: &str) -> anyhow::Result<()> {
        let presence = self.directory.get_presence(user_id).await?;
        let peer_id = PeerId::from_str(&presence.peer_id)
            .map_err(|e| anyhow::anyhow!("participant published an invalid peer id: {e}"))?;
        let addrs: Vec<Multiaddr> = presence
            .multiaddrs
            .iter()
            .filter_map(|addr| addr.parse::<Multiaddr>().ok())
            .collect();
        self.node.register_peer_address(peer_id, &addrs);

        if let Some(call) = self.voice_call.as_ref() {
            call.note_peer_identity(peer_id, user_id.to_string());
        }

        // Tie-break so both sides don't race to dial: the lexicographically
        // smaller user_id initiates: the other side will see our own
        // presence announcement and connect to us instead.
        //
        // `spawn_ensure_connected`, not the awaited `ensure_connected`: this
        // whole method runs inside `next_event`'s own call chain (reacting
        // to a `VoicePresence` it just received), and awaiting the dial
        // here would deadlock; see `VoiceCallState::spawn_ensure_connected`.
        if self.user_id().as_str() < user_id
            && let Some(call) = self.voice_call.as_ref()
        {
            call.spawn_ensure_connected(peer_id, user_id.to_string());
        }
        Ok(())
    }

    /// Joins a voice channel: starts local audio I/O + the mesh-dialing
    /// machinery and announces our presence to the rest of the group. Only
    /// one call is active at a time: joining a different channel first
    /// leaves whichever one we were already in.
    pub async fn join_voice_channel(&mut self, group_id: &str, channel_id: &str) -> anyhow::Result<()> {
        if self.voice_call.is_some() {
            self.leave_voice_channel()?;
        }
        let control = self.node.voice_control();
        let call = VoiceCallState::start(group_id.to_string(), channel_id.to_string(), control).await?;
        // A failure here (e.g. gossipsub's mesh for this topic hasn't
        // finished forming yet) is transient and shouldn't block joining
        // the call itself; leaving `voice_call_last_heartbeat` unset makes
        // the very next `maybe_send_voice_heartbeat` retry immediately
        // rather than waiting out a full heartbeat interval.
        self.voice_call_last_heartbeat = match self.node.send_voice_presence(group_id, channel_id, true) {
            Ok(()) => Some(std::time::Instant::now()),
            Err(e) => {
                tracing::warn!(error = %e, group_id, channel_id, "initial voice presence announce failed, will retry");
                None
            }
        };
        self.voice_call = Some(call);
        Ok(())
    }

    /// Leaves whatever voice channel we're currently in: a no-op if we're
    /// not in one. Announces our departure so other participants' UIs
    /// update immediately rather than waiting for our heartbeat to lapse.
    pub fn leave_voice_channel(&mut self) -> anyhow::Result<()> {
        let Some(call) = self.voice_call.take() else {
            return Ok(());
        };
        // Local cleanup (dropping `call` above tears down audio I/O and
        // every open stream) already happened regardless of whether this
        // announce makes it out; a lost departure announcement just means
        // other participants find out we're gone when our heartbeat lapses
        // instead of immediately, not that we failed to leave.
        if let Err(e) = self.node.send_voice_presence(&call.group_id, &call.channel_id, false) {
            tracing::warn!(error = %e, "failed to announce leaving the voice channel");
        }
        self.voice_call_last_heartbeat = None;
        Ok(())
    }

    pub fn set_voice_changer_enabled(&self, enabled: bool) {
        if let Some(call) = &self.voice_call {
            call.set_changer_enabled(enabled);
        }
    }

    pub fn set_mic_muted(&self, muted: bool) {
        if let Some(call) = &self.voice_call {
            call.set_muted(muted);
        }
    }

    pub fn voice_participants(&self) -> Vec<String> {
        self.voice_call.as_ref().map(|c| c.participants()).unwrap_or_default()
    }

    /// The noise-gate threshold (dBFS) below which captured mic audio is
    /// never sent at all; also what drives the speaking indicators (see
    /// `voice::VoiceCallState`'s module docs for why the receive side needs
    /// no separate detection of its own).
    pub fn set_mic_threshold_db(&self, db: f32) {
        if let Some(call) = &self.voice_call {
            call.set_mic_threshold_db(db);
        }
    }

    pub fn set_hear_self(&self, enabled: bool) {
        if let Some(call) = &self.voice_call {
            call.set_hear_self(enabled);
        }
    }

    /// User ids currently speaking in the active voice call, including
    /// ourselves: a single combined list so callers don't need to special-
    /// case "am I speaking" separately from everyone else.
    pub fn voice_speaking_participants(&self) -> Vec<String> {
        let Some(call) = &self.voice_call else {
            return Vec::new();
        };
        let mut ids = call.speaking_participants();
        if call.is_local_speaking() {
            ids.push(self.user_id());
        }
        ids
    }

    /// Re-announces our presence in the active voice channel if the
    /// heartbeat interval has elapsed; cheap to call often (e.g. from the
    /// Tauri actor's own polling loop); only actually sends when due. A
    /// no-op if we're not in a call.
    pub fn maybe_send_voice_heartbeat(&mut self) {
        let Some(call) = self.voice_call.as_ref() else {
            return;
        };
        let due = self
            .voice_call_last_heartbeat
            .is_none_or(|last| last.elapsed() >= voice::PRESENCE_HEARTBEAT);
        if !due {
            return;
        }
        // Only push the timestamp forward on success. A failure (e.g. the
        // gossipsub mesh for this topic hasn't finished forming yet) should
        // be retried again on the *next* call shortly after, not made to
        // wait out a full heartbeat interval, otherwise a slow-forming
        // mesh could mean only one or two real attempts in a given window
        // instead of fast retries until it's ready.
        match self.node.send_voice_presence(&call.group_id, &call.channel_id, true) {
            Ok(()) => self.voice_call_last_heartbeat = Some(std::time::Instant::now()),
            Err(e) => tracing::warn!(error = %e, "failed to send voice presence heartbeat, will retry"),
        }
    }
}
