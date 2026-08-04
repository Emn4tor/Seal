#!/usr/bin/env bash
#
# Launches two instances of the built Seal app side by side on this Mac,
# each with its own identity, so you can add each other as contacts and
# test DMs/groups without a second machine. Mirrors the `P2P_CHAT_PROFILE`
# dev-mode pattern from README.md §3, but against the real compiled app
# instead of `npm run tauri dev`.
#
# By default this now actually exercises the relay/dcutr path instead of
# trivially succeeding over LAN: two processes on the same Mac share one
# network stack, so a direct dial between them (even over loopback) would
# always work regardless of what "network" they're meant to represent --
# that's not what two real people on two different networks/behind two
# different NATs get. So this script:
#   1. Spins up its own throwaway `directory-server` instance (temp DB,
#      loopback-only, admin token generated on the fly) with its relay
#      advertised on loopback -- self-contained, doesn't touch or add test
#      noise to a real deployed server.
#   2. Launches both app instances with P2P_CHAT_SIMULATE_WAN=1, which
#      makes each one withhold its LAN address from presence entirely (see
#      AppService::load_or_create) -- not just prefer the relay, genuinely
#      unreachable by direct dial -- so the only way for them to reach each
#      other is through that relay, with dcutr then attempting its usual
#      upgrade-to-direct on top (which, since both really are on localhost,
#      should actually succeed -- a nice bonus check that the full
#      relay->upgrade pipeline works end to end, not just the fallback).
#
# Usage:
#   ./scripts/run-two-mac-instances.sh                     # profiles: alice, bob
#   ./scripts/run-two-mac-instances.sh carol dave          # custom profile names
#   ./scripts/run-two-mac-instances.sh --same-lan          # old behavior: real
#                                                           # LAN addresses allowed,
#                                                           # relay/dcutr not exercised
#   ./scripts/run-two-mac-instances.sh --server URL        # skip the local test
#                                                           # server, point both
#                                                           # instances at URL instead
#                                                           # (e.g. the real deployed
#                                                           # server) -- still sets
#                                                           # P2P_CHAT_SIMULATE_WAN
#
# Both instances keep fully separate local identities, keys, contacts, and
# messages (see accounts::account_dir) even though they share one Tauri
# app-data directory.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script launches the macOS app and must run on macOS." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

SIMULATE_WAN=1
SERVER_URL=""
PROFILES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --same-lan)
      SIMULATE_WAN=0
      shift
      ;;
    --server)
      SERVER_URL="${2:?--server needs a URL}"
      shift 2
      ;;
    --server=*)
      SERVER_URL="${1#--server=}"
      shift
      ;;
    -h|--help)
      sed -n '2,43p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      PROFILES+=("$1")
      shift
      ;;
  esac
done

PROFILE_A="${PROFILES[0]:-alice}"
PROFILE_B="${PROFILES[1]:-bob}"

# Prefer a freshly built app bundle in the repo; fall back to an installed
# copy in /Applications.
find_app_binary() {
  local candidates=(
    "$REPO_ROOT/target/release/bundle/macos/Seal.app/Contents/MacOS/Seal"
    "/Applications/Seal.app/Contents/MacOS/Seal"
  )
  for c in "${candidates[@]}"; do
    if [[ -x "$c" ]]; then
      echo "$c"
      return 0
    fi
  done
  return 1
}

APP_BIN="$(find_app_binary)" || {
  echo "Couldn't find a built app. Build one first:" >&2
  echo "  ./scripts/build-mac-app.sh    (or build-mac-dmg.sh)" >&2
  exit 1
}

echo "Using app binary: $APP_BIN"

# ---------------------------------------------------------------------------
# Local throwaway directory-server (skipped if --server was given)
# ---------------------------------------------------------------------------

DIR_SERVER_PID=""
TMP_DIR=""

if [[ -n "$SERVER_URL" ]]; then
  echo "Using directory server: $SERVER_URL (--server given, not starting a local one)"
