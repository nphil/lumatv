#!/bin/sh
# Builds opennow-core (OpenNOW's application core: accounts, catalog, CloudMatch sessions, the streamer's parent) for the set, from
# the revision pinned in Cargo.toml, with the `keyring` crate replaced by crates/keyring (the TV has no OS credential store; the core
# then keeps its sessions in its own 0600 JSON files). The checkout lives under gfn/build/OpenNOW (not in git) and the build in
# gfn/build/core-target. `--host` builds for the workstation instead (the UI's host tests drive that one).
#
#   sh gfn/engine/build-core.sh          -> gfn/build/core/aarch64/opennow-core
#   sh gfn/engine/build-core.sh --host   -> gfn/build/core/host/opennow-core
set -eu
here=$(cd "$(dirname "$0")" && pwd)
rev=$(sed -n 's/.*rev = "\([0-9a-f]\{40\}\)".*/\1/p' "$here/Cargo.toml" | head -1)
url=https://github.com/OpenCloudGaming/OpenNOW
build=$here/../build
src=$build/OpenNOW
mkdir -p "$build"
if [ ! -d "$src/.git" ]; then
  git init -q "$src"
  git -C "$src" remote add origin "$url"
fi
if [ "$(git -C "$src" rev-parse HEAD 2>/dev/null || true)" != "$rev" ]; then
  git -C "$src" fetch -q --depth 1 origin "$rev"
  git -C "$src" checkout -q FETCH_HEAD
fi
mode=${1:-tv}
c1="patch.crates-io.keyring.path='$here/crates/keyring'"
c2="target.x86_64-unknown-linux-gnu.linker='$here/tools/cc-host'"
c3="env.CC='$here/tools/cc-host'"
c4="env.CXX='$here/tools/cxx-host'"
manifest=$src/native/opennow-core/Cargo.toml
target_dir=$build/core-target
if [ "$mode" = "--host" ]; then
  cargo build --release --manifest-path "$manifest" --target-dir "$target_dir" --config "$c1" --config "$c2" --config "$c3" --config "$c4"
  mkdir -p "$build/core/host"
  cp "$target_dir/release/opennow-core" "$build/core/host/opennow-core"
  echo "built $build/core/host/opennow-core"
else
  cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --manifest-path "$manifest" --target-dir "$target_dir" --config "$c1" --config "$c2" --config "$c3" --config "$c4"
  mkdir -p "$build/core/aarch64"
  cp "$target_dir/aarch64-unknown-linux-gnu/release/opennow-core" "$build/core/aarch64/opennow-core"
  echo "built $build/core/aarch64/opennow-core"
  readelf -h "$build/core/aarch64/opennow-core" | grep -E 'Class|Machine'
fi
