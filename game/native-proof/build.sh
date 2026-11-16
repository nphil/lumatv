#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cache=${XDG_CACHE_HOME:-$HOME/.cache}/lumagame-native-proof
sdk=${WEBOS_SDK:-$HOME/webos-ndk/arm-webos-linux-gnueabi_sdk-buildroot}
ares=${ARES_PACKAGE:-$cache/ares/node_modules/.bin/ares-package}
archive=$cache/SDL2-2.30.12-webos.tar.gz
sdl=$cache/sdl
sha=63dc9ea268bdf64801fce5e0c19585ee6aaf79f0b08486c853c47f922385c698
mkdir -p "$cache"
if [ ! -f "$archive" ]; then
  curl -fL -H 'Accept: application/octet-stream' https://api.github.com/repos/webosbrew/SDL-webOS/releases/assets/517606350 -o "$archive.tmp"
  mv "$archive.tmp" "$archive"
fi
printf '%s  %s\n' "$sha" "$archive" | sha256sum -c
if [ ! -f "$sdl/include/SDL2/SDL.h" ] || [ ! -f "$sdl/lib/libSDL2-2.0.so.0" ]; then
  mkdir -p "$sdl"
  tar -xzf "$archive" -C "$sdl"
fi
[ -x "$sdk/bin/arm-webos-linux-gnueabi-gcc" ] || { echo "Set WEBOS_SDK to the relocated webOS ARM SDK" >&2; exit 1; }
[ -x "$ares" ] || { echo "Set ARES_PACKAGE to ares-package (see README)" >&2; exit 1; }
build=$here/build
stage=$build/stage
mkdir -p "$stage/lib" "$build/dist"
"$sdk/bin/arm-webos-linux-gnueabi-gcc" -std=c11 -D_POSIX_C_SOURCE=200809L -O2 -Wall -Wextra -Werror \
  -I"$sdl/include/SDL2" "$here/main.c" -L"$sdl/lib" -lSDL2 -Wl,--allow-shlib-undefined \
  -Wl,-rpath,'$ORIGIN/lib' -o "$stage/native-proof"
cp -L "$sdl/lib/libSDL2-2.0.so.0" "$stage/lib/libSDL2-2.0.so.0"
cp "$sdl/share/licenses/SDL2/LICENSE.txt" "$stage/SDL-LICENSE.txt"
cp "$here/appinfo.json" "$stage/appinfo.json"
python3 - "$stage/icon.png" <<'PY'
import struct, sys, zlib
w = h = 256
rows = []
for y in range(h):
    row = bytearray([0])
    for x in range(w):
        inside = 42 <= y <= 205 and abs(x - 128) * 163 <= (y - 42) * 84
        row.extend((64, 204, 230, 255) if inside else (7, 16, 25, 255))
    rows.append(row)
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
with open(sys.argv[1], 'wb') as f:
    f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows))) + chunk(b'IEND', b''))
PY
"$ares" --no-minify "$stage" -o "$build/dist"
sha256sum "$stage/native-proof" "$stage/lib/libSDL2-2.0.so.0" "$build/dist/"*.ipk