else
  # Offset from the real defaults (8080/8090/4001) so this can run alongside
  # a real `directory-server` or `cargo run` dev instance without colliding.
  DS_PUBLIC_PORT=18080
  DS_ADMIN_PORT=18090
  DS_RELAY_PORT=14001

  TMP_DIR="$(mktemp -d -t seal-two-instances)"
  echo
  echo "-- Building directory-server (release) --"
  (cd "$REPO_ROOT" && cargo build --release -p directory-server --bin directory-server)
  DS_BIN="$REPO_ROOT/target/release/directory-server"

  if command -v openssl >/dev/null 2>&1; then
    DS_ADMIN_TOKEN="$(openssl rand -hex 32)"
  else
    DS_ADMIN_TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  fi

  echo "-- Starting a throwaway directory-server on 127.0.0.1:$DS_PUBLIC_PORT --"
  echo "   (relay on 127.0.0.1:$DS_RELAY_PORT, db + relay identity under $TMP_DIR)"
  DIRECTORY_DB_PATH="$TMP_DIR/directory.sqlite3" \
  DIRECTORY_PUBLIC_ADDR="127.0.0.1:$DS_PUBLIC_PORT" \
  DIRECTORY_ADMIN_ADDR="127.0.0.1:$DS_ADMIN_PORT" \
  DIRECTORY_ADMIN_TOKEN="$DS_ADMIN_TOKEN" \
  DIRECTORY_RELAY_ADDR="127.0.0.1:$DS_RELAY_PORT" \
  DIRECTORY_RELAY_KEY_PATH="$TMP_DIR/relay-identity.key" \
  DIRECTORY_RELAY_EXTERNAL_MULTIADDR="/ip4/127.0.0.1/tcp/$DS_RELAY_PORT" \
  RUST_LOG="${RUST_LOG:-info}" \
    "$DS_BIN" > "$TMP_DIR/directory-server.log" 2>&1 &
  DIR_SERVER_PID=$!

  SERVER_URL="http://127.0.0.1:$DS_PUBLIC_PORT"

  echo -n "Waiting for it to come up and advertise a relay"
  ready=0
  for _ in $(seq 1 50); do
    if curl -fsS "$SERVER_URL/v1/relay-info" >/dev/null 2>&1; then
      ready=1
      break
    fi
    if ! kill -0 "$DIR_SERVER_PID" 2>/dev/null; then
      echo
      echo "directory-server exited before starting up. Log:" >&2
      cat "$TMP_DIR/directory-server.log" >&2
      exit 1
    fi
    echo -n "."
    sleep 0.2
  done
  echo
  if [[ "$ready" -ne 1 ]]; then
    echo "Timed out waiting for $SERVER_URL/v1/relay-info. Log:" >&2
    cat "$TMP_DIR/directory-server.log" >&2
    kill "$DIR_SERVER_PID" 2>/dev/null || true
    exit 1
  fi
  echo "directory-server ready — pid $DIR_SERVER_PID, log at $TMP_DIR/directory-server.log"
fi
echo

# ---------------------------------------------------------------------------
# Launch both app instances
# ---------------------------------------------------------------------------

if [[ "$SIMULATE_WAN" -eq 1 ]]; then
  echo "Simulating two different networks: both instances will withhold their" \
       "LAN address and can only reach each other through the relay."
else
  echo "--same-lan: both instances will advertise their real LAN address," \
       "so they'll connect directly and never touch the relay/dcutr path."
fi
echo

# The app only checks whether P2P_CHAT_SIMULATE_WAN is *set* (see
# actor.rs), not its value -- so --same-lan has to omit it entirely rather
# than set it to "0".
launch_env_common=("P2P_CHAT_DIRECTORY_URL=$SERVER_URL")
if [[ "$SIMULATE_WAN" -eq 1 ]]; then
  launch_env_common+=("P2P_CHAT_SIMULATE_WAN=1")
fi

env "P2P_CHAT_PROFILE=$PROFILE_A" "${launch_env_common[@]}" "$APP_BIN" &
PID_A=$!
echo "Launched '$PROFILE_A' — pid $PID_A"

# Stagger slightly so the two windows don't both grab focus/layout at the
# exact same instant.
sleep 1

env "P2P_CHAT_PROFILE=$PROFILE_B" "${launch_env_common[@]}" "$APP_BIN" &
PID_B=$!
echo "Launched '$PROFILE_B' — pid $PID_B"

echo
echo "Both running against $SERVER_URL. Each has its own identity, keys,"
echo "contacts and messages; add each other as a contact using the ID from"
echo "Settings in one window, paste it into 'Add someone' in the other."
echo
if [[ -n "$DIR_SERVER_PID" ]]; then
  echo "Stop everything (apps + the throwaway directory-server) with:"
  echo "  kill $PID_A $PID_B $DIR_SERVER_PID"
  echo "(temp files under $TMP_DIR are left in place for inspection; rm -rf" \
       "it once you're done)"
else
  echo "Stop them with:"
  echo "  kill $PID_A $PID_B"
fi
