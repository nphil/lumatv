#!/bin/sh
# Builds ambient-probe and lumatv-capture (the service the package ships) for the set (32-bit ARM, soft-float EABI, glibc 2.28 like every other 32-bit piece here) and a
# host build of the same source with a fake libvtcapture for the logic tests (test.sh). The probe dlopens LG's library
# at run time, so nothing from LG is needed to link.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -o "$out/arm/ambient-probe" "$here/probe.c" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -o "$out/host/ambient-probe" "$here/probe.c" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -shared -fPIC -Wall -Wextra -o "$out/host/libvtcapture.so.1" "$here/fake_vtcapture.c" -lpthread -Wl,-soname,libvtcapture.so.1
# lumatv-capture: the long-running service the package ships (service.c + the reduce), same target, same host build against the fake library.
# reduce_neon.c is the one file built for ARMv7-VE with NEON and hardware divide: zig cannot select NEON code for the soft-float
# target, so it is compiled for the hard-float one (cortex-a15 is the ARMv7-VE profile) and linked into the soft-float program; its
# interface carries no floating-point value, and reduce.c only calls it when the kernel reports NEON and divide.
# -ffp-contract=off: the reduce's double-precision formulas must stay plain IEEE operations.
zig cc -target arm-linux-gnueabihf.2.28 -mcpu=cortex_a15 -O2 -Wall -Wextra -ffp-contract=off -c -o "$out/arm/reduce_neon.o" "$here/reduce_neon.c"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -ffp-contract=off -DLUMATV_NEON -o "$out/arm/lumatv-capture" "$here/service.c" "$here/reduce.c" "$out/arm/reduce_neon.o" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -ffp-contract=off -o "$out/host/lumatv-capture" "$here/service.c" "$here/reduce.c" -ldl
# the reduce against the double-precision code it replaced (test.sh): host, and the set's CPU (scalar and NEON paths, under qemu-arm)
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -ffp-contract=off -o "$out/host/reduce_test" "$here/reduce_test.c" "$here/reduce.c"
zig cc -target arm-linux-gnueabi.2.28 -O2 -Wall -Wextra -ffp-contract=off -DLUMATV_NEON -o "$out/arm/reduce_test" "$here/reduce_test.c" "$here/reduce.c" "$out/arm/reduce_neon.o"
for bin in ambient-probe lumatv-capture; do
    echo "built $out/arm/$bin:"
    readelf -h "$out/arm/$bin" | grep -E 'Class|Machine'
    readelf -d "$out/arm/$bin" | grep NEEDED
    ls -la "$out/arm/$bin"
done
