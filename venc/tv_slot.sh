#!/bin/sh
# The encoder slot, run ON THE SET as root (busybox sh), with the binary beside it:
#
#   scp venc/build/arm64/lumatv-venc venc/tv_slot.sh tv:/tmp/venc/
#   ssh tv 'sh /tmp/venc/tv_slot.sh'            # Home in front, nothing playing, nothing else touching the nodes
#   ssh tv 'sh /tmp/venc/tv_slot.sh --full'     # adds 1080p and the 150-frame runs
#   scp -r tv:/tmp/venc-slot/ docs/research/venc/   # then: python3 venc/tools/decode_check.py docs/research/venc/<dir>
#
# Every step is one small probe run in its own process. A step gets a time budget; when it is over, the
# step is killed and the slot checks that the process is gone. A process that survives SIGKILL for 5 s is
# stuck inside the kernel: the slot writes WEDGED, stops, and the only way out is a cold boot (Nitin's
# action). The first non-zero exit code also stops the slot (after the end-of-slot snapshot). Nothing is
# written outside /tmp; the encoder is powered up for the length of a run and released at its exit.
# Never run anything against /dev/video24 (the V4L2 M2M node): its mmap writes into kernel memory
# (docs/research/venc-result.md, "The first slot").
set -u
D=/tmp/venc-slot/$(date +%Y%m%d-%H%M%S)
B=/tmp/venc/lumatv-venc
mkdir -p "$D"
FULL=0
[ "${1:-}" = "--full" ] && FULL=1
log() { echo "$(date +%H:%M:%S) $*" | tee -a "$D/log.txt"; }
STOP=""
step() { # name, budget seconds, then the probe arguments; stdout+stderr to name.out, exit code to name.rc
    [ -n "$STOP" ] && return 1
    n=$1; t=$2; shift 2
    log "== $n: $B $* (budget ${t}s)"
    "$B" "$@" >"$D/$n.out" 2>&1 &
    pid=$!
    i=0
    while kill -0 "$pid" 2>/dev/null && [ "$i" -lt "$t" ]; do sleep 1; i=$((i+1)); done
    if kill -0 "$pid" 2>/dev/null; then
        log "== $n: over budget, killing"
        kill -KILL "$pid" 2>/dev/null
        j=0
        while kill -0 "$pid" 2>/dev/null && [ "$j" -lt 5 ]; do sleep 1; j=$((j+1)); done
        if kill -0 "$pid" 2>/dev/null; then
            cat "/proc/$pid/stat" "/proc/$pid/wchan" >"$D/$n.wedged" 2>&1; echo >>"$D/$n.wedged"
            cat "/proc/$pid/stack" >>"$D/$n.wedged" 2>&1
            log "== $n: WEDGED: pid $pid survives SIGKILL (see $n.wedged); stopping the slot; cold boot needed"
            echo wedged >"$D/WEDGED"
            STOP=wedged
            return 1
        fi
        wait "$pid" 2>/dev/null
        echo 137 >"$D/$n.rc"
        log "== $n: killed after ${t}s"
        STOP="$n"
        return 1
    fi
    wait "$pid"; rc=$?
    echo "$rc" >"$D/$n.rc"
    tail -n 4 "$D/$n.out" | tee -a "$D/log.txt"
    log "== $n: rc $rc"
    [ "$rc" = 0 ] || STOP="$n"
    return $rc
}
holders() { # who has a device node open
    for p in /proc/[0-9]*; do
        for f in "$p"/fd/*; do
            l=$(readlink "$f" 2>/dev/null) || continue
            case "$l" in */dev/venc|*/dev/video24|*/dev/video60|*/dev/lg/venc0) echo "$(cat "$p"/comm 2>/dev/null) $(basename "$p") $l";; esac
        done
    done
}
snapshot() {
    s=$1
    grep -E 'MemAvailable|CmaFree|CmaTotal' /proc/meminfo >"$D/mem-$s.txt"
    cat /proc/hma/status >"$D/hma-$s.txt" 2>&1
    holders >"$D/holders-$s.txt"
    systemctl --failed --no-legend >"$D/failed-$s.txt" 2>&1
    luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' >"$D/fg-$s.txt" 2>&1
    dmesg | tail -n 200 >"$D/dmesg-$s.txt" 2>&1
}

