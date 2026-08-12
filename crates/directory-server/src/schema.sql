CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    ed25519_key TEXT NOT NULL,
    curve25519_key TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- One row per account device: a device's OTK pool is independent of its
-- sibling devices' pools, so two devices sharing an account never contend
-- for or invalidate each other's keys.
CREATE TABLE IF NOT EXISTS one_time_keys (
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    key_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    is_fallback INTEGER NOT NULL DEFAULT 0,
    claimed INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, device_id, key_id)
);


-- One row per account device, not per account: each device heartbeats its
-- own row without clobbering its siblings', so a sender can fan out to
-- every device that's currently reachable.
CREATE TABLE IF NOT EXISTS presence (
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    peer_id TEXT NOT NULL,
    multiaddrs TEXT NOT NULL,
    relay_addrs TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    share_online_status INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (user_id, device_id)
);

-- A device's own Ed25519/Curve25519 identity, distinct from the account's
-- master key. `cert_signature` proves it was actually linked, not
-- self-declared; checked both by contacts and by the server on write.
CREATE TABLE IF NOT EXISTS devices (
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    device_ed25519_key TEXT NOT NULL,
    device_curve25519_key TEXT NOT NULL,
    master_ed25519_key TEXT NOT NULL,
    cert_signature TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, device_id)
);

CREATE TABLE IF NOT EXISTS groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    roster_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS group_members (
    group_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE IF NOT EXISTS channels (
    channel_id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    position INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channels_group ON channels (group_id, position);

CREATE TABLE IF NOT EXISTS nonces (
    user_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, nonce)
);
