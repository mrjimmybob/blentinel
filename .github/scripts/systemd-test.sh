#!/bin/sh
# The install and rollback test on a real systemd (proposal 0013 §9, D-104).
#
#   sudo .github/scripts/systemd-test.sh <signed release> <signed broken release>
#
# Run on a throwaway machine (a GitHub runner): it installs services as root.
# The releases are a test build that trusts the committed test key
# (hub/end2end/release-key.json): never a release anyone could install.
#
# 1. hub.sh from the folder, as a user runs it: the signature checked with
#    openssl, the hub installed as a service, the probes added (D-57).
# 2. The first Owner, a company, a site and an enrollment code, by the API.
# 3. The probe the hub added, installed with the code; it reports.
# 4. `hub upgrade` to the broken release (newer, signed, passes its
#    self-test, then ruins the database and dies): the old program and the
#    old database must come back, and the probe keep reporting (D-75).
# 5. Both uninstalled, nothing left running.
set -eu

[ $# -eq 2 ] || {
    echo "usage: systemd-test.sh <signed release> <signed broken release>" >&2
    exit 2
}
[ "$(id -u)" -eq 0 ] || {
    echo "run it as root" >&2
    exit 2
}
release=$(realpath "$1")
broken=$(realpath "$2")
HUB=https://127.0.0.1:3443
PASSWORD=ci-only-password-$$
STATE=/var/lib/blentinel
work=$(mktemp -d)

step() { printf '\n== %s\n' "$*"; }
logs() {
    printf '\n== Logs\n'
    journalctl -u blentinel-hub --no-pager -n 60 || true
    journalctl -u blentinel-probe --no-pager -n 30 || true
}
die() {
    printf '\nFAIL: %s\n' "$*" >&2
    exit 1
}
trap 'status=$?; [ $status -eq 0 ] || logs; rm -rf "$work"; exit $status' EXIT

# The hub's API with a session (cookie jar) and the CSRF header.
api() {
    curl -fsSk -b "$work/jar" -c "$work/jar" -H 'x-blentinel: 1' \
        -H 'content-type: application/json' -X "$1" ${3:+--data "$3"} "$HUB$2"
}
# Retry a check once a second for up to $1 seconds.
wait_for() {
    what=$1 limit=$2
    shift 2
    n=0
    until "$@" >/dev/null 2>&1; do
        n=$((n + 1))
        [ $n -lt "$limit" ] || die "timed out after ${limit}s waiting for $what"
        sleep 1
    done
}
answers() { curl -fsk "$HUB/api/setup"; }
last_seen() { api GET /api/probes | jq -r '.[0].last_seen_at // empty'; }
reported() { [ -n "$(last_seen)" ]; }
reported_after() { [ "$(last_seen)" != "$1" ] && [ -n "$(last_seen)" ]; }

step "1. hub.sh from the release folder"
BLENTINEL_RELEASE_URL="file://$release" sh "$release/hub.sh"
systemctl is-active --quiet blentinel-hub || die "the hub service isn't running"
wait_for "the hub to answer" 60 answers
version=$(jq -r .version "$release/release.json")
/opt/blentinel/hub/hub --version | grep -q "$version" || die "the installed hub isn't $version"
[ -f "$STATE/releases/$version/manifest.json" ] || die "hub install didn't add the probes"

step "2. The first Owner, a company, a site and an enrollment code"
code=$(tr -d '[:space:]' <"$STATE/setup.code")
api POST /api/setup "{\"code\":\"$code\",\"username\":\"owner\",\"display_name\":\"CI\",\"password\":\"$PASSWORD\"}" >/dev/null
api POST /api/companies '{"id":"ci","name":"CI"}' >/dev/null
api POST /api/companies/ci/sites '{"id":"runner","name":"runner"}' >/dev/null
enroll=$(api POST /api/enrollment-codes \
    "{\"company_id\":\"ci\",\"site_id\":\"runner\",\"probe_name\":\"runner\",\"hub_url\":\"$HUB\"}" |
    jq -r .code)
[ -n "$enroll" ] || die "no enrollment code"

step "3. The probe the hub offers, installed with the code"
cp "$STATE/releases/$version/probe-x86_64-unknown-linux-musl" "$work/probe"
chmod 0755 "$work/probe"
"$work/probe" install --code "$enroll"
systemctl is-active --quiet blentinel-probe || die "the probe service isn't running"
wait_for "the probe's first report" 90 reported
echo "the probe reported at $(last_seen)"

step "4. hub upgrade to a broken release: the old hub and database must come back"
companies=$(api GET /api/companies | jq -c '[.[].id] | sort')
program=$(sha256sum /opt/blentinel/hub/hub | cut -d' ' -f1)
if /opt/blentinel/hub/hub upgrade --from "$broken" >"$work/upgrade.log" 2>&1; then
    cat "$work/upgrade.log"
    die "hub upgrade to the broken release says it succeeded"
fi
cat "$work/upgrade.log"
grep -q "the old one is back" "$work/upgrade.log" || die "hub upgrade didn't roll back"
[ "$(sha256sum /opt/blentinel/hub/hub | cut -d' ' -f1)" = "$program" ] || die "the old program isn't back"
[ -f /opt/blentinel/hub/hub.failed ] || die "the broken program wasn't kept aside"
systemctl is-active --quiet blentinel-hub || die "the hub service isn't running after the rollback"
wait_for "the hub to answer after the rollback" 60 answers
api POST /api/auth/login "{\"username\":\"owner\",\"password\":\"$PASSWORD\"}" >/dev/null ||
    die "the Owner can't log in: the database didn't come back"
[ "$(api GET /api/companies | jq -c '[.[].id] | sort')" = "$companies" ] || die "the companies changed"
seen=$(last_seen)
wait_for "a report after the rollback" 90 reported_after "$seen"
echo "rolled back; the probe reported again at $(last_seen)"

step "5. Uninstall"
/var/lib/blentinel-probe/probe uninstall --delete-data
/opt/blentinel/hub/hub uninstall --delete-data
! systemctl is-active --quiet blentinel-hub || die "the hub service still runs"
! systemctl is-active --quiet blentinel-probe || die "the probe service still runs"
[ ! -e /opt/blentinel/hub/hub ] || die "the hub program is still there"
echo
echo "PASS: installed, enrolled, reported, rolled back, uninstalled"
