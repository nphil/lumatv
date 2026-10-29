#!/bin/sh
# Mixer-hole window test, ON THE SET (busybox sh, devmem) as root, after the owner's go: one mixer-hole variant at a time, a hole for HOLD
# seconds over a LumaTV panel on a playing video (docs/research/layer-deep-result.md § 8.4). Run with `ctl pagealpha on` (AR24 pages).
# Same mechanics as tools/layerdeep-tv.sh (revert file written first, detached dead-man armed before the first write, every write read
# back, the revert read back) but without the typed yes: the authority is Nitin's request for this test, relayed by Main.
#
# Units, measured from Nitin's photos: a hole's x counts the 1920-wide UI space (one unit is two panel pixels), y, w and h count panel
# pixels (3840x2160). A window at panel pixels X Y W H is x = X/2, y = Y, w = W, h = H. The hole register's select field is a mask: bit 0
# (value 3) picks hole 0, value 1 picks none.
#
#   holetest-r19w.sh [-n] VARIANT [HOLD]       -n: reads only, prints the writes
#   variants: a   B.OSD1 hole 0 with x 1440 y 810 w 960 h 540 (a window at the right edge), hole register 1 (select field 0: opens nothing)
#             b   the same with hole register 3 (the lab's on_b: opens hole 0)
#             d   B.OSD0 mixer (0xCC480200 block) holes 0 and 1, hole register 3 (OSD0 feeds B.OSD1: no effect expected)
#             f   variant b and the mixer's forced video colour cleared (B.OSD1 miracast_ctrl bit 31, live 1)
#             x   hole 0 at X Y W H (env, the register units above), hole register SEL (default 3)
#             y   holes 0 to 2 at H0 H1 H2 ("x,y,w,h" each), hole register SEL (default 15)
#   env: DEADMAN (HOLD+8) seconds until the detached revert; OUT
D=/tmp/lumatv
OUT=${OUT:-$D/out/r19w-hole}
DEVMEM=${DEVMEM:-$(command -v devmem)}
FB0=/proc/lg/fbdev/fb0/status
LD=$D/lib/ld-linux-aarch64.so.1
DRY=0
[ "$1" = -n ] && { DRY=1; shift; }
V=$1
HOLD=${2:-10}
DEADMAN=${DEADMAN:-$((HOLD + 8))}
mkdir -p "$OUT" || exit 1
[ -n "$DEVMEM" ] || { echo "no devmem"; exit 2; }

# B.OSD1 (0xCC480280 block): the live blend mixer; B.OSD0 (0xCC480200 block): unconfigured, OSD0 does not feed it.
HSEL1=0xCC48029C; H0A1=0xCC4802C8; H0B1=0xCC4802CC; H1A1=0xCC4802D0; H1B1=0xCC4802D4; H2A1=0xCC4802D8; H2B1=0xCC4802DC; VF1=0xCC48028C
HSEL0=0xCC48021C; H0A0=0xCC480248; H0B0=0xCC48024C; H1A0=0xCC480250; H1B0=0xCC480254
# hole 0: w 960 (0:12), x 1440 (13:25), en bit 28; h 540 (0:11), y 810 (12:23), alpha 0 (24:31)
RA=0x10B403C0; RB=0x0032A21C
hex() { printf '0x%08X' "$1"; }

case "$V" in
    a) DESC="B.OSD1 hole 0 centred, osd0_en 1 hole_sel 0 (the lab's on_a)"
       WRITES="$H0A1 $RA
$H0B1 $RB
$HSEL1 0x00000001" ;;
    b) DESC="B.OSD1 hole 0 centred, osd0_en 1 hole_sel 1 (the lab's on_b)"
       WRITES="$H0A1 $RA
$H0B1 $RB
$HSEL1 0x00000003" ;;
    d) DESC="B.OSD0 mixer holes 0 and 1 centred, osd0_en 1 hole_sel 1"
       WRITES="$H0A0 $RA
$H0B0 $RB
$H1A0 $RA
$H1B0 $RB
$HSEL0 0x00000003" ;;
    f) DESC="variant b and B.OSD1 forced video colour off (miracast_ctrl bit 31)"
       WRITES="$H0A1 $RA
$H0B1 $RB
$VF1 0x00000000
$HSEL1 0x00000003" ;;
    x) X=${X:-1440}; Y=${Y:-810}; W=${W:-960}; H=${H:-540}; SEL=${SEL:-3}
       DESC="B.OSD1 hole 0 at x $X y $Y w $W h $H, hole register $SEL"
       WRITES="$H0A1 $(hex $(( W | (X << 13) | (1 << 28) )))
$H0B1 $(hex $(( H | (Y << 12) )))
$HSEL1 $(hex $SEL)" ;;
    y) SEL=${SEL:-15}
       WR=""
       for g in "$H0" "$H1" "$H2"; do
           n=$((${n:--1} + 1))
           [ -n "$g" ] || continue
           OIFS=$IFS; IFS=,; set -- $g; IFS=$OIFS
           case $n in 0) a=$H0A1; b=$H0B1 ;; 1) a=$H1A1; b=$H1B1 ;; *) a=$H2A1; b=$H2B1 ;; esac
           WR="$WR$a $(hex $(( $3 | ($1 << 13) | (1 << 28) )))
$b $(hex $(( $4 | ($2 << 12) )))
"
       done
       DESC="B.OSD1 holes ${H0:--} / ${H1:--} / ${H2:--}, hole register $SEL"
       WRITES="${WR}$HSEL1 $(hex $SEL)" ;;
    *) sed -n '2,17p' "$0"; exit 2 ;;
