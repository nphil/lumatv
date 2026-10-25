#!/bin/sh
# Runs a 64-bit client on the Rockchip Mali blob through the heap shim, or, when that path cannot start, runs the
# 32-bit fallback on LG's own driver.
#
#   launch.sh [--bundle DIR] [--shim FILE] [--grace SECONDS] [--fallback CMD] -- CLIENT64 [ARGS...]
#
# The 64-bit path is used when the bundle (fetch-blob.sh), the shim and the client exist, the kernel is 64-bit,
# /dev/mali0 exists and a buffer allocator exists (/dev/dri/card0, /dev/dma_buf_te or /dev/dma_buf_unified, the only
# one in SAM's jail for native apps). It has failed to start when
# the client exits non-zero (or dies on a signal) within the grace period; that covers a loader error (127),
# a blob that cannot open the GPU, and a Wayland/EGL setup failure. The fallback is then run as `CMD ARGS...`
# with the same arguments. A non-zero exit after the grace period is the client's own and is returned as is: the
# caller's watchdog owns restarts, a second path would only hide a crash loop.
#
# The client learns which path it is on from the environment:
#   LUMATV_GFX_PATH=64|32   LUMATV_GFX_LIB=<libmali to dlopen: the blob, or the fallback driver>
#
# Exit status: the client's (64-bit or fallback); 69 when the 64-bit path is unavailable and there is no fallback.
#
# Overrides (environment): LUMATV_GFX64_BUNDLE, LUMATV_GFX64_SHIM, LUMATV_GFX64_GRACE (default 5),
# LUMATV_GFX64_FALLBACK, LUMATV_GFX32_LIB (default /usr/lib/libmali.so.0), LUMATV_GFX64_DISABLE=1 (always fall
# back), LUMATV_HEAP / LUMATV_HEAP_LOG (heap shim). Test hooks: LUMATV_GFX64_LOADER, LUMATV_GFX64_UNAME,
# LUMATV_GFX64_DEV (device directory, default /dev).
HERE=$(cd "$(dirname "$0")" && pwd)
BUNDLE=${LUMATV_GFX64_BUNDLE:-$HERE/build/bundle}
SHIM=${LUMATV_GFX64_SHIM:-$HERE/build/out/heapshim.so}
GRACE=${LUMATV_GFX64_GRACE:-5}
FALLBACK=${LUMATV_GFX64_FALLBACK:-}
GFX32_LIB=${LUMATV_GFX32_LIB:-/usr/lib/libmali.so.0}
DEV=${LUMATV_GFX64_DEV:-/dev}

while [ $# -gt 0 ]; do
  case "$1" in
    --bundle) BUNDLE=$2; shift 2 ;;
    --shim) SHIM=$2; shift 2 ;;
    --grace) GRACE=$2; shift 2 ;;
    --fallback) FALLBACK=$2; shift 2 ;;
    --) shift; break ;;
    *) echo "gfx64: unknown option $1" >&2; exit 64 ;;
  esac
done
[ $# -gt 0 ] || { echo "gfx64: no client given (usage: launch.sh [options] -- CLIENT64 [ARGS...])" >&2; exit 64; }
CLIENT=$1; shift
LOADER=${LUMATV_GFX64_LOADER:-$BUNDLE/lib/ld-linux-aarch64.so.1}

MALI=$(ls "$BUNDLE"/lib/libmali*.so 2>/dev/null | head -n 1)

say() { echo "gfx64: $*" >&2; }

# Echoes why the 64-bit path cannot be used, or nothing when it can.
preflight() {
  [ "${LUMATV_GFX64_DISABLE:-0}" = 1 ] && { echo "disabled by LUMATV_GFX64_DISABLE"; return; }
  case "${LUMATV_GFX64_UNAME:-$(uname -m)}" in aarch64|arm64) ;; *) echo "kernel is not 64-bit ($(uname -m))"; return ;; esac
  [ -x "$LOADER" ] || { echo "no loader at $LOADER (run fetch-blob.sh)"; return; }
  [ -n "$MALI" ] || { echo "no libmali in $BUNDLE/lib (run fetch-blob.sh)"; return; }
  [ -f "$SHIM" ] || { echo "no heap shim at $SHIM (run build.sh)"; return; }
  [ -x "$CLIENT" ] || { echo "client $CLIENT is not executable"; return; }
  [ -c "$DEV/mali0" ] || { echo "no $DEV/mali0"; return; }
  [ -c "$DEV/dri/card0" ] || [ -c "$DEV/dma_buf_te" ] || [ -c "$DEV/dma_buf_unified" ] ||
    { echo "no buffer allocator ($DEV/dri/card0, $DEV/dma_buf_te, $DEV/dma_buf_unified)"; return; }
}

uptime_s() { read -r u _ < /proc/uptime; echo "$u"; }

fall_back() {
  why=$1; code=$2; shift 2
  if [ -z "$FALLBACK" ]; then say "64-bit path unavailable: $why; no fallback given"; exit "$code"; fi
  say "64-bit path unavailable: $why; falling back to LG's 32-bit driver: $FALLBACK"
  LUMATV_GFX_PATH=32 LUMATV_GFX_LIB=$GFX32_LIB
  export LUMATV_GFX_PATH LUMATV_GFX_LIB
  exec $FALLBACK "$@"
}

WHY=$(preflight)
[ -z "$WHY" ] || fall_back "$WHY" 69 "$@"

LUMATV_GFX_PATH=64 LUMATV_GFX_LIB=$MALI
export LUMATV_GFX_PATH LUMATV_GFX_LIB
say "64-bit path: $MALI"

STOPPING=
t0=$(uptime_s)
"$LOADER" --library-path "$BUNDLE/lib" --preload "$SHIM" "$CLIENT" "$@" &
PID=$!
trap 'STOPPING=1; kill -TERM "$PID" 2>/dev/null' TERM INT HUP
wait "$PID"; RC=$?
# `wait` returns early when the trap fires; reap the child and report the termination.
[ -n "$STOPPING" ] && { wait "$PID" 2>/dev/null; exit 143; }
trap - TERM INT HUP

[ "$RC" -eq 0 ] && exit 0
t1=$(uptime_s)
if awk -v a="$t0" -v b="$t1" -v g="$GRACE" 'BEGIN { exit !(b - a < g) }'; then
  [ "$RC" -gt 128 ] && how="signal $((RC - 128))" || how="exit $RC"
  fall_back "client failed to start ($how within ${GRACE}s)" "$RC" "$@"
fi
exit "$RC"
