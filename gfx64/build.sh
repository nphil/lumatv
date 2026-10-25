#!/bin/sh
# Cross-builds the aarch64 pieces into gfx64/build/out/ with zig:
#   heapshim.so  the dma_heap shim (glibc 2.36 ABI, the bundle's libc)
#   x1client     the on-screen probe (docs/research/arch/x1/x1client.c), linked against the bundle's libwayland-client
# The probe needs the bundle (fetch-blob.sh) and is skipped without it.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$HERE/build/out
BUNDLE=${LUMATV_GFX64_BUNDLE:-$HERE/build/bundle}
T=aarch64-linux-gnu.2.36
mkdir -p "$OUT"
zig cc -target $T -O2 -Wall -Wextra -shared -fPIC -o "$OUT/heapshim.so" "$HERE/heapshim.c" -ldl
echo "build: $OUT/heapshim.so"
if [ -e "$BUNDLE/lib/libwayland-client.so.0" ]; then
  zig cc -target $T -O2 -o "$OUT/x1client" "$HERE/../docs/research/arch/x1/x1client.c" \
    "$BUNDLE/lib/libwayland-client.so.0" -ldl -lm -Wl,--allow-shlib-undefined
  echo "build: $OUT/x1client"
else
  echo "build: no bundle, probe skipped (run fetch-blob.sh)"
fi
