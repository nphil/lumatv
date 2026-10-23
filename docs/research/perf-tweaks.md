# LumaTV performance tweaks: measurements on the C4

Measured on the OLED65C4PUA (webOS 10.2.1, kernel 5.4.268) on 2026-10-23.
Rule under test: every frame on its 60 Hz deadline, at rest and in motion.

## Resume here

- Done: platform survey (below), kernel-side frame probe verified
  (`drm_vblank_event_delivered` = one presented frame; 60 Hz vblank).
- Next: baseline runs (idle, d-pad sweep, Quick Settings, app-to-Home, video
  behind Home), then tweaks one at a time, then root causes of the gaps.
- Scripts: `docs/research/perf/capture.sh` (on the set at
  `/tmp/perf/capture.sh`: `start NAME [full|basic]`, `mark TEXT`,
  `stop NAME`), `docs/research/perf/analyse.py` (frame stats and gap
  attribution from a trace; `--gap MS` threshold, `--sm PID`).
  Workloads are driven from the workstation (HA `webostv.button` keys,
  markers through `capture.sh mark P:<phase>`); raw traces live in
  `/tmp/perf` on the workstation, not in the repo.
- TV state (handed back to Nitin 2026-10-23 19:35): Active, LG Home in
  front with LumaHome over it, Filmmaker confirmed. Nothing was ever
  applied: no governor, GPU policy, priority, affinity, env-file, service or
  Qt-env change was made before the handback. All probes stopped: ftrace
  off, events disabled, buffer back to 1408 kB, sampler gone, `/tmp/perf`
  holds only `capture.sh`. Do not touch the TV until Main says it is free.
- Revert list for anything still applied: none.

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
