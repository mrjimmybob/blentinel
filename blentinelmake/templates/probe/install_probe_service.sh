#!/bin/bash
# Install or upgrade the Blentinel probe as a systemd service.
#
# Everything the probe owns lives in /opt/blentinel-probe: its identity
# (identity.key), enrollment, the checks its hub gave it (config.json), report
# buffer and log. An upgrade replaces ONLY the binary. Deleting identity.key
# would give the probe a new ID that has to be enrolled on the hub again.
# There is no config file to edit: checks are managed on the hub (D-49).
#
# Run from the unpacked package folder: sudo ./install_probe_service.sh
set -euo pipefail

INSTALL_DIR=/opt/blentinel-probe
SERVICE_USER=blentinel
SERVICE=blentinel-probe

cd "$(dirname "$0")"
if [ "$(id -u)" -ne 0 ]; then
    echo "Please run as root: sudo $0" >&2
    exit 1
fi

if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
fi

systemctl stop "$SERVICE" 2>/dev/null || true
install -d -m 0750 -o "$SERVICE_USER" -g "$SERVICE_USER" "$INSTALL_DIR"
install -m 0755 -o "$SERVICE_USER" -g "$SERVICE_USER" probe "$INSTALL_DIR/probe"

# First install: the probe isn't enrolled yet.
FIRST_INSTALL=0
if [ ! -f "$INSTALL_DIR/enrollment.json" ]; then
    FIRST_INSTALL=1
fi

install -m 0644 blentinel-probe.service /etc/systemd/system/"$SERVICE".service
systemctl daemon-reload
systemctl enable "$SERVICE"

if [ "$FIRST_INSTALL" = 1 ]; then
    echo "Blentinel probe installed in $INSTALL_DIR (not started yet)."
    echo
    echo "Next:"
    echo "  1. In the hub's web UI: Companies > the company > Add probe. Copy the code."
    echo "  2. sudo -u $SERVICE_USER $INSTALL_DIR/probe enroll <code>"
    echo "  3. sudo systemctl start $SERVICE"
    echo "  4. Add its checks on the hub; the probe picks them up with its next report."
else
    systemctl restart "$SERVICE"
    echo "Blentinel probe upgraded and restarted ($INSTALL_DIR; identity and enrollment kept)."
fi
echo "Logs: journalctl -u $SERVICE -f   (and $INSTALL_DIR/blentinel_probe.log)"
