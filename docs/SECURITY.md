# Security notes

For what's protected and what isn't, see [`THREAT_MODEL.md`](./THREAT_MODEL.md). This doc is
about the mechanics and how to keep them honest.

## Cryptography

Nothing here is homemade. End-to-end encryption is [vodozemac](https://github.com/matrix-org/vodozemac)
(the audited Rust library behind Matrix/Element): Olm (Double Ratchet) for 1:1 sessions,
Megolm (sender-key ratchet) for groups. Transport encryption is libp2p's Noise implementation.
Local-storage encryption is XChaCha20-Poly1305 (`storage::crypto`, via RustCrypto's
`chacha20poly1305`) keyed by a random 256-bit key held in the OS keychain, not a
password-derived key: there's no KDF to attack because there's no password.

## Multi-device / pairing

Each device has its own Olm identity (`ChatNode::device_identity`) and its own
`DeviceCertificate`, signed by the account's master key and verified both server-side
(`directory-server::routes::devices::register_device`, the one narrow exception to the
directory's crypto-blindness — see `THREAT_MODEL.md`) and client-side
(`identity::Identity::verify`, in `AppService::add_contact_by_user_id`) before either side
trusts it. QR pairing (`crates/net/src/pairing_protocol.rs`) additionally transmits the
account's master private key itself to the joining device — not just a device cert — wrapped
in an ephemeral-X25519-ECDH-derived XChaCha20-Poly1305 layer on top of the Noise-encrypted
transport, gated by a single-use token expiring after 120s
(`net::PAIRING_TOKEN_TTL_SECS`). **This is a real, deliberate deviation** from the safer
device-cert-only design, with real consequences (no device revocation, any paired device can
mint further devices) — see `THREAT_MODEL.md`'s "known limitations" for the full reasoning
and blast-radius discussion before touching this code path.

## Logging

Every `tracing::*!` call site in the workspace has been reviewed by hand (there are ~16 of
them); none log message plaintext, private key material, session pickles, or the directory
server's admin token. `directory-server`'s `TraceLayer` (tower-http) logs method/path/status/
latency only, never bodies. If you add a new log line, check: does this field come from
user-authored content, key material, or session state? If yes, don't log it; log the
*type* of failure instead (already the pattern throughout `crypto-session` and `net`'s error
types).

## Purge

Two independent purge paths, at two different layers, on purpose:

- **Local (per-device):** `storage::panic_purge` deletes the OS-keychain KEK first
  (crypto-shred), then the SQLite file + WAL/SHM as a courtesy. Exposed in the app as
  Settings → Data & Privacy, gated behind typing a confirmation phrase
  (`SettingsPanel.tsx`).
- **Directory (operator-only):** `directory-admin purge --yes` against the loopback-bound,
  bearer-token-protected admin API (`crates/directory-server/src/admin.rs`). Not reachable
  from the public internet by default, and not exposed anywhere in the end-user app;
  wiping the shared directory is an operator action, not a user one, since it affects
  everyone who's registered against that instance.

Neither purge can affect the other: your local purge doesn't touch the directory, and a
directory purge doesn't touch anyone's local data (every directory record is a cache of
data the client already has; see `THREAT_MODEL.md` and `directory-server`'s route docs).

## Dependency auditing

```
cargo install cargo-audit --locked   # once
cargo audit                          # from the repo root
```

Wired into CI (`.github/workflows/ci.yml`). Ignored advisories live in `.cargo/audit.toml`
with a specific justification each, never a blanket suppression. Re-check that list
periodically; an ignored advisory today may have a fix available next month.

## Reporting a vulnerability

This is a personal/early-stage project without a dedicated security contact yet. If you find
something, open an issue describing the concern without exploit details in the public
tracker, or reach the maintainer directly if you have a way to.

This file was augmented/rephrased by Claude Codea