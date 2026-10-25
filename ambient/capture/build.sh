#!/bin/sh
# Builds ambient-probe for the set (32-bit ARM, soft-float EABI, glibc 2.28 like every other 32-bit piece here) and a
# host build of the same source with a fake libvtcapture for the logic tests (test.sh). The probe dlopens LG's library
# at run time, so nothing from LG is needed to link.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -o "$out/arm/ambient-probe" "$here/probe.c" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -o "$out/host/ambient-probe" "$here/probe.c" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -shared -fPIC -Wall -Wextra -o "$out/host/libvtcapture.so.1" "$here/fake_vtcapture.c" -lpthread -Wl,-soname,libvtcapture.so.1
echo "built $out/arm/ambient-probe:"
readelf -h "$out/arm/ambient-probe" | grep -E 'Class|Machine'
readelf -d "$out/arm/ambient-probe" | grep NEEDED
ls -la "$out/arm/ambient-probe"
