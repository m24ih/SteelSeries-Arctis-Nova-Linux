#!/usr/bin/env bash
# Simple installer/uninstaller for arctis_chatmix
# Usage:
#   ./install.sh              # Install or upgrade (user service + udev + rtkit)
#   ./install.sh --system     # Install or upgrade system-wide service
#   ./install.sh --uninstall  # Remove service, binary, and udev rule
set -euo pipefail

BINARY=""
SYSTEM_MODE="no"
UNINSTALL="no"
REPO="m24ih/SteelSeries-Arctis-Nova-Linux"
RELEASE_BIN_URL="https://github.com/${REPO}/releases/latest/download/arctis_chatmix"
TMP_DIR=""

cleanup() {
  if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

print_help() {
  cat <<'USAGE'
Usage: install.sh [OPTIONS]

Install, upgrade, or uninstall arctis_chatmix.
With no options, installs/upgrades as a user systemd service, configures udev rules,
ensures audio group membership, and starts/restarts the service.

Options:
  --system         Install or uninstall system-wide instead of per-user
  --uninstall      Remove arctis_chatmix service, binary, and udev rule
  --binary PATH    Use a specific arctis_chatmix binary
  -h, --help       Show this help and exit
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
  --system)
    SYSTEM_MODE="yes"
    shift
    ;;
  --uninstall)
    UNINSTALL="yes"
    shift
    ;;
  --binary)
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
      echo "Error: --binary requires a file path." >&2
      exit 2
    fi
    BINARY="$2"
    shift 2
    ;;
  -h | --help)
    print_help
    exit 0
    ;;
  *)
    echo "Unknown option: $1" >&2
    print_help
    exit 2
    ;;
  esac
done

run_privileged() {
  if [[ $EUID -ne 0 ]]; then
    sudo "$@"
  else
    "$@"
  fi
}

