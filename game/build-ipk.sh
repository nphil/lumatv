#!/bin/sh
# Packs LumaGame as its own ipk, org.nphil.lumagame_<version>_all.ipk, in game/build/. tools/build-ipk.sh ships it inside LumaTV's package
# (payload/game/) and tools/gameapp.sh installs it. The version stays 0.1.0 while the programs change, so the build is named by
# `build-id`, a hash of every packed file: in the app's directory and as the control file's Build-Id, which is what the installer compares.
#
# Inside: the feeder as `main` (lumagame-main: the installer's role is keyed to `main`'s path; started by SAM it starts the UI and
# serves the engines' video), the 64-bit UI (bin64/lumagame-ui), run64.sh (the UI starts every engine through it), the 64-bit runtime
# (loader, glibc, the Mali driver and the heap shim: gfx64/) and engines/<id>/ with each engine's engine.json:
#   gfn         lumagame-engine-gfn, OpenNOW's core and the stream engine; the core execs the stream engine by path with no loader of
#               its own, so both are shell wrappers that run the real binaries through the package's loader
#   punktfunk   lumagame-engine-punktfunk
#   steam       lumagame-engine-steam
#   gamestream  the GPL-3.0 release pinned in game/engines.lock, fetched and checked by game/tools/fetch-gamestream.sh
#
# Needs: GF_UI_GAME's feeder from sh gfn/feeder/build.sh; cd gfn/engine && cargo zigbuild --release --target
# aarch64-unknown-linux-gnu.2.36 -p lumatv-gfn-streamer; sh gfn/engine/build-core.sh; sh rustui/tools/build-game.sh; cd game && cargo
# zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.36 -p lumagame-engine-gfn; game/engines/punktfunk/build.sh; game/engines/steam/build.sh;
# gfx64/fetch-blob.sh and gfx64/build.sh.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
feeder=$root/gfn/feeder/build/arm/lumagame-main
ui=$root/rustui/build/game/bin64/lumagame-ui
gfn=$here/target/aarch64-unknown-linux-gnu/release/lumagame-engine-gfn
core=$root/gfn/build/core/aarch64/opennow-core
opennow_license=$root/gfn/build/OpenNOW/LICENSE
streamer=$root/gfn/engine/target/aarch64-unknown-linux-gnu/release/opennow-streamer
punktfunk=$here/engines/punktfunk
steam=$here/engines/steam
bundle=$root/gfx64/build/bundle/lib
shim=$root/gfx64/build/out/heapshim.so
for f in "$feeder" "$ui" "$gfn" "$core" "$opennow_license" "$streamer" "$punktfunk/bin/lumagame-engine-punktfunk" "$steam/bin/lumagame-engine-steam" "$shim" \
  "$bundle/ld-linux-aarch64.so.1" "$root/gfx64/launch.sh"; do
  [ -e "$f" ] || { echo "missing $f (see the header of this script)" >&2; exit 1; }
done
build=$here/build/ipk
data=$build/data/usr/palm/applications/$id
rm -rf "$build"
mkdir -p "$data/bin64" "$data/gfx64/bundle" "$data/engines/gfn/bin" "$data/engines/punktfunk/bin" "$data/engines/steam/bin" "$build/control"
cp "$here/package/appinfo.json" "$here/package/run64.sh" "$here/package/icon.png" "$here/package/largeIcon.png" \
  "$here/package/extraLargeIcon.png" "$data/"
cp "$feeder" "$data/lumagame-main"
cp "$ui" "$data/bin64/lumagame-ui"
cp "$root/gfx64/launch.sh" "$shim" "$data/gfx64/"
cp -R "$bundle" "$data/gfx64/bundle/"

engine=$data/engines/gfn
cp "$here/crates/lumagame-engine-gfn/engine.json" "$engine/"
cp "$opennow_license" "$engine/OPENNOW-LICENSE"
cp "$gfn" "$engine/bin/lumagame-engine-gfn"
cp "$core" "$engine/bin/opennow-core.bin"
cp "$streamer" "$engine/bin/opennow-streamer.bin"
for name in opennow-core opennow-streamer; do
  cat > "$engine/bin/$name" <<WRAP
#!/bin/sh
here=\$(cd "\$(dirname "\$0")" && pwd)
lib=\$here/../../../gfx64/bundle/lib
exec "\$lib/ld-linux-aarch64.so.1" --library-path "\$lib" "\$here/$name.bin" "\$@"
WRAP
done

cp "$punktfunk/engine.json" "$punktfunk/LICENSE-APACHE" "$punktfunk/THIRD-PARTY-NOTICES.txt" "$data/engines/punktfunk/"
cp "$punktfunk/bin/lumagame-engine-punktfunk" "$data/engines/punktfunk/bin/"

cp "$steam/engine.json" "$data/engines/steam/"
cp "$steam/bin/lumagame-engine-steam" "$data/engines/steam/bin/"

sh "$here/tools/fetch-gamestream.sh" "$data"

chmod 755 "$data/lumagame-main" "$data/run64.sh" "$data/bin64/"* "$data/gfx64/launch.sh" "$data/gfx64/bundle/lib/ld-linux-aarch64.so.1" \
  "$engine/bin/"* "$data/engines/punktfunk/bin/"* "$data/engines/steam/bin/"*
( cd "$data" && find . -type f ! -name build-id -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16 ) > "$data/build-id"
build_id=$(cat "$data/build-id")
( cd "$build/data" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../data.tar.gz . )
printf 'Package: %s\nVersion: %s\nBuild-Id: %s\nArchitecture: all\nMaintainer: nphil\nDescription: Game streaming on the video plane\n' "$id" "$version" "$build_id" > "$build/control/control"
( cd "$build/control" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../control.tar.gz control )
echo "2.0" > "$build/debian-binary"
out=$here/build/${id}_${version}_all.ipk
rm -f "$out"
ar rc "$out" "$build/debian-binary" "$build/control.tar.gz" "$build/data.tar.gz"
ls -la "$out"
