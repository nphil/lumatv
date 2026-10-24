#!/bin/sh
# X2 run matrix on the set. Each run: private server (exits when the client disconnects or after 10 s), then the
# 32-bit libmali client for a few frames. Logs land in /tmp/x2/<run>.srv.log and <run>.cli.log.
cd /tmp/x2 || exit 1
MALI=/usr/lib/libmali.so.0
export XDG_RUNTIME_DIR=/tmp/x2 WAYLAND_DISPLAY=wayland-x2
run() {
  name=$1; shift; srvargs=$1; shift; cliargs=$*
  rm -f wayland-x2 wayland-x2.lock
  ( ./x2srv --seconds 10 $srvargs > $name.srv.log 2>&1 & )
  sleep 0.4
  timeout 8 ./x2cli $MALI $cliargs > $name.cli.log 2>&1
  echo "$name: client exit $?"
  n=0; while pgrep -x x2srv >/dev/null && [ $n -lt 12 ]; do sleep 0.5; n=$((n+1)); done
  pkill -x x2srv 2>/dev/null
  echo "  $(grep summary $name.srv.log)"
}
run r1-all ""                                   --frames 6 --size 256x256
run r2-nomali "--no-mali"                        --frames 6 --size 256x256
run r3-dmabuf-only "--no-mali --no-drm"          --frames 6 --size 256x256
run r4-none "--no-mali --no-drm --no-dmabuf"     --frames 6 --size 256x256
run r5-egl "--egl $MALI"                         --frames 6 --size 256x256
run r6-all-1080p ""                              --frames 4 --size 1920x1080
run r7-afbc "--afbc"                             --frames 4 --size 256x256
# 64-bit importer: Rockchip's g24p0 blob (same mali_buffer_sharing v5 tables) serving LG's 32-bit client
M64=/tmp/m64; BLOB=$M64/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so
if [ -x $M64/x2srv64 ]; then
  run64() {
    name=$1; shift; srvargs=$1; shift; cliargs=$*
    rm -f wayland-x2 wayland-x2.lock
    ( $M64/lib/ld-linux-aarch64.so.1 --library-path $M64/lib $M64/x2srv64 --seconds 10 $srvargs > $name.srv.log 2>&1 & )
    sleep 0.8
    timeout 8 ./x2cli $MALI $cliargs > $name.cli.log 2>&1
    echo "$name: client exit $?"
    n=0; while pgrep -x x2srv64 >/dev/null && [ $n -lt 12 ]; do sleep 0.5; n=$((n+1)); done
    pkill -x x2srv64 2>/dev/null
    echo "  $(grep summary $name.srv.log)"
  }
  run64 r8-egl64-render "--egl $BLOB --drm-device /dev/dri/renderD128"  --frames 6 --size 256x256
  run64 r9-egl64-card0  "--egl $BLOB --drm-device /dev/dri/card0"       --frames 6 --size 256x256
fi
pkill -x x2srv 2>/dev/null; pkill -x x2srv64 2>/dev/null; pkill -x x2cli 2>/dev/null
echo "left running: $(pgrep -l 'x2srv|x2cli' || echo none)"
grep -E 'CmaFree|MemAvailable' /proc/meminfo
