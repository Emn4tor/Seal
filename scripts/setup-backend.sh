#!/usr/bin/env bash
#
# Interactive setup for the Seal directory server (crates/directory-server).
# Asks which Linux distribution you're on, installs that distro's build
# prerequisites, builds the release binary, creates a dedicated system user,
# writes a systemd service, and — if you want — installs Caddy and
# configures a domain with automatic HTTPS. Safe to re-run: it won't clobber
# an existing admin token or overwrite the Caddy block for a different
# domain without asking first.
#
# Targets Linux + systemd (so: not Alpine, which uses OpenRC — that would
# need a genuinely different service-file code path, not just a different
# package manager). For local Mac testing, see README.md §3 ("running it in
# dev mode" / the embedded server) or just run:
#   cargo run --release -p directory-server --bin directory-server
#
# Usage: sudo ./scripts/setup-backend.sh

set -euo pipefail

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script targets Linux servers with systemd." >&2
  echo "For local/Mac testing, run: cargo run --release -p directory-server --bin directory-server" >&2
  exit 1
fi

if [[ $EUID -ne 0 ]]; then
  echo "This needs root — it creates a system user, writes a systemd unit," >&2
  echo "and installs packages. Re-run with sudo:" >&2
  echo "  sudo $0" >&2
  exit 1
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "No systemctl found — this script needs systemd. If you're on Alpine or" >&2
  echo "another OpenRC/non-systemd distro, this script won't work as-is; you'd" >&2
  echo "need an OpenRC init script instead of the systemd unit this writes." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_USER="seal-directory"
SERVICE_GROUP="seal-directory"
DATA_DIR="/var/lib/seal-directory"
CONFIG_DIR="/etc/seal-directory"
TOKEN_FILE="$CONFIG_DIR/admin-token.env"
UNIT_FILE="/etc/systemd/system/seal-directory.service"
BIN_PATH="/usr/local/bin/directory-server"

echo "== Seal directory server setup =="
echo "Repo: $REPO_ROOT"
echo

# ---------------------------------------------------------------------------
# 1. Which distro?
# ---------------------------------------------------------------------------
#
# Asked rather than silently auto-detected, per the brief — but pre-filled
# with a best guess from /etc/os-release so it's a one-keystroke confirm on
# the common case rather than real friction.

detect_distro_guess() {
  local id="" id_like=""
  if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    id="${ID:-}"
    id_like="${ID_LIKE:-}"
  fi
  case "$id $id_like" in
    *debian*|*ubuntu*) echo "debian" ;;
    *fedora*|*rhel*|*centos*) echo "fedora" ;;
    *arch*) echo "arch" ;;
    *suse*) echo "opensuse" ;;
    *) echo "" ;;
  esac
}

prompt_distro() {
  local guess choice
  guess="$(detect_distro_guess)"

  echo "Which Linux distribution family is this?" >&2
  echo "  1) Debian / Ubuntu / Mint / Pop!_OS          (apt)" >&2
  echo "  2) Fedora / RHEL / CentOS Stream / Rocky / Alma (dnf)" >&2
  echo "  3) Arch / Manjaro / EndeavourOS              (pacman)" >&2
  echo "  4) openSUSE / SLES                            (zypper)" >&2
  if [[ -n "$guess" ]]; then
    echo "Detected: looks like '$guess' — press Enter to accept, or pick a number." >&2
  fi

  local default_num=""
  case "$guess" in
    debian) default_num=1 ;;
    fedora) default_num=2 ;;
    arch) default_num=3 ;;
    opensuse) default_num=4 ;;
  esac

  read -rp "Enter 1-4${default_num:+ [$default_num]}: " choice
  choice="${choice:-$default_num}"
  case "$choice" in
    1) echo "debian" ;;
    2) echo "fedora" ;;
    3) echo "arch" ;;
    4) echo "opensuse" ;;
    *) echo "Unrecognized choice: '$choice'." >&2; exit 1 ;;
  esac
}

DISTRO="$(prompt_distro)"
echo "Using: $DISTRO"
echo

# ---------------------------------------------------------------------------
# 2. Per-distro build prerequisites
# ---------------------------------------------------------------------------
# A C compiler + pkg-config (directory-server bundle-compiles SQLite from
# source), plus curl/gnupg/ca-certificates/openssl, which the later Caddy
# and admin-token steps need regardless of which path they take.

install_prereqs_debian() {
  apt-get update
  apt-get install -y build-essential pkg-config curl gnupg ca-certificates openssl
}

install_prereqs_fedora() {
  dnf install -y gcc gcc-c++ make pkgconf-pkg-config curl gnupg2 ca-certificates openssl
}

install_prereqs_arch() {
  pacman -Sy --noconfirm base-devel pkgconf curl gnupg ca-certificates openssl
}

