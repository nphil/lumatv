#!/bin/sh
# TV smoke test of the 64-bit path, run ON the set from the directory smoke-tv.py uploads to (default /tmp/gfx64):
#   launch.sh heapshim.so x1client bundle/lib/*
# Runs the on-screen probe (x1client, the wl.c lineage in docs/research/) as a full-screen overlay client of LG's
# compositor, through the launcher and the heap shim, with no fallback so that a failure shows. Two runs: 8 s, the
# length of the earlier PLAN.md probe, then SECONDS (default 20). Prints frames, fps, frames over 20 ms and which
# buffer source the shim used.
#   sh smoke.sh [SECONDS] [auto|dumb|te]
#
# Refuses to run (exit 2, "VERDICT FAIL") unless tvpower's getPowerState says "Active": with the panel off (Active
# Standby, Screen Saver) frame callbacks are not paced and the probe reports 150+ fps, which proves nothing.
# GFX64_POWER_STATE overrides the query (test hook).
#
# Expected (PLAN.md, "64-bit drawing path", 8 s over idle Home):
#   dumb buffers (CMA)   60.2-60.8 fps, 2-10 frames over 20 ms     -> shim log: "via drm dumb", no dma_buf_te
#   dma_buf_te (pages)   55.9-56.6 fps, 35-41 frames over 20 ms    -> shim log: "via dma_buf_te (CMA short)"
# The longer run also carries LG's compositor floor (X1: 51.2 fps and 17 % of frames over 20 ms for a trivial
# client over idle Home, 60 s): a shortfall there is LG's compositor, not the shim. The verdict line judges the 8 s
# run only.
D=${GFX64_DIR:-$(cd "$(dirname "$0")" && pwd)}
SECS=${1:-20}
HEAP=${2:-auto}
MALI=$(ls "$D"/bundle/lib/libmali*.so 2>/dev/null | head -n 1)
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}
export X1_WINDOW_TYPE=${X1_WINDOW_TYPE:-_WEBOS_WINDOW_TYPE_OVERLAY}
export LUMATV_GFX64_BUNDLE=$D/bundle LUMATV_GFX64_SHIM=$D/heapshim.so LUMATV_HEAP=$HEAP LUMATV_HEAP_LOG=1

power_state() {
  if [ -n "${GFX64_POWER_STATE:-}" ]; then echo "$GFX64_POWER_STATE"; return; fi
  luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null |
    sed -n 's/.*"state" *: *"\([^"]*\)".*/\1/p' | head -n 1
}
PSTATE=$(power_state)
if [ "$PSTATE" != Active ]; then
  echo "VERDICT FAIL: power state is '${PSTATE:-unknown}', not 'Active' (screen on); nothing was run"
  exit 2
fi

cma() { grep CmaFree /proc/meminfo | awk '{print $2}'; }

# run <seconds>: runs the probe, leaves RC, FRAMES, FPS, B17, B20, B33, BOVER, WORST, DUMB, TE, FAIL
run() {
  LOG=${TMPDIR:-/tmp}/gfx64-smoke-$1s.log
  echo "== $1 s run, heap=$HEAP, CmaFree $(cma) kB, uptime $(cut -d' ' -f1 /proc/uptime)"
  X1_SECONDS=$1 sh "$D/launch.sh" --grace 5 -- "$D/x1client" "$MALI" > "$LOG" 2>&1
  RC=$?
  pkill -x x1client 2>/dev/null
  grep -E 'heapshim|gfx64:|\[x1\] (EGL|GL|window|RESULT)|ERROR: ld.so' "$LOG" | grep -v 'ELFCLASS32'
  set -- $(sed -n 's/.*RESULT frames=\([0-9]*\) in \([0-9.]*\) s = \([0-9.]*\) fps; gaps: <=17.5 ms \([0-9]*\), 17.5-20 \([0-9]*\), 20-33.5 \([0-9]*\), >33.5 \([0-9]*\); worst \([0-9.]*\) ms.*/\1 \3 \4 \5 \6 \7 \8/p' "$LOG")
  FRAMES=${1:-0}; FPS=${2:-0}; B17=${3:-0}; B20=${4:-0}; B33=${5:-0}; BOVER=${6:-0}; WORST=${7:-0}
  DUMB=$(grep -c 'via drm dumb' "$LOG"); TE=$(grep -c 'via dma_buf_te' "$LOG"); FAIL=$(grep -c 'FAILED' "$LOG")
  OVER20=$((B33 + BOVER))
  echo "-- exit $RC, CmaFree after $(cma) kB"
  echo "RESULT ${SECS_RUN}s: frames $FRAMES, $FPS fps, frames over 20 ms $OVER20 (20-33.5 ms $B33, >33.5 ms $BOVER), worst $WORST ms, allocs dumb $DUMB te $TE failed $FAIL"
}

SECS_RUN=8; run 8
R_RC=$RC; R_FPS=$FPS; R_DUMB=$DUMB; R_TE=$TE; R_FAIL=$FAIL
if [ "$SECS" != 8 ]; then SECS_RUN=$SECS; run "$SECS"; fi

# Verdict on the 8 s run.
ok_fps() { awk -v f="$1" -v m="$2" 'BEGIN { exit !(f >= m) }'; }
if [ "$R_RC" != 0 ] || [ "$R_FAIL" != 0 ] || [ $((R_DUMB + R_TE)) = 0 ]; then
  echo "VERDICT FAIL: probe exit $R_RC, failed allocations $R_FAIL, shim allocations $((R_DUMB + R_TE))"
elif [ "$R_TE" = 0 ] && ok_fps "$R_FPS" 59.5; then
  echo "VERDICT PASS: dumb (CMA) buffers, $R_FPS fps (expected 60.2-60.8)"
elif [ "$R_DUMB" = 0 ] && ok_fps "$R_FPS" 55; then
  echo "VERDICT PASS (fallback): dma_buf_te buffers, $R_FPS fps (expected 55.9-56.6)"
else
  echo "VERDICT CHECK: dumb $R_DUMB te $R_TE at $R_FPS fps; expected dumb 60.2-60.8 or te 55.9-56.6"
fi
