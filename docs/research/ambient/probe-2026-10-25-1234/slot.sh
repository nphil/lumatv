#!/bin/sh
# written by tv_probe_slot.py; runs on the set, under /tmp/ambient-slot-202610251234 only (and the flag file, which is created and removed below)
R=/tmp/ambient-slot-202610251234
FLAG=/var/luna/preferences/vt_devmode
cd "$R" || exit 9
mkdir "$R/run.lock" || exit 8
echo $$ > "$R/slot.pid"
DM=
now() { date +%s.%N; }
removed() { if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed $1 $(now)" >> "$R/flag-events.txt"; fi; }
finish() { removed "$1"; if [ -n "$DM" ]; then kill "$DM" 2>/dev/null; DM=; fi; }
waitfor() { n=0; while [ ! -e "$1" ] && [ "$n" -lt "$2" ]; do sleep 1; n=$((n + 1)); done; }
trap 'finish trap' EXIT
trap 'exit 130' INT TERM HUP
echo "== home $(now)"
cat /proc/interrupts > "$R/interrupts-home-before.txt"
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' > "$R/home-launch.txt" 2>&1
sleep 5
luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$R/home-foreground.txt" 2>&1
( sleep 85; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created home $(now)" >> "$R/flag-events.txt"
"$R/ambient-probe" -s 60 -r 30 -L home -c "$R/home.csv" -d "$R/home.bin" > "$R/home.txt" 2>&1
echo "exit=$?" >> "$R/home.txt"
finish end
cat /proc/interrupts > "$R/interrupts-home-after.txt"
echo "== hdmi2 $(now)"
cat /proc/interrupts > "$R/interrupts-hdmi2-before.txt"
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.hdmi2"}' > "$R/hdmi2-launch.txt" 2>&1
sleep 8
luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$R/hdmi2-foreground.txt" 2>&1
( sleep 85; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created hdmi2 $(now)" >> "$R/flag-events.txt"
"$R/ambient-probe" -s 60 -r 30 -L hdmi2 -c "$R/hdmi2.csv" -d "$R/hdmi2.bin" > "$R/hdmi2.txt" 2>&1
echo "exit=$?" >> "$R/hdmi2.txt"
finish end
cat /proc/interrupts > "$R/interrupts-hdmi2-after.txt"
echo "== netflix $(now)"
cat /proc/interrupts > "$R/interrupts-netflix-before.txt"
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"netflix","params":{"contentTarget":"m=https://www.netflix.com/watch/80141336&source_type=4"}}' > "$R/netflix-launch.txt" 2>&1
sleep 14
now > "$R/netflix-key.txt"
waitfor "$R/netflix-key-sent.txt" 30
sleep 14
luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$R/netflix-foreground.txt" 2>&1
( sleep 85; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created netflix $(now)" >> "$R/flag-events.txt"
"$R/ambient-probe" -s 60 -r 30 -L netflix -c "$R/netflix.csv" -d "$R/netflix.bin" > "$R/netflix.txt" 2>&1
echo "exit=$?" >> "$R/netflix.txt"
finish end
cat /proc/interrupts > "$R/interrupts-netflix-after.txt"
echo done > "$R/done.txt"
