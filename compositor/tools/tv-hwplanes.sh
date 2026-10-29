#!/bin/sh
# Hardware planes: the TV-round steps of docs/research/hw-planes-result.md, executed ON THE SET as /tmp/lumatv/hwplanes.sh and driven from the workstation by
# tools/tv_hwplanes.py (luna-send needs a PTY). Every step reads, or switches one `ctl` setting for a few seconds and puts it back; nothing is written outside
# /tmp/lumatv/out/hwplanes. The compositor is the live one (runtime dir /tmp/xdg, started by lumatv-sm with the words of /tmp/lumatv/compositor.flags);
# HWPLANES_RT=/tmp/lumatv points the steps at a maintenance window's (tv-kms-window.sh).
#
#   hwplanes.sh read                         the switches, the planes and the memory as they are now
#   hwplanes.sh alpha                        P0: the OSD's constant alpha against the pages' format (needs --page-alpha on for the PASS)
#   hwplanes.sh pointer                      P1/P2: 60 moves of the pointer in the frame, then on the cursor plane: frames drawn, the plane's registers
#   hwplanes.sh afbc                         A1/A2: what the plane and the GPU list, the pages' layout, the registers, a frame time reading
#   hwplanes.sh shot NAME                    LG's DISPLAY and VIDEO captures to out/hwplanes/NAME-display.png and NAME-video.png
#   hwplanes.sh feeder CLIP SIZE CODEC FPS SECS   the feeder in a video window, in the background (V2 to V6)
#   hwplanes.sh vwin                         the claims, the windows, the sinks videooutputd reports
#   hwplanes.sh sizes                        V6: a claim moved through sizes from 114x64 to full screen, a capture each
#   hwplanes.sh stop                         kill the feeder, release every claim, vwin off, pointer gpu
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
RT=${HWPLANES_RT:-/tmp/xdg}
OUT=$D/out/hwplanes
FEEDER=${HWPLANES_FEEDER:-/tmp/gfn/lumatv-gfn-feeder}
APP=org.nphil.lumatv.cameras
mkdir -p $OUT

ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $RT "$@" 2>/dev/null; }
jnum() { grep -oE "\"$1\":-?[0-9.]+" | head -1 | cut -d: -f2; }
jstr() { grep -oE "\"$1\":\"[^\"]*\"" | head -1 | cut -d'"' -f4; }
jbool() { grep -oE "\"$1\":(true|false)" | head -1 | cut -d: -f2; }
# a line of /proc/lg/fbdev/fbN/status by its key (" hdr.xpos: 0", " status.fb_flip_cnt     = 233696")
fb() { sed -n "s/^ *$2 *[:=] *//p" /proc/lg/fbdev/fb$1/status 2>/dev/null | head -1; }
say() { echo "hw: $*"; }
cap() { timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$OUT/$1.png\",\"method\":\"$2\",\"format\":\"PNG\",\"width\":960,\"height\":540}" 2>/dev/null | tr ',' '\n' | grep writtenBytes | tr -dc '0-9'; }
pages_of() { echo "$1" | sed -n 's/.*"pages":{\([^}]*\)}.*/\1/p'; }

step_read() {
    say "flags file: $(cat $D/compositor.flags 2>/dev/null || echo none)"
    say "compositor: $(ctl status | jstr output); drawn $(ctl status | jnum drawn)"
    A=$(ctl afbc)
    P=$(pages_of "$A")
    say "pages: kind=$(echo "$P" | jstr kind) asked=$(echo "$P" | jstr asked) alpha=$(echo "$P" | jbool alpha) bytesEach=$(echo "$P" | jnum bytesEach) fallback=$(echo "$P" | grep -oE '"fallback":(null|"[^"]*")' | head -1)"
    say "afbc switch: $(echo "$A" | jstr mode); pageAlpha: $(echo "$A" | jbool pageAlpha)"
    O=$(ctl pointer)
    say "pointer: mode=$(echo "$O" | jstr mode) carried=$(echo "$O" | jbool carried) route=$(echo "$O" | jstr route) errors=$(echo "$O" | jnum errors) lastError=$(echo "$O" | jstr lastError)"
    V=$(ctl vwin)
    say "vwin: mode=$(echo "$V" | jstr mode) free=$(echo "$V" | jnum free) claims: $(echo "$V" | grep -oE '"state":"[a-z]*"' | tr '\n' ' ')"
    for n in 0 3; do
        say "fb$n: visible=$(fb $n visible) xpos=$(fb $n hdr.xpos) ypos=$(fb $n hdr.ypos) mem=$(fb $n hdr.w_mem)x$(fb $n hdr.h_mem) out=$(fb $n hdr.w_out)x$(fb $n hdr.h_out) global_alpha_en=$(fb $n hdr.global_alpha_en) flips=$(fb $n status.fb_flip_cnt)"
    done
    say "memory: $(grep -E 'CmaFree|MemAvailable' /proc/meminfo | tr -s ' ' | tr '\n' ' ')"
}

