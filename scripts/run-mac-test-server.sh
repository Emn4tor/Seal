#!/usr/bin/env bash
#
# Runs a local Seal directory server for testing on this Mac — bound to all
# interfaces on plain HTTP, reachable at this machine's LAN IP. No domain,
# no reverse proxy, no TLS, nothing else set up. Good for testing the app
# across two devices on the same network (or two profiles on this one
# machine); not for real hosting — see scripts/setup-backend.sh (Linux) for
# that.
#
# Runs in the foreground; Ctrl-C stops it. Re-running reuses the same data
# and admin token instead of starting fresh each time.
#
# Usage: ./scripts/run-mac-test-server.sh

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script is for macOS. On Linux, see scripts/setup-backend.sh." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust first: https://rustup.rs" >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="$HOME/.seal-test-server"
DB_PATH="$DATA_DIR/directory.sqlite3"
TOKEN_FILE="$DATA_DIR/admin-token"
PUBLIC_PORT=8080
ADMIN_PORT=8090

mkdir -p "$DATA_DIR"

# ---------------------------------------------------------------------------
# Find this Mac's local network IP, just to print it — the server itself
# binds 0.0.0.0 so it's reachable both this way and via 127.0.0.1 locally.
# ---------------------------------------------------------------------------

LOCAL_IP=""
for iface in en0 en1 en2; do
  ip="$(ipconfig getifaddr "$iface" 2>/dev/null || true)"
  if [[ -n "$ip" ]]; then
    LOCAL_IP="$ip"
    break
  fi
done
if [[ -z "$LOCAL_IP" ]]; then
  echo "Couldn't find a LAN IP (tried en0/en1/en2 — are you on Wi-Fi/Ethernet?)." >&2
  echo "Other devices won't be able to reach this; it'll still work for" >&2
  echo "testing against 127.0.0.1 on this same machine." >&2
  LOCAL_IP="127.0.0.1"
fi

# ---------------------------------------------------------------------------
# Admin token — generated once, reused on every later run of this script
# ---------------------------------------------------------------------------

if [[ -f "$TOKEN_FILE" ]]; then
  TOKEN="$(cat "$TOKEN_FILE")"
else
  if command -v openssl >/dev/null 2>&1; then
    TOKEN="$(openssl rand -hex 32)"
  else
    TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  fi
  printf '%s' "$TOKEN" > "$TOKEN_FILE"
  chmod 600 "$TOKEN_FILE"
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

echo "== Seal local test server (macOS) =="
echo "-- Building (release) --"
(cd "$REPO_ROOT" && cargo build --release -p directory-server --bin directory-server)
echo

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

SERVER_URL="http://$LOCAL_IP:$PUBLIC_PORT"

echo "Reachable at:   $SERVER_URL  (and http://127.0.0.1:$PUBLIC_PORT on this Mac)"
echo "Data:           $DB_PATH"
echo "Admin token:    $TOKEN_FILE"
echo "Purge:          cargo run --release -p directory-server --bin directory-admin -- \\"
echo "                  --admin-url http://127.0.0.1:$ADMIN_PORT --token \"\$(cat $TOKEN_FILE)\" purge"
echo
echo "Point the app at it: enter '$SERVER_URL' as a custom server on its"
echo "first-run screen, or launch it with:"
echo "  P2P_CHAT_DIRECTORY_URL=$SERVER_URL npm run tauri dev"
echo
echo "Ctrl-C to stop."
echo

exec env \
  DIRECTORY_DB_PATH="$DB_PATH" \
  DIRECTORY_PUBLIC_ADDR="0.0.0.0:$PUBLIC_PORT" \
  DIRECTORY_ADMIN_ADDR="127.0.0.1:$ADMIN_PORT" \
  DIRECTORY_ADMIN_TOKEN="$TOKEN" \
  "$REPO_ROOT/target/release/directory-server"
