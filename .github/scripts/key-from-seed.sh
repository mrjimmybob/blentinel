#!/bin/sh
# Writes a release signing key as the PEM openssl reads, from its seed
# (proposal 0013 §6.2, D-104, D-107):
#
#   BLENTINEL_SIGNING_SEED=<64 hex> .github/scripts/key-from-seed.sh <out PEM> <trusted public keys>
#
# The seed is the 32 bytes `blentinelmake release keygen` writes as
# release.key, in hex. It comes from the environment, never from the command
# line: arguments show in the process list, and a step's command is logged.
# Nothing here prints it.
#
# <trusted public keys> is the list the builds compiled in (hex, separated by
# commas or spaces). The key's public half must be one of them: a key the
# binaries don't trust would sign a release nothing can install. Prints that
# public half (hex), which isn't secret.
set -eu

die() {
    printf 'key-from-seed: %s\n' "$*" >&2
    exit 1
}

[ $# -eq 2 ] || die "usage: BLENTINEL_SIGNING_SEED=<hex> key-from-seed.sh <out PEM> <trusted public keys>"
out=$1
trusted=$2
# Spaces and line ends dropped: a secret pasted or piped in may end with one.
seed=$(printf '%s' "${BLENTINEL_SIGNING_SEED:-}" | tr -d ' \t\r\n')
BLENTINEL_SIGNING_SEED=$seed
export BLENTINEL_SIGNING_SEED
[ -n "$seed" ] || die "BLENTINEL_SIGNING_SEED is empty: is the secret set, in this job's environment?"
case $seed in
*[!0-9a-fA-F]*) die "BLENTINEL_SIGNING_SEED isn't hex" ;;
esac
[ ${#seed} -eq 64 ] || die "BLENTINEL_SIGNING_SEED must be 64 hex digits (32 bytes), not ${#seed}"

# The seed as PKCS#8 (RFC 8410): a fixed 16-byte prefix, then the 32 bytes.
# Python reads the seed from its environment, so it never is an argument.
umask 077
{
    echo "-----BEGIN PRIVATE KEY-----"
    python3 -c 'import base64, os; print(base64.b64encode(bytes.fromhex("302e020100300506032b657004220420" + os.environ["BLENTINEL_SIGNING_SEED"])).decode())'
    echo "-----END PRIVATE KEY-----"
} >"$out"

public=$(openssl pkey -in "$out" -pubout -outform DER | tail -c 32 | od -An -v -tx1 | tr -d ' \n')
for key in $(printf '%s' "$trusted" | tr ',' ' '); do
    if [ "$(printf '%s' "$key" | tr 'A-F' 'a-f')" = "$public" ]; then
        echo "$public"
        exit 0
    fi
done
rm -f "$out"
die "the key's public half ($public) isn't among the trusted keys the binaries were built with"
