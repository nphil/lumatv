#!/bin/sh
# Cross-builds the client for the set (aarch64, glibc 2.36, the private Debian loader of gfx64/) into rustui/build/tv/.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-rustui --features window
mkdir -p build/tv
cp target/aarch64-unknown-linux-gnu/release/lumatv-rustui build/tv/
ls -la build/tv
