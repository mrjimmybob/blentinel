#!/bin/sh
# Verifies a signed hub release with one public key, as a machine that
# trusts only that key would (proposal 0013 §10.4 C, D-107):
#
#   .github/scripts/verify-release.sh <release folder> <public key, hex>
#
# 1. SHA256SUMS.sig is the key's Ed25519 signature of SHA256SUMS (as hub.sh
#    checks it), and every file listed has its sum, and no file is unlisted;
# 2. inside probes.tar.gz, manifest.json.sig (hex) is the key's signature of
#    manifest.json, and every probe has the SHA-256 the manifest says.
#
# Exits 0 only if all of it holds. Uses openssl, sha256sum, tar and jq.
set -eu

die() {
    printf 'verify-release: %s\n' "$*" >&2
    exit 1
}

[ $# -eq 2 ] || die "usage: verify-release.sh <release folder> <public key hex>"
folder=$1
hex=$2
case $hex in
*[!0-9a-fA-F]*) die "the public key isn't hex" ;;
esac
[ ${#hex} -eq 64 ] || die "the public key must be 64 hex digits"
for file in SHA256SUMS SHA256SUMS.sig probes.tar.gz; do
    [ -f "$folder/$file" ] || die "$folder has no $file"
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The public key as PEM (RFC 8410): a fixed 12-byte prefix, then the 32 bytes.
{
    echo "-----BEGIN PUBLIC KEY-----"
    python3 -c 'import base64, sys; print(base64.b64encode(bytes.fromhex("302a300506032b6570032100" + sys.argv[1])).decode())' "$hex"
    echo "-----END PUBLIC KEY-----"
} >"$work/public.pem"
verify() { openssl pkeyutl -verify -pubin -inkey "$work/public.pem" -rawin -in "$1" -sigfile "$2" >/dev/null 2>&1; }

verify "$folder/SHA256SUMS" "$folder/SHA256SUMS.sig" || die "SHA256SUMS.sig isn't this key's signature"
(cd "$folder" && sha256sum -c --strict --quiet SHA256SUMS) || die "a file doesn't match SHA256SUMS"
listed=$(awk '{ sub(/^\*/, "", $2); print $2 }' "$folder/SHA256SUMS" | LC_ALL=C sort)
present=$(for path in "$folder"/*; do
    file=${path##*/}
    case $file in SHA256SUMS | SHA256SUMS.sig) ;; *) echo "$file" ;; esac
done | LC_ALL=C sort)
[ "$listed" = "$present" ] || die "the folder's files aren't exactly those SHA256SUMS lists"

mkdir "$work/probes"
tar -xzf "$folder/probes.tar.gz" -C "$work/probes"
tr -d '\n' <"$work/probes/manifest.json.sig" | python3 -c 'import sys; sys.stdout.buffer.write(bytes.fromhex(sys.stdin.read()))' >"$work/manifest.sig"
verify "$work/probes/manifest.json" "$work/manifest.sig" || die "manifest.json.sig isn't this key's signature"
jq -r '.artifacts[] | "\(.sha256)  \(.file)"' "$work/probes/manifest.json" >"$work/probes.sums"
[ -s "$work/probes.sums" ] || die "the probe manifest lists no probe"
(cd "$work/probes" && sha256sum -c --strict --quiet ../probes.sums) || die "a probe doesn't match its manifest"
printf 'verify-release: %s verifies with %s\n' "$folder" "$hex" >&2
