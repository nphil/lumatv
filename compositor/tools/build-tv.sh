#!/bin/sh
# Builds everything the set needs and assembles compositor/deploy/:
#   bin/lumatv-compositor, bin/lumatv-testclient   64-bit glibc (cargo zigbuild, glibc 2.36)
#   bin/x2cli                                       the X2 client, 32-bit, drawing through LG's own libmali
#   bin/lumatv-sm                                   the C4 supervisor and luna bridge, 32-bit (sm/lumatv-sm.c)
#   lib/                                            the private Debian bookworm arm64 loader and libraries plus the
#                                                   64-bit Mali blob, as used by X2 r8b (docs/research/shell-architecture.md)
#   run.sh                                          the on-set runner (tools/tv-run.sh)
# The bundle is not committed; tools/tv-first-light.sh deploy copies it to /tmp/lumatv on the set.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
m64=${M64_LIBS:-/data/home/tmp/m64dl/deploy/lib}
out=$here/deploy
cd "$here"
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-compositor
rm -rf "$out"
mkdir -p "$out/bin" "$out/lib"
cp target/aarch64-unknown-linux-gnu/release/lumatv-compositor target/aarch64-unknown-linux-gnu/release/lumatv-testclient "$out/bin/"
x2=$here/../docs/research/arch/x2
zig cc -target arm-linux-gnueabi.2.28 -O1 -s -o "$out/bin/x2cli" "$x2/x2cli.c" "$x2/../sysroot32/libwayland-client.so.0.20.0" -ldl -Wl,--allow-shlib-undefined
cp "$m64"/* "$out/lib/"
sh "$here/sm/build.sh" > /dev/null
cp "$out/sm/lumatv-sm" "$out/bin/lumatv-sm"
cp "$here/tools/tv-run.sh" "$out/run.sh"
cp "$here/tools/tv-kms-window.sh" "$out/kms-window.sh"
cp "$here/tools/tv-sm-dryrun.sh" "$out/sm-dryrun.sh"
cp "$here/tools/tv-cutover.sh" "$out/cutover.sh"
cp "$here/tools/tv-recover-home.sh" "$out/recover-home.sh"
cp "$here/../docs/research/arch/wlproto_from_elf.py" "$out/wlproto_from_elf.py"
chmod +x "$out/bin/"* "$out/run.sh"
du -sh "$out"
ls -la "$out/bin"
