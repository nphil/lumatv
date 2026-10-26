#!/bin/sh
# Builds lumatv-vplane for the set (32-bit ARM, soft-float EABI, glibc 2.28 like every other 32-bit piece here) and a host build of the
# same source for the tests (test.sh, tests/test_vplane_player.py), with a stand-in for LG's NDL library (tests/fake_ndl.c). The player
# dlopens LG's library at run time and speaks Wayland on the wire, so nothing from LG is needed to link.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
src="$here/src/vplane.c $here/src/vpl_json.c $here/src/vpl_vpc.c $here/src/vpl_plan.c $here/src/vpl_ui.c $here/src/vpl_wl.c $here/src/vpl_ndl.c"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -Werror -o "$out/arm/lumatv-vplane" $src -lm -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined -o "$out/host/lumatv-vplane" $src -lm -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -shared -fPIC -o "$out/host/libNDL_directmedia.so.1" "$here/tests/fake_ndl.c" -lpthread -Wl,-soname,libNDL_directmedia.so.1
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined -o "$out/host/plan_test" "$here/tests/plan_test.c" "$here/src/vpl_plan.c" "$here/src/vpl_json.c" "$here/src/vpl_ui.c" "$here/src/vpl_vpc.c" -lm
echo "built $out/arm/lumatv-vplane:"
readelf -h "$out/arm/lumatv-vplane" | grep -E 'Class|Machine'
readelf -d "$out/arm/lumatv-vplane" | grep NEEDED
ls -la "$out/arm/lumatv-vplane"
