use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crypto_session::{AttachmentPayload, SyncMessage};
use identity::{Identity, Keychain};
use libp2p::{Multiaddr, PeerId};
use net::DirectoryClient;
use storage::{LocalStore, StoredAttachment, StoredContactDevice};
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

fn to_sync_message(m: storage::StoredMessage) -> SyncMessage {
    SyncMessage {
        message_id: m.message_id,
        conversation_id: m.conversation_id,
        sender_user_id: m.sender_user_id,
        body: m.body,
        attachment: m.attachment.map(|a| AttachmentPayload {
            filename: a.filename,
            mime_type: a.mime_type,
            exif_stripped: a.exif_stripped,
            data: a.data,
        }),
        sent_at: m.sent_at,
    }
}

/// How many one-time keys to keep published. Always tops up on startup
/// rather than tracking unclaimed count; fine at this project's scale.
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

/// Everything a QR code needs to encode for another device to find and
/// pair with this account — returned by `start_pairing`, parsed back out
/// of the scanned QR and passed to `join_via_pairing` on the other end.
#[derive(Debug, Clone)]
pub struct PairingOffer {
    pub user_id: String,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub relay_addrs: Vec<String>,
    pub token: String,
    pub ephemeral_pubkey: [u8; 32],
    pub expires_at: i64,
}

/// What the joining side's network exchange comes back with — everything
/// `join_via_pairing` needs to seed a fresh data directory before
/// `load_or_create` can resume as the paired account.
pub struct PairingResult {
    device_id: String,
    device_identity: Identity,
    payload: net::PairingPayload,
}

/// One entry of `AppService::list_my_devices` — the "Devices" settings
/// screen's whole data model.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub device_id: String,
    pub is_this_device: bool,
    pub online: bool,
}

/// Ties identity, local storage, the directory client, and the P2P chat
/// node into the single service the Tauri command layer calls into. Aborts
/// the presence-heartbeat task when dropped (e.g. account switch).
struct HeartbeatGuard(tokio::task::JoinHandle<()>);

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Which side of a 1:1 call proposal we are — determines which incoming
/// signaling messages are meaningful for it (an `Outgoing` call only cares
/// about `CallAccept`/`CallDecline`; an `Incoming` one is waiting on the
/// local user to call `accept_call`/`decline_call`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallDirection {
    Outgoing,
    Incoming,
}

/// A 1:1 call proposed but not yet connected, either direction. Distinct
/// from `voice_call`, which only exists once audio is flowing.
struct PendingCall {
    call_id: String,
    peer_user_id: String,
    direction: CallDirection,
    /// Captured at `call_contact` time (for `Outgoing`) so `handle_call_accepted`
    /// can start audio with the same device preferences the frontend had
    /// selected when the user initiated the call — unset for `Incoming`,
    /// where the frontend only supplies these later, at `accept_call` time.
    preferred_input: Option<String>,
    preferred_output: Option<String>,
}