install_prereqs_opensuse() {
  zypper --non-interactive install -y gcc gcc-c++ make pkg-config curl gpg2 ca-certificates openssl
}

echo "-- Installing build prerequisites for $DISTRO --"
"install_prereqs_$DISTRO"
echo

# ---------------------------------------------------------------------------
# 3. Rust toolchain (universal — rustup, not a distro package)
# ---------------------------------------------------------------------------

if ! command -v cargo >/dev/null 2>&1; then
  echo "-- cargo not found --"
  read -rp "Install Rust now via rustup? [Y/n] " install_rust
  install_rust="${install_rust:-y}"
  if [[ "$install_rust" =~ ^[Yy] ]]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
  else
    echo "Install Rust yourself (https://rustup.rs) and re-run this script." >&2
    exit 1
  fi
fi
echo

# ---------------------------------------------------------------------------
# 4. Build
# ---------------------------------------------------------------------------

echo "-- Building the release binary (this can take a few minutes the first time) --"
(cd "$REPO_ROOT" && cargo build --release -p directory-server --bin directory-server)
install -m 755 "$REPO_ROOT/target/release/directory-server" "$BIN_PATH"
echo "Installed to $BIN_PATH"
echo

# ---------------------------------------------------------------------------
# 5. System user + data/config dirs
# ---------------------------------------------------------------------------

if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
  echo "-- Creating system user '$SERVICE_USER' --"
  useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
else
  echo "-- System user '$SERVICE_USER' already exists, leaving it as-is --"
fi

install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 750 "$DATA_DIR"
install -d -o root -g root -m 750 "$CONFIG_DIR"
echo

# ---------------------------------------------------------------------------
# 6. Admin token (generated once, never rotated by re-running this script)
# ---------------------------------------------------------------------------

if [[ -f "$TOKEN_FILE" ]]; then
  echo "-- Admin token already exists at $TOKEN_FILE, keeping it --"
else
  echo "-- Generating an admin token --"
  if command -v openssl >/dev/null 2>&1; then
    TOKEN="$(openssl rand -hex 32)"
  else
    TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  fi
  printf 'DIRECTORY_ADMIN_TOKEN=%s\n' "$TOKEN" > "$TOKEN_FILE"
  chmod 600 "$TOKEN_FILE"
  chown root:root "$TOKEN_FILE"
  echo "Saved to $TOKEN_FILE (root-readable only)."
  echo "This is what you pass to 'directory-admin' to purge the server. Back it up somewhere safe:"
  echo "  $TOKEN"
fi
echo

# ---------------------------------------------------------------------------
# 7. Domain + SSL, or not
# ---------------------------------------------------------------------------

# Universal fallback: Caddy's own prebuilt static binary. Used for openSUSE
# (whose repos don't reliably carry Caddy) and as a safety net if a
# distro-specific package install fails for any reason.
#
# NOTE: relies on caddyserver.com's public download endpoint — verify this
# still resolves the way it did when this script was written if it ever
# stops working; Caddy's own install docs (https://caddyserver.com/docs/install)
# are the source of truth.
install_caddy_static_binary() {
  echo "Installing Caddy from its official static binary release..."
  local arch
  case "$(uname -m)" in
    x86_64) arch="amd64" ;;
    aarch64|arm64) arch="arm64" ;;
    armv7l) arch="armv7" ;;
    *)
      echo "No known Caddy static binary for architecture '$(uname -m)'." >&2
      echo "Install Caddy yourself: https://caddyserver.com/docs/install" >&2
      return 1
      ;;
  esac
  local tmp
  tmp="$(mktemp -d)"
  curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=${arch}" -o "$tmp/caddy"
  chmod +x "$tmp/caddy"
  install -m 755 "$tmp/caddy" /usr/local/bin/caddy
  rm -rf "$tmp"

  mkdir -p /etc/caddy
  touch /etc/caddy/Caddyfile
  if [[ ! -f /etc/systemd/system/caddy.service ]]; then
    cat > /etc/systemd/system/caddy.service <<'EOF'
[Unit]
Description=Caddy
After=network.target

[Service]
Type=notify
ExecStart=/usr/local/bin/caddy run --environ --config /etc/caddy/Caddyfile
ExecReload=/usr/local/bin/caddy reload --config /etc/caddy/Caddyfile --force
TimeoutStopSec=5s
LimitNOFILE=1048576
PrivateTmp=true
ProtectSystem=full
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
  fi
}

install_caddy_debian() {
  apt-get install -y debian-keyring debian-archive-keyring apt-transport-https
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-stable.list
  apt-get update
  apt-get install -y caddy
}

install_caddy_fedora() {
  dnf install -y 'dnf-command(copr)'
  dnf copr enable -y @caddy/caddy
  dnf install -y caddy
}

