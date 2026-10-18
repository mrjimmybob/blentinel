#!/bin/sh
# Installs zig, the C compiler and linker cargo-zigbuild cross-compiles the
# Linux probes with (D-104), from ziglang.org, checked against the SHA-256
# pinned here (the same idea as pinning an action to a commit: a changed
# download is refused, not run). Adds it to the job's PATH.
#
# To update: the version and the x86_64-linux "shasum" from
# https://ziglang.org/download/index.json, in a pull request of its own.
set -eu
version=0.15.2
sha256=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239

name=zig-x86_64-linux-$version
dir=${RUNNER_TEMP:?run me in GitHub Actions}
curl -fsSL --retry 3 -o "$dir/$name.tar.xz" "https://ziglang.org/download/$version/$name.tar.xz"
echo "$sha256  $dir/$name.tar.xz" | sha256sum -c -
tar -xJf "$dir/$name.tar.xz" -C "$dir"
echo "$dir/$name" >>"$GITHUB_PATH"
"$dir/$name/zig" version
