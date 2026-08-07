-- This database file is plain SQLite. Columns suffixed `_blob` hold data
-- individually encrypted at the application layer (see crypto.rs) with the
-- OS-keychain-backed KEK — not whole-file encryption. Non-sensitive metadata
-- (ids, timestamps, contact/group names, roles) is left in the clear since
-- it isn't secret and needs to stay queryable/indexable.

-- Single-row table: the local user's own identity (vodozemac Account pickle).
CREATE TABLE IF NOT EXISTS identity (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    pickle_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

-- Single-row table: this account's libp2p transport keypair (separate from
-- the vodozemac chat identity above — see net::swarm's doc comment on why
-- they're deliberately different keys). Without this, a fresh keypair would
-- get minted on every launch, which mints a fresh PeerId every launch too —
-- any contact who cached the old one (i.e. everyone who added this account
-- before its most recent restart) silently can't reach it anymore, with no
-- automatic recovery, until they remove and re-add the contact.
CREATE TABLE IF NOT EXISTS p2p_identity (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    keypair_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

-- One row per active Olm ratchet session with a peer. Rewritten after every
-- encrypt/decrypt since the ratchet state must survive process restarts.
CREATE TABLE IF NOT EXISTS sessions_olm (
    peer_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    pickle_blob BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (peer_user_id, session_id)
);

CREATE TABLE IF NOT EXISTS sessions_megolm_out (
    group_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    pickle_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    rotated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions_megolm_in (
    group_id TEXT NOT NULL,
    sender_user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    pickle_blob BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, sender_user_id, session_id)
);

CREATE TABLE IF NOT EXISTS contacts (
    user_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    ed25519_key TEXT NOT NULL,
    curve25519_key TEXT NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    last_seen_at INTEGER
);

-- Users whose direct messages we no longer want to receive. Kept separate
-- from `contacts` on purpose: it needs to outlive contact removal (and any
-- self-heal that would otherwise silently re-add the sender as a contact
-- on their next message) rather than share that row's lifecycle.
CREATE TABLE IF NOT EXISTS blocked_contacts (
    user_id TEXT PRIMARY KEY,
    blocked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    roster_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS group_members (
    group_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE IF NOT EXISTS channels (
    channel_id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    position INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    sender_user_id TEXT NOT NULL,
    body_blob BLOB NOT NULL,
    sent_at INTEGER NOT NULL,
    delivered INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages (conversation_id, sent_at);

-- Envelopes queued for a peer/group that wasn't reachable yet; retried when
-- presence indicates they're back online. There is deliberately no
-- server-side equivalent of this table. Already Olm/Megolm ciphertext bound
-- for the network, so no additional application-layer encryption here.
CREATE TABLE IF NOT EXISTS outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    envelope_bytes BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0
);
