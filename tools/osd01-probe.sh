#!/bin/sh
# The second-OSD-layer probe of docs/research/scanout-research.md (§ A second OSD layer in the hardware, "The probe").
# Executed ON THE SET (busybox sh, devmem), as root, with the owner at the panel. It writes OSD0_1's block (0xCC480400-0xCC480440)
# and, in stage 1, the idle scratch register 0xCC480530, and nothing else; every register goes back to the value stage 0 read.
# The baseline is read live because it is not the doc's all-zero set when LG's compositor is up: there OSD0_1's ctrl_main reads
# 0x00080010 and its base, palette and bitmap pointers a dummy address, so a fixed list of zeros would not be a revert.
#
#   osd01-probe.sh [-n] stage0   regdump and register baseline, LG capture "before"
#   osd01-probe.sh [-n] stage1   scratch write and read-back on the idle blocks (0xCC480430, 0xCC480530)
#   osd01-probe.sh [-n] stage2   the mirror: dead-man revert armed first, twelve writes, read-backs, captures, revert, check
#   osd01-probe.sh [-n] all      stage0, stage1, a PAUSE s wait after printing READY FOR STAGE 2, stage2
#   osd01-probe.sh status        the registers now against the baseline, and the fb0 flip count
#   osd01-probe.sh revert        put the registers back from the baseline now
#   -n  dry run: reads only; the writes are printed, nothing is written, no dead-man, no capture, no sleeps.
#
# Environment: OUT (/tmp/osd01) the logs, captures and dumps; HOLD (6) seconds the layer stays on; DEADMAN (12) seconds until the
# detached revert fires; PAUSE (60); DEVMEM, REGDUMP, FB0, LUNA (luna-send), NOPTY=1 to skip the pty luna-send needs.
# Stage 2 needs LG's capture service (luna-send wants a terminal: script(1) gives it one when stdin is none).
OUT=${OUT:-/tmp/osd01}
DEVMEM=${DEVMEM:-$(command -v devmem)}
REGDUMP=${REGDUMP:-/proc/lg/fbdev/regdump}
FB0=${FB0:-/proc/lg/fbdev/fb0/status}
LUNA=${LUNA:-luna-send}
HOLD=${HOLD:-6}
DEADMAN=${DEADMAN:-12}
PAUSE=${PAUSE:-60}
DRY=${DRY:-0}
[ "$1" = -n ] && { DRY=1; shift; }

# OSD0, read only: the layer being scanned.
OSD0_MAIN=0xCC480300 OSD0_BASE=0xCC480304 OSD0_PLT=0xCC480334 OSD0_BMP=0xCC480338
# OSD0_1 (osdx_ctrl[1]), the registers stage 2 writes, in the order the revert restores them (layer off first).
L1_MAIN=0xCC480400 L1_SYNC=0xCC480440 L1_BASE=0xCC480404 L1_POS=0xCC480420 L1_MEM=0xCC480424 L1_OUT=0xCC480428
L1_FMT=0xCC48042C L1_KEY=0xCC480430 L1_PLT=0xCC480434 L1_BMP=0xCC480438
SCRATCH=0xCC480530   # osdx_ctrl[2], "OSD1 (removed)": stage 1 only
S2REGS="$L1_MAIN $L1_SYNC $L1_BASE $L1_POS $L1_MEM $L1_OUT $L1_FMT $L1_KEY $L1_PLT $L1_BMP"
TOP_DPATH=0xcc480020  # regdump's spelling; bits 20 and 24 are the AFBC enables of the two image pipelines

