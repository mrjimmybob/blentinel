#!/bin/sh
# The deployment agent's end-to-end test on a real systemd (proposal 0013
# §10.4 D, D-107):
#
#   sudo .github/scripts/deploy-test.sh <signed release> <signed broken release>
#
# Run on a throwaway machine (a GitHub runner): it installs services and
# users as root. One machine plays both: lab-control (the agent, as the user
# deployer, started by its systemd unit) and staging (the forced command,
# reached over a real sshd on 127.0.0.1). GitHub is fake-github.py. The
# releases are the test build, signed with the browser tests' public test
# key, which stands in for the staging key on both sides.
#
# The scripts and units are ops/roles/deploy_*'s own files; what Ansible
# renders from the inventory (the settings, sudoers, authorized_keys) is
# written here the same way.
#
#   1. A first build: GitHub fails once while it runs, then fails the success
#      report once. Each run carries on from where the last stopped; in the
#      end the hub is installed with hub.sh, on the version, and GitHub has
#      one Deployment, for the run's commit: in_progress, then success. The
#      artifact was downloaded once, and without the token.
#   2. Nothing new: the next run does nothing.
#   3. The forced command: anything but deploy and status is refused, and a
#      release not signed with a pinned key is refused by staging itself.
#   4. A broken build (signed, newer, its hub dies): hub upgrade puts the old
#      hub back, and GitHub gets a failure saying so. Staging runs the first
#      build still.
#   5. A tampered build (release.json changed after signing): refused at
#      verification, before staging is asked anything. GitHub gets a failure.
set -eu
# Files below get the modes the roles give them, whatever the runner's umask:
# sshd refuses an authorized_keys others can write.
umask 022

[ $# -eq 2 ] || {
    echo "usage: deploy-test.sh <signed release> <signed broken release>" >&2
    exit 2
}
[ "$(id -u)" -eq 0 ] || {
    echo "run it as root" >&2
    exit 2
}
release=$(realpath "$1")
broken=$(realpath "$2")
repo=$(cd "$(dirname "$0")/../.." && pwd)
roles=$repo/ops/roles
key=$(jq -r .public "$repo/hub/end2end/release-key.json")
TOKEN=ci_test_token
PORT=8765
gh=$(mktemp -d)
chmod 0755 "$gh"
work=$(mktemp -d)

step() { printf '\n== %s\n' "$*"; }
die() {
    printf '\nFAIL: %s\n' "$*" >&2
    exit 1
}
logs() {
    printf '\n== Logs\n'
    journalctl --all -o cat -u blentinel-deploy-agent --no-pager -n 80 || true
    journalctl -u blentinel-hub --no-pager -n 40 || true
    cat "$gh/deployments.json" 2>/dev/null || true
}
trap 'status=$?; [ $status -eq 0 ] || logs; kill "${fake:-0}" 2>/dev/null; rm -rf "$work"; exit $status' EXIT

step "Staging's half: the account, its forced command, sudo, sshd"
command -v sshd >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -qq openssh-server >/dev/null
for tool in jq python3 openssl curl flock; do command -v "$tool" >/dev/null || die "needs $tool"; done
useradd --system --home-dir /home/blentinel-deploy --no-create-home --shell /bin/sh blentinel-deploy
usermod -p '*' blentinel-deploy
install -d -m 0755 /home/blentinel-deploy /home/blentinel-deploy/.ssh /usr/local/lib/blentinel-deploy /etc/blentinel-deploy
install -d -m 0700 /var/lib/blentinel-deploy
for f in ssh-entry deploy status; do install -m 0755 "$roles/deploy_target/files/$f" /usr/local/lib/blentinel-deploy/; done
install -m 0644 "$repo/.github/scripts/verify-release.sh" /usr/local/lib/blentinel-deploy/
printf 'STAGING_KEYS=%s\nSUDO=/usr/bin/sudo\nHTTPS_PORT=3443\nMAX_RELEASE_BYTES=1073741824\nKEEP_RELEASES=3\n' "$key" \
    >/etc/blentinel-deploy/target.conf
echo 'blentinel-deploy ALL=(root) NOPASSWD: /usr/local/lib/blentinel-deploy/deploy "", /usr/local/lib/blentinel-deploy/status ""' \
    >"$work/sudoers"
visudo -cf "$work/sudoers" >/dev/null
install -m 0440 "$work/sudoers" /etc/sudoers.d/blentinel-deploy
systemctl start ssh || systemctl start sshd

