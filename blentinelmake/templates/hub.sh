#!/bin/sh
# Blentinel hub installer (ROADMAP M2-10, DECISIONS D-57).
#
#   curl -fsSL https://github.com/mrjimmybob/blentinel/releases/latest/download/hub.sh | sudo sh
#
# Options after `-s --` go to `hub install`, e.g.:
#   curl -fsSL …/hub.sh | sudo sh -s -- --https-port 443
#
# Environment:
#   BLENTINEL_VERSION=0.2.0      install that release instead of the latest
#   BLENTINEL_RELEASE_URL=<url>  take the release files from that folder instead
#   BLENTINEL_STAGING=1          a staging hub: also trust the staging key(s)
#                                below, and install it with --staging (D-106)
#
# What it does: downloads SHA256SUMS and checks its signature against the
# release key(s) below, then downloads the hub for this machine and the
# signed probe release, checks each against its signed SHA-256, and runs
# `hub install` (which also adds the probes, so Add probe can offer them).
# If any check fails, nothing is installed. `blentinelmake release hub`
# writes this file with the release keys filled in.
set -eu

RELEASES="https://github.com/mrjimmybob/blentinel/releases"

# The Ed25519 release public keys this script trusts (PEM).
RELEASE_KEYS='@RELEASE_KEYS_PEM@'
# The staging public keys (PEM): trusted only with BLENTINEL_STAGING=1, for a
# staging hub. Every other install ignores them (0013 §6.2).
STAGING_KEYS='@STAGING_KEYS_PEM@'

say() { printf 'blentinel: %s\n' "$*" >&2; }
die() {
    say "$*"
    exit 1
}

[ "$(uname -s)" = Linux ] || die "this script installs the hub on Linux; on Windows run: hub.exe install"
[ "$(id -u)" -eq 0 ] || die "run it as root: curl -fsSL …/hub.sh | sudo sh"

case "$(uname -m)" in
x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
*) die "there is no hub build for $(uname -m) (x86_64 and aarch64 only)" ;;
esac

for tool in openssl sha256sum tar mktemp awk; do
    command -v "$tool" >/dev/null 2>&1 || die "needs $tool (e.g. apt install $tool)"
done
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "needs curl or wget"
fi

if [ -n "${BLENTINEL_RELEASE_URL:-}" ]; then
    base=${BLENTINEL_RELEASE_URL%/}
elif [ -n "${BLENTINEL_VERSION:-}" ]; then
    base="$RELEASES/download/v${BLENTINEL_VERSION#v}"
else
    base="$RELEASES/latest/download"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT TERM
cd "$work"

get() { fetch "$base/$1" "$1" || die "can't download $base/$1"; }

trusted=$RELEASE_KEYS
staging=
if [ "${BLENTINEL_STAGING:-}" = 1 ]; then
    [ -n "$STAGING_KEYS" ] || die "BLENTINEL_STAGING=1, but this script trusts no staging key"
    trusted="$RELEASE_KEYS
$STAGING_KEYS"
    staging=--staging
fi

# 1. The list of SHA-256s, signed with a release key.
say "release: $base"
get SHA256SUMS
get SHA256SUMS.sig
printf '%s\n' "$trusted" | awk '/BEGIN PUBLIC KEY/ { n++ } n { print > ("key" n ".pem") }'
verified=
for key in key*.pem; do
    [ -f "$key" ] || continue
    if openssl pkeyutl -verify -pubin -inkey "$key" -rawin \
        -in SHA256SUMS -sigfile SHA256SUMS.sig >/dev/null 2>&1; then
        verified=yes
        break
    fi
done
[ -n "$verified" ] || die "SHA256SUMS isn't signed by a release key this script trusts: not installing"

# 2. The hub for this machine and the probes, each with its signed SHA-256.
grep -E "^[0-9a-f]{64}  (hub-$target|probes\.tar\.gz)\$" SHA256SUMS >wanted || true
[ "$(wc -l <wanted)" -eq 2 ] || die "this release has no hub for $target"
get "hub-$target"
get probes.tar.gz
sha256sum -c wanted >/dev/null 2>&1 || die "a download doesn't match its signed SHA-256: not installing"

# 3. The probes beside the hub, so `hub install` adds them.
mkdir -p app/probes
mv "hub-$target" app/hub
chmod 0755 app/hub
tar -xzf probes.tar.gz -C app/probes
say "signature and checksums verified; installing"
./app/hub install ${staging:+"$staging"} "$@"
