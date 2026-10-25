#!/bin/sh
# Builds the card app's launcher (lumatv-card.c) for the set and for the host test.
#   build/arm/lumatv-card     32-bit ARM, soft-float EABI, glibc 2.28 (what LG's userspace is), dynamically linked against
#                             libluna-service2.so.3 and libglib-2.0.so.0. The workstation has neither of LG's libraries, so the link
#                             uses stub shared objects carrying only the symbol names and sonames; on the set the loader binds the real ones
#                             (the way compositor/sm/build.sh does it for lumatv-sm).
#   build/host/               the same source for x86_64 against the real host glib and fakels2.c, for tests/test_card_launcher.py:
#                             lumatv-card, libluna-service2.so.3 (the fake), fakeapp.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$here/build}
mkdir -p "$out/stubs" "$out/arm" "$out/host"

cat > "$out/stubs/ls2.c" <<'EOS'
// Symbol stubs for the link only (never run): the set's libluna-service2.so.3 provides the real functions.
#define S(name) void name(void) {}
S(LSErrorInit) S(LSErrorFree) S(LSRegister) S(LSUnregister) S(LSGmainAttach) S(LSCall) S(LSCallCancel) S(LSMessageGetPayload)
EOS
cat > "$out/stubs/glib.c" <<'EOS'
#define S(name) void name(void) {}
S(g_main_loop_new) S(g_main_loop_run) S(g_main_loop_quit) S(g_io_channel_unix_new) S(g_io_add_watch) S(g_timeout_add) S(g_child_watch_add) S(g_unix_signal_add)
EOS

t=arm-linux-gnueabi.2.28
zig cc -target $t -shared -fPIC -o "$out/arm/libluna-service2.so.3" "$out/stubs/ls2.c" -Wl,-soname,libluna-service2.so.3
zig cc -target $t -shared -fPIC -o "$out/arm/libglib-2.0.so.0" "$out/stubs/glib.c" -Wl,-soname,libglib-2.0.so.0
zig cc -target $t -O2 -s -Wall -Wextra -Wno-unused-parameter -o "$out/arm/lumatv-card" "$here/lumatv-card.c" "$out/arm/libluna-service2.so.3" "$out/arm/libglib-2.0.so.0"

# The host variant: the real glib of the workstation (the .so.0 has no dev symlink), the fake library, the same source.
glib=$(ls /usr/lib/x86_64-linux-gnu/libglib-2.0.so.0 /usr/lib64/libglib-2.0.so.0 2>/dev/null | head -n 1 || true)
if [ -n "$glib" ]; then
  h=x86_64-linux-gnu.2.36
  zig cc -target $h -O1 -g -shared -fPIC -Wall -Wextra -o "$out/host/libluna-service2.so.3" "$here/fakels2.c" "$glib" -Wl,-soname,libluna-service2.so.3
  zig cc -target $h -O1 -g -Wall -Wextra -Wno-unused-parameter -o "$out/host/lumatv-card" "$here/lumatv-card.c" "$out/host/libluna-service2.so.3" "$glib"
  cp "$here/fakeapp.sh" "$out/host/fakeapp.sh"
  echo "built $out/host (the launcher against the fake hub)"
else
  echo "build: no libglib-2.0.so.0 on this host; the host variant is skipped"
fi
echo "built $out/arm/lumatv-card:"
readelf -d "$out/arm/lumatv-card" | grep -E 'NEEDED' || true
readelf -l "$out/arm/lumatv-card" | grep -A1 INTERP | tail -1
ls -la "$out/arm/lumatv-card"
