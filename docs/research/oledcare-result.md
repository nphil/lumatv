# OLED care: LumaTV's own screens on the panel

Built and host-tested; off until its TV check passes. Switch: `theme.oledcare.enabled` (Settings › System › OLED care › LumaTV panel care › LumaTV screens, or `lumatv theme set '{"oledcare":{"enabled":true}}'`; the row moved from Home & look › Screensaver with `docs/research/oledcare-apps-result.md`). The compositor reads `theme/theme.json` at the idle tick, at most every 2 s, so a change shows within 2 s. Mocks: `docs/mocks/oled-care/` (the row as it was on the Screensaver tab, its help sheet, a figure of the orbit and the dim; dark and light, 100 % and 200 % text); the row's present place is `docs/mocks/oled-care-apps/`.

## What it does

Only the shell's two windows are touched (`ShellHome`, `ShellSystem`: Home with its status bar, clock and dock, and the system layer with the volume bar, mute mark, banners, toasts and prompts). An app's card, a video, the moving wallpaper's plane and the keyboard window are never moved or dimmed.

| | Rule |
|---|---|
| Orbit | Both windows are drawn at one offset that walks a square ring of `orbitPx` (3) pixels, one pixel on one axis per beat (`stepSec`, 30 s): 24 positions, 12 minutes a lap. From the origin it first walks to the ring the same way, so nothing jumps. |
| Beat | Taken on a frame that is drawn anyway (any window with a commit to draw); on a still screen 2 s after it is due, by the idle tick the loop already has. No timer of its own. Its frame does not hold the tick at the panel's rate (`Output::quiet_frame`: the screen is at rest 150 ms after that frame's flip, not 1 s). The beat clock counts only while a shell window is on the screen; while none is, nothing runs. |
| Dim | Home only. Still (no key or pointer, fewer than 2 commits a second) for `dimAfterMin` (5): eased over `dimOverMin` (10) to `dimTo` (0.85), in steps of 0.01 taken at beats. A key or pointer movement restores it at once; a Home that starts moving (wallpaper motion) or appears again starts bright. The system layer is not dimmed: what it holds for long is a camera pop-up or a prompt, and a still picture cannot be told from a video there. |
| Saver | While the shell's saver is up (`state.saver`) the care stands aside: the saver is mostly black, moves, and dims over twenty minutes itself (`screensaver.dimOverMin` 20 to `dimTo` 0.4; the clock at `clockLevel` 0.4). |
| Floor | `dimTo` is held at 0.8 or above: the palette's second ink keeps 4.9 : 1 on its sheet at 0.8 (4.4 at 0.75), dark and light (`tests/test_oledcare.py`). |

The dim is a `u_dim` multiplier on the colour of the shell window's texture in the GL renderer (alpha untouched; premultiplied buffers), the software renderer does the same. A shifted Home shows the black behind it in a strip of at most 3 px at two edges, and a shifted or dimmed Home is composed instead of going to the plane whole (`Plan::sole_fullscreen` needs the full rectangle at full brightness).

Tokens in `payload/theme/theme.json`: `oledcare.{enabled, orbitPx 0-4, stepSec 10-600, dimAfterMin 1-120, dimOverMin 1-240, dimTo 0.8-1}`. `ctl status` shows `oledcare` (`enabled`, `offset`, `dim`, `beats`).

## Checks run

`cargo test` (156 unit tests, 10 of them for `oledcare`; `tests/oledcare.rs` 5 on the scene and the software renderer), `cargo check --features ui`, clippy and rustfmt for the new files, `python3 -m unittest tests.test_oledcare`, the Screensaver tab's two hosted Settings tests (`tests/test_settings.py`, the second now also turns OLED care on from its row), `tests/qmlhost/compile_all.py` 0 failed, `test_cutover`, `test_gate`, `test_tokens`.

## TV check (not run: no TV access)

1. Switch on with short timings: `lumatv theme set '{"oledcare":{"enabled":true,"stepSec":10,"dimAfterMin":1,"dimOverMin":2}}'`. `lumatv-compositor ctl status` within 2 s: `oledcare.enabled` true.
2. Home, hands off: `offset` changes by one pixel on one axis about every 12 s (`beats` +1), never two pixels, never past 3; a `ctl capture` shows the status bar and dock moved with the picture, black strip of 3 px at most at the top and left edges at the corner position. After 1 min `dim[0]` falls from 1 to 0.85 in 0.01 steps over 2 min; a remote key sets it back to 1.
3. Over a playing video (an HDMI input, Netflix): press Volume; the bar's offset follows `offset`; two captures 30 s apart show the video at the same place and brightness (compare a block of the picture outside the bar).
4. Idle cost: `ctl wake reset`, wait 60 s on a still Home with `stepSec` 30, `ctl wake`: the tick rate stays near 10 a second (a beat adds one drawn frame, not a minute of 60 Hz ticks). Same with the switch off: the numbers match.
5. Frame deadlines: Home with wallpaper motion on, care on against off, `tv-gate.sh frames`: no skipped refresh, `renderUsMax` within 1 ms of the off run (a shifted Home is composed, not scanned out whole).
6. With the saver up (`ctl shell` state `saver` true) the offset and `dim` stand still.
7. Switch off in Settings: the offset returns to (0,0) in one step during the toggle's own frame.

Pass: 2 to 7 hold; then `oledcare.enabled` ships on.

## TV round 2026-10-28 (round 14)

`a1555df`, `oledcare` on with `stepSec` 10, `dimAfterMin` 1, `dimOverMin` 2 (the theme keeps those three keys, `enabled` off). Home's `offset` moved one pixel on one axis per beat
((0,0), (-1,0), (-2,0), (-3,0), (-3,-1), (-3,-2), then round to (3,3), (1,3)), never past 3; Home and its chrome moved together; switching off returned (0,0). `dim` stayed 1.0 for 2 minutes of no key:
Home runs the moving wallpaper, and a Home that moves is not still by the care's own rule (`MOVING_PER_S`): the dim was not shown. Not run: steps 3 to 7. Result: orbit PASS, dim not exercised; off, go-live with Main's word.
