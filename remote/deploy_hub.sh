#!/bin/bash
set -e

BASE_DIR="/home/user/blentinel-builder/blentinel"
VPS="ubuntu@141.147.23.56"

cd "$BASE_DIR"

# Find newest publish folder
LATEST=$(ls -t hub-*.zip 2>/dev/null | head -n1)
echo "Deploying release: $LATEST"

if [ -z "$LATEST" ]; then
    echo "Error: No published file matching 'hub-*.zip' found"
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
    echo "Updating existing install ..."
    sudo cp -r "$APP_DIR/"* /opt/blentinel/hub/
    sudo systemctl restart blentinel-hub
else
    echo "First install ..."
    cd "$APP_DIR"
    chmod +x install_hub_service.sh
    ./install_hub_service.sh
fi
ls -la "$APP_DIR"
EOF

echo "Deploy finished."