# UI at 120 Hz: the refresh test mode

Can Home and LumaTV's menus draw at 120 fps under LumaTV's compositor with no dropped frame? The mode is built and measurable
(`--refresh 120`, `ctl refresh 120`, `tv_gate.py --only frames --refresh 120`), off by default. Round 14 ran it: the output runs at 120 and
the compositor paces to it; the shell's slides came at 60 fps on the 120 Hz grid, and the cause is the shell's animation clock, not its
cost (the section "The shell's cadence" below; fixed offline, a second TV run decides).

## Expected verdict [INFERENCE]

The output does run at 120 Hz with one module parameter, and the compositor paces to it. The shell client does not keep up as it
is: a frame of Home costs 11-13.5 ms of GPU at 60 Hz against a budget of about 7.5 ms at 120, so every animated frame would take two
refreshes (60 fps shown on a 120 Hz grid, each flip a refresh late). With the 120-only cut built here (the widgets and the dock drawn
from a cached texture each while a panel slides over Home) the quick panel's and the picker's slides are expected to fit; the D-pad's
focus moves on the dock are not (the dock itself animates, so it cannot be cached), and the toast is the system layer composed over
Home by the compositor (two full-screen layers, about 2 ms more GPU). Expected result of the TV check: `GATE refresh` at 120 Hz with
flips, `DE2VSYNC` and `osd_irq` all near 120 a second at idle Home; `quick` and `picker` with no interval over 10 ms if the estimate of
5-6 ms a frame holds; `home-dpad` failing on `burst_over20` (intervals over 10 ms); `toast` undecided. Idle Home costs the same as at 60
(the wallpaper ticks at 10 Hz either way).