esac
# every address of a variant is inside the two mixers' hole blocks (B.OSD0 0xCC48021C-0xCC48025C, B.OSD1 0xCC48028C-0xCC4802DC)
for _a in $(echo "$WRITES" | awk '{print $1}'); do
    case "$_a" in
        0xCC48021C|0xCC480248|0xCC48024C|0xCC480250|0xCC480254|0xCC480258|0xCC48025C|0xCC48028C|0xCC48029C|0xCC4802C8|0xCC4802CC|0xCC4802D0|0xCC4802D4|0xCC4802D8|0xCC4802DC) ;;
        *) echo "refused: $_a is not a mixer hole register"; exit 2 ;;
    esac
done

LOG=$OUT/hole-$V.log
cs() { read -r _up _ < /proc/uptime; echo "${_up%.*}${_up#*.}"; }
T0=$(cs)
log() { _t=$(( $(cs) - T0 )); printf '+%d.%02d  %s\n' $((_t / 100)) $((_t % 100)) "$*" | tee -a "$LOG"; }
rd() { "$DEVMEM" "$1" 32; }
same() { [ $(( ($1 ^ $2) & 0xFFFFFFFF )) -eq 0 ]; }
fb() { sed -n "s/^ *$1 *[:=] *//p" $FB0 2>/dev/null | head -1; }
dewin() { tr -s ' \n' ' ' < /proc/lg/de/win0/status 2>/dev/null | cut -c1-160; }
ctlsum() { $LD --library-path $D/lib $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg status 2>/dev/null | grep -oE 'flips [0-9]+ late [0-9]+ lost [0-9]+' | head -1; }
: > "$LOG"
log "variant $V: $DESC; hold $HOLD s, dead-man $DEADMAN s, uptime $(cut -d' ' -f1 /proc/uptime)"

# The revert file first: every touched register back to what it reads now, in reverse order of the writes.
ADDRS=$(echo "$WRITES" | awk '{print $1}')
REV=; for a in $ADDRS; do REV="$a $REV"; done
{
    echo "echo \"revert \$(date -u +%T) uptime \$(cut -d' ' -f1 /proc/uptime)\" >> $OUT/revert.log"
    for a in $REV; do echo "$DEVMEM $a 32 $(rd $a)"; done
} > "$OUT/revert-$V.sh"
log "the registers before: $(for a in $ADDRS; do printf '%s=%s ' $a $(rd $a); done)"
log "the writes:"; echo "$WRITES" | sed 's/^/    /' | tee -a "$LOG"
log "the revert file:"; sed 's/^/    /' "$OUT/revert-$V.sh" | tee -a "$LOG"
if [ "$DRY" = 1 ]; then log "dry run: nothing written"; exit 0; fi

ARMED=0
DM=
cleanup() { [ "$ARMED" = 1 ] && { ARMED=0; log "revert (script exit)"; sh "$OUT/revert-$V.sh"; }; }
trap cleanup EXIT
trap 'exit 1' INT TERM HUP
cat /proc/lg/fbdev/regdump > "$OUT/hole-$V-before.txt"
setsid nohup sh -c "sleep $DEADMAN; sh $OUT/revert-$V.sh" > /dev/null 2>&1 &
DM=$!
ARMED=1
log "dead-man armed (pid $DM, $DEADMAN s); flips $(fb status.fb_flip_cnt); $(ctlsum); de win0: $(dewin)"

echo "$WRITES" > "$OUT/writes-$V.txt"
while read -r a v; do
    "$DEVMEM" "$a" 32 "$v" || { log "!! devmem write $a failed"; exit 1; }
    r=$(rd "$a")
    if same "${r:-0xDEADBEEF}" "$v"; then ok=ok; else ok=DIFF; fi
    log "W $a <- $v  read ${r:-none}  $ok"
done < "$OUT/writes-$V.txt"
log "written; the hole is up for $HOLD s"
HALF=$((HOLD / 2))
sleep "$HALF"
cat /proc/lg/fbdev/regdump > "$OUT/hole-$V-mid.txt"
log "mid-hold: $(for a in $ADDRS; do printf '%s=%s ' $a $(rd $a); done); flips $(fb status.fb_flip_cnt); $(ctlsum); de win0: $(dewin)"
sleep $((HOLD - HALF))
log "end of hold: $(for a in $ADDRS; do printf '%s=%s ' $a $(rd $a); done); flips $(fb status.fb_flip_cnt); $(ctlsum)"

# put back, read back; the dead-man is a backstop and goes once the registers read as they were
for try in 1 2 3; do
    sh "$OUT/revert-$V.sh"
    bad=0
    for a in $ADDRS; do
        want=$(grep " $a 32 " "$OUT/revert-$V.sh" | awk '{print $4}')
        got=$(rd $a)
        same "$got" "$want" || { bad=1; log "!! $a reads $got, wanted $want (try $try)"; }
    done
    [ $bad = 0 ] && break
done
ARMED=0
[ -n "$DM" ] && kill "$DM" 2>/dev/null
sleep 0.3
kill -0 "$DM" 2>/dev/null && log "!! the dead-man is still alive (pid $DM)"
if [ $bad = 0 ]; then log "RESTORED: every register reads as before"; else log "!! NOT RESTORED"; fi
log "after: $(for a in $ADDRS; do printf '%s=%s ' $a $(rd $a); done); flips $(fb status.fb_flip_cnt); $(ctlsum)"
cat /proc/lg/fbdev/regdump > "$OUT/hole-$V-after.txt"
log "dmesg tail: $(dmesg | tail -3 | cut -c1-120 | tr '\n' ';')"
log "variant $V done"
