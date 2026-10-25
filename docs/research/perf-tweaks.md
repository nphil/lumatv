# LumaTV performance tweaks: measurements on the C4

Measured on the OLED65C4PUA (webOS 10.2.1, kernel 5.4.268) on 2026-10-23.
Rule under test: every frame on its 60 Hz deadline, at rest and in motion.

## Resume here

Status 2026-10-24 20:35: done. All measurements taken, every tweak
reverted, the set is in Active Standby, Filmmaker, with nothing of this
work left on it (old tool restored, env file back to its single line,
`env.extra`, backups, QML cache and all `/tmp` probe directories
removed; wallpaper motion back to 30 fps; compositor pid 12279 from the
last tool restart at 20:26). Raw traces, `summary.txt` and per-run
analyses are in `/tmp/perf` on the workstation only.

Harness (in `docs/research/perf/`): `run.py --tag T --repeat N
--workloads idle,dpad,qs,apphome,appfull,video,coldhome,pressure,hog
[--apply CMD --revert CMD] [--start K]` drives the workloads with HA
keys and luna launches; `capture.sh` (copy to `/tmp/perf` on the set)
records the ftrace and the 1 s sampler; `analyse.py TRACE --sm PID
--gap MS` gives the frame stats, the dropped-frame classification and
the per-gap attribution; `tweak.py` sets the compositor's scheduling
policy and affinity; `pin.py` mlocks its file pages. HA key presses
fail with HTTP 500 now and then; `run.py` retries six times.

Open items for whoever continues: the X1 `te` run with a trace that
fits the ring (basic events only); the hog workload again once the
preload trim and the Home stub are in; Netflix 24p output rate; and the
one-clock change to idle Home (recommendation 2).

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
  `QML_FORCE_DISK_CACHE=1` is decided and ships in phase 1),
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

## Tweak results

Each row: same workloads as the baseline, 2 runs; "dropped" as defined
above. "cache" is the control pass with the new tool (disk cache on,
compositor restarted) and matches the stock baseline.

### T9: `QT_QPA_UPDATE_IDLE_TIME=0` (KEEP)

Applied through `env.extra` + `lumaglass apply` (compositor restart).

| workload | base / cache | T9 |
|---|---|---|
| D-pad: dropped of animation frames | 15/187, 17/183 (8-9 %) | 12/297, 10/294 (3.4-4.0 %) |
| D-pad: fps, worst | 53.1-53.9, 33-50 ms | 54.4-54.8, 33.4 ms |
| QS open: dropped | 11-15 of 18-28 (46-67 %) | 2-4 of 80-103 (2.5-4.0 %) |
| QS open: fps | 45.9-47.1 | 49.9-50.7 |
| QS close: dropped | 6-7 of 21-49 (12-29 %) | 2-4 of 48-65 (3.4-8.2 %) |
| Idle: dropped (clock hand) | 12-13 of 94-96 (13 %) | 7-11 of 96-106 (7-10 %) |
| Idle fps | 36.4-36.7 | 37.0-37.2 |
| Home over YouTube, d-pad: dropped | 8/73-81 (10-11 %) | 4-5 of 46-56 (7-11 %) |

With the timer gone the frame chain is flip -> GPU 8.1 -> commit 0.8 ->
render, so a render can take about 7 ms instead of 2.4 before the next
vblank is missed. The animation-driven frame counts rise because the
renderer now keeps up with every request (Quick Settings runs a
continuous 60 Hz animation while open, which the baseline served at 45
fps). Remaining drops have renders of 4.7-6 ms or an extra GPU pass.
Risk: none seen in 25 min of use (keys, panels, app launches, video);
Qt documents the 5 ms only as idle time for event gathering. Ships in
phase 1 with the disk cache, as a fixed line of the tool's `env_line`
(Main, 2026-10-24); until then nothing stays on the set. T9 stayed
applied for T6, T2, T10 and T1-T5 (all measured against T9 alone) and
was removed with T7's revert at 20:10.

