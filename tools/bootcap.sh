#!/bin/sh
# One-off capture of the panel from the boot hook, for the power-on logo check.
#
# usage: bootcap.sh <seconds> <dir>
#
# The logo is up about 14-30 s into a boot and sshd answers on the new system only at about 46 s
# (measured 2026-10-25 16:34), so a capture driven from the host starts too late. autostart.sh starts
# this at ~14 s when the boot hook finds $STATE/bootcap (the flag is removed there, so it is one boot
# only). Frames and `frames.txt` (index, uptime, bytes) land in <dir> on tmpfs; `done` is written last.
secs=$1; dir=$2
case "$secs" in ''|*[!0-9]*) exit 0 ;; esac
[ "$secs" -gt 120 ] && secs=120
[ -n "$dir" ] || exit 0
mkdir -p "$dir" 2>/dev/null || exit 0
rm -f "$dir"/*.png "$dir/frames.txt" "$dir/done" 2>/dev/null
i=0
end=$(( $(cut -d. -f1 /proc/uptime) + secs ))
while [ "$(cut -d. -f1 /proc/uptime)" -lt "$end" ] && [ "$i" -lt 240 ]; do
  read -r t _ < /proc/uptime
  timeout 5 luna-send -n 1 luna://com.webos.service.capture/executeOneShot \
    "{\"path\":\"$dir/$i.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":480,\"height\":270}" >/dev/null 2>&1 </dev/null
  echo "$i $t $(stat -c %s "$dir/$i.png" 2>/dev/null || echo 0)" >> "$dir/frames.txt"
  i=$((i+1))
  sleep 0.5
done
: > "$dir/done"
exit 0
