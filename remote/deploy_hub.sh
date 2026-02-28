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

    echo "Start the updated blentinel service ..."
    sudo systemctl start blentinel-hub
    sleep 2
    sudo systemctl --quiet is-active blentinel-hub || {
        echo "Hub failed to start"
        sudo systemctl status blentinel-hub --no-pager
        exit 1
    }
else
    echo "First install ..."
    cd "$APP_DIR"
    chmod +x install_hub_service.sh
    ./install_hub_service.sh
fi
ls -la "$APP_DIR"
EOF

echo "Deploy finished."
