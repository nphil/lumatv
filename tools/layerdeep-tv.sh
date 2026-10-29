#!/bin/sh
# The display-layer checks of docs/research/layer-deep-result.md § 6, executed ON THE SET (busybox sh, devmem) as root from a terminal, by
# the owner, LumaTV's compositor live (runtime dir /tmp/xdg; LD_RT=/tmp/lumatv for a maintenance window's). Reads and `ctl` switches first;
# the raw-register steps print their writes, ask for a typed "yes", arm a detached dead-man that runs the step's revert file DEADMAN seconds
# after the first write, and revert on exit as well. Logs and dumps go to /tmp/lumatv/out/layerdeep/.
#
#   layerdeep-tv.sh observe      the switches, the planes, the OSD header, three regdumps a second apart and their diff, LG's orbit steps
#   layerdeep-tv.sh orbit        the OLED care's orbit as the plane's position: care on at radius 1, step 10 s, `ctl scanout offset on`,
#                                readings until a placed frame shows and across one Screen Shift step; the theme file is put back
#   layerdeep-tv.sh pageprobe    `ctl pageprobe` at 1280x720, 1440x810 and 1920x1080: the OSD scaler on a smaller buffer
#   layerdeep-tv.sh sharp        the OSD sharpener's gain at 0x00, 0x40, 0x80 and the sharpener off, 8 s each, then 1 and 0x10 again
#   layerdeep-tv.sh vsp          raw: OSD0's VSP contrast halved, then grayscale, 4 s each (0xCC480360)
#   layerdeep-tv.sh hole         raw: a 960x540 hole with alpha 0 in the middle of B.OSD1's OSD0 input, hole select 0 then 1, 4 s each
#   layerdeep-tv.sh osd01        raw: /tmp/lumatv/osd01-probe.sh all (tools/osd01-probe.sh copied beside this script; its own dead-man), then
#                                the 4K mode bit and the combiner's bypass, 4 s each
#   layerdeep-tv.sh revert       run the last raw step's revert file now
#   -n before a step: dry run, the writes are printed and nothing is written
#
# Environment: DEADMAN (12) seconds until the detached revert; HOLD (4) seconds a raw write stays; OUT; LD_RT.
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
RT=${LD_RT:-/tmp/xdg}
OUT=${OUT:-$D/out/layerdeep}
DEVMEM=${DEVMEM:-$(command -v devmem)}
REGDUMP=${REGDUMP:-/proc/lg/fbdev/regdump}
FB0=${FB0:-/proc/lg/fbdev/fb0/status}
DIAG=${DIAG:-/proc/lg/fbdev/chip/diag}
THEME=${THEME:-/var/lib/lumatv/theme/theme.json}
DEADMAN=${DEADMAN:-12}
HOLD=${HOLD:-4}
DRY=0
[ "$1" = -n ] && { DRY=1; shift; }
STEP=$1
mkdir -p "$OUT" || exit 1

# Registers (docs/research/display-chip-direct.md § 1.2, tools/regdecode.py find).
VSP0=0xCC480360          # OSD0.osd0_vsp_bosd_ctrl_0: vspyc_en bit 0, graycolor_en bit 1, r_center 8:15, r_contrast 16:25 (live 0x02000001)
HOLE_SEL=0xCC48029C      # B.OSD1 ctrl_bosd_mixer_hole: osd0_en bit 0, osd0_hole_sel bits 1:3
HOLE0_WX=0xCC4802C8      # B.OSD1 muxa: hole0 w 0:12, x 13:25, en bit 28, reverse bit 29
HOLE0_HY=0xCC4802CC      # B.OSD1 muxb: hole0 h 0:11, y 12:23, alpha 24:31
OSD01_4K=0xCC480440      # OSD0_1 reg_4k_osd_main_ctrl_0: reg_4k_mode_en bit 31 (the probe's bit 0 was the layer's osd_sync_enable)
TOP_FBDC0=0xCC480060     # TOP ctrl_fbdc0: reg_osd0_1x2_bypass_en bit 0 (live 0x000ff111)

