#!/bin/sh
# On the set, as root (busybox sh): what the weather view costs in memory and whether it gives it all back. NOT RUN ON THE TV YET.
#
#   sh wx-memory-probe.sh [run]     open the radar page, zoom and pan, close it, and judge the samples (the default)
#   sh wx-memory-probe.sh sample L  one sample now under the label L (what `run` takes four of), appended to $OUT/samples.tsv
#   sh wx-memory-probe.sh judge     the verdict on the samples in $OUT/samples.tsv (what `run` ends with; the host's test feeds it)
#
# Home is in front, the screen on, the view closed. It presses the remote through `lumatv-compositor ctl remote` (codes are the evdev ones the
# compositor takes: Up 103, Down 108, Left 105, Right 106, OK 28, Back 158), the page's own zoom keys and the Magic Remote's wheel, so what
# is measured is the path a person takes. Four samples, each of the UI process (`lumatv-rustui`), the compositor and the helper:
#
#   baseline  Home, the view closed, 3 s of rest
#   open      the radar page up for 12 s: the backdrop, the map, the frame on show and the next, the loop playing
#   zoomed    after the zoom keys in and out and the wheel about two places of the screen, 4 s after the last
#   closed    Back, then 5 s
#
# Each sample: VmRSS, RssAnon, RssFile, RssShmem and Pss (`smaps_rollup`) of the three processes; the picture files the UI has made, which are
# the pixels of its layers and so the textures the compositor holds for them (`--shm` directory: count and KiB); `MemAvailable` and `CmaFree`;
# the GPU's own accounting of the compositor when the driver's debugfs gives it (`/sys/kernel/debug/mali0/gpu_memory`, kept as a file; the
# kilobytes are the compositor's pages x 4, and are `n/a` when the file is not there or not in that form); and what `ctl mem` says of the
# compositor. Evidence is in $OUT (/tmp/wxmem): samples.tsv, summary.txt, the raw files.
#
# PASS needs all of these (the limits are what the design holds: about 40 MB of pictures at most in the UI, 27 MB of tiles in the helper):
#   closed UI Pss          <= baseline + 4 MiB          the UI gave back its pictures and decoder buffers
#   closed UI layer files  <= baseline + 2 files and + 1 MiB   every map and radar texture is gone
#   closed helper RSS      <= baseline + 8 MiB          the engines, caches and threads of the view are gone
#   closed helper view     /v1/wx/state answers 409 (not open) 5 s after the close
#   closed compositor RSS  <= baseline + 8 MiB; GPU kilobytes (when readable) <= baseline + 8 MiB
#   open UI layer files    <= baseline + 64 files and + 48 MiB   the view holds a handful of pictures, not a frame for every step of the day
#   zoomed UI layer files  <= baseline + 64 files and + 56 MiB   a zoom does not make pictures that are never given back
#   open + zoomed helper   <= baseline + 64 MiB
# Where the 5 s are not enough for the compositor (it trims its heap when it has been quiet for 5 s: `idle.rs`) the closed row is read again
# 10 s later (`LATE=1`) and the verdict is on the later one, which the summary says.
#
# Environment: OUT (/tmp/wxmem), CTL (the compositor binary: /var/lib/lumatv/compositor/bin/lumatv-compositor), PORT (18731, the helper),
# OPEN_KEYS (the codes from Home to the weather view: "108 106 106 28" = down, right, right, OK onto the weather card), PAGE_KEYS (to the
# radar page from the view's first: "106"), ZOOM_KEYS (default: Up x4, Down x8, Up x4), CLOSE_KEYS ("158"), WHEEL=0 to leave the wheel out,
# LATE=1 to read a second closed row 10 s on. The key paths are those of the host render (`tests/render_rust_weather.py`); a Home that
# opens with the focus elsewhere needs OPEN_KEYS of its own.
OUT=${OUT:-/tmp/wxmem}
CTL=${CTL:-/var/lib/lumatv/compositor/bin/lumatv-compositor}
PORT=${PORT:-18731}
OPEN_KEYS=${OPEN_KEYS:-"108 106 106 28"}
PAGE_KEYS=${PAGE_KEYS:-"106"}
ZOOM_KEYS=${ZOOM_KEYS:-"103 103 103 103 108 108 108 108 108 108 108 108 103 103 103 103"}
CLOSE_KEYS=${CLOSE_KEYS:-"158"}
WHEEL=${WHEEL:-1}
LATE=${LATE:-0}
KIB=1024
TSV=$OUT/samples.tsv

mkdir -p "$OUT" || exit 1
nap() { sleep "$1" 2>/dev/null || sleep 1; }
say() { printf '%s\n' "$*" | tee -a "$OUT/summary.txt"; }

# The pid of the process whose command line holds $1 (the first, none of this script's).
pid_cmd() {
    for d in /proc/[0-9]*; do
        p=${d#/proc/}
        [ "$p" = "$$" ] && continue
        c=$({ tr '\0' ' ' < "$d/cmdline"; } 2>/dev/null) || continue
        case "$c" in *"$1"*) case "$c" in *wx-memory-probe*) continue ;; esac; echo "$p"; return ;; esac
    done
}

