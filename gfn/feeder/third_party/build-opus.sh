#!/bin/sh
# libopus's decoder as two static archives, build/opus/arm/libopus-dec.a (32-bit ARM soft-float EABI, glibc 2.28, like the feeder) and
# build/opus/host/libopus-dec.a (the tests). Fixed-point (the set's feeder is ARMv7 and the decode is integer maths either way), no
# float API, no assembly, no encoder: the archive carries the encoder's objects out only where the decoder needs them.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
src=$("$here/third_party/fetch-opus.sh")
out=$here/build/opus
if [ -f "$out/arm/libopus-dec.a" ] && [ -f "$out/host/libopus-dec.a" ] && [ "$out/arm/libopus-dec.a" -nt "$0" ]; then exit 0; fi
files=$(cd "$src" && {
  for f in celt/bands celt/celt celt/celt_decoder celt/cwrs celt/entcode celt/entdec celt/entenc celt/kiss_fft celt/laplace celt/mathops celt/mdct \
           celt/modes celt/pitch celt/celt_lpc celt/quant_bands celt/rate celt/vq; do echo "$f.c"; done
  sed -n '/^SILK_SOURCES = /,/^$/p' silk_sources.mk | grep -o 'silk/[A-Za-z0-9_]*\.c'
  for f in src/opus src/opus_decoder src/extensions src/repacketizer; do echo "$f.c"; done
})
flags="-O2 -DOPUS_BUILD -DUSE_ALLOC=1 -DUSE_ALLOCA -DFIXED_POINT -DDISABLE_FLOAT_API -DHAVE_LRINTF -DHAVE_LRINT -w -ffunction-sections -fdata-sections \
 -I$src/include -I$src/celt -I$src/silk -I$src/silk/fixed -I$src/src -I$src"
for t in arm:arm-linux-gnueabi.2.28 host:x86_64-linux-gnu.2.36; do
  name=${t%%:*}
  target=${t#*:}
  obj=$out/$name/obj
  rm -rf "$out/$name"
  mkdir -p "$obj"
  echo "$files" | (cd "$src" && xargs -P 8 -I{} sh -c 'zig cc -target "$1" '"$flags"' -c "$2" -o "$3/$(echo "$2" | tr / _).o"' _ "$target" {} "$obj")
  (cd "$obj" && zig ar rcs ../libopus-dec.a *.o)
  rm -rf "$obj"
done
echo "built $out/{arm,host}/libopus-dec.a" >&2