cs() { read -r _up _ < /proc/uptime; echo "${_up%.*}${_up#*.}"; }
T0=$(cs)
log() { _t=$(( $(cs) - T0 )); printf '+%d.%02d  %s\n' $((_t / 100)) $((_t % 100)) "$*" | tee -a "$OUT/$STEP.log"; }
nap() { [ "$DRY" = 1 ] || sleep "$1"; }
ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $RT "$@" 2>/dev/null; }
jnum() { grep -oE "\"$1\":-?[0-9.]+" | head -1 | cut -d: -f2; }
jstr() { grep -oE "\"$1\":\"[^\"]*\"" | head -1 | cut -d'"' -f4; }
jbool() { grep -oE "\"$1\":(true|false)" | head -1 | cut -d: -f2; }
jarr() { grep -oE "\"$1\":\[[^]]*\]" | head -1 | sed 's/.*://'; }
fb() { sed -n "s/^ *$1 *[:=] *//p" $FB0 2>/dev/null | head -1; }
rd() { "$DEVMEM" "$1" 32; }
same() { [ $(( ($1 ^ $2) & 0xFFFFFFFF )) -eq 0 ]; }
flips() { fb status.fb_flip_cnt; }
# Registers that differ between two regdumps: address, first value, second value.
rdiff() { awk 'NR == FNR { if ($1 ~ /^0x/) v[$1] = $NF; next } ($1 ~ /^0x/) && ($1 in v) && v[$1] != $NF { print $1, v[$1], $NF }' "$1" "$2"; }
header() { log "fb0: visible=$(fb visible) xpos=$(fb hdr.xpos) ypos=$(fb hdr.ypos) mem=$(fb hdr.w_mem)x$(fb hdr.h_mem) out=$(fb hdr.w_out)x$(fb hdr.h_out) flips=$(flips)"; }
# The set has no `lumatv` CLI: a theme set is a merge into theme.json, which the compositor re-reads within 2 s.
lumatv() {
    [ "$1 $2" = "theme set" ] || return 1
    python3 - "$3" "$THEME" <<'PY'
import json, sys
def merge(a, b):
    for k, v in b.items():
        if isinstance(v, dict) and isinstance(a.get(k), dict): merge(a[k], v)
        else: a[k] = v
t = json.load(open(sys.argv[2])); merge(t, json.loads(sys.argv[1])); json.dump(t, open(sys.argv[2], "w"), indent=2)
PY
}
orbit_moves() { dmesg | grep -c vsc_set_orbit_move; }
photo() { log "PHOTO: $*"; }

