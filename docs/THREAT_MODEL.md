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

One narrow, deliberate exception to "structurally opaque": `POST /v1/users/{user_id}/devices`
(`directory-server::routes::devices::register_device`) verifies not just the outer request
signature (every route does that) but also the *nested* `DeviceCertificate` signature inside
the body, using the same `ed25519_dalek::verify_strict` primitive `auth.rs` already calls
everywhere else — this stops anyone from `POST`ing a garbage device certificate onto someone
else's `user_id` (device-list poisoning). It's still write-authorization, the same category of
check every other endpoint performs, one level deeper; it doesn't touch message content and
doesn't require `crypto-session` as a dependency.

**Multi-device identity.** An account's trust anchor is its master Ed25519/Curve25519 keypair
(`crates/identity`) — `user_id` is its fingerprint, contacts verify each other against it,
safety numbers are about it. Every device (including the very first one) additionally
generates its own local Olm identity (`ChatNode::device_identity`, a distinct vodozemac
`Account`, never the same Curve25519 key as the master identity or any other device) and
registers a `DeviceCertificate{device_id, device_ed25519_key, device_curve25519_key,
master_ed25519_key, signature}` signed by the master key. Sending a message fans out one Olm
encryption per verified device a contact has (`ChatNode::encrypt_and_send_direct`); a contact
adder verifies every cert against the claimed master key client-side too
(`Identity::verify`, in `AppService::add_contact_by_user_id`), not just trusting the
directory's own check above. Two devices are cryptographically independent from the moment
each has its own device cert: neither needs the other online to send or receive.

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
- **QR pairing transmits the account's master private key, not just a device certificate.**
  This is a deliberate deviation from the safer design (device certs only, private keys never
  leaving the device that generated them) that a from-scratch multi-device scheme would use —
  forced by `directory-server`'s write-auth model requiring the *master* key's signature on
  every account-level write (`register_user`, `put_presence`, `upload_otks`,
  `register_device`), which a linked device would otherwise be unable to produce on its own if
  it ever needed to re-register the account (e.g. the directory was purged while the original
  device was offline) — and the user explicitly requires every device to work fully
  independently, not as a read-only mirror of one "real" device. Concretely
  (`net::PairingPayload`, `AppService::join_via_pairing`): the joining device receives
  `master_identity_pickle_json` — the account's actual signing key — over the pairing channel.
  That channel is defense in depth, not a plaintext transfer: an ephemeral X25519 ECDH key
  (fresh per pairing attempt, `net::pairing_protocol::generate_ephemeral_keypair`) wraps the
  payload in XChaCha20-Poly1305, itself riding inside the already Noise-encrypted libp2p
  transport, gated by a single-use token that expires after `PAIRING_TOKEN_TTL_SECS` (120s).
  But the blast radius of a compromised pairing exchange is categorically larger than a
  device-cert-only scheme's: intercepting one doesn't just add a rogue *device*, it hands over
  the whole account, permanently, with no revocation path (see the next point). Treat the QR
  code and the ~2-minute pairing window with the same care as the private key itself — don't
  display or scan it somewhere a screen-recorder, shoulder-surfer, or network position you
  don't trust could capture it.
- **No device revocation.** There's no way to invalidate a device's certificate or its copy of
  the master private key once pairing has completed — removing a "device" isn't implemented,
  even at the storage layer. A lost or compromised paired device (phone theft, in particular)
  currently has no remedy short of abandoning the account entirely (a fresh identity, contacts
  re-added one by one). This is the sharpest edge of the current multi-device design and the
  most important gap to close next.
- **Any paired device can mint further device certificates**, not just the one that showed the
  original QR code — a consequence of every device receiving the master private key (previous
  point), not a separate design choice. There's no "primary device" concept once pairing has
  happened once.
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