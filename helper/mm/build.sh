#!/bin/sh
# Builds lumatv-mm for the set: 32-bit ARM, soft-float EABI, glibc 2.28 (what LG's userspace is), dynamically linked against
# libluna-service2.so.3 and libglib-2.0.so.0, as compositor/sm/build.sh does. The workstation has neither library, so the link uses stub
# shared objects carrying only the symbol names and sonames; on the set the loader binds the real ones.
# Then builds and runs the host tests (test.c: a fake libluna-service2 replaying the callers recorded on the set).
#   build/arm/lumatv-mm (+ .sha256), shipped by tools/build-ipk.sh as payload/mm/lumatv-mm
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/stubs"

cat > "$out/stubs/ls2.c" <<'EOF'
// Symbol stubs for the link only (never run): the set's libluna-service2.so.3 provides the real functions.
#define S(name) void name(void) {}
S(LSErrorInit) S(LSErrorFree) S(LSRegister) S(LSUnregister) S(LSRegisterCategory) S(LSGmainAttach) S(LSCallOneReply) S(LSSignalSend)
S(LSMessageGetPayload) S(LSMessageGetMethod) S(LSMessageGetSender) S(LSMessageGetSenderServiceName) S(LSMessageIsSubscription)
S(LSMessageReply) S(LSSubscriptionAdd) S(LSSubscriptionReply)
EOF
cat > "$out/stubs/glib.c" <<'EOF'
#define S(name) void name(void) {}
S(g_main_loop_new) S(g_main_loop_run) S(g_io_channel_unix_new) S(g_io_add_watch) S(g_timeout_add)
EOF

stubs() { # target outdir
    mkdir -p "$2"
    zig cc -target "$1" -shared -fPIC -o "$2/libluna-service2.so.3" "$out/stubs/ls2.c" -Wl,-soname,libluna-service2.so.3
    zig cc -target "$1" -shared -fPIC -o "$2/libglib-2.0.so.0" "$out/stubs/glib.c" -Wl,-soname,libglib-2.0.so.0
}
flags="-O2 -Wall -Wextra -Werror -pthread"

stubs arm-linux-gnueabi.2.28 "$out/arm"
# shellcheck disable=SC2086
zig cc -target arm-linux-gnueabi.2.28 $flags -s -o "$out/arm/lumatv-mm" "$here/lumatv-mm.c" "$out/arm/libluna-service2.so.3" "$out/arm/libglib-2.0.so.0"

stubs x86_64-linux-gnu.2.36 "$out/host"
# The test build leaves out main and the live loop, so their pieces go unused there.
# shellcheck disable=SC2086
zig cc -target x86_64-linux-gnu.2.36 $flags -Wno-unused-function -Wno-unused-variable -DLUMATV_MM_TEST -o "$out/host/lumatv-mm-test" "$here/lumatv-mm.c" "$out/host/libglib-2.0.so.0"
"$out/host/lumatv-mm-test"

(cd "$out/arm" && sha256sum lumatv-mm > lumatv-mm.sha256)
readelf -d "$out/arm/lumatv-mm" | grep -E 'NEEDED' || true
readelf -l "$out/arm/lumatv-mm" | grep -A1 INTERP | tail -1
echo "built $out/arm/lumatv-mm: $(wc -c < "$out/arm/lumatv-mm") bytes, sha256 $(cut -d' ' -f1 "$out/arm/lumatv-mm.sha256")"