step "Lab-control's half: the user deployer, its key, the agent and its units"
useradd --system --home-dir /var/lib/blentinel-deploy-agent --create-home --shell /usr/sbin/nologin deployer
chmod 0700 /var/lib/blentinel-deploy-agent
install -d -o deployer -g deployer -m 0700 /var/lib/blentinel-deploy-agent/.ssh
runuser -u deployer -- ssh-keygen -q -t ed25519 -N '' -f /var/lib/blentinel-deploy-agent/.ssh/id_ed25519
printf 'restrict,from="127.0.0.1",command="/usr/local/lib/blentinel-deploy/ssh-entry" %s\n' \
    "$(cat /var/lib/blentinel-deploy-agent/.ssh/id_ed25519.pub)" >/home/blentinel-deploy/.ssh/authorized_keys
chmod 0644 /home/blentinel-deploy/.ssh/authorized_keys
install -d -m 0755 /usr/local/lib/blentinel-deploy-agent
install -m 0755 "$roles/deploy_agent/files/agent" /usr/local/lib/blentinel-deploy-agent/
install -m 0644 "$roles/deploy_agent/files/unzip-release.py" "$repo/.github/scripts/verify-release.sh" \
    /usr/local/lib/blentinel-deploy-agent/
install -d -g deployer -m 0750 /etc/blentinel-deploy-agent
cat >/etc/blentinel-deploy-agent/agent.conf <<EOF
REPOSITORY=test/blentinel
WORKFLOW=build.yml
API_URL=http://127.0.0.1:$PORT
TOKEN_FILE=/etc/blentinel-deploy-agent/github-token
STAGING_KEYS=$key
STAGING_HOST=127.0.0.1
STAGING_USER=blentinel-deploy
SSH_KEY=/var/lib/blentinel-deploy-agent/.ssh/id_ed25519
KNOWN_HOSTS=/etc/blentinel-deploy-agent/known_hosts
HUB_URL=https://127.0.0.1:3443
SMOKE_SECONDS=120
MAX_ATTEMPTS=3
MAX_ARTIFACT_BYTES=1073741824
EOF
echo "$TOKEN" >/etc/blentinel-deploy-agent/github-token
echo "127.0.0.1 $(cut -d' ' -f1,2 /etc/ssh/ssh_host_ed25519_key.pub)" >/etc/blentinel-deploy-agent/known_hosts
chgrp deployer /etc/blentinel-deploy-agent/agent.conf /etc/blentinel-deploy-agent/github-token
chmod 0640 /etc/blentinel-deploy-agent/agent.conf
chmod 0440 /etc/blentinel-deploy-agent/github-token
sed -e 's/{{ ansible_managed }}/written by deploy-test.sh/' -e 's/{{ deploy_agent_user }}/deployer/g' \
    -e 's#{{ deploy_agent_home }}#/var/lib/blentinel-deploy-agent#g' \
    "$roles/deploy_agent/templates/blentinel-deploy-agent.service.j2" >/etc/systemd/system/blentinel-deploy-agent.service
! grep -q '{{' /etc/systemd/system/blentinel-deploy-agent.service || die "the unit has a value the test doesn't fill in"
systemctl daemon-reload

step "The agent's key reaches the forced command"
# With the agent's own SSH options, as its user.
as_agent() { # <command> [< stdin]
    runuser -u deployer -- ssh -F /dev/null -i /var/lib/blentinel-deploy-agent/.ssh/id_ed25519 -o BatchMode=yes \
        -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/etc/blentinel-deploy-agent/known_hosts \
        -o GlobalKnownHostsFile=/dev/null blentinel-deploy@127.0.0.1 "$1"
}
as_agent status </dev/null >"$work/out" 2>&1 || {
    cat "$work/out"
    journalctl -u ssh -u sshd --no-pager -n 30 || true
    die "staging's forced command didn't answer status"
}
cat "$work/out"
grep -q '^version=none$' "$work/out" || die "status on a staging with no hub doesn't say version=none"

step "A fake GitHub"
zip_release() { # <folder> <zip>
    python3 -c 'import os, sys, zipfile
with zipfile.ZipFile(sys.argv[2], "w", zipfile.ZIP_DEFLATED) as z:
    for name in sorted(os.listdir(sys.argv[1])):
        z.write(os.path.join(sys.argv[1], name), name)' "$1" "$2"
}
v1=$(jq -r .version "$release/release.json")
commit=$(jq -r .commit "$release/release.json")
zip_release "$release" "$gh/v1.zip"
jq -n --arg token "$TOKEN" --arg sha "$commit" --arg v "$v1" --arg file "$gh/v1.zip" '{
    token: $token, repository: "test/blentinel",
    runs: [{id: 101, run_number: 1, head_sha: $sha}],
    artifacts: {"101": [{id: 501, name: ("staging-" + $v), file: $file}]},
    faults: {create: 1, success: 1}}' >"$gh/state.json"
