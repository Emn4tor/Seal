#!/usr/bin/env bash
#
# Builds the Linux app bundles: an AppImage (runs on most distros without
# installing anything) and a .deb (Debian/Ubuntu). See README.md §1 for the
# system packages you need installed first (webkit2gtk, etc.).
#
# Usage: ./scripts/build-linux.sh

set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script builds the Linux app and must run on Linux." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT/apps/desktop"

npm install
npm run tauri build -- --bundles appimage,deb

echo
echo "Done. Bundles at:"
find "$REPO_ROOT/target/release/bundle/appimage" "$REPO_ROOT/target/release/bundle/deb" \
  -type f \( -name '*.AppImage' -o -name '*.deb' \) 2>/dev/null
