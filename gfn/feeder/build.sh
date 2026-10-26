#!/bin/sh
# Builds lumatv-gfn-feeder for the set (32-bit ARM, soft-float EABI, glibc 2.28, like every 32-bit piece here) and for the host (the
# tests, tests/test_gfn_feeder.py), with the NDL stand-in (tests/fake_ndl.c). The window code is vplane's vpl_wl.c, compiled in: LG's
# library is dlopened at run time and the compositor is spoken to on the wire, so nothing of LG's is linked. libopus's decoder
# (third_party/, fetched and pinned) is linked statically for the PCM fallback.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
sh "$here/third_party/build-opus.sh"
opus_src=$("$here/third_party/fetch-opus.sh")
src="$here/src/feeder.c $here/src/gf_annexb.c $here/src/gf_ndl.c $here/src/gf_log.c $here/src/gf_opus.c $here/../../vplane/src/vpl_wl.c"
inc="-I$here/src -I$here/../../vplane/src -I$opus_src/include"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -Werror $inc -o "$out/arm/lumatv-gfn-feeder" $src "$out/opus/arm/libopus-dec.a" -ldl -Wl,--gc-sections
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined $inc -o "$out/host/lumatv-gfn-feeder" $src "$out/opus/host/libopus-dec.a" -ldl
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -shared -fPIC -o "$out/host/libNDL_directmedia.so.1" "$here/tests/fake_ndl.c" -lpthread -Wl,-soname,libNDL_directmedia.so.1
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined -I$here/src -o "$out/host/split_test" "$here/tests/split_test.c" "$here/src/gf_annexb.c"
echo "built $out/arm/lumatv-gfn-feeder:"
readelf -h "$out/arm/lumatv-gfn-feeder" | grep -E 'Class|Machine'
readelf -d "$out/arm/lumatv-gfn-feeder" | grep NEEDED
ls -la "$out/arm/lumatv-gfn-feeder"
