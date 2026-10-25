#!/bin/sh
# Copies the LumaShow modules this compositor shares from the LumaShow tree (they are modules of a binary crate,
# not crates, so a path dependency is not possible). pacer.rs is copied verbatim and never edited here; its test
# file is cut before the tests that drive LumaShow's quality ladder, which is not shared (docs/research/
# compositor-c2-result.md, "Reuse or new"). Re-run after a LumaShow change.
set -eu
src=${LUMASHOW_SRC:-/data/home/LumaShow/lumashow}
dst=$(cd "$(dirname "$0")/.." && pwd)/crates/lumatv-compositor/src/vendor
rev=$(git -C "$src" rev-parse --short HEAD 2>/dev/null || echo unknown)
cp "$src/crates/compositor/src/pacer.rs" "$dst/pacer.rs"
awk '/^\/\/ --- The Full ladder over the pacer.s own prediction/ { exit } { print }' "$src/crates/compositor/src/pacer_tests.rs" > "$dst/pacer_tests.rs"
printf '%s\n' "$rev" > "$dst/LUMASHOW_REV"
echo "synced from LumaShow $rev"
