#!/bin/sh
# The UI engine's frames under LumaTV's compositor, measured on the set: what an idle frame costs the GPU (the RustUi spike's
# prerequisite: under 1 ms for a 10 fps frame), what an animated one costs, the system layer coming and going, and direct
# scan-out of a full-screen client with the UI attached. Executed ON THE SET as /tmp/lumatv/ui-measure.sh (busybox sh), driven
# from the workstation by tools/tv_ui_measure.py, inside the same maintenance window as kms-window.sh: LG's compositor is
# stopped, LumaTV's compositor is DRM master and draws the demo surface (--ui-demo) on the primary plane, then LG's compositor
# is started again. About 2.5 minutes; the panel shows the demo, then black until LG's Home is back.
#
#   ui-measure.sh run       the four phases below, then restore
#   ui-measure.sh restore   only start LG's compositor again (if a run was interrupted)
#
# Phases (each its own compositor run, --seconds long; LUMATV_UI_DEMO_HZ paces the demo's ticker, LUMATV_UI_DEMO_VOLUME_S its
# volume bar):
#   idle10   30 s  ticker at 10 Hz, nothing else: the idle gate (GPU busy share x 100 ms = GPU ms per frame, under 1 ms passes)
#   anim60   20 s  ticker at 60 Hz: an animated frame every refresh
#   volume   24 s  ticker at 10 Hz, the volume bar up 2 s in every 6: the system layer's frames
#   direct   30 s  x2cli full screen (LG's libmali, AFBC dma-bufs) over the 10 Hz ticker, the bar up 2 s in every 10: direct
#                  scan-out while the bar is down, composition while it is up
# For each phase /tmp/lumatv/out/ui/ gets PHASE.server.log, PHASE.gpu (one line a second: uptime, mali busy_time, idle_time,
# the compositor's utime+stime ticks), PHASE.status.json (ctl status near the end, with the summary's "ui frames ..." and
# "direct N"), and PHASE.png (ctl capture: the page last composed). The workstation side reads them (tv_ui_measure.py).
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
MALI32=/usr/lib/libmali.so.0
SRV="$LD --library-path $LIB $D/bin/lumatv-compositor --runtime-dir $D"
CTL="$LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D"
export XDG_RUNTIME_DIR=$D WAYLAND_DISPLAY=wayland-lumatv LUMATV_LOG=info
OUT=$D/out/ui
GPU=/sys/kernel/debug/mali0/dvfs_utilization

. "$D/compositor.sh"
restore() {
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    pkill -f 'bin/x2cli' 2>/dev/null
    sleep 0.5
    systemctl start surface-manager-daemon
    echo "restore: surface-manager-daemon started ($(systemctl is-active surface-manager-daemon))"
    sh $D/recover-home.sh
}

# sample NAME: one line a second into NAME.gpu until the compositor is gone
sample() {
    while pid=$(pgrep -f 'bin/lumatv-compositor' | head -1) && [ -n "$pid" ]; do
        cpu=$(awk '{print $14 + $15}' /proc/$pid/stat 2>/dev/null)
        echo "$(cut -d' ' -f1 /proc/uptime) $(tr '\n' ' ' < $GPU 2>/dev/null) cpu=$cpu"
        sleep 1
    done
}

# phase NAME SECONDS HZ VOLUME_S [client...]: one compositor run with the demo, the sampler beside it, status and capture 5 s
# before the end, the client (if any) started 2 s in
phase() {
    name=$1; secs=$2; hz=$3; vol=$4; shift 4
    echo "== $name: $secs s, ticker $hz Hz, volume every ${vol:-never} s${1:+, client: $*}"
    ( env LUMATV_UI_DEMO_HZ=$hz ${vol:+LUMATV_UI_DEMO_VOLUME_S=$vol} $SRV --renderer kms --driver $BLOB --buffers own --card /dev/dri/card0 \
        --ui-demo --seconds "$secs" > $OUT/$name.server.log 2>&1 & )
    n=0; while [ ! -S $D/wayland-lumatv ] && [ $n -lt 50 ]; do sleep 0.1; n=$((n+1)); done
    if [ ! -S $D/wayland-lumatv ]; then echo "$name: the compositor did not start"; sed -n 1,40p $OUT/$name.server.log; return 1; fi
    sample > $OUT/$name.gpu &
    SAMPLER=$!
    if [ $# -gt 0 ]; then
        sleep 2
        ( timeout $((secs - 2)) "$@" > $OUT/$name.client.log 2>&1 & )
    fi
    sleep $((secs - 5))
    $CTL status > $OUT/$name.status.json 2>&1
    $CTL capture $OUT/$name.png > /dev/null 2>&1
    n=0; while pgrep -f 'bin/lumatv-compositor' >/dev/null && [ $n -lt 30 ]; do sleep 0.5; n=$((n+1)); done
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    pkill -f 'bin/x2cli' 2>/dev/null
    wait $SAMPLER 2>/dev/null
    comp_current_log >> $OUT/$name.server.log
    grep -E 'summary|error|warn|gl error' $OUT/$name.server.log | grep -v 'ld.so.preload' | tail -8
    sleep 1
}

case "${1:-help}" in
restore) restore ;;
run)
    if ! grep -q ui $D/FEATURES 2>/dev/null; then echo "the bundle was not built with the ui feature (LUMATV_FEATURES=ui); nothing run"; exit 2; fi
    [ -r $GPU ] || echo "warning: $GPU is not readable; no GPU figures"
    rm -rf $OUT; mkdir -p $OUT
    echo "before: surface-manager $(pgrep -x surface-manager | head -1) active=$(systemctl is-active surface-manager-daemon)"
    grep -E 'MemAvailable|CmaFree' /proc/meminfo
    trap restore EXIT INT TERM
    systemctl stop surface-manager-daemon
    sleep 1
    echo "stopped: surface-manager pids: $(pgrep -x surface-manager || echo none)"
    phase idle10 30 10 ""
    phase anim60 20 60 ""
    phase volume 24 10 6
    phase direct 30 10 10 $D/bin/x2cli $MALI32 --frames 1500 --size 1920x1080 --interval 1
    trap - EXIT INT TERM
    restore
    ;;
*) sed -n 2,20p $0 ;;
esac
