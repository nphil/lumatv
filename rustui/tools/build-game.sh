#!/bin/sh
# The same private-loader aarch64 path as the GFN card; game/build-ipk.sh packages this binary.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_PROFILE_RELEASE_DEBUG=false \
  cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --target-dir "$here/target" -p lumatv-rustui --features window --bin lumagame-ui
mkdir -p build/game/bin64
cp target/aarch64-unknown-linux-gnu/release/lumagame-ui build/game/bin64/lumagame-ui
chmod 755 build/game/bin64/lumagame-ui
