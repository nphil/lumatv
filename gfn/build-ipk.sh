#!/bin/sh
# Packs the GeForce NOW app as its own ipk, org.nphil.lumatv.gfn_<version>_all.ipk, in gfn/build/. The app is a package of its own
# (appInstallService/dev/install, as for the card app), and tools/build-ipk.sh ships this ipk inside LumaTV's package (payload/gfn/) so that
# a round's install carries it: `lumatv apply` updates an installed app, `lumatv gfn install` puts it on first (tools/gfnapp.sh).
# The package's version stays 0.1.0 while its programs change, so the build is named by `build-id`, a hash of every packed file: in the
# app's directory and as the control file's Build-Id, which is what the installer compares.
#
# Inside: the feeder as `main` (the installer's role is keyed to `main`'s path and names the media services NDL registers; started by
# SAM it starts the UI and serves the engine), the 64-bit UI, engine and core under bin64/ with run64.sh, the 64-bit runtime (loader,
# glibc, the Mali driver and the heap shim: gfx64/), the wallpaper, and the latency tools (bin64/lumatv-gfn-latency and the send-stamp
# preload, which the engine loads only when /var/lib/lumatv-gfn/sendtap exists). The probe streams are NOT included (they go to /tmp for the TV
# steps).
#
# Needs: sh gfn/feeder/build.sh; cd gfn/engine && for each of `-p lumatv-gfn-streamer`, `-p lumatv-gfn-sendtap` and `-p lumatv-gfn-pad --bin
# lumatv-gfn-latency`: cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 (a `--bin` selection builds no library, so the
# preload is its own call); sh gfn/engine/build-core.sh; sh rustui/tools/build-gfn.sh; gfx64/fetch-blob.sh and gfx64/build.sh.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$here/package/appinfo.json" | head -1)
feeder=$here/feeder/build/arm/lumatv-gfn-feeder
streamer=$here/engine/target/aarch64-unknown-linux-gnu/release/opennow-streamer
latency=$here/engine/target/aarch64-unknown-linux-gnu/release/lumatv-gfn-latency
sendtap=$here/engine/target/aarch64-unknown-linux-gnu/release/liblumatv_gfn_sendtap.so
core=$here/build/core/aarch64/opennow-core
bundle=$root/gfx64/build/bundle/lib
app=$root/rustui/build/gfn/bin64/lumatv-gfn-app
shim=$root/gfx64/build/out/heapshim.so
for f in "$feeder" "$streamer" "$latency" "$sendtap" "$core" "$app" "$shim" "$bundle/ld-linux-aarch64.so.1" "$root/gfx64/launch.sh" "$root/payload/wallpaper/wall_1080.png"; do
  [ -e "$f" ] || { echo "missing $f (see the header of this script)" >&2; exit 1; }
done
build=$here/build/ipk
data=$build/data/usr/palm/applications/$id
rm -rf "$build"
mkdir -p "$data/bin64" "$data/gfx64/bundle" "$data/assets/wallpaper" "$build/control"
cp "$here/package/appinfo.json" "$here/package/run64.sh" "$data/"
cp "$here/package/icon.png" "$here/package/largeIcon.png" "$here/package/extraLargeIcon.png" "$here/package/splash.png" "$data/"
cp "$feeder" "$data/lumatv-gfn-feeder"
# The core execs the engine by path with no loader of its own, and the set's 32-bit userland has no /lib/ld-linux-aarch64.so.1, so
# the path the core gets is a shell wrapper that runs the real binary through the package's loader.
cp "$streamer" "$data/bin64/opennow-streamer.bin"
cat > "$data/bin64/opennow-streamer" <<'WRAP'
#!/bin/sh
here=$(cd "$(dirname "$0")/.." && pwd)
# Latency runs: `touch /var/lib/lumatv-gfn/sendtap` before the stream stamps each send's return (docs/research/gfn-input-result.md).
if [ -e "${LUMATV_GFN_DATA:-/var/lib/lumatv-gfn}/sendtap" ] && [ -f "$here/bin64/lumatv-gfn-sendtap.so" ]; then
  export LD_PRELOAD="$here/bin64/lumatv-gfn-sendtap.so"
fi
exec "$here/gfx64/bundle/lib/ld-linux-aarch64.so.1" --library-path "$here/gfx64/bundle/lib" "$here/bin64/opennow-streamer.bin" "$@"
WRAP
cp "$sendtap" "$data/bin64/lumatv-gfn-sendtap.so"
cp "$latency" "$data/bin64/lumatv-gfn-latency.bin"
cat > "$data/bin64/lumatv-gfn-latency" <<'WRAP'
#!/bin/sh
here=$(cd "$(dirname "$0")/.." && pwd)
exec "$here/gfx64/bundle/lib/ld-linux-aarch64.so.1" --library-path "$here/gfx64/bundle/lib" "$here/bin64/lumatv-gfn-latency.bin" "$@"
WRAP
cp "$core" "$data/bin64/opennow-core"
cp "$app" "$data/bin64/lumatv-gfn-app"
cp "$root/gfx64/launch.sh" "$shim" "$data/gfx64/"
cp -R "$bundle" "$data/gfx64/bundle/"
cp "$root/payload/wallpaper/wall_1080.png" "$data/assets/wallpaper/"
chmod 755 "$data/lumatv-gfn-feeder" "$data/run64.sh" "$data/bin64/"* "$data/gfx64/launch.sh" "$data/gfx64/bundle/lib/ld-linux-aarch64.so.1"
( cd "$data" && find . -type f ! -name build-id | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16 ) > "$data/build-id"
build_id=$(cat "$data/build-id")
( cd "$build/data" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../data.tar.gz . )
printf 'Package: %s\nVersion: %s\nBuild-Id: %s\nArchitecture: all\nMaintainer: nphil\nDescription: GeForce NOW on the video plane\n' "$id" "$version" "$build_id" > "$build/control/control"
( cd "$build/control" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='1970-01-01 00:00:00 UTC' -czf ../control.tar.gz control )
echo "2.0" > "$build/debian-binary"
out=$here/build/${id}_${version}_all.ipk
rm -f "$out"
ar rc "$out" "$build/debian-binary" "$build/control.tar.gz" "$build/data.tar.gz"
ls -la "$out"
tar -tzf "$build/data.tar.gz" | sort
