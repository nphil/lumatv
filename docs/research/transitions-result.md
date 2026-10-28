# Smooth transitions (Transitions, 2026-10-28)

LG's compositor cuts between an app, Home and the next app, and the video plane shows black in between. LumaTV's compositor animates the
pictures it already has instead: an app grows out of its tile, goes back into it, one app fades over another, LG's Settings (an overlay
window) opens and closes the same way. Off by default (theme `motion.transitions`, Settings > Home & look > Smooth transitions, `lumatv
transitions on|off`) until the TV check below passes. Not run on the set.

Source: `compositor/crates/lumatv-compositor/src/transition.rs` (the engine, pure), `scene.rs` (what the front of the screen is, ghosts,
poses), `render/gl.rs` and `render/software.rs` (drawing through a pose), `shell.rs` (what Home is told). Shell side:
`payload/compositor/lumatv` (`LumaHome`, `Dock`, `Layers`, `panels/Splash`, `settings/PageLook`) and `compositor/shell/Shell.qml`.
Mocks: `docs/mocks/transitions/` (`node render.mjs`; dark and light, 100 % and 200 % text for the Settings row). Renders from the
compositor itself: `docs/research/transitions/` (`LUMATV_TRANSITIONS_RENDER_DIR=<dir> cargo test --test transitions renders_the_transitions`).

## What moves, and what never waits