pub struct AppService {
    pub node: ChatNode,
    directory: DirectoryClient,
    store: LocalStore,
    display_name: String,
    voice_call: Option<VoiceCallState>,
    voice_call_last_heartbeat: Option<std::time::Instant>,
    pending_call: Option<PendingCall>,
    /// Who's currently announced as present in each voice channel, keyed by
    /// `channel_id` (globally unique, a UUID — no need to also key by
    /// `group_id`) — tracked for *every* channel we're subscribed to, not
    /// just whichever one we're actively in a call in. `VoicePresence` rides
    /// the group's regular gossipsub topic (see `node::send_voice_presence`),
    /// which we're already subscribed to for every group we're a member of,
    /// so this costs nothing extra on the wire — it just means no longer
    /// discarding an announcement as irrelevant when it isn't for our own
    /// active call. What this enables: showing who's in a voice channel
    /// *before* joining it, instead of only finding out after.
    voice_channel_presence: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// Whether the directory-server presence heartbeat currently marks this
    /// account as visible to contacts as "online". Read fresh on every
    /// heartbeat tick by the closure passed to
    /// `net::presence::run_presence_heartbeat_loop`, so flipping this takes
    /// effect on the next heartbeat, no restart needed. Doesn't affect
    /// whether the heartbeat itself keeps running: that's still required
    /// regardless, since it's also how contacts learn how to reach this
    /// peer at all.
    share_online_status: Arc<AtomicBool>,
    _presence_heartbeat: HeartbeatGuard,
    /// This device's own dialable address info as of startup — the same
    /// values already given to presence, kept around so `start_pairing`
    /// can put them in a QR code without re-deriving them.
    known_addrs: Vec<String>,
    known_relay_addrs: Vec<String>,
    /// The contacts snapshot for whichever pairing offer is currently
    /// outstanding — consumed the moment a matching
    /// `ChatEvent::PairingRequested` arrives, see `next_event`.
    pending_pairing_bootstrap: Option<net::PairingBootstrap>,
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
        Self::load_or_create_with(data_dir, directory_url, display_name, false).await
    }

    /// Like `load_or_create`, but with `simulate_wan` exposed for testing:
    /// makes this account withhold its LAN address from presence so it's
    /// reachable only through the relay, exercising the relay/dcutr path
    /// even when both instances are on one machine. Real code should use
    /// `load_or_create` instead.
    pub async fn load_or_create_with(
        data_dir: PathBuf,
        directory_url: String,
        display_name: Option<String>,
        simulate_wan: bool,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let keychain = Keychain::for_app_data_dir(&data_dir)?;
        // Reading (or, on first run, creating) the KEK can now involve a
        // blocking, interactive Touch ID/password prompt on macOS (see
        // `identity::Keychain`) — run it off the async runtime's worker
        // threads so a slow or ignored prompt can't tie one up.
        let kek = tokio::task::spawn_blocking(move || keychain.load_or_create_kek())
            .await
            .expect("keychain blocking task panicked")?;
        let store = LocalStore::open(&data_dir.join("local.sqlite3"), kek)?;

        let stored = store.load_identity()?;
        let (identity, display_name) = match stored {
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

        let directory = DirectoryClient::new(directory_url.clone());

        // Registration is idempotent and cheap: always re-assert on
        // startup, since the directory server may have been purged since
        // we last ran.
        directory.register(&identity, &display_name).await?;

        // Load this account's persisted libp2p transport keypair and
        // device_id, or mint and save both on first run — otherwise every
        // launch would hand out a fresh PeerId, stranding existing contacts.
        let p2p_keypair = match store.load_p2p_keypair()? {
            Some(bytes) => libp2p::identity::Keypair::from_protobuf_encoding(&bytes)
                .map_err(|e| anyhow::anyhow!("stored libp2p keypair is corrupt: {e}"))?,
            None => {
                let keypair = libp2p::identity::Keypair::generate_ed25519();
                store.save_p2p_keypair(&keypair.to_protobuf_encoding()?, now())?;
                keypair
            }
        };
        let device_id = match store.load_device_id()? {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                store.save_device_id(&id)?;
                id
            }
        };
        // This device's own Olm account, deliberately distinct from the
        // account's master `identity` even on the very first device.
        let mut device_identity = match store.load_device_olm_pickle()? {
            Some(pickle_json) => Identity::from_pickle_json(&pickle_json)?,
            None => {
                let fresh = Identity::generate();
                store.save_device_olm_pickle(&fresh.pickle_to_json()?)?;
                fresh
            }
        };

        // OTKs are generated for *this device's* account — the master
        // identity never participates in Olm sessions directly.
        let otk_result = device_identity
            .account_mut()
            .generate_one_time_keys(ONE_TIME_KEY_BATCH);
        let _ = otk_result; // discarded (evicted) keys, if any; nothing to clean up locally
        let keys: Vec<OneTimeKeyEntry> = device_identity
            .account()
            .one_time_keys()
            .into_iter()
            .map(|(id, key)| OneTimeKeyEntry {
                key_id: id.to_base64(),
                public_key: STANDARD.encode(key.as_bytes()),
            })
            .collect();
        device_identity.account_mut().generate_fallback_key();
        let fallback = device_identity
            .account()
            .fallback_key()
            .into_iter()
            .next()
            .map(|(id, key)| OneTimeKeyEntry {
                key_id: id.to_base64(),
                public_key: STANDARD.encode(key.as_bytes()),
            });
        // Signed/authorized by the master `identity` (directory writes are
        // always master-authorized), but the keys themselves came from
        // `device_identity`'s account above.
        directory
            .upload_one_time_keys(&identity, &device_id, keys, fallback)
            .await?;
        device_identity.account_mut().mark_keys_as_published();
        store.save_device_olm_pickle(&device_identity.pickle_to_json()?)?;

        // Registers this device's own certificate, signed by the master
        // key. Idempotent, so safe to re-assert on every launch.
        let self_device_cert = {
            let mut cert = wire_proto::DeviceCertificate {
                device_id: device_id.clone(),
                device_ed25519_key: device_identity.ed25519_public_base64(),
                device_curve25519_key: device_identity.curve25519_public_base64(),
                master_ed25519_key: identity.ed25519_public_base64(),
                signature: String::new(),
            };
            cert.signature = identity.sign(&cert.signing_bytes());
            cert
        };
        directory
            .register_device(&identity, self_device_cert)
            .await?;

        let pickle_json = identity.pickle_to_json()?;
        store.save_identity(&identity.user_id(), &display_name, &pickle_json, now())?;

        // Bind 0.0.0.0 (not loopback) so two machines on the same LAN can
        // reach each other; every non-loopback address that comes up gets
        // advertised below, since the dial side already tries every
        // candidate on a contact. Peers behind a NAT still need the
        // relay/autonat path handled further down.

        // TCP only for now, not QUIC: this is the first place in the
        // workspace that would actually bind a QUIC listener (earlier
        // phases only ever listened on TCP), and it needs its own look
        // before relying on it.
        let mut node =
            ChatNode::with_keypair(identity, p2p_keypair, device_id.clone(), device_identity)?;
        node.listen_on(Multiaddr::from_str("/ip4/0.0.0.0/tcp/0")?)?;

        // First announce at startup, immediately followed below by a
        // recurring heartbeat so it doesn't just expire after the server's
        // 300s TTL cap.
        let listen_addrs = node
            .wait_for_listen_addrs(std::time::Duration::from_millis(300))
            .await;
        // Advertise every listening address, loopback included: two
        // instances on the same machine (the local two-account test flow)
        // can have a LAN address present but temporarily undialable (e.g.
        // macOS's Local Network permission), where loopback still works.
        // A remote peer just can't reach loopback and falls through to the
        // other candidates, so including it is a free fallback.
        //
        // `simulate_wan` skips this and advertises nothing, mimicking a
        // peer with no directly-dialable address at all (see this
        // method's doc comment).
        let addrs: Vec<String> = if simulate_wan {
            Vec::new()
        } else {
            listen_addrs.iter().map(|a| a.to_string()).collect()
        };
        let peer_id_str = node.local_peer_id().to_string();

        // Best-effort NAT-traversal fallback: reserve a circuit through the
        // directory's relay (if any) so we're still reachable when nobody
        // can dial `addrs` directly — the common case for peers behind
        // different NATs. `dcutr` then tries to upgrade to a direct
        // connection on its own; any failure here just falls back to
        // LAN-only reachability, not a startup failure.
        //
        // Must happen here, blocking, before the actor loop starts driving
        // `node`'s swarm (single-threaded, see below), so the timeout is
        // kept short to avoid a visibly hanging launch when the relay is
        // unreachable.
        let relay_addrs: Vec<String> = match directory.get_relay_info().await {
            Ok(info) => match info.multiaddr.parse::<Multiaddr>() {
                Ok(relay_addr) => match node
                    .reserve_relay_circuit(relay_addr, std::time::Duration::from_secs(3))
                    .await
                {
                    Ok(circuit_addr) => vec![circuit_addr.to_string()],
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to reserve a relay circuit; falling back to LAN-only reachability");
                        vec![]
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, multiaddr = %info.multiaddr, "directory server returned an unparseable relay multiaddr");
                    vec![]
                }
            },
            Err(e) => {
                tracing::debug!(error = %e, "no relay available from the directory server");
                vec![]
            }
        };

        // Enabled by default; the frontend re-asserts the user's actual
        // saved preference right after startup via `set_share_online_status`
        // (see `apps/desktop`'s `onlineStatusSettings.ts`), same pattern as
        // the mic threshold and other locally-persisted settings that need
        // to reach backend state.
        let share_online_status = Arc::new(AtomicBool::new(true));

        directory
            .put_presence(
                &node.identity,
                &device_id,
                &peer_id_str,
                addrs.clone(),
                relay_addrs.clone(),
                share_online_status.load(Ordering::Relaxed),
                300,
            )
            .await?;

        // Recurring heartbeat so presence survives past the one-shot
        // announce's 300s TTL. Runs on its own task since `AppService` is
        // single-threaded (see `apps/desktop/src-tauri/src/actor.rs`), so
        // it re-announces the same addresses captured above rather than
        // re-querying the swarm. A second `Identity` is built from the
        // same pickle since `Identity` (wrapping a non-`Clone`
        // `vodozemac::olm::Account`) can't just be shared. 150s keeps a
        // healthy margin before the 300s TTL expires.
        let heartbeat_identity = std::sync::Arc::new(Identity::from_pickle_json(&pickle_json)?);
        let heartbeat_device_id = device_id.clone();
        // Kept around (not just moved into the heartbeat closure below) so
        // `start_pairing` can reuse the same dialable-address info for a
        // pairing offer's QR code without re-deriving it.
        let known_addrs = addrs.clone();
        let known_relay_addrs = relay_addrs.clone();
        let heartbeat_addrs = addrs;
        let heartbeat_relay_addrs = relay_addrs;
        let heartbeat_share_online_status = share_online_status.clone();
        let heartbeat_handle = tokio::spawn(net::presence::run_presence_heartbeat_loop(
            directory_url,
            heartbeat_identity,
            heartbeat_device_id,
            peer_id_str,
            move || heartbeat_addrs.clone(),
            move || heartbeat_relay_addrs.clone(),
            move || heartbeat_share_online_status.load(Ordering::Relaxed),
            std::time::Duration::from_secs(150),
        ));

        // Re-subscribe to groups we're already in so we keep receiving
        // messages; our own ability to *send* in a group we created before
        // restarting currently needs a fresh outbound session (see the
        // module-level note on session persistence being a known gap).
        for group in store.list_groups()? {
            node.join_group_topic(&group.group_id);
        }

        let mut svc = Self {
            node,
            directory,
            store,
            display_name,
            voice_call: None,
            voice_call_last_heartbeat: None,
            pending_call: None,
            voice_channel_presence: std::collections::HashMap::new(),
            share_online_status,
            _presence_heartbeat: HeartbeatGuard(heartbeat_handle),
            known_addrs,
            known_relay_addrs,
            pending_pairing_bootstrap: None,
        };
        svc.discover_missing_groups().await;
        Ok(svc)
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

    /// Starts offering to pair a new device: mints a one-time token and
    /// snapshots current contacts so the joining device can message them
    /// immediately. A second call before the first is answered replaces it.
    pub fn start_pairing(&mut self) -> anyhow::Result<PairingOffer> {
        let contacts = self.store.list_contacts()?;
        let mut pairing_contacts = Vec::with_capacity(contacts.len());
        for c in contacts {
            let devices = self
                .store
                .list_contact_devices(&c.user_id)?
                .into_iter()
                .map(|d| wire_proto::DeviceCertificate {
                    device_id: d.device_id,
                    device_ed25519_key: d.device_ed25519_key,
                    device_curve25519_key: d.device_curve25519_key,
                    master_ed25519_key: c.ed25519_key.clone(),
                    signature: d.cert_signature,
                })
                .collect();
            pairing_contacts.push(net::PairingContact {
                user_id: c.user_id,
                display_name: c.display_name,
                ed25519_key: c.ed25519_key,
                curve25519_key: c.curve25519_key,
                verified: c.verified,
                devices,
            });
        }
        self.pending_pairing_bootstrap = Some(net::PairingBootstrap {
            contacts: pairing_contacts,
        });
        let (token, ephemeral_pubkey) = self.node.start_pairing_offer();
        Ok(PairingOffer {
            user_id: self.user_id(),
            peer_id: self.node.local_peer_id().to_string(),
            multiaddrs: self.known_addrs.clone(),
            relay_addrs: self.known_relay_addrs.clone(),
            token,
            ephemeral_pubkey,
            expires_at: now() + net::PAIRING_TOKEN_TTL_SECS as i64,
        })
    }

    /// Runs the joining side of a pairing exchange against a scanned QR
    /// payload, using a throwaway identity. Only runs the network exchange;
    /// `join_via_pairing` is what seeds a data directory with the result.
    pub async fn complete_pairing(offer: &PairingOffer) -> anyhow::Result<PairingResult> {
        let peer_id: PeerId = offer
            .peer_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid peer id in pairing offer: {e}"))?;
        let addrs: Vec<Multiaddr> = offer
            .multiaddrs
            .iter()
            .chain(offer.relay_addrs.iter())
            .filter_map(|a| a.parse().ok())
            .collect();
        if addrs.is_empty() {
            anyhow::bail!("pairing offer has no dialable addresses");
        }

        let device_id = uuid::Uuid::new_v4().to_string();
        let device_identity = Identity::generate();
        let device_ed25519_key = device_identity.ed25519_public_base64();
        let device_curve25519_key = device_identity.curve25519_public_base64();

        let mut node = ChatNode::new(Identity::generate())?;
        node.request_pairing(
            peer_id,
            addrs,
            offer.token.clone(),
            offer.ephemeral_pubkey,
            device_id.clone(),
            device_ed25519_key,
            device_curve25519_key,
        );

        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(net::PAIRING_TOKEN_TTL_SECS);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("timed out waiting for the inviting device to respond");
            }
            let event = tokio::time::timeout(remaining, node.next_event())
                .await
                .map_err(|_| {
                    anyhow::anyhow!("timed out waiting for the inviting device to respond")
                })?;
            match event {
                ChatEvent::PairingCompleted(payload) => {
                    return Ok(PairingResult {
                        device_id,
                        device_identity,
                        payload: *payload,
                    });
                }
                ChatEvent::PairingFailed(reason) => anyhow::bail!("pairing failed: {reason}"),
                // Transport noise (e.g. `Connected` while the dial is still
                // settling) — irrelevant to pairing specifically, keep
                // waiting for the actual response.
                _ => continue,
            }
        }
    }

    /// `complete_pairing` plus writing its result into a fresh data
    /// directory, then resuming through the normal `load_or_create` entrypoint.
    pub async fn join_via_pairing(
        data_dir: PathBuf,
        directory_url: String,
        offer: &PairingOffer,
    ) -> anyhow::Result<Self> {
        let result = Self::complete_pairing(offer).await?;

        std::fs::create_dir_all(&data_dir)?;
        let keychain = Keychain::for_app_data_dir(&data_dir)?;
        let kek = tokio::task::spawn_blocking(move || keychain.load_or_create_kek())
            .await
            .expect("keychain blocking task panicked")?;
        let store = LocalStore::open(&data_dir.join("local.sqlite3"), kek)?;

        let identity = Identity::from_pickle_json(&result.payload.master_identity_pickle_json)?;
        store.save_identity(
            &identity.user_id(),
            &result.payload.display_name,
            &result.payload.master_identity_pickle_json,
            now(),
        )?;

        // A fresh libp2p transport keypair for this device — needs *some*
        // row to attach `device_id`/the device Olm pickle to first.
        let p2p_keypair = libp2p::identity::Keypair::generate_ed25519();
        store.save_p2p_keypair(&p2p_keypair.to_protobuf_encoding()?, now())?;
        store.save_device_id(&result.device_id)?;
        store.save_device_olm_pickle(&result.device_identity.pickle_to_json()?)?;

        for contact in &result.payload.bootstrap.contacts {
            store.upsert_contact(
                &contact.user_id,
                &contact.display_name,
                &contact.ed25519_key,
                &contact.curve25519_key,
                now(),
            )?;
            let devices: Vec<StoredContactDevice> = contact
                .devices
                .iter()
                .map(|d| StoredContactDevice {
                    device_id: d.device_id.clone(),
                    device_ed25519_key: d.device_ed25519_key.clone(),
                    device_curve25519_key: d.device_curve25519_key.clone(),
                    cert_signature: d.signature.clone(),
                })
                .collect();
            store.replace_contact_devices(&contact.user_id, &devices, now())?;
        }
        drop(store);

        Self::load_or_create(data_dir, directory_url, None).await
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

    /// Every device registered to this account, annotated with whether
    /// it's this device and whether it's currently reachable.
    pub async fn list_my_devices(&mut self) -> anyhow::Result<Vec<DeviceInfo>> {
        let my_id = self.user_id();
        let my_device_id = self.node.device_id().to_string();
        let device_certs = self.directory.get_devices(&my_id).await?;
        let presences = self.directory.get_presence_all(&my_id).await?;
        Ok(device_certs
            .into_iter()
            .map(|cert| {
                let online = presences.iter().any(|p| p.device_id == cert.device_id);
                DeviceInfo {
                    is_this_device: cert.device_id == my_device_id,
                    device_id: cert.device_id,
                    online,
                }
            })
            .collect())
    }

    /// Whether contacts can currently see this account as "online". Takes
    /// effect on the next presence heartbeat (at most 150s), not
    /// immediately: the setting only changes what the *next* heartbeat
    /// asserts, it doesn't retroactively edit the directory server's
    /// already-stored record.
    pub fn set_share_online_status(&self, enabled: bool) {
        self.share_online_status.store(enabled, Ordering::Relaxed);
    }

    pub fn contacts_presence_lookup_plan(
        &self,
    ) -> anyhow::Result<(
        std::collections::HashMap<String, bool>,
        Vec<String>,
        DirectoryClient,
    )> {
        let contacts = self.store.list_contacts()?;
        let mut known_online = std::collections::HashMap::with_capacity(contacts.len());
        let mut still_unknown = Vec::new();
        for c in contacts {
            if self.node.is_connected_to(&c.user_id) {
                known_online.insert(c.user_id, true);
            } else {
                still_unknown.push(c.user_id);
            }
        }
        Ok((known_online, still_unknown, self.directory.clone()))
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

    /// Looks a user up on the directory, caches their public identity and
    /// verified device list locally, and connects to whichever devices are
    /// currently reachable. The network connection is re-established fresh each time.
    pub async fn add_contact_by_user_id(&mut self, user_id: &str) -> anyhow::Result<()> {
        let user = self.directory.get_user(user_id).await?;
        self.store.upsert_contact(
            &user.user_id,
            &user.display_name,
            &user.ed25519_key,
            &user.curve25519_key,
            now(),
        )?;

        // Verify each cert ourselves before trusting it, not just relying
        // on the directory server's own write-time check. A cert claiming
        // a different master key or a bad signature is dropped.
        let device_certs = self.directory.get_devices(user_id).await?;
        let mut verified_devices = Vec::with_capacity(device_certs.len());
        for cert in &device_certs {
            if cert.master_ed25519_key != user.ed25519_key {
                tracing::warn!(
                    user_id,
                    device_id = %cert.device_id,
                    "dropping a device certificate whose master key doesn't match this account"
                );
                continue;
            }
            if let Err(e) = identity::Identity::verify(
                &cert.master_ed25519_key,
                &cert.signing_bytes(),
                &cert.signature,
            ) {
                tracing::warn!(
                    error = %e,
                    user_id,
                    device_id = %cert.device_id,
                    "dropping a device certificate with an invalid signature"
                );
                continue;
            }
            verified_devices.push(storage::StoredContactDevice {
                device_id: cert.device_id.clone(),
                device_ed25519_key: cert.device_ed25519_key.clone(),
                device_curve25519_key: cert.device_curve25519_key.clone(),
                cert_signature: cert.signature.clone(),
            });
        }
        self.store
            .replace_contact_devices(user_id, &verified_devices, now())?;

        // Only devices both verified above *and* currently announcing
        // presence end up reachable; an offline verified device just
        // doesn't get a `DeviceContact` entry.
        let presences = self.directory.get_presence_all(user_id).await?;
        let mut node_devices = Vec::with_capacity(verified_devices.len());
        for device in &verified_devices {
            let Some(presence) = presences.iter().find(|p| p.device_id == device.device_id) else {
                continue;
            };
            let Ok(peer_id) = PeerId::from_str(&presence.peer_id) else {
                tracing::warn!(user_id, device_id = %device.device_id, "contact's device published an invalid peer id");
                continue;
            };
            // Relay candidates alongside LAN/direct ones: costs nothing when
            // a direct address already works, and is what makes a device on
            // a different network reachable at all when it doesn't.
            let addrs: Vec<Multiaddr> = presence
                .multiaddrs
                .iter()
                .chain(presence.relay_addrs.iter())
                .filter_map(|addr| addr.parse::<Multiaddr>().ok())
                .collect();
            node_devices.push(crate::contact::DeviceContact {
                device_id: device.device_id.clone(),
                curve25519_key: device.device_curve25519_key.clone(),
                peer_id,
                addrs,
            });
        }
        // Deliberately not also calling `node.dial(...)` here: a second,
        // independent dial racing the lazy one in `send_envelope` caused a
        // hang during development (tie-breaking closed the wrong connection).
        self.node.add_contact(&user.user_id, node_devices);
        Ok(())
    }

    pub fn remove_contact(&mut self, user_id: &str) -> anyhow::Result<()> {
        self.store.remove_contact(user_id)?;
        self.node.remove_contact(user_id);
        Ok(())
    }

    /// Blocks a user's direct messages, independent of and outliving
    /// contact removal: a removed contact who messages again gets
    /// silently self-healed back in (see `next_event`'s `DirectMessage`
    /// handling), a blocked one doesn't, since that check runs first and
    /// short-circuits before the self-heal ever runs.
    pub fn block_contact(&mut self, user_id: &str) -> anyhow::Result<()> {
        self.store.block_user(user_id, now())?;
        Ok(())
    }

    pub fn unblock_contact(&mut self, user_id: &str) -> anyhow::Result<()> {
        self.store.unblock_user(user_id)?;
        Ok(())
    }

    pub fn is_contact_blocked(&self, user_id: &str) -> bool {
        self.store.is_blocked(user_id).unwrap_or(false)
    }

    /// Re-fetches this contact's presence from the directory whenever we
    /// don't have a live connection to them: a peer's address is ephemeral
    /// session data (see `add_contact_by_user_id`) that changes on every
    /// restart of their app, so trusting a once-cached address forever
    /// left sends silently failing after the other side restarted.
    ///
    /// Gating the refresh on connection state (rather than refreshing
    /// unconditionally) isn't just an optimization: an always-refresh
    /// version reliably broke `full_app_service_flow_dm_then_group` by
    /// adding enough latency before `invite_to_group`'s key share to trip
    /// libp2p's idle-connection timeout on the just-formed gossipsub mesh.
    async fn ensure_connected_contact(&mut self, user_id: &str) -> anyhow::Result<()> {
        if !self.node.has_contact(user_id) || !self.node.is_connected_to(user_id) {
            self.add_contact_by_user_id(user_id).await?;
        }
        Ok(())
    }

    /// Ensures an Olm session exists with *every* currently-known device of
    /// this contact, not just one — a send fans out to all of them, so each
    /// needs its own session established from that device's own OTK pool.
    async fn ensure_direct_session(&mut self, peer_user_id: &str) -> anyhow::Result<()> {
        for device in self.node.contact_devices(peer_user_id) {
            if self
                .node
                .has_direct_session_with_device(peer_user_id, &device.device_id)
            {
                continue;
            }
            let otk = self
                .directory
                .claim_one_time_key(peer_user_id, &device.device_id)
                .await?;
            self.node
                .ensure_outbound_session(peer_user_id, &device.device_id, &otk.public_key)?;
        }
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
        let message_id = uuid::Uuid::new_v4().to_string();
        self.node
            .send_direct_message(peer_user_id, &message_id, body, attachment.clone())?;
        self.store.insert_message(
            &message_id,
            peer_user_id,
            &self.user_id(),
            body,
            attachment.as_ref().map(to_stored_attachment).as_ref(),
            now(),
        )?;
        Ok(())
    }

    /// Refreshes the in-memory view of this account's *other* devices,
    /// keyed under our own `user_id`. Deliberately never persisted, since
    /// this isn't a real contact and shouldn't show up in `list_contacts`.
    async fn refresh_own_devices(&mut self) -> anyhow::Result<()> {
        let my_id = self.user_id();
        let my_device_id = self.node.device_id().to_string();
        let my_master_key = self.node.identity.ed25519_public_base64();
        let device_certs = self.directory.get_devices(&my_id).await?;
        let mut verified_devices = Vec::with_capacity(device_certs.len());
        for cert in &device_certs {
            if cert.device_id == my_device_id {
                continue;
            }
            if cert.master_ed25519_key != my_master_key {
                tracing::warn!(
                    device_id = %cert.device_id,
                    "dropping a device certificate whose master key doesn't match this account"
                );
                continue;
            }
            if let Err(e) = Identity::verify(
                &cert.master_ed25519_key,
                &cert.signing_bytes(),
                &cert.signature,
            ) {
                tracing::warn!(
                    error = %e,
                    device_id = %cert.device_id,
                    "dropping a device certificate with an invalid signature"
                );
                continue;
            }
            verified_devices.push(cert.clone());
        }
        let presences = self.directory.get_presence_all(&my_id).await?;
        let mut node_devices = Vec::with_capacity(verified_devices.len());
        for device in &verified_devices {
            let Some(presence) = presences.iter().find(|p| p.device_id == device.device_id) else {
                continue;
            };
            let Ok(peer_id) = PeerId::from_str(&presence.peer_id) else {
                tracing::warn!(device_id = %device.device_id, "this account's own device published an invalid peer id");
                continue;
            };
            let addrs: Vec<Multiaddr> = presence
                .multiaddrs
                .iter()
                .chain(presence.relay_addrs.iter())
                .filter_map(|a| a.parse::<Multiaddr>().ok())
                .collect();
            node_devices.push(crate::contact::DeviceContact {
                device_id: device.device_id.clone(),
                curve25519_key: device.device_curve25519_key.clone(),
                peer_id,
                addrs,
            });
        }
        self.node.add_contact(&my_id, node_devices);
        Ok(())
    }

    /// Stores messages carried by a sync exchange. `insert_message`'s
    /// `INSERT OR IGNORE` on `message_id` makes this safe to call with
    /// messages we already have, so no separate dedup check is needed.
    fn store_synced_messages(&self, messages: Vec<SyncMessage>) {
        for m in messages {
            if let Err(e) = self.store.insert_message(
                &m.message_id,
                &m.conversation_id,
                &m.sender_user_id,
                &m.body,
                m.attachment.as_ref().map(to_stored_attachment).as_ref(),
                m.sent_at,
            ) {
                tracing::warn!(error = %e, message_id = %m.message_id, "failed to store a synced message");
            }
        }
    }

    /// Manually reconciles message history with one other device of this
    /// account — the "press Sync on phone" flow. Also re-runs
    /// `discover_missing_groups` first, since pairing bootstrap never carries groups.
    pub async fn sync_with_device(&mut self, peer_device_id: &str) -> anyhow::Result<()> {
        self.discover_missing_groups().await;
        self.refresh_own_devices().await?;
        let my_id = self.user_id();
        if !self
            .node
            .has_direct_session_with_device(&my_id, peer_device_id)
        {
            let otk = self
                .directory
                .claim_one_time_key(&my_id, peer_device_id)
                .await?;
            self.node
                .ensure_outbound_session(&my_id, peer_device_id, &otk.public_key)?;
        }
        let since = self.store.load_sync_cursor(peer_device_id)?;
        let messages = self
            .store
            .list_messages_since(since)?
            .into_iter()
            .map(to_sync_message)
            .collect();
        self.node
            .send_sync_request(peer_device_id, since, messages)?;
        Ok(())
    }

    /// Answers a sync request from one of this account's own other
    /// devices: stores its delta, gathers our own since its cursor, and
    /// replies. Best-effort throughout — nothing here should disrupt the event loop.
    async fn handle_sync_requested(
        &mut self,
        from_device_id: &str,
        since: i64,
        messages: Vec<SyncMessage>,
    ) {
        self.store_synced_messages(messages);
        if let Err(e) = self.refresh_own_devices().await {
            tracing::warn!(error = %e, device_id = %from_device_id, "failed to look up the requesting device before answering a sync request");
            return;
        }
        let my_id = self.user_id();
        if !self
            .node
            .has_direct_session_with_device(&my_id, from_device_id)
        {
            let otk = match self
                .directory
                .claim_one_time_key(&my_id, from_device_id)
                .await
            {
                Ok(otk) => otk,
                Err(e) => {
                    tracing::warn!(error = %e, device_id = %from_device_id, "failed to claim a one-time key to answer a sync request");
                    return;
                }
            };
            if let Err(e) =
                self.node
                    .ensure_outbound_session(&my_id, from_device_id, &otk.public_key)
            {
                tracing::warn!(error = %e, device_id = %from_device_id, "failed to establish a session to answer a sync request");
                return;
            }
        }
        let my_delta = match self.store.list_messages_since(since) {
            Ok(msgs) => msgs.into_iter().map(to_sync_message).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to gather this device's own delta to answer a sync request");
                return;
            }
        };
        if let Err(e) = self.node.send_sync_response(from_device_id, my_delta) {
            tracing::warn!(error = %e, device_id = %from_device_id, "failed to send a sync response");
            return;
        }
        if let Err(e) = self.store.save_sync_cursor(from_device_id, now()) {
            tracing::warn!(error = %e, device_id = %from_device_id, "failed to advance the sync cursor after answering a sync request");
        }
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

    /// Owner-only (enforced server-side, same `update_roster` route as
    /// `invite_to_group`). Rotates the *caller's own* outbound Megolm
    /// session afterward and re-shares it with everyone left, so the
    /// removed member can't read anything the caller sends from here on —
    /// see `rotate_group_key`'s doc comment for what this does and doesn't
    /// guarantee (it doesn't make *other* remaining members rotate theirs).
    pub async fn remove_member_from_group(
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
                vec![],
                vec![member_user_id.to_string()],
                current.roster_version,
            )
            .await?;
        self.persist_group(&updated)?;

        // Excludes the caller: `share_group_key` looks the target up as a
        // `Contact`, and you're not your own contact — including yourself
        // here would error out of the loop before reaching anyone after it.
        let my_id = self.user_id();
        let remaining_member_ids: Vec<String> = updated
            .members
            .iter()
            .map(|m| m.user_id.clone())
            .filter(|id| *id != my_id)
            .collect();
        self.node
            .rotate_group_key(group_id, &remaining_member_ids)?;
        Ok(updated.into())
    }

    /// Removes yourself from a group's roster and drops it from local
    /// storage. Doesn't rotate anyone's Megolm key: `rotate_group_key`
    /// only ever rotates *the caller's own* outbound session, and rotating
    /// your own session on the way out protects nobody — the members who
    /// actually stay would need to rotate theirs, which nothing currently
    /// triggers automatically on a plain self-removal (same known gap
    /// noted on `remove_member_from_group`/`rotate_group_key`).
    pub async fn leave_group(&mut self, group_id: &str) -> anyhow::Result<()> {
        let current = self.directory.get_group(group_id).await?;
        self.directory
            .update_roster(
                &self.node.identity,
                group_id,
                vec![],
                vec![self.user_id()],
                current.roster_version,
            )
            .await?;
        self.store.delete_group(group_id)?;
        self.node.leave_group_topic(group_id);
        Ok(())
    }

    pub async fn send_group_message(
        &mut self,
        group_id: &str,
        channel_id: &str,
        body: &str,
        attachment: Option<AttachmentPayload>,
    ) -> anyhow::Result<()> {
        check_attachment_size(attachment.as_ref())?;
        let message_id = uuid::Uuid::new_v4().to_string();
        self.node.send_group_message(
            group_id,
            channel_id,
            &message_id,
            body,
            attachment.clone(),
        )?;
        let conversation_id = format!("{group_id}:{channel_id}");
        self.store.insert_message(
            &message_id,
            &conversation_id,
            &self.user_id(),
            body,
            attachment.as_ref().map(to_stored_attachment).as_ref(),
            now(),
        )?;
        Ok(())
    }

    /// Owner-only (enforced server-side, see `directory-server`'s
    /// `create_channel` route). The channel itself is just routable
    /// metadata on the directory server, not a new Megolm/gossipsub
    /// dimension — but other members still need to *learn* it exists, so
    /// this broadcasts a `GroupPayload::ChannelsChanged` nudge over the
    /// group's existing topic afterward (best-effort: a publish failure
    /// here doesn't undo the channel, which the server already has: anyone
    /// online receives the nudge and refetches, anyone who isn't catches up
    /// via `refresh_group` the next time they open the group instead).
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
        if let Err(e) = self.node.send_channels_changed(group_id) {
            tracing::warn!(error = %e, group_id, "failed to announce the new channel to other members");
        }
        Ok(group.into())
    }

    /// Re-fetches a group's current state from the directory server and
    /// persists it locally — the directory server is the source of truth
    /// for group/channel metadata (unlike messages, which never touch it).
    /// Called reactively when a fellow member's `GroupChannelsChanged`
    /// announcement arrives (see `next_event`), and also exposed directly
    /// (`commands::refresh_group`) so the frontend can force one, e.g. right
    /// when a group is opened, in case that announcement was missed.
    pub async fn refresh_group(&mut self, group_id: &str) -> anyhow::Result<GroupInfo> {
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

    /// Answers a validated pairing request: certifies the joining device's
    /// keys with this account's master signature, registers the
    /// certificate with the directory, and sends back the account snapshot.
    async fn handle_pairing_requested(
        &mut self,
        response_id: u64,
        device_id: String,
        device_ed25519_key: String,
        device_curve25519_key: String,
    ) {
        let Some(bootstrap) = self.pending_pairing_bootstrap.take() else {
            tracing::warn!(
                "received a pairing request with no outstanding offer to answer — ignoring"
            );
            return;
        };
        let mut cert = wire_proto::DeviceCertificate {
            device_id,
            device_ed25519_key,
            device_curve25519_key,
            master_ed25519_key: self.node.identity.ed25519_public_base64(),
            signature: String::new(),
        };
        cert.signature = self.node.identity.sign(&cert.signing_bytes());

        if let Err(e) = self
            .directory
            .register_device(&self.node.identity, cert.clone())
            .await
        {
            tracing::warn!(error = %e, "failed to register the paired device's certificate with the directory");
        }

        let master_identity_pickle_json = match self.node.identity.pickle_to_json() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failed to pickle this account's identity for pairing");
                return;
            }
        };
        let payload = net::PairingPayload {
            master_identity_pickle_json,
            display_name: self.display_name.clone(),
            cert,
            bootstrap,
        };
        if let Err(e) = self.node.respond_to_pairing(response_id, &payload) {
            tracing::warn!(error = %e, "failed to send the pairing response");
        }
    }

    /// Handles a fellow member asking us for a group's key (`ChatEvent::
    /// GroupKeyRequested`, from `DirectPayload::GroupKeyRequest`). We only
    /// have anything to give if we ourselves already have an outbound
    /// session for that group — and critically, we independently verify
    /// `from` is actually on the *current* roster before sharing anything:
    /// the request itself is just an unauthenticated claim ("I should be
    /// in this group"), Olm-authenticated as coming from `from` but not as
    /// coming from an actual member. Best-effort throughout: this runs
    /// reactively off the swarm event loop, nothing here should ever be
    /// allowed to fail loudly enough to disrupt it.
    async fn handle_group_key_requested(&mut self, group_id: &str, from: &str) {
        if !self.node.has_outbound_group_session(group_id) {
            return;
        }
        let record = match self.directory.get_group(group_id).await {
            Ok(record) => record,
            Err(e) => {
                tracing::warn!(error = %e, group_id = %group_id, "failed to verify roster membership for a group-key request");
                return;
            }
        };
        if !record.members.iter().any(|m| m.user_id == from) {
            tracing::warn!(group_id = %group_id, %from, "ignoring a group-key request from a non-member");
            return;
        }
        if let Err(e) = self.ensure_connected_contact(from).await {
            tracing::warn!(error = %e, %from, "failed to connect to a peer requesting a group key");
            return;
        }
        if let Err(e) = self.ensure_direct_session(from).await {
            tracing::warn!(error = %e, %from, "failed to establish a session with a peer requesting a group key");
            return;
        }
        if let Err(e) = self.node.share_group_key(group_id, from) {
            tracing::warn!(error = %e, group_id = %group_id, %from, "failed to answer a group-key request");
        }
    }

    /// Finds group memberships the local store doesn't know about yet and
    /// requests their key from the owner. Needed because membership
    /// otherwise only ever arrives via one fire-and-forget P2P message
    /// (the initial key-share at invite time) — if that's lost (we were
    /// offline, a dial failed, anything), there was previously no way to
    /// ever find out or recover: group_ids aren't discoverable any other
    /// way, and nothing retried it. Runs once at startup
    /// (`load_or_create_with`); best-effort throughout, since a directory-
    /// server hiccup here shouldn't block using the app for everything
    /// else.
    async fn discover_missing_groups(&mut self) {
        let my_id = self.user_id();
        let known_group_ids = match self.directory.list_my_groups(&my_id).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(error = %e, "failed to check for group memberships we might be missing");
                return;
            }
        };
        let local_group_ids: std::collections::HashSet<String> = match self.store.list_groups() {
            Ok(groups) => groups.into_iter().map(|g| g.group_id).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to read local groups");
                return;
            }
        };

        for group_id in known_group_ids {
            if local_group_ids.contains(&group_id) {
                continue;
            }
            if let Err(e) = self.request_missing_group_key(&group_id).await {
                tracing::warn!(error = %e, group_id = %group_id, "failed to request the key for a group we're apparently already in");
            }
        }
    }

    async fn request_missing_group_key(&mut self, group_id: &str) -> anyhow::Result<()> {
        let record = self.directory.get_group(group_id).await?;
        // Persisted now (name/members are non-sensitive server metadata,
        // no different from any other group lookup) even without the key
        // yet, so it shows up in the UI right away rather than staying
        // invisible until the round-trip below completes.
        self.persist_group(&record)?;
        let owner = record
            .members
            .iter()
            .find(|m| m.role == wire_proto::GroupRole::Owner)
            .ok_or_else(|| anyhow::anyhow!("group {group_id} has no owner on record"))?;
        self.ensure_connected_contact(&owner.user_id).await?;
        self.ensure_direct_session(&owner.user_id).await?;
        self.node.request_group_key(group_id, &owner.user_id)?;
        Ok(())
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
                    ref message_id,
                    ref from,
                    ref body,
                    ref attachment,
                } => {
                    if self.is_contact_blocked(from) {
                        tracing::debug!(%from, "dropping a direct message from a blocked user");
                        continue;
                    }
                    let _ = self.store.insert_message(
                        message_id,
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
                    ref message_id,
                    ref group_id,
                    ref channel_id,
                    ref from,
                    ref body,
                    ref attachment,
                } => {
                    // A working inbound Megolm session only proves the
                    // sender once shared a key for this group, not that
                    // they're still, or ever were, on its roster (a kicked
                    // member keeps whatever session they already had; an
                    // outsider can send an unsolicited key-share for a
                    // group_id they merely know). Cross-check against the
                    // locally cached roster before this ever reaches
                    // storage or the UI.
                    match self.store.is_group_member(group_id, from) {
                        Ok(true) => {
                            let conversation_id = format!("{group_id}:{channel_id}");
                            let _ = self.store.insert_message(
                                message_id,
                                &conversation_id,
                                from,
                                body,
                                attachment.as_ref().map(to_stored_attachment).as_ref(),
                                now(),
                            );
                            return event;
                        }
                        Ok(false) => {
                            tracing::warn!(group_id = %group_id, %from, "dropping a group message from a non-member");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, group_id = %group_id, "failed to check group membership, dropping the message rather than trusting it");
                        }
                    }
                }
                ChatEvent::GroupKeyReceived {
                    ref group_id,
                    ref from,
                } => {
                    // Same reasoning as `GroupMessage` above: an unsolicited
                    // key-share is only trustworthy if its sender is
                    // actually on the group's roster right now. A fresh
                    // directory fetch (not the local cache) since this can
                    // legitimately be the very first time we've heard of
                    // this group, e.g. a real new invite.
                    match self.directory.get_group(group_id).await {
                        Ok(record) if record.members.iter().any(|m| &m.user_id == from) => {
                            if let Err(e) = self.accept_group_invite(&group_id.clone()).await {
                                tracing::warn!(error = %e, group_id = %group_id, "failed to accept a group invite");
                            }
                            return event;
                        }
                        Ok(_) => {
                            tracing::warn!(group_id = %group_id, %from, "ignoring a group key share from a non-member");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, group_id = %group_id, "failed to verify roster membership for a group key share, ignoring it");
                        }
                    }
                }
                ChatEvent::GroupChannelsChanged { ref group_id } => {
                    // A fellow member created a channel: refetch so it shows
                    // up here too instead of only ever existing for them.
                    if let Err(e) = self.refresh_group(&group_id.clone()).await {
                        tracing::warn!(error = %e, group_id = %group_id, "failed to refresh a group after a channels-changed announcement");
                    }
                    return event;
                }
                ChatEvent::VoicePresence {
                    group_id,
                    channel_id,
                    from,
                    joined,
                } => {
                    if let Some(translated) = self
                        .handle_voice_presence(group_id, channel_id, from, joined)
                        .await
                    {
                        return translated;
                    }
                    // Irrelevant to any call we're currently in, not worth
                    // surfacing, loop around for the next real event.
                }
                ChatEvent::GroupKeyRequested { group_id, from } => {
                    self.handle_group_key_requested(&group_id, &from).await;
                    // Purely an internal protocol handshake — see the
                    // variant's own doc comment — never surfaced to the
                    // frontend, loop around for the next real event.
                }
                ChatEvent::CallInvited { from, call_id } => {
                    if let Some(translated) = self.handle_call_invited(from, call_id).await {
                        return translated;
                    }
                    // Auto-declined (we were already busy) — not worth
                    // surfacing, loop around for the next real event.
                }
                ChatEvent::CallAccepted {
                    from,
                    from_device_id,
                    call_id,
                } => {
                    if let Some(translated) = self
                        .handle_call_accepted(from, from_device_id, call_id)
                        .await
                    {
                        return translated;
                    }
                    // Didn't match our own outgoing call (stale/unrelated —
                    // e.g. crossed paths with our own cancellation), ignore.
                }
                ChatEvent::CallDeclined { from, call_id } => {
                    if let Some(translated) = self.handle_call_declined(from, call_id) {
                        return translated;
                    }
                }
                ChatEvent::CallEnded { from, call_id } => {
                    if let Some(translated) = self.handle_call_ended(from, call_id) {
                        return translated;
                    }
                }
                ChatEvent::CallFailed {
                    peer_user_id,
                    call_id,
                    reason,
                } => {
                    self.handle_call_failed(&call_id);
                    return ChatEvent::CallFailed {
                        peer_user_id,
                        call_id,
                        reason,
                    };
                }
                ChatEvent::PairingRequested {
                    response_id,
                    device_id,
                    device_ed25519_key,
                    device_curve25519_key,
                } => {
                    self.handle_pairing_requested(
                        response_id,
                        device_id,
                        device_ed25519_key,
                        device_curve25519_key,
                    )
                    .await;
                    // Purely an internal protocol handshake, like
                    // `GroupKeyRequested` — never surfaced to the frontend,
                    // loop around for the next real event.
                }
                ChatEvent::SyncRequested {
                    from_device_id,
                    since,
                    messages,
                } => {
                    self.handle_sync_requested(&from_device_id, since, messages)
                        .await;
                    // Purely an internal protocol handshake, like
                    // `PairingRequested` — never surfaced to the frontend,
                    // loop around for the next real event.
                }
                ChatEvent::SyncCompleted {
                    ref device_id,
                    ref messages,
                } => {
                    self.store_synced_messages(messages.clone());
                    if let Err(e) = self.store.save_sync_cursor(device_id, now()) {
                        tracing::warn!(error = %e, device_id = %device_id, "failed to advance the sync cursor after a completed sync");
                    }
                    return event;
                }
                other => return other,
            }
        }
    }

    /// Reacts to another member's voice-channel join/leave announcement.
    /// Always updates `voice_channel_presence` (the "who's in this channel
    /// right now" bookkeeping, kept for every channel we're subscribed to —
    /// see that field's doc comment), regardless of whether we're actually
    /// in a call there ourselves. If we *are* in that specific channel's
    /// call, this also updates the live call's own participant set and, on
    /// a fresh join, resolves and dials them — the mesh-connecting side
    /// stays scoped to an active call, since there's no reason to open a
    /// voice stream to someone just because we're both members of a group
    /// that happens to have a channel they joined.
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

        let bookkeeping_changed = {
            let entry = self
                .voice_channel_presence
                .entry(channel_id.clone())
                .or_default();
            if joined {
                entry.insert(from.clone())
            } else {
                entry.remove(&from)
            }
        };

        let relevant = self.voice_call.as_ref().is_some_and(|c| {
            matches!(&c.scope, voice::CallScope::Group { group_id: g, channel_id: ch } if g == &group_id && ch == &channel_id)
        });
        if !relevant {
            return bookkeeping_changed.then(|| {
                let user_ids = self
                    .voice_channel_presence
                    .get(&channel_id)
                    .map(|set| set.iter().cloned().collect())
                    .unwrap_or_default();
                ChatEvent::VoiceParticipantsChanged {
                    group_id,
                    channel_id,
                    user_ids,
                }
            });
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

    /// Who's currently in a voice channel, without needing to join it —
    /// `voice_channel_presence`'s public face. Empty (not an error) for a
    /// channel nobody's announced presence in, including one we've simply
    /// never received an announcement for yet (e.g. right after startup,
    /// before anyone currently in the channel has re-announced — presence
    /// is republished periodically, see `voice::PRESENCE_HEARTBEAT`, so this
    /// self-heals within a few seconds rather than staying stale).
    pub fn channel_voice_participants(&self, channel_id: &str) -> Vec<String> {
        self.voice_channel_presence
            .get(channel_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Resolves a voice-channel participant's current network address via
    /// the directory (the same lookup `add_contact_by_user_id` does, just
    /// without persisting them as a contact: a fellow voice participant
    /// isn't necessarily someone we've 1:1-messaged) and opens a stream to
    /// them if we're the initiating side of the pair.
    async fn connect_voice_peer(&mut self, user_id: &str) -> anyhow::Result<()> {
        // Picks whichever device's presence is returned first — a known
        // simplification, since call-signaling doesn't yet track which
        // specific device is the one actually in the call.
        let presence = self
            .directory
            .get_presence_all(user_id)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("{user_id} has no currently reachable device"))?;
        let peer_id = PeerId::from_str(&presence.peer_id)
            .map_err(|e| anyhow::anyhow!("participant published an invalid peer id: {e}"))?;
        let addrs: Vec<Multiaddr> = presence
            .multiaddrs
            .iter()
            .chain(presence.relay_addrs.iter())
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
    pub async fn join_voice_channel(
        &mut self,
        group_id: &str,
        channel_id: &str,
        preferred_input: Option<String>,
        preferred_output: Option<String>,
    ) -> anyhow::Result<()> {
        self.leave_any_current_call();
        let control = self.node.voice_control();
        let call = VoiceCallState::start(
            voice::CallScope::Group {
                group_id: group_id.to_string(),
                channel_id: channel_id.to_string(),
            },
            control,
            preferred_input,
            preferred_output,
        )
        .await?;
        // A failure here (e.g. gossipsub's mesh for this topic hasn't
        // finished forming yet) is transient and shouldn't block joining
        // the call itself; leaving `voice_call_last_heartbeat` unset makes
        // the very next `maybe_send_voice_heartbeat` retry immediately
        // rather than waiting out a full heartbeat interval.
        self.voice_call_last_heartbeat = match self
            .node
            .send_voice_presence(group_id, channel_id, true)
        {
            Ok(()) => Some(std::time::Instant::now()),
            Err(e) => {
                tracing::warn!(error = %e, group_id, channel_id, "initial voice presence announce failed, will retry");
                None
            }
        };
        self.voice_call = Some(call);
        Ok(())
    }

    /// Leaves whatever voice call we're currently in — group or 1:1 — a
    /// no-op if we're not in one. Announces our departure so the other
    /// side(s) find out immediately rather than waiting for a heartbeat to
    /// lapse (group) or never at all (1:1, which has no heartbeat — see
    /// `maybe_send_voice_heartbeat`).
    pub fn leave_voice_channel(&mut self) -> anyhow::Result<()> {
        let Some(call) = self.voice_call.take() else {
            return Ok(());
        };
        // Local cleanup (dropping `call` above tears down audio I/O and
        // every open stream) already happened regardless of whether this
        // announce makes it out; a lost departure announcement just means
        // the other side finds out we're gone late (group: next heartbeat
        // lapse) or, for a 1:1 call, potentially not until they notice the
        // connection itself dropped.
        match &call.scope {
            voice::CallScope::Group {
                group_id,
                channel_id,
            } => {
                if let Err(e) = self.node.send_voice_presence(group_id, channel_id, false) {
                    tracing::warn!(error = %e, "failed to announce leaving the voice channel");
                }
            }
            voice::CallScope::Direct {
                peer_user_id,
                call_id,
            } => {
                if let Err(e) = self.node.send_call_end(peer_user_id, call_id) {
                    tracing::warn!(error = %e, "failed to announce ending a direct call");
                }
            }
        }
        self.voice_call_last_heartbeat = None;
        Ok(())
    }

    /// Tears down whatever call is currently occupying us — a connected
    /// one (`voice_call`, group or direct, via `leave_voice_channel`) and/or
    /// a still-ringing one (`pending_call`, either direction) — so that
    /// joining a voice channel or starting a new call always begins from a
    /// clean slate instead of piling a second one on top of the first and
    /// leaving both the backend and (via the resulting events) the frontend
    /// in an inconsistent state. Best-effort: a failed departure/decline
    /// announcement here is logged, not propagated — the *new* action this
    /// is clearing the way for is what the caller actually cares about
    /// succeeding, not whether the old call's goodbye message made it out.
    fn leave_any_current_call(&mut self) {
        if let Err(e) = self.leave_voice_channel() {
            tracing::warn!(error = %e, "failed to leave the current voice call while auto-leaving for a new one");
        }
        if let Some(pending) = self.pending_call.take() {
            let result = match pending.direction {
                CallDirection::Incoming => self
                    .node
                    .send_call_decline(&pending.peer_user_id, &pending.call_id),
                CallDirection::Outgoing => self
                    .node
                    .send_call_end(&pending.peer_user_id, &pending.call_id),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, call_id = %pending.call_id, "failed to notify the peer while auto-leaving a pending call");
            }
        }
    }

    /// Calls `peer_user_id` 1:1 — sends a ring and returns the call id the
    /// frontend should track (to show "Calling…" and let the user cancel
    /// via `end_call`). The actual audio doesn't start until they accept;
    /// see `ChatEvent::CallAccepted`. Auto-leaves whatever call (group or
    /// 1:1, connected or still ringing) we were already in first — there's
    /// no call-waiting, but starting a new call is always allowed, not
    /// rejected, matching `join_voice_channel`'s own auto-leave.
    pub async fn call_contact(
        &mut self,
        peer_user_id: &str,
        preferred_input: Option<String>,
        preferred_output: Option<String>,
    ) -> anyhow::Result<String> {
        self.leave_any_current_call();
        self.ensure_connected_contact(peer_user_id).await?;
        self.ensure_direct_session(peer_user_id).await?;
        let call_id = uuid::Uuid::new_v4().to_string();
        self.node.send_call_invite(peer_user_id, &call_id)?;
        self.pending_call = Some(PendingCall {
            call_id: call_id.clone(),
            peer_user_id: peer_user_id.to_string(),
            direction: CallDirection::Outgoing,
            preferred_input,
            preferred_output,
        });
        Ok(call_id)
    }

    /// Accepts a currently-ringing incoming call (`call_id` from the
    /// `ChatEvent::CallInvited` the frontend is showing) and starts the
    /// actual voice stream.
    pub async fn accept_call(
        &mut self,
        call_id: &str,
        preferred_input: Option<String>,
        preferred_output: Option<String>,
    ) -> anyhow::Result<()> {
        let matches = self
            .pending_call
            .as_ref()
            .is_some_and(|p| p.call_id == call_id && p.direction == CallDirection::Incoming);
        if !matches {
            anyhow::bail!("no incoming call with id {call_id} to accept");
        }
        let pending = self
            .pending_call
            .take()
            .expect("just checked is_some_and above");
        if let Err(e) = self.node.send_call_accept(&pending.peer_user_id, call_id) {
            self.pending_call = Some(pending); // put it back so the caller can retry
            return Err(e);
        }
        self.start_direct_call(
            &pending.peer_user_id,
            call_id,
            preferred_input,
            preferred_output,
        )
        .await
    }

    /// Rejects a currently-ringing incoming call.
    pub fn decline_call(&mut self, call_id: &str) -> anyhow::Result<()> {
        let matches = self
            .pending_call
            .as_ref()
            .is_some_and(|p| p.call_id == call_id && p.direction == CallDirection::Incoming);
        if !matches {
            anyhow::bail!("no incoming call with id {call_id} to decline");
        }
        let pending = self
            .pending_call
            .take()
            .expect("just checked is_some_and above");
        self.node.send_call_decline(&pending.peer_user_id, call_id)
    }

    /// Ends a 1:1 call in any state: cancels one still ringing (either
    /// direction we initiated ourselves — declining an incoming call the
    /// user hasn't decided on yet is `decline_call`'s job, not this) or
    /// hangs up one already connected (delegating to `leave_voice_channel`,
    /// which already knows how to sign off a `Direct` call). A no-op if
    /// `call_id` doesn't match anything current — e.g. it already ended on
    /// the other side and the frontend hasn't caught up yet.
    pub fn end_call(&mut self, call_id: &str) -> anyhow::Result<()> {
        if let Some(pending) = &self.pending_call
            && pending.call_id == call_id
        {
            let peer_user_id = pending.peer_user_id.clone();
            self.pending_call = None;
            return self.node.send_call_end(&peer_user_id, call_id);
        }
        if self.voice_call.as_ref().is_some_and(
            |c| matches!(&c.scope, voice::CallScope::Direct { call_id: id, .. } if id == call_id),
        ) {
            return self.leave_voice_channel();
        }
        Ok(())
    }

    /// Shared by `accept_call` (the callee's side) and `handle_call_accepted`
    /// (the caller's side, once the callee's acceptance arrives) — starts
    /// local audio I/O and dials the peer's voice stream directly, no
    /// group/gossipsub presence involved at all.
    async fn start_direct_call(
        &mut self,
        peer_user_id: &str,
        call_id: &str,
        preferred_input: Option<String>,
        preferred_output: Option<String>,
    ) -> anyhow::Result<()> {
        if self.voice_call.is_some() {
            self.leave_voice_channel()?;
        }
        let control = self.node.voice_control();
        let call = VoiceCallState::start(
            voice::CallScope::Direct {
                peer_user_id: peer_user_id.to_string(),
                call_id: call_id.to_string(),
            },
            control,
            preferred_input,
            preferred_output,
        )
        .await?;
        // Unlike a group call — where `participants` is populated purely by
        // `VoicePresence` gossip arriving over time — a direct call has
        // exactly one possible participant and we already know who: no
        // gossip round-trip needed, and none is coming (`CallScope::Direct`
        // never publishes `VoicePresence`). Mark them present immediately
        // rather than leaving `participants` empty until some other signal
        // populates it, which for a direct call would be never.
        call.note_presence(peer_user_id, true);
        self.voice_call = Some(call);
        self.connect_voice_peer(peer_user_id).await?;
        Ok(())
    }

    /// Reacts to an incoming `CallInvite`: auto-declines if we're already
    /// busy (ringing or in a call — no call-waiting), otherwise self-heals
    /// the caller into a known contact (same reasoning as `DirectMessage`'s
    /// self-heal in `next_event` — a call, like a message, can arrive from
    /// someone we've never looked up before) and starts ringing.
    async fn handle_call_invited(&mut self, from: String, call_id: String) -> Option<ChatEvent> {
        if self.voice_call.is_some() || self.pending_call.is_some() {
            if let Err(e) = self.node.send_call_decline(&from, &call_id) {
                tracing::warn!(error = %e, %from, "failed to auto-decline an incoming call while busy");
            }
            return None;
        }
        if !self.node.has_contact(&from)
            && let Err(e) = self.add_contact_by_user_id(&from.clone()).await
        {
            tracing::warn!(error = %e, %from, "failed to look up an unknown caller");
        }
        self.pending_call = Some(PendingCall {
            call_id: call_id.clone(),
            peer_user_id: from.clone(),
            direction: CallDirection::Incoming,
            preferred_input: None,
            preferred_output: None,
        });
        Some(ChatEvent::CallInvited { from, call_id })
    }

    /// Reacts to the callee accepting our outgoing call: starts the actual
    /// voice stream using the device preferences captured back when we
    /// first called them (`call_contact`). Ignored if it doesn't match a
    /// call we're actually currently ringing (a race with our own
    /// cancellation, or a stale/replayed message).
    ///
    /// `CallInvite` fanned out to every device, so best-effort notifies the
    /// ones that didn't answer to stop ringing; a failure there doesn't
    /// affect the call we're actually starting here.
    async fn handle_call_accepted(
        &mut self,
        from: String,
        from_device_id: String,
        call_id: String,
    ) -> Option<ChatEvent> {
        let matches = self.pending_call.as_ref().is_some_and(|p| {
            p.call_id == call_id && p.peer_user_id == from && p.direction == CallDirection::Outgoing
        });
        if !matches {
            return None;
        }
        let pending = self
            .pending_call
            .take()
            .expect("just checked is_some_and above");
        if let Err(e) = self
            .node
            .send_call_end_to_other_devices(&from, &call_id, &from_device_id)
        {
            tracing::warn!(error = %e, %from, "failed to notify sibling devices that the call was answered elsewhere");
        }
        if let Err(e) = self
            .start_direct_call(
                &from,
                &call_id,
                pending.preferred_input,
                pending.preferred_output,
            )
            .await
        {
            tracing::warn!(error = %e, %from, "failed to start the voice stream after the callee accepted");
            let _ = self.node.send_call_end(&from, &call_id);
            return Some(ChatEvent::CallEnded { from, call_id });
        }
        Some(ChatEvent::CallAccepted {
            from,
            from_device_id,
            call_id,
        })
    }

    /// Reacts to the callee declining our outgoing call. Ignored (`None`)
    /// if it doesn't match what we're actually ringing.
    fn handle_call_declined(&mut self, from: String, call_id: String) -> Option<ChatEvent> {
        let matches = self
            .pending_call
            .as_ref()
            .is_some_and(|p| p.call_id == call_id && p.peer_user_id == from);
        if !matches {
            return None;
        }
        self.pending_call = None;
        Some(ChatEvent::CallDeclined { from, call_id })
    }

    /// Reacts to the other side ending a call — cancelling one still
    /// ringing or hanging up one already connected. Tears down whichever
    /// local state (`pending_call` or `voice_call`) actually matches;
    /// ignored (`None`) if neither does (e.g. we already hung up ourselves
    /// and this crossed paths with our own `CallEnd`).
    fn handle_call_ended(&mut self, from: String, call_id: String) -> Option<ChatEvent> {
        let was_pending = self
            .pending_call
            .as_ref()
            .is_some_and(|p| p.call_id == call_id && p.peer_user_id == from);
        if was_pending {
            self.pending_call = None;
        }
        let was_active = self.voice_call.as_ref().is_some_and(|c| {
            matches!(&c.scope, voice::CallScope::Direct { peer_user_id, call_id: id } if peer_user_id == &from && id == &call_id)
        });
        if was_active {
            self.voice_call = None;
            self.voice_call_last_heartbeat = None;
        }
        (was_pending || was_active).then_some(ChatEvent::CallEnded { from, call_id })
    }

    /// Reacts to our own `CallInvite`/`CallAccept` failing to reach the
    /// peer (see `ChatNode::pending_call_sends`) — most commonly because
    /// their last-known address is stale and they're not actually online
    /// right now. Tears down whatever local state was already started for
    /// `call_id`: a still-ringing `pending_call` (the common case — our
    /// `CallInvite` never got out), or — if this was a failed
    /// `CallAccept` — the `voice_call` `start_direct_call` had already
    /// spun up locally before the failure could be known (`accept_call`
    /// starts audio right after queuing the send, not after it's confirmed
    /// delivered). A no-op if neither matches (we already ended this call
    /// ourselves and this crossed paths with the failure).
    fn handle_call_failed(&mut self, call_id: &str) {
        if self
            .pending_call
            .as_ref()
            .is_some_and(|p| p.call_id == call_id)
        {
            self.pending_call = None;
        }
        if self.voice_call.as_ref().is_some_and(
            |c| matches!(&c.scope, voice::CallScope::Direct { call_id: id, .. } if id == call_id),
        ) {
            self.voice_call = None;
            self.voice_call_last_heartbeat = None;
        }
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

    /// `false` outside an active call, same as `set_mic_muted`. Exists so
    /// the frontend has a way to ask "am I actually muted right now?"
    /// instead of only ever tracking its own optimistic local guess — no
    /// such query existed before, which is part of what let the UI and the
    /// real backend state drift out of sync.
    pub fn is_mic_muted(&self) -> bool {
        self.voice_call.as_ref().is_some_and(|call| call.is_muted())
    }

    /// Flips the current mute state and returns the new one — a no-op
    /// (returns `false`) outside an active call, same as `set_mic_muted`.
    /// Used by the tray menu's mic-mute item, which has no other way to
    /// know the current state before deciding which way to flip it.
    pub fn toggle_mic_muted(&self) -> bool {
        let Some(call) = &self.voice_call else {
            return false;
        };
        let new_muted = !call.is_muted();
        call.set_muted(new_muted);
        new_muted
    }

    pub fn voice_participants(&self) -> Vec<String> {
        self.voice_call
            .as_ref()
            .map(|c| c.participants())
            .unwrap_or_default()
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
        // Only a group call needs this: it's how a group member who joins
        // the channel after we did discovers we're already there (no
        // replay on gossipsub). A 1:1 call has no presence topic at all —
        // we're either directly connected to the one other participant or
        // the call is over, nothing to periodically re-announce.
        let voice::CallScope::Group {
            group_id,
            channel_id,
        } = &call.scope
        else {
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
        match self.node.send_voice_presence(group_id, channel_id, true) {
            Ok(()) => self.voice_call_last_heartbeat = Some(std::time::Instant::now()),
            Err(e) => {
                tracing::warn!(error = %e, "failed to send voice presence heartbeat, will retry")
            }
        }
    }
}

/// Resolves the contacts a [`AppService::contacts_presence_lookup_plan`]
/// couldn't already answer from a live connection.
pub async fn resolve_contacts_online_status(
    mut known_online: std::collections::HashMap<String, bool>,
    still_unknown: Vec<String>,
    directory: DirectoryClient,
) -> std::collections::HashMap<String, bool> {
    let lookups = still_unknown.into_iter().map(|user_id| {
        let directory = directory.clone();
        async move {
            let is_online = directory
                .get_presence_all(&user_id)
                .await
                .map(|records| records.iter().any(|r| r.share_online_status))
                .unwrap_or(false);
            (user_id, is_online)
        }
    });
    known_online.extend(futures::future::join_all(lookups).await);
    known_online
}
