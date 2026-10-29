#!/bin/sh
# Copies the LumaShow compositor modules the renderer is made of into render/src/ (and LumaShow's `ui` crate into lumashow-ui/) (they are modules of a binary crate,
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
# LumaShow's `ui` crate (the app side: Fonts, atlases, the components) is copied the same way into lumashow-ui/ as a crate of its own, then
# patches/ui/*.patch: the paths its build reads, and the glyph source hook of `Fonts` (the grid-fit text, docs/research/text-render-result.md).
# Its tests/ are not copied and its unit tests are not run here (the crate is built with test = false).
ui_dst=$here/lumashow-ui
rm -rf "$ui_dst"
mkdir -p "$ui_dst"
cp "$src/crates/ui/Cargo.toml" "$src/crates/ui/build.rs" "$ui_dst/"
cp -r "$src/crates/ui/src" "$ui_dst/src"
for patch_file in "$here"/patches/ui/*.patch; do
  [ -f "$patch_file" ] || continue
  (cd "$ui_dst" && git apply -p1 "$patch_file")
done
echo "synced from LumaShow $rev"