log "slot $D, binary $B, full=$FULL"
[ -x "$B" ] || { log "no binary at $B"; exit 2; }
snapshot before
if [ -s "$D/holders-before.txt" ]; then cat "$D/holders-before.txt" | tee -a "$D/log.txt"; log "something holds an encoder or tap node; stopping"; exit 3; fi
ls -l /dev/venc /dev/video24 /dev/lg/venc0 /dev/hma /dev/video60 | tee -a "$D/log.txt"

# read-only
step caps 10 caps
step vtcaps 10 vtcaps
# the encoder, one call further each time
step gate 10 gate
step create-320 15 create -w 320 -h 240
step map-320 15 map -w 320 -h 240
step hma-320 10 hma -w 320 -h 240
step one-320 20 synth -w 320 -h 240 -r 30 -n 1 -b 500 -g 30 -o "$D/one-320.h264"
step synth-320 20 synth -w 320 -h 240 -r 30 -n 30 -b 500 -g 30 -o "$D/synth-320.h264" --csv "$D/synth-320.csv"
step create-720 15 create -w 1280 -h 720
step synth-720 30 synth -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/synth-720.h264" --csv "$D/synth-720.csv"
# the tap, one call further each time (Home in front: the osd point shows the UI)
step tapopen-osd 15 tapopen -l osd -w 640 -h 368 --nbuf 2
step vt1-osd 20 vt -l osd -w 640 -h 368 -r 30 -n 1 -b 1000 -g 30 --nbuf 2 -o "$D/vt1-osd.h264"
step vt-osd-720 30 vt -l osd -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/vt-osd-720.h264" --csv "$D/vt-osd-720.csv"
step tapopen-blended 15 tapopen -l blended -w 1280 -h 720
step vt-blended-720 30 vt -l blended -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/vt-blended-720.h264" --csv "$D/vt-blended-720.csv"
if [ "$FULL" = 1 ]; then
    step synth-1080 40 synth -w 1920 -h 1080 -r 30 -n 60 -b 8000 -g 30 -o "$D/synth-1080.h264" --csv "$D/synth-1080.csv"
    step synth-720-max 40 synth -w 1280 -h 720 -r 240 -n 150 -b 4000 -g 30 -o "$D/synth-720-max.h264" --csv "$D/synth-720-max.csv"
    step synth-1080-max 40 synth -w 1920 -h 1080 -r 240 -n 150 -b 8000 -g 30 -o "$D/synth-1080-max.h264" --csv "$D/synth-1080-max.csv"
    step vt-blended-1080 40 vt -l blended -w 1920 -h 1080 -r 30 -n 150 -b 8000 -g 30 -o "$D/vt-blended-1080.h264" --csv "$D/vt-blended-1080.csv"
    step vt-display-720 30 vt -l display -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/vt-display-720.h264" --csv "$D/vt-display-720.csv"
fi

[ -n "$STOP" ] && log "stopped at step $STOP"
snapshot after
diff "$D/failed-before.txt" "$D/failed-after.txt" >"$D/failed-diff.txt" 2>&1 || log "NEW FAILED UNITS: $(cat "$D/failed-diff.txt")"
dmesg | grep -i -E 'venc|vt_|vtg|capture|hma|lowpower.*venc|v4l2' | tail -n 120 >"$D/dmesg-venc.txt" 2>&1
holders >"$D/holders-end.txt"; [ -s "$D/holders-end.txt" ] && log "NODES STILL HELD: $(cat "$D/holders-end.txt")"
ls -la "$D" | tee -a "$D/log.txt"
log "done; pull $D"
echo done >"$D/done"
[ -z "$STOP" ]
