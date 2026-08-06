# Threat model

## What this protects against

**A compromised or malicious directory server.** The directory (`crates/directory-server`)
never receives message content, structurally, not by policy: its dependency graph doesn't
include `crypto-session` or the P2P message-handling code in `net`, so there's no code path
by which it could touch plaintext even if compromised. Every write is signed by the caller's
Ed25519 identity key (`crates/wire-proto/src/signing.rs`), and `user_id` is the fingerprint of
that key (`wire-proto::user_id_from_ed25519`), so the directory can't forge a registration,
presence record, or group-roster change on anyone's behalf. Full compromise or a subpoena
against the operator yields: pubkeys, display names, and *stale* network addresses. Nothing
about who talked to whom about what.

**Network eavesdropping between peers.** Transport is Noise-encrypted (libp2p) end to end,
and message content is *additionally* encrypted above that with Olm (1:1,
`crates/crypto-session/src/olm.rs`) or Megolm (groups, `.../megolm.rs`); an attacker who breaks
the transport layer alone still gets ciphertext.

**Device theft/seizure after a purge.** `storage::panic_purge` deletes the OS-keychain KEK
*before* deleting the local database file (`crates/storage/src/purge.rs`). Every sensitive
column (identity pickle, session state, message bodies) is encrypted with that KEK
(`crates/storage/src/crypto.rs`); once it's gone, the remaining ciphertext is permanently
undecipherable regardless of whether the file itself is later recovered from disk.

**A removed group member reading future messages.** Megolm sessions are rotated and
re-shared to remaining members on removal (`ChatNode::rotate_group_key`), not just on a
roster flag, verified by
`crypto-session/tests/megolm.rs::removed_member_cannot_decrypt_messages_sent_after_rotation`.

**Network eavesdropping on voice audio.** Same guarantee as everything else: transport is
Noise-encrypted end to end between the two call participants (`crates/net/src/voice_protocol.rs`,
`crates/core/src/voice.rs`), even across a relay circuit: the relay only forwards opaque
bytes, the Noise handshake is between the real endpoints, not the relay.

## What this does *not* protect against (known limitations)

- **A compromised, unlocked device.** Once the OS-keychain KEK is available to the running
  process, the local encrypted store is an open book to anything with equivalent access
  (malware, another process running as the same user, physical access to an unlocked
  machine). This is standard for local-encryption-at-rest and not specific to this app.
- **Traffic analysis / connection metadata.** Direct libp2p connections and gossipsub
  publishes are visible to network observers as *connections*, even though content is
  opaque. There's no mixnet or cover traffic here: an observer positioned to watch both
  peers can infer that they're talking, and roughly when and how much, just not what.
- **The `sender_user_id`/`sender_curve25519_key` routing hint on a `DirectEnvelope`
  (`crates/crypto-session/src/envelope.rs`) is an unauthenticated claim**, used only to pick
  which local session to try. This is safe by construction, not by policy: a forged claim
  makes decryption fail (wrong/missing session), it can never make forged plaintext look
  like it came from someone else; the actual authentication is the Olm session itself,
  bound to the peer's real key via the 3DH handshake.
- **A single compromised group member can leak the current Megolm key** to whoever they
  want, going forward, until the next rotation. This is inherent to any sender-keys group
  scheme (Megolm, Signal's sender keys, etc.), not a bug here.
- **No multi-device support.** One identity == one device's keychain entry
  (`identity::Keychain::for_app_data_dir`). Restoring an identity onto a second device isn't
  implemented.
- **No forward secrecy across an app restart yet.** Olm/Megolm session *state* lives in
  memory only (`crypto-session`'s managers), not yet persisted to `storage`'s
  `sessions_olm`/`sessions_megolm_*` tables (schema exists, CRUD doesn't yet). A restart
  means re-establishing sessions on next contact: a UX rough edge, not a confidentiality
  issue, since a fresh session is still fully secure.
- **Presence is a single-shot announcement, not a heartbeat** (`AppService::load_or_create`).
  It expires after the directory's TTL cap (300s); a long-running session doesn't keep
  re-announcing yet, so contacts may need a restart to find a fresh address.
- **Voice audio is not additionally Olm/Megolm-wrapped, unlike text and files.** It rides the
  same Noise-encrypted transport as everything else in this app, but a real-time,
  per-frame application-layer ratchet on top of that would be substantial extra
  engineering, deferred as explicit future work, not silently skipped. Practically: whoever
  can already read your Noise-encrypted traffic (i.e. is genuinely one of the two connection
  endpoints) can hear the call, same as the transport-level guarantee everywhere else in
  this app.
- **Voice topology is full mesh with no size limit enforced.** Every participant opens a
  direct stream to every other participant (`crates/core/src/voice.rs`), deliberately, so no
  server ever touches decrypted or mixed audio (an SFU/relay-mixer would mean exactly that,
  contradicting this project's "no traces" premise). This doesn't scale to large calls; it's
  an explicit ceiling appropriate to this project's small-group scope, not a bug.
- **Voice-channel presence is visible to the whole group, not just call participants.** A
  join/leave announcement (`GroupPayload::VoicePresence`) travels over the group's existing
  Megolm session and gossipsub topic, the same audience as a text message in that group, so
  any member who can decrypt group messages can see who's in a voice call even without
  joining it themselves.
- **The one-click voice changer is a best-effort disguise, not an anonymity guarantee.** It's
  a real, audible pitch shift (a phase vocoder, `crates/audio/src/pitch_shift.rs`) meant to
  defeat casual or incidental voice recognition. It is explicitly not claimed to resist
  serious voice-biometric analysis: forensic speaker identification can potentially still
  work through a simple pitch shift, and the UI (`VoiceCallPanel.tsx`) labels it that way
  rather than overselling it as real anonymity.


This file was augmented/rephrased by Claude Codea