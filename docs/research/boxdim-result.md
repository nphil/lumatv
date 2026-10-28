# Box dim: bright still boxes of an app dimmed, the rest not

Built and host-tested; off until its TV check passes. Switch: `theme.oledcare.boxDim` (Settings › System › OLED care › Dim bright still areas, or `lumatv theme set '{"oledcare":{"boxDim":true}}'`), read by the compositor within 2 s. Design and reasons: `boxdim-design.md`.

## What it does

A grid of 64 x 36 cells (30 px) over the screen. A cell's content is still while no commit's damage touches it (nothing is read for that); its brightness is one sample every 4 s on a screen still that long, a 64 x 36 GPU pass over the app's pictures with the video's holes black, 9 KB read back. A cell still for `boxDimAfterMin` (3) whose two last samples agree gets a target gain by brightness: 1 up to `boxDimMinLuma` (0.35), down to `boxDimTo` (0.8) at 0.65 and above. Targets are dilated one cell and blurred, so a bright box is at the floor across its whole area (within 2.5 % over its last 15 px) and the feather, about 90 px wide, lies on the dark around it; the map is an 8-bit texture sampled bilinearly, so no gain step between two pixels exceeds a code value on white. Gains fall over `boxDimRampSec` (60) in steps every 2 s and come back within 150 ms, stepped every frame.

| | Rule |
|---|---|
| Applies to | Items of app windows (cards, overlays, popups, the keyboard). Never the shell's two windows, the wallpaper plane, the cursor, the switcher's backdrop, a window in transition, or anything in a hole (nothing is drawn there). |
| Restarts everything | A key or pointer move; damage of 16 cells (120 x 120 px) or more in one frame; a window appearing, leaving, restacking or moving by more than 4 px; a hole changing; a transition; the saver; a capture; the switch off. |
| Restarts its own cells | Damage smaller than that (a clock, a caret, a spinner). Those cells never dim by themselves; next to still bright cells they keep the region's gain through the dilation, so nothing pulses. |
| Snaps to 1 | The saver, a capture (`screencap shot`/`record` write the picture as the app drew it), the switch off, no app on the screen. Everything else ramps out over 150 ms. |
| Frame path | No map: unchanged (the plain program, no second texture, direct scan-out through `Plan::sole_fullscreen`). Map engaged: app items go through a second program with one more texel fetch; a step damages the changed cells plus one of margin and is a quiet frame when nothing else is pending. |

`appDim` multiplies with it. `ctl capture PATH` reads the page as composed, dim included.

Tokens: `oledcare.{boxDim, boxDimAfterMin 1-120, boxDimRampSec 5-600, boxDimTo 0.75-1, boxDimMinLuma 0.1-0.9}`. `ctl status` `oledcare.boxDim`: `on`, `engaged`, `cells` (below 1), `lowest`, `stillSec`, `samples`, `steps`, `sampleUsMax`, `restart` (why the grid last restarted). Journal: `oledcare: boxDim on: N cells, lowest 0.xx` and `oledcare: boxDim off (key|change|capture|saver|off|...)`. The first five samples log `boxdim: sample N took X us`.

## Host numbers

Mesa llvmpipe, 1920 x 1080, one opaque full-screen item: a full composed frame with the map 6.75 ms against 7.0 ms without (the one texel fetch is noise on a CPU rasteriser); the sample pass 173 ms there (518 000 bilinear taps on the CPU; on the set's GPU it is a fraction of one composed frame, and `sampleUsMax` says what). The grid's own work per step: under 50 µs on the host for all 2304 cells.

## Checks run

`cargo test -p lumatv-compositor` (10 unit tests in `boxdim`, `tests/boxdim.rs` 3 on the scene and the software renderer, `tests/boxdim_gl.rs` 2 on Mesa: the sample pass's grid and the feathered picture), `cargo check --features ui`, clippy and rustfmt, `python3 -m unittest tests.test_oledcare`, `tests/qmlhost/compile_all.py`.

## TV check (not run: no TV access)

Quick timings for the check: `lumatv theme set '{"oledcare":{"boxDim":true,"boxDimAfterMin":1,"boxDimRampSec":20}}'`. `lumatv-compositor ctl status` within 2 s: `oledcare.boxDim.on` true. `ctl note boxdim-start` marks the journal.

1. Netflix (or any app with bright still boxes) on a browse page, hands off. `ctl status` every 10 s: `boxDim.samples` grows by about one per 4 s, `stillSec` grows. After 60 s `engaged` true and `cells` > 0; `lowest` falls to 0.8 over the next 20 s; the journal has `oledcare: boxDim on: N cells`. Pass: the boxes look like the same UI a little less bright, with no visible rectangle, band or edge around them at any point of the ramp (watch it, and photograph the screen at the end).
2. Pictures: at 90 s `ctl capture /tmp/dim.png` (the composed page, dim in it), then `ctl screencap shot` (the picture as the app drew it; the map snaps off for it). Inside a bright box the capture is 0.80 x the shot (±2 codes); on the dark surround beyond 100 px of any box the two match (±1). After the shot `engaged` is false with `restart` `capture`, and it engages again after the wait.
3. A key: with it engaged, press an arrow. `ctl status` within 300 ms: `engaged` false, journal `oledcare: boxDim off (key)`; the boxes brighten over about 150 ms, no flash or step. `stats` `direct` scan-out frames grow again once the app is still (the app is back on the plane).
4. A small change: an app with a clock or spinner (Live TV's banner, a media player's progress bar) engaged; the clock ticks and `engaged` stays true, `restart` unchanged, the region around the clock does not flicker. A popup or a page change (more than 120 x 120 px) sets `engaged` false with `restart` `change`.
5. Video: full-screen playback for 2 min: `engaged` stays false (the hole's cells read black). Playback in a small hole with still UI around (YouTube's player page): the UI may dim; two captures 30 s apart show the hole at the same place and black in both, and the video on the panel at the same brightness (compare a block of it by eye or photo).
6. Cost: `ctl status` `sampleUsMax` under 2000 (the first five samples' `boxdim: sample N took X us` lines agree); `tv-gate.sh frames` with the switch on against off while moving through the app's menus: no skipped refresh, `renderUsMax` within 1 ms.
7. Idle: `ctl wake reset`, 60 s hands off with the map engaged and the ramp over, `ctl wake`: the tick rate stays near 10 a second (a step is a quiet frame, one every 2 s at most, none once the ramp is done).
8. The saver up (`ctl shell` state `saver` true): `engaged` false, `restart` `saver`. Switch off in Settings: `engaged` false, journal `oledcare: boxDim off (off)`, the app direct again.

Pass: 1 to 8 hold; then `boxDim` ships on with the shipped timings.
