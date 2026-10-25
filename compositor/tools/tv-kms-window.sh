#!/bin/sh
# C3 on the screen: a maintenance window in which LG's compositor is stopped, LumaTV's compositor becomes DRM
# master of card0 and scans its own composition out on the primary plane, then LG's compositor is started again.
# Executed ON THE SET as /tmp/lumatv/kms-window.sh (busybox sh), driven from the workstation by
# tools/tv-first-light.sh kms. Takes about 60 s; the panel is black between LG's compositor stopping and
# LumaTV's first frame, and again until LG's Home is back. Apps running at the time are restarted by LG as after
# any compositor restart (the mod does one on every boot); SAM is never touched.
#
#   kms-window.sh run      stop LG's compositor, run the window, start LG's compositor, sweep the crash report
#   kms-window.sh restore  only start LG's compositor again (if a run was interrupted)
# Logs and captures: /tmp/lumatv/out/kms/. Expected results: docs/research/compositor-c2-result.md, "C3 status".
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
MALI32=/usr/lib/libmali.so.0
SRV="$LD --library-path $LIB $D/bin/lumatv-compositor --runtime-dir $D --log-protocol"
CLI64="$LD --library-path $LIB $D/bin/lumatv-testclient"
export XDG_RUNTIME_DIR=$D WAYLAND_DISPLAY=wayland-lumatv LUMATV_LOG=debug
OUT=$D/out/kms

restore() {
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    pkill -f 'bin/x2cli' 2>/dev/null
    pkill -f 'bin/lumatv-testclient' 2>/dev/null
    sleep 0.5
    systemctl start surface-manager-daemon
    echo "restore: surface-manager-daemon started ($(systemctl is-active surface-manager-daemon))"
    # WebAppMgr hit its start limit while the compositor was away and the web Home cannot launch until it is
    # reset; recover-home.sh does that, launches Home and sweeps the librdx report LG's inputcommon leaves.
    sh $D/recover-home.sh
}

case "${1:-help}" in
restore) restore ;;
run)
    rm -rf $OUT; mkdir -p $OUT
    echo "before: surface-manager $(pgrep -x surface-manager | head -1) active=$(systemctl is-active surface-manager-daemon)"
    grep -E 'MemAvailable|CmaFree' /proc/meminfo
    trap restore EXIT INT TERM
    systemctl stop surface-manager-daemon
    sleep 1
    echo "stopped: surface-manager pids: $(pgrep -x surface-manager || echo none); DRM holders: $(ls -l /proc/*/fd 2>/dev/null | grep -c card0)"
    # Case A: the 64-bit shm client, 600 frames at the tick (10 s; the X2 colour pattern saturates to ff8000 after
    # frame 10, so the panel holds one colour long enough to capture); composition into the dumb-buffer ring,
    # in-fence commits, flip timestamps into the pacer. A capture (8 MB read back through the CPU mapping) costs
    # the frame it lands on, so only every 200th frame is captured and the client's callback gaps stay honest.
    ( $SRV --renderer kms --driver $BLOB --buffers own --card /dev/dri/card0 --seconds 40 --exit-on-disconnect --capture-dir $OUT/a --capture-every 200 > $OUT/a-server.log 2>&1 & )
    n=0; while [ ! -S $D/wayland-lumatv ] && [ $n -lt 50 ]; do sleep 0.1; n=$((n+1)); done
    if [ ! -S $D/wayland-lumatv ]; then echo "A: server did not start"; sed -n 1,40p $OUT/a-server.log; exit 1; fi
    sleep 0.5
    timeout 30 $CLI64 --frames 600 --size 1920x1080 > $OUT/a-client.log 2>&1 &
    CLIPID=$!
    sleep 3
    touch $OUT/a-ready   # the workstation side captures the panel now (tools/tv_kms.py)
    sleep 4
    $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D status > $OUT/a-status.json 2>&1
    wait $CLIPID; echo "A: client exit $?"
    n=0; while pgrep -f 'bin/lumatv-compositor' >/dev/null && [ $n -lt 10 ]; do sleep 0.5; n=$((n+1)); done
    pkill -f 'bin/lumatv-compositor' 2>/dev/null; sleep 0.5
    grep -E 'kms:|direct:|egl:|error|warn|summary' $OUT/a-server.log | grep -v 'ld.so.preload' | head -30
    # Case B: LG's own client (32-bit libmali, AFBC dma-bufs): the direct scan-out attempt (ADDFB2 with modifier
    # 0x0800000000000151). "direct: fb N for ..." means the plane took it; "ADDFB2 refused" means composition.
    ( $SRV --renderer kms --driver $BLOB --buffers own --card /dev/dri/card0 --seconds 30 --exit-on-disconnect --capture-dir $OUT/b --capture-every 200 > $OUT/b-server.log 2>&1 & )
    n=0; while [ ! -S $D/wayland-lumatv ] && [ $n -lt 50 ]; do sleep 0.1; n=$((n+1)); done
    sleep 0.5
    timeout 30 $D/bin/x2cli $MALI32 --frames 600 --size 1920x1080 --interval 1 > $OUT/b-client.log 2>&1 &
    CLIPID=$!
    sleep 3
    touch $OUT/b-ready
    sleep 4
    $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D status > $OUT/b-status.json 2>&1
    wait $CLIPID; echo "B: client exit $?"
    n=0; while pgrep -f 'bin/lumatv-compositor' >/dev/null && [ $n -lt 10 ]; do sleep 0.5; n=$((n+1)); done
    pkill -f 'bin/lumatv-compositor' 2>/dev/null; sleep 0.5
    grep -E 'kms:|direct:|egl:|error|warn|summary' $OUT/b-server.log | grep -v 'ld.so.preload' | head -30
    trap - EXIT INT TERM
    restore
    ;;
*) sed -n 2,12p $0 ;;
esac
