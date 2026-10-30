# Settings at 120 Hz: why frames miss (round 21 gate, round 22 ring)

## Resume here

Done (forge main, own worktree `/tmp/Frames120-wt`, delete it only when finished):

- Gate measurement fixed (`8642b179`): `analyse.py` read the supervisor, not the compositor; render limits follow the refresh; every measured cycle's
  trace ring is kept (`/tmp/perf/<name>.ring.json`) and summed in the gate's output (`perf/ringsum.py`); the ring holds 768 KiB (25 s at 120 Hz); a long
  `ctl ring N` is answered from a thread.
- The cause found in Round 22's ring (`compositor/slot-2026-10-30-r22/frames120-settings-ring.json`, build 5afc7e41): below.
- Fix in this branch: Home's GPU wallpaper stands still and its weather sprites rest while Settings (or a panel) is over Home
  (`Home::set_overlay`, `WallpaperSurface::set_held`, `Shell::after`), released 450 ms after the overlay goes.

Next, in order:

1. If cut off here: `cargo test -p lumatv-ui --lib home::` and `shell::`, `cargo clippy -p lumatv-ui`, push (what is not pushed is in the worktree).
2. TV re-check (Round 22 owns the set): install this build, then
   `python3 compositor/tools/tv_gate.py --only frames --quick` and the same with `--refresh 120`. Pass: every surface `dropped` 0 and `burst_over20` 0.
   Each cycle's ring is in `/tmp/perf/gate-c4shell-120-<surface>-m<i>.ring.json`; `python3 docs/research/perf/ringsum.py FILE` prints it, and the gate prints
   one line per cycle (`... frames drawn, render p50/p90/max ..., owed ...`). The expected change: `settings-scroll` frames owing about 1.93 Mpx (`owed
   1930240 px in 1 rects, ui 203 draws 157 layers`) vanish, three at a time, every 100-150 ms.
3. What is left (not fixed here, needs a decision): the opening fade itself (below), and `quick`/`picker` at 120 Hz.

Open: whether Settings may open without its 220 ms fade at 120 Hz (the shell does not know the refresh: `welcome.refresh_hz` is read at start and the
compositor's `refresh` topic is not consumed by the Rust shell).

## What the gate said, and what it measured

`perf/gate.py` passed `pidof surface-manager` (the supervisor, `lumatv-sm`) as `--sm`; LumaTV's compositor is its child (`ld-linux-aarch6`, the `loop N`
marker `capture.sh` writes into the trace). So `p90_render_ms` (0.1-0.6 ms in every LumaTV gate), `anim_frames` and `dropped` were read off the
supervisor's few runs. Same trace read with the compositor's pid (round 21, 120 Hz, first cycle): settings-scroll 163 animated frames (gate: 52), 82
dropped (gate 35), 60 of them in motion (gate 15), render p90 3.4 ms. At 60 Hz the corrected `dropped_in_motion` is 1, as the gate said. `burst_over20`
comes from the flips alone and was right.

## The cause (Round 22's ring at 120 Hz, Settings open, then six Down steps)

Steady frames are cheap: `owed 43-100 k px in 3-4 rects, ui 39-58 layers, render 1.0-1.7 ms`, GPU done 3-4 ms, every 8.3 ms.

Every 100-150 ms, three frames in a row (one per page of the ring) owe the whole Settings area, `owed 1930240 px in 1 rects, ui 203-205 draws
157-158 layers`, render 1.7-2.7 ms, GPU done 13-16 ms each: 133 of the ring's 874 frames, in clusters of three. At idle Home the same cluster comes once a
second (`owed 1930240 px, ui 101 draws 70 layers`, GPU 7-9 ms): it is the GPU wallpaper's step (Silk, `wallpaperMotion.fps`, 10 a second in the shipped
theme), which changes every pixel it shows. Under Settings the wallpaper is under panes that are 96 % opaque (`sheet` alpha 0xF5, so the engine counts no
cover), so each step is drawn again with Home's 70 layers and Settings' 87 above them, three times. At 60 Hz the same frames (about 14 ms) fit a 16.7 ms
refresh; at 120 Hz they take two or three refreshes each, which is the 16.7-25 ms intervals the gate counts. The weather card's sprites (43 k px, a frame
every refresh, `weather_art_runs` ignored Settings) add a frame under the panes every 8.3 ms.

The gate's trace agrees: at 120 Hz the walk's drops sit in the opening, the rail steps (pages switched about 60 ms apart, a wallpaper cluster every
100 ms) and the long page, and the held-key phases between them are clean.

The opening fade is separate and not fixed here: the first 0.3 s are 26 frames that owe the whole screen with 157 layers (Home 70, Settings 87), GPU done
15-32 ms each (`L flip ... frame 14.8, 19.8, 30.0, 25.8, 32.3 ms`). At 60 Hz that fits; at 120 Hz a fade over a full screen of 157 layers cannot (the
engine redraws every layer under a changing alpha; Home alone costs 7 ms a frame). Options: open without the fade when the refresh is 100 Hz or more
(needs the refresh in the shell), cache Home as one picture while a modal fades over it, or hide Home's cards and dock for the fade and keep the wallpaper.

## Notes on the engine, for whoever goes on

- A frame costs about 70 us of CPU for each layer drawn in each pass (render 1.0-1.7 ms for 39-58 layers), and the cost model (`damage::DRAW_WORTH`, from
  LumaShow's Show: 63 us a layer, 10 ns a pixel) merges a frame's rectangles into one cover when the scene has many layers: that is why a wallpaper step
  under Settings is one rectangle of 1.93 Mpx and not ten small ones.
- A sheet drawn opaque (alpha 255) would be a cover and the tiles under it would not count as changed; not done, the theme's 96 % is the owner's.
- The pacer's `SLACK_NS` (5.5 ms) and the idle tick's `IDLE_LEAD_NS` (6 ms) were set for 16.7 ms refreshes: at 120 Hz the idle tick plans two refreshes
  ahead (its latch is 4.67 ms after a flip), so an isolated frame lands 16.7 ms after the last flip. Not touched.
