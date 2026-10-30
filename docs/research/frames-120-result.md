# Settings at 120 Hz: why frames miss (round 21 gate, round 22 ring)

## Resume here

On forge main:

- `8642b179`, `85ac5a6d` the gate's measurement: `analyse.py` read the supervisor, not the compositor, and took the compositor's 0.1 ms wakeups for renders; render
  limits follow the refresh; every measured cycle's trace ring is kept (`/tmp/perf/<name>.ring.json`, `perf/ringsum.py`); `perf/frametl.py` prints a trace's flips.
- `9e100ea4` Home's GPU wallpaper stands still and its weather rests while Settings or a panel is over Home; Settings opens without its fade from 100 Hz.
- `b421f486` Round 22 on 2138070f (60 and 120 Hz, wallpaper motion off as the gate sets it), in order of weight:
  1. **The 120 Hz run is not a 120 Hz run after 04:42.** A camera popup (a hardware video window) made the TV re-time the panel from 120 to 100 Hz
     (`the panel refreshes every 10.00 ms now (8.33 before)` in the compositor log, DE2VSYNC 100.8/s at the end of the run) and it stayed there, also after
     the popup closed; every flip 10.0 ms after the last was a dropped frame against the 10 ms of 120 Hz (idle Home 1205). The gate now judges each window by
     the refresh the panel ran at (`analyse.panel_drop_ms`), and a new check `panel_refresh` fails a window on another refresh as what it is. At a true 120 Hz
     (home-dpad's first captures: 120.0 flips/s, worst interval 8.4 ms, 0 dropped, 0 burst_over20 in the first, 2 intervals of 16.7 ms in the next) Home with
     its weather art drawing every refresh holds 120 fps. The popup is an event (a camera), not the gate's: whoever owns the hardware video windows should
     find out how the panel is put back (`ctl refresh 60` did not; a compositor restart did).
  2. **An idle Home redrew 93 % of the screen one to four times a second**, three frames of 87 layers (7.5 ms of GPU each at 60 Hz, 7 at 120), in every ring of
     both runs, with the wallpaper still: Home's redraw (the clock's second, any key) said the look of the dock's glass and the pose of its cards again, and the
     engine took each for a change. Each cluster after a cheap stretch was also where the pacer had planned a short lead, so the first heavy frame missed its
     refresh (`late 1 lost 1`, 33.4 ms at 60 Hz). The engine now leaves a `Restyle` or `Set` that says what is already so (and a wallpaper's fps alone) as it is
     (`patches/lumatv-scene.patch`); on the host an idle Home draws 31 kpx a second where it drew 1.13 Mpx (`tests/test_home_idle.py`).
  3. **Home is calm** (`Home::set_calm`, `Shell::after`): its wallpaper stands still and its weather rests while anything is over it, 450 ms after, and for 2 s after
     a key. The weather's sprites draw a frame every refresh, so that the compositor is in step with the panel when a key arrives and the screens never go
     quiet (`burst_end_ms` of quick, picker and toast read the whole window, 1.7-2.0 s, at 60 Hz).

Next, in order:

1. TV re-check (Round 22 owns the set), install this build: `python3 compositor/tools/tv_gate.py --only frames --quick`, then the same with `--refresh 120`.
   Read the gate's `panel_refresh` first: a window on another refresh says nothing of the asked one (restart the compositor, run the surface again).
   Pass: every surface `dropped` 0 and `burst_over20` 0. Rings: `/tmp/perf/gate-c4shell[-120]-<surface>-m<i>.ring.json` (`perf/ringsum.py FILE`).
2. What is left, from the 60 Hz rings of 2138070f: the first key after a stretch of per-refresh frames (the weather's) answers 33.4 ms after the last flip (the update
   comes after a vblank, the frame costs a refresh: two refreshes whatever the pacer does), which `burst_over20` counts as a drop in the burst; once the screen
   is calm before the key (the second cycle on) it does not. If it still fails the gate, the burst should start at the response frame, as `dropped_in_motion` does.
3. Panels and Settings still redraw Home's 87 layers under a changing alpha: 12.8 ms of GPU for the first frame of quick's open (render 3-4 ms), so a refresh and
   more at 60 Hz and impossible at 120. Caching Home as one picture while a modal fades over it is the structural fix.

## What the gate said, and what it measured

`perf/gate.py` passed `pidof surface-manager` (the supervisor, `lumatv-sm`) as `--sm`; LumaTV's compositor is its child (`ld-linux-aarch6`, the `loop N`
marker `capture.sh` writes into the trace). So `p90_render_ms` (0.1-0.6 ms in every LumaTV gate), `anim_frames` and `dropped` were read off the
supervisor's few runs. Read off the compositor's thread (the `loop` pid) the same traces gave a second error: the thread wakes for every vblank and commit
for 0.1 ms, and `analyse.py` took such a wakeup for the render of an animated frame (apps-grid at 120 Hz: 8 drops in motion, every one a flip 25-217 ms
after the last, between keys 60-120 ms apart, its "render" 0.10-0.13 ms). A render is now a burst of 0.3 ms or more (the ring's frames: 0.6 ms and up).

Corrected, first cycle of each round 21 surface (`dropped_in_motion`, `burst_over20`): at 60 Hz every surface 0 in motion; `burst_over20` quick 4-8 a
window, home-widgets 1. At 120 Hz settings-scroll 9 in motion (92 animated frames, 44 over 10 ms) and `burst_over20` 25; picker 1 in motion and 1-4
`burst_over20`; quick 1-2 `burst_over20`; home-widgets 1; home-dpad, apps-grid, toast 0.

## The cause (Round 22's ring at 120 Hz, Settings open, then six Down steps)

Steady frames are cheap: `owed 43-100 k px in 3-4 rects, ui 39-58 layers, render 1.0-1.7 ms`, GPU done 3-4 ms, every 8.3 ms.

Every 100-150 ms, three frames in a row (one per page of the ring) owe the whole Settings area, `owed 1930240 px in 1 rects, ui 203-205 draws
157-158 layers`, render 1.7-2.7 ms, GPU done 13-16 ms each: 133 of the ring's 874 frames, in clusters of three. At idle Home the same cluster comes once a
second (`owed 1930240 px, ui 101 draws 70 layers`, GPU 7-9 ms): it is the GPU wallpaper's step (Silk, `wallpaperMotion.fps`, 10 a second in the shipped
theme), which changes every pixel it shows. Under Settings the wallpaper is under panes that are 96 % opaque (`sheet` alpha 0xF5, so the engine counts no
cover), so each step is drawn again with Home's 70 layers and Settings' 87 above them, three times. At 60 Hz the same frames (about 14 ms) fit a 16.7 ms
refresh; at 120 Hz they take two or three refreshes each, which is the 16.7-25 ms intervals the gate counts. The weather card's sprites (43 k px, a frame
every refresh, `weather_art_runs` ignored Settings) add a frame under the panes every 8.3 ms.

