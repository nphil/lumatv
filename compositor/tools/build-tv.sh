#!/bin/sh
# Builds everything the set needs and assembles compositor/deploy/:
#   bin/lumatv-compositor, bin/lumatv-testclient, bin/lumatv-loadgen   64-bit glibc (cargo zigbuild, glibc 2.36)
#   bin/x2cli                                       the X2 client, 32-bit, drawing through LG's own libmali
#   bin/lumatv-sm                                   the C4 supervisor and luna bridge, 32-bit (sm/lumatv-sm.c)
#   assets/ui/                                      native UI assets
#   lib/                                            the private Debian bookworm arm64 loader and libraries plus the
#                                                   64-bit Mali blob, as used by X2 r8b (docs/research/shell-architecture.md)
#   run.sh                                          the on-set runner (tools/tv-run.sh)
#   gate.sh, lpc-probe.sh                           the gate (tools/tv-gate.sh) and the read-only interrupt, wake-up and memory-limit probes (tools/tv-lpc-probe.sh)
#   scanout-proof.sh                                the off-screen proof of direct scan-out (tools/tv-scanout-proof.sh: the 256x256 LG-driver client on a private socket)
#   hwplanes.sh                                     the hardware-planes steps (tools/tv-hwplanes.sh, driven by tools/tv_hwplanes.py)
#   mvread.sh                                       the read-only Multi View probes: the media server's files, the media stack's words, the pipelines and sinks (tools/tv-multiview-read.sh)
#   subwindow.sh                                    the sub-window check beside an app's video (tools/tv-subwindow.sh: the feeder in the starfish pipeline asking for the sub sink)
#   feeder/                                         the camera feeder (gfn/feeder/, 32-bit) with its decoder libraries and feeder.sh; cutover.sh install updates an installed feeder from it
#   HASHES                                          SHA-256 of bin/* and the feeder, as built
#   rustjson.py                                     the gate's reader of `ctl shell` (tools/tv-rustjson.py: the rust check, and the QML client's part of the shell check)
# The bundle is not committed; tools/tv-first-light.sh deploy copies it to /tmp/lumatv on the set. LUMATV_FEATURES names cargo
# features for the compositor and defaults to `ui` (the engine host, the Rust shell's bin/lumatv-ui and --ui-demo): the deploy that a
# round's installer runs sets no environment, and a bundle without `ui` (round 17's) has no Rust shell to switch on. `LUMATV_FEATURES=`
# builds without it; FEATURES in the bundle says which it was built with.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
# The loader, libraries and Mali blob the set runs; a copy of the set's own /var/lib/lumatv/compositor/lib, checked against its
# bundle.manifest, when the download is gone.
m64=${M64_LIBS:-$HOME/.cache/lumatv/m64/lib}
out=$here/deploy
features=${LUMATV_FEATURES-ui}
cd "$here"
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-compositor ${features:+--features "$features"}
rm -rf "$out"
mkdir -p "$out/bin" "$out/lib"
printf '%s\n' "$features" > "$out/FEATURES"
cp target/aarch64-unknown-linux-gnu/release/lumatv-compositor target/aarch64-unknown-linux-gnu/release/lumatv-testclient target/aarch64-unknown-linux-gnu/release/lumatv-loadgen "$out/bin/"
# The Rust shell (rustui/ui, bin/lumatv-ui beside the compositor: shell_rust.rs finds it there) goes with the engine host.
case ",$features," in *,ui,*)
    (cd "$here/../rustui" && cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-ui)
    cp "$here/../rustui/target/aarch64-unknown-linux-gnu/release/lumatv-ui" "$out/bin/" ;;
esac
x2=$here/../docs/research/arch/x2
zig cc -target arm-linux-gnueabi.2.28 -O1 -s -o "$out/bin/x2cli" "$x2/x2cli.c" "$x2/../sysroot32/libwayland-client.so.0.20.0" -ldl -Wl,--allow-shlib-undefined
cp "$m64"/* "$out/lib/"
sh "$here/sm/build.sh" > /dev/null
x1=$here/../docs/research/arch/x1
zig cc -target aarch64-linux-gnu.2.36 -O1 -o "$out/bin/x1client" "$x1/x1client.c" "$m64/libwayland-client.so.0" -ldl -lm -Wl,--allow-shlib-undefined
zig cc -target aarch64-linux-gnu.2.36 -O1 -shared -fPIC -o "$out/lib/x1heapshim.so" "$x1/x1heapshim.c" -ldl
cp "$out/sm/lumatv-sm" "$out/bin/lumatv-sm"
mkdir -p "$out/assets"
cp -R "$here/assets/ui" "$out/assets/ui"
cp "$here/tools/tv-run.sh" "$out/run.sh"
cp "$here/tools/tv-kms-window.sh" "$out/kms-window.sh"
cp "$here/tools/tv-sm-dryrun.sh" "$out/sm-dryrun.sh"
cp "$here/tools/tv-ui-measure.sh" "$out/ui-measure.sh"
cp "$here/tools/tv-cutover.sh" "$out/cutover.sh"
cp "$here/tools/tv-recover-home.sh" "$out/recover-home.sh"
# The firmware and boot checks are tools/compositor.sh's, run on the set by cutover.sh and gate.sh beside the tool's own copy.
cp "$here/../tools/compositor.sh" "$out/compositor.sh"
cp "$here/../tools/logrelay.py" "$out/logrelay.py"
cp "$here/firmware/verified.list" "$out/verified.list"
cp "$here/tools/tv-uinput-key.py" "$out/uinput-key.py"
cp "$here/tools/tv-wlprobe.py" "$out/wlprobe.py"
cp "$here/tools/tv-gate.sh" "$out/gate.sh"
cp "$here/tools/tv-lpc-probe.sh" "$out/lpc-probe.sh"
cp "$here/tools/tv-scanout-proof.sh" "$out/scanout-proof.sh"
cp "$here/tools/tv-hwplanes.sh" "$out/hwplanes.sh"
cp "$here/tools/tv-rustjson.py" "$out/rustjson.py"
cp "$here/tools/tv-multiview-read.sh" "$out/mvread.sh"
cp "$here/tools/tv-subwindow.sh" "$out/subwindow.sh"
cp "$here/../docs/research/arch/wlproto_from_elf.py" "$out/wlproto_from_elf.py"
sh "$here/tools/stage-feeder.sh" "$out"
chmod +x "$out/bin/"* "$out/run.sh" "$out/scanout-proof.sh" "$out/hwplanes.sh" "$out/subwindow.sh"
cat "$out/HASHES"
du -sh "$out"
ls -la "$out/bin"