python3 -I "$repo/.github/scripts/fake-github.py" "$gh" "$PORT" &
fake=$!
n=0
until curl -fs -o /dev/null -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:$PORT/repos/test/blentinel/deployments?sha=x&environment=staging"; do
    n=$((n + 1))
    [ $n -lt 30 ] || die "the fake GitHub didn't start"
    sleep 1
done

# The agent, once, as its timer starts it; its exit status.
agent() {
    systemctl start blentinel-deploy-agent.service >/dev/null 2>&1 || true
    systemctl show --property ExecMainStatus --value blentinel-deploy-agent.service
}
state() { cat "/var/lib/blentinel-deploy-agent/builds/$1/state" 2>/dev/null || echo none; }
deployments() { jq length "$gh/deployments.json" 2>/dev/null || echo 0; }
statuses() { jq -c --arg sha "$2" '[.[] | select(.sha == $sha and .payload.version == "'"$1"'") | .statuses[].state]' "$gh/deployments.json"; }
downloads() { grep -c '"path": "/blob/' "$gh/requests.jsonl" || true; }
hub_sha() { sha256sum "/proc/$(systemctl show --property MainPID --value blentinel-hub)/exe" | cut -d' ' -f1; }

step "1. A first build, with GitHub failing on the way: each run carries on"
[ "$(agent)" = 75 ] || die "a failed Deployment creation didn't stop the run as passing"
[ "$(state "$v1")" = deploying ] || die "state after the first run: $(state "$v1"), not deploying"
[ "$(deployments)" = 0 ] || die "a Deployment exists, though GitHub failed to make it"
[ "$(agent)" = 75 ] || die "a failed success report didn't stop the run as passing"
[ "$(state "$v1")" = succeeded ] || die "state after the second run: $(state "$v1"), not succeeded"
[ ! -e /var/lib/blentinel-deploy-agent/current ] || die "staging's version was recorded before GitHub knew"
[ "$(agent)" = 0 ] || die "the third run failed"
[ "$(state "$v1")" = reported ] || die "state after the third run: $(state "$v1")"
[ "$(/opt/blentinel/hub/hub --version)" = "$v1" ] || die "the installed hub isn't $v1"
systemctl is-active --quiet blentinel-hub || die "the hub isn't running"
grep -q '^staging = true' /etc/blentinel/blentinel_hub.toml || die "the hub wasn't installed as a staging hub"
[ "$(statuses "$v1" "$commit")" = '["in_progress","success"]' ] || die "v1's statuses: $(statuses "$v1" "$commit")"
[ "$(deployments)" = 1 ] || die "$(deployments) Deployments, not 1"
[ "$(jq -r '.[0].sha' "$gh/deployments.json")" = "$commit" ] || die "the Deployment isn't for the run's commit"
[ "$(jq -r '.[0].payload.files["hub-x86_64-unknown-linux-musl"]' "$gh/deployments.json")" = \
    "$(cd "$release" && sha256sum hub-x86_64-unknown-linux-musl | cut -d' ' -f1)" ] ||
    die "the record doesn't carry the hub's SHA-256"
[ "$(downloads)" = 1 ] || die "the artifact was downloaded $(downloads) times, not once"
! grep '"path": "/blob/' "$gh/requests.jsonl" | grep -q '"authorized": true' ||
    die "the token was sent to the artifact's storage"
[ "$(cat /var/lib/blentinel-deploy-agent/current)" = "$v1" ] || die "current isn't $v1"
[ "$(jq -r .result /var/lib/blentinel-deploy-agent/deployments.log)" = success ] || die "no success line in deployments.log"
running=$(hub_sha)
[ "$running" = "$(cd "$release" && sha256sum hub-x86_64-unknown-linux-musl | cut -d' ' -f1)" ] ||
    die "the running hub isn't the release's"
echo "deployed $v1 ($commit)"

step "2. Nothing new: nothing done"
[ "$(agent)" = 0 ] || die "a run with nothing new failed"
if [ "$(deployments)" != 1 ] || [ "$(downloads)" != 1 ]; then
    die "a run with nothing new did something"
fi

step "3. The forced command refuses what it isn't for"
code=0
as_agent id </dev/null >"$work/out" 2>&1 || code=$?
if [ "$code" != 64 ] || grep -q uid= "$work/out"; then
    die "the forced command ran 'id' ($code): $(cat "$work/out")"
