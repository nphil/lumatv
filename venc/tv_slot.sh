#!/bin/sh
# The first encoder slot, run ON THE SET as root (busybox sh), with the binary beside it:
#
#   scp venc/build/arm64/lumatv-venc venc/tv_slot.sh tv:/tmp/venc/
#   ssh tv 'sh /tmp/venc/tv_slot.sh'            # Home in front, nothing playing, about 6 minutes
#   ssh tv 'sh /tmp/venc/tv_slot.sh --apps'     # adds the Netflix (protected) and HDMI 2 captures; receiver muted first
#   scp -r tv:/tmp/venc-slot/ docs/research/venc/   # then: python3 venc/tools/decode_check.py docs/research/venc/<dir>
#
# Every step is one probe run that exits by itself; the only state it leaves is files under /tmp/venc-slot.
# Nothing is written outside /tmp. The set's encoder is powered up for the length of a run and released at
# its exit (fd close), the video tap is taken for a run and given back the same way. A step that fails
# stops the dependent steps, never the collection. Read the slot plan in docs/research/venc-result.md first.
set -u
D=/tmp/venc-slot/$(date +%Y%m%d-%H%M%S)
B=/tmp/venc/lumatv-venc
mkdir -p "$D"
APPS=0
[ "${1:-}" = "--apps" ] && APPS=1
log() { echo "$(date +%H:%M:%S) $*" | tee -a "$D/log.txt"; }
run() { # name, then the probe arguments; stdout+stderr to name.out, exit code to name.rc
    n=$1; shift
    log "== $n: $B $*"
    "$B" "$@" >"$D/$n.out" 2>&1; rc=$?
    echo "$rc" >"$D/$n.rc"
    tail -n 3 "$D/$n.out" | tee -a "$D/log.txt"
    log "== $n: rc $rc"
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
snapshot() { # before/after state
    s=$1
    cat /proc/meminfo | grep -E 'MemAvailable|CmaFree|CmaTotal' >"$D/mem-$s.txt"
    cat /proc/hma/status >"$D/hma-$s.txt" 2>&1
    holders >"$D/holders-$s.txt"
    systemctl --failed --no-legend >"$D/failed-$s.txt" 2>&1
    luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' >"$D/fg-$s.txt" 2>&1
    dmesg | tail -n 200 >"$D/dmesg-$s.txt" 2>&1
}

log "slot $D, binary $B, apps=$APPS"
[ -x "$B" ] || { log "no binary at $B"; exit 2; }
snapshot before
cat "$D/holders-before.txt" | tee -a "$D/log.txt"
if [ -s "$D/holders-before.txt" ]; then log "something holds an encoder or tap node; stopping (see holders-before.txt)"; exit 3; fi
ls -l /dev/venc /dev/video24 /dev/lg/venc0 /dev/hma /dev/video60 | tee -a "$D/log.txt"

# 1. read-only
run caps caps || true
run vtcaps vtcaps || true

# 2. the encoder alone, from CPU memory: a 2 s check, then 720p30, 1080p30 and 1080p as fast as it goes
if run synth-720p-short synth -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/synth-720p-short.h264" --csv "$D/synth-720p-short.csv"; then
    run synth-720p30 synth -w 1280 -h 720 -r 30 -n 300 -b 4000 -g 30 -o "$D/synth-720p30.h264" --csv "$D/synth-720p30.csv" || true
    run synth-1080p30 synth -w 1920 -h 1080 -r 30 -n 300 -b 8000 -g 30 -o "$D/synth-1080p30.h264" --csv "$D/synth-1080p30.csv" || true
    run synth-1080p-max synth -w 1920 -h 1080 -r 240 -n 300 -b 8000 -g 30 -o "$D/synth-1080p-max.h264" --csv "$D/synth-1080p-max.csv" || true
    run synth-720p-max synth -w 1280 -h 720 -r 240 -n 300 -b 4000 -g 30 -o "$D/synth-720p-max.h264" --csv "$D/synth-720p-max.csv" || true
    ENC_OK=1
else
    log "the /dev/venc route failed; the V4L2 node is the fallback"
    ENC_OK=0
fi

# 3. the V4L2 M2M node (the fallback route), short
run m2m-720p m2m -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/m2m-720p.h264" --csv "$D/m2m-720p.csv" || true

# 4. the screen: the tap into the encoder, Home in front (the UI)
if [ "$ENC_OK" = 1 ]; then
    run vt-osd-720p vt -l osd -w 1280 -h 720 -r 30 -n 150 -b 4000 -g 30 -o "$D/vt-osd-720p.h264" --csv "$D/vt-osd-720p.csv" || true
    run vt-blended-720p vt -l blended -w 1280 -h 720 -r 30 -n 150 -b 4000 -g 30 -o "$D/vt-blended-720p.h264" --csv "$D/vt-blended-720p.csv" || true
    run vt-blended-1080p vt -l blended -w 1920 -h 1080 -r 30 -n 300 -b 8000 -g 30 -o "$D/vt-blended-1080p.h264" --csv "$D/vt-blended-1080p.csv" || true
    run vt-display-720p vt -l display -w 1280 -h 720 -r 30 -n 60 -b 4000 -g 30 -o "$D/vt-display-720p.h264" --csv "$D/vt-display-720p.csv" || true
fi

# 5. with apps in front (receiver muted by Main first): protected video must come back refused or black
if [ "$APPS" = 1 ] && [ "$ENC_OK" = 1 ]; then
    luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"netflix"}' >/dev/null 2>&1
    sleep 30
    luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' >"$D/fg-netflix.txt" 2>&1
    run vt-netflix-blended vt -l blended -w 1280 -h 720 -r 30 -n 150 -b 4000 -g 30 -o "$D/vt-netflix-blended.h264" --csv "$D/vt-netflix-blended.csv" || true
    luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.hdmi2"}' >/dev/null 2>&1
    sleep 12
    luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' >"$D/fg-hdmi2.txt" 2>&1
    run vt-hdmi2-blended vt -l blended -w 1280 -h 720 -r 30 -n 150 -b 4000 -g 30 -o "$D/vt-hdmi2-blended.h264" --csv "$D/vt-hdmi2-blended.csv" || true
    luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' >/dev/null 2>&1
    sleep 3
    luna-send -n 1 luna://com.webos.applicationManager/closeByAppId '{"id":"netflix"}' >/dev/null 2>&1
    luna-send -n 1 luna://com.webos.applicationManager/closeByAppId '{"id":"com.webos.app.hdmi2"}' >/dev/null 2>&1
fi

snapshot after
diff "$D/failed-before.txt" "$D/failed-after.txt" >"$D/failed-diff.txt" 2>&1 || log "NEW FAILED UNITS: $(cat "$D/failed-diff.txt")"
dmesg | grep -i -E 'venc|vt_|vtg|capture|hma|lowpower.*venc' | tail -n 80 >"$D/dmesg-venc.txt" 2>&1
ls -la "$D" | tee -a "$D/log.txt"
log "done; pull $D"
echo done >"$D/done"