# A raw step: the terminal, the owner's yes, the dead-man. REVERT is the file of writes that undo the step, written by the step first.
confirm() {
    [ "$DRY" = 1 ] && return 0
    [ -t 0 ] || { log "!! raw writes need a terminal (the owner at the set)"; exit 2; }
    [ -n "$DEVMEM" ] || { log "!! no devmem"; exit 2; }
    log "the writes of this step:"; sed 's/^/    /' "$OUT/$STEP.writes" | tee -a "$OUT/$STEP.log"
    log "the revert file ($OUT/revert.sh):"; sed 's/^/    /' "$OUT/revert.sh" | tee -a "$OUT/$STEP.log"
    printf 'Type yes to write (anything else stops): '
    read -r _ans
    [ "$_ans" = yes ] || { log "not written"; exit 3; }
}
ARMED=0
cleanup() { [ "$ARMED" = 1 ] && { ARMED=0; log "revert (script exit)"; sh "$OUT/revert.sh"; }; }
arm() {
    if [ "$DRY" = 1 ]; then log "DRY  setsid nohup sh -c 'sleep $DEADMAN; sh $OUT/revert.sh' &"; return; fi
    setsid nohup sh -c "sleep $DEADMAN; sh $OUT/revert.sh" > /dev/null 2>&1 &
    ARMED=1
    trap cleanup EXIT
    trap 'exit 1' INT TERM HUP
    log "dead-man armed (pid $!, $DEADMAN s)"
}
revert_now() { if [ "$DRY" = 1 ]; then log "DRY  sh $OUT/revert.sh"; else sh "$OUT/revert.sh"; fi; ARMED=0; }
# One write with its read-back.
w() {
    if [ "$DRY" = 1 ]; then log "DRY  devmem $1 32 $2"; return 0; fi
    "$DEVMEM" "$1" 32 "$2" || { log "!! devmem write $1 failed"; return 1; }
    _r=$(rd "$1")
    same "${_r:-0xDEADBEEF}" "$2" && _ok=ok || _ok=DIFF
    log "W $1 <- $2  read ${_r:-none}  $_ok"
}
# The revert file: the registers given, back to what they read now, in reverse order of the arguments.
mk_revert() {
    {
        echo "echo \"revert \$(date -u +%T) uptime \$(cut -d' ' -f1 /proc/uptime)\" >> $OUT/revert.log"
        _rev=; for _a in "$@"; do _rev="$_a $_rev"; done
        for _a in $_rev; do echo "$DEVMEM $_a 32 $(rd "$_a")"; done
    } > "$OUT/revert.sh"
}
dumps() { # NAME: a regdump, and the registers that differ from the step's first dump
    cat $REGDUMP > "$OUT/$STEP-$1.txt"
    [ -s "$OUT/$STEP-first.txt" ] || cp "$OUT/$STEP-$1.txt" "$OUT/$STEP-first.txt"
    log "regdump $1: $(rdiff "$OUT/$STEP-first.txt" "$OUT/$STEP-$1.txt" | grep -vE '^0xcc4801(08|10|3c)|^0xcc4803dc|^0xcc4808' | tr '\n' ';' | cut -c1-300)"
}

step_observe() {
    log "observe: $(date -u '+%F %T') UTC, uptime $(cut -d' ' -f1 /proc/uptime) s, kernel $(uname -r)"
    log "dri master: $(grep -E ' y ' /sys/kernel/debug/dri/0/clients 2>/dev/null | tr -s ' ' | cut -c1-80)"
    S=$(ctl status)
    log "compositor: output=$(echo "$S" | jstr output) drawn=$(echo "$S" | jnum drawn)"
    log "scanout: $(ctl scanout | cut -c1-400)"
    log "pageprobe: $(ctl pageprobe)"
    log "afbc: $(ctl afbc | cut -c1-200)"
    header
    log "diag: $(grep -E 'support_4kosd|hw_wa_flag|pqwfw|ti_gain|sw_status|vsync_clk|disp.path' $DIAG | tr -s ' ' | tr '\n' ' ')"
    log "osd_sr=$(cat /proc/lg/fbdev/chip/osd_sr 2>/dev/null | head -1 | cut -c1-40) gain: $(head -1 /proc/lg/fbdev/chip/osd_sr_gain | cut -c1-40)"
    log "pqefw_osd0_out_ctrl=$(cat /sys/module/lg1k/parameters/fbdev_o22_pqefw_osd0_out_ctrl) vsync_cnt_on_flip=$(cat /sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip)"
    for i in 1 2 3; do cat $REGDUMP > "$OUT/observe-rd$i.txt"; sleep 1; done
    log "regdump 1 vs 3 (status, GSR and AFBC debug dropped): $(rdiff "$OUT/observe-rd1.txt" "$OUT/observe-rd3.txt" | grep -vE '^0xcc4801(08|10|3c)|^0xcc4803dc|^0xcc4808' | tr '\n' ';' | cut -c1-300)"
    log "OSD0 hdr0 (xpos<<16|ypos): $(grep -i '^ *0xcc480320 ' "$OUT/observe-rd1.txt" | awk '{print $NF}'); crop ctrl_1/2: $(grep -iE '^ *0xcc48039[04] ' "$OUT/observe-rd1.txt" | awk '{print $NF}' | tr '\n' ' ')"
    log "VSP: $(grep -iE '^ *0xcc48036[04] ' "$OUT/observe-rd1.txt" | awk '{print $NF}' | tr '\n' ' '); B.OSD1 hole sel/0: $(grep -iE '^ *0xcc48029c |^ *0xcc4802c[8c] ' "$OUT/observe-rd1.txt" | awk '{print $NF}' | tr '\n' ' ')"
    log "orbit steps so far: $(orbit_moves); last: $(dmesg | grep vsc_set_orbit_move | tail -2 | sed 's/.*oi\[/oi[/' | tr '\n' ' ')"
    log "kprobe (not run; Nitin decides): echo 'p:lumatv/flip osd_drm_flip_image id=%x0 x=%x2 y=%x3' > /sys/kernel/debug/tracing/kprobe_events"
    log "observe done: $OUT/observe-rd*.txt"
}