What the TV run decides: the GPU time of a slide frame with the cache (the estimate is from the parts, not measured), whether the
Qt client makes a frame per callback at 8.33 ms spacing or inserts its own 5 ms update timer (`QT_QPA_UPDATE_IDLE_TIME`,
`perf-tweaks.md` T9: measured on LG's eglfs, not on the wayland client), Mali busy and the SoC temperature over a slide at 120, and
whether the kernel takes the divider write at all (the gate reads the file back).

## How the output runs at 120

- The panel and the display engine already run at 120 Hz at Home: `DE2VSYNC` and `osd_irq` 120 a second (`hardware-census.md`,
  `lpc-tv-result.md`), `ovi/status` `dispFramerate 3` (the 144 Hz option, `gfn-444-vrr-result.md`). The OSD plane latches a new
  framebuffer at every one of those vsyncs (SRC `chip/o22/fbdev/fbdev_hw_o22_cmn.c:2796`, `osd_hdr_ready_flag`).
- What runs at 60 is the page-flip completion the pacer keys to. LG's `lg1k` DRM driver hands the DRM core a vblank from
  `FBDEV_SyncWithVsync` (SRC `core/fbdev/fbdev_drv.c:2936-2997`, `osd_drm_vsync_tick` to `drm_crtc_handle_vblank`, `core/drm/drm.c:545`),
  which returns early on every second OSD vsync: `++g_fbdev_vsync_counter % g_fbdev_vsync_cnt_on_flip` with the module parameter at 2
  ("value 2 makes 60Hz handler (120Hz/2=60Hz)", `:546`). The parameter is `/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip`,
  mode 0644, read as 2 on the set in every frame-rate probe (`framerate/*/before.txt`). At 1 the DRM core sees every OSD vsync:
  page flips land 8.33 ms apart. The DRM path's own flip (`osd_drm_flip_image`) never waits for a vsync (`fbdev_drm.c:497-500`), so
  nothing else in the driver depends on the value.
- The KMS mode is cosmetic: the connector lists 720p, 1080p, 4K and 8K "60", the CRTC never sets panel timing, and the compositor
  commits with `ALLOW_MODESET` once (`gfn-444-vrr-result.md` (a), `framerate-result.md`). Nothing about the mode changes.
- 1080p UI over the 4K120 path: the OSD plane is 1920x1080, scaled to the panel by the OSD upscaler (`reg_4k_osd_main_ctrl_2`,
  `FBDEV_O22_SUPPORT_4KOSD_UPSCALER`) and fetched at every engine vsync whatever the divider says, so the display side's cost is the same
  at 60 and 120. What is new at 120 is the kernel's vblank handler and the compositor's page-flip events and shell callbacks at 120 a
  second.
- Under 50-family content the engine runs 100 Hz and the divider at 1 gives 100 flips a second; 24p film leaves the engine at 120
  (`framerate-result.md`). The pacer follows the flips it measures (`kms/timing.rs`, 100 and 120 added to the rates it snaps to) and
  is keyed by order at a switch, since the measured path reads 16.67 ms after 8.33 as a skipped refresh, not a slower panel.

## What limits it

The budget at 120 Hz is 8.33 ms a frame less what a flip needs before its refresh (`FLIP_LEAD_NS` 0.5 ms) and the driver's cutoff for
the in-fence (a page the GPU finishes after it slips a refresh, `kms/timing.rs`): about 7.5 ms of GPU per frame with the CPU of the
next frame overlapped, as the shell's early callbacks allow (`compositor-surfaces-result.md`, b193b28).

What the shell's frames cost at 60 Hz on the set (`compositor-surfaces-result.md`, `shell-host-result.md`):

| frame | GPU | CPU (qml-runner, one thread) | compositor |
|---|---|---|---|
| picker slide over Home | 13.0-13.5 ms in 4 jobs | 2.2-3.5 ms | under 1.1 ms, the buffer scanned out whole |
| Home in the D-pad's windows | 11.0 ms | 3.2 ms | the same |
| quick panel slide | a millisecond or two under the picker | | |
| Home idle (LG's compositor, the same QML) | 8.1-9.0 ms | 2.8-3.4 ms | |

So a slide's frame is 15.4-16.4 ms of CPU and GPU: at 120 it misses every second refresh by arithmetic, and the GPU alone is 1.5-1.8
times the budget. To fit, the GPU time of an animated frame must fall under about 7 ms: a cut of 40-50 % from 11-13.5. Mali G52 at
1080p has no slack to give: 30 % busy during a 60 fps UI (`hardware-census.md`) becomes 60 % at 120 for the same frames, and a frame
over the budget costs a whole refresh, not a fraction.

Where the time goes in a slide frame: Qt's scene graph redraws every node every frame, so the slide's frames carry all of Home (the
wallpaper quad, each widget's glass and content, the dock's tiles, the status bar) under the scrim and the sheet. The cut that follows
from that is to stop redrawing what does not move.

## What was built

Compositor (`compositor/crates/lumatv-compositor`):

- `refresh.rs`: the divider as a file (`Divider`: written for 120, the value found put back at 60 and when the compositor exits), the
  measure per flip since the last switch (`Measure`: flips one, two, three and more refreshes apart at the period in force, the worst
  interval, frames late, lost and missed by the pacer's reading, and the frame pipeline from draw to the GPU's finish as a fixed
  histogram with p50, p99 and max to a quarter of a millisecond).
- `--refresh 60|120` (`config.rs`, default 60) and `ctl refresh [60|120|reset]` (`control.rs`): the switch writes the divider first (a
  refused write changes nothing), keys the pacer to the period by order (`FlipTiming::rekey`), sets the fixed tick to it (`ticker.rs`
  reads the live config), starts the measure over and tells the shell (`refresh` topic, `{"hz": N}`); `refresh` alone reads the mode and
  the measure; `reset` starts the measure over at the mode in force.
- The supervisor (`sm/lumatv-sm.c`) puts the divider back to 2 before it execs LG's compositor on a fallback, for a compositor killed
  mid-run; the gate driver does the same in its clean-up.

Shell (`payload/compositor/lumatv`, `compositor/shell/Shell.qml`):

- `Shell.qml` hands the topic to LumaHome as `refreshHz`; `kit/Lu.qml` exposes it as `Lu.refreshHz` and `Lu.fast` (100 Hz or more).
- At `Lu.fast`, while the quick panel or the picker is on the screen or on its way (`Layers.panelSliding`), LumaHome draws its widget
  layer and its dock from one item layer each (`layer.enabled`; the dock's texture reaches past its rect for the glass shadows), rendered
  when the slide starts and again only when something in them changes (the clock's second). At 60 nothing is layered: the look is
  unchanged. Expected cost of a slide frame at 120 [INFERENCE from the parts]: the wallpaper quad about 2 ms, the two cached quads about
  1 ms, the scrim under 1 ms, the sheet (its cached content and its glass quad) 1.5-2 ms: 5-6 ms. The first frame of a slide pays the
  layers' render once (about one frame of Home).

Gate (`compositor/tools/tv-gate.sh`, `tv_gate.py`, `docs/research/perf/gate.py`, `analyse.py`):

- `gate.sh refresh [60|120|reset] [SECS]`: switches (or not) and measures SECS of the screen as it is: the OSD plane's flips, `DE2VSYNC`
  and `osd_irq` a second from `/proc/interrupts` and `/proc/lg/fbdev/fb0/flip_cnt`, the compositor's measure, Mali's busy share from
  `mali0/dvfs_utilization`, the SoC's temperature from `/proc/lg/pm/temperature`; the divider read back from its file.
- `tv_gate.py --refresh 120`: `refresh 120 10` before the surfaces, the surface gate with its dropped-frame threshold at 10 ms
  (`analyse.set_refresh`: a refresh and a fifth, 20 ms at 60), `refresh reset 10` after them (idle Home again, the thermal reading that
  matters), `refresh 60 5`, and the divider forced back to 2 in the clean-up whatever happened.

Host tests: `refresh.rs` (periods, the divider written, read back, refused when absent, put back on restore and on drop; the
histogram's percentiles; the measure's counts), `kms/timing.rs` (flips at 120 key the pacer by themselves, 100 Hz followed and left,
rekey back to 60 where the measured path cannot, the measure reading a 15 ms frame on a 120 Hz grid as late and two refreshes apart),
`tests/refresh.rs` (the control command on a fake KMS: divider, pacer, tick, topic, journal, measure, a refused switch changing nothing,
an output without a panel taking the number), `tests/test_cutover.py` (the driver's order at 120 and the divider's restore).

## TV check

Nitin at the set or a slot with it free; LumaTV's compositor live as a session (the gate does that). About 25 minutes.

1. `python3 compositor/tools/tv_gate.py --only frames --refresh 120`. It runs, in order: the live check, `gate.sh refresh 120 10`
   (`GATE refresh: measured hz 120 divider 1 ...`), the surface gate over `home-dpad,quick,picker,toast` at 120 (`GATE frames at 120 Hz:`),
   `gate.sh refresh read 10` (idle Home after the surfaces, the compositor's measure since the switch, which covers the slides), `gate.sh
   refresh 60 5`, then `wake`, `mem`, `callbacks` as at 60.
2. Read, in `docs/research/compositor/gate-<date>.txt`:
   - `GATE refresh` at 120: `divider 1`, `shell 120` (the client's own report: the mode reached its QML); OSD flips/s, `DE2VSYNC/s` and
     `osd_irq/s` near 120 at idle Home are the mode working (flips at idle Home are the wallpaper's 10 a second plus the clock: the
     engine's rates are the proof, the compositor's `flips_per_second` the UI's); `pipeline_ms` p50/p99 at idle; `mali busy` and
     `temperature` before the surfaces.
   - `GATE frames at 120 Hz`: per surface `burst_over20` (intervals over 10 ms inside the burst), `dropped`, `first_frame`, `burst_end`.
     The bar: zero `burst_over20` and `dropped` on `quick` and `picker`. `home-dpad` is expected to fail; `toast` is a report.
   - The second `GATE refresh` (after the surfaces): `intervals` (`one`, `two`, `three`, `more`), `frames` (`late`, `lost`, `missed`),
     `pipeline_ms` p99 and max, `mali busy` and `temperature` against the first line: the thermal and power cost of the run.
   - `GATE refresh` at 60 after the switch back: `divider 2`, rates at 60, and `/tmp/lumatv-tv-out/out/gate/refresh-60.json`.
   - The compositor log: `kms: refresh 120 Hz: ... = 1, the pacer keyed to 8.33 ms`, then `page-flip stamp` lines at 8.33 and the
     `the panel refreshes every ... ms now` line only if content changed the engine's rate.
3. If the kernel refused the write (`GATE refresh: FAIL refresh 120: ... reads 2 after writing 1`): the mode is unreachable from
   userspace on this build; nothing else to try short of the kernel.
4. If `quick` and `picker` pass and `home-dpad` fails on intervals over 10 ms: the next cut is the dock's focus move (each tile its own
   cached layer, the lift animated on the focused tile only), decided from `frames-120.json`'s `burst_first_render_ms` and the trace's
   GPU busy per frame.
5. If `quick` and `picker` fail with `pipeline_ms` p99 over 13 ms in the second `GATE refresh` (the measure covers the slides now): the
   slide frame costs more than the two refreshes a frame in flight has; the gate's traces (`/tmp/perf/gate-c4shell-120-*`) say which GPU
   jobs; the wallpaper quad and the sheet's glass are the next candidates. If they fail with intervals still at 16.7 ms and `shell 120`
   in `GATE refresh`: the per-frame motion is not in the chain on the set's Qt (`afterAnimating` not emitted there, or `update()` not
   delivered on the callback); `GATE refresh`'s `slides:` field (the `[FrameMotion]` lines of the shell's log, steps per slide) says which.
6. Whatever the verdict the run ends at 60 with the divider at 2 (`divider after: 2` in the driver's output). A set left at 120 by a
   crash is put back by the supervisor's fallback or, failing that, by `echo 2 > /sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip`.

Abort: a black or torn panel after `refresh 120` (not expected: the OSD plane's own latch is unchanged) is `ctl refresh 60` from the
workstation, else the echo above, else the session's safety timer brings LG's compositor back.

## Open

The Qt wayland client's frame cadence at 8.33 ms callbacks (whether its 5 ms update timer sits in the chain as it did under eglfs);
the GPU cost of a slide frame with the cache, and of the compositor's composition of the system layer over Home at 120; the engine's
behaviour at 100 Hz with the divider at 1 (50-family content in front, the pacer's switch to 100 and back); the thermal trend over
minutes of 120 fps animation rather than the gate's seconds; whether a persistent 120 (the supervisor's exec line, a flag file) is ever
wanted, which this test mode does not provide.

## TV round 2026-10-28 (round 14, `compositor/slot-2026-10-28-r14/frames-120.json`, `gate-120-gate-lines.txt`)

`a1555df`, `tv_gate.py --only frames --refresh 120`: `GATE refresh` 120: `divider 1`, DE2VSYNC 120.8/s, `osd_irq` 120.2/s, OSD flips 10.1/s at idle Home; `pipeline_ms` p50 4.75, p99 4.75 (budget 8.33),
no late, lost or missed frames; Mali 11.7 %, 57 C. After the surfaces: 120 intervals, p99 4.85 ms, 0 late, 0 lost, Mali 11.8 %, 56 C. Surfaces at 120 Hz: home-dpad `dropped` 1, quick `burst_over20` 32, picker 35,
toast first frame 51.9 ms: the bar (zero `burst_over20` and `dropped` on quick and picker) is not met. The slide frame's cost is not the limit (p99 4.85 ms, 0 late); the shell's cadence is
(step 5: `QT_QPA_UPDATE_IDLE_TIME=0` untried). The run ended at 60 (`divider after: 2`, `GATE refresh` 60 `divider 2`). Verdict: the mode is reachable and stable at idle; the UI's bursts do not reach it.

## The shell's cadence: Qt's 16 ms animation timer (after round 14)

Round 14's slides: every frame of `quick-open` and `picker-open` 16.7 ms apart (`frames-120.json`, the windows' `gaps`: 16.7, 16.7, 16.7,
25.0), two refreshes each, and the compositor's log full of `page-flip interval 16.68 ms, nothing due at the refresh between`: the
client's commits came every 16.7 ms, each drawn and shown as soon as it came. Nothing in the compositor held them: its pipeline was
4.75 ms, `late` 0, and the shell's frame callbacks go out a frame ahead (`b193b28`).

Why 16.7: the shell client runs Qt's basic render loop (LG's `qml-runner`, `WEBOS_GL_DISABLE_THREADED_RENDERING=1`, the environment LG's
Qt apps run with), and the basic loop installs no animation driver (qtdeclarative 5.12 `qsgrenderloop.cpp`: `QSGGuiThreadRenderLoop::
animationDriver()` returns null), so every QML animation (`NumberAnimation`, `Behavior`, `Timer`) steps on `QUnifiedTimer`'s default
driver: a 16 ms `QBasicTimer` (`qabstractanimation.cpp`, `DEFAULT_TIMER_INTERVAL 16`), whatever the refresh. One animation step every 16 ms
is one dirty frame every 16 ms, and QtWayland (`qwaylandwindow.cpp` 5.12: `requestUpdate` holds a request while a frame callback is
pending and delivers it on the callback) renders one frame per step. The frames land on the next 120 Hz refresh each: 16.7 ms apart, two
refreshes, as measured. The threaded loop would step per frame with its vsync driver, but that driver advances by `1000 / QScreen::
refreshRate()` per frame, read once at start from the compositor's `wl_output` mode (60 Hz): double speed at 120 fps, a shell restart
per switch, and a render loop LG's apps never run with.

The fix is in the QML, at `Lu.fast` only: `kit/FrameMotion.qml` steps a motion once per rendered frame. A value set in the window's
`afterAnimating` (the basic loop emits it between polish and sync) is in that frame, and `window.update()` asks for the next, which
QtWayland delivers on the compositor's next frame callback: one step per refresh. Motion time advances a refresh per frame and is put
back on the wall clock when more than a frame off either way (a skipped frame; the host, which has no refresh), as Qt's vsync driver does.
`panels/PanelMotion.qml` drives the quick panel's and the picker's `progress` with it at 120 (the NumberAnimation at 60), `Dock.qml` the
row scroll (the Behavior at 60); the dock's focus move is one redraw per key and needs nothing. The shell reports `refreshHz` in its
state, so `ctl shell` (and `GATE refresh`'s `shell` field) say the mode reached the client; each motion logs its frame count and wall
time at its end (`[FrameMotion] N steps in M ms for D`), which `GATE refresh` prints as `slides:`.

Host measurement (`tests/test_ui120_motion.py`, Qt 5.15 with `QSG_RENDER_LOOP=basic`, software GL): a 1,500 ms linear motion stepped 97
times in 97 frames (one step per frame, the frames kept coming by the motion itself), its value on the wall clock (33.0 at 500 ms, 66.6 at
1,000) and on its target at the end, one frame in the second after it (the clock's); the panel slide at 120 stepped by frames (17 for the
110 ms in, progress 0.73 at 40 ms as out-cubic says) and at 60 by the NumberAnimation with no step (0.64 at 40 ms: the 16 ms timer's lag).
The host's frames come at its own pace (6-16 ms); the set's come at the callbacks, which is what the next run measures.

Expected on the set [INFERENCE]: the slides' intervals at 8.33 ms with the compositor's early callbacks (a frame started at each landing,
two in flight, 13 ms of GPU budget each); `quick` and `picker` `burst_over20` 0 if the slide frame's GPU time stays under about 13 ms
(round 14 did not measure it: the measure was reset before the idle read; this run's `refresh read` after the surfaces covers them, its
`pipeline_ms` p99 and `late` are the numbers); `burst_end` about 5 ms earlier than at 60. `home-dpad`: the row scroll per frame, the focus
move one frame per key as before. The toast's first frame is the service's hand-over, not the shell's (`compositor-surfaces-result.md`).

## TV round 15: the GPU has 8.33 ms for two frames (`compositor/slot-2026-10-28-r15/frames-120.json`; the traces stayed on the workstation, `/tmp/perf/gate-c4shell-120-*`)

`0308184`: `GATE refresh` `shell 120`, idle pipeline p99 4.88 ms, 0 late; over the surfaces 72 late of 6,276 frames, p99 5.0, max 11.5. The
slides stepped per refresh (most intervals 8.33 ms) and still lost 2-3 refreshes each: `quick` `burst_over20` 14, `picker` 10, `toast` 3,
`home-dpad` `dropped` 2. No `[FrameMotion]` line reached the shell's stdout file: the runner's QML console goes elsewhere (PmLog), so the
slides are now in the shell's state (`Lu.slides`, `ctl shell`), where the gate reads them first.

The trace of a quick-panel slide (`gate-c4shell-120-quick-m0-r1.trace`, `mali_job_slots_event` and `sched_switch`), one refresh of it:

| ms | what |
|---|---|
| 8.82 | flip (frame N lands); the compositor pays N (0.12 ms), releases N−1's buffer to the shell |
| 11.68 | the shell's Mali thread commits N+1: the blob sends the commit when the GPU job is done, not at `eglSwapBuffers` |
| 12.25-16.48 | the compositor draws N+1 (0.95 ms of CPU); its fragment job runs **4.1 ms** (a 1080p ARGB quad into the linear page) |
| 12.60 | the callback for N+1 goes to the shell at the draw (nothing ahead of it) |
| 13.6-15.8 | the shell renders N+2: 2.2 ms of CPU, then its fragment job, queued behind the compositor's, runs 16.48-20.58: **4.1 ms** |
| 17.16 | flip (N+1 lands), 0.7 ms after the compositor's fence |
| 21.1 | the shell's commit of N+2; the compositor draws it 21.8-22.2, its job runs to about 26: the flip due at 25.5 is missed, N+2 lands at 33.8 |

So the GPU does two fragment jobs per refresh, 4.1 ms each: the compositor composes every frame of Home (direct scan-out needs the
buffer's dma-buf, and with `--buffers driver` every client buffer is the blob's own, `BufferKind::External`; the 64-bit blob lists no
`EGL_MESA_image_dma_buf_export`, so there is no way to a framebuffer from one), and the shell's slide frame costs as much. 8.2 ms of
GPU in an 8.33 ms refresh, serialised through the blob's commit-after-completion: a frame makes its refresh only when the two jobs
run back to back with nothing else on the GPU, and loses one otherwise. In the 16.7 ms windows the gate recorded, the CPUs were idle
(the shell 1.5-3.5 ms, the compositor 1-1.8 ms) and the GPU busy 2-8 ms: no one was slow; the sum does not fit.

What that leaves: the composition is fixed at about 4.1 ms (one full-screen pass on this GPU, bandwidth-bound), so the shell's slide
frame must cost well under 4 ms for the pair to fit with margin. At 120 Hz (`Lu.fast`) Home now draws everything a panel slides over
(the wallpaper, the widgets, the dock: one `base` item, `LumaHome.qml`) from one cached texture while a panel is on the screen, on its
way, warmed or lingering (`Layers.panelSliding`), and holds its wallpaper tick and the clock's hands while a slide moves
(`Layers.panelMoving`), so the texture is not rendered again mid-slide. A slide frame is then one full-screen quad, the panel's scrim
and the sheet: about 2 ms of GPU [INFERENCE: three full-screen passes fewer at 0.6-1 ms each], the pair about 6 ms of 8.33. The
texture is rendered at the gear's press (the warm), not in the slide's first frame; the picker, opened at the Input key's press, pays
it in its first frame. At 60 the item draws as its children always did (the host renders at 60 pixel-identical bar the clock's hands).

Still outside the shell's reach: the compositor's 4.1 ms. The gate's `late` and `burst_over20` at 120 will not reach zero while every
frame of Home is composed; a zero-drop 120 needs the shell's buffer on the plane, which the blob's buffer path forbids.

Host (`tests/test_ui120_motion.py`): at 120 the base is layered from the panel's open through its linger, the tick holds while the
slide moves and runs again once it is settled, the two slides land in `slides` (17 and 21 steps for 110 ms on the host); at 60 nothing
is layered or held and `slides` stays empty.

Next run should show: `GATE refresh` `shell 120` and `slides (steps/ms/for)` with about 13-14 steps per 110 ms slide; `quick` and
`picker` `burst_over20` down from 14/10 towards 0-2 (the first frame of a picker slide and any frame the GPU shares with something else);
the measure's `late` over the surfaces well under round 15's 72; `pipeline_ms` p99 about 5 (the composition, unchanged).
