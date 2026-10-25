#!/bin/sh
# On the set, read-only: what the card app's processes are doing. Run by tv-card.py (NOT RUN ON THE TV YET).
#
#   sh tv-card-probe.sh pids        one line per process of the app: role pid uid command line. Roles: `stub` (the 32-bit launcher SAM
#                                   started), `shell` (gfx64/launch.sh), `app` (the 64-bit app, which runs under the bundle's loader)
#   sh tv-card-probe.sh ticks PID   utime + stime of the process, in clock ticks
#   sh tv-card-probe.sh rss PID     VmRSS in kB
#   sh tv-card-probe.sh facts       the set's side of the contract: who an app runs as, the devices, the mounts, memory, the compositor
pids() {
  for d in /proc/[0-9]*; do
    p=${d#/proc/}
    exe=$(readlink "$d/exe" 2>/dev/null) || continue
    cmd=$(tr '\0' ' ' < "$d/cmdline" 2>/dev/null)
    case "$exe" in
      */bin/lumatv-card) role=stub ;;
      *) case "$cmd" in
           *bin64/lumatv-card-app*) case "$cmd" in *ld-linux-aarch64*) role=app ;; *) role=shell ;; esac ;;
           *) continue ;;
         esac ;;
    esac
    uid=$(awk '/^Uid:/ {print $2}' "$d/status" 2>/dev/null)
    echo "$role $p $uid $cmd"
  done
}

case "$1" in
  pids) pids ;;
  ticks) awk '{print $14 + $15}' "/proc/$2/stat" 2>/dev/null ;;
  rss) awk '/^VmRSS:/ {print $2}' "/proc/$2/status" 2>/dev/null ;;
  facts)
    echo "id: $(id)"
    echo "machine: $(uname -m)"
    ls -l /dev/mali0 /dev/dri/card0 /dev/dma_buf_te 2>&1
    grep -E " /media/developer | /tmp | /var " /proc/mounts
    df -k /media/developer /tmp 2>&1 | tail -n +2
    grep -E "MemAvailable|CmaFree" /proc/meminfo
    echo "compositor: $(pgrep -f 'bin/lumatv-compositor' > /dev/null 2>&1 && echo LumaTV || echo LG)"
    ;;
  *) echo "usage: tv-card-probe.sh pids | ticks PID | rss PID | facts" >&2; exit 2 ;;
esac
