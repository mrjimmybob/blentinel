#!/bin/sh
# Tests the signing scripts on a small fake release (proposal 0013 §10.4 C,
# D-107): key-from-seed.sh refuses a seed the binaries don't trust,
# sign-release.sh signs, and verify-release.sh accepts the signing key only,
# and refuses a changed file, an unlisted file, and a probe its manifest
# doesn't vouch for even when SHA256SUMS is re-signed around it.
#
#   .github/scripts/signing-test.sh
#
# Run from the repository root. Needs openssl, jq, python3 and shellcheck,
# all on GitHub's Ubuntu runner. Uses the browser tests' test key only.
set -eu

S=.github/scripts
shellcheck "$S/key-from-seed.sh" "$S/verify-release.sh" "$S/signing-test.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() {
    echo "signing-test: FAILED: $*" >&2
    exit 1
}
refused() { if "$@" >/dev/null 2>&1; then fail "accepted: $*"; fi; }

release() {
    mkdir -p "$1/probes"
    printf 'hub' >"$1/hub-x86_64-unknown-linux-musl"
    printf '#!/bin/sh\n' >"$1/hub.sh"
    echo '{"version":"0.1.0-dev.5"}' >"$1/release.json"
    printf 'linux' >"$1/probes/probe-x86_64-unknown-linux-musl"
    printf 'windows' >"$1/probes/probe-x86_64-pc-windows-msvc.exe"
    jq -n --arg a "$(sha256sum <"$1/probes/probe-x86_64-unknown-linux-musl" | cut -d' ' -f1)" \
        --arg b "$(sha256sum <"$1/probes/probe-x86_64-pc-windows-msvc.exe" | cut -d' ' -f1)" \
        '{format: 1, version: "0.1.0-dev.5", artifacts: [
            {file: "probe-x86_64-unknown-linux-musl", sha256: $a},
            {file: "probe-x86_64-pc-windows-msvc.exe", sha256: $b}]}' >"$1/probes/manifest.json"
}

seed=$(jq -r .seed hub/end2end/release-key.json)
public=$(jq -r .public hub/end2end/release-key.json)
other_key="$work/other.pem"
openssl genpkey -algorithm ed25519 -out "$other_key"
other=$(openssl pkey -in "$other_key" -pubout -outform DER | tail -c 32 | od -An -v -tx1 | tr -d ' \n')

# SHA256SUMS of a folder made again, and signed with a key: what someone
# who changed a file would do.
resum() { # <folder> <private key>
    (
        cd "$1"
        rm SHA256SUMS SHA256SUMS.sig
        # shellcheck disable=SC2046 # release file names have no spaces
        sha256sum --text $(find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) >SHA256SUMS
        openssl pkeyutl -sign -inkey "$2" -rawin -in SHA256SUMS -out SHA256SUMS.sig
    )
}

# key-from-seed.sh: the key, private to its owner, only if trusted.
key="$work/key.pem"
shown=$(BLENTINEL_SIGNING_SEED=$seed sh "$S/key-from-seed.sh" "$key" "$other, $public")
[ "$shown" = "$public" ] || fail "key-from-seed printed $shown"
[ "$(stat -c %a "$key")" = 600 ] || fail "the key file isn't mode 600"
# A secret stored with a line end, as a pipe in PowerShell gives it.
crlf=$(printf '%s\r\n ' "$seed")
BLENTINEL_SIGNING_SEED=$crlf sh "$S/key-from-seed.sh" "$work/crlf.pem" "$public" >/dev/null ||
    fail "a seed ending with a line end was refused"
cmp -s "$key" "$work/crlf.pem" || fail "a seed ending with a line end gave another key"
BLENTINEL_SIGNING_SEED=$seed refused sh "$S/key-from-seed.sh" "$work/untrusted.pem" "$other"
[ ! -e "$work/untrusted.pem" ] || fail "an untrusted key was left on disk"
for bad in "" zz 0123 "$seed$seed"; do
    BLENTINEL_SIGNING_SEED=$bad refused sh "$S/key-from-seed.sh" "$work/bad.pem" "$public"
done

# sign-release.sh, then verify-release.sh with the right key and another.
release "$work/r"
sh "$S/sign-release.sh" "$work/r" "$key" >/dev/null 2>&1
sh "$S/verify-release.sh" "$work/r" "$public" || fail "the signed release doesn't verify"
refused sh "$S/verify-release.sh" "$work/r" "$other"

# A changed file: as it is, then with SHA256SUMS made again and signed by
# another key. And an unlisted file.
cp -R "$work/r" "$work/changed"
printf 'x' >>"$work/changed/hub-x86_64-unknown-linux-musl"
refused sh "$S/verify-release.sh" "$work/changed" "$public"
cp -R "$work/changed" "$work/resigned"
resum "$work/resigned" "$other_key"
refused sh "$S/verify-release.sh" "$work/resigned" "$public"
cp -R "$work/r" "$work/extra"
touch "$work/extra/extra"
refused sh "$S/verify-release.sh" "$work/extra" "$public"

# A probe swapped inside probes.tar.gz, with SHA256SUMS re-made and
# re-signed, so that only the inner layer can catch it: first with the
# manifest left alone (its sums refuse the probe), then with the manifest
# rewritten to match (its signature refuses it).
swap() { # <folder> <rewrite the manifest: yes|no>
    cp -R "$work/r" "$1"
    rm -rf "$work/probes" && mkdir "$work/probes"
    tar -xzf "$1/probes.tar.gz" -C "$work/probes"
    printf 'other' >"$work/probes/probe-x86_64-unknown-linux-musl"
    if [ "$2" = yes ]; then
        sum=$(sha256sum <"$work/probes/probe-x86_64-unknown-linux-musl" | cut -d' ' -f1)
        jq --arg s "$sum" '(.artifacts[] | select(.file == "probe-x86_64-unknown-linux-musl") | .sha256) = $s' \
            "$work/probes/manifest.json" >"$work/manifest.json"
        mv "$work/manifest.json" "$work/probes/manifest.json"
    fi
    (cd "$work/probes" && tar -czf "$1/probes.tar.gz" manifest.json manifest.json.sig probe-*)
    resum "$1" "$key"
}
swap "$work/swapped" no
refused sh "$S/verify-release.sh" "$work/swapped" "$public"
swap "$work/rewritten" yes
refused sh "$S/verify-release.sh" "$work/rewritten" "$public"

echo "signing-test: passed"
