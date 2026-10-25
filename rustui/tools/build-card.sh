#!/bin/sh
# Builds the card app's programs for the set into rustui/build/card/, the files tools/build-ipk.sh packs into the app's ipk:
#   bin/lumatv-card            the 32-bit launcher SAM starts (app/stub/lumatv-card.c; zig cc, arm-linux-gnueabi, glibc 2.28)
#   bin64/lumatv-card-app      the 64-bit app (aarch64, glibc 2.36, run through the private Debian loader of gfx64/), stripped
# The 64-bit runtime (gfx64/build/bundle, gfx64/build/out/heapshim.so) is not built here: gfx64/fetch-blob.sh and gfx64/build.sh do.
# The app is built in its own target directory so that its stripped, release build never replaces the spike's.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_PROFILE_RELEASE_DEBUG=false \
  cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 --target-dir "$here/target/card" -p lumatv-rustui --features window --bin lumatv-card-app
sh app/stub/build.sh "$here/app/stub/build" > /dev/null
out=build/card
rm -rf "$out"
mkdir -p "$out/bin" "$out/bin64"
cp app/stub/build/arm/lumatv-card "$out/bin/lumatv-card"
cp target/card/aarch64-unknown-linux-gnu/release/lumatv-card-app "$out/bin64/lumatv-card-app"
chmod 755 "$out/bin/lumatv-card" "$out/bin64/lumatv-card-app"
ls -la "$out/bin" "$out/bin64"
