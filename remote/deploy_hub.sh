#!/bin/bash
set -euo pipefail

BASE_DIR="/home/user/blentinel-builder/blentinel"
VPS="ubuntu@141.147.23.56"

cd "$BASE_DIR"

# Find newest publish zip
LATEST=$(ls -t publish/hub-*.zip 2>/dev/null | head -n1 || true)
echo "Deploying release: $LATEST"

if [ -z "${LATEST:-}" ]; then
    echo "Error: No published file matching 'publish/hub-*.zip' found"
    exit 1
fi

# Upload zipped publication file
scp "$LATEST" "$VPS:/tmp/blentinel-hub.zip"

# Install or update remotely
ssh "$VPS" << 'EOF'

set -euo pipefail

rm -rf /tmp/blentinel-hub
mkdir -p /tmp/blentinel-hub

unzip -q /tmp/blentinel-hub.zip -d /tmp/blentinel-hub

# publish zip contains "app/"
APP_DIR=$(find /tmp/blentinel-hub -type d -name app | head -n1)

if [ -z "${APP_DIR:-}" ]; then
    echo "ERROR: app directory not found in publish zip"
    exit 1
fi

# if systemctl status blentinel-hub >/dev/null 2>&1; then
if sudo systemctl is-enabled blentinel-hub >/dev/null 2>&1; then
    echo "Updating existing installation ..."

    sudo rsync -av \
        --exclude "blentinel_hub.toml" \
        --exclude "hub_identity.key" \
        --exclude "hub_auth.token" \
        --exclude "blentinel.db*" \
        --exclude "blentinel-hub.service" \
        "$APP_DIR/" /opt/blentinel/hub/

    sudo chown -R ubuntu:ubuntu /opt/blentinel/hub

    echo "Reload systemd units..."
    sudo systemctl daemon-reload

    echo "Restart the updated blentinel service ..."
    sudo systemctl restart blentinel-hub

    # Give systemd a moment to settle
    sleep 2

    if ! sudo systemctl --quiet is-active blentinel-hub; then
        echo "Hub failed to start. Showing status:"
        sudo systemctl status blentinel-hub --no-pager || true
        echo "Recent logs:"
        sudo journalctl -u blentinel-hub -n 20 --no-pager || true
        exit 1
    fi

else
    echo "First install ..."
    cd "$APP_DIR"
    chmod +x install_hub_service.sh
    ./install_hub_service.sh
fi

ls -la "$APP_DIR"

EOF

echo "Deploy finished."
