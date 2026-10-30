#!/bin/sh
# The sub window beside an app's video (docs/research/multiview-result.md, M2), run ON THE SET with LumaTV's compositor in front and an app's
# video playing on MAIN (YouTube at 1080p): the installed feeder opens its video window, the claim is made, the feeder loads the clip in LG's
# starfish pipeline asking for the sub sink, and after ten seconds everything that says where the video went is written to $OUT. Reversible:
# the claim is released and the feeder killed at the end (or `tv-hwplanes.sh stop`). Nothing under /usr or the root filesystem is written.
#
#   tv-subwindow.sh run [LABEL]      the check (LABEL names the files; default m2)
#   tv-subwindow.sh stop             end it early
#
# Pass: videooutput-status.json lists SUB0 connected for com.webos.app.home at 1280x720, 30 a second, with the app's video on MAIN unchanged;
# vwin.json shows the claim placed with sink SUB0; feeder.jsonl has fed 30 a second and dropped 0; messages.txt has `sinkType[4]`,
# `_changeSinkNormalWindow current [2:MainVideo], new [4:SubVideo]`, SUB_SCALER acquired and `[doConnect] ... sinkname:SUB0`.
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
RT=${HWPLANES_RT:-/tmp/xdg}
FEEDER=${SUBWINDOW_FEEDER:-/var/lib/lumatv/bin/lumatv-gfn-feeder}
CLIP=${SUBWINDOW_CLIP:-/tmp/gfn/clips/cam720p30.h264}
APP=org.nphil.lumatv.cameras
SUB_APP=${SUBWINDOW_APP:-com.webos.app.home}
label=${2:-m2}
OUT=$D/out/subwindow/$label
ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $RT "$@" 2>/dev/null; }
luna() { timeout 10 luna-send -n 1 "$@" 2>&1; }
say() { echo "subwindow: $*"; }

stop() {
    pkill -f "lumatv-gfn-feeder.* -a $APP( |\$)" 2> /dev/null
    ctl vwin release all > /dev/null
    say "stopped: the feeder killed, the claims released"
}

run() {
    [ -x "$FEEDER" ] || { say "no $FEEDER (lumatv feeder install)"; exit 1; }
    [ -f "$CLIP" ] || { say "no clip $CLIP (tv_hwplanes.py put-clip)"; exit 1; }
    mkdir -p "$OUT"
    ctl vwin apps > /dev/null
    say "vwin before: $(ctl vwin | grep -oE '"fg":(null|\{[^}]*\})' | head -1); mode $(ctl vwin | grep -oE '"mode":"[a-z]*"' | head -1)"
    luna luna://com.webos.service.videooutput/getStatus '{}' > "$OUT/videooutput-before.json"
    pkill -f "lumatv-gfn-feeder.* -a $APP( |\$)" 2> /dev/null
    sleep 0.5
    # the window first, the load four seconds later: the claim in between is what puts the sub app on the foreground list before the pipeline exists
    XDG_RUNTIME_DIR=$RT WAYLAND_DISPLAY=wayland-0 $FEEDER --play "$CLIP" --size 1280x720 --codec h264 --fps 30 --seconds 40 --loop -w video -a $APP \
        --ndl-app "$SUB_APP" --pipe smp --sink sub_video --delay-load 4000 --stats "$OUT/feeder.jsonl" > "$OUT/feeder.log" 2>&1 &
    say "feeder started: pid $!"
    sleep 1.5
    ctl vwin claim "$label" pip 1300 60 560 315 $APP h264 1280x720@30 > "$OUT/claim.json"
    say "claim: $(grep -oE '"state":"[a-z]*"|"slot":"[a-z]*"|"reason":"[^"]*"' "$OUT/claim.json" | tr '\n' ' ')"
    sleep 1
    luna luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$OUT/sam-foreground.json"
    say "SAM's foreground apps: $(grep -oE '"appId":"[^"]*"' "$OUT/sam-foreground.json" | tr '\n' ' ')"
    sleep 9
    luna luna://com.webos.service.videooutput/getStatus '{}' > "$OUT/videooutput-status.json"
    luna luna://com.webos.media/getActivePipelines '{}' > "$OUT/ums-pipelines.json"
    luna luna://com.webos.service.multiviewcontroller/getStatus '{}' > "$OUT/mvc-status.json"
    ctl vwin > "$OUT/vwin.json"
    timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$OUT/display.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}" > /dev/null 2>&1
    grep -a -E 'lumatv-gfn-feeder.*(avc\.|ACQUIRE|_changeSink|sinkType|smp\.api (LOAD|setSink|load)|NL_MEDIA_INFO)|doConnect\]|winMode|policy action|setSubType|released by policy|Scaler Resource' /var/log/messages > "$OUT/messages.txt" 2>/dev/null
    say "sinks: $(grep -oE '"sink":"[A-Z0-9]*"|"connected":(true|false)|"appId":"[^"]*"|"width":[0-9]+|"frameRate":[0-9]+' "$OUT/videooutput-status.json" | tr '\n' ' ')"
    say "claim now: $(grep -oE '"placed":(true|false)|"sink":(null|"[^"]*")|"state":"[a-z]*"' "$OUT/vwin.json" | head -3 | tr '\n' ' ')"
    say "feeder: $(tail -n 1 "$OUT/feeder.jsonl" 2>/dev/null | cut -c1-160)"
    say "sink choice: $(grep -oE 'sinkType\[[0-9]+\]|current \[[0-9]:[A-Za-z]+\], new \[[0-9]:[A-Za-z]+\]|sinkname:[A-Z0-9]+[^,]*, attribute:[^,]*, winModeSet:[0-9], winMode:[a-z_]*|resource":"SUB_SCALER"[^]]*|set winMode\([a-z]*\) automatically|ignore sinkInfo[^"]*|Scaler Resource Allocation Failed' "$OUT/messages.txt" | sort | uniq -c | tr '\n' ';')"
    stop
    luna luna://com.webos.service.videooutput/getStatus '{}' > "$OUT/videooutput-after.json"
    say "after: $(grep -oE '"sink":"[A-Z0-9]*"|"connected":(true|false)' "$OUT/videooutput-after.json" | tr '\n' ' ')"
    say "files in $OUT"
}

case "${1:-help}" in
run) run ;;
stop) stop ;;
*) sed -n '2,13p' "$0" ;;
esac
