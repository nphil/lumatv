#!/bin/sh
# Builds lumatv-sm for the set: 32-bit ARM, soft-float EABI, glibc 2.28 (what LG's userspace is), dynamically
# linked against libluna-service2.so.3 and libglib-2.0.so.0. The workstation has neither library, so the link
# uses stub shared objects carrying only the symbol names and sonames; on the set the loader binds the real ones.
# Also builds a host (x86_64) variant against the same stubs for the JSON scanner tests (sm/test.sh).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/../deploy/sm
mkdir -p "$out/stubs"

cat > "$out/stubs/ls2.c" <<'EOF'
// Symbol stubs for the link only (never run): the set's libluna-service2.so.3 provides the real functions.
#define S(name) void name(void) {}
S(LSErrorInit) S(LSErrorFree) S(LSRegister) S(LSUnregister) S(LSRegisterCategory) S(LSCategorySetData) S(LSGmainAttach)
S(LSCall) S(LSCallOneReply) S(LSCallCancel) S(LSMessageRef) S(LSMessageUnref) S(LSMessageGetPayload) S(LSMessageGetMethod)
S(LSMessageGetSender) S(LSMessageGetSenderServiceName) S(LSMessageGetUniqueToken) S(LSMessageIsSubscription) S(LSMessageReply)
S(LSSubscriptionAdd) S(LSSubscriptionReply) S(LSSubscriptionSetCancelFunction)
EOF
cat > "$out/stubs/glib.c" <<'EOF'
#define S(name) void name(void) {}
S(g_main_loop_new) S(g_main_loop_run) S(g_main_loop_quit) S(g_io_channel_unix_new) S(g_io_add_watch) S(g_timeout_add) S(g_child_watch_add) S(g_source_remove)
EOF

build() { # target outdir
    t=$1; d=$2
    mkdir -p "$d"
    zig cc -target "$t" -shared -fPIC -o "$d/libluna-service2.so.3" "$out/stubs/ls2.c" -Wl,-soname,libluna-service2.so.3
    zig cc -target "$t" -shared -fPIC -o "$d/libglib-2.0.so.0" "$out/stubs/glib.c" -Wl,-soname,libglib-2.0.so.0
    zig cc -target "$t" -O2 -s -Wall -Wextra -Wno-unused-parameter -pthread -o "$d/lumatv-sm" "$here/lumatv-sm.c" "$d/libluna-service2.so.3" "$d/libglib-2.0.so.0"
}
build arm-linux-gnueabi.2.28 "$out/arm"
build x86_64-linux-gnu.2.36 "$out/host"
# The scanner tests, host build, run now.
zig cc -target x86_64-linux-gnu.2.36 -O1 -DLUMATV_SM_TEST -Wno-unused-function -pthread -o "$out/host/lumatv-sm-test" "$here/lumatv-sm.c" "$out/host/libluna-service2.so.3" "$out/host/libglib-2.0.so.0"
"$out/host/lumatv-sm-test"
# The set's binary: strip the stub paths from the record (only the sonames matter) and show what it needs.
cp "$out/arm/lumatv-sm" "$out/lumatv-sm"
echo "built $out/lumatv-sm:"
readelf -d "$out/lumatv-sm" | grep -E 'NEEDED|interpreter' || true
readelf -l "$out/lumatv-sm" | grep -A1 INTERP | tail -1
ls -la "$out/lumatv-sm"
