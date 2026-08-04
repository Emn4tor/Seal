#!/usr/bin/env bash
#
# Builds the macOS .dmg installer.
#
# Unsigned by default — if you have an Apple Developer ID, configure signing
# in apps/desktop/src-tauri/tauri.conf.json (bundle.macOS.signingIdentity)
# first, or Gatekeeper will warn people who open it that it's from an
# unidentified developer.
#
# Usage: ./scripts/build-mac-dmg.sh

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script builds a macOS .dmg and must run on macOS." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# If a previous run got killed or hung mid-bundle, it can leave its staging
# .dmg volume mounted. Tauri's DMG bundler drives Finder via AppleScript
# looking for a disk named exactly "Seal" — if a stale same-named volume is
# still mounted, Finder auto-renames the new one and the script waits
# forever for a disk name that no longer exists. Clear any of our own stale
# mounts before starting so this can't compound run over run.
while IFS= read -r mount_point; do
  echo "Detaching stale DMG staging volume: $mount_point"
  hdiutil detach "$mount_point" -force >/dev/null 2>&1 || true
done < <(hdiutil info | awk -v root="$REPO_ROOT/target/release/bundle/macos" '
  /^image-path/ { is_stale = index($0, root) > 0 }
  /\/Volumes\// && is_stale { print $NF }
')

cd "$REPO_ROOT/apps/desktop"

# Bakes the official Seal network in as the recommended server option on
# the first-run screen — a build-time default, not a runtime lock-in;
# "Custom server" and "Local test server" stay available regardless.
export SEAL_DEFAULT_DIRECTORY_URL="https://seal.emn4tor.de"

npm install
npm run tauri build -- --bundles dmg

echo
echo "Done. DMG at:"
find "$REPO_ROOT/target/release/bundle/dmg" -name '*.dmg'
