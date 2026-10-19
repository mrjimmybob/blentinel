#!/usr/bin/env bash
# The ops playbooks' test (proposal 0013 phase 3, D-105), for CI and for a
# developer with Docker: run from the repository's root.
#
# Lints the playbooks, then builds two fresh machines in containers the way
# docs/ops.md builds the real ones: lab-control (Debian 13) is set up with
# bootstrap.sh, and the admin's PC logs into it over SSH and runs site.yml
# there, which configures lab-control itself and staging (Ubuntu 26.04), with
# sudo's password:
#   0. a run in check mode must work on them (the preview docs/ops.md asks for);
#   1. the first run builds them;
#   2. the second run must change nothing (idempotent);
#   3. a run in check mode must change nothing either (a trustworthy drift check);
#   4. verify.yml checks the result.
set -euo pipefail
export MSYS_NO_PATHCONV=1 # Git Bash on Windows: leave container paths alone

net=ops-ci
targets=(ops-lab-control ops-staging)
all=("${targets[@]}" ops-pc)

cleanup() {
  docker rm --force "${all[@]}" >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

echo "::group::Images"
docker build --quiet --tag ops-ci-debian --build-arg BASE=debian:13 --file ops/ci/Dockerfile.target ops/ci
docker build --quiet --tag ops-ci-ubuntu --build-arg BASE=ubuntu:26.04 --file ops/ci/Dockerfile.target ops/ci
docker build --quiet --tag ops-ci-pc --file ops/ci/Dockerfile.pc ops/ci
echo "::endgroup::"

docker network create --subnet 198.51.100.0/24 --gateway 198.51.100.1 "$net" >/dev/null
systemd_flags=(--detach --privileged --cgroupns=host --volume /sys/fs/cgroup:/sys/fs/cgroup:rw
  --tmpfs /run --tmpfs /run/lock --network "$net")
docker run "${systemd_flags[@]}" --name ops-lab-control --hostname lab-control --ip 198.51.100.28 ops-ci-debian >/dev/null
docker run "${systemd_flags[@]}" --name ops-staging --hostname staging --ip 198.51.100.29 ops-ci-ubuntu >/dev/null
docker run --detach --name ops-pc --network "$net" --ip 198.51.100.9 ops-ci-pc sleep infinity >/dev/null

# Copies, not mounts: Ansible ignores an ansible.cfg in a world-writable
# folder, which is what a Windows checkout looks like from a container. On
# lab-control the copy stands for the clone docs/ops.md makes, at this commit.
docker exec ops-pc mkdir -p /work
docker cp ops ops-pc:/work/ops
docker exec ops-pc chmod -R go-w /work/ops
docker exec ops-lab-control install -d -o ops -g ops /home/ops/blentinel
docker cp ops ops-lab-control:/home/ops/blentinel/ops
docker exec ops-lab-control sh -c 'chown -R ops:ops /home/ops/blentinel && chmod -R go-w /home/ops/blentinel'

# The PC's key and sudo's password exist only for this run.
docker exec ops-pc ssh-keygen -q -t ed25519 -N '' -C ops-ci-pc -f /root/.ssh/id_ed25519
pc_key=$(docker exec ops-pc cat /root/.ssh/id_ed25519.pub)
password=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
for t in "${targets[@]}"; do
  docker exec "$t" sh -c "echo 'ops:$password' | chpasswd \
    && install -d -m 0700 -o ops -g ops /home/ops/.ssh \
    && echo '$pc_key' > /home/ops/.ssh/authorized_keys \
    && chown ops:ops /home/ops/.ssh/authorized_keys \
    && chmod 0664 /home/ops/.ssh/authorized_keys" # group-writable, as found on a real machine
done

# Trusts <address>'s host key for <user> in <container>, as a first ssh does,
# once its sshd answers.
known_host() {
  local container=$1 user=$2 ip=$3
  for _ in $(seq 60); do
    if docker exec --user "$user" "$container" sh -c \
      "ssh-keyscan -t ed25519 $ip 2>/dev/null >> ~/.ssh/known_hosts && grep -q '^$ip ' ~/.ssh/known_hosts"; then
      return
    fi
    sleep 1
  done
  echo "::error::$ip's sshd didn't answer."
  return 1
}

echo "::group::Lint"
docker exec --workdir /work/ops ops-pc sh -c 'ansible --version && yamllint --strict . && ansible-lint --offline'
docker exec --workdir /work/ops ops-pc shellcheck bootstrap.sh ci/converge.sh
echo "::endgroup::"

echo "::group::Bootstrap lab-control (docs/ops.md, set up)"
bootstrap() {
  docker exec --user ops ops-lab-control sh -c \
    "echo '$password' | sudo -S -p '' sh /home/ops/blentinel/ops/bootstrap.sh"
}
bootstrap
lab_key=$(docker exec ops-lab-control cat /home/ops/.ssh/blentinel_staging.pub)
bootstrap >/dev/null
[ "$(docker exec ops-lab-control cat /home/ops/.ssh/blentinel_staging.pub)" = "$lab_key" ] \
  || { echo "::error::A second bootstrap.sh replaced lab-control's key."; exit 1; }
# Drift for lab_control to correct: a folder anyone can list.
docker exec ops-lab-control chmod 0755 /etc/blentinel-ops
# The owner's steps after it: write the inventory, and let lab-control's key
# into staging once (ssh-copy-id, which also trusts staging's host key).
docker exec --user ops ops-lab-control \
  cp /home/ops/blentinel/ops/ci/inventory.yml /etc/blentinel-ops/inventory/hosts.yml
docker exec ops-staging sh -c "echo '$lab_key' >> /home/ops/.ssh/authorized_keys"
known_host ops-lab-control ops 198.51.100.29
known_host ops-pc root 198.51.100.28
# What the owner types instead: sudo's password, and the keys in the inventory.
docker exec --user ops ops-lab-control sh -c "umask 077 && echo '$password' > /home/ops/become \
  && printf 'ci_pc_key: %s\nci_lab_control_key: %s\n' '$pc_key' '$lab_key' > /home/ops/keys.yml"
echo "::endgroup::"

# From the PC: log into lab-control and run a playbook there.
play() {
  docker exec ops-pc ssh ops@198.51.100.28 "cd blentinel/ops && ANSIBLE_FORCE_COLOR=1 ansible-playbook \
    --become-password-file /home/ops/become --extra-vars @/home/ops/keys.yml $*"
}
# Every host's PLAY RECAP line must say changed=0.
no_changes() {
  local recap
  recap=$(sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -E '^[a-z-]+ +: ok=')
  echo "$recap"
  [ "$(echo "$recap" | wc -l)" -eq 2 ] && ! echo "$recap" | grep -qE 'changed=[1-9]|failed=[1-9]|unreachable=[1-9]'
}

echo "::group::Syntax"
play --syntax-check site.yml verify.yml
echo "::endgroup::"

log=$(mktemp)

echo "::group::0. Check mode on the fresh machines: the preview must work"
play --check --diff site.yml
echo "::endgroup::"

echo "::group::1. First run: builds the machines"
play site.yml
echo "::endgroup::"

echo "::group::2. Second run: must change nothing"
play site.yml | tee "$log"
echo "::endgroup::"
no_changes "$log" || { echo "::error::The second run changed something: the playbooks aren't idempotent."; exit 1; }

echo "::group::3. Check mode: must change nothing"
play --check --diff site.yml | tee "$log"
echo "::endgroup::"
no_changes "$log" || { echo "::error::Check mode reports changes on machines just applied."; exit 1; }

echo "::group::4. Verify"
play verify.yml
echo "::endgroup::"

echo "bootstrap.sh and the playbooks build both machines, change nothing on a second run or in check mode, and verify."
