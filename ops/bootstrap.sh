#!/bin/sh
# The first step on a fresh lab-control (Debian 13), before Ansible can run:
#   sudo ~/blentinel/ops/bootstrap.sh
# from the admin account, in the checkout docs/ops.md says to clone. It
# installs Ansible and Git from Debian, makes the private inventory's folder,
# and the key lab-control reaches staging with. Running it again changes
# nothing that exists: it never replaces a key or an inventory.
# From then on the lab_control role keeps all of this (D-105).
set -eu

fail() { echo "bootstrap: $*" >&2; exit 1; }

admin=${SUDO_USER:-root}
if [ "$(id -u)" != 0 ] || [ "$admin" = root ]; then
  fail "run it with sudo, from the admin account"
fi
# shellcheck source=/dev/null
. /etc/os-release
if [ "$ID" != debian ] || [ "${VERSION_ID:-}" != 13 ]; then
  fail "lab-control is Debian 13; this is $PRETTY_NAME"
fi

here=$(cd "$(dirname "$0")" && pwd)
home=$(getent passwd "$admin" | cut -d: -f6)
config=/etc/blentinel-ops
inventory=$config/inventory
key=$home/.ssh/blentinel_staging

echo "Installing Ansible and Git from Debian"
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install --yes -qq ansible git >/dev/null

echo "Making the private inventory's folder, $inventory"
install -d -o "$admin" -g "$admin" -m 0700 "$config" "$inventory"
if [ ! -e "$inventory/hosts.yml" ]; then
  install -o "$admin" -g "$admin" -m 0600 "$here/inventory.example/hosts.yml" "$inventory/hosts.yml"
  new_inventory=yes
fi

if [ ! -e "$key" ]; then
  echo "Making the key lab-control reaches staging with, $key"
  install -d -o "$admin" -g "$admin" -m 0700 "$home/.ssh"
  runuser -u "$admin" -- ssh-keygen -q -t ed25519 -N '' -C "$admin@$(hostname)-to-staging" -f "$key"
fi

echo
echo "Done. Next (docs/ops.md, \"Set up lab-control\"):"
if [ "${new_inventory:-}" = yes ]; then
  echo "  - Write the inventory: $inventory/hosts.yml is a copy of the example;"
  echo "    or restore your copy of $config over it."
fi
echo "  - lab-control's key, for staging's base_admin_keys in the inventory:"
echo "      $(cat "$key.pub")"
echo "  - Let it into staging once: ssh-copy-id -i $key ops@<staging>"
