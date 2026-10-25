#!/bin/sh
# C4's supervisor and luna bridge beside LG's compositor, with nothing at stake: `lumatv-sm --dry-run` registers
# the compositor's bus names with a ".lumatv" suffix, spawns the 64-bit compositor on the private socket
# (offscreen GL), heartbeats it, forwards luna calls both ways, and on a fallback request exits with 75 instead of
# touching the mount or LG's binary. Executed ON THE SET as /tmp/lumatv/sm-dryrun.sh.
#   sm-dryrun.sh run    start it, probe the bus names, run the X2 client, ask for a fallback, report
#   sm-dryrun.sh clean
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
MALI32=/usr/lib/libmali.so.0
STATE=$D/state
OUT=$D/out/sm
export XDG_RUNTIME_DIR=$D WAYLAND_DISPLAY=wayland-lumatv

clean() {
    pkill -f 'bin/lumatv-sm' 2>/dev/null
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    pkill -f 'bin/x2cli' 2>/dev/null
    rm -f $D/wayland-lumatv $D/wayland-lumatv.lock $D/lumatv-control
}

# luna-send hangs without a PTY in a plain SSH exec (HANDOFF); from the set's own shell it is fine, and the
# workstation side (tools/tv-first-light.sh sm) calls these through a PTY as well.
probe() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>&1 | head -c 600; echo; }

case "${1:-help}" in
clean) clean; echo "left running: $(pgrep -fl 'lumatv|x2cli' || echo none)" ;;
run)
    clean
    rm -rf $OUT $STATE; mkdir -p $OUT $STATE
    ln -s $D/lib $STATE/lib; ln -s $D/bin $STATE/bin
    ( $D/bin/lumatv-sm --dry-run --state $STATE > $OUT/sm.log 2>&1 & )
    n=0; while [ ! -S $D/wayland-lumatv ] && [ $n -lt 60 ]; do sleep 0.1; n=$((n+1)); done
    sleep 1
    echo "--- sm.log"; cat $OUT/sm.log
    echo "--- compositor.log"; grep -vE ' debug ' $STATE/compositor.log | head -10
    echo "--- getForegroundAppInfo (no window yet)"; probe com.webos.surfacemanager.lumatv/getForegroundAppInfo '{}'
    echo "--- coverUI getStatus"; probe com.webos.coverUI.lumatv/getStatus '{}'
    timeout 15 $D/bin/x2cli $MALI32 --frames 120 --size 1920x1080 --interval 1 > $OUT/client.log 2>&1 &
    sleep 1.5
    echo "--- getForegroundAppInfo (X2 client mapped)"; probe com.webos.surfacemanager.lumatv/getForegroundAppInfo '{}'
    echo "--- getCurrentState"; probe com.webos.surfacemanager.lumatv/getCurrentState '{}'
    echo "--- subscription push test: getFocusedWindowInfo"; timeout 4 luna-send -n 2 -f luna://com.webos.surfacemanager.lumatv/getFocusedWindowInfo '{"subscribe":true}' 2>&1 | head -c 400; echo
    wait 2>/dev/null
    echo "--- fallback request through the control socket (dry run: sm exits 75)"
    $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D fallback dry-run-test
    sleep 1
    echo "--- sm.log tail"; tail -5 $OUT/sm.log
    echo "--- state files"; ls -la $STATE; cat $STATE/last-fallback 2>/dev/null; echo; cat $STATE/escaped 2>/dev/null; echo
    echo "sm pids left: $(pgrep -f 'bin/lumatv-sm' || echo none); compositor pids left: $(pgrep -f 'bin/lumatv-compositor' || echo none)"
    clean
    ;;
*) sed -n 2,8p $0 ;;
esac
