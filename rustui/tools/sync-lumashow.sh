#!/bin/sh
# Copies LumaShow renderer modules, UI, protocol and mesh into LumaTV's local crates.
# LumaTV-specific changes are applied from patches/ so upstream remains untouched.
# Tests outside the copied modules are retained. Re-run after a LumaShow change.
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
# The TV extends the wire format; keep that change local to its vendored protocol.
protocol_dst=$here/compositor-protocol
rm -rf "$protocol_dst"
mkdir -p "$protocol_dst"
cp "$src/crates/compositor-protocol/Cargo.toml" "$protocol_dst/"
cp -r "$src/crates/compositor-protocol/src" "$protocol_dst/src"
for patch_file in "$here"/patches/protocol/*.patch; do
  [ -f "$patch_file" ] || continue
  (cd "$protocol_dst" && git apply -p1 "$patch_file")
done
# Mesh types cross the same boundary, so this copy uses the TV protocol too.
mesh_dst=$here/mesh3d
rm -rf "$mesh_dst"
mkdir -p "$mesh_dst"
cp "$src/crates/mesh3d/Cargo.toml" "$mesh_dst/"
cp -r "$src/crates/mesh3d/src" "$mesh_dst/src"
echo "synced from LumaShow $rev"
