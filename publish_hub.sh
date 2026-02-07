#!/usr/bin/env bash
set -e

PUBLISH_ROOT=publish/hub
OUT_DIR=$PUBLISH_ROOT/app
TARGET_DIR=target/release
BIN_NAME=hub

mkdir -p "$OUT_DIR"

echo "Publishing HUB (native Linux)..."

./build_hub.sh --release

cp "$TARGET_DIR/$BIN_NAME" "$OUT_DIR/"

cat > "$OUT_DIR/blentinel_hub.toml" <<EOF
[server]
host = "127.0.0.1"
port = 3000
db_path = "blentinel.db"
identity_key_path = "hub_identity.key"
probe_timeout_secs = 120

[[probes]]
name = "SERVER-1"
public_key = "PUT_PROBE_PUBLIC_KEY_HERE"
EOF

cat > "$OUT_DIR/blentinel-hub.service" <<EOF
[Unit]
Description=Blentinel Hub
After=network.target

[Service]
Type=simple
ExecStart=/opt/blentinel/hub/hub
Restart=always
RestartSec=5
User=blentinel
WorkingDirectory=/opt/blentinel/hub
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

cat > "$OUT_DIR/install_hub_service.sh" <<EOF
#!/usr/bin/env bash
set -e

sudo useradd -r -s /usr/sbin/nologin blentinel || true
sudo mkdir -p /opt/blentinel/hub
sudo cp * /opt/blentinel/hub
sudo chown -R blentinel:blentinel /opt/blentinel
sudo cp blentinel-hub.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable blentinel-hub
sudo systemctl start blentinel-hub
EOF

chmod +x "$OUT_DIR/install_hub_service.sh"

tar czf publish/hub.tar.gz -C publish hub

echo "Publish completed:"
echo "  publish/hub"
echo "  publish/hub.tar.gz"
