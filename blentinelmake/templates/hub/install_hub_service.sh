#!/bin/bash
# Install or upgrade the Blentinel hub as a systemd service.
#
#   program files  /opt/blentinel/hub      replaced on every upgrade
#   configuration  /etc/blentinel/blentinel_hub.toml   written only on first install
#   state          /var/lib/blentinel      database, key, setup code: never touched
#
# Run from the unpacked package folder: sudo ./install_hub_service.sh
set -euo pipefail

INSTALL_DIR=/opt/blentinel/hub
STATE_DIR=/var/lib/blentinel
CONFIG_DIR=/etc/blentinel
CONFIG="$CONFIG_DIR/blentinel_hub.toml"
SERVICE_USER=blentinel
SERVICE=blentinel-hub

cd "$(dirname "$0")"
if [ "$(id -u)" -ne 0 ]; then
    echo "Please run as root: sudo $0" >&2
    exit 1
fi

# Service user (no login, no home)
if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
fi

# Program files: only what this package ships (never runtime files that may
# have been left in the package folder).
systemctl stop "$SERVICE" 2>/dev/null || true
install -d -m 0755 "$INSTALL_DIR"
install -m 0755 hub "$INSTALL_DIR/hub"
rm -rf "$INSTALL_DIR/pkg"
cp -r pkg "$INSTALL_DIR/pkg"
if [ -f favicon.ico ]; then install -m 0644 favicon.ico "$INSTALL_DIR/favicon.ico"; fi

# State directory, readable only by the service user
install -d -m 0700 -o "$SERVICE_USER" -g "$SERVICE_USER" "$STATE_DIR"

# Configuration: first install only, pointed at the state directory
if [ ! -f "$CONFIG" ]; then
    install -d -m 0755 "$CONFIG_DIR"
    sed "s|^state_dir = .*|state_dir = \"$STATE_DIR\"|" blentinel_hub.toml > "$CONFIG"
    chown root:"$SERVICE_USER" "$CONFIG"
    chmod 0640 "$CONFIG"
    FIRST_INSTALL=1
fi

install -m 0644 blentinel-hub.service /etc/systemd/system/"$SERVICE".service
systemctl daemon-reload
systemctl enable "$SERVICE"
systemctl restart "$SERVICE"

echo "Blentinel hub installed: $INSTALL_DIR (state: $STATE_DIR, config: $CONFIG)"
if [ "${FIRST_INSTALL:-0}" = 1 ]; then
    echo
    echo "First install:"
    echo "  - The hub listens on the host/port in $CONFIG (127.0.0.1:3000 by default)."
    echo "    Set host = \"0.0.0.0\" so probes on other machines can reach it, then:"
    echo "    sudo systemctl restart $SERVICE"
    echo "  - Open the web UI and create the first Owner with the setup code:"
    echo "    sudo cat $STATE_DIR/setup.code   (also in the log)"
fi
echo "Logs: journalctl -u $SERVICE -f"