step_alpha() {
    A=$(ctl afbc)
    want=$(echo "$A" | jbool pageAlpha)
    en=$(fb 0 hdr.global_alpha_en)
    say "P0 pageAlpha=$want fb0 hdr.global_alpha_en=$en hdr.global_alpha=$(fb 0 hdr.global_alpha)"
    if [ "$want" = true ] && [ "$en" = 0 ]; then say "P0 PASS: the pages carry alpha and the OSD blends per pixel"
    elif [ "$want" = false ] && [ "$en" = 1 ]; then say "P0 BASELINE: XR24 pages, the OSD's constant alpha on (a hole is opaque black)"
    else say "P0 FAIL: the register does not follow the page format"; fi
}

moves() { # N: N moves of the pointer along a diagonal, the last at (300+20*(N-1), 200+10*(N-1))
    i=0
    while [ $i -lt $1 ]; do ctl pointer $((300 + i * 20)) $((200 + i * 10)) > /dev/null; i=$((i + 1)); done
}

step_pointer() {
    ctl pointer gpu > /dev/null
    ctl pointer 300 200 > /dev/null
    sleep 1
    d0=$(ctl status | jnum drawn)
    moves 60
    sleep 0.5
    d1=$(ctl status | jnum drawn)
    say "P1 in the frame: frames drawn for 60 moves: $((d1 - d0)) (expected: about 60, one per move)"
    say "P1 fb3 in the frame: visible=$(fb 3 visible) flips=$(fb 3 status.fb_flip_cnt) (expected: off, 0 more)"
    R=$(ctl pointer plane)
    say "P1 plane switch: ok=$(echo "$R" | jbool ok) $(echo "$R" | jstr error)"
    sleep 1
    ctl pointer 300 200 > /dev/null
    sleep 0.5
    f0=$(fb 3 status.fb_flip_cnt); d2=$(ctl status | jnum drawn)
    moves 60
    sleep 0.5
    d3=$(ctl status | jnum drawn); f1=$(fb 3 status.fb_flip_cnt)
    O=$(ctl pointer)
    say "P1 on the plane: frames drawn for 60 moves: $((d3 - d2)) (expected: 0 to 2)"
    say "P1 fb3 on the plane: visible=$(fb 3 visible) flips +$((f1 - f0)) (expected: on, up to 60 flips) xpos=$(fb 3 hdr.xpos) ypos=$(fb 3 hdr.ypos) mem=$(fb 3 hdr.w_mem)x$(fb 3 hdr.h_mem) out=$(fb 3 hdr.w_out)x$(fb 3 hdr.h_out) global_alpha_en=$(fb 3 hdr.global_alpha_en)"
    say "P1 expected at the last move (1480,790): xpos about 2*(1480-16)=2928, ypos about 2*(790-16)=1548 if the driver scales by 2 and takes the hotspot off"
    say "P1 plane report: route=$(echo "$O" | jstr route) shows=$(echo "$O" | jnum shows) moves=$(echo "$O" | jnum moves) hides=$(echo "$O" | jnum hides) coalesced=$(echo "$O" | jnum coalesced) errors=$(echo "$O" | jnum errors) broken=$(echo "$O" | grep -oE '"broken":(true|false)' | head -1) lastError=$(echo "$O" | jstr lastError)"
    cap pointer-plane DISPLAY > /dev/null
    say "P2 the DISPLAY capture with the pointer on the plane is out/hwplanes/pointer-plane.png (the pointer is in it only if the capture tap is after the cursor plane)"
    ctl pointer 100 100 > /dev/null
    sleep 0.5
    say "P1 fb3 at (100,100): xpos=$(fb 3 hdr.xpos) ypos=$(fb 3 hdr.ypos) (expected 168 and 168 for the same rule: the hotspot is taken off, then x2)"
    ctl pointer gpu > /dev/null
    sleep 0.5
    say "P1 back on the GPU: visible=$(fb 3 visible) carried=$(ctl pointer | jbool carried)"
}

