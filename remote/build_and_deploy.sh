#!/bin/bash
set -e

cd ~/blentinel-builder/blentinel

echo "Checking for updates..."
git fetch

LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/main)
REMOTE_IP=141.147.23.56

if [ "$LOCAL" = "$REMOTE" ]; then
    echo "No changes."
    exit 0
fi

echo "New commit found — pulling latest..."
git pull

echo "Building the build tool..."
chmod u+x ./build_blentinelmake.sh
./build_blentinelmake.sh

echo "Building..."
./target/release/blentinelmake hub publish

echo "Find latest published directory..."
LATEST=$(ls -dt publish/hub-* | head -n 1)

echo "Latest release: $LATEST"
echo "Uploading binary..."
scp -r "$LATEST/app" ubuntu@$REMOTE_IP:/tmp/blentinel-hub

ssh ubuntu@$REMOTE_IP << 'EOF'

if systemctl list-units --full -all | grep -Fq blentinel-hub.service; then
    echo "Updating existing install..."
    sudo cp -r /tmp/blentinel-hub/* /opt/blentinel/hub/
    sudo systemctl restart blentinel-hub
else
    echo "First install..."
    cd /tmp/blentinel-hub
    chmod +x install_hub_service.sh
    ./install_hub_service.sh
fi

EOF

echo "Restarting service..."
ssh ubuntu@$REMOTE_IP "sudo systemctl restart blentinel-hub"

echo "Deploy complete."