mkdir -p "$OUT" || exit 1
cs() { read -r _up _ < /proc/uptime; echo "${_up%.*}${_up#*.}"; }   # uptime in centiseconds
T0=$(cs)
log() { _t=$(( $(cs) - T0 )); printf '+%d.%02d  %s\n' $((_t / 100)) $((_t % 100)) "$*" | tee -a "$OUT/probe.log"; }
nap() { [ "$DRY" = 1 ] || sleep "$1"; }
rd() { "$DEVMEM" "$1" 32; }
same() { [ $(( ($1 ^ $2) & 0xFFFFFFFF )) -eq 0 ]; }
flips() { sed -n 's/.*status\.fb_flip_cnt *= *//p' "$FB0"; }
base_of() { while read -r _a _v; do [ "$_a" = "$1" ] && { echo "$_v"; return; }; done < "$OUT/baseline.txt"; }
dumpval() { awk -v a="$(echo "$1" | tr 'A-F' 'a-f')" '$1 == a { print $NF }' "$2"; }
# Registers that differ between two regdumps: address, first value, second value.
rdiff() { awk 'NR == FNR { if ($1 ~ /^0x/) v[$1] = $NF; next } ($1 ~ /^0x/) && ($1 in v) && v[$1] != $NF { print $1, v[$1], $NF }' "$1" "$2"; }

# luna-send needs a controlling terminal.
pty() {
    if [ -t 0 ] || [ "$NOPTY" = 1 ]; then timeout 8 sh -c "$1"; else timeout 8 script -q -c "$1" /dev/null < /dev/null; fi
}
# LG's capture service, the panel as DISPLAY sees it, 960x540 PNG.
cap() {
    _f=$OUT/$1.png
    if [ "$DRY" = 1 ]; then log "DRY  capture $_f"; return 0; fi
    rm -f "$_f"
    _t=$(cs)
    _r=$(pty "$LUNA -n 1 -f luna://com.webos.service.capture/executeOneShot '{\"path\":\"$_f\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}'" 2>&1 | tr -d '\r\n ')
    log "capture $1: $_r in $(( $(cs) - _t )) cs, $(wc -c < "$_f" 2> /dev/null || echo 0) bytes"
}

# Write, read back at once, log. A read-back that differs outside the optional mask counts: the header-ready flag
# (bit 29 of ctrl_main) is cleared when the hardware takes the header, so a read-back may lack it (it stayed set for
# OSD0_1 under LG's compositor, 2026-10-29).
WDIFF=0
w() {
    if [ "$DRY" = 1 ]; then log "DRY  devmem $1 32 $2"; return 0; fi
    "$DEVMEM" "$1" 32 "$2" || { log "!! devmem write $1 failed"; return 1; }
    _r=$(rd "$1")
    if [ $(( (${_r:-0xDEADBEEF} ^ $2) & 0xFFFFFFFF & ~${3:-0} )) -eq 0 ]; then _ok=ok; else _ok=DIFF; WDIFF=$((WDIFF + 1)); fi
    log "W $1 <- $2  read ${_r:-none}  $_ok"
}

baseline_read() {
    : > "$OUT/baseline.txt"
    for _a in $S2REGS $SCRATCH $OSD0_MAIN $OSD0_BASE $OSD0_PLT $OSD0_BMP; do echo "$_a $(rd "$_a")" >> "$OUT/baseline.txt"; done
}
rdall() { _s=; for _a in $S2REGS; do _s="$_s ${_a#0xCC48}=$(rd "$_a")"; done; log "regs $1:$_s"; }
# Every OSD0_1 register against the baseline; names the ones that differ.
verify() {
    _bad=0
    for _a in $S2REGS; do
        _b=$(base_of "$_a"); _c=$(rd "$_a")
        same "${_c:-0xDEADBEEF}" "$_b" || { log "!! $_a reads ${_c:-none}, baseline $_b"; _bad=1; }
    done
    return $_bad
}
# The revert, as a file of its own so the detached dead-man runs exactly what the script would.
mk_revert() {
    {
        echo "echo \"revert \$(date -u +%T) uptime \$(cut -d' ' -f1 /proc/uptime)\" >> $OUT/revert.log"
        for _a in $S2REGS; do echo "$DEVMEM $_a 32 $(base_of "$_a")"; done
    } > "$OUT/revert.sh"
}
revert_now() { if [ "$DRY" = 1 ]; then log "DRY  sh $OUT/revert.sh"; else sh "$OUT/revert.sh"; fi; }

