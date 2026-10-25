#!/bin/sh
# Host test of launch.sh with a fake loader and fake clients: which path runs, when the fallback triggers, what
# exit code comes back. No 64-bit hardware is needed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
FAILS=0
ok() { if [ "$2" = "$3" ]; then :; else FAILS=$((FAILS+1)); echo "FAIL $1: got '$2', want '$3'"; fi; }

# Fake bundle, device tree and loader (the real loader is invoked as: ld --library-path DIR --preload SHIM prog args).
mkdir -p "$T/b/lib" "$T/dev/dri"
: > "$T/b/lib/libmali-fake-wayland-gbm.so"; : > "$T/shim.so"
: > "$T/dev/mali0"
cat > "$T/ld" <<'EOF'
#!/bin/sh
[ "$1" = --library-path ] && [ "$3" = --preload ] || exit 99
echo "loader lib=$2 preload=$4" > "$LOG.loader"
shift 4; exec "$@"
EOF
cat > "$T/ok64" <<'EOF'
#!/bin/sh
echo "path=$LUMATV_GFX_PATH lib=$LUMATV_GFX_LIB args=$*" > "$LOG.client"
exit 0
EOF
cat > "$T/fail64" <<'EOF'
#!/bin/sh
exit 3
EOF
cat > "$T/segv64" <<'EOF'
#!/bin/sh
kill -SEGV $$
EOF
cat > "$T/late64" <<'EOF'
#!/bin/sh
sleep 2; exit 9
EOF
cat > "$T/fb32" <<'EOF'
#!/bin/sh
echo "path=$LUMATV_GFX_PATH lib=$LUMATV_GFX_LIB args=$*" > "$LOG.fallback"
exit 0
EOF
chmod +x "$T"/ld "$T"/ok64 "$T"/fail64 "$T"/segv64 "$T"/late64 "$T"/fb32
export LUMATV_GFX64_LOADER=$T/ld LUMATV_GFX64_UNAME=aarch64 LUMATV_GFX64_DEV=$T/dev
export LUMATV_GFX64_BUNDLE=$T/b LUMATV_GFX64_SHIM=$T/shim.so LUMATV_GFX64_GRACE=1

run() {  # name client [launcher options...]; leaves rc in RC, files in $LOG.*
  LOG=$T/$1; export LOG; rm -f "$LOG".*; c=$2; shift 2
  "$HERE/launch.sh" "$@" -- "$T/$c" a "b c" 2> "$LOG.err"; RC=$?
}
has() { [ -e "$LOG.$1" ] && echo yes || echo no; }
: > "$T/dev/dma_buf_te"; rm -f "$T/dev/dma_buf_te"   # no devices yet
# Device files must be character devices for the launcher's -c test; use real ones that exist everywhere.
rm -f "$T/dev/mali0"; ln -s /dev/null "$T/dev/mali0"; ln -s /dev/null "$T/dev/dri/card0"

run t1 ok64 --fallback "$T/fb32"
ok "64-bit path runs" "$RC $(has client) $(has fallback)" "0 yes no"
ok "client env" "$(cat "$LOG.client")" "path=64 lib=$T/b/lib/libmali-fake-wayland-gbm.so args=a b c"
ok "loader args" "$(cat "$LOG.loader")" "loader lib=$T/b/lib preload=$T/shim.so"

run t2 fail64 --fallback "$T/fb32"
ok "quick failure falls back" "$RC $(has fallback)" "0 yes"
ok "fallback env and args" "$(cat "$LOG.fallback")" "path=32 lib=/usr/lib/libmali.so.0 args=a b c"
ok "fallback logged" "$(grep -c 'exit 3 within' "$LOG.err")" "1"

run t3 segv64 --fallback "$T/fb32"
ok "quick signal falls back" "$RC $(has fallback)" "0 yes"

run t4 late64 --fallback "$T/fb32"
ok "late failure is the client's" "$RC $(has fallback)" "9 no"

run t5 fail64
ok "quick failure, no fallback: client's code" "$RC $(has fallback)" "3 no"

# Preflight failures never start the client.
LUMATV_GFX64_DEV=$T/nodev run t6 ok64 --fallback "$T/fb32"
ok "missing /dev/mali0" "$RC $(has client) $(has fallback)" "0 no yes"
LUMATV_GFX64_BUNDLE=$T/empty run t7 ok64 --fallback "$T/fb32"
ok "missing bundle" "$RC $(has client) $(has fallback)" "0 no yes"
LUMATV_GFX64_UNAME=armv7l run t8 ok64 --fallback "$T/fb32"
ok "32-bit kernel" "$RC $(has client) $(has fallback)" "0 no yes"
LUMATV_GFX64_SHIM=$T/none.so run t9 ok64 --fallback "$T/fb32"
ok "missing shim" "$RC $(has client) $(has fallback)" "0 no yes"
LUMATV_GFX64_DISABLE=1 run t10 ok64 --fallback "$T/fb32"
ok "kill switch" "$RC $(has client) $(has fallback)" "0 no yes"
LUMATV_GFX64_DEV=$T/nodev run t11 ok64
ok "unavailable, no fallback" "$RC $(has client)" "69 no"
rm "$T/dev/dri/card0"
run t12 ok64 --fallback "$T/fb32"
ok "no allocator device" "$RC $(has client) $(has fallback)" "0 no yes"
ln -s /dev/null "$T/dev/dma_buf_te"
run t13 ok64 --fallback "$T/fb32"
ok "dma_buf_te alone is enough" "$RC $(has client) $(has fallback)" "0 yes no"
# SAM's jail for native apps: mali0 and dma_buf_unified, no dri/, no dma_buf_te.
rm "$T/dev/dma_buf_te"
ln -s /dev/null "$T/dev/dma_buf_unified"
run t13b ok64 --fallback "$T/fb32"
ok "dma_buf_unified alone is enough" "$RC $(has client) $(has fallback)" "0 yes no"
ok "dma_buf_unified: client env" "$(cat "$LOG.client")" "path=64 lib=$T/b/lib/libmali-fake-wayland-gbm.so args=a b c"
rm "$T/dev/dma_buf_unified"
run t13c ok64
ok "no allocator, no fallback: 69, all three devices named" "$RC $(grep -c 'dri/card0, .*dma_buf_te, .*dma_buf_unified' "$LOG.err")" "69 1"
ln -s /dev/null "$T/dev/dma_buf_te"

# SIGTERM to the launcher stops the client and does not fall back.
cat > "$T/hang64" <<'EOF'
#!/bin/sh
exec sleep 30
EOF
chmod +x "$T/hang64"
LOG=$T/t14; export LOG; rm -f "$LOG".*
"$HERE/launch.sh" --fallback "$T/fb32" -- "$T/hang64" 2> "$LOG.err" & lp=$!
sleep 0.5; kill -TERM $lp; wait $lp; RC=$?
ok "SIGTERM: 143, no fallback" "$RC $(has fallback)" "143 no"
ok "SIGTERM: client gone" "$(pgrep -f "sleep 30" | wc -l | tr -d ' ')" "0"

[ $FAILS = 0 ] && echo "test_launch: all passed" || echo "test_launch: $FAILS FAILED"
[ $FAILS = 0 ]
