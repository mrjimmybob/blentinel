#!/usr/bin/env bash
# The ops playbooks' test (proposal 0013 phase 3, D-105), for CI and for a
# developer with Docker: run from the repository's root.
#
# Lints the playbooks, then applies site.yml to two fresh machines in
# containers (lab-control on Debian 13, staging on Ubuntu 26.04) from a Debian
# 13 controller, over SSH with sudo's password, as on the real machines:
#   0. a run in check mode must work on them (the preview docs/ops.md asks for);
#   1. the first run builds them;
#   2. the second run must change nothing (idempotent);
#   3. a run in check mode must change nothing either (a trustworthy drift check);
#   4. verify.yml checks the result.
set -euo pipefail
export MSYS_NO_PATHCONV=1 # Git Bash on Windows: leave container paths alone

net=ops-ci
targets=(ops-lab-control ops-staging)
all=("${targets[@]}" ops-controller)

cleanup() {
  docker rm --force "${all[@]}" >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

echo "::group::Images"
docker build --quiet --tag ops-ci-debian --build-arg BASE=debian:13 --file ops/ci/Dockerfile.target ops/ci
docker build --quiet --tag ops-ci-ubuntu --build-arg BASE=ubuntu:26.04 --file ops/ci/Dockerfile.target ops/ci
docker build --quiet --tag ops-ci-controller --file ops/ci/Dockerfile.controller ops/ci
echo "::endgroup::"

docker network create --subnet 198.51.100.0/24 --gateway 198.51.100.1 "$net" >/dev/null
systemd_flags=(--detach --privileged --cgroupns=host --volume /sys/fs/cgroup:/sys/fs/cgroup:rw
  --tmpfs /run --tmpfs /run/lock --network "$net")
docker run "${systemd_flags[@]}" --name ops-lab-control --hostname lab-control --ip 198.51.100.28 ops-ci-debian >/dev/null
docker run "${systemd_flags[@]}" --name ops-staging --hostname staging --ip 198.51.100.29 ops-ci-ubuntu >/dev/null
docker run --detach --name ops-controller --network "$net" --ip 198.51.100.9 ops-ci-controller sleep infinity >/dev/null

# A copy, not a mount: Ansible ignores an ansible.cfg in a world-writable
# folder, which is what a Windows checkout looks like from a container.
docker exec ops-controller mkdir -p /work
docker cp ops ops-controller:/work/ops
docker exec ops-controller chmod -R go-w /work/ops

# The admin key and sudo's password exist only for this run.
docker exec ops-controller ssh-keygen -q -t ed25519 -N '' -C ops-ci -f /root/.ssh/id_ed25519
pub=$(docker exec ops-controller cat /root/.ssh/id_ed25519.pub)
password=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
for t in "${targets[@]}"; do
  docker exec "$t" sh -c "echo 'ops:$password' | chpasswd \
    && install -d -m 0700 -o ops -g ops /home/ops/.ssh \
    && echo '$pub' > /home/ops/.ssh/authorized_keys \
    && chown ops:ops /home/ops/.ssh/authorized_keys \
    && chmod 0664 /home/ops/.ssh/authorized_keys" # group-writable, as found on a real machine
done
docker exec ops-controller sh -c "umask 077 && echo '$password' > /work/become \
  && printf 'base_admin_keys:\n  - %s\n' '$pub' > /work/keys.yml"

for ip in 198.51.100.28 198.51.100.29; do
  for _ in $(seq 60); do
    if docker exec ops-controller sh -c "ssh-keyscan -t ed25519 $ip 2>/dev/null >> /root/.ssh/known_hosts" \
      && docker exec ops-controller grep -q "^$ip " /root/.ssh/known_hosts; then
      break
    fi
    sleep 1
  done
done

in_controller() {
  docker exec --workdir /work/ops --env ANSIBLE_FORCE_COLOR=1 ops-controller "$@"
}
play() {
  in_controller ansible-playbook --inventory ci/inventory.yml --become-password-file /work/become \
    --extra-vars @/work/keys.yml "$@"
}
# Every host's PLAY RECAP line must say changed=0.
no_changes() {
  local recap
  recap=$(sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -E '^[a-z-]+ +: ok=')
  echo "$recap"
  [ "$(echo "$recap" | wc -l)" -eq 2 ] && ! echo "$recap" | grep -qE 'changed=[1-9]|failed=[1-9]|unreachable=[1-9]'
}

echo "::group::Lint"
in_controller ansible --version
in_controller yamllint --strict .
in_controller ansible-lint --offline
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

echo "The playbooks build both machines, change nothing on a second run or in check mode, and verify."
