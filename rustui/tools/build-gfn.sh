#!/bin/sh
# Builds the GeForce NOW app's 64-bit UI for the set into rustui/build/gfn/bin64/lumatv-gfn-app (aarch64, glibc 2.36, run through the
# private Debian loader of gfx64/, stripped), which gfn/build-ipk.sh packs. Same target directory as the card app's build.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_PROFILE_RELEASE_DEBUG=false \
  cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --target-dir "$here/target/card" -p lumatv-rustui --features window --bin lumatv-gfn-app
out=build/gfn
rm -rf "$out"
mkdir -p "$out/bin64"
cp target/card/aarch64-unknown-linux-gnu/release/lumatv-gfn-app "$out/bin64/lumatv-gfn-app"
chmod 755 "$out/bin64/lumatv-gfn-app"
ls -la "$out/bin64"
