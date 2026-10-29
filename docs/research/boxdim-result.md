# Box dim: bright still boxes of an app dimmed, the rest not

Built and host-tested; off until its TV check passes. Switch: `theme.oledcare.boxDim` (Settings › System › OLED care › Dim bright still areas, or `lumatv theme set '{"oledcare":{"boxDim":true}}'`), read by the compositor within 2 s. Design and reasons: `boxdim-design.md`. The same grid says when an app is still for `appDim` and `appShift` (`oledcare-apps-result.md`).

## What it does

A grid of 64 x 36 cells (30 px) over the screen. A commit's damage marks the cells that may have changed; a sample of the app's pictures says whether they did: two 15-tap GPU passes into 64 x 36, three numbers a cell (its mean luma, that of its left and of its lower part), the video's holes black, read back once the GPU has finished. A cell whose numbers did not move is still however often the app commits it (LG's browser and Plex commit the same frame every 4 to 5 s). A cell still for `boxDimAfterMin` (3) gets a target gain by brightness: 1 up to `boxDimMinLuma` (0.35), down to `boxDimTo` (0.8) at 0.65 and above. Targets are dilated one cell and blurred, so a bright box is at the floor across its whole area (within 2.5 % over its last 15 px) and the feather, about 90 px wide, lies on the dark around it; the map is an 8-bit texture sampled bilinearly, so no gain step between two pixels exceeds a code value on white. Gains fall over `boxDimRampSec` (60) in steps every 2 s and come back within 150 ms, stepped every frame.

| | Rule |
|---|---|
| Applies to | Items of app windows (cards, overlays, popups, the keyboard). Never the shell's two windows, the wallpaper plane, the cursor, the switcher's backdrop, a window in transition, or anything in a hole (nothing is drawn there). |
| Sampled | 4 s after the last sample when anything was damaged since, on a tick that drew nothing (8 s on any tick); a check right after the frame that drew damage on a dimmed cell or an engaged app (the first commit after a quiet stretch at once, then 250 ms apart); never within 2 s of a key; never on a screen that commits nothing. |
| Restarts everything | A key or pointer move; a sample that finds 16 cells (120 x 120 px) or more changed; a window appearing, leaving, restacking or moving by more than 4 px; a hole changing; a transition; the saver; a capture; the switch off. |
| Restarts its own cells | A sample that finds fewer changed (a clock, a caret, a spinner). Those cells never dim by themselves; next to still bright cells they keep the region's gain through the dilation, so nothing pulses. |
| Taken for change | Damage no sample has judged for 12 s, and every damaged cell when the output cannot sample (`blind`): nothing is dimmed that nobody looked at. |
| Snaps to 1 | The saver, a capture (`screencap shot`/`record` write the picture as the app drew it), the switch off, no app on the screen. Everything else ramps out over 150 ms. |
| Frame path | No map: unchanged (the plain program, no second texture, direct scan-out through `Plan::sole_fullscreen`). Map engaged: app items go through a second program with one more texel fetch; a step damages the changed cells plus one of margin and is a quiet frame when nothing else is pending. |

`appDim` multiplies with it. `ctl capture PATH` reads the page as composed, dim included.

Tokens: `oledcare.{boxDim, boxDimAfterMin 1-120, boxDimRampSec 5-600, boxDimTo 0.75-1, boxDimMinLuma 0.1-0.9}`. `ctl status` `oledcare.boxDim`: `on`, `tracking`, `engaged`, `cells` (below 1), `lowest`, `stillSec`, `unjudged`, `samples`, `checks`, `steps`, `warm`, `blind`, `sampleUsMax` (the loop's worst on a sample's submission or read), `warmUsMax` (its worst piece of the warm-up), `restart` (why the grid last restarted). Journal: `oledcare: boxDim on: N cells, lowest 0.xx` and `oledcare: boxDim off (key|change|capture|saver|off|...)`. The log has `boxdim: warm-up piece took X us` for each of the four pieces and `boxdim: sample N: begun in X us, read in Y us, Z cells changed` for the first five samples.

## Host numbers

Mesa llvmpipe, 1920 x 1080, one opaque full-screen item: a full composed frame with the map 3.7 ms against 4.0 ms without (the one texel fetch is noise on a CPU rasteriser). The warm-up's pieces: 23 ms and 2 ms (the two passes' compiles), 0.2 ms (the gained program), 279 ms (the targets and a first draw through each program, which llvmpipe runs on the CPU); a sample's submission 77 ms there, read once the fence signalled. On llvmpipe the GPU's work is the CPU's, so these are not the set's numbers: on the Mali the passes are a fraction of one composed frame, and `sampleUsMax` and `warmUsMax` say what the loop spends.

## Checks run

