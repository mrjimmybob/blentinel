#!/bin/sh
# Makes the deliberately broken release of the rollback test (proposal 0013
# §9, D-104) from an assembled, unsigned release:
#
#   .github/scripts/broken-release.sh <assembled release> <out folder>
#
# A newer version (hub upgrade takes only newer ones) whose hub passes its
# self-test, so `hub upgrade` switches to it, and then, started as the
# service, ruins the database, as a failed migration might, and exits.
# `hub upgrade` must then put back the old program and the old database.
# Sign it with sign-release.sh like any release: it fails only once running.
set -eu

[ $# -eq 2 ] || {
    echo "usage: broken-release.sh <assembled release> <out folder>" >&2
    exit 2
}
rm -rf "$2"
cp -R "$1" "$2"
cd "$2"

version=$(jq -r .version release.json)
newer=$(echo "$version" | awk -F. '{ printf "%d.%d.%d", $1, $2, $3 + 1 }')
jq --arg v "$newer" '.version = $v' release.json >release.json.new
mv release.json.new release.json

for hub in hub-*; do
    cat >"$hub" <<EOF
#!/bin/sh
# The broken hub of the rollback test: it passes the self-test, then can't run.
if [ "\${1:-}" = self-test ]; then
    echo "Blentinel hub $newer"
    exit 0
fi
printf 'not a database any more' >/var/lib/blentinel/blentinel.db
echo "broken on purpose: the database is ruined" >&2
exit 1
EOF
    chmod 0755 "$hub"
done
echo "broken release $newer in $(pwd)"
