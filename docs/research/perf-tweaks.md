# LumaTV performance tweaks: measurements on the C4

Measured on the OLED65C4PUA (webOS 10.2.1, kernel 5.4.268) on 2026-10-23.
Rule under test: every frame on its 60 Hz deadline, at rest and in motion.

## Resume here

Status 2026-10-24 19:20: TV window open (Main, 19:13). Set rebooted today
(uptime 7 h at 19:14; compositor pid 6183); found in Screen Saver with
Netflix paused in front, taken to Home and Netflix closed with
`closeByAppId`. Filmmaker confirmed. `capture.sh` and `tweak.py`
redeployed to `/tmp/perf`. Running the baseline now. Tweaks applied on
the set at this moment: none.

Done offline: platform survey, probe verified (`drm_vblank_event_delivered`
= one presented frame), one 5 s idle trace analysed: frame pipeline, a
fixed 5 ms Qt update timer inside every animated frame (strongest
candidate for missed deadlines, tweak T9), the cause of the uneven 36-38
fps idle, background load. Orchestrator, analyser, scheduler helper and
the env-file patch written; provisional recommendations and the
replace-vs-tune list at the end. Not done: every before/after number.

Decided by Main (21:55): option (a). The `env.extra` hook is in
`tools/lumaglass`, committed as `cb57f53` (not deployed): allowlisted
`KEY=VALUE` lines of `/var/lib/lumaglass/env.extra` go into the env file,
an env change restarts the compositor at `apply`, `revert` deletes the
file, and `QML_FORCE_DISK_CACHE=1` (decided) is always set, with the QML
cache cleared when the staged compositor set changes. Deploy it to the set
only in the TV window, backed up and restored (step 4, T9/T7). Priority
when the TV is free: baseline, T9, T6, T7, then T1-T5 and T8, then the
ShellArchitecture floor (step 4b).

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
   - T9 then T7, the env-file tweaks, with the repo's tool deployed for
     the test (`TOOL=/media/developer/apps/usr/palm/applications/org.nphil.lumaglass/tools/lumaglass`):
     1. Backup first: `cp $TOOL /var/lib/lumaglass/lumaglass.bak`. If
        `md5sum $TOOL` equals `git show 98071c7:tools/lumaglass | md5sum`
        (the version before the hook), sftp `git show cb57f53:tools/lumaglass`
        over it; otherwise apply `git show cb57f53 -- tools/lumaglass` to
        the deployed copy. Check `sh -n $TOOL`.
     2. `$TOOL apply` (env gains `QML_FORCE_DISK_CACHE=1`, restart). Check
        Home recovers (capture at 320x180 > 79 kB). Run `--tag cache`
        `dpad,qs,apphome` x2: the baseline for T9/T7 with the same tool.
     3. `echo QT_QPA_UPDATE_IDLE_TIME=0 > /var/lib/lumaglass/env.extra;
        $TOOL apply`; confirm with `tr '\0' '\n' < /proc/$(pidof
        surface-manager)/environ | grep QT_QPA`; run `--tag t9` x2.
     4. `echo QSG_RENDER_LOOP=threaded > /var/lib/lumaglass/env.extra;
        $TOOL apply`; if the panel stays black, `rm` the file and `apply`
        again. Run `--tag t7` x2.
     5. Restore: `rm /var/lib/lumaglass/env.extra; cp
        /var/lib/lumaglass/lumaglass.bak $TOOL`, then `$TOOL fps on` and
        `$TOOL fps off` (the old tool rewrites the env to its single line
        and restarts on the QML change), `rm -rf
        /home/root/.cache/surface-manager/qmlcache`, `rm
        /var/lib/lumaglass/lumaglass.bak`; env file must read only
        `QML2_IMPORT_PATH=/var/lib/lumaglass/qml:/usr/lib/qt5/qml`.
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

Dropped frame, defined: Home at rest presents at 30-37 fps by design
(wallpaper ticks), so an interval of two vblanks is not a miss by itself.
The analyser tells the two apart by what asked for the frame: a render
burst that follows the previous commit after Qt's 5 ms update timer was
requested by the animation driver (60 Hz wanted); one after a longer idle
was requested by a slower timer (wallpaper or clock, 30 Hz or less
wanted). "Dropped" below counts animation-driven frames that took two or
more vblanks. Workloads: `docs/research/perf/run.py` drives them with HA
remote keys and luna launches; each capture is one workload run.