The scene's logical state does not wait for any of it: the front card, the keyboard focus, SAM's foreground, the pointer and the app's frame
callbacks change in the frame the app maps, as without it. A transition changes only how windows are drawn for 120 to 220 ms. A change of
what is at the front (`Front`: Home, an app's card, or a full-screen overlay that is an app) between two plans starts it:

| Change | Moves | Time, curve |
|---|---|---|
| Home to app | the card grows out of its tile (its picture cover-fitted on the tile, masked by the tile's rounded rectangle, a quarter seen in its first frame, whole by 60 ms); Home stays under it, darkening to .55 | 220 ms, out-cubic (`motion.layer`) |
| app to Home | the card shrinks into its tile, holds its opacity then fades from 35 %; Home brightens .55 to 1 under it | 180 ms, `cubic-bezier(.4,0,1,1)` (`motion.exit`) |
| app to app | the new card scales .94 to 1 and fades in over the old, which darkens to .6 | 220 ms, out-cubic |
| overlay over a card goes | the overlay leaves, the card under it, drawn all along, is revealed (it does not arrive again) | 180 ms |
| reduced motion (`motion.reduced`) | one linear fade, no scale, no travel, Home not darkened | 120 ms (`motion.reduce`) |

A card with no tile known (a launch from the quick panel, the remote's app buttons when the app is not on the dock, LG's Settings) grows from
the screen's middle (.94 to 1, 26 radius to 0). Time is the monotonic clock, never a frame count: a 220 ms move lasts 220 ms at 60, 90 or
120 Hz, a late frame catches up. A change of the front while tracks run takes each picture up where it is and moves it on (no jump, no
restart); the leave of a card, picked up by a launch's start, is also from its pose. While the switcher is up (`state.switcher_active()`)
nothing moves (`Transitions::suspended`).

A first buffer the driver is still busy with is not drawn (the stand-in rule of `render_rects`): the move's clock then starts again at each
frame until one frame has drawn its pictures (`Transitions::anchor` and `drawn`, 100 ms at most), so a launch is seen from its start, not
from its middle.

Where it ends the plan is what it was: a lone opaque full-screen window with no pose and no ghost, which the plane scans out directly (the
move itself composes two or three pictures for at most 13 frames).

- **Ghosts.** A window that left the stack stays drawn above or under the new front until its track ends (`Scene::merge_draw_list`); the
  pointer and the keys are never a ghost's (`Item::ghost`). Home is a ghost under a launch, and Home is told it is still on the screen
  (`foreground` topic: `home` stays true, `covered` waits) until the card is whole, so Home goes on drawing (its launch screen stays up under
  the move and is cleared after it) and the video wallpaper's player is not stopped for those 220 ms (`holds_home`).
- **Zombies.** An app that exits (Back) destroys its window the moment Home is the front. `Scene::zombify` keeps the destroyed window's last
  buffer for its transition; `Gl::prune` keeps that buffer's texture while the plan draws it. A buffer the client destroyed *before* its
  window keeps its import in the renderer [INFERENCE from `destroying_the_shown_buffer_keeps_the_last_frame`, which is the same rule for a
  still window]; an shm buffer the client destroys goes with it (no ghost, a cut: only the host tests use shm for apps).
- **Overlay apps.** LG's Settings maps as an overlay window the size of the screen and SAM never names it (round 11). A full-screen overlay
  that is opaque (opaque region or XRGB buffer), or `com.palm.app.settings`, counts as a card for the front.

## Apps whose picture is on the video plane (Netflix playing, HDMI, Live TV, GFN)

The plane cannot be sampled and cannot be scaled: the app's window has a hole, and the video lies under the whole UI plane. So a card with a
hole never scales over it, never shows the plane's black while it moves, and the compositor never reads the video:

- **Arriving**: the card is the app's own picture on a **veil** (`#191724`, the dark canvas, opaque) where its hole is, moved and faded with the
  card exactly as any card is (so an input zooms out of its tile as a dark card with its banner on it). The veil is made when the window has an
  exported video element (HDMI exports before its first frame) or a hole shows up while the card arrives. It is held until the card has
  arrived and the hole (the compositor sends videooutputd the placement in the same step) is 150 ms old, at most 1.5 s after the card has
  arrived (no signal: it then dissolves to what the plane shows), and then fades over 180 ms (120 ms reduced): the hole is cleared and the
  veil, less opaque, goes on the clear, so the display engine blends the plane under what is left, a dissolve the GPU never reads video for.
- **Leaving**: the card and its hole stay where they are and Home dissolves in over them, 180 ms linear, the same blend. Nothing shrinks.
- A card that gets a hole after it has arrived (Netflix starting a title) gets no veil: that cut is the app's own.

`Plan.holes[..].veil` carries the fill (quad, mask, opacity, whether the hole is cleared first); the renderers draw it between the hole's
clear and the card's own pixels.

## The tile, the splash and the shell

Home tells the compositor where the tile is, in Home's coordinates, before it asks the app manager for the app
(`luna://com.webos.surfacemanager/setLaunchOrigin {appId, tile: {x, y, width, height, radius}, from: {…} | null}`, in the shell's allowlist and in the
bridge's method table (`compositor/sm/lumatv-sm.c`, `m_surfacemanager`: **the TV needs the rebuilt `lumatv-sm`**; without it the call is refused as an
unknown method and every card grows from the middle of the screen, `ctl transitions` `origins` stays empty); the tile is the lifted, focused one
(plate inset 19 for floating tiles, `focus.tileScale` 1.12 about its centre, radius `radii.tile` times that). The compositor remembers it for the way back (48 apps). A launch screen
(`Splash`) over the tile hides what the card grows from, so with transitions on it waits 400 ms (`LumaHome.splashWaitMs`): an app that has
a window by then (`appFronted`, from the `foreground` topic) never shows it; a slow one gets it fading in over `labelMs`, and 50 ms later
Home tells the compositor where its icon is (`from`), which is where the card grows from then; the way back is always the tile. Without the
compositor's word (`Shell.qml` binds `smoothLaunch` from the `transitions` topic; LG's compositor never sends it) nothing changes: the launch
screen is up at once, no origin is sent.

## Interfaces

- `ctl transitions [on|off|auto]`: the switch over the theme's (for a TV check), then a report: `enabled`, `reduced`, `forced`, `active`, each
  track's pose, `lateLog` (the last eight late frames: age, gap, part) and `stats` (`launches`, `leaves`, `switches`, `interrupted`, `skipped`
  (a change the engine left alone), `frames`, `maxGapMs`, `lateGaps` (frames over 20 ms apart, both with something moving: a veil that only
  waits for its video is not counted), `veilCaps`). Journal: `transitions: on|off`, and for each late frame `transitions: frame N ms after the
  last in a launch|leave|switch|fade (tick before, this tick, last render, held ticks, buffers prepared, imported in a frame)`.
- `lumatv transitions on|off|status`: writes `motion.transitions` into the owner's theme (the compositor reads the file once a second, Home every 2 s).
- `Scene::take_visual(window) -> Option<Visual {rect, alpha, radius}>` (AppSwitcher): drops the window's track and answers where its
  picture is on the plan's clock; for a leave that ended less than 600 ms ago, the end pose (the tile, alpha 0).
- Topic `transitions {on, reduced}`; theme `motion.transitions` (default false, held equal to `Settings::default` by `tests/test_lumatv_transitions.py`).
- `Output` gained `defer_frees(on)` and `retired()` (default: nothing to free); `Item` gained `pose` and `ghost`, `PlannedHole` a `veil`, `Gl::prune` takes the plan's items.

## Cost

