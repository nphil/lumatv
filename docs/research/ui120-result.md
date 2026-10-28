# UI at 120 Hz: the refresh test mode

Can Home and LumaTV's menus draw at 120 fps under LumaTV's compositor with no dropped frame? The mode is built and measurable
(`--refresh 120`, `ctl refresh 120`, `tv_gate.py --only frames --refresh 120`), off by default; the set has not run it yet.

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
   `gate.sh refresh reset 10` (idle Home after the surfaces), `gate.sh refresh 60 5`, then `wake`, `mem`, `callbacks` as at 60.
2. Read, in `docs/research/compositor/gate-<date>.txt`:
   - `GATE refresh` at 120: `divider 1`; OSD flips/s, `DE2VSYNC/s` and `osd_irq/s` near 120 at idle Home are the mode working (flips at
     idle Home are the wallpaper's 10 a second plus the clock: the engine's rates are the proof, the compositor's `flips_per_second` the
     UI's); `pipeline_ms` p50/p99 at idle; `mali busy` and `temperature` before the surfaces.
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
5. If `quick` and `picker` fail with `pipeline_ms` p99 over 7.5 ms in the second `GATE refresh`: the slide frame still costs more than
   the budget; the gate's traces (`/tmp/perf/gate-c4shell-120-*`) say which GPU jobs; the wallpaper quad and the sheet's glass are the
   next candidates. If they fail with p99 under 7 ms and `late` high: the shell's cadence, not its cost (the Qt update timer or a
   callback a frame too late); `QT_QPA_UPDATE_IDLE_TIME=0` in the shell's environment is the one knob left untried.
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
