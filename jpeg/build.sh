#!/bin/sh
# Builds lumatv-jpeg for the set (32-bit ARM, soft-float EABI, glibc 2.28 like lumatv-capture) and a host build of the same
# source with a fake libdile_photo for the logic tests (test.sh). The service dlopens LG's library at run time, so nothing from
# LG is needed to link. convert_neon.c is the one file built for ARMv7-VE with NEON, the way ambient/capture/reduce_neon.c is:
# zig cannot select NEON code for the soft-float target, so it is compiled for the hard-float one and linked in; its interface
# carries no floating-point value, and convert.c calls it only when the kernel reports NEON.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
zig cc -target arm-linux-gnueabihf.2.28 -mcpu=cortex_a15 -O2 -Wall -Wextra -c -o "$out/arm/convert_neon.o" "$here/convert_neon.c"
src="$here/service.c $here/convert.c $here/cpu.c"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -DLUMATV_NEON -o "$out/arm/lumatv-jpeg" $src "$out/arm/convert_neon.o" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -o "$out/host/lumatv-jpeg" $src -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -shared -fPIC -Wall -Wextra -o "$out/host/libdile_photo.so.1" "$here/fake_dilephoto.c" -Wl,-soname,libdile_photo.so.1
# the converters against double precision, and the NEON ones against the scalar ones: host (scalar only), and the set's CPU under qemu-arm
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -o "$out/host/convert_test" "$here/convert_test.c" "$here/convert.c" -lm
zig cc -target arm-linux-gnueabi.2.28 -O2 -Wall -Wextra -DLUMATV_NEON -o "$out/arm/convert_test" "$here/convert_test.c" "$here/convert.c" "$out/arm/convert_neon.o" -lm
echo "built $out/arm/lumatv-jpeg:"
readelf -h "$out/arm/lumatv-jpeg" | grep -E 'Class|Machine'
readelf -d "$out/arm/lumatv-jpeg" | grep NEEDED
ls -la "$out/arm/lumatv-jpeg"