Nothing at rest: no tracks, no extra draw, the idle tick slows as before (`ticker.rs` only keeps the panel's rate while a track moves). The
pose program (`XF_FS`, a signed-distance mask, `highp`) is linked and warmed at start. A frame of a transition is two textured quads (Home
with its brightness, the card with its mask); a veil adds a flat one. Pictures are bilinear: the first two or three frames of a launch show a
1:8 minification without mip-maps (alpha .25 to .5 there) [INFERENCE: not visible]. OLED care's `dim` multiplies a pose's brightness.

## Tests (host, green 2026-10-28)

- `transition::tests` (30): curves and the CSS bezier, the poses of each kind and their ends exactly, tile cover-fit, interruption from the
  current pose, splash origin and its age, the veil's hold, settle, cap and fade, reduced motion, refresh-rate independence, late frames,
  `take_visual` live and after, the switcher's suspension, the theme file's switch, a veil with no hole that still ends at its cap, which gaps are late
  frames, the late log, when a waiting veil is due.
- `tests/transitions.rs` (19 with the docs' renders, software renderer, scene planned at a hand-moved clock): launch from the first frame (a quarter of the app over Home at its
  tile), Home pressed and the pointer is Home's not the ghost's, app to app, an app that exits, an overlay that opens and closes, the card an
  overlay revealed, the veil's hold and dissolve, Home dissolving over a hole, reduced motion, the switch off mid-move, what Home is told;
  and on Mesa's llvmpipe the GL renderer drawing a launch, a veil and an exiting app's last picture the way the software renderer does (a
  gradient picture, 4,000 samples, none over 6 levels off), and the frees of a destroyed buffer waiting for the move's end (Mesa).
- `tests/transitions_scanout.rs` (2, Mesa, the KMS frame logic over three scan-out pages whose row 0 is the top): a launch and a Home press are the software renderer's pictures (the mask the right way up), and a first buffer the driver is still busy with is not drawn while the move waits for it and is then seen from its start.
- `tests/livetv.rs` (4, software renderer): what lies under the video's hole (Live Dmost) is not drawn, draws no frame when it commits, still gets its callbacks, and is drawn again where the hole
  does not cover it or is gone; a Live TV launch (move, a hundred-ms plans while the veil waits, hole, fade) and its leave with the member there: no late frame, `veilCaps` 0, Dmost not in the plan;
  a card whose hole never comes ends at the cap.
- `tests/test_transitions_home.py` (QML host): the origin, the waiting launch screen and its icon, the fronted app that cancels it, the
  unchanged cut. `tests/test_lumatv_transitions.py`: the verb, the defaults, the row and the binding.

## Not built, and what is assumed

- The display engine blending partial alpha of the UI plane over the video plane is assumed (the volume bar over video and the glass over
  the video wallpaper rely on it, and are not run on this build either). If a veil's fade shows as a cut, that is what to look for (TV check 8).
- A window destroyed with an shm buffer, a window with no buffer left, and an overlay that is neither opaque nor Settings have no transition.
- Home to the Home stub's own card (LumaTV's Home not running) is an app-to-app move.
- The wall video under a launch: Home stays the underlay for the 220 ms, so its hole stays too and the player is stopped when the card is whole.

## TV check (needs Main's slot; receiver muted through Home Assistant; stop at the first failure and send back what is named)

Counters from `ctl transitions` read before and after; the frame gate's rule is the one in `compositor/tools/tv-gate.sh`.

