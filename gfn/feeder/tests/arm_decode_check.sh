#!/bin/sh
# The ARM build of FFmpeg's decoders against the host's, picture by picture: tests/decode_crc.c decodes a clip with each and prints a checksum a picture; the ARM
# one runs under qemu-arm-static (static, musl: no sysroot needed). The NEON assembly is the part nothing else here can judge (the set can). Needs
# `FFMPEG_ARM_STATIC=1 sh third_party/build-ffmpeg.sh` and qemu-arm-static; skips (exit 77) without them.
#   arm_decode_check.sh CLIP.h264 [h264|h265]
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
clip=${1:?clip}
codec=${2:-h264}
out=$here/build/ffmpeg
command -v qemu-arm-static >/dev/null 2>&1 || { echo "no qemu-arm-static"; exit 77; }
[ -f "$out/armstatic/lib/libavcodec.a" ] || { echo "no static ARM build (FFMPEG_ARM_STATIC=1 sh third_party/build-ffmpeg.sh)"; exit 77; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
zig cc -target x86_64-linux-gnu.2.36 -O1 -I"$here/src" -I"$out/host/include" -o "$tmp/host" "$here/tests/decode_crc.c" "$here/src/gf_annexb.c" \
  -L"$out/host/lib" -lavcodec-lumatv -lavutil-lumatv -Wl,-rpath,"$out/host/lib" -lm -lpthread
zig cc -target arm-linux-musleabi -mcpu=cortex_a9 -mfpu=neon -mfloat-abi=softfp -O1 -static -I"$here/src" -I"$out/armstatic/include" -o "$tmp/arm" "$here/tests/decode_crc.c" \
  "$here/src/gf_annexb.c" "$out/armstatic/lib/libavcodec.a" "$out/armstatic/lib/libavutil.a" -lm -lpthread
"$tmp/host" "$clip" "$codec" > "$tmp/host.txt"
qemu-arm-static "$tmp/arm" "$clip" "$codec" > "$tmp/arm.txt" 2> "$tmp/arm.err"
cat "$tmp/arm.err"
n=$(wc -l < "$tmp/host.txt")
if cmp -s "$tmp/host.txt" "$tmp/arm.txt"; then
  echo "arm_decode_check: $n pictures, the ARM build (NEON) decodes every one as the host does"
else
  echo "arm_decode_check: DIFFERENT"; diff "$tmp/host.txt" "$tmp/arm.txt" | head -10; exit 1
fi
