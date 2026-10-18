#!/bin/sh
# Checks the release binaries in a folder (proposal 0013 §9, D-104):
#
#   .github/scripts/check-binaries.sh <folder>
#
# - every Linux binary is static: no program interpreter, no shared library
#   (PRINCIPLES: the probe needs nothing installed on its machine; D-13);
# - each is built for the processor its name says;
# - the ARM ones for the right ARM: an ARMv6 binary (the Pi Zero) must not
#   contain ARMv7 code, which the linker records in the file's attributes;
# - the Windows ones are 64-bit x86 programs.
#
# Needs readelf (binutils) and file.
set -eu

[ $# -eq 1 ] || {
    echo "usage: check-binaries.sh <folder>" >&2
    exit 2
}
failed=0
bad() {
    printf '  FAIL %s: %s\n' "$name" "$*"
    failed=1 this=1
}

for path in "$1"/hub-* "$1"/probe-*; do
    [ -f "$path" ] || continue
    name=$(basename "$path") this=0
    case $name in
    *-windows-*)
        file -b "$path" | grep -q 'PE32+ executable.*x86-64' || bad "not a 64-bit Windows program: $(file -b "$path")"
        [ $this -eq 1 ] || echo "  ok   $name: $(file -b "$path")"
        continue
        ;;
    *x86_64-unknown-linux-*) machine='Advanced Micro Devices X86-64' arch= ;;
    *aarch64-unknown-linux-*) machine='AArch64' arch= ;;
    *armv7-unknown-linux-*) machine='ARM' arch='v7' ;;
    *arm-unknown-linux-*) machine='ARM' arch='v6' ;;
    *)
        bad "no rule for this target"
        continue
        ;;
    esac
    readelf -hW "$path" | grep -q "Machine: *$machine\$" || bad "not built for $machine"
    if readelf -lW "$path" | grep -q 'Requesting program interpreter'; then
        bad "dynamically linked: $(readelf -lW "$path" | grep 'program interpreter')"
    fi
    if readelf -dW "$path" 2>/dev/null | grep -q '(NEEDED)'; then
        bad "needs shared libraries: $(readelf -dW "$path" | grep '(NEEDED)' | tr -s ' ' | tr '\n' ' ')"
    fi
    if [ -n "$arch" ]; then
        # Tag_CPU_arch is the newest architecture any part of the program
        # was built for (the linker keeps the highest): v6 or v6KZ for the Pi
        # Zero, v7 for a Pi 2 or later.
        found=$(readelf -AW "$path" | sed -n 's/^ *Tag_CPU_arch: *//p' | head -1)
        case $arch:$found in
        v6:v6*) ;;
        v7:v7*) ;;
        *) bad "built for ARM ${found:-unknown}, not $arch" ;;
        esac
        arch=" (ARM $found)"
    fi
    [ $this -eq 1 ] || echo "  ok   $name: static, $machine$arch"
done
exit $failed
