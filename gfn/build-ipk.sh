#!/bin/sh
# Packs the GeForce NOW app as its own ipk, org.nphil.lumatv.gfn_<version>_all.ipk, in gfn/build/. Nothing of LumaTV's package is
# touched: this app installs and removes on its own through appInstallService/dev/install (tools/cardapp.sh does the same for the
# card app; the same calls apply with this id).
#
# Inside: the feeder as `main` (the installer's role is keyed to `main`'s path and names the media services NDL registers), the 64-bit
# engine and core under bin64/ with run64.sh and the loader and glibc of the 64-bit runtime, the probe streams are NOT included (they
# go to /tmp for the TV steps). The 64-bit UI is added here when it exists (docs/research/gfn-app-result.md, "The app").
#
# Needs: sh gfn/feeder/build.sh; cd gfn/engine && cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-gfn-streamer;
# sh gfn/engine/build-core.sh; gfx64/fetch-blob.sh (for the loader and glibc).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
feeder=$here/feeder/build/arm/lumatv-gfn-feeder
streamer=$here/engine/target/aarch64-unknown-linux-gnu/release/opennow-streamer
core=$here/build/core/aarch64/opennow-core
bundle=$root/gfx64/build/bundle/lib
for f in "$feeder" "$streamer" "$core" "$bundle/ld-linux-aarch64.so.1"; do
  [ -e "$f" ] || { echo "missing $f (see the header of this script)" >&2; exit 1; }
done
build=$here/build/ipk
data=$build/data/usr/palm/applications/$id
rm -rf "$build"
mkdir -p "$data/bin64" "$data/gfx64/bundle/lib" "$build/control"
cp "$here/package/appinfo.json" "$here/package/run64.sh" "$data/"
cp "$root/assets/icon.png" "$root/assets/largeIcon.png" "$root/assets/splash.png" "$data/"
cp "$feeder" "$data/lumatv-gfn-feeder"
cp "$streamer" "$data/bin64/opennow-streamer"
cp "$core" "$data/bin64/opennow-core"
for l in ld-linux-aarch64.so.1 libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libgcc_s.so.1 libstdc++.so.6; do
  cp "$bundle/$l" "$data/gfx64/bundle/lib/"
done
chmod 755 "$data/lumatv-gfn-feeder" "$data/run64.sh" "$data/bin64/"* "$data/gfx64/bundle/lib/ld-linux-aarch64.so.1"
( cd "$build/data" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../data.tar.gz . )
printf 'Package: %s\nVersion: %s\nArchitecture: all\nMaintainer: nphil\nDescription: GeForce NOW on the video plane\n' "$id" "$version" > "$build/control/control"
( cd "$build/control" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../control.tar.gz control )
echo "2.0" > "$build/debian-binary"
out=$here/build/${id}_${version}_all.ipk
rm -f "$out"
ar rc "$out" "$build/debian-binary" "$build/control.tar.gz" "$build/data.tar.gz"
ls -la "$out"
tar -tzf "$build/data.tar.gz" | sort
