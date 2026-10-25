#!/bin/sh
# Runs on the set, from the directory tv-run.py uploads to (default /tmp/rustui):
#
#   sh tv-run.sh NAME PHASES [client options...]
#
# Runs the Rust UI client as a full-screen overlay of LG's compositor through the 64-bit launcher and the heap shim, with the
# ftrace frame capture armed around it (drm_vblank_event_delivered: what the panel received), and leaves in /tmp/perf:
#   NAME.trace  the ftrace dump (analyse.py cuts it at the client's P:<phase> markers)
#   NAME.sys    the 1 s system sampler (per-CPU, GPU busy and idle, surface-manager's CPU)
# and in /tmp/rustui: NAME.log, the client's own log (per-phase RESULT lines). PHASES is the client's --run list (idle:60,mix:30).
# SHOT=<seconds> also takes a screenshot of the panel that long into the run (/tmp/rustui/NAME.png).
#
# Refuses to run unless tvpower says "Active": with the panel off, frame callbacks are not paced and every number is meaningless.
D=$(cd "$(dirname "$0")" && pwd)
NAME=${1:?name}; PHASES=${2:?phases}; shift 2
GFX=${GFX64_DIR:-/tmp/gfx64}
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}
export LUMATV_GFX64_BUNDLE=$GFX/bundle LUMATV_GFX64_SHIM=$GFX/heapshim.so LUMATV_HEAP=${LUMATV_HEAP:-auto} LUMATV_HEAP_LOG=1

PSTATE=$(luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null |
  sed -n 's/.*"state" *: *"\([^"]*\)".*/\1/p' | head -n 1)
if [ "$PSTATE" != Active ]; then
  echo "VERDICT FAIL: power state is '${PSTATE:-unknown}', not 'Active' (screen on); nothing was run"
  exit 2
fi
pgrep -x lumatv-rustui >/dev/null && { echo "a client is already running"; exit 3; }

cma() { grep CmaFree /proc/meminfo | awk '{print $2}'; }
mkdir -p /tmp/perf
echo "== $NAME: $PHASES $* | CmaFree $(cma) kB | MemAvailable $(grep MemAvailable /proc/meminfo | awk '{print $2}') kB | uptime $(cut -d' ' -f1 /proc/uptime)"
sh "$D/capture.sh" start "$NAME" basic
if [ -n "${SHOT:-}" ]; then
  ( sleep "$SHOT"; luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot \
      "{\"path\":\"/tmp/rustui/$NAME.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":1920,\"height\":1080}" >/dev/null 2>&1 ) &
fi
sh "$GFX/launch.sh" --grace 5 -- "$D/lumatv-rustui" --mode window --assets "$D/assets" --shm /tmp/rustui/shm \
  --run "$PHASES" --log "$D/$NAME.log" "$@" > "$D/$NAME.out" 2>&1
RC=$?
sh "$D/capture.sh" stop "$NAME"
pkill -x lumatv-rustui 2>/dev/null
grep -E 'heapshim|gfx64:|ERROR: ld.so|keys from|panicked|error' "$D/$NAME.out" | grep -v 'ELFCLASS32' | head -n 20
grep -E 'RESULT|built the home|atlas|GL:|done:' "$D/$NAME.log"
echo "-- exit $RC, CmaFree after $(cma) kB, MemAvailable $(grep MemAvailable /proc/meminfo | awk '{print $2}') kB"