The gate's trace agrees: at 120 Hz the walk's drops sit in the opening (19), the rail steps (30: pages switched about 60 ms apart, a wallpaper cluster every
100 ms) and the page changes at the end (9); the held-key phases between them are clean.

The opening fade is separate: the first 0.3 s were 26 frames that owe the whole screen with 157 layers (Home 70, Settings 87), GPU done 15-32 ms each (`L flip
... frame 14.8, 19.8, 30.0, 25.8, 32.3 ms`). At 60 Hz that fits; at 120 Hz a fade over a full screen of 157 layers cannot (the engine redraws every layer under a
changing alpha; Home alone costs 7 ms a frame). So from 100 Hz Settings now opens and closes without it (one heavy frame, then quiet). A visible change at 120
Hz only (a test mode, off by default); 60 Hz is as it was. Other ways, if the fade is wanted at 120 Hz: cache Home as one picture while a modal fades over it,
or hide Home's cards and dock for the fade and keep the wallpaper.

## Notes on the engine, for whoever goes on

- `ctl refresh`'s `motion.dropped` (compositor `refresh.rs`, `STALL_NS` 20 ms) is the PLAN's frame rule at any refresh: at 120 Hz it reads a 16.7 ms interval as
  fine. The gate scales its own limit (`--refresh 120`: 10 ms); read the gate, not that counter, for 120 Hz.

- A frame costs about 70 us of CPU for each layer drawn in each pass (render 1.0-1.7 ms for 39-58 layers), and the cost model (`damage::DRAW_WORTH`, from
  LumaShow's Show: 63 us a layer, 10 ns a pixel) merges a frame's rectangles into one cover when the scene has many layers: that is why a wallpaper step
  under Settings is one rectangle of 1.93 Mpx and not ten small ones.
- A sheet drawn opaque (alpha 255) would be a cover and the tiles under it would not count as changed; not done, the theme's 96 % is the owner's. Culling in a
  pass is by the layer's whole extent (a full-screen layer under an inset pane is drawn in every pass that meets it): per-pass culling with the tile
  visibility the engine already has (`tiles::visible`) would take Home's layers out of the passes under the panes; it is in LumaShow's renderer (vendored).
- The pacer's `SLACK_NS` (5.5 ms) and the idle tick's `IDLE_LEAD_NS` (6 ms) were set for 16.7 ms refreshes: at 120 Hz the idle tick plans two refreshes
  ahead (its latch is 4.67 ms after a flip: the compositor thread's renders start there), so an isolated frame lands 16.7 ms after the last flip. Not touched.
- `tests/host_frames.py` walks Settings on the host harness and prints each key's frames (layers, pixels, CPU on Mesa); `perf/frametl.py` prints a per-flip
  timeline of a trace (compositor thread, shell process, GPU jobs) from `frametl_parse.py`.
