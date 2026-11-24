#!/bin/sh
# One command for the whole LumaGame package from a clean checkout: every input of game/build-ipk.sh is built here, then the package is
# packed. Needs zig, cargo, cargo-zigbuild, cmake, ninja, make, curl, python3 (all on the build workers; nothing else, no sysroot). Downloads
# are pinned and hash-checked by the fetch scripts. On the build workers (a clean copy of the tree is the working directory):
#   run_on_worker  cwd=<checkout>  tools=[rust, zig]  results=[game/build/org.nphil.lumagame_0.1.0_all.ipk]
#   command: CARGO_BUILD_JOBS=8 sh game/tools/build-full.sh
# The result is game/build/org.nphil.lumagame_<version>_all.ipk.
set -eu
umask 022 # the package keeps the modes the installer gives (0755/0644) whatever the build host's umask is
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8}
export CMAKE_GENERATOR=${CMAKE_GENERATOR:-Ninja}
step() { echo "== $* ($(date +%H:%M:%S))"; }

step feeder
sh gfn/feeder/build.sh
step streamer
(cd gfn/engine && cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-gfn-streamer)
step core
sh gfn/engine/build-core.sh
step ui
sh rustui/tools/build-game.sh
step gfn-engine
(cd game && cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.36 -p lumagame-engine-gfn)
step punktfunk
game/engines/punktfunk/build.sh
step steam
game/engines/steam/build.sh
step gfx64
sh gfx64/fetch-blob.sh
sh gfx64/build.sh
step pack
sh game/build-ipk.sh
