#!/bin/sh
# written by tv_probe_slot.py; runs on the set, under /tmp/ambient-slot-202610251941 only, with the staged service under /var/lib/lumatv and the flag file, which is
# created and removed below
R=/tmp/ambient-slot-202610251941
STATE=/var/lib/lumatv
FLAG=/var/luna/preferences/vt_devmode
cd "$R" || exit 9
mkdir "$R/run.lock" || exit 8
echo $$ > "$R/slot.pid"
DM=
SVC=
COL=
now() { date +%s; }
removed() { if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed $1 $(now)" >> "$R/flag-events.txt"; fi; }
stop_service() {
  if [ -n "$COL" ]; then kill "$COL" 2>/dev/null; wait "$COL" 2>/dev/null; COL=; fi
  if [ -n "$SVC" ]; then kill "$SVC" 2>/dev/null; wait "$SVC" 2>/dev/null; SVC=; fi
}
finish() { removed "$1"; if [ -n "$DM" ]; then kill "$DM" 2>/dev/null; DM=; fi; }
waitfor() { n=0; while [ ! -e "$1" ] && [ "$n" -lt "$2" ]; do sleep 1; n=$((n + 1)); done; }
trap 'stop_service; finish trap' EXIT
trap 'exit 130' INT TERM HUP
# the helper's own copy of the service, if one runs, leaves the capture to this one
sh "$R/capture.sh" stop "$STATE" >/dev/null 2>&1
echo "== home $(now)"
cat /proc/interrupts > "$R/interrupts-home-before.txt"
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' > "$R/home-launch.txt" 2>&1
sleep 5
luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$R/home-foreground.txt" 2>&1
LOG0=$(wc -c < /var/log/messages 2>/dev/null || echo 0)
( sleep 105; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created home $(now)" >> "$R/flag-events.txt"
"$STATE/bin/lumatv-capture" -s "$R/cap.sock" -r 10 -i 100 -b 1000 > "$R/home.service.log" 2>&1 &
SVC=$!
echo $SVC > "$R/svc.pid"
python3 "$R/slot_collect.py" "$R/cap.sock" 60 home "$R/home.json" --pid $SVC > "$R/home.txt" 2>&1 &
COL=$!
wait $COL
echo "exit=$?" >> "$R/home.txt"
COL=
n=0; while [ "$n" -lt 40 ] && ! grep -q "capture ended" "$R/home.service.log" 2>/dev/null; do sleep 0.1; n=$((n + 1)); done
stop_service
finish end
tail -c +$((LOG0 + 1)) /var/log/messages 2>/dev/null | grep -E 'LSHUB|capture|vtCapture' | tail -n 40 > "$R/home.log.txt"
cat /proc/interrupts > "$R/interrupts-home-after.txt"
echo done > "$R/done.txt"
