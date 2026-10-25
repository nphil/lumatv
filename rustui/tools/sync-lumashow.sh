#!/bin/sh
# Copies the LumaShow compositor modules the renderer is made of into render/src/ (they are modules of a binary crate,
# not a crate, so a path dependency is not possible; the same arrangement as compositor/tools/sync-lumashow.sh). The files
# are copied verbatim and never edited by hand; the two changes LumaTV needs are patches/*.patch, applied here: a window
# surface and readback for the GL context (gl.rs), and no gralloc importer, which is the Echo Show's (live.rs). Tests are
# not copied. Re-run after a LumaShow change.
set -eu
src=${LUMASHOW_SRC:-/data/home/LumaShow/lumashow}
here=$(cd "$(dirname "$0")/.." && pwd)
dst=$here/render/src
from=$src/crates/compositor/src
rev=$(git -C "$src" rev-parse --short HEAD 2>/dev/null || echo unknown)
modules="anim atlas backdrop census clock damage feather geometry gl glass groups idle ladder live log mesh ops pacer
  plates prepared profile renderer scene session shaders shadow shm stats still wallpaper wallpapers"
for m in $modules; do
  rm -rf "$dst/$m.rs" "$dst/$m"
  [ -f "$from/$m.rs" ] && cp "$from/$m.rs" "$dst/$m.rs"
  [ -d "$from/$m" ] && cp -r "$from/$m" "$dst/$m"
done
# test-only files inside module directories
find "$dst" -mindepth 2 -name '*_tests.rs' -delete
find "$dst" -name 'counted.rs' -delete
printf '%s\n' "$rev" > "$dst/LUMASHOW_REV"
for patch_file in "$here"/patches/*.patch; do
  (cd "$dst" && git apply -p1 "$patch_file")
done
echo "synced from LumaShow $rev"
