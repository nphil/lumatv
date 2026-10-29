#!/bin/sh
# The display lab's checks ON THE SET (busybox sh, devmem), as root, READ ONLY: it reads the six targets the lab may change and compares them
# with the readings it saved before the lab was opened, and asks the helper's door how it stands. It writes nothing to a register and starts
# nothing (docs/research/display-lab-result.md § 5).
#
#   displaylab-tv.sh baseline        read the six targets and keep them (do this with the lab closed, first)
#   displaylab-tv.sh check [LABEL]   read them again and say whether each is what the baseline says; exit 1 when one is not
#   displaylab-tv.sh watch [SECS]    read them every second (20): time how long a way out takes to put things back (kill the UI client, stop
#                                    the helper, leave by Back) while this runs
#   displaylab-tv.sh door            GET /v1/displaylab as the shell does: whether the set can be reached, the controls, the session, the journal
#   displaylab-tv.sh results         GET /v1/displaylab/results: the last run's answers
#
# Environment: DEVMEM, DIAG, OUT, PORT (18731).
D=/tmp/lumatv
OUT=${OUT:-$D/out/displaylab}
DEVMEM=${DEVMEM:-$(command -v devmem)}
DIAG=${DIAG:-/proc/lg/fbdev/chip/diag}
PORT=${PORT:-18731}
BASE=$OUT/baseline.txt
mkdir -p "$OUT" || exit 1

# The six targets of helper/app/src/displaylab/allow.rs, by the names the helper's journal and log use.
read_all() {
    [ -n "$DEVMEM" ] || { echo "!! no devmem" >&2; exit 2; }
    echo "osd_sr_gain $(grep 'sr_ctrl\[0\].ti_gain' "$DIAG" | head -1 | tr -s ' ' | cut -d' ' -f2)"
    echo "osd_sr $(grep 'osd_sr.sw_status' "$DIAG" | head -1 | tr -s ' ' | cut -d' ' -f2)"
    echo "osd0_vsp $("$DEVMEM" 0xCC480360 32)"
    echo "hole0_wx $("$DEVMEM" 0xCC4802C8 32)"
    echo "hole0_hy $("$DEVMEM" 0xCC4802CC 32)"
    echo "hole_sel $("$DEVMEM" 0xCC48029C 32)"
}

door() {
    curl -s -H 'X-Luma-Client: lumatv-ui' "http://127.0.0.1:$PORT$1"
    echo
}

case "$1" in
baseline)
    read_all | tee "$BASE"
    echo "kept in $BASE"
    ;;
check)
    [ -f "$BASE" ] || { echo "no baseline: run '$0 baseline' first, with the lab closed" >&2; exit 2; }
    now=$(read_all)
    bad=0
    while read -r name was; do
        is=$(echo "$now" | while read -r n v; do [ "$n" = "$name" ] && echo "$v"; done)
        if [ "$is" = "$was" ]; then echo "same     $name $is"; else echo "CHANGED  $name was $was is $is"; bad=1; fi
    done < "$BASE"
    echo "${2:-check}: $([ $bad = 0 ] && echo 'everything is as it was' || echo 'NOT everything is as it was')" | tee -a "$OUT/checks.log"
    exit $bad
    ;;
watch)
    n=${2:-20}
    i=0
    while [ "$i" -lt "$n" ]; do
        printf '+%ds  ' "$i"
        read_all | tr '\n' ' '
        echo
        i=$((i + 1))
        sleep 1
    done
    ;;
door)
    door /v1/displaylab
    ;;
results)
    door /v1/displaylab/results
    ;;
*)
    sed -n '2,14p' "$0"
    exit 2
    ;;
esac