step_orbit() {
    R=$(ctl scanout)
    mode=$(echo "$R" | jstr mode)
    log "orbit: scanout mode=$mode offset=$(echo "$R" | jarr lastAt) direct=$(echo "$R" | jnum direct) composed=$(echo "$R" | jnum composed) last=$(echo "$R" | jstr state)/$(echo "$R" | jstr reason)"
    [ "$mode" = direct ] || { log "!! scanout is $mode: 'ctl scanout direct' first (and Home must be a client window the plane takes: direct frames in the report)"; return 1; }
    [ -f $THEME ] && cp $THEME "$OUT/theme.json.bak" && log "theme backed up to $OUT/theme.json.bak"
    [ "$DRY" = 1 ] || lumatv theme set '{"oledcare":{"enabled":true,"orbitPx":1,"stepSec":10}}' > /dev/null
    [ "$DRY" = 1 ] || ctl scanout offset on > /dev/null
    log "care on, radius 1, step 10 s; offset on; the ring walks up and left first (composed, 'shifted'), then right and down (placed)"
    photo "the top-left corner of the panel now, as the reference"
    moves0=$(orbit_moves)
    placed=0; t=0
    while [ $t -lt 180 ]; do
        R=$(ctl scanout)
        pf=$(echo "$R" | jnum placedFrames); at=$(echo "$R" | jarr lastAt); sh=$(echo "$R" | grep -oE '"shifted":[0-9]+' | cut -d: -f2)
        log "t=${t}s placedFrames=${pf:-0} lastAt=$at shifted=${sh:-0} direct=$(echo "$R" | jnum direct) last=$(echo "$R" | jstr state)/$(echo "$R" | jstr reason) fb0 xpos=$(fb hdr.xpos) ypos=$(fb hdr.ypos)"
        if [ "${pf:-0}" -gt 0 ] && [ "$at" != "[0,0]" ] && [ -n "$at" ]; then
            placed=1
            log "PLACED at $at: expected fb0 hdr.xpos/ypos = $at (unsigned, in the scaler's units)"
            photo "the top-left corner now: the strip's width in panel pixels is one unit of CRTC_X (1 = panel pixel, 2 = UI pixel)"
            break
        fi
        [ "$DRY" = 1 ] && break
        sleep 5; t=$((t + 5))
    done
    [ $placed = 1 ] || log "!! no placed frame in 180 s: the journal says why (ctl journal | grep scanout)"
    if [ $placed = 1 ] && [ "$DRY" != 1 ]; then
        log "holding for a Screen Shift step (every 180 s): hdr.xpos must keep its value across it"
        t=0
        while [ $t -lt 200 ] && [ "$(orbit_moves)" = "$moves0" ]; do sleep 5; t=$((t + 5)); done
        log "after ${t}s: orbit steps $moves0 -> $(orbit_moves); fb0 xpos=$(fb hdr.xpos) ypos=$(fb hdr.ypos); scanout lastAt=$(ctl scanout | jarr lastAt)"
        log "journal: $(ctl journal | grep -oE 'scanout: [^"]*' | tail -8 | tr '\n' ';' | cut -c1-400)"
    fi
    [ "$DRY" = 1 ] || ctl scanout offset off > /dev/null
    if [ -f "$OUT/theme.json.bak" ] && [ "$DRY" != 1 ]; then cp "$OUT/theme.json.bak" $THEME && lumatv theme set '{}' > /dev/null; log "theme put back"; fi
    log "orbit done; offset off"
}

