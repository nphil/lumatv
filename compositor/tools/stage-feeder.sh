#!/bin/sh
# Builds the camera feeder (gfn/feeder/build.sh) and stages it in a bundle: OUT/feeder/{lumatv-gfn-feeder,lib/,feeder.sh}. tv-cutover.sh install
# hands that directory to feeder.sh update. FEEDER_BUILD names an already built gfn/feeder/build/arm (the host test's), which skips the build.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
out=$1
src=${FEEDER_BUILD:-}
if [ -z "$src" ]; then
    sh "$here/gfn/feeder/build.sh" > /dev/null
    src=$here/gfn/feeder/build/arm
fi
[ -f "$src/lumatv-gfn-feeder" ] || { echo "stage-feeder: no $src/lumatv-gfn-feeder" >&2; exit 1; }
rm -rf "$out/feeder"
mkdir -p "$out/feeder/lib"
cp "$src/lumatv-gfn-feeder" "$out/feeder/"
cp "$src"/lib/lib*-lumatv.so.* "$out/feeder/lib/"
cp "$here/payload/feeder/feeder.sh" "$out/feeder/feeder.sh"
chmod +x "$out/feeder/lumatv-gfn-feeder"
# The installed binaries' hashes, in one place beside the bundle.
(cd "$out" && sha256sum bin/* feeder/lumatv-gfn-feeder > HASHES)