install_caddy_arch() {
  pacman -Sy --noconfirm caddy
}

install_caddy_opensuse() {
  # zypper repo availability for Caddy varies by openSUSE version — go
  # straight to the static binary rather than guess a repo name.
  install_caddy_static_binary
}

install_caddy() {
  if command -v caddy >/dev/null 2>&1; then
    echo "Caddy is already installed, skipping."
    return
  fi
  if ! "install_caddy_$DISTRO"; then
    echo "Distro-specific Caddy install failed, falling back to the static binary." >&2
    install_caddy_static_binary
  fi
}

read -rp "Set up a domain with automatic HTTPS for you (via Caddy)? [y/N] " setup_ssl
setup_ssl="${setup_ssl:-n}"

PUBLIC_ADDR="127.0.0.1:8080"
SERVER_URL=""

if [[ "$setup_ssl" =~ ^[Yy] ]]; then
  read -rp "Domain or subdomain to use (e.g. directory.example.com): " DOMAIN
  if [[ -z "$DOMAIN" ]]; then
    echo "No domain entered, aborting." >&2
    exit 1
  fi

  echo
  echo "-- Installing Caddy if it isn't already --"
  install_caddy

  echo
  echo "-- Configuring Caddy for $DOMAIN --"
  CADDY_BLOCK_MARKER="# seal-directory ($DOMAIN)"
  CADDYFILE="/etc/caddy/Caddyfile"
  mkdir -p "$(dirname "$CADDYFILE")"
  touch "$CADDYFILE"
  if grep -qF "$CADDY_BLOCK_MARKER" "$CADDYFILE" 2>/dev/null; then
    echo "Caddyfile already has a block for $DOMAIN, leaving it as-is."
  else
    {
      echo ""
      echo "$CADDY_BLOCK_MARKER"
      echo "$DOMAIN {"
      echo "    reverse_proxy 127.0.0.1:8080"
      echo "}"
    } >> "$CADDYFILE"
    echo "Added a reverse-proxy block to $CADDYFILE."
  fi
  systemctl enable --now caddy >/dev/null 2>&1 || true
  systemctl reload caddy 2>/dev/null || systemctl restart caddy

  echo "Make sure $DOMAIN's DNS A/AAAA record points at this server before" \
       "traffic arrives — Caddy requests the certificate on first request."

  PUBLIC_ADDR="127.0.0.1:8080"
  SERVER_URL="https://$DOMAIN"
else
  echo
  echo "Skipping domain/SSL setup — you'll front this yourself, or use it as plain HTTP."
  read -rp "Bind directly to all interfaces on plain HTTP (0.0.0.0:8080)? [y/N] " bind_public
  bind_public="${bind_public:-n}"
  if [[ "$bind_public" =~ ^[Yy] ]]; then
    PUBLIC_ADDR="0.0.0.0:8080"
    SERVER_URL="http://$(hostname -I 2>/dev/null | awk '{print $1}'):8080"
  else
    PUBLIC_ADDR="127.0.0.1:8080"
    SERVER_URL="http://127.0.0.1:8080"
    echo "Bound to loopback only — put your own reverse proxy in front of 127.0.0.1:8080"
    echo "(or SSH-tunnel to it) when you're ready to expose it."
  fi
fi
echo

# ---------------------------------------------------------------------------
# 8. systemd unit
# ---------------------------------------------------------------------------

echo "-- Writing systemd service --"
cat > "$UNIT_FILE" <<EOF
[Unit]
Description=Seal directory server
After=network.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_GROUP
Environment=DIRECTORY_DB_PATH=$DATA_DIR/directory.sqlite3
Environment=DIRECTORY_PUBLIC_ADDR=$PUBLIC_ADDR
Environment=DIRECTORY_ADMIN_ADDR=127.0.0.1:8090
EnvironmentFile=$TOKEN_FILE
ExecStart=$BIN_PATH
Restart=on-failure

ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
NoNewPrivileges=true
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now seal-directory
echo

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo "== Done =="
echo "Distro:                  $DISTRO"
echo "Server URL for clients:  $SERVER_URL"
echo "Admin token:             $TOKEN_FILE (root-readable only)"
echo "Logs:                    journalctl -u seal-directory -f"
echo "Purge everything:        cargo run --release -p directory-server --bin directory-admin -- \\"
echo "                           --admin-url http://127.0.0.1:8090 \\"
echo "                           --token \"\$(grep -oP '(?<==).*' $TOKEN_FILE)\" purge"
echo
echo "Point the app at this server by launching it with:"
echo "  P2P_CHAT_DIRECTORY_URL=$SERVER_URL npm run tauri dev"
echo "or by entering it as a custom server in the app's first-run screen / Settings."