step_pageprobe() {
    for case in "1280x720 20" "1440x810 20" "1920x1080 10"; do
        set -- $case
        log "pageprobe $1 for $2 s: $(ctl pageprobe "$1" "$2" | cut -c1-200)"
        photo "the whole panel: grid cells across = ${1%x*}/32; the stripe block top left; the diagonal; the ramp at the bottom; the squares' edges"
        [ "$DRY" = 1 ] && continue
        sleep 2
        header
        log "dmesg (last 3 fbdev/drm lines): $(dmesg | grep -E 'fbdev|drm' | tail -3 | cut -c1-120 | tr '\n' ';')"
        while ctl pageprobe | grep -q '"active":true'; do sleep 2; done
        log "over: $(ctl pageprobe)"; header
    done
    log "pageprobe done"
}

step_sharp() {
    for v in 0x00 0x40 0x80; do
        [ "$DRY" = 1 ] && { log "DRY  echo $v > /proc/lg/fbdev/chip/osd_sr_gain"; continue; }
        echo "$v" > /proc/lg/fbdev/chip/osd_sr_gain
        log "osd_sr_gain $v: diag $(grep -E 'ti_gain' $DIAG | head -1 | tr -s ' ')"
        photo "Home's text at gain $v (0x10 is the set's own)"
        sleep 8
    done
    [ "$DRY" = 1 ] || { echo 0x10 > /proc/lg/fbdev/chip/osd_sr_gain; echo 0 > /proc/lg/fbdev/chip/osd_sr; }
    log "osd_sr 0 (the sharpener and its RGB-YC round trip bypassed): diag $(grep -E 'sw_status|hw_status' $DIAG | tr -s ' ' | tr '\n' ' ')"
    photo "Home's text and colours with the sharpener off"
    nap 8
    [ "$DRY" = 1 ] || echo 1 > /proc/lg/fbdev/chip/osd_sr
    log "sharp done: osd_sr $(grep sw_status $DIAG | tr -s ' '), gain $(grep ti_gain $DIAG | head -1 | tr -s ' ')"
}

step_vsp() {
    mk_revert $VSP0
    printf '%s\n' "$DEVMEM $VSP0 32 0x01000001   # contrast 0x100 of 0x200: the UI at half contrast, $HOLD s" "$DEVMEM $VSP0 32 0x02000003   # graycolor_en: the UI without colour, $HOLD s" > "$OUT/$STEP.writes"
    log "vsp: $VSP0 reads $(rd $VSP0) (expected 0x02000001: contrast 0x200, centre 0, vspyc on)"
    confirm; dumps before; arm
    w $VSP0 0x01000001 || return 1
    photo "Home at half contrast (darker, the ramp of greys compressed)"
    nap "$HOLD"; dumps contrast; header
    w $VSP0 0x02000003 || return 1
    photo "Home in grey"
    nap "$HOLD"; dumps grey
    revert_now
    log "after the revert: $VSP0 reads $(rd $VSP0); flips $(flips)"
    dumps after
    log "vsp done"
}