SCRIPT_PATH="${BASH_SOURCE[0]:-}"
if [[ -n "$SCRIPT_PATH" && -f "$SCRIPT_PATH" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
else
  SCRIPT_DIR="$PWD"
fi
REAL_USER="${SUDO_USER:-$(id -un)}"
REAL_HOME="$(getent passwd "$REAL_USER" 2>/dev/null | cut -d: -f6 || true)"
[[ -z "$REAL_HOME" ]] && REAL_HOME="$HOME"
REAL_UID="$(id -u "$REAL_USER")"

if [[ -n "${XDG_CONFIG_HOME:-}" && "$XDG_CONFIG_HOME" != "/root/.config" ]]; then
  CONFIG_DIR="$XDG_CONFIG_HOME/arctis-nova-7"
else
  CONFIG_DIR="$REAL_HOME/.config/arctis-nova-7"
fi

SERVICE_NAME="arctis_chatmix.service"
USER_BIN_DIR="$REAL_HOME/.local/bin"
USER_UNIT_DIR="$REAL_HOME/.config/systemd/user"
SYSTEM_BIN_DIR="/usr/local/bin"
SYSTEM_UNIT_DIR="/etc/systemd/system"
UDEV_RULE_PATH="/etc/udev/rules.d/99-arctis.rules"
WEB_DASHBOARD_URL="http://127.0.0.1:3927"

# All 23 SteelSeries Arctis Nova 7 RX & TX Product IDs
PIDS=(
  "2200" "2202" "2204" "2206" "2208" "220a"
  "2238" "223a" "2256" "2258" "2278" "227a"
  "227c" "227e" "2288" "228a" "229c" "229e"
  "22a1" "22a4" "22a5" "22a9" "22ad"
)

run_user_systemctl() {
  if [[ $EUID -eq 0 && "$REAL_USER" != "root" ]]; then
    sudo -u "$REAL_USER" XDG_RUNTIME_DIR="/run/user/${REAL_UID}" systemctl --user "$@"
  else
    systemctl --user "$@"
  fi
}

download_file() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    return 1
  fi
}

resolve_binary() {
  if [[ -n "$BINARY" ]]; then
    if [[ ! -f "$BINARY" ]]; then
      echo "Binary not found: $BINARY" >&2
      exit 2
    fi
    return
  fi

  if [[ -f "./arctis_chatmix" ]]; then
    BINARY="./arctis_chatmix"
  elif [[ -f "${SCRIPT_DIR}/target/release/arctis_chatmix" ]]; then
    BINARY="${SCRIPT_DIR}/target/release/arctis_chatmix"
  elif [[ -f "${SCRIPT_DIR}/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1; then
    echo "No prebuilt binary found; building release binary with cargo..."
    (cd "$SCRIPT_DIR" && cargo build --release)
    BINARY="${SCRIPT_DIR}/target/release/arctis_chatmix"
  else
    TMP_DIR="$(mktemp -d)"
    local arch
    arch="$(uname -m)"
    if [[ "$arch" == "x86_64" ]]; then
      echo "Downloading latest prebuilt binary from GitHub Releases..."
      if download_file "$RELEASE_BIN_URL" "${TMP_DIR}/arctis_chatmix"; then
        chmod +x "${TMP_DIR}/arctis_chatmix"
        BINARY="${TMP_DIR}/arctis_chatmix"
        return
      fi
      echo "[warn] Prebuilt release download failed; falling back to source build..." >&2
    fi

    if command -v git >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
      echo "Cloning shallow repository and building with cargo..."
      git clone --depth 1 "https://github.com/${REPO}.git" "${TMP_DIR}/repo"
      (cd "${TMP_DIR}/repo" && cargo build --release)
      BINARY="${TMP_DIR}/repo/target/release/arctis_chatmix"
    else
      echo "Could not download prebuilt binary and git/cargo are not available." >&2
      exit 2
    fi
  fi
}

init_user_config() {
  local bin_path="$1"
  if [[ $EUID -eq 0 && "$REAL_USER" != "root" ]]; then
    sudo -u "$REAL_USER" HOME="$REAL_HOME" mkdir -p "$CONFIG_DIR/profiles"
    sudo -u "$REAL_USER" HOME="$REAL_HOME" "$bin_path" --init-config 2>/dev/null || true
  else
    mkdir -p "$CONFIG_DIR/profiles"
    "$bin_path" --init-config 2>/dev/null || true
  fi
}

ensure_audio_group() {
  getent group audio >/dev/null || run_privileged groupadd -r audio
  if [[ "$REAL_USER" != "root" ]] && ! id -nG "$REAL_USER" | grep -qw audio; then
    echo "Adding user '$REAL_USER' to 'audio' group..."
    run_privileged usermod -aG audio "$REAL_USER"
    echo "NOTE: Log out and log back in for audio group changes to take effect."
  fi
}

install_udev() {
  echo "Installing udev rule to ${UDEV_RULE_PATH}..."
  if [[ -f "${SCRIPT_DIR}/config/99-arctis.rules" ]]; then
    run_privileged install -Dm644 "${SCRIPT_DIR}/config/99-arctis.rules" "${UDEV_RULE_PATH}"
  else
    local udev_content=""
    for pid in "${PIDS[@]}"; do
      udev_content+='ATTRS{idVendor}=="1038", ATTRS{idProduct}=="'"$pid"'", MODE="0660", GROUP="audio", TAG+="uaccess"
KERNEL=="hidraw*", ATTRS{idVendor}=="1038", ATTRS{idProduct}=="'"$pid"'", MODE="0660", GROUP="audio", TAG+="uaccess"
ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="1038", ATTR{idProduct}=="'"$pid"'", TEST=="power/control", ATTR{power/control}="on"
'
    done
    echo "$udev_content" | run_privileged tee "${UDEV_RULE_PATH}" >/dev/null
  fi
  run_privileged udevadm control --reload
  for pid in "${PIDS[@]}"; do
    run_privileged udevadm trigger --subsystem-match=usb --attr-match=idVendor=1038 --attr-match=idProduct="$pid" 2>/dev/null || true
    run_privileged udevadm trigger --subsystem-match=hidraw --attr-match=idVendor=1038 --attr-match=idProduct="$pid" 2>/dev/null || true
  done
}

install_rtkit() {
  if systemctl status rtkit-daemon.service >/dev/null 2>&1; then
    return
  fi
  echo "Installing and enabling rtkit (RealtimeKit)..."
  if command -v pacman >/dev/null 2>&1; then
    run_privileged pacman -S --noconfirm rtkit
  elif command -v apt-get >/dev/null 2>&1; then
    run_privileged apt-get install -y rtkit
  elif command -v dnf >/dev/null 2>&1; then
    run_privileged dnf install -y rtkit
  else
    echo "[warn] Could not detect package manager; install 'rtkit' manually if needed."
    return
  fi
  run_privileged systemctl enable --now rtkit-daemon.service 2>/dev/null || true
}

install_user() {
  mkdir -p "$USER_BIN_DIR" "$USER_UNIT_DIR"
  install -m 755 "$BINARY" "$USER_BIN_DIR/arctis_chatmix"
  init_user_config "$USER_BIN_DIR/arctis_chatmix"

  cat >"${USER_UNIT_DIR}/${SERVICE_NAME}" <<'UNIT'
[Unit]
Description=Arctis Nova 7 ChatMix, 10-Band DSP EQ & Web Companion Service
Documentation=https://github.com/m24ih/Arctis-Nova-7-Chatmix
Wants=pipewire.service
After=pipewire.service graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=%h/.local/bin/arctis_chatmix
PassEnvironment=DISPLAY WAYLAND_DISPLAY XDG_CURRENT_DESKTOP DBUS_SESSION_BUS_ADDRESS
Environment=ARCTIS_SIDETONE_DISABLE=1
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT

  if [[ $EUID -eq 0 && "$REAL_USER" != "root" ]]; then
    chown -R "$REAL_USER":"$(id -gn "$REAL_USER")" "$USER_BIN_DIR/arctis_chatmix" "${USER_UNIT_DIR}/${SERVICE_NAME}" 2>/dev/null || true
  fi

  run_user_systemctl daemon-reload
  run_user_systemctl enable "$SERVICE_NAME"
  run_user_systemctl restart "$SERVICE_NAME"
}

install_system() {
  echo "Installing system-wide..."
  run_privileged install -m 755 "$BINARY" "${SYSTEM_BIN_DIR}/arctis_chatmix"
  init_user_config "${SYSTEM_BIN_DIR}/arctis_chatmix"

  local unit_content
  read -r -d '' unit_content <<'UNIT' || true
[Unit]
Description=Arctis Nova 7 ChatMix, 10-Band DSP EQ & Web Companion Service
Documentation=https://github.com/m24ih/Arctis-Nova-7-Chatmix
Wants=pipewire.service
After=pipewire.service

[Service]
Type=simple
ExecStart=/usr/local/bin/arctis_chatmix
Environment=ARCTIS_SIDETONE_DISABLE=1
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT

  echo "$unit_content" | run_privileged tee "${SYSTEM_UNIT_DIR}/${SERVICE_NAME}" >/dev/null
  run_privileged systemctl daemon-reload
  run_privileged systemctl enable "$SERVICE_NAME"
  run_privileged systemctl restart "$SERVICE_NAME"
}

run_install() {
  echo "== arctis_chatmix installer =="
  resolve_binary
  if [[ "$SYSTEM_MODE" == "yes" ]]; then
    install_system
  else
    install_user
  fi
  ensure_audio_group || echo "[warn] Could not configure audio group automatically."
  install_udev
  install_rtkit

  echo ""
  echo "✓ Done."
  echo "  • Config & EQ Profiles: $CONFIG_DIR"
  echo "  • Web Companion UI:     $WEB_DASHBOARD_URL"
}

run_uninstall() {
  echo "== arctis_chatmix uninstaller =="
  if [[ "$SYSTEM_MODE" == "yes" ]]; then
    run_privileged systemctl stop "$SERVICE_NAME" 2>/dev/null || true
    run_privileged systemctl disable "$SERVICE_NAME" 2>/dev/null || true
    run_privileged rm -f "${SYSTEM_UNIT_DIR}/${SERVICE_NAME}" "${SYSTEM_BIN_DIR}/arctis_chatmix"
    run_privileged systemctl daemon-reload
  else
    run_user_systemctl stop "$SERVICE_NAME" 2>/dev/null || true
    run_user_systemctl disable "$SERVICE_NAME" 2>/dev/null || true
    rm -f "${USER_UNIT_DIR}/${SERVICE_NAME}" "${USER_BIN_DIR}/arctis_chatmix"
    run_user_systemctl daemon-reload 2>/dev/null || true
  fi

  if [[ -f "$UDEV_RULE_PATH" ]]; then
    run_privileged rm -f "$UDEV_RULE_PATH"
    run_privileged udevadm control --reload
  fi

  echo ""
  echo "✓ Uninstall complete."
  echo "  Note: User configuration and EQ profiles in $CONFIG_DIR were preserved."
}

if [[ "$UNINSTALL" == "yes" ]]; then
  run_uninstall
else
  run_install
fi
