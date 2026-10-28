#!/bin/sh
# Builds lumatv-venc for the set and for the host.
#
# The set's binary is the 64-bit static one (aarch64, musl): the kernel's own V4L2 layouts, no compat
# path, nothing needed from the set. The 32-bit build is made with glibc 2.28 (soft-float EABI, like the
# set's own 32-bit libraries) and NOT with musl: musl on 32-bit ARM has a 64-bit time_t, which makes
# struct v4l2_buffer 80 bytes where the set's 5.4 kernel knows only the 68-byte layout, so QUERYBUF, QBUF
# and DQBUF would be refused. The 64-bit caveat of the video tap's address query is handled in vtcap.c.
#
# The host build links the fake kernel (tests/fake_sys.c) in place of the real system calls, so the same
# probe and the unit tests run on the workstation (test.sh).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/arm64" "$out/host"
common="$here/venc.c $here/hma.c $here/vtcap.c $here/annexb.c $here/synth.c $here/pools.c $here/watchdog.c"
flags="-O2 -Wall -Wextra -Wno-unused-parameter"

zig cc -target arm-linux-gnueabi.2.28 -O2 -Wall -Wextra -Wno-unused-parameter -s -o "$out/arm/lumatv-venc" "$here/probe.c" $common "$here/sysops.c"
zig cc -target aarch64-linux-musl -mcpu=cortex_a76 $flags -static -s -o "$out/arm64/lumatv-venc" "$here/probe.c" $common "$here/sysops.c"

hostflags="-O1 -g -Wall -Wextra -Wno-unused-parameter"
zig cc -target x86_64-linux-gnu.2.36 $hostflags -o "$out/host/lumatv-venc" "$here/probe.c" $common "$here/tests/fake_sys.c"
zig cc -target x86_64-linux-gnu.2.36 $hostflags -o "$out/host/venc_test" "$here/tests/venc_test.c" $common "$here/tests/fake_sys.c"

for b in arm/lumatv-venc arm64/lumatv-venc; do
    echo "built $out/$b:"
    python3 "$(dirname "$(command -v zig)")/readelf.py" -h "$out/$b" 2>/dev/null | grep -E 'Class|Machine' || true
    ls -la "$out/$b"
done