stage0() {
    log "stage 0: $(date -u '+%F %T') UTC, uptime $(cut -d' ' -f1 /proc/uptime) s, kernel $(uname -r), dry=$DRY"
    cat "$REGDUMP" > "$OUT/regdump-before.txt"
    baseline_read
    cat "$FB0" > "$OUT/fb0-before.txt"
    log "baseline: $(tr '\n' ' ' < "$OUT/baseline.txt")"
    _d=$(dumpval $TOP_DPATH "$OUT/regdump-before.txt")
    log "TOP ctrl_dpath 0x$_d (bit 20 osd0_afbc_en, bit 24 osd1_afbc_en); OSD0 bitmap $(rd $OSD0_BMP), AFBC0 base $(dumpval 0xcc480804 "$OUT/regdump-before.txt"), AFBC1 base $(dumpval 0xcc482704 "$OUT/regdump-before.txt")"
    _f1=$(flips); nap 2; _f2=$(flips)
    log "fb0 flips $_f1 -> $_f2 in 2 s"
    cap before
}

# Whether the idle blocks hold a value: only a read-back of the pattern counts (a latched shadow reads back old too).
stage1() {
    [ -s "$OUT/baseline.txt" ] || baseline_read
    _bad=0
    for _a in $L1_KEY $SCRATCH; do
        _o=$(rd "$_a")
        if [ "$DRY" = 1 ]; then log "DRY  devmem $_a 32 0xA5A5A5A5, read back, devmem $_a 32 $_o"; continue; fi
        "$DEVMEM" "$_a" 32 0xA5A5A5A5; _n=$(rd "$_a")
        "$DEVMEM" "$_a" 32 "$_o"; _b=$(rd "$_a")
        same "$_n" 0xA5A5A5A5 && _hit=HIT || _hit="no hit"
        log "scratch $_a was $_o, now $_n ($_hit), restored $_b"
        same "$_b" "$_o" || { log "!! $_a not restored"; _bad=1; }
    done
    return $_bad
}

ARMED=0
cleanup() { [ "$ARMED" = 1 ] && { ARMED=0; log "revert (script exit)"; revert_now; }; }

