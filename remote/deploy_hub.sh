#!/bin/bash
set -e

BASE_DIR="/home/user/blentinel-builder/blentinel"
VPS="ubuntu@141.147.23.56"

cd "$BASE_DIR"

# Find newest publish zip
LATEST=$(ls -t publish/hub-*.zip 2>/dev/null | head -n1)
echo "Deploying release: $LATEST"

if [ -z "$LATEST" ]; then
    echo "Error: No published file matching 'publish/hub-*.zip' found"
    exit 1
fi

# Upload zipped publication file
scp "$LATEST" "$VPS:/tmp/blentinel-hub.zip"

# Install or update remotely
ssh "$VPS" << 'EOF'

set -e

rm -rf /tmp/blentinel-hub
mkdir -p /tmp/blentinel-hub

unzip -q /tmp/blentinel-hub.zip -d /tmp/blentinel-hub

# publish zip contains "app/"
APP_DIR=$(find /tmp/blentinel-hub -type d -name app | head -n 1)

if systemctl list-units --full -all | grep -Fq blentinel-hub.service; then

    echo "Stop the running blentinel service ..."
    sudo systemctl stop blentinel-hub || true

    echo "Updating existing installation ..."
    sudo rsync -av \
    --exclude "blentinel_hub.toml" \
    --exclude "hub_identity.key" \
    --exclude "hub_auth.token" \
    --exclude "blentinel.db*" \
    "$APP_DIR/" /opt/blentinel/hub/

    sudo chown -R ubuntu:ubuntu /opt/blentinel/hub

    echo "Reload systemd units..."
    sudo systemctl daemon-reload

    echo "Restart the updated blentinel service ..."
    sudo systemctl restart blentinel-hub

    sleep 2

    if ! sudo systemctl --quiet is-active blentinel-hub; then
        echo "Hub failed to start. Showing logs:"
        sudo journalctl -u blentinel-hub -n 20 --no-pager
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
