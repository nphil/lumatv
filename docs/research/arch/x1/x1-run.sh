#!/bin/sh
# X1 floor probe on the set: a trivial 64-bit client of LG's compositor, full-screen overlay over whatever is
# showing, 60 Hz for X1_SECONDS (default 60). Needs /tmp/m64 (64-bit blob + Debian arm64 libs) and /tmp/x1.
# Usage: sh /tmp/x1/x1-run.sh [seconds] [dumb|te|auto]   (log: /tmp/x1/x1-<tag>.log)
M64=/tmp/m64
SECS=${1:-60}; HEAP=${2:-auto}; TAG=$(date +%H%M%S)
export XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0
export X1_SECONDS=$SECS X1_WINDOW_TYPE=${X1_WINDOW_TYPE:-_WEBOS_WINDOW_TYPE_OVERLAY}
[ "$HEAP" != auto ] && export X1_HEAP=$HEAP
echo "x1: start $(date +%T) uptime $(cut -d' ' -f1 /proc/uptime) CmaFree $(grep CmaFree /proc/meminfo | awk '{print $2}') kB heap=$HEAP secs=$SECS" | tee /tmp/x1/x1-$TAG.log
$M64/lib/ld-linux-aarch64.so.1 --library-path $M64/lib --preload /tmp/x1/x1heapshim.so /tmp/x1/x1client $M64/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so 2>&1 | tee -a /tmp/x1/x1-$TAG.log
echo "x1: end $(date +%T) uptime $(cut -d' ' -f1 /proc/uptime) CmaFree $(grep CmaFree /proc/meminfo | awk '{print $2}') kB exit ${PIPESTATUS:-?}" | tee -a /tmp/x1/x1-$TAG.log
pgrep -l x1client && pkill -x x1client
echo "x1: log /tmp/x1/x1-$TAG.log"
