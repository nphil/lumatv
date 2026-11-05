# Settings at 120 Hz: why frames miss (round 21 gate, round 22 ring)

## Resume here

On forge main (round 24's engine work):

- **Session 6 on `c7704853`** (120 Hz): quick `burst_over20` 3, settings-scroll 6 drops and 12, home-widgets 4 drops and 9. Two causes, found on the host
  with the owner's own theme and layout (`tests/host_budget.py --owner <the gate's receipts>`, which also serves the Smart home card's two cameras a still
  a second and lists the shell's own CPU time a frame, `ui`):
  - **Camera stills on the UI thread.** Home-idle, home-dpad, home-widgets and quick each had a UI stall of 18-24 ms twice a second, 0.2 s apart, inside
    their key bursts. A still (a 24-bit BMP at its cell's size) was fitted by a per-pixel sampler at its own size, laid over a cleared layer by a second
    per-pixel pass, scrimmed in floats and repainted for every camera when either arrived: 13.5 ms of the thread on the host, 14-16 ms a frame. A fit at
    the picture's own size copies rows, the layer takes the picture as it is, the scrim's rows are hoisted and a cell repaints for its own camera's still:
    1.1-1.8 ms; no home-widgets frame takes more than 3.4 ms of the thread now.
  - **The dims in one frame.** The focus entering the cards dimmed the other cards and the dock together, and each page of the ring drew all of it:
    118 layers a frame on the host, a GPU busy of 13.1 ms on the set at +1.03 s. They step one at a time, three refreshes apart now: 84 layers.
  Not changed (host numbers on the owner's layout; none of this was run on the set): the dock's own step with the cards settling (101-104 layers for
  three frames at 'to the dock', 'along the dock' and 'down and up the dock'); a lift's spring counts as settled only within 0.0005 of its travel
  (`Units::Progress`, `compositor-protocol/src/motion.rs`), so a card whose lift is gone is redrawn whole, 24-53 layers a frame, for about 40 frames after
  a move (opacity 0.0013 at the 44th); Settings page changes (45-69 layers on the host, 87-120 on the set, 3-9 ms of UI for the rows) and Settings
  opening (the rail's rows made cold, 5-8 ms of UI on the set); the quick panel's first open (3-6 ms of UI against about 1 at the later ones).
- `91589d0f` **Home from one picture.** The engine keeps stack 0 (Home) whole in a render target of its own, the floor (`rustui/render/src/host.rs`): a
  frame draws into it what Home changed, and a page gets Home as one copy of it in each rectangle it owes. A panel or Settings moving over a Home that
  stands draws its own layers and one copy, not Home's 70-90 in every pass, and the floor is current whenever Home shows, so an opening's first frames
  only copy it. The panels' sheets have no backdrop of their own (`kit::pane`: tint only): their first frames were Home drawn again under the dim, which
  appears over the whole screen, once for each page of the ring. The compositor also merges a page's rectangles where one pass over the engine's layers
  costs less (`Engine::merge_areas`, the floor counted as one layer), within a quarter more pixels (below). Settings no longer covers Home (5a4d3f65 undone): under the floor Home costs one copy,
  and the cover cost a whole frame as Settings opened and closed. The toast's `burst_end` limit is 300 ms (its slide is 220 ms).
- `f14f87dd` **Pages copy what they owe.** A composed frame copies from the page drawn last what its page owes from the frames before (a framebuffer blit)
  and draws only what it changed (`kms::frames`): a whole-screen change is one frame of drawing and two copies, not three frames of it.
  `LUMATV_FLOOR=0` and `LUMATV_PAGE_COPY=0` in the compositor's environment (`surface-manager.env`) switch either off.
- The host harness draws its pages as the compositor does (two stacks at the shell's `system_z`, the ring, the copies) and keeps its own time with
  `--virtual` (one refresh a frame, however slow the host). `tests/host_budget.py` measures each gate surface that way; `test_frame_budget` holds quick,
  picker and settings-scroll to 80 layers a frame at 120 Hz and every frame to the same frame drawn whole.
- **Round 26 on a988b80a** (120 Hz, pop-ups off), the compositor's own count against round 23: settings-scroll 41 -> 5, quick 7 -> 0 (burst_over20 1),
  picker 5 -> 0, home-dpad 0 -> 1, but apps-grid 3 -> 13 and home-widgets 3 -> 24 (with `LUMATV_FLOOR=0` 8 and 22, with `LUMATV_PAGE_COPY=0` 18 and 23);
  60 Hz settings-scroll 3, apps-grid 2, home-widgets 6. A page's rectangles merged by the cost of a pass (`MERGE_GROWTH_QUARTERS`: a cover may draw at most
  a quarter more of the panel than the rectangles, for the page and for the floor's own passes) was this round's reading of the rings; round 27 shows
  the cause was elsewhere (below).
- **Round 27 on 93496e61** (120 Hz): apps-grid 29, home-widgets 28, settings-scroll 9, the rest 0; at 60 Hz settings-scroll 5, apps-grid 1,
  home-widgets 6. The old path (`LUMATV_FLOOR=0 LUMATV_PAGE_COPY=0 LUMATV_AHEAD=0`) at 120 Hz: apps-grid 3, home-widgets 3. Nearly every drop is a frame
  whose GPU work outran the refresh. Home's own walks draw the same rectangles on both paths, to the pixel; through the floor and the copies they cost
  more: 0.9 Mpx of Home in one rectangle drew 122 layers and took 7.5 ms of GPU (39 layers and 4.2 ms on the old path), the floor's passes split by the
  quarter's bound and copied into the page after; frames of few layers between them took more as well (0.23 Mpx and 20 layers: 7.7 ms at the median
  against 2.8), which the ring cannot divide between the pages' copies and the heavy frames queued before them. The floor and the copies are now
  used only while something of stack 1 is over Home (a panel, Settings, the picker, a toast: what they were made for, and the three frames after one,
  while the pages still owe its place); Home alone is drawn as on the old path, straight into the page, each page drawing all it owes. The floor keeps
  owing what Home changed meanwhile, and the first frame in which Home stands and nothing waits to move draws it (`Engine::finish`), so a panel
  opening over Home still only copies it. Taking a large commit in ahead of its frame (`take_in_ahead`) did not shorten
  the frame after OK on "All settings" (7.7-10.9 ms of the loop with it): it is off unless `LUMATV_AHEAD=1`.

The most layers one frame drew on the host (`tests/host_budget.py`: Home with its feeds, wallpaper motion and the weather's sprites off as the gate has
them), with the floor and the copies off (`LUMATV_FLOOR=0 LUMATV_PAGE_COPY=0`) and on, and the GPU estimate at 1.5 ms + 0.085 ms a layer:

| Surface | 120 Hz off | 120 Hz on | 60 Hz off | 60 Hz on |
|---|---|---|---|---|
| quick | 112 (11.0 ms) | 36 (4.6 ms) | 122 | 36 |
| picker | 98 (9.8 ms) | 11 (2.4 ms) | 100 | 11 |
| settings-scroll | 175 (16.4 ms) | 68 (7.3 ms) | 160 | 75 |
| home-widgets (Home alone: the old path either way) | 115 (11.3 ms) | 115 (11.3 ms) | 115 | 115 |
| toast | 25 | 12 | 25 | 12 |

The set's Home and Settings have more layers than the host's (round 22: Home 70-87 in a frame, a Settings frame 87 of its own), so its frames are
heavier by about that much.

Of the plan, two items were not done:

- **Settings keeping its page rows.** A page change draws the page's layers whether its rows are made new or kept: the cost is the layers a frame's
  passes draw, not how new they are. With the copies a page change is one frame (68 layers on the host at 120 Hz), not three.
- **home-widgets' dim as one layer.** Whatever changes in stack 0 is drawn again into the floor with everything under it, so a dim layer there costs what
  the cards' restyles cost. A dim layer over the floor (stack 1) would cost two layers but is another look (it darkens, where the cards' 86 % opacity
  shows the wallpaper through) and would sit over an app when Home is not in front. The focus entering the cards is one frame now (119 layers on the
  host: the cards and the dock with what is under them), not three. The look or group pictures in the engine is the owner's call.

TV check list (both gates, pop-ups off):

1. Install forge main at the commit after `1830f537` that keeps the floor and the copies to frames with something over Home, or later; the
   compositor's and ui's md5 equal the build.
2. The receiver muted (the gate turns pop-ups off and back on itself).
3. 120 Hz first: `python3 compositor/tools/tv_gate.py --only frames --frames-quick --refresh 120`; `panel_refresh` 119.9 on every surface.
4. Nothing worse than round 23 (120 Hz): apps-grid and home-widgets 3 or fewer (round 27's old path: 3 and 3), home-dpad 0, quick 0, picker 0, toast 0,
   settings-scroll 9 or fewer (round 23: 41).
5. If apps-grid or home-widgets drop more: those two with `LUMATV_FLOOR=0 LUMATV_PAGE_COPY=0` once, to compare; nothing else should change them now.
6. 60 Hz: the same without `--refresh`; `panel_refresh` 59.9; settings-scroll 5 or fewer, apps-grid 1, home-widgets 6 or fewer (round 27).
7. Rings (`perf/ringsum.py`): Home's frames of 0.89-1.01 Mpx in one rectangle (891648, 912384, 1011200 px) at round 23's 39-46 `layers` again (round
   27: 122-130); after a walk on Home, one frame of `owed 0 px` drawing Home's layers (the floor brought up to date); a panel's and Settings' frames at
   about 80 `layers` or fewer.
8. By eye: Home under the quick panel, the picker and Settings, as they open and after each closes (the floor up to date after a walk on Home); a
   toast and the volume over a playing app.
9. Compositor log: no `page copy: GL error`.
10. settings-scroll at 60 Hz, the frame after OK's release (round 27: `P` 7.7-10.9 ms against `render` 2.3-2.7, flip 33.4 ms after the last): not
    fixed, not expected to change.

**settings-scroll at 60 Hz in round 26** (3 dropped, 1 a cycle). The frames of 150-217 ms at +7.2 s of the open window, and of 183-300 ms in the close
windows, are idle gaps after the walk's last frame: the compositor's thread ran 0.6-0.8 ms in them and the GPU 0-7 ms, nothing was due, and the ring
counts none of them. The drops are the walk's first frame, the same in each cycle: OK on "All settings" starts the quick panel's slide-out and
Settings' fade in one commit (Settings made: about 100 layers). The engine draws the fade's first frame at the latch the pacer set from the cheap
frames before it (8.8 ms before the refresh), and that pass takes 7.0-7.2 ms of CPU (render 2.8-3.0 ms), so the frame misses its refresh; the next
frame takes its place, and the flip comes 33.4 ms after the last (`late 1 lost 1 missed`). Rounds 22-23 had the same frame (0-4 drops
on this surface at 60 Hz): not from this round's changes. Taking the commit in as it arrives (`Engine::take_in_ahead`, `LUMATV_AHEAD=1`) moved 323 us
of it out of the frame on the host; on the set the frame still took 7.7-10.9 ms of the loop (round 27), so the pass waits on something else, which a
trace of that frame (`perf/frametl.py`) would show.

Next, in order:

1. The TV round above.
2. The frame after OK on "All settings" (item 10): a trace of its pass.
3. Home's frames of 1.16-1.93 Mpx in one rectangle (1158144 px, the host's bounds of the status bar and the cards: 6.8 ms of GPU at the median in
   round 23, 7.1 in round 27): the compositor's `damage::Tiles::rects` grows only its last rectangle, so things side by side are a rectangle a tile row
   each and, past 12, their bounds. Growing every rectangle the row above ended with (as the engine's `TileSet::rects` does) drew Home's walk on the
   host in 0.4 Mpx where the bounds were 1.1; untried on the set.
4. home-widgets' focus-enter dim (the owner's call above).
5. Re-check after any change: `python3 tests/host_budget.py --refresh 120` on the host (it counts layers, not pixels: the set's per-pixel cost is not in it),
   then the gates on the set; rings as in 7.

## Rounds 22 and 23

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
  3. **Home is calm** (`Home::set_calm`, `Shell::after`): its wallpaper stands still and its weather rests while anything is over it, and 450 ms after.
- Round 22 on 888dc431 (60 Hz, pop-ups off, panel 59.9 Hz everywhere): home-dpad 17, apps-grid 2, home-widgets 7 dropped, quick/picker/settings-scroll/home-widgets
  `burst_over20` 2-9, toast `burst_end` 1995 ms. Read off the rings, these were mostly the gate:
  1. **The trace cannot tell which frames were animated**: a key's response frame, a frame the idle tick drew, the last frame of a motion and the first of the next
     key look alike, so home-dpad read one "drop in motion" per key (keys 350 ms apart) while the compositor's own count was 0. The gate now takes `dropped` from
     the ring's flag (`anim`: a frame with an animation still running after it; `ringsum.motion_drops`, the rule of `refresh::Measure::motion`), per window, and
     falls back to the trace where the ring does not reach the window.
  2. **The weather card draws every refresh** (a sprite layer moving on engine animations, since after round 21): every flip of an idle Home is "after an animating
     frame", every key's first heavy frame (12.8 ms of GPU, 87 layers, the whole screen) is judged as a drop, and no burst of frames ever ends (toast `burst_end` =
     its 2 s hold). The gate's `still` now switches the card's sprites off with the wallpaper's tick (theme `weather.animated`, default on, shipped in the theme; the
     user's file is put back). The input calm of b421f486 (2 s after a key) is gone: it was there for these bursts, and its only proven effect was on the gate.
  3. **What is real**, by the compositor's own count per window, 60 Hz: quick 3 and picker 1 (the first frame of an open, after weather frames: gone with them off),
     settings-scroll 7 (mid-walk: a page change is three frames of 11-12 ms GPU, 149 layers, 1.49 Mpx, and the first is late), home-widgets 3 (+1.07 s: the focus
     enters the cards, they all dim, 166 layers, 9-13 ms), apps-grid 1. Each is the first heavy frame after cheap ones: the pacer plans for the cost of the frames
     before it, so a step in cost lands one refresh late (`late 1 lost 1`, 33.4 ms), and the frames after it are on time.

- `807b87e8` the gate as above (ring count, `still` covers the weather card, no input calm), then the cover below.
- Round 23 at 60 Hz on 89507fcb (panel 59.9, pop-ups off), the compositor's own count: home-dpad, home-widgets, toast 0; quick 3, picker 3, settings-scroll 4, apps-grid 4,
  every one the first heavy frame of an open (a whole-screen frame of 45-180 layers, 8-15 ms of GPU, after cheap ones: `late 1 lost 1`, 33.4 ms). Idle Home drew a frame
  every refresh (13 layers, 78-83 kpx, `anim`) all through the run, the size of the weather card's sprites on the host (77 kpx), which stopped while a panel was up (the
  calm): the gate's `weather.animated=false` had not reached the card. The cause is in the gate: it judged the wallpaper's switch alone, and a file with the wallpaper
  off (the owner's, or an earlier run's left over) made it change nothing. It now reads and sets each switch (`MOTION_SWITCHES`: `wallpaperMotion.enabled`,
  `weather.animated`) on its own and puts back only what it changed. With the card drawing, the frame before every first heavy frame is "a frame with a motion running
  after it", so that frame is judged and counted, and the pacer, sized by cheap frames, has no time for it; with the card at rest the first frame of an open follows
  a still screen and is not judged, the next ones are on time (the pacer sizes them by the slowest recent frame).
- Round 22 at 120 Hz on 888dc431 (panel 119.9 Hz in every window, pop-ups off), the compositor's own count per window (art on): home-dpad 2 of 803 flips judged,
  apps-grid 2 of 1303, home-widgets 4 of 1683, toast 0 of 2528, picker 7 of 38, quick 9 of 91, **settings-scroll 192 of 2063** (the trace said 6). The panels' drops are
  the first two frames of an open (12-13 ms of GPU, cold layers), the rest of a panel's frames take 5-7 ms. Settings' are the page changes: the rail's steps
  (3.0-4.25 s of the walk: 28 drops) and the way back (5.75-6.75 s: 32), frames of 150-190 layers at 10-16 ms.
- **What a frame costs** (604 frames of an idle GPU, 120 Hz run; `gpu = 1.5 ms + 0.085 ms x layers drawn`, R2 0.47; pixels and rectangles add nothing, R2 0.08):
  the layers a frame touches, whatever they cover. The 8.3 ms of a 120 Hz refresh is about 80 layers. Home alone is 70-87, a panel's frame 100-125, a Settings frame
  150-190, and Home was 45 % of a Settings frame under sheets that leave it only the margins.
- **Settings covers Home** (`Home::set_under_modal`, `Shell::after`): once Settings' sheets have faded in (at once from 100 Hz) Home's cards, dock and status bar
  step back as they do for the Apps library, and are back the moment Settings closes. The host's Settings step has 10-14 layers fewer (its Home has no dock tiles;
  the set's has 70-odd), and the picture is the same to the eye (cards showed faintly through the 96 % sheets and in the gutters).
- **Settings' rail walked fast makes the page it comes to** (`SettingsSurface::page_waits`): a step within 200 ms of the last page made, with the focus on the rail,
  waits until the rail has rested 120 ms; the first step after a rest, a step into the page and a tab make their page at once. On the host a fast walk (8 steps 60 ms
  apart) draws 21.5 Mpx in 11 heavy frames where it drew 30.7 Mpx in 25, and ends on the same page; a walk of deliberate steps (400 ms) is unchanged
  (`tests/test_settings_rail.py`, `page_waits` unit test).

- **Round 23 on 89507fcb with the gate of 25cb7b06** (panel 59.9 / 119.9 Hz, pop-ups off; the owner's file has `wallpaperMotion.enabled=false` and no `weather` key, so the
  weather card is switched off by the gate for the still surfaces and the idle per-refresh frames are gone from the rings). The compositor's own count:
  - 60 Hz: dropped quick 1, home-widgets 2, the rest 0; burst_over20 home-dpad 1, quick 2, settings-scroll 1, home-widgets 2, the rest 0; burst_end quick 187 ms (limit 180)
    and toast 255 ms. What is left is the first whole-screen frame of an open (quick) and the step where the focus enters Home's cards and the others dim (home-widgets, +1.07 s of
    its walk: 155 layers, ~13 ms of GPU, one frame at 33.4 ms): a frame of 12-13 ms GPU after cheap ones lands one refresh late whatever is done before it.
  - 120 Hz: dropped quick 7, picker 5, settings-scroll 41, apps-grid 3, home-widgets 3, home-dpad 0; toast burst_end 247 ms. Panels: the first two frames of an open cost 12-15 ms of GPU
    each (picker's have 13 layers and 83 kpx: the cost is not layers but the glass's backdrop capture, which a panel's first frame pays), Settings: its opening (8 frames of
    10-19 ms), the rail's page steps and the page changes. Not reachable at 8.3 ms without the engine's help (a backdrop made ahead, Home as one picture under a fading panel).
  - The toast's burst is its own slide (220 ms, the layer beat) against the gate's 180 ms limit for the burst end: it failed so in round 21 too (241-253 ms). The limit or the beat is
    the owner's to choose; nothing here moves it.

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
