# Settings at 120 Hz: why frames miss (round 21 gate, round 22 ring)

## Resume here

Done (forge main, own worktree `/tmp/Frames120-wt`, delete it only when finished):

- `8642b179` gate measurement: `analyse.py` read the supervisor, not the compositor (and took the compositor's 0.1 ms wakeups for renders, since fixed);
  render limits follow the refresh; every measured cycle's trace ring is kept (`/tmp/perf/<name>.ring.json`) and summed in the gate's output (`perf/ringsum.py`); the ring holds 768 KiB (25 s at 120 Hz); a long `ctl ring N` is
  answered from a thread.
- `9e100ea4` the fix, from Round 22's ring (`compositor/slot-2026-10-30-r22/frames120-settings-ring.json`, build 5afc7e41): Home's GPU wallpaper stands still
  and its weather sprites rest while Settings or a panel is over Home (`Home::set_overlay`, `WallpaperSurface::set_held`, `Shell::after`; released 450 ms after
  the overlay goes, past its fade-out); Settings opens and closes without its fade when the compositor's `refresh` topic says 100 Hz or more
  (`Kit::modal_beat_ms`, `Kit::refresh_hz`).
- Host tests: `cargo test --release -p lumatv-ui` (1194 tests; new: `shell::tests::settings_over_home_stills_...`, `the_refresh_topic_takes_the_modals_fade_away_from_100_hz`,
  `home::model::tests::a_modal_over_home_...`, `home::wallpaper::tests::a_held_wallpaper_...`);
  `python3 -m unittest test_gate` in `tests/` (86).

Next, in order:

1. TV re-check (Round 22 owns the set): install this build, then `python3 compositor/tools/tv_gate.py --only frames --quick` and the same with
   `--refresh 120`. Pass: every surface `dropped` 0 and `burst_over20` 0. The gate now prints, per measured cycle, the ring's numbers
   (`... frames drawn, render p50/p90/max ..., owed p50/p90/max ... px`); the files are `/tmp/perf/gate-c4shell-120-<surface>-m<i>.ring.json`
   (`python3 docs/research/perf/ringsum.py FILE [NOTE [SECONDS]]`). Expected: `settings-scroll` no longer has frames of `owed 1930240 px ... 157 layers` every
   100-150 ms; its opening is one frame of that size, not 26.
2. Whatever still misses at 120 Hz, read from those rings: candidates below.
3. If a quick/picker/Home-only surface (wallpaper steps with nothing over Home) still misses: the step itself costs 7-9 ms of GPU three times (Home 70 layers,
   1.93 Mpx); options are fewer layers in Home or a wallpaper that steps less often while keys are pressed.

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
