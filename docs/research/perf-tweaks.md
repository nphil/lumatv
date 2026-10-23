# LumaTV performance tweaks: measurements on the C4

Measured on the OLED65C4PUA (webOS 10.2.1, kernel 5.4.268) on 2026-10-23.
Rule under test: every frame on its 60 Hz deadline, at rest and in motion.

## Resume here

Status 2026-10-23 20:00: TV handed back to Nitin (watching). Hold all TV
work until Main says "TV is free". Nothing is applied on the set; nothing
to revert. TV state at handback: Active, LG Home in front with LumaHome
over it, Filmmaker confirmed, ftrace off and events disabled, buffer back
to 1408 kB, `/tmp/perf` on the set holds only `capture.sh`.

Done: platform survey, probe verified (`drm_vblank_event_delivered` = one
presented frame), one 5 s idle trace analysed (sections "How a frame is
produced" and "What the idle frame draws"), orchestrator written, tweak
plan and gap hypotheses written. Not done: everything that needs the TV.

Remaining TV plan (about 85 min of screen time), run from the
workstation (`pip install --break-system-packages paramiko` if missing;
`/data/home/tmp/ha-token-fetch.sh` first so `/data/home/tmp/ha-token`
exists):

1. Pre-flight (2 min). `luna-send -n 1
   luna://com.webos.service.tvpower/power/getPowerState {}` must not show
   Active with a non-Home app in `getForegroundAppInfo`. From Active
   Standby: `power/powerOn {"reason":"remoteKey"}` then `turnOnScreen`
   (it answers -102 if the screen is already on; fine). Confirm
   `settingsservice/getSystemSettings {"category":"picture","keys":["pictureMode"]}`
   = filmMaker. Copy `docs/research/perf/capture.sh` to
   `/tmp/perf/capture.sh` on the set (sftp) and `chmod +x` it.
2. Baseline (25 min):
   `python3 docs/research/perf/run.py --tag base --repeat 3 --workloads idle,dpad,qs,apphome`
   then `python3 docs/research/perf/run.py --tag base --repeat 2 --workloads video`
   (YouTube `youtube.leanback.v4` with a free video, Home over it). Each
   run prints one summary line per phase and appends to
   `/tmp/perf/summary.txt`; per-run detail in `/tmp/perf/<name>.txt`.
   Copy the summary into the Baseline table below.
3. Root causes (10 min): for every gap over 17 ms in the dpad, qs and
   apphome traces read the gap block in `<name>.txt` (compositor run /
   runnable / sleep split, GPU busy, what ran on each CPU, reclaim, cpuhp,
   block I/O, wakeups) and classify. Write the classes into "Root causes".
4. Tweaks (45 min), one at a time with `--apply` and `--revert`, for the
   workloads the tweak can touch, 2 repeats each:
   - T1 `--apply 'echo always_on > /sys/class/misc/mali0/device/power_policy' --revert 'echo coarse_demand > /sys/class/misc/mali0/device/power_policy'`
   - T2 `--apply 'echo 0 > /proc/lg/pm/mp_enable; echo 1 > /sys/devices/system/cpu/cpu3/online' --revert 'echo 1 > /proc/lg/pm/mp_enable'`
   - T3 `--apply 'for i in 176 178; do echo 2 > /proc/irq/$i/smp_affinity; done; echo 4 > /proc/irq/124/smp_affinity' --revert 'for i in 176 178 124; do echo f > /proc/irq/$i/smp_affinity; done'`
   - T4 `--apply 'python3 /tmp/perf/tweak.py fifo 1' --revert 'python3 /tmp/perf/tweak.py other'`
   - T5 `--apply 'python3 /tmp/perf/tweak.py affinity 2' --revert 'python3 /tmp/perf/tweak.py affinity f'` (copy `docs/research/perf/tweak.py` to `/tmp/perf/` in pre-flight; busybox here may lack taskset and chrt)
   - T6 `--apply '/media/developer/apps/usr/palm/applications/org.nphil.lumaglass/tools/lumaglass theme set "{\"wallpaperMotion\":{\"fps\":10}}"' --revert '... {\"fps\":30}'`, and a second run with `{"enabled":false}` / `{"enabled":true}`; wait 3 s after apply (2 s theme poll).
   - T7 threaded render loop: add `QSG_RENDER_LOOP=threaded` to
     `/var/systemd/system/env/surface-manager.env`, then `lumaglass apply`
     will not restart (env unchanged from its view), so restart with the
     tool's path: `systemctl restart surface-manager-daemon` is NOT allowed
     bare; use `lumaglass fps on` then `lumaglass fps off` (each rebuilds
     and restarts through `restart_surface`, which also sweeps the crash
     reports). Check Home recovers (capture > 79 kB). Measure. Remove the
     line, restart the same way. Risk: LG's QML may assume the basic loop;
     if the screen stays black, remove the line and restart.
   - T8 `--apply 'echo 20 > /proc/sys/vm/swappiness' --revert 'echo 100 > /proc/sys/vm/swappiness'` with `apphome` only.
4b. For ShellArchitecture (10 min, numbers go in "Compositor floor"):
   - Floor: wallpaper motion off (`theme set {"wallpaperMotion":{"enabled":false}}`),
     LumaHome idle, then the m64 on-screen probe (`docs/research/m64/wl.c`,
     64-bit blob + heap shim on DRM dumb buffers; binaries from the 10-22
     session, look in `/var/lib/lumaglass/exp` and `/tmp` on the set) as a
     full-screen overlay for 60 s inside `capture.sh start floor full`.
     Report presented frames over 20 ms from `drm_vblank_event_delivered`
     and compositor CPU per presented frame (`sm_run_ms / frames`). Repeat
     under T7 if T7 runs. Restore motion after.
   - Video: in the `video` workload, compositor CPU per presented frame
     and GPU busy for `video-idle` (app UI still) vs `video-dpad` (Home
     animating over video); add a phase with YouTube's own UI animating
     (d-pad inside YouTube before pressing HOME) if the run allows.
5. Stability (5 min): compositor RSS from the sampler (`rss=` pages in
   `<name>.sys`) across the 3 apphome runs and 3 qs runs; `dmesg | grep -i
   -E "oom|killed process"`; `/proc/lg/pm/status` temperature.
6. Finish: `luna-send -n 1 luna://com.webos.service.tvpower/power/powerOff '{"reason":"remoteKey"}'`,
   confirm `getPowerState` = Active Standby, confirm Filmmaker, and check
   every revert in step 4 took (`power_policy`, `mp_enable`, IRQ masks
   `f`, compositor policy/nice, affinity `f`, env file = only
   `QML2_IMPORT_PATH=...`, theme fps 30). Fill the Baseline, Tweaks and
   Recommendations sections of this file.

## Platform facts that bound the search

- CPU: 4x A76, one OPP (1.4 GHz), governor `performance`, no cpuidle
  driver exposed. CPU frequency cannot drop, so no DVFS ramp exists to tune.
- LG's own PM governor (`/proc/lg/pm`, kernel task `LGDTV-PMDRV-TAS`) hot-plugs
  CPU3 by load: `mp_enable=1`, sampling 20 ms, `up_threshold_easy 75`,
  `up_threshold_hard 90`, `mpd_count 100`. 261 CPU3 shutdowns in 7 h of
  mostly standby; every few seconds while idle. Each unplug is a
  stop_machine plus task and timer migration; each plug is a secondary boot
  (dmesg `CPU3: Booted secondary processor`). `freq_enable=0`.
- GPU: Mali-G52 3-core, kbase r1p0 with Bifrost DDK r46p0 (32-bit).
  `power_policy` `coarse_demand` (`always_on` available), `pm_poweroff`
  `400000 2 0` (shader cores off after 2 x 0.4 ms idle), runtime PM auto
  with 100 ms autosuspend, `dvfs_period 100`, `js_scheduling_period 100`.
  No devfreq node; LG's `dvfs_utilization` busy/idle counters are in
  `/sys/kernel/debug/mali0/`.
- IRQ affinity: every interrupt (`osd_irq` 104, `DE2VSYNC` 175 at 120 Hz,
  both GPU lines 176 and 178, eth0, mmc0) sits on CPU0
  (`smp_affinity f`, effective `1`); `irqbalance` absent.
- Compositor process: nice -19 already (`surface-manager.sh`), OOM score
  -1000, all CPUs allowed, cpuset root, cpu.shares 1024 at root (no
  per-service CPU weights). 21 threads; `tInput` and `tCursor` are
  SCHED_RR 99. No thread named QSGRenderThread: rendering is on the GUI
  thread (basic loop), confirmed below by the ioctl trace.
- Compositor environment (stock): `QSG_ATLAS_WIDTH/HEIGHT 1024`,
  `QSG_TRANSIENT_IMAGES 1`, `QSG_PROGRAM_BINARY_STORE
  /var/cache/qsg-shader-cache`, `QML_DISABLE_DISK_CACHE 1` (LumaTV's
  `QML_FORCE_DISK_CACHE=1` is decided but not yet in the env file),
  `QML_NO_TOUCH_COMPRESSION 1`, `QMLSCENE_DEVICE customcontext`,
  `QT_QPA_EGLFS_KMS_ATOMIC 1`, `QT_QPA_EGLFS_INTEGRATION eglfs_starfish`.
- Scheduler: CFS defaults for 4 cores (`sched_latency 18 ms`,
  `min_granularity 2.25 ms`, `wakeup_granularity 3 ms`), `skew_tick=1`,
  PREEMPT kernel. RT users: `lginput2` (7 threads at RR 99), `tInput`,
  `tCursor`, `terminalmanager`, `bthidmanager`, `mrcu_ls2` all RR 99.
- Memory: 2 GB; in standby 133 MB free, 809 MB available, swap
  `/dev/f2io-0` (LG's compressed RAM swap, 600 MB) 528 MB used, swappiness
  100; since boot 646 MB swapped out, 138 MB swapped in,
  `allocstall_movable 154`, `pgscan_direct 128k`. CMA 692 MB total, 104 MB
  free in standby. MGLRU present (`/sys/kernel/debug/lru_gen`).
- Output: one 1920x1080 ABGR8888 plane with AFBC (modifier
  `0x800000000000062`), vblank 60 Hz, OSD and DE2 vsync interrupts at
  120 Hz.
- Tooling on the set: tracefs with `drm`, `sched`, `irq`, `mali`,
  `cpuhp`, `vmscan`, `cma`, `compaction`, `workqueue` events; python3;
  no perf, strace or gdb.

## Measurement method

A presented frame is one `drm_vblank_event_delivered` trace event (the
page-flip completion the compositor's swap waits on). Idle Home gives
188 events in 5 s (37.6 fps, the known idle rate) and `drm_vblank_event`
gives 300 (60 Hz), so the event counts exactly what reached the panel.
Per workload the analyser reports presented frames, intervals over 17, 20
and 33 ms, the worst interval, and attributes each gap over 17 ms to what
ran on each CPU during it (`sched_switch`), interrupts, GPU power state and
job slots, hot-plug, reclaim and CMA events. The GPU profiler is never on.
A 1 s sampler records per-CPU time, PSI, memory, vmstat, GPU busy/idle,
compositor faults, CPU online mask and SoC temperature alongside.

## How a frame is produced (from the first idle trace, 5 s, 189 frames)

Read off `sched_switch`, `mali_job_slots_event`, `irq_handler_entry` and
the DRM events for one idle Home frame (basic render loop, GUI thread does
everything; the only other actor is the Mali driver's `mali-cmar-backe`
thread):

| offset | what |
|---|---|
| 0.0 ms | vblank; `drm_vblank_event_delivered` for the previous flip |
| 0.1 ms | GUI thread wakes from `waitForFlip`, 0.2 ms, hands the swap to the driver thread |
| 0.4 ms | GPU tiler job (slot 1) 0.5 ms, shader cores powered up 40 us before it |
| 0.9 ms | GPU fragment job (slot 0) **7.5 ms** for the idle Home scene |
| 8.5 ms | job IRQ; GUI thread wakes (it was blocked in `gbm_surface_lock_front_buffer`), 0.6 ms: atomic commit of the flip |
| 9.1 ms | GUI thread idle until the next animation tick; the next frame's QML render (2-3 ms CPU) runs here |
| 16.7 ms | next vblank: the committed buffer is scanned out |

So the pipeline is serial: CPU render (2-3 ms) -> wait for vblank -> GPU
(8.1 ms) -> commit -> next vblank. One frame uses about 12 ms of the
16.7 ms budget at idle; the GPU fragment pass is 60 % of it. Anything that
pushes the fragment pass past about 13 ms, or the CPU render past about
7 ms, misses the vblank and shows as a 33 ms interval. The GUI thread is
blocked for the whole GPU pass, so keys and luna messages queue behind
it (input-to-photon is 2-3 vblanks).

- GPU at idle Home: 379 jobs, 1502 ms busy in 5.0 s = 30 % of the GPU
  for a still screen (the 30 fps wallpaper motion layer plus whatever
  else repaints). 10 updates/s (decided) cuts this to about 8 %.
- GPU power gating costs nothing visible: shader cores go off 1 ms after
  each fragment job and come back 40 us before the next; 900 transitions
  a second, all sub-0.1 ms. `always_on` is not a frame win.
- Compositor GUI thread CPU: 541 ms in 5 s at idle = 11 % of a core, 2.8-3.4
  ms per presented frame. Scheduling latency at idle: max 0.21 ms (it is
  nice -19). Scheduling is not what makes idle frames late.
- Background CPU during idle Home, per 5 s: `pq_thread` 276 ms (5.5 % of a
  core, LG picture-quality kernel thread), `tAiToneService` 104 ms,
  `servicemanager` 100 ms, `MainWebModule` 73 ms, `ls-hubd` 69 ms,
  `mrcu_ls2` 49 ms, `kswapd0` 49 ms, `lginput2` 46 ms but **2,000 wakeups
  a second** (SCHED_RR 99, 65 wakeups per 33 ms, on CPU2), `memchute` 41 ms.
- IRQ time on CPU0 per 33 ms: arch_timer 0.7-1.1 ms (1 kHz tick, no idle
  tick skipping seen), GPU 0.07, vsync and OSD 0.06, IPI 0.05.
- A sibling agent's `grep -l` over `/usr/sbin` pinned one core and drove
  eMMC reads through dm-verity (`verity_work`, `kworker/*:*H`) and kswapd
  during this trace; it was stopped. Its presence on CPU2/3 did not touch
  the compositor on CPU0, which is itself a data point: one busy core of
  unrelated work does not reach the frame at idle.

## What the idle frame draws (from `LumaHome.qml`)

Qt Quick re-renders the whole window for every presented frame, so the
7.5 ms fragment pass is the full scene no matter what changed: the live
wallpaper (`liveWall`, one full-screen `ShaderEffect` over a 64x36
`GridMesh`, one texture read plus scrim per pixel, ticked by a Timer at
`wallpaperMotion.fps`, 30 by default), every Glass quad (cards, dock,
tiles, status bar: each an SDF `ShaderEffect` sampling the baked
`blurTex`), text and icons. The blur chain itself is baked once. The
wallpaper motion block lives in `theme.json`, which Home polls every 2 s,
so `fps`, `enabled` and the look parameters can be A/B-tested live with
`lumaglass theme set` and no restart.

## Tweak plan (each applied alone, measured, reverted)

| id | tweak | apply | revert |
|---|---|---|---|
| T1 | GPU cores never power-gated | `echo always_on > /sys/class/misc/mali0/device/power_policy` | `echo coarse_demand > ...` |
| T2 | LG core hot-plug off | `echo 0 > /proc/lg/pm/mp_enable; echo 1 > /sys/devices/system/cpu/cpu3/online` | `echo 1 > /proc/lg/pm/mp_enable` |
| T3 | GPU and network IRQs off CPU0 | `echo 2 > /proc/irq/{176,178}/smp_affinity; echo 4 > /proc/irq/124/smp_affinity` | `echo f > ...` (stock mask) |
| T4 | Compositor GUI thread SCHED_FIFO | python3 `os.sched_setscheduler(pid, SCHED_FIFO, 1)` | `SCHED_OTHER`, then nice -19 back |
| T5 | Compositor GUI thread pinned off CPU0 | `os.sched_setaffinity(pid, {1})` | `{0,1,2,3}` |
| T6 | Wallpaper motion 30 -> 10 fps, and off | `lumaglass theme set wallpaperMotion.fps 10` (live) | `... 30` |
| T7 | `QSG_RENDER_LOOP=threaded` | env file + restart through the tool | remove, restart |
| T8 | swappiness 100 -> 20 | `echo 20 > /proc/sys/vm/swappiness` | `echo 100 > ...` |

None survives a reboot by itself; a kept tweak goes into the tool's apply
and boot hook (sysfs/procfs writes at boot-bind) or the env file.

## Hypotheses for the 57-87 ms Home-return gaps (to be tested by trace)

- Direct reclaim inside the compositor's own allocations while an app that
  just ran still holds memory (`mm_vmscan_direct_reclaim_begin` with
  pid = compositor; 133 MB free, swappiness 100).
- Compositor pages swapped out while the app was in front, faulted back on
  show (`majflt` delta in the sampler, swap on the RAM-backed `f2io`).
- LG's compositor capturing the departing app's frame for the recents card
  (a readback on the GUI thread) or re-layout of its own views.
- LumaHome's show path: theme poll with `triggeredOnStart`, feed refresh
  and clock catch-up all on the first visible frame.
