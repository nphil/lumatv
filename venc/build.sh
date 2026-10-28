#!/bin/sh
# Builds lumatv-venc for the set and for the host.
#
# The set's binary is 32-bit ARM, static (musl): the three ioctl ABIs it speaks are what LG's own 32-bit
# libraries speak (libkadaptor over /dev/venc, libhma, libvtcapture over /dev/video60), so the compat ioctl
# paths it takes are the ones LG exercises every day; it needs nothing from the set to run. A 64-bit build
# (aarch64, static) is made as well to keep the sources clean for the helper, which is 64-bit; the one
# 64-bit caveat is written in vt_abi.h (the physical-address query reads 32-bit pointers).
#
# The host build links the fake kernel (tests/fake_sys.c) in place of the real system calls, so the same
# probe and the unit tests run on the workstation (test.sh).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/arm64" "$out/host"
common="$here/venc.c $here/hma.c $here/vtcap.c $here/annexb.c $here/synth.c"
flags="-O2 -Wall -Wextra -Wno-unused-parameter"

zig cc -target arm-linux-musleabihf -mcpu=cortex_a76 $flags -static -s -o "$out/arm/lumatv-venc" "$here/probe.c" $common "$here/sysops.c"
zig cc -target aarch64-linux-musl -mcpu=cortex_a76 $flags -static -s -o "$out/arm64/lumatv-venc" "$here/probe.c" $common "$here/sysops.c"

hostflags="-O1 -g -Wall -Wextra -Wno-unused-parameter"
zig cc -target x86_64-linux-gnu.2.36 $hostflags -o "$out/host/lumatv-venc" "$here/probe.c" $common "$here/tests/fake_sys.c"
zig cc -target x86_64-linux-gnu.2.36 $hostflags -o "$out/host/venc_test" "$here/tests/venc_test.c" $common "$here/tests/fake_sys.c"

for b in arm/lumatv-venc arm64/lumatv-venc; do
    echo "built $out/$b:"
    python3 "$(dirname "$(command -v zig)")/readelf.py" -h "$out/$b" 2>/dev/null | grep -E 'Class|Machine' || true
    ls -la "$out/$b"
done