stage2() {
    [ -s "$OUT/baseline.txt" ] || { log "!! no baseline: run stage0 first"; return 1; }
    for _a in $S2REGS; do
        _c=$(rd "$_a"); _b=$(base_of "$_a")
        same "$_c" "$_b" || { log "!! $_a is $_c, stage 0 read $_b: not running"; return 1; }
    done
    [ $(( $(rd $L1_MAIN) & 1 )) -eq 0 ] && [ $(( $(rd $L1_SYNC) & 0x80000001 )) -eq 0 ] || { log "!! OSD0_1 is not idle: not running"; return 1; }
    [ $(( $(rd $OSD0_MAIN) & 1 )) -eq 1 ] || { log "!! OSD0 is off: not running"; return 1; }
    BMP=$(rd $OSD0_BMP); BASE=$(rd $OSD0_BASE); PLT=$(rd $OSD0_PLT)
    [ $((BMP)) -ne 0 ] || { log "!! OSD0's bitmap pointer is 0: not running"; return 1; }
    mk_revert
    log "stage 2: OSD0_1 reads OSD0's page: bitmap $BMP, base $BASE, palette $PLT; on for $HOLD s, dead-man at $DEADMAN s"
    cap s2-pre
    _f0=$(flips)
    # The dead-man first: a process of its own (own session, immune to hangup) that restores every register after DEADMAN s,
    # whatever becomes of this shell or the connection.
    if [ "$DRY" = 1 ]; then
        log "DRY  setsid nohup sh -c 'sleep $DEADMAN; sh $OUT/revert.sh' &   revert.sh:"
        sed 's/^/DRY    /' "$OUT/revert.sh"
    else
        setsid nohup sh -c "sleep $DEADMAN; sh $OUT/revert.sh" > /dev/null 2>&1 &
        ARMED=1
        trap cleanup EXIT
        trap 'exit 1' INT TERM HUP
        log "dead-man armed (pid $!)"
    fi
    _armed_at=$(cs)
    w $L1_MAIN 0x10080010 || return 1       # header from registers, header-ready enabled, feeder order 2; layer off
    w $L1_BASE "$BASE" || return 1
    w $L1_POS  0x07800000 || return 1       # x 1920, y 0
    w $L1_MEM  0x07800438 || return 1       # 1920x1080 in memory
    w $L1_OUT  0x07800438 || return 1       # 1920x1080 out, no scaling
    w $L1_FMT  0xEE8003C0 || return 1       # ARGB, constant alpha 0x80
    w $L1_KEY  0x00000000 || return 1
    w $L1_PLT  "$PLT" || return 1
    w $L1_BMP  "$BMP" || return 1
    w $L1_MAIN 0x30080010 0x20000000 || return 1   # header ready
    nap 1
    w $L1_SYNC 0x00000001 || return 1       # second enable
    w $L1_MAIN 0x30080011 0x20000000 || return 1   # layer on
    _t_on=$(cs)
    log "layer on; $WDIFF read-back(s) differ"
    rdall on
    cat "$REGDUMP" > "$OUT/regdump-on.txt"
    cap s2-on
    if [ "$DRY" != 1 ] && [ $(( $(cs) - _t_on )) -lt $(( HOLD * 100 - 300 )) ]; then
        while [ $(( $(cs) - _t_on )) -lt 250 ]; do sleep 0.1; done
        rdall on2
        cap s2-on2
    fi
    if [ "$DRY" != 1 ]; then while [ $(( $(cs) - _t_on )) -lt $(( HOLD * 100 )) ]; do sleep 0.1; done; fi
    log "layer on for $(( ($(cs) - _t_on) / 100 )) s, flips $_f0 -> $(flips); reverting"
    revert_now
    ARMED=0
    if verify; then
        log "every OSD0_1 register is back at its baseline"
    else
        log "!! not at baseline after the revert: waiting for the dead-man"
        while [ $(( $(cs) - _armed_at )) -lt $(( DEADMAN * 100 + 150 )) ]; do sleep 0.2; done
        if verify; then log "at baseline after the dead-man"; else log "!! STILL NOT AT BASELINE: standby and wake, then a power cycle"; fi
    fi
    cap s2-after
    cat "$REGDUMP" > "$OUT/regdump-after.txt"
    rdiff "$OUT/regdump-before.txt" "$OUT/regdump-after.txt" > "$OUT/regdump-after.diff"
    rdiff "$OUT/regdump-before.txt" "$OUT/regdump-on.txt" > "$OUT/regdump-on.diff"
    log "regdump registers that differ: after vs before $(wc -l < "$OUT/regdump-after.diff"), on vs before $(wc -l < "$OUT/regdump-on.diff")"
    # A still Home does not flip by itself: a toast makes LG's compositor draw, which shows the panel still updates.
    _f1=$(flips)
    if [ "$DRY" != 1 ]; then
        pty "$LUNA -n 1 -f luna://com.webos.notification/createToast '{\"message\":\"OSD0_1 probe finished\",\"sourceId\":\"org.nphil.lumatv\"}'" > /dev/null 2>&1
        sleep 1.5
    fi
    log "panel update check: fb0 flips $_f1 -> $(flips)"
    cap s2-toast
    log "stage 2 done; $WDIFF write read-back(s) differed"
}

case "$1" in
    stage0) stage0 ;;
    stage1) stage1 ;;
    stage2) stage2 ;;
    all) stage0; stage1; log "READY FOR STAGE 2"; nap "$PAUSE"; stage2 ;;
    status) rdall now; verify && echo "at baseline"; echo "fb0 flips $(flips)" ;;
    revert) mk_revert; revert_now; verify && echo "at baseline" ;;
    *) sed -n '2,19p' "$0"; exit 2 ;;
esac