## Baseline (stock settings, 2026-10-24; wallpaper motion 30 fps)

Three runs each unless noted. "anim" = frames the animation driver asked
for, "dropped" = those that took 2+ vblanks, "render" = GUI-thread CPU
per animation frame (p90), "GPU" = fragment+tiler busy per presented
frame. Compositor scheduling latency never exceeded 3.7 ms in any window
and was under 1.3 ms in most; no window shows direct reclaim, CMA
allocation or block I/O inside a gap.

| workload | fps | anim | dropped | render p90 ms | worst ms |
|---|---|---|---|---|---|
| Home idle 10 s | 36.4-36.6 | 83-108 | 10-12 (9-13 %) | 1.87-1.88 | 33.4 (one run 50.1 x4) |
| D-pad sweep (18 keys, 6.7 s) | 53.0-53.9 | 132-160 | 13 (8-10 %) | 2.08-2.22 | 50.1 |
| Quick Settings open (9 opens) | 44.7-48.3 | 18-22 | 10-13 (48-65 %) | 2.6-3.2 | 50-66.7 |
| Quick Settings close (9) | 41.7-48.3 | 13-44 | 3-7 (14-30 %) | 1.8-2.6 | 33.4-50.1 |
| Settings overlay open, 7 s | 28.7-39.5 | 36-63 | 6-13 (17-23 %) | 2.0 | 50.1 |
| Home after Settings overlay, 4 s | 38.5-40.7 | 35-43 | 4-6 (9-17 %) | 1.7-1.9 | 33.4-66.7 |

The per-frame GPU cost is 7.6-8.1 ms in every window (fragment pass of
the whole 1080p scene), the commit 0.6-0.8 ms, and the Qt update timer
5.0 ms, so an animation frame meets its vblank only when its CPU render
stays under about 2.4 ms. Every dropped frame in every baseline trace had
a render of 2.0-4.9 ms (median of all animation frames: 1.65-2.1 ms).
Nothing else moves in a gap: in the d-pad and Quick Settings gaps the
other three cores are 90 % idle, and the compositor's wait for a CPU is
at most 1.3 ms.