### T6: wallpaper motion 30 -> 10 fps, and off (KEEP 10 fps, as decided)

Live `theme set`, measured with T9 applied; reverted to 30 fps after.

| | 30 fps (T9) | 10 fps | off |
|---|---|---|---|
| Idle: presented fps | 37.0-37.2 | 20.4 | 12.7-13.1 |
| Idle: GPU busy per 10 s | 2991-2995 ms (30 %) | 1641-1651 ms (16.5 %) | 917-925 ms (9 %) |
| Idle: compositor CPU per 10 s | 1028-1055 ms | 552-570 ms | 352-361 ms |
| Idle: dropped clock-hand frames | 7-11 of 96-106 | 2 of 80 (2.5 %) | 1-3 of 69-79 |
| D-pad: dropped | 10-12 of 294-297 (3.4-4.0 %) | 6-12 of 265-277 (2.2-4.5 %) | 9-10 of 269-277 (3.2-3.7 %) |
| D-pad: GPU per presented frame | 8.07 ms | 8.06 ms | 7.18 ms |

The wallpaper costs 13.5 % of the GPU and 4.6 % of a core at 30 fps and
nothing in dropped frames: animation frames redraw the whole scene
anyway, and the warp pass itself is 0.9 ms of the 8.1 ms GPU frame
(motion off: 7.2 ms). 10 fps is the decided setting; the remaining idle
frames are the second hand (80 a second-hand frames per 10 s). The
"worst interval" column is meaningless at 10 fps and below: nothing is
presented while nothing changes.

### T2: LG core hot-plug off (`mp_enable=0`, CPU3 held online) (OPTIONAL)

Measured with T9 on; reverted after (`mp_enable=1`).

| | T9 alone | T9 + T2 |
|---|---|---|
| Idle: dropped | 7-11 of 96-106 (7-10 %) | 3-7 of 93-103 (3-7 %) |
| D-pad: dropped | 10-12 of 294-297 (3.4-4.0 %) | 8-13 of 293-307 (2.7-4.2 %) |
| QS open: dropped | 2-4 of 80-103 (2.5-4 %) | 0-4 of 83-102 (0-3.9 %) |
| hot-plug events per 10 s | 1-2 | 0 |

Within run-to-run noise except a small gain at idle. Each unplug is a
`stop_machine` of about 1 ms on every core (the `migration/3` time in
the traces is the time CPU3 spends offline: 57-72 % of idle windows),
and a plug a secondary-core boot; neither landed inside a dropped
frame in any trace. Worth keeping only for predictability, and it
raises idle power by keeping a fourth core awake. Not reboot safe
(procfs write; would go into the boot hook).

### T10: pin the compositor's file pages, and its cgroup `memory.swappiness=0` (NOT ON THEIR OWN)

The eviction case made repeatable: browser in front, a 450 MB hog
(`run.py` workload `hog`) touches memory for 8 s and exits, then HOME.
`drop_caches` does not reproduce it (it leaves mapped pages alone: 0
faults). Numbers are the compositor's major faults over the run and the
worst Home-return interval:

| | run 1 | run 2 |
|---|---|---|
| hog, nothing protected | 4,588 faults, 367 ms | 2,035 faults, 250 ms |
| hog + `pin.py` (63 MB of the compositor's resident file pages mlocked by a helper) | 1,160 faults, 567 ms | 270 faults, 417 ms |
| hog + pin + compositor cgroup `memory.swappiness=0` | 1,264 faults, 918 ms | 1,359 faults, 134 ms |

The pin removes 75-87 % of the compositor's own faults, and the return is
no better, because the gap is no longer the compositor: at 450 MB of
pressure LG's memory manager (`memchute`, log at 00:03:43 UTC) moved to
level "medium" and closed Netflix, YouTube and **LG's Home app**; the
HOME key then relaunched LG Home (Flutter, 36 MB read back through
dm-verity, 0.9-2.8 s with nothing presented), and the preload manager
relaunched YouTube, Netflix, Live TV and HDMI2 in the background within
the next 30 s. The compositor's own share of a return under pressure is
the 1,000-4,600 faults (0.5 ms each, on the GUI thread) and its heap
coming back from the compressed swap (`VmSwap` 25 MB).

What follows for the plan: memory headroom is the fix, not pinning.
Dropping the YouTube, LG Channels and browser preloads (decided, about
307 MB) and replacing the 67 MB Flutter Home with the 26 MB stub (phase
1) together keep the set about 350 MB further from the point where an
app launch evicts the compositor or memchute closes Home. The cgroup
swappiness and the pin are cheap and could follow (the tool's boot hook
could write the cgroup value; the pin would live in the native helper),
but only after the headroom, since both move the pressure onto the apps.
Reverted: pin process killed, `memory.swappiness` back to 60.

Side finding from the cold runs: a web page in the browser switched the
output to **50 Hz** (vblank 20.0 ms) for as long as it was in front; the
UI plane follows the video pipeline's output rate, so over 50 Hz or 24 Hz
content the UI's frame budget is 20 or 41.7 ms and "60 fps" is not on
offer. To be measured with Netflix 24p content (not started here).

### T1, T3, T4, T5: GPU `always_on`, IRQ affinity, SCHED_FIFO, CPU pinning (REJECT: no effect)

One run each of d-pad and Quick Settings, with T9 on, each reverted
after. T9 alone: d-pad 3.4-4.0 % dropped, QS open 2.5-4.0 %.

| tweak | d-pad dropped | QS open dropped | compositor max sched latency |
|---|---|---|---|
| T1 `power_policy=always_on` | 11/301 (3.7 %) | 1-3 of 90-98 (1-3 %) | 0.6-5.5 ms |
| T3 GPU IRQs 176/178 -> CPU1, eth0 -> CPU2 | 8/276 (2.9 %) | 2-3 of 87-103 (2-3 %) | 1.9-4.2 ms |
| T4 GUI thread SCHED_FIFO 1 | 8/296 (2.7 %) | 1 of 90-98 (1 %) | 0.02-0.04 ms |
| T5 GUI thread pinned to CPU1 | 10/298 (3.4 %) | 2-5 of 103-123 (2-4 %) | 0.1-0.8 ms |

All within run-to-run noise. FIFO removes scheduling latency entirely,
which confirms it was never on the critical path (0.2-5 ms worst, and
never inside a dropped frame). The GPU's power gating costs 40 us before
each job and `always_on` changes nothing in the frame. Moving the
interrupts off CPU0 changes nothing because the GUI thread migrates
freely and CPU0's interrupt load is about 1 ms per 33 ms.

Observation on the way: the unit starts the compositor at `Nice=0`; LG
renices it to -19 a few minutes after start (a restarted compositor read
nice 0 at 1 min and -19 at 10 min). The runs above were all at -19.

### T7: `QSG_RENDER_LOOP=threaded` (REJECT: black screen)

Applied alone through `env.extra` + `apply`. The compositor started
with two `QSGRenderThread`s, logged `QEGLPlatformContext: eglMakeCurrent
failed: 3002` (EGL_BAD_ACCESS: LG's compositor keeps its GL context
bound on the GUI thread for its own surface handling) once a second and
presented nothing: 0 frames in 4 s, capture 6.9 kB. Reverted by removing
`env.extra` and applying again; the tool's Home relaunch gave up after
three attempts and 101 s ("home did not come back"), and one manual
`applicationManager/launch` of `com.webos.app.home` brought it back.
The basic loop is the only loop this compositor runs; T9 is the way to
take the 5 ms out of it.

### T8: global swappiness (NOT RUN)

Superseded by the cgroup test under T10: the compositor's own cgroup
has its own `memory.swappiness`, and the global value also governs the
apps, whose swapping is what frees memory for a launch.

## Stability

From the 1 s sampler across the whole session (74 runs, 2 h):

- Compositor RSS: the boot compositor went 114 -> 123 MB during the
  first Quick Settings opens (the resident panel) and the first app
  returns, then stayed at 122-123 MB across 6 Settings returns, 3
  browser returns and 2 YouTube runs. The compositor restarted with the
  disk cache ran at 144-155 MB (its predecessor had 27 MB in swap). After
  the hog runs RSS sat at 101-106 MB with the rest swapped out and never
  faulted back: swapped compositor pages stay swapped until touched, so a
  pressure event leaves a slower compositor behind it.
- No OOM kills by the kernel in `dmesg`; LG's `memchute` closes apps
  itself at its "medium" level (above), Home included.
- SoC temperature 55-62 C throughout, no thermal event, CPU fixed at
  1.4 GHz.
- CMA free fell to 0-3 MB during every YouTube run and most app runs
  (video decode plus app surfaces), from 17-54 MB with Home alone. Any
  LumaTV use of contiguous memory (the 64-bit path's dumb buffers) will
  contend with playback; the plan's check before relying on CMA stands.
- The crash reports each compositor restart produces were swept by the
  tool (2-3 per restart).

## Compositor floor (for the shell-architecture study)

From the same traces, per presented frame (CPU = compositor GUI-thread
time in the window divided by presented frames; GPU = Mali busy):

| situation | presented fps | CPU per frame | GPU per frame |
|---|---|---|---|
| LumaHome idle (wallpaper 30 fps + clock) | 36.5 | 2.8 ms | 8.0 ms |
| LumaHome, d-pad animations | 53-55 | 3.0 ms | 8.1 ms |
| LumaHome over YouTube video, still | 35.7 | 2.8 ms | 8.0 ms |
| LumaHome over YouTube video, d-pad | 51 | 3.0 ms | 8.0 ms |
| Browser (full-screen client) in front, Home hidden | 7-12 | 5.1 ms | 6.3 ms |
| Settings (overlay client) over Home | 38-40 | 3.3 ms | 8.6 ms |

With a client in front and Home hidden the compositor still spends 6.3
ms of GPU and 5 ms of CPU per presented client frame: that is the cost
of compositing one full-screen surface through Qt Quick on this driver,
and it is what direct scan-out of a client buffer (a plane per client,
no composition pass) would remove. The idle-Home 8.0 ms is the full
scene; the wallpaper warp is 0.9 ms of it (T6).

X1, the floor itself (ShellArchitecture's 64-bit client: one clear and
one moving 240x480 rect per frame, full-screen overlay over idle Home,
wallpaper motion off, `eglSwapInterval 1`), counted with the DRM probe:

| buffers | run | client swaps | presented frames | 2-vblank frames | worst |
|---|---|---|---|---|---|
| DRM dumb (CMA, all three from CMA) | 60 s | 3,075 = 51.2 fps; swap gaps: 53 under 17.5 ms, 2,466 at 17.5-20, 555 at 20-33.5, 0 over | 3,086 (51.2 fps) | 530 of 3,085 (17 %) | 33.4 ms |
| `dma_buf_te` (ordinary pages) | 20 s | 959 = 47.9 fps; 631 at 17.5-20, 312 at 20-33.5, 2 over 33.5 | (full trace overflowed) | | 49.3 ms (client clock) |

So LG's compositor does not hold 60 fps for a trivial client either:
about one presented frame in six takes two vblanks, with the compositor
doing nothing but composing that one surface over a still Home (Home
itself presented 58 fps with motion off, because the client's overlay
keeps the scene animating). After the client exited the compositor ran
at 685 ms of CPU and 1284 ms of GPU per 3 s (3.9 ms CPU and 7.4 ms GPU
per presented frame) while Home recovered. The earlier PLAN.md figure
(60.2-60.8 fps, 2-10 over 20 ms in 8 s) was measured with the client's
own clock over 8 s; over 60 s and counted at the panel the floor is
51 fps with 17 % of frames late. This is the compositing cost of option
B's host, not of the client.

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

## Tweak plan (each applied alone, measured, reverted) and verdicts

| id | tweak | apply | revert | verdict |
|---|---|---|---|---|
| T1 | GPU cores never power-gated | `echo always_on > /sys/class/misc/mali0/device/power_policy` | `echo coarse_demand > ...` | no effect |
| T2 | LG core hot-plug off | `echo 0 > /proc/lg/pm/mp_enable; echo 1 > /sys/devices/system/cpu/cpu3/online` | `echo 1 > /proc/lg/pm/mp_enable` | optional, idle only |
| T3 | GPU and network IRQs off CPU0 | `echo 2 > /proc/irq/{176,178}/smp_affinity; echo 4 > /proc/irq/124/smp_affinity` | `echo f > ...` (stock mask) | no effect |
| T4 | Compositor GUI thread SCHED_FIFO | `tweak.py fifo 1` | `tweak.py other` | no effect |
| T5 | Compositor GUI thread pinned to CPU1 | `tweak.py affinity 2` | `tweak.py affinity f` | no effect |
| T6 | Wallpaper motion 30 -> 10 fps, and off | `lumaglass theme set '{"wallpaperMotion":{"fps":10}}'` (live) | `... 30` | keep 10 fps |
| T7 | `QSG_RENDER_LOOP=threaded` | `env.extra` + `apply` | remove, `apply` | black screen |
| T8 | global swappiness 100 -> 20 | `echo 20 > /proc/sys/vm/swappiness` | `echo 100 > ...` | not run (see T10) |
| T9 | `QT_QPA_UPDATE_IDLE_TIME=0` | `env.extra` + `apply` | remove, `apply` | keep |
| T10 | pin compositor file pages; its cgroup `memory.swappiness=0` | `pin.py &`; `echo 0 > .../surface-manager-daemon.service/memory.swappiness` | kill `pin.py`; `echo 60 > ...` | not alone |

None survives a reboot by itself; a kept tweak goes into the tool's apply
and boot hook (sysfs/procfs writes at boot-bind) or the env file.

## The four hypotheses for the Home-return gaps, as measured

- Direct reclaim inside the compositor: no `mm_vmscan_direct_reclaim`
  with the compositor's pid in any gap. Not it.
- Compositor pages swapped or evicted while the app was in front, faulted
  back on show: yes, the main cause (1,000-4,600 major faults per
  pressured return, D-state waits on dm-verity reads in the gap).
- LG compositor work on the departing app (recents capture, relayout):
  not seen on the GUI thread in the gaps; the extra work there is
  LumaHome's own texture rebuild (four 4 ms GPU jobs, pixmap reader
  busy), the 50 ms third vblank on every warm return.
- LumaHome's show path (theme poll, feeds, clock catch-up): 1-3 ms of
  render on the first frames, inside the budget once T9 is on.

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
the budget (chain about 10.7 ms at idle); measured as T9 above. It is
set in the compositor's env file through the tool's `env.extra` hook
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

## Ranked recommendations

1. **`QT_QPA_UPDATE_IDLE_TIME=0` (T9). Keep.** Dropped animation frames:
   d-pad 8-9 % -> 3.4-4.0 %, Quick Settings open 46-67 % -> 2.5-4 %,
   close 12-29 % -> 3-8 %, idle clock 13 % -> 7-10 %; QS animates at 50
   fps instead of 45. One line in `env.extra` today; make it a fixed
   line of the tool's `env_line` like the disk cache. No side effect
   seen.
2. **Wallpaper motion at 10 fps (T6, decided). Keep.** Idle GPU 30 % ->
   16.5 %, compositor CPU 10.3 % -> 5.6 % of a core, idle clock-hand
   drops 7-10 % -> 2.5 %. Then drive the second hand and the wallpaper
   from one 100 ms clock (wallpaper phase from elapsed time) so idle Home
   presents 10 frames a second instead of 20.
3. **Memory headroom before anything else that touches memory.** The
   Home-return gaps (50-117 ms normally, 250-920 ms under a 450 MB
   launch) are the compositor paging itself back in (1,000-4,600 major
   faults) and, past that, LG's memory manager closing Home and the
   preloads relaunching. Dropping the YouTube, LG Channels and browser
   preloads (decided, about 307 MB) and the 26 MB Home stub for the
   67 MB Flutter Home (phase 1) are the fixes; measure the hog workload
   again after them. Pinning the compositor's file pages (T10) and
   `memory.swappiness=0` on its cgroup cut its faults by 75-87 % but did
   not shorten the return on their own; they are a cheap second layer
   once the headroom exists.
4. **Render under 7 ms per animation frame, GPU under 8 ms.** With T9 the
   chain is flip -> GPU 8.1 -> commit 0.8 -> render -> next flip; the
   remaining drops all had renders of 4.7-6 ms or an extra GPU pass
   (texture rebuild on show). The gate for new surfaces: p90 render
   under 3 ms, no `ShaderEffectSource` re-render on show (keep the blur
   and icons resident while Home is hidden: the 50 ms third vblank on
   every return is their rebuild).
5. **Hot-plug off (T2): optional.** Idle drops 7-10 % -> 3-7 %, nothing
   elsewhere; costs idle power. If wanted, `echo 0 > /proc/lg/pm/mp_enable`
   from the boot hook.
6. **Reject:** GPU `always_on` (T1), IRQ affinity (T3), SCHED_FIFO (T4),
   CPU pinning (T5): no change, scheduling latency was never in a dropped
   frame. `QSG_RENDER_LOOP=threaded` (T7): black screen, EGL_BAD_ACCESS.
   CPU governor: nothing to tune (one OPP). Stopping LG daemons for
   frame rate: all background work is about 25 % of one core and never
   in a gap; it is a memory and power question.
7. Small: guard the frame probe's `onFrameSwapped` Connections with
   `enabled: root.fpsEnabled` (JS per frame while off); fix the tool's
   Home relaunch after a restart that gives up at 101 s (one luna launch
   brought Home back).

## LG layers whose replacement (not tuning) would remove a measured cost

- Compositing of app frames: with a full-screen client in front and Home
  hidden, the compositor spends 6.3 ms of GPU and about 5 ms of CPU per
  presented client frame on this driver. A compositor that scans client
  buffers out on their own plane (the lg1k has an unused overlay plane)
  would spend none of it, and the UI's 8 ms frame would no longer sit in
  series with the app's.
- Serial frame pipeline (Qt 5.12 basic loop on `eglfs_starfish`): the
  GUI thread blocks for the whole 8.1 ms GPU pass and commits only after
  it, so CPU and GPU add up instead of overlapping, and the threaded loop
  does not work in this compositor. A compositor committing with an
  in-fence and rendering a frame ahead gives the CPU and the GPU 16.7 ms
  each; T9 recovers the 5 ms Qt timer but not this.
- Full-scene redraw per frame: Qt Quick 2 has no damage regions, so a
  4 px wallpaper ripple re-renders all 1920x1080 (7.5 ms of fragment
  work; the warp itself is 0.9 ms). Damage tracking or the wallpaper on
  its own plane cuts the idle GPU cost by about 8x.
- Output rate follows the video pipeline: a browser page put the UI
  plane at 50 Hz for as long as it was in front (vblank 20.0 ms). A UI
  of its own would still have to live with 50 and 24 Hz output while
  such content plays; "60 fps everywhere" needs a plane that is not
  slaved to the content rate, if the SoC offers one.
- `lginput2`: a 2 kHz poll at RT 99, about 1 % of a core and 2,000 timer
  interrupts a second. CPU and power only; never in a dropped frame.
- LG's memory manager and preloads: 307 MB of warm apps and a Home that
  memchute closes under pressure are what turn a launch into a 250-920 ms
  Home return; LumaTV owning Home and the preload list removes both.
