#!/bin/sh
# Builds lumatv-lsbridge for the set: 32-bit ARM, soft-float EABI, glibc 2.28 (what LG's userspace is), dynamically linked against
# libluna-service2.so.3 and libglib-2.0.so.0, as helper/mm/build.sh and compositor/sm/build.sh do. The workstation has neither library,
# so the link uses stub shared objects carrying only the symbol names and sonames; on the set the loader binds the real ones.
# Then builds and runs the host test (test.c: a fake libluna-service2).
#   build/arm/lumatv-lsbridge, embedded into the helper by app/build.rs (env LUMATV_LSBRIDGE names another file)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
mkdir -p "$out/stubs"

cat > "$out/stubs/ls2.c" <<'EOF'
// Symbol stubs for the link only (never run): the set's libluna-service2.so.3 provides the real functions.
#define S(name) void name(void) {}
S(LSErrorInit) S(LSErrorFree) S(LSRegister) S(LSUnregister) S(LSGmainAttach) S(LSCall) S(LSCallCancel) S(LSMessageGetPayload)
EOF
cat > "$out/stubs/glib.c" <<'EOF'
#define S(name) void name(void) {}
S(g_main_loop_new) S(g_main_loop_run) S(g_main_loop_quit) S(g_io_channel_unix_new) S(g_io_add_watch)
EOF

stubs() { # target outdir
    mkdir -p "$2"
    zig cc -target "$1" -shared -fPIC -o "$2/libluna-service2.so.3" "$out/stubs/ls2.c" -Wl,-soname,libluna-service2.so.3
    zig cc -target "$1" -shared -fPIC -o "$2/libglib-2.0.so.0" "$out/stubs/glib.c" -Wl,-soname,libglib-2.0.so.0
}
flags="-O2 -Wall -Wextra -Werror"

stubs arm-linux-gnueabi.2.28 "$out/arm"
# shellcheck disable=SC2086
zig cc -target arm-linux-gnueabi.2.28 $flags -s -o "$out/arm/lumatv-lsbridge" "$here/lumatv-lsbridge.c" "$out/arm/libluna-service2.so.3" "$out/arm/libglib-2.0.so.0"

stubs x86_64-linux-gnu.2.36 "$out/host"
# The test build leaves out main, so its pieces go unused there.
# shellcheck disable=SC2086
zig cc -target x86_64-linux-gnu.2.36 $flags -Wno-unused-function -Wno-unused-variable -DLUMATV_LSBRIDGE_TEST -o "$out/host/lumatv-lsbridge-test" "$here/lumatv-lsbridge.c"
"$out/host/lumatv-lsbridge-test"

(cd "$out/arm" && sha256sum lumatv-lsbridge > lumatv-lsbridge.sha256)
readelf -d "$out/arm/lumatv-lsbridge" | grep -E 'NEEDED' || true
readelf -l "$out/arm/lumatv-lsbridge" | grep -A1 INTERP | tail -1
echo "built $out/arm/lumatv-lsbridge: $(wc -c < "$out/arm/lumatv-lsbridge") bytes, sha256 $(cut -d' ' -f1 "$out/arm/lumatv-lsbridge.sha256")"