Quick Settings' open transition is the worst case because its frames
(LG's QML plus LumaHome still ticking underneath) render in 2.0-3.2 ms,
so about 6 of every 10 are late; it does not matter that the panel is
resident. Settings is a transparent overlay: Home keeps drawing under it
at 36 fps for the whole 7 s it is open.

| workload | fps | anim | dropped | render p90 ms | worst ms |
|---|---|---|---|---|---|
| Browser (full-screen card) in front, 7 s | 6.7-12.3 | 2-38 | 2-7 | 0.9-2.2 | 400-1700 (the browser's own first paints) |
| Home after the browser, 4 s | 35.2-38.0 | 29-35 | 4-7 (12-24 %) | 1.7-3.4 | **116.8** (run 1), 50.1, 50.0 |
| YouTube playing, HOME pressed, first 1 s | 38.9 | 11 | 6 (55 %) | 5.6 | 50.0 |
| Home over YouTube video, idle 6 s | 35.7-35.8 | 45-52 | 4-5 (8-11 %) | 1.9 | 33.4-50.1 |
| Home over YouTube video, d-pad 10 keys | 51.0-51.2 | 73-81 | 8 (10-11 %) | 2.3 | 33.4-50.0 |

## Root causes of the Home-return gaps

Traced frame by frame in `base-appfull-0` (first browser launch after the
reboot, Home back after 7 s; the 116.8 ms gap) and `base-video-1` (Home
over YouTube; six of the first eleven frames dropped):

1. Page-cache eviction of the compositor itself. The sampler's
   `/proc/<pid>/stat` shows the compositor taking about 1,400 major
   faults around the browser launch and the return (7,447 -> 8,857 in
   4 s); in the next two runs, with the browser's pages already cached,
   it took 23 and 6. Inside the 116.8 ms gap the GUI thread is in D state
   on `kworker/1:1H` (dm-verity) a dozen times and 2.7 MB is read from
   the rootfs (`mmcblk0p28`) plus 0.56 MB from `/var` (`p58`); 348 block
   requests land in the 4 s window. With 240 MB free and swappiness 100,
   a 70 MB app launch evicts the compositor's mapped libraries and QML
   data, and the first frames after the return page them back in at
   about 0.5 ms per fault on the GUI thread. The earlier 57-87 ms gaps
   are the partial-eviction case.
2. Texture rebuild on show. Before the first real frame the compositor
   runs four short GPU jobs of 4 ms (the blur chain and other
   `ShaderEffectSource`s re-rendering) and its `QQuickPixmapReader`
   thread decodes images from `/var` while the GUI thread blocks on
   reads of its own: textures of the hidden Home are not kept, so the
   return re-decodes icons and re-bakes the wallpaper blur. That is the
   50 ms (3 vblank) return seen in every run even with a warm cache, and
   the 5.6 ms renders of the first Home frames over YouTube.
3. Nothing else: no direct reclaim or CMA allocation in any gap, no
   kswapd on the compositor's CPU inside a gap, compositor scheduling
   latency under 1 ms in the gaps, GPU jobs of normal length, no
   hot-plug inside the gaps (CPU3 came and went 0-2 times per window,
   each unplug a 0.5-1 ms `migration/3` stop on the other cores).

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
| T9 | `QT_QPA_UPDATE_IDLE_TIME=0` (no 5 ms wait before each animated frame) | env file + compositor restart | remove, restart |

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

## Finding: a fixed 5 ms idle in every animated frame (Qt's update timer)

In the idle trace, during the second hand's 160 ms animation (60 fps), the
GUI thread returns from the swap and then sleeps exactly 5.0 ms before
rendering the next frame, every frame (17.59 -> 22.60, 1.16 -> 6.15,
34.61 -> 39.65 ms in the timeline above). That is Qt 5's default
`QPlatformWindow::requestUpdate()`: the basic render loop schedules the
next animation frame through `requestUpdate()`, eglfs does not override
it, and the default posts the update request after
`QT_QPA_UPDATE_IDLE_TIME` ms, 5 by default ("to give the event loop a bit
of idle time").

Because the basic loop is serial, the 5 ms sits inside the frame budget:

| step | ms |
|---|---|
| flip done -> GUI thread -> driver thread -> GPU start | 0.4 |
| GPU tiler + fragment (idle Home) | 8.1 |
| job IRQ -> GUI thread, atomic commit | 0.8 |
| **Qt update timer** | **5.0** |
| render next frame on CPU (sync + scene graph render) | 1.64 median, 2.0 max |
| slack before the next flip | 1.05 median, 0.16 min |

Slack measured over the 33 chained 60 fps frames of the trace (renders
that follow a commit by the 5 ms timer); GPU fragment 7.55 ms in all 206
frames (p95 7.56), tiler 0.47. At idle the chain uses about 15.7 of
16.7 ms. Each frame whose CPU render plus GPU pass exceeds about 10.7 ms
makes the next one late by the excess; the
lateness accumulates frame over frame, and once it passes about 7.7 ms a
frame misses its vblank (a 33 ms interval), which resets the chain. So a
focus move or panel open whose frames need 3.5-4 ms of CPU render drops a
frame every 6-10 frames. `QT_QPA_UPDATE_IDLE_TIME=0` returns the 5 ms to
the budget (chain about 10.7 ms at idle). To be measured as T9. It is set
in the compositor's env file through the tool's new `env.extra` hook
(the decided `QML_FORCE_DISK_CACHE=1` goes through the same env line).

## Finding: why idle Home runs at an uneven 36-38 fps

Two unrelated clocks request frames:

- `liveWall`'s Timer at `Math.round(1000 / 30)` = 33 ms. Its frames land
  on alternate vblanks (33.4 ms); the timer runs 0.36 ms short of two
  vblanks, so every 46 ticks (1.5 s) its phase slips past a vblank and one
  interval becomes 50 ms: the 2 intervals over 33 ms in 5 s of idle, a
  visible hitch in the wallpaper motion.
- The analog clock's second hand (`ClockWidget.qml`, 1 s Timer, a 160 ms
  `OutBack` NumberAnimation on the angle): 160 ms of 60 fps every second,
  about 9.6 extra frames a second, out of phase with the wallpaper ticks.

Measured intervals in 5 s: 106 of two vblanks, 80 of one, 2 of three:
150 wallpaper frames plus 39 clock frames = 189. At the decided 10 updates
a second the idle rate becomes about 10 + 9.6, still uneven unless both
run off one clock: one Timer at 100 ms that advances the wallpaper phase
and the second hand together (the hand then snaps per tick or keeps its
160 ms ease, which is then the only 60 fps stretch), with the wallpaper
phase taken from elapsed time rather than tick count so a late tick does
not slow the motion.

## Background load at idle Home (5.5 s trace, probe processes excluded)

| thread | % of one core | wakeups/s | what |
|---|---|---|---|
| surface-manager (GUI thread) | 10.7 | 133 | the compositor |
| pq_thread | 5.5 | 31 | LG picture-quality kernel thread |
| tAiToneService | 2.1 | 49 | LG AI picture/sound tone |
| servicemanager | 1.8 | 2.5 | LG service manager |
| MainWebModule + Rasterizer | 2.0 | 208 | a background WebAppMgr app |
| ls-hubd | 1.3 | 3.4 | luna bus (partly the probe's own luna calls) |
| mrcu_ls2 | 1.0 | 99 | Magic Remote service, RR 99 |
| lginput2 | 1.0 | **1969** | input daemon, polls every 0.507 ms, RR 99 |
| memchute | 0.9 | 87 | LG low-memory manager (kills apps) |
| kswapd0 | 0.9 | 2.7 | reclaim with 133 MB free, swappiness 100 |
| mali-cmar-backe | 0.9 | 209 | Mali driver's submit thread (compositor) |
| node | 0.7 | 4 | Glasshouse |
| cobalt | 0.2 | 51 | preloaded YouTube |

Everything outside the compositor adds up to about 25 % of one core of
four; the compositor's GUI thread sees at most 0.21 ms of scheduling
latency. Background daemons do not delay idle frames. Stopping them is a
memory and power question (each wakeup keeps a core out of idle), not a
frame-rate fix at rest; whether they matter under load is measured in the
app-to-Home runs. `lginput2`'s 2 kHz poll at RT 99 is a replaceable cost
(LG input stack, about 1 % of a core and 2,000 timer interrupts a second
on CPU2); it does not reach the compositor.

## Recommendations so far (provisional: from one idle trace and the code; before/after numbers pending the TV window)

1. `QT_QPA_UPDATE_IDLE_TIME=0` for the compositor (T9). Expected: the
   5 ms Qt update timer leaves every animated frame, so a frame's CPU
   render can grow from about 2.6 ms to about 7.6 ms before lateness
   accumulates; the main candidate for the missed deadlines during
   focus moves and panel opens. Kept, it becomes a fixed line of the
   tool's `env_line`, like the disk cache.
2. One clock for idle Home: wallpaper phase and second hand advanced by
   one 100 ms Timer (the decided 10 updates a second), wallpaper phase from
   elapsed time. Expected: idle drops from 37.7 to about 10-16 presented
   frames a second, the 50 ms wallpaper hitch every 1.5 s disappears, and
   idle GPU busy falls from 30 % to about 8-13 %.
3. Not worth doing, from the platform facts: CPU governor or minimum
   frequency (one 1.4 GHz OPP, already `performance`); GPU `always_on`
   (power-up is 40 us before each job and never on the critical path);
   stopping LG daemons for frame rate at rest (all background work is
   about 25 % of one core and the compositor waits at most 0.21 ms for a
   CPU). To be confirmed under load in the T1/T2 runs.
4. Small: the frame probe's `onFrameSwapped` handler runs JS on every
   frame even with the probe off; guard the Connections with
   `enabled: root.fpsEnabled`.

## LG layers whose replacement (not tuning) would remove a measured cost

- Serial frame pipeline in the compositor (Qt 5.12 basic loop on LG's
  `eglfs_starfish`): the GUI thread blocks for the whole GPU pass
  (8.1 ms of each idle frame) and does the atomic commit only after the
  GPU finishes, so CPU and GPU work add instead of overlapping. A
  compositor that commits with an in-fence (`IN_FENCE_FD`) and renders
  ahead would give the CPU and the GPU 16.7 ms each. Tuning gets part of
  it: T9 removes the 5 ms timer, T7 (threaded loop) moves the blocking
  off the GUI thread.
- Full-scene redraw per frame: Qt Quick 2 has no damage regions, so a
  4 px wallpaper ripple re-renders all 1920x1080 (7.5 ms fragment, 30 %
  of the GPU at 30 updates a second). A compositor with damage tracking,
  or the wallpaper on its own hardware plane (the lg1k has an overlay
  plane used once since boot), would cost a fraction of that.
- `lginput2`: 2 kHz poll at RT 99, about 1 % of a core and 2,000 timer
  interrupts a second; event-driven input would cost close to nothing.
  CPU only, no frame cost measured.
