#!/bin/sh
# Builds lumatv-homestub: the set's binary (aarch64, static, musl: it needs no loader and none of LG's 32-bit libraries) in build/arm64/,
# and a host build of the same source in build/host/ for tests/test_homestub_native.py, which runs it against a stand-in compositor.
# `zig` is the only tool (the same one the other components' build.sh use).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/arm64" "$out/host"
flags="-Os -s -Wall -Wextra -fno-stack-protector -fno-unwind-tables -fno-asynchronous-unwind-tables -ffunction-sections -fdata-sections -Wl,--gc-sections"
# shellcheck disable=SC2086
zig cc -target aarch64-linux-musl -static $flags -o "$out/arm64/lumatv-homestub" "$here/homestub.c"
# shellcheck disable=SC2086
zig cc -target x86_64-linux-musl -static $flags -o "$out/host/lumatv-homestub" "$here/homestub.c"
echo "built $out/arm64/lumatv-homestub:"
readelf -h "$out/arm64/lumatv-homestub" | grep -E 'Class|Machine'
readelf -l "$out/arm64/lumatv-homestub" | grep -E 'INTERP|DYNAMIC' || echo "static: no interpreter, no dynamic section"
ls -la "$out/arm64/lumatv-homestub"
