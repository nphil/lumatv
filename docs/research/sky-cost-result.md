# Sky cost: the weather sky on an idle Home (2026-11-05)

Evidence tags: `TV:` read on the set (file named), `HOST:` measured on the host harness (Mesa, one refresh a frame: it counts frames and draws, its CPU is not the set's), `[INFERENCE]` derived, not measured.

## Result

The sky no longer holds the screen at 120 frames a second. The UI gives every sprite of the sky a step by how far it goes between two frames (`weather_art::step_for`); the compositor wants a frame for it that often (`Request::Animate::step_ms`, protocol 15), sleeps between those frames, and the UI is no longer woken for frames that show it nothing new. A calm sky is drawn 15 times a second, snow and wind 30, rain 60. HOST: every sky condition drew 120.0 frames a second before; after, 15.0 to 60.1. TV: not run. `compositor/tools/tv-idle-probe.py run` measures it over 60 s.

[INFERENCE] from the cost per flip the set measured, an idle Home with a calm sky takes about 7 % of a core in the compositor and the UI together, snow and wind about 10 %, rain about 17 %, against the owner's 8 %. The target is within reach for a calm sky only; for the rest the frame itself must get cheaper (below).

## What the set measured

`TV:` `/data/home/tmp/lumatv-tv-20261104d/idle-home*.json`, build f98fb940, profile Nitin, 120 Hz, 60 s, no keys.

| | flips a second | ticks a second | compositor | UI |
|---|---|---|---|---|
| sky moving | 119.9, all at every refresh | 119.9 | 24.8 % of a core (main thread 23.0, Mali's 1.7) | 6.8 % |
| sky off | 8.5 | 127.5 | 3.1 % | 1.9 % |

Solving the two windows for the compositor's main thread: 0.11 ms a tick and 1.81 ms a flip; the UI took 0.44 ms for each extra frame. The engine's own CPU a frame is about 1.5 ms (`ctl status`: 51 draws, 30 layers, 218 k px a frame, 10.5 % of the page, 203 k of it changed). Drawing was already limited to the sky's box by the 16 px tile ring (`damage.rs`); the cost is a frame's draws and what surrounds them, not its area. A frame's GPU time is about 1.5 ms + 0.085 ms a layer (`frames-120-result.md`, R² 0.47).

## What was done

1. **Damage.** Nothing to add: the compositor redraws the tiles the card changes and no more. The whole OSD page is flipped, which costs bandwidth (995 MB/s at 120 Hz), not CPU, and no plane can flip less (below).
2. **A cheaper sky.** The sky has no shader: it is cached sprites rasterised once at native resolution (soft glows and far clouds at a quarter of it), moved by the compositor as quads (below).
3. **A lower rate.** The sky is an ambient motion. `Request::Animate` carries `step_ms`; the engine's demand for it is a frame at `last frame + step` (`Demand::At`), the layer is drawn where the motion is at the frame's own time, and a commit of nothing but such motions (the sky restating its paths every 400 ms) asks for no frame. The UI picks the step per sprite from the straight line the compositor draws for it over its lead (`step_for`, `STEPS`): the longest of 66, 33 and 16 ms in which the sprite moves at most 1.5, 4 and 5 px and fades at most 0.08, 0.15 and 0.25, and a frame at every refresh beyond that (the weather view's rain, 551 px/s). What the sky asks for on the card, from the art's own paths over 600 s of ticks (`weather_art` test `the_sky_asks_for_the_frames_...`, which prints every condition and both stages):

| Sky on the card | frames a second | fastest sprite |
|---|---|---|
| clear, partly, cloudy, fog | 15.2 | 14 px/s (0.9 px a step) |
| clear and partly night | 19.2 | a shooting star, 86 px/s, 33 and 16 ms while it crosses (17 % of the time) |
| snow, any sky with wind | 30.3 | wind 109 px/s (3.6 px a step), snow 69 px/s (2.3) |
| drizzle, rain | 62.5 | 258 px/s (4.1 px a step, a streak is 20 to 30 long) |
| storm | 65.0 | as rain, and every refresh for 4 % (the strikes) |

   Rain stays at 60: at 30 a streak would jump 8.6 px, a third of its length. The weather view's big sky asks for 28 to 30 calm, 62.5 with snow or wind, 120 in rain (as before).
4. **The loop sleeps between frames.** `Tick::rearm` arms the grid's latch for the engine's next due time (at most `idle_tick_ns`, 100 ms) while the loop is calm between frames (a flip landing and a commit that asks for no frame do not end the calm) and owes nothing (`paced_wait`, `Output::idle_between_frames`, `Ui::due_in`). `ctl idle 0` keeps the panel's rate. A tick that comes a half refresh early finds the step due (`Ui::wants_frame` slack).
5. **The UI is told of a frame only when it has news** (`Engine::frame_done`): a new commit's tag, uploads or backdrops. It was woken, and sent an event, for every frame of the sky.
6. **Measurement.** `ctl status` shows the engine's CPU by phase (place, plan, backdrops, draw, damage, apply) and how the compositor's ticks and passes were spent (`tickPartsUs`, `passUsTotal`); `tv-idle-probe.py` reads them with the CPU of both processes by thread.

The pacer (`kms/timing.rs` `rate_from_recent`) re-keys the panel's period when four flip intervals in a row are one known rate that is not 1 to 4 periods of the current one. 16.7, 33.3 and 66.7 ms are 2, 4 and 8 periods of 120 Hz and no known rate; 40 and 42 ms (25 and 24 Hz) would be taken for a slower panel, so the steps avoid them.

**The idle gate.** `home-idle` is measured with the sky on (`still: False`, `docs/research/perf/gate.py`). Before, every sky frame was an animated flip (119.9 a second: `refresh.measure.motion.intervals`, which the gate's `ring_judged` counts) and the trace tool took the frames for animated ones (the compositor idle under 7.5 ms before each). After, a paced frame is neither (`ui_animating` is false for `Demand::At`; its gap before the render is over 7.5 ms), so the gate's `p90 animated-frame render` reads "no animated frames" and passes without reading the sky's frames, and `ring_judged` falls to about 0. The sky's per-frame cost is read from the probe's engine microseconds a frame instead. The probe's `EXPECT` has the new numbers: flips a second 12 to 32 for a calm sky (15 and what Home draws for itself), animating intervals under 5.

## Hardware paths

| Path | Verdict | Evidence |
|---|---|---|
| (a) a separate OSD or DRM plane for the sky | not usable | The DRM planes are primary 32, overlay 34 and cursor 36 (`arch/drmplanes-01.txt`). Overlay 34 (fb1, OSD1) shares OSD0's hardware and the driver ignores its enable while OSD0 is on; OSD0_1 is the 4K path's right-half reader and the E13 probe found no layer there (`layer-deep-result.md`, `scanout-research.md`). The cursor plane is AR24, at most 128x128, and above all UI. |
| (b) a reduced-resolution render and the display scaler | whole plane only, and already used | The UI renders 1920x1080 and the OSD scaler takes it to 4K. A smaller page was refused: `pageprobe 1280x720` ended in `atomic commit: Invalid argument`, the watchdog fired and LG's compositor took over (`layer-deep-result.md` § 8). No scaler exists per layer; the sky's soft glows are already made at a quarter size by `Ctx::low`. The OSD sharpener showed no visible difference at its three gains. |
| (c) AFBC | works, does not reduce CPU | A1 `planeTakes true`, `gpuDraws true`, live switch A3 passed (`compositor/slot-2026-10-29-r20/hwplanes/log.txt`). It saves scanout and GPU bandwidth (48 to 73 % estimated), not the CPU of a frame; one `renderUsLast` pair read 1.0 ms AFBC against 1.3 to 1.8 linear. Off by default (`ctl afbc on`). |
| (d) Mali-friendly shaders and cached layers | already so | There is no sky shader and no full-screen pass: the sprites are cached textures. A floor cache exists in the engine and is not used for the sky (it is deliberately off when nothing is over Home). |
| (e) the sky pre-rendered as a loop on the video decoder | ruled out | The video plane is under every OSD pixel, so the glass cannot blur it, and a hole needs mixer register writes. A small video window made the TV leave Filmmaker Mode and shift colours (`vwin-refresh-result.md`); the panel re-times to 100 Hz for 25 and 50 fps video and only 30 and 60 fps restore 120 (`framerate-result.md`, `HANDOFF.md` of 2026-11-04). The video-plane wallpaper measured no saving: CPU 15.8 % in all (player 4.1, compositor 3.1, shell 5.2) and CMA free down to 3.3 MB (`videoplane-saver-result.md`). |

None of the five reduces the CPU of a sky frame. The usable lever is fewer frames.

## Vector or bitmap

The sky is already vector art made into bitmaps once: SDF-rasterised sprites at native resolution (`weather_sprites.rs`, `weather_art.rs`), uploaded once, moved by the compositor as textured quads, their paths restated every 400 ms. The cost of a frame is its draws, the area it fills and how often it is made, so the comparison is about what each approach adds to that:

| Approach | Per frame | Crispness |
|---|---|---|
| cached sprites (now) | no rasterising; the draws of the layers in view (HOST: 15 to 31 at the median, 60 at most in rain) | the sprite's own pixels at 1:1, sharp edges on whole pixels at rest |
| SVG | rasterised to the same textures (it is a source format here, not a draw path), so the same | the same, if made at native resolution |
| a shader drawing the shapes (SDF) | the same quads, and the shape's maths for every pixel of every frame; a cloud is dozens of overlapping SDFs | as sharp as the maths; nothing measured on the Mali |
| paths tessellated every frame | the tessellation on the CPU each frame; making one picture takes about 28 ms of host CPU (3.0 s for 432 pictures on 4 threads), nearly a whole 33 ms step | as sharp |

Cached sprites rendered once at native resolution are crisp and cheapest, as expected: what a sky costs is how often and how much is redrawn, not the art's format. Measured on the host (not the Mali): the same sprites drawn at 120 and at 15 a second differ by the factor 8 in frames and by none in layers or pixels a frame. A per-frame shader or tessellation would add work to every frame and could not lower the number of frames, which is the lever. Crispness: the strips in `/data/home/tmp/sky-cost/` (`strip-<case>.png` at 100 %, `zoom-<case>.png` at 6x, for clear night, cloudy, rain, snow and windy) show the new build's frames beside today's of the same times; the sprites are the same pixels. The harness does not repeat the sky's phase from run to run, so frames of two runs cannot be compared pixel for pixel: in the sky's box two runs of today's build differ by 2 500 pixels over 2 levels (largest 85 levels) in cloudy and 5 400 (117) in rain, more than the new build's frames differ from today's (1 400 and 47; 0 and 0).

## Host numbers

`weather_frames.py --layout shipped --refresh 120 --seconds 10 --rest` (before: `3e185b7e`), frames a second and the layers the median frame draws (est. GPU busy = frames x (1.5 + 0.085 x layers) ms):

| Case | before | after | layers p50 | est. GPU busy before, after |
|---|---|---|---|---|
| clear day | 120.0 | 15.0 | 15 | 33 %, 4 % |
| clear night | 120.0 | 18.1 | 28, 29 | 47 %, 7 % |
| partly day | 120.0 | 15.1 | 23 | 41 %, 5 % |
| cloudy | 120.0 | 15.1 | 17 | 35 %, 4 % |
| snow | 120.0 | 34.1 | 21, 25 | 39 %, 12 % |
| windy | 120.0 | 30.1 | 26 | 45 %, 11 % |
| rain | 120.0 | 60.1 | 27 | 46 %, 23 % |
| storm | 120.0 | 60.0 | 31 | 50 %, 25 % |
| at rest | 4.4 | 4.4 | 5 | |

`host_budget.py --surfaces sky` (Home's own feeds, the sky moving 10 s): 1200 frames (120.0 a second) before, 209 (20.9) after, most layers 34 and 38, work of a frame at its worst 4.4 and 4.7 ms, no frame differing from the frame drawn whole.

## On the set

The probe (`compositor/tools/tv-idle-probe.py`, on the set as root, stdlib only):

    tv-idle-probe.py run --secs 60 --label after      /tmp/lumatv/out/idle/after.json and a summary
    tv-idle-probe.py compare before.json after.json

It reports the compositor's and UI's CPU as shares of a core (user and system, by thread, context switches), flips, ticks, wake-ups and animating intervals a second, the engine's microseconds a frame by phase, how the ticks were spent, the GPU's busy share, the panel's rate, what Home showed (the weather code, the theme's sky switch), and checks quiet Home, 120 Hz, the flip rates the sky's class expects and the 8 % target. A run on the build before this change (no phase totals) compares by CPU and flips.

## Not done, not measured

- The TV run. The CPU numbers above are inferences from the set's cost per flip.
- What 30 and 15 a second look like on the panel. The steps are at most 1.5 px at 15, 4 at 30 and 5 at 60, and the strips show the frames, but nobody has watched them move.
- The pacer with 66 ms intervals, and the sleeping loop, on the hardware (both pass on the host: `ticker.rs` tests, `ui/tests/ambient_motion.rs`).
- The frame's own cost: 1.53 ms of the engine's CPU for 51 draws. Candidates, to be chosen from the probe's phase totals: one GL state save and restore (`Gles::enter` and `leave`) round plan, draw and finish instead of four, and a cached copy of the sky's static layers. Both change the engine and need a TV measurement.
- The 4 % of time a storm's strikes draw at every refresh, the weather view's rain (120 a second as before), and what a profile with other cards and a wallpaper adds.
