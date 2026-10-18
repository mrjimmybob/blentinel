#!/bin/sh
# Signs an assembled release in place (proposal 0013 §6.2, D-104).
#
#   .github/scripts/sign-release.sh <release folder> <Ed25519 private key, PEM>
#
# The folder is what `blentinelmake release assemble` writes: the hubs,
# hub.sh, release.json and probes/ (the probes and their manifest.json).
# This writes what `blentinelmake release hub` writes when it signs, in the
# same formats (D-55, D-57): probes/manifest.json.sig (hex), probes.tar.gz,
# SHA256SUMS and SHA256SUMS.sig (raw).
#
# Why a script and not blentinelmake: the job that signs must run no code
# from the build. Compiling runs every dependency's build script, and a
# signing program built in that job could send the key anywhere. This uses
# only openssl, sha256sum and tar, which come with the runner. Ed25519
# signatures don't depend on the program that makes them (RFC 8032), and the
# release is then opened by hub.sh, `hub install` and `hub upgrade` in the
# systemd test (.github/scripts/systemd-test.sh).
set -eu

die() {
    printf 'sign-release: %s\n' "$*" >&2
    exit 1
}

[ $# -eq 2 ] || die "usage: sign-release.sh <release folder> <private key PEM>"
folder=$1
key=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
[ -f "$key" ] || die "no key at $2"
[ -f "$folder/probes/manifest.json" ] || die "$folder isn't an assembled release (no probes/manifest.json)"
[ ! -e "$folder/SHA256SUMS.sig" ] || die "$folder is signed already"
cd "$folder"

sign() { openssl pkeyutl -sign -inkey "$key" -rawin -in "$1" -out "$2"; }

# 1. The probe manifest: its signature in hex, on one line.
sign probes/manifest.json probes/manifest.json.raw
od -An -v -tx1 probes/manifest.json.raw | tr -d ' \n' >probes/manifest.json.sig
printf '\n' >>probes/manifest.json.sig
rm probes/manifest.json.raw

# 2. The probe release as one archive, its files at the top level.
(cd probes && tar -czf ../probes.tar.gz manifest.json manifest.json.sig probe-*)
rm -r probes

# 3. SHA256SUMS of every file, sorted by name as bytes, and its raw signature.
files=$(ls | LC_ALL=C sort) # before SHA256SUMS exists
# shellcheck disable=SC2086 # release file names have no spaces
sha256sum --text $files >SHA256SUMS
sign SHA256SUMS SHA256SUMS.sig

# 4. Checked as hub.sh checks it, with the public half of the key.
public=$(mktemp)
trap 'rm -f "$public"' EXIT
openssl pkey -in "$key" -pubout -out "$public"
openssl pkeyutl -verify -pubin -inkey "$public" -rawin -in SHA256SUMS -sigfile SHA256SUMS.sig >/dev/null ||
    die "the signature of SHA256SUMS doesn't verify"
sha256sum -c --quiet SHA256SUMS
printf 'sign-release: signed %s\n' "$(pwd)" >&2
cat SHA256SUMS