fi
cp -R "$release" "$work/unpinned"
jq '.version = "9.9.9"' "$release/release.json" >"$work/unpinned/release.json"
code=0
(cd "$work/unpinned" && tar -cf - -- *) | as_agent deploy >"$work/out" 2>&1 || code=$?
[ "$code" = 65 ] || die "staging took a release whose signature doesn't match ($code): $(cat "$work/out")"
[ ! -e /var/lib/blentinel-deploy/releases/9.9.9 ] || die "staging kept a release it refused"
[ "$(hub_sha)" = "$running" ] || die "a refused release changed the running hub"
if printf 'x' | as_agent deploy >"$work/out" 2>&1; then
    die "staging took a non-tar"
fi

step "4. A broken build: hub upgrade puts the old hub back, and GitHub hears so"
v2=$(jq -r .version "$broken/release.json")
zip_release "$broken" "$gh/v2.zip"
jq --arg sha "$commit" --arg v "$v2" --arg file "$gh/v2.zip" '.runs = [{id: 102, run_number: 2, head_sha: $sha}] + .runs
    | .artifacts["102"] = [{id: 502, name: ("staging-" + $v), file: $file}]' "$gh/state.json" >"$work/s" && mv "$work/s" "$gh/state.json"
[ "$(agent)" = 1 ] || die "a rolled-back deployment didn't fail the run"
[ "$(state "$v2")" = reported ] || die "the broken build's state: $(state "$v2")"
[ "$(head -n 1 "/var/lib/blentinel-deploy-agent/builds/$v2/reason")" = rolled-back ] || die "the reason isn't rolled-back"
[ "$(statuses "$v2" "$commit")" = '["in_progress","failure"]' ] || die "the broken build's statuses: $(statuses "$v2" "$commit")"
jq -r --arg v "$v2" '.[] | select(.payload.version == $v) | .statuses[-1].description' "$gh/deployments.json" |
    grep -q "put the old hub back" || die "the failure doesn't say the old hub is back"
[ "$(hub_sha)" = "$running" ] || die "the hub running isn't the first build's"
[ "$(/opt/blentinel/hub/hub --version)" = "$v1" ] || die "the installed hub isn't $v1 any more"
[ "$(cat /var/lib/blentinel-deploy-agent/current)" = "$v1" ] || die "current changed to a failed build"
[ "$(agent)" = 0 ] || die "a failed build was tried again"

step "5. A tampered build: refused at verification, staging never asked"
cp -R "$broken" "$work/tampered"
jq '.version = "0.1.2"' "$broken/release.json" >"$work/tampered/release.json"
zip_release "$work/tampered" "$gh/v3.zip"
jq --arg sha "$commit" --arg file "$gh/v3.zip" '.runs = [{id: 103, run_number: 3, head_sha: $sha}] + .runs
    | .artifacts["103"] = [{id: 503, name: "staging-0.1.2", file: $file}]' "$gh/state.json" >"$work/s" && mv "$work/s" "$gh/state.json"
releases=$(ls /var/lib/blentinel-deploy/releases)
[ "$(agent)" = 1 ] || die "a tampered build didn't fail the run"
[ "$(head -n 1 /var/lib/blentinel-deploy-agent/builds/0.1.2/reason)" = verification ] || die "the reason isn't verification"
[ "$(statuses 0.1.2 "$commit")" = '["failure"]' ] || die "the tampered build's statuses: $(statuses 0.1.2 "$commit")"
[ "$(ls /var/lib/blentinel-deploy/releases)" = "$releases" ] || die "staging was sent a tampered build"
[ "$(hub_sha)" = "$running" ] || die "a tampered build changed the running hub"
[ "$(wc -l </var/lib/blentinel-deploy-agent/deployments.log)" = 3 ] || die "deployments.log hasn't 3 lines"

step "6. Staging rebuilt without a hub: the newest good build goes back on, once"
/opt/blentinel/hub/hub uninstall --delete-data
rm -f /var/lib/blentinel-deploy/approved
jq '.runs = [.runs[] | select(.id == 101)]' "$gh/state.json" >"$work/s" && mv "$work/s" "$gh/state.json"
[ "$(agent)" = 0 ] || die "the redeployment failed"
[ "$(/opt/blentinel/hub/hub --version)" = "$v1" ] || die "the hub didn't come back on $v1"
[ "$(statuses "$v1" "$commit")" = '["in_progress","success","in_progress","success"]' ] ||
    die "v1's statuses after the redeployment: $(statuses "$v1" "$commit")"
[ "$(deployments)" = 4 ] || die "$(deployments) Deployments, not 4: the redeployment reused an old one"
[ "$(agent)" = 0 ] || die "the run after the redeployment failed"
[ "$(deployments)" = 4 ] || die "the redeployment was done twice"

step "Uninstall"
/opt/blentinel/hub/hub uninstall --delete-data
echo
echo "PASS: deployed with interruptions, idempotent, forced command enforced, rolled back and reported, tampered build refused, redeployed once to a rebuilt staging"
