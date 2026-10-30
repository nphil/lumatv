#!/bin/sh
# Builds lumatv-gfn-feeder for the set (32-bit ARM, soft-float EABI, glibc 2.28, like every 32-bit piece here) and for the host (the
# tests, tests/test_gfn_feeder.py), with the NDL stand-in (tests/fake_ndl.c). The window code is vplane's vpl_wl.c, compiled in: LG's
# library is dlopened at run time and the compositor is spoken to on the wire, so nothing of LG's is linked. libopus's decoder
# (third_party/, fetched and pinned) is linked statically for the PCM fallback. The texture path (a camera's picture decoded here when no video
# window is free) dlopens FFmpeg's H.264 and H.265 decoders from `lib/` beside the program: third_party/build-ffmpeg.sh builds them (LGPL, shared, the
# decoders only) and this script copies them to build/{arm,host}/lib/, where the package takes them from; the feeder is not linked against them.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm" "$out/host"
sh "$here/third_party/build-opus.sh"
sh "$here/third_party/build-ffmpeg.sh"
opus_src=$("$here/third_party/fetch-opus.sh")
src="$here/src/feeder.c $here/src/gf_annexb.c $here/src/gf_ndl.c $here/src/gf_log.c $here/src/gf_opus.c $here/src/gf_sam.c $here/src/gf_swdec.c $here/src/gf_yuv.c $here/../../vplane/src/vpl_wl.c"
inc="-I$here/src -I$here/../../vplane/src -I$opus_src/include"
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -Werror $inc -I"$out/ffmpeg/arm/include" -o "$out/arm/lumatv-gfn-feeder" $src "$out/opus/arm/libopus-dec.a" -ldl -lpthread -Wl,--gc-sections
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined $inc -I"$out/ffmpeg/host/include" -o "$out/host/lumatv-gfn-feeder" $src "$out/opus/host/libopus-dec.a" -ldl -lpthread
for t in arm host; do
  rm -rf "$out/$t/lib"
  mkdir -p "$out/$t/lib"
  cp -a "$out/ffmpeg/$t/lib"/lib*-lumatv.so.* "$out/$t/lib/"
done
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined,address -fno-sanitize-recover=undefined -I$here/src -o "$out/host/yuv_test" "$here/tests/yuv_test.c" "$here/src/gf_yuv.c"
# what a picture costs as a texture, in CPU time: for the host here and for the set (tests/swdec_bench.c; the set's number is the TV check's)
zig cc -target x86_64-linux-gnu.2.36 -O2 -Wall -Wextra -Werror -I$here/src -I"$out/ffmpeg/host/include" -o "$out/host/swdec_bench" "$here/tests/swdec_bench.c" "$here/src/gf_swdec.c" "$here/src/gf_yuv.c" "$here/src/gf_annexb.c" -ldl
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -Werror -I$here/src -I"$out/ffmpeg/arm/include" -o "$out/arm/swdec_bench" "$here/tests/swdec_bench.c" "$here/src/gf_swdec.c" "$here/src/gf_yuv.c" "$here/src/gf_annexb.c" -ldl -lpthread
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -shared -fPIC -o "$out/host/libNDL_directmedia.so.1" "$here/tests/fake_ndl.c" -lpthread -Wl,-soname,libNDL_directmedia.so.1
zig cc -target x86_64-linux-gnu.2.36 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined -I$here/src -o "$out/host/split_test" "$here/tests/split_test.c" "$here/src/gf_annexb.c"
# SAM's registration (gf_sam.c) dlopens LG's luna-service2 and glib: the host test runs it against the card launcher's stand-in hub
# (rustui/app/stub/fakels2.c) on the workstation's own glib; without that glib the SAM tests are skipped.
glib=$(ls /usr/lib/x86_64-linux-gnu/libglib-2.0.so.0 /usr/lib64/libglib-2.0.so.0 2>/dev/null | head -n 1 || true)
if [ -n "$glib" ]; then
  zig cc -target x86_64-linux-gnu.2.36 -O1 -g -shared -fPIC -Wall -Wextra -o "$out/host/libluna-service2.so.3" "$here/../../rustui/app/stub/fakels2.c" "$glib" -Wl,-soname,libluna-service2.so.3
fi
echo "built $out/arm/lumatv-gfn-feeder:"
readelf -h "$out/arm/lumatv-gfn-feeder" | grep -E 'Class|Machine'
readelf -d "$out/arm/lumatv-gfn-feeder" | grep NEEDED
ls -la "$out/arm/lumatv-gfn-feeder"
