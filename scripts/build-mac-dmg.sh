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
cd "$REPO_ROOT/apps/desktop"

npm install
npm run tauri build -- --bundles dmg

echo
echo "Done. DMG at:"
find "$REPO_ROOT/target/release/bundle/dmg" -name '*.dmg'
