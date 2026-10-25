#!/bin/sh
# Boot-time records for the power-on logo check. Both modes are started by the boot hook (autostart.sh).
#
# usage: bootcap.sh <seconds> <dir>     frames of the panel, and what is in front once a second
#        bootcap.sh events <file>       this boot's /var/log/messages lines about the logo path
#
# Nothing of ours can run before kdump.service starts the early hook, 12-15 s into a boot (nothing writable is
# mounted on /var until mount-readwrite.service, 10.8-13.1 s), and /var/log/messages itself begins at ~12.7 s. The
# 2026-10-25 17:50 boot showed the panel black from the capture's first frame (14.6 s) and LG's HDMI 2 banner at
# 16.8 s. So this starts as the worker's first act and cannot reach further back; what happens before it is not
# recoverable from the set (see docs/research/notify-slice-result.md, "Where the logos sit in a boot").
#
# Frames mode: `frames.txt` (index, uptime, bytes) and the PNGs, `state.txt` (uptime, the app manager's foreground
# app, the system UI manager's, the boot cover's status, the power state: a logo window is a system UI, so it shows
# there and not in the app manager) land in <dir> on tmpfs; `done` is written last. Started when the hook finds
# $STATE/bootcap holding a number of seconds (the flag is removed there: one boot only), or by hand over SSH before a
# power-on from standby, which starts no boot and so no hook.
#
# Events mode: the lines of every /var/log/messages* file that name the logo path, with the log's own monotonic stamps,
# into <file>; the previous boot's file is kept beside it as <file>.prev. Lines of earlier boots may come first (their
# stamps restart at 0).
if [ "$1" = events ]; then
  out=$2
  [ -n "$out" ] || exit 0
  logdir=${BOOTCAP_LOGDIR:-/var/log}
  mkdir -p "$(dirname "$out")" 2>/dev/null || exit 0
  [ -f "$out" ] && mv -f "$out" "$out.prev" 2>/dev/null
  {
    read -r up _ < /proc/uptime
    echo "# $(date '+%Y-%m-%d %H:%M:%S'), uptime $up s: lines of $logdir/messages* naming the logo path"
    for f in $(ls "$logdir"/messages.* 2>/dev/null | sort -t. -k2 -nr) "$logdir/messages"; do
      [ -f "$f" ] || continue
      case "$f" in *.gz) zcat "$f" 2>/dev/null ;; *) cat "$f" 2>/dev/null ;; esac
    done | grep -a -iE 'bootlogo|masterbrand|\[Logo\]|screencover|cpcover|coverui|FRESH_LAUNCH_SYSTEMUI|closeAnimation|offlogo|lgLogo|bootd' | head -n 400
  } > "$out.tmp" 2>/dev/null && mv -f "$out.tmp" "$out"
  exit 0
fi

secs=$1; dir=$2
case "$secs" in ''|*[!0-9]*) exit 0 ;; esac
[ "$secs" -gt 120 ] && secs=120
[ -n "$dir" ] || exit 0
mkdir -p "$dir" 2>/dev/null || exit 0
rm -f "$dir"/*.png "$dir/frames.txt" "$dir/state.txt" "$dir/done" 2>/dev/null
end=$(( $(cut -d. -f1 /proc/uptime) + secs ))

ask() {
  # One luna call, flattened and cut: the bus answers in JSON and only the head of it says what is wanted.
  timeout 3 luna-send -n 1 "$1" '{}' 2>/dev/null </dev/null | tr -d '\n ' | cut -c1-140
}

# A second process, so that its four calls a second do not slow the frames down.
(
  while [ "$(cut -d. -f1 /proc/uptime)" -lt "$end" ]; do
    read -r t _ < /proc/uptime
    echo "$t fg=$(ask luna://com.webos.applicationManager/getForegroundAppInfo) ui=$(ask luna://com.webos.service.systemUIManager/getForegroundAppInfo) cover=$(ask luna://com.webos.coverUI/getStatus) power=$(ask luna://com.webos.service.tvpower/power/getPowerState)" >> "$dir/state.txt"
    sleep 1
  done
) &
sampler=$!

i=0
while [ "$(cut -d. -f1 /proc/uptime)" -lt "$end" ] && [ "$i" -lt 240 ]; do
  read -r t _ < /proc/uptime
  timeout 5 luna-send -n 1 luna://com.webos.service.capture/executeOneShot \
    "{\"path\":\"$dir/$i.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":480,\"height\":270}" >/dev/null 2>&1 </dev/null
  echo "$i $t $(stat -c %s "$dir/$i.png" 2>/dev/null || echo 0)" >> "$dir/frames.txt"
  i=$((i+1))
  sleep 0.3
done
wait "$sampler" 2>/dev/null
: > "$dir/done"
exit 0