step_hole() {
    mk_revert $HOLE0_HY $HOLE0_WX $HOLE_SEL
    printf '%s\n' "$DEVMEM $HOLE0_WX 32 0x10B403C0   # hole 0: x 1440, w 960, enabled" "$DEVMEM $HOLE0_HY 32 0x0032A21C   # y 810, h 540, alpha 0" \
        "$DEVMEM $HOLE_SEL 32 0x00000001   # OSD0's input takes hole select 0, $HOLD s" "$DEVMEM $HOLE_SEL 32 0x00000003   # then hole select 1, $HOLD s" > "$OUT/$STEP.writes"
    log "hole: sel $(rd $HOLE_SEL) wx $(rd $HOLE0_WX) hy $(rd $HOLE0_HY) (all expected 0)"
    confirm; dumps before; arm
    w $HOLE0_WX 0x10B403C0 || return 1
    w $HOLE0_HY 0x0032A21C || return 1
    w $HOLE_SEL 0x00000001 || return 1
    photo "the middle of the panel: a 960x540 rectangle (or 480x540 if the units are the half-width path's) showing what is under the OSD"
    nap "$HOLD"; dumps sel0; header
    w $HOLE_SEL 0x00000003 || return 1
    photo "the middle again (hole select 1)"
    nap "$HOLD"; dumps sel1
    revert_now
    log "after the revert: sel $(rd $HOLE_SEL) wx $(rd $HOLE0_WX) hy $(rd $HOLE0_HY); flips $(flips)"
    dumps after
    log "hole done"
}

step_osd01() {
    log "osd01: stage 1 and 2 are tools/osd01-probe.sh (its own dead-man); LumaTV's pages must be linear: afbc $(ctl afbc | jstr mode)"
    if [ "$DRY" = 1 ]; then OUT=$OUT/osd01 sh "$D/osd01-probe.sh" -n all; else OUT=$OUT/osd01 NOPTY=1 PAUSE=10 sh "$D/osd01-probe.sh" all; fi
    mk_revert $OSD01_4K $TOP_FBDC0
    top=$(rd $TOP_FBDC0); k=$(rd $OSD01_4K)
    printf '%s\n' "$DEVMEM $OSD01_4K 32 $(printf '0x%08X' $(( k | 0x80000000 )))   # reg_4k_mode_en, $HOLD s" \
        "$DEVMEM $TOP_FBDC0 32 $(printf '0x%08X' $(( top & ~1 )))   # reg_osd0_1x2_bypass_en cleared, $HOLD s" > "$OUT/$STEP.writes"
    log "stage 3: $OSD01_4K reads $k, $TOP_FBDC0 reads $top (expected 0x000ff111)"
    confirm; dumps before; arm
    w $OSD01_4K "$(printf '0x%08X' $(( k | 0x80000000 )))" || return 1
    photo "the panel: most likely unchanged or a broken OSD0 (half width, doubled); a second picture would be the find"
    nap "$HOLD"; dumps 4k; header
    w $TOP_FBDC0 "$(printf '0x%08X' $(( top & ~1 )))" || return 1
    photo "the panel again (combiner not bypassed)"
    nap "$HOLD"; dumps combiner
    revert_now
    log "after the revert: $OSD01_4K $(rd $OSD01_4K), $TOP_FBDC0 $(rd $TOP_FBDC0); flips $(flips)"
    dumps after
    log "osd01 done"
}

case "$STEP" in
    observe) step_observe ;;
    orbit) step_orbit ;;
    pageprobe) step_pageprobe ;;
    sharp) step_sharp ;;
    vsp) step_vsp ;;
    hole) step_hole ;;
    osd01) step_osd01 ;;
    revert) [ -f "$OUT/revert.sh" ] && sh "$OUT/revert.sh" && echo "reverted: $(cat "$OUT/revert.sh" | grep -c devmem) writes" || echo "no revert file" ;;
    *) sed -n '2,19p' "$0"; exit 2 ;;
esac
