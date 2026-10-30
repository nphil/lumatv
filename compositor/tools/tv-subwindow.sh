#!/bin/sh
# The sub window beside an app's video (docs/research/multiview-result.md, M2 and M3), run ON THE SET with LumaTV's compositor in front and an
# app's video playing on MAIN (YouTube): the installed feeder opens its video window, the claim is made, the feeder loads the clip in LG's
# starfish pipeline asking for the sub sink, and after ten seconds everything that says where the video went is written to $OUT. Reversible:
# the claim is released, the feeder killed and the compositor's foreground setting put back at the end (or `tv-subwindow.sh stop`). Nothing
# under /usr or the root filesystem is written.
#
#   tv-subwindow.sh run [LABEL]      the check (LABEL names the files; default m3)
#   tv-subwindow.sh stop             end it early
#
# SUBWINDOW_APP is the app the pipeline is opened for: `front` (the default) is the app whose video is on MAIN, which SAM already lists in
# front; any other id is listed beside it by the compositor (`ctl vwin fg ID`) for the run. Home is refused: LG's AV connector opens Home's
# media pipeline on the main window in pip mode whatever sink it asks for (M2).
#
# Pass: videooutput-status.json lists SUB0 connected for the pipeline's app at 1280x720, 30 a second, with the app's video on MAIN unchanged;
# vwin.json shows the claim placed with sink SUB0; feeder.jsonl has fed 30 a second and dropped 0; messages.txt has
# `_changeSinkNormalWindow current [2:MainVideo], new [4:SubVideo]` and no change back, SUB_SCALER acquired and `[doConnect] ... sinkname:SUB0`.
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
RT=${HWPLANES_RT:-/tmp/xdg}
FEEDER=${SUBWINDOW_FEEDER:-/var/lib/lumatv/bin/lumatv-gfn-feeder}
CLIP=${SUBWINDOW_CLIP:-/tmp/gfn/clips/cam720p30.h264}
APP=org.nphil.lumatv.cameras
SUB_APP=${SUBWINDOW_APP:-front}
label=${2:-m3}
OUT=$D/out/subwindow/$label
ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $RT "$@" 2>/dev/null; }
luna() { timeout 10 luna-send -n 1 "$@" 2>&1; }
say() { echo "subwindow: $*"; }
front_app() { luna luna://com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d ' \n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p'; }

# the compositor's foreground setting as `ctl vwin fg` takes it back: `off`, `APP` or `APP mv`
fg_now() {
    f=$(ctl vwin | tr -d ' \n' | grep -oE '"fg":(null|\{[^}]*\})' | head -1)
    case "$f" in
    '"fg":null' | '') echo off ;;
    *'"multiview":true'*) echo "$(echo "$f" | sed -n 's/.*"app":"\([^"]*\)".*/\1/p') mv" ;;
    *) echo "$f" | sed -n 's/.*"app":"\([^"]*\)".*/\1/p' ;;
    esac
}

stop() {
    pkill -f "lumatv-gfn-feeder.* -a $APP( |\$)" 2> /dev/null
    ctl vwin release all > /dev/null
    if [ -f $D/out/subwindow/fg-before ]; then
        # shellcheck disable=SC2046
        ctl vwin fg $(cat $D/out/subwindow/fg-before) > /dev/null
        rm -f $D/out/subwindow/fg-before
    fi
    say "stopped: the feeder killed, the claims released, the foreground setting $(fg_now)"
}

run() {
    [ -x "$FEEDER" ] || { say "no $FEEDER (lumatv feeder install)"; exit 1; }
    [ -f "$CLIP" ] || { say "no clip $CLIP (tv_hwplanes.py put-clip)"; exit 1; }
    front=$(front_app)
    [ "$SUB_APP" = front ] && SUB_APP=$front
    case "$SUB_APP" in
    '') say "no app in front"; exit 1 ;;
    com.webos.app.home) say "Home's media pipeline never takes the sub window (LG's AV connector puts it on MAIN in pip mode); name another app"; exit 1 ;;
    esac
    mkdir -p "$OUT"
    ctl vwin apps > /dev/null
    fg_now > $D/out/subwindow/fg-before
    # the app in front is on SAM's list already; another app is listed beside it by the compositor while the claim holds the sub window
    if [ "$SUB_APP" = "$front" ]; then ctl vwin fg off > /dev/null; else ctl vwin fg "$SUB_APP" > /dev/null; fi
    say "front $front, pipeline for $SUB_APP, compositor's list: $(fg_now) (was $(cat $D/out/subwindow/fg-before))"
    luna luna://com.webos.service.videooutput/getStatus '{}' > "$OUT/videooutput-before.json"
    pkill -f "lumatv-gfn-feeder.* -a $APP( |\$)" 2> /dev/null
    sleep 0.5
    # the window first, the load four seconds later: the claim in between lists the pipeline's app before the pipeline exists
    XDG_RUNTIME_DIR=$RT WAYLAND_DISPLAY=wayland-0 $FEEDER --play "$CLIP" --size 1280x720 --codec h264 --fps 30 --seconds 40 --loop -w video -a $APP \
        --ndl-app "$SUB_APP" --pipe smp --sink sub_video --delay-load 4000 --stats "$OUT/feeder.jsonl" > "$OUT/feeder.log" 2>&1 &
    say "feeder started: pid $!"
    sleep 1.5
    ctl vwin claim "$label" pip 1300 60 560 315 $APP h264 1280x720@30 > "$OUT/claim.json"
    say "claim: $(grep -oE '"state":"[a-z]*"|"slot":"[a-z]*"|"reason":"[^"]*"' "$OUT/claim.json" | tr '\n' ' ')"
    sleep 1
    luna luna://com.webos.applicationManager/getForegroundApps '{}' > "$OUT/sam-foreground-apps.json"
    say "SAM's foreground apps (what the media server reads): $(grep -oE '"appId":"[^"]*"' "$OUT/sam-foreground-apps.json" | tr '\n' ' ')"
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
    say "sink choice: $(grep -oE 'current \[[0-9]:[A-Za-z]+\], new \[[0-9]:[A-Za-z]+\]|sinkname:[A-Z0-9]+[^,]*, attribute:[^,]*, winModeSet:[0-9], winMode:[a-z_]*|resource":"SUB_SCALER"[^]]*|set winMode\([a-z]*\) automatically|ignore sinkInfo[^"]*|Scaler Resource Allocation Failed|policy action[^"]*' "$OUT/messages.txt" | sort | uniq -c | tr '\n' ';')"
    stop
    luna luna://com.webos.service.videooutput/getStatus '{}' > "$OUT/videooutput-after.json"
    say "after: $(grep -oE '"sink":"[A-Z0-9]*"|"connected":(true|false)|"appId":"[^"]*"' "$OUT/videooutput-after.json" | tr '\n' ' ')"
    say "files in $OUT"
}

case "${1:-help}" in
run) run ;;
stop) stop ;;
*) sed -n '2,17p' "$0" ;;
esac