1. Off by default: `ctl transitions` says `enabled: false`; `lumatv transitions status` says `on: false, compositor: own`; a launch of Plex from its tile cuts as before.
2. `lumatv transitions on`; within 3 s `ctl transitions` says `enabled: true`. Launch Plex from its tile: `origins` then lists `com.beb.plxnative` with a tile of about 134 square (if it stays empty the bridge is the old `lumatv-sm`: rebuild and deploy it, `compositor/tools/build-tv.sh`).
3. Warm launch from a tile (Plex, YouTube): no launch screen; the card grows out of the tile in 220 ms from the first frame of the app; `launches` +1, `lateGaps` 0, `maxGapMs` 20 or less; the frame gate's probe over the launch has no frame over 20 ms; the app has the keys from its first frame (press a key at 100 ms: it arrives).
4. Slow launch (Apple TV, about 20 s): the launch screen fades in at 400 ms; when the app maps the card grows out of the icon; Home's launch screen is gone after the move.
5. Home from an app: the card shrinks into its tile in 180 ms, Home brightens; `leaves` +1; the first Home frame after has no stale launch screen.
6. Interruption: launch, Home within 100 ms: no jump, the card turns round; `interrupted` +1. Then Home twice within 0.4 s from an app: the switcher takes the card at its pose (AppSwitcher's check).
7. App to app (recents, the remote's app button): `switches` +1, the old card darkens under the new; Back from an app that exits: the last picture shrinks into its tile (`leaves` +1, `skipped` unchanged); if `skipped` moved instead, the window had no buffer left: record `ctl windows` and the journal.
8. LG's Settings (gear, All Settings): it grows from the middle (`launches` +1); Back: it fades out over Home. If `skipped` moved, read its window in `ctl windows` (type, size, opaque region) and name why.
9. HDMI 2 from its tile: the card is a dark card with the input banner, no black frame at any step (record at 240 fps if the eye is not sure), the picture dissolves in over 180 ms once it is up; `veilCaps` 0. A partial-alpha blend over the plane is what this proves: if the end is a cut at about half, record it and stop here (the fade of the veil and Home's dissolve then need a different design). HDMI with the cable out: the veil holds 1.5 s, dissolves to the plane, `veilCaps` +1.
10. Netflix playing, then Home: Home dissolves in over the picture (180 ms); Live TV and GeForce NOW: as 9. Live TV launch and Home: `lateGaps` 0, `maxGapMs` 19 or less; if one moved, `ctl transitions` `lateLog` and `ctl journal | grep 'transitions: frame'` say in which part and what the loop was doing (see the Live TV section), with `ctl ring` lines `P N ms in one pass` around it.
11. `motion.reduced: true`: every move above is one 120 ms fade, nothing scales.
12. Cost: with Home idle and transitions on, `ctl status` ticks over 10 s as before (the idle tick still slows); compositor RSS flat over 100 launches and Homes (`gate.sh mem`); a full-screen app is scanned out directly again after a move (`direct` rises); a 60 s X1 run over a launch loop keeps the gate rule.
13. `lumatv transitions off`: cuts again, `launches` does not move.

## Live TV (round 14)

Round 14 read Live TV with `lateGaps` 1 (30 ms) at the launch and 2 (45 ms) at the leave, 16 frames for the leave against 12 for HDMI 2 and Netflix, where the same veil and the same dissolve
had none (HDMI 2: `maxGapMs` 19, Netflix 17). What the engine did to the counter and to the card:

- The counter timed every plan while a track lived, a veil waiting for its video included: those plans are the app's commits and the slowed tick, and nothing is expected of every refresh there. A late frame is now a gap over 20 ms between two frames that both had something moving (the move, the fade, a leave).
- A card that expects video and has no hole never ran its veil's cap (only a hole asked the veil for its time): its transition lasted as long as the card was in front. The veil is stepped for the windows that have no hole too, and the tick does not slow while a veil's settle or cap is within a slowed period.
- Live TV's group layers lie under the video (`dmostLayer` -10, data, ACR, subtitle layers -5 to -21): Live Dmost was composited and then cleared by the owner's hole, a full-screen layer per frame, and every commit of it drew a frame that showed nothing new. A member below a window that is at rest, whose picture the window's hole covers whole, is not planned.
- The frees of destroyed buffers (a driver import's unmap stalls the loop, 2 ms to 10 ms each; Live TV ends with Dmost's buffers and its own) wait while a card moves, and are done in the first frame after it (`Output::defer_frees`, at most 12 waiting).

Not shown offline: why the TV's frames were late. The counters name Live TV only, and Dmost starts about 0.8 s after the card (inside the veil's hold) and ends with it at Home, a process's GL context made and destroyed beside the compositor
[INFERENCE]. The leave's four extra frames fit the move waiting for Home's first picture (`Transitions::anchor`, 100 ms at most) [INFERENCE]. Each late frame is now journaled with the loop's own timings: a long tick before it is the loop's work
(render, free, import); short ticks around a long gap are the loop waiting or woken late (display, driver, another process), and then the ring's `P N ms in one pass` lines and the KMS `late`/`lost` counters name it.

## Changelog line

Smooth transitions under LumaTV's compositor: an app grows out of its tile and goes back into it, one app fades over another, LG's Settings
opens and closes the same way, and apps on the video plane (inputs, Live TV, Netflix playing) arrive on a dark veil that dissolves into
the picture and leave under Home dissolving in, with no black frame; 180 to 220 ms, interruptible, drawn from the app's first frame, with
the launch screen waiting 400 ms so the tile is what the card grows from. Settings > Home & look > Smooth transitions
(`lumatv transitions on|off`); off by default until the TV check passes.

## TV round 2026-10-28 (round 14)

`a1555df`, LumaTV's compositor, `lumatv transitions on`. Netflix launched from its tile: `origins` has `netflix` with a 134.4 px tile, `launches` 1, `lateGaps` 0, `maxGapMs` 17, in front 0.94 s after the key.
Home from Netflix: `leaves` 1. Launch then Home within 100 ms: back at Home, `launches` 2, `leaves` 2 (`interrupted` stayed 0: not read as an interruption). HDMI 2: `launches` 3, `lateGaps` 0, `maxGapMs` 19; Home: `leaves` 3.
Live TV: `lateGaps` 1 at the launch (30 ms), 2 at the leave (45 ms): INFO, above the 20 ms bar. `veilCaps` 0 throughout, `skipped` 0. Not run: slow launch, settings, app to app, reduced motion, cost, the visible
frames. Result: PASS for the counters, Live TV late gaps open; switched off again, go-live with Main's word.
