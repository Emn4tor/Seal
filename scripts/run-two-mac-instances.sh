#!/usr/bin/env bash
#
# Launches two instances of the built Seal app side by side on this Mac,
# each with its own identity, so you can add each other as contacts and
# actually test DMs/groups without a second machine. Mirrors the
# `P2P_CHAT_PROFILE` dev-mode pattern from README.md §3, but against the
# real compiled app instead of `npm run tauri dev`.
#
# Usage:
#   ./scripts/run-two-mac-instances.sh                # profiles: alice, bob
#   ./scripts/run-two-mac-instances.sh carol dave      # custom profile names
#
# Both instances share the same directory-server choice (server.json is
# per-machine, not per-profile — see README.md §3) but keep fully separate
# local identities, keys, contacts, and messages.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script launches the macOS app and must run on macOS." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PROFILE_A="${1:-alice}"
PROFILE_B="${2:-bob}"

# Prefer a freshly built app bundle in the repo; fall back to an installed
# copy in /Applications.
find_app_binary() {
  local candidates=(
    "$REPO_ROOT/target/release/bundle/macos/desktop.app/Contents/MacOS/desktop"
    "/Applications/desktop.app/Contents/MacOS/desktop"
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
echo

P2P_CHAT_PROFILE="$PROFILE_A" "$APP_BIN" &
PID_A=$!
echo "Launched '$PROFILE_A' — pid $PID_A"

# Stagger slightly so the two windows don't both grab focus/layout at the
# exact same instant.
sleep 1

P2P_CHAT_PROFILE="$PROFILE_B" "$APP_BIN" &
PID_B=$!
echo "Launched '$PROFILE_B' — pid $PID_B"

echo
echo "Both running. Each has its own identity, keys, contacts and messages;"
echo "add each other as a contact using the ID from Settings in one window,"
echo "paste it into 'Add someone' in the other."
echo
echo "Stop them with:"
echo "  kill $PID_A $PID_B"