kib() { awk -v k="$2:" '$1 == k { print $2; exit }' "/proc/$1/status" 2>/dev/null; }
pss() { awk '$1 == "Pss:" { print $2; exit }' "/proc/$1/smaps_rollup" 2>/dev/null; }

# The picture files of the UI: count and KiB of its `--shm` directory.
shm_of() { { tr '\0' '\n' < "/proc/$1/cmdline"; } 2>/dev/null | awk 'p { print; exit } $0 == "--shm" { p = 1 }'; }
layer_files() {
    [ -d "$1" ] || { echo "n/a n/a"; return; }
    ls -l "$1" 2>/dev/null | awk 'NR > 1 && $1 ~ /^-/ { n++; b += $5 } END { printf "%d %d\n", n, b / 1024 }'
}

# The compositor's pages in the GPU's accounting: lines of the debugfs file that name its pid, the used-pages column x 4.
gpu_kib() {
    f=/sys/kernel/debug/mali0/gpu_memory
    [ -r "$f" ] || { echo n/a; return; }
    cp "$f" "$OUT/gpu_memory-$2.txt" 2>/dev/null
    awk -v pid="$1" '$2 == pid && $3 ~ /^[0-9]+$/ { s += $3 } END { if (NR > 0 && s > 0) printf "%d\n", s * 4; else print "n/a" }' "$f" 2>/dev/null
}