step_afbc() {
    ctl afbc > $OUT/afbc.json
    A=$(cat $OUT/afbc.json)
    P=$(pages_of "$A")
    say "A1 pages: kind=$(echo "$P" | jstr kind) asked=$(echo "$P" | jstr asked) format=$(echo "$P" | jstr format) modifier=$(echo "$P" | jstr modifier) bytesEach=$(echo "$P" | jnum bytesEach) count=$(echo "$P" | jnum count) fallback=$(echo "$P" | grep -oE '"fallback":(null|"[^"]*")' | head -1)"
    say "A1 the plane and the GPU driver as listed: out/hwplanes/afbc.json (probe)"
    sed -n '1,200p' /proc/lg/fbdev/fb0/status > $OUT/fb0-status.txt 2>/dev/null
    say "A1 fb0: global_alpha_en=$(fb 0 hdr.global_alpha_en) w_mem=$(fb 0 hdr.w_mem) h_mem=$(fb 0 hdr.h_mem) flips=$(fb 0 status.fb_flip_cnt) (the whole status is out/hwplanes/fb0-status.txt: look for an AFBC field)"
    say "A1 frame time: renderUsLast=$(ctl status | jnum renderUsLast) renderUsMax=$(ctl status | jnum renderUsMax) drawn=$(ctl status | jnum drawn)"
    say "A2 bandwidth: estimate only (assumed ratios, afbc.json 'estimate'); no memory-bandwidth counter has been read on this set"
    cap afbc DISPLAY > /dev/null
    say "A2 capture: out/hwplanes/afbc.png (compare with a linear run: tv_hwplanes.py compare A.png B.png)"
}

step_shot() {
    n=$(cap "$1-display" DISPLAY)
    v=$(cap "$1-video" VIDEO)
    say "shot $1: display $n bytes, video $v bytes"
}

step_feeder() {
    clip=$1; size=$2; codec=$3; fps=$4; secs=$5
    [ -x $FEEDER ] || { say "no $FEEDER: upload gfn/feeder/build/arm/lumatv-gfn-feeder"; exit 1; }
    [ -f "$clip" ] || { say "no clip $clip"; exit 1; }
    pkill -f lumatv-gfn-feeder 2> /dev/null
    sleep 0.5
    XDG_RUNTIME_DIR=$RT WAYLAND_DISPLAY=wayland-0 $FEEDER --play "$clip" --size "$size" --codec "$codec" --fps "$fps" --seconds "$secs" --loop -w video -a $APP --stats $OUT/feeder-stats.jsonl > $OUT/feeder.log 2>&1 &
    say "feeder started: pid $! ($clip $size $codec at $fps for $secs s, window type video, app $APP); log out/hwplanes/feeder.log"
}

step_vwin() {
    ctl vwin > $OUT/vwin.json
    V=$(cat $OUT/vwin.json)
    say "vwin: mode=$(echo "$V" | jstr mode) free=$(echo "$V" | jnum free) apps=$(echo "$V" | jnum apps) wall=$(echo "$V" | jbool wall) subscribed=$(echo "$V" | jbool subscribed)"
    say "vwin claims: $(echo "$V" | grep -oE '"claims":\[[^]]*\]' | head -1)"
    say "vwin windows: $(echo "$V" | grep -oE '"windows":\[[^]]*\]' | head -1)"
    timeout 10 luna-send -n 1 luna://com.webos.service.videooutput/getStatus '{}' > $OUT/videooutput-status.json 2>&1
    say "videooutputd sinks: $(grep -oE '"sink":"[A-Z0-9]*"|"connected":(true|false)|"subScalerOccupied":(true|false)|"zOrder":[0-9]+' $OUT/videooutput-status.json | tr '\n' ' ')"
    say "feeder: $(tail -c 300 $OUT/feeder-stats.jsonl 2>/dev/null | tr '\n' ' ')"
}

step_sizes() {
    for r in 114x64 160x90 320x180 640x360 1280x720 1920x1080; do
        w=${r%x*}; h=${r#*x}
        x=$(( (1920 - w) / 2 )); y=$(( (1080 - h) / 2 ))
        ctl vwin move t1 $x $y $w $h > /dev/null
        sleep 2
        c=$(cap "size-$r" DISPLAY)
        say "V6 $r at $x,$y: capture $c bytes; $(ctl vwin | grep -oE '"placed":(true|false)' | head -1); feeder: $(tail -c 160 $OUT/feeder-stats.jsonl 2>/dev/null | tr '\n' ' ')"
    done
}

step_stop() {
    pkill -f lumatv-gfn-feeder 2> /dev/null
    ctl vwin release all > /dev/null
    ctl vwin off > /dev/null
    ctl pointer gpu > /dev/null
    say "stopped: feeder killed, claims released, vwin off, pointer on the GPU"
}

case "${1:-help}" in
read) step_read ;;
alpha) step_alpha ;;
pointer) step_pointer ;;
afbc) step_afbc ;;
shot) step_shot "${2:-shot}" ;;
feeder) shift; step_feeder "$@" ;;
vwin) step_vwin ;;
sizes) step_sizes ;;
stop) step_stop ;;
*) sed -n 2,19p $0 ;;
esac