`cargo test -p lumatv-compositor` (16 unit tests in `boxdim`, among them a client committing the same picture in bursts every 4 s and one committing it every frame, both still, and one committing a moving picture every frame, never still; `tests/boxdim.rs` 4 on the scene and the software renderer, among them the same picture committed again from a swap chain every 4 to 5 s for 2.5 minutes, with the box dim and the app features both engaging and both coming back within 3 frames of a new picture; `tests/boxdim_gl.rs` 2 on Mesa: the warm-up in four pieces, the asynchronous sample's numbers, the feathered picture), `tests/oledcare.rs` (the app features' change from the samples: the same picture again and a caret leave an engaged app, a new picture puts it back the frame after, within 250 ms in a run of commits), `cargo check --features ui`, clippy and rustfmt, `python3 -m unittest tests.test_oledcare`, `tests/qmlhost/compile_all.py`.

## TV check (not run: no TV access)

Quick timings for the check: `lumatv theme set '{"oledcare":{"boxDim":true,"boxDimAfterMin":1,"boxDimRampSec":20}}'`. `lumatv-compositor ctl status` within 2 s: `oledcare.boxDim.on` and `tracking` true; within a few seconds `warm` true (the log has four `warm-up piece` lines). `ctl note boxdim-start` marks the journal.

1. Round 16's case first: LG's browser on the static `data:` page of two light boxes, then Plex's library, hands off. `ctl status` every 10 s: `stillSec` grows without falling back (it did every 4 to 5 s in round 16), `samples` grows by about one per burst of commits, `restart` stays `appeared`. After 60 s `engaged` true and `cells` > 0; `lowest` falls to 0.8 over the next 20 s; the journal has `oledcare: boxDim on: N cells`. Pass: the boxes look like the same UI a little less bright, with no visible rectangle, band or edge around them at any point of the ramp (watch it, and photograph the screen at the end), and the dim never pumps (`lowest` never rises while hands are off).
2. Netflix on a browse page, hands off: as 1 once its previews stop; while a preview plays in the UI, `engaged` stays false.
3. Pictures: at 90 s `ctl capture /tmp/dim.png` (the composed page, dim in it), then `ctl screencap shot` (the picture as the app drew it; the map snaps off for it). Inside a bright box the capture is 0.80 x the shot (±2 codes); on the dark surround beyond 100 px of any box the two match (±1). After the shot `engaged` is false with `restart` `capture`, and it engages again after the wait.
4. A key: with it engaged, press an arrow. `ctl status` within 300 ms: `engaged` false, journal `oledcare: boxDim off (key)`; the boxes brighten over about 150 ms, no flash or step. `stats` `direct` scan-out frames grow again once the app is still (the app is back on the plane).
5. A change without a key: with it engaged on the browser page, change the page from the host (`luna-send` a new `target` to the browser, or a page that swaps its content on a timer). `engaged` false within 300 ms of the change, `restart` `change`, `checks` +1; the new page shows dimmed for one frame at most (film it at 120 fps or watch for a flash). A clock or spinner (Live TV's banner, a media player's progress bar): `engaged` stays true, `restart` unchanged, the region around it does not flicker.
6. Video: full-screen playback for 2 min: `engaged` stays false (the hole's cells read black). Playback in a small hole with still UI around (YouTube's player page): the UI may dim; two captures 30 s apart show the hole at the same place and black in both, and the video on the panel at the same brightness (compare a block of it by eye or photo).
7. Cost: `warmUsMax` and `sampleUsMax` under 2000 (the log's `warm-up piece` and `sample N: begun in X us, read in Y us` lines agree); `tv-gate.sh frames` with the switch on against off while moving through the app's menus: no skipped refresh, `renderUsMax` within 1 ms, the pipeline's max within 1 ms (round 16: 6.2 and 7.6 ms on against 4.9 and 4.8 off, with the old synchronous read).
8. Idle: `ctl wake reset`, 60 s hands off with the map engaged and the ramp over, `ctl wake`: the tick rate stays near 10 a second (a step is a quiet frame, one every 2 s at most, none once the ramp is done).
9. The saver up (`ctl shell` state `saver` true): `engaged` false, `restart` `saver`. Switch off in Settings: `engaged` false, journal `oledcare: boxDim off (off)`, the app direct again.

Pass: 1 to 9 hold; then `boxDim` ships on with the shipped timings.

## TV round 2026-10-28 (round 16)

`64e015d`, `boxDim` on with `boxDimAfterMin` 1: FAIL, never engaged. `restart` was `change` all the time and `cells` 0 over 80 s on a static `data:` page in LG's browser: the browser commits the same frame again in bursts of 1 to 12 every 4 to 5 s, whole surface damaged, and damage was taken for change. The sample cost 56.0 ms the first time and 2.8 ms after (bar 2 ms). Fixed since: stillness is the samples' (the picture), not the damage's; the sample is two 15-tap passes made ahead in pieces and read asynchronously. Steps 1 to 9 are to be run again.
