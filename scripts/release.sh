#!/usr/bin/env bash
#
# Bumps the project version in every place docs/RELEASING.md says it has to
# live by hand (Cargo.toml, the desktop app's own Cargo.toml,
# tauri.conf.json, package.json), refreshes Cargo.lock so it isn't left
# stale, and tags the current commit. Deliberately does NOT commit or push
# anything — review the diff and commit/push it yourself (the final printed
# instructions cover why the tag has to move *after* you commit, not before).
#
# Usage: ./scripts/release.sh vX.Y.Z
#   e.g. ./scripts/release.sh v0.2.1

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 vX.Y.Z" >&2
  exit 1
fi

TAG="$1"
if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Expected a tag like v0.2.1 (matches release.yml's 'v*' trigger and" >&2
  echo "tauri-action's v__VERSION__ tagName) — got: $TAG" >&2
  exit 1
fi
VERSION="${TAG#v}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

for cmd in git cargo; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "This script needs '$cmd' on PATH." >&2
    exit 1
  fi
done

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Working tree isn't clean — commit or stash first, so the version bump" >&2
  echo "doesn't get tangled up with unrelated changes." >&2
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "Tag $TAG already exists." >&2
  exit 1
fi

# Portable in-place edit: BSD sed (macOS) and GNU sed (Linux) disagree on
# `-i` syntax (`-i ''` vs `-i''`/`-i`), so this sidesteps `-i` entirely —
# `cat > file` rather than `mv` so the original file's permissions and
# inode survive the edit.
replace_in_file() {
  local file="$1" script="$2"
  local tmp
  tmp="$(mktemp)"
  sed -E "$script" "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
  if ! grep -qF "$VERSION" "$file"; then
    echo "Failed to bump version in $file — the replacement pattern didn't" >&2
    echo "match anything. Has that file's version field changed shape?" >&2
    exit 1
  fi
}

echo "Bumping version to $VERSION..."

replace_in_file "Cargo.toml" \
  "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"\$/version = \"$VERSION\"/"
echo "  Cargo.toml (workspace)"

replace_in_file "apps/desktop/src-tauri/Cargo.toml" \
  "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"\$/version = \"$VERSION\"/"
echo "  apps/desktop/src-tauri/Cargo.toml"

replace_in_file "apps/desktop/src-tauri/tauri.conf.json" \
  "s/^([[:space:]]*\"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\",?)\$/\1$VERSION\2/"
echo "  apps/desktop/src-tauri/tauri.conf.json"

replace_in_file "apps/desktop/package.json" \
  "s/^([[:space:]]*\"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\",?)\$/\1$VERSION\2/"
echo "  apps/desktop/package.json"

echo
echo "Refreshing Cargo.lock..."
cargo check --workspace --quiet

echo
echo "Tagging $TAG..."
git tag "$TAG"

cat <<EOF

Done. Version bumped to $VERSION and tag '$TAG' created locally.

IMPORTANT: the tag points at the current commit, which does NOT include this
version bump yet. Before pushing:

  1. Review:   git status && git diff
  2. Commit:   git add -A && git commit -m "chore: Bump version to $VERSION"
  3. Move the tag onto that new commit (it's still on the old one):
               git tag -f $TAG
  4. Push both:
               git push
               git push origin $TAG

Pushing '$TAG' before step 3 triggers a release build whose tag says
$VERSION but whose files (tauri.conf.json included) still say the old one.
EOF
