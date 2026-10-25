#!/bin/sh
# The whole TV slot in one command, for a 15 minute slot (screen on): upload, an 8 s look with a panel screenshot, then
# idle 45 s + D-pad sweep 30 s ("mix" runs the gate's dpad and sweep scripts in turn). Optional extras run only while time is left.
#
#   sh rustui/tools/slot.sh [DEADLINE_HHMM]        default 0748; nothing new starts after DEADLINE minus the run's length
#
# Results (RESULT lines, analyse.py output, the screenshot) land in rustui/build/runs/ and print at the end.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
dl=${1:-0748}
cd "$here/.."
left() { echo $(( ($(date -d "${dl%??}:${dl#??}" +%s) - $(date +%s)) / 60 )); }   # minutes to the deadline (same day)
need() { [ "$(left)" -ge "$1" ] || { echo "skip $2: $(left) min left, needs $1"; return 1; }; }
sh rustui/tools/build-tv.sh >/dev/null 2>&1 || echo "build skipped or failed; using rustui/build/tv as is"
need 4 upload || exit 1
SHOT=4 python3 rustui/tools/tv-run.py look idle:8 || exit 1
need 4 measure || exit 1
python3 rustui/tools/tv-run.py rust idle:45,mix:30
if need 4 "flip cost"; then python3 rustui/tools/tv-run.py rust-direct idle:15,mix:30 --direct; fi
if need 3 "gpu-sync"; then python3 rustui/tools/tv-run.py rust-gpusync mix:30 --gpu-sync; fi
echo "== RESULT lines"; grep -h RESULT rustui/build/runs/*.log 2>/dev/null
echo "== screenshot rustui/build/runs/look.png (check: upright?)"