# The helper's status for a path; 000 when it does not answer (curl prints that itself and exits with an error).
code() { c=$(curl -s -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$PORT$1" 2>/dev/null); echo "${c:-000}"; }

HEADER="label	uptime_s	ui_rss	ui_anon	ui_file	ui_shmem	ui_pss	ui_layer_files	ui_layer_kib	comp_rss	comp_anon	comp_file	comp_shmem	comp_pss	comp_gpu_kib	help_rss	help_anon	help_file	help_shmem	help_pss	wx_state	mem_available	cma_free"

sample() {
    L=$1
    ui=$(pid_cmd lumatv-rustui)
    comp=$(pid_cmd bin/lumatv-compositor)
    help=$(pid_cmd lumatv-helper)
    [ -f "$TSV" ] || printf '%s\n' "$HEADER" > "$TSV"
    shm=$(shm_of "$ui")
    set -- $(layer_files "$shm")
    nf=$1 nk=$2
    gpu=$(gpu_kib "$comp" "$L")
    "$CTL" ctl mem > "$OUT/ctl-mem-$L.json" 2>/dev/null
    cp "/proc/$comp/smaps_rollup" "$OUT/comp-smaps_rollup-$L.txt" 2>/dev/null
    cp "/proc/$ui/smaps_rollup" "$OUT/ui-smaps_rollup-$L.txt" 2>/dev/null
    up=$(cut -d' ' -f1 /proc/uptime)
    printf '%s\t%s\t' "$L" "$up" >> "$TSV"
    if [ -n "$ui" ]; then printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t' "$(kib "$ui" VmRSS)" "$(kib "$ui" RssAnon)" "$(kib "$ui" RssFile)" "$(kib "$ui" RssShmem)" "$(pss "$ui")" "$nf" "$nk" >> "$TSV"; else printf 'n/a\tn/a\tn/a\tn/a\tn/a\t%s\t%s\t' "$nf" "$nk" >> "$TSV"; fi
    if [ -n "$comp" ]; then printf '%s\t%s\t%s\t%s\t%s\t%s\t' "$(kib "$comp" VmRSS)" "$(kib "$comp" RssAnon)" "$(kib "$comp" RssFile)" "$(kib "$comp" RssShmem)" "$(pss "$comp")" "$gpu" >> "$TSV"; else printf 'n/a\tn/a\tn/a\tn/a\tn/a\tn/a\t' >> "$TSV"; fi
    if [ -n "$help" ]; then printf '%s\t%s\t%s\t%s\t%s\t' "$(kib "$help" VmRSS)" "$(kib "$help" RssAnon)" "$(kib "$help" RssFile)" "$(kib "$help" RssShmem)" "$(pss "$help")" >> "$TSV"; else printf 'n/a\tn/a\tn/a\tn/a\tn/a\t' >> "$TSV"; fi
    printf '%s\t%s\t%s\n' "$(code /v1/wx/state)" "$(awk '$1 == "MemAvailable:" { print $2 }' /proc/meminfo)" "$(awk '$1 == "CmaFree:" { print $2 }' /proc/meminfo)" >> "$TSV"
    say "sampled $L"
}

# A column of the row labelled $1.
field() { awk -F'\t' -v l="$1" -v n="$2" 'NR == 1 { for (i = 1; i <= NF; i++) if ($i == n) c = i } $1 == l { print $c }' "$TSV"; }

verdict=0
# $1 what, $2 the value now, $3 the value before, $4 the allowance (KiB or files): PASS when now <= before + allowance; a value that is not a number is not judged.
within() {
    case "$2$3" in *n/a*|"") say "SKIP  $1: not readable ($2, $3)"; return ;; esac
    if [ "$2" -le $(( $3 + $4 )) ]; then say "PASS  $1: $2 <= $3 + $4"; else say "FAIL  $1: $2 > $3 + $4"; verdict=1; fi
}

tap() { "$CTL" ctl remote "$1" tap > /dev/null 2>&1; nap "${2:-0.5}"; }

run() {
    : > "$OUT/summary.txt"
    rm -f "$TSV"
    p=$(luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null | sed -n 's/.*"state" *: *"\([^"]*\)".*/\1/p' | head -n 1)
    if [ "$p" != Active ]; then say "VERDICT FAIL: power state is '${p:-unknown}', not 'Active'; nothing was pressed"; exit 2; fi
    [ -n "$(pid_cmd lumatv-rustui)" ] || { say "VERDICT FAIL: no lumatv-rustui process (is LumaTV's own Home up?); nothing was pressed"; exit 2; }
    [ -x "$CTL" ] || { say "VERDICT FAIL: $CTL is not there (CTL=)"; exit 2; }
    if [ "$(code /v1/wx/state)" != 409 ]; then say "VERDICT FAIL: the helper's weather view is open (or the helper is not answering) before the probe; close it and run again"; exit 2; fi
    say "== weather memory probe $(date) | uptime $(cut -d' ' -f1 /proc/uptime)"
    nap 3
    sample baseline

    for k in $OPEN_KEYS; do tap "$k" 0.8; done
    for k in $PAGE_KEYS; do tap "$k" 0.8; done
    t=0
    while [ "$(code /v1/wx/state)" = 409 ] && [ "$t" -lt 20 ]; do nap 1; t=$((t + 1)); done
    if [ "$(code /v1/wx/state)" = 409 ]; then say "VERDICT FAIL: the view did not open within 20 s of the keys (OPEN_KEYS=$OPEN_KEYS PAGE_KEYS=$PAGE_KEYS); the keys that reach the weather card are Home's layout's"; exit 2; fi
    nap 12
    sample open

    for k in $ZOOM_KEYS; do tap "$k" 0.6; done
    if [ "$WHEEL" = 1 ]; then
        # a zoom about two places of the screen, the pointer shown by the aim; a notch each way at each
        "$CTL" ctl pointer aim 400 300 > /dev/null 2>&1
        for i in 1 2 3 4 5 6; do "$CTL" ctl wheel up > /dev/null 2>&1; nap 0.4; done
        "$CTL" ctl pointer aim 1500 800 > /dev/null 2>&1
        for i in 1 2 3 4 5 6; do "$CTL" ctl wheel down > /dev/null 2>&1; nap 0.4; done
    fi
    nap 4
    sample zoomed

    for k in $CLOSE_KEYS; do tap "$k" 0.8; done
    nap 5
    sample closed
    if [ "$LATE" = 1 ]; then nap 10; sample closed_late; fi

    judge
}

# The verdict on the rows of the samples file: the row after the close is the later one when there is one.
judge() {
    row=closed
    grep -q '^closed_late	' "$TSV" && row=closed_late
    say "== judged on the row '$row'"
    b=baseline
    within "closed UI Pss" "$(field $row ui_pss)" "$(field $b ui_pss)" $((4 * KIB))
    within "closed UI layer files" "$(field $row ui_layer_files)" "$(field $b ui_layer_files)" 2
    within "closed UI layer KiB" "$(field $row ui_layer_kib)" "$(field $b ui_layer_kib)" $KIB
    within "closed helper RSS" "$(field $row help_rss)" "$(field $b help_rss)" $((8 * KIB))
    if [ "$(field $row wx_state)" = 409 ]; then say "PASS  closed helper view: /v1/wx/state answers 409"; else say "FAIL  closed helper view: /v1/wx/state answers $(field $row wx_state), not 409"; verdict=1; fi
    within "closed compositor RSS" "$(field $row comp_rss)" "$(field $b comp_rss)" $((8 * KIB))
    within "closed compositor GPU KiB" "$(field $row comp_gpu_kib)" "$(field $b comp_gpu_kib)" $((8 * KIB))
    within "open UI layer files" "$(field open ui_layer_files)" "$(field $b ui_layer_files)" 64
    within "open UI layer KiB" "$(field open ui_layer_kib)" "$(field $b ui_layer_kib)" $((48 * KIB))
    within "zoomed UI layer files" "$(field zoomed ui_layer_files)" "$(field $b ui_layer_files)" 64
    within "zoomed UI layer KiB" "$(field zoomed ui_layer_kib)" "$(field $b ui_layer_kib)" $((56 * KIB))
    within "open helper RSS" "$(field open help_rss)" "$(field $b help_rss)" $((64 * KIB))
    within "zoomed helper RSS" "$(field zoomed help_rss)" "$(field $b help_rss)" $((64 * KIB))
    if [ "$verdict" = 0 ]; then say "VERDICT PASS"; else say "VERDICT FAIL"; fi
    say "samples: $TSV"
    exit "$verdict"
}

case "${1:-run}" in
    run) run ;;
    sample) sample "${2:?label}" ;;
    judge) judge ;;
    *) echo "usage: wx-memory-probe.sh [run | sample LABEL | judge]" >&2; exit 2 ;;
esac
