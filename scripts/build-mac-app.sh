#!/usr/bin/env bash
#
# Builds the raw macOS .app bundle — no .dmg installer wrapping it. Useful
# for testing a real release build locally, or distributing outside the
# usual installer flow (e.g. zipping it yourself).
#
# Unsigned by default; see build-mac-dmg.sh's header for signing notes —
# the same applies here.
#
# Usage: ./scripts/build-mac-app.sh

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script builds a macOS .app and must run on macOS." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT/apps/desktop"

# Bakes the official Seal network in as the recommended server option on
# the first-run screen — a build-time default, not a runtime lock-in;
# "Custom server" and "Local test server" stay available regardless.
export SEAL_DEFAULT_DIRECTORY_URL="https://seal.emn4tor.de"

npm install
npm run tauri build -- --bundles app

echo
echo "Done. App bundle at:"
find "$REPO_ROOT/target/release/bundle/macos" -maxdepth 1 -name '*.app'
