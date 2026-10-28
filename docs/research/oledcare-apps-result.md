# OLED care for apps: a still app dims and drifts

Built and host-tested; both switches off until their TV check passes. Switches: `theme.oledcare.appDim` and `theme.oledcare.appShift`, each its own and independent of `oledcare.enabled` (LumaTV's own screens). Settings › System › OLED care, under LG's rows, group "LumaTV panel care" with five toggles in this order: LumaTV screens, Dim still apps, Shift apps, Dim bright still areas, Protect panel refresh. The last two write `boxDim` and `refreshGuard` for the modules that read them. Mocks: `docs/mocks/oled-care-apps/` (dark and light, 100 % and 200 % text, the help sheet). The compositor reads `theme/theme.json` at the idle tick, at most every 2 s.

## What it does

The card in front (`WindowType::Card`, not Home's stub, not a covering overlay such as LG's Settings) is *still* when none of these happened for `appDimAfterMin` (3): a key or pointer movement; a commit that redraws at least 1/64 of the screen; more than 2 commits a second; a video hole on its surface or its group; a transition. Then, at beats of its own (`stepSec`, shared with the orbit):

| | Rule |
|---|---|
| `appDim` | Eases from 1 down to `appDimTo` (0.75, never below 0.7) over `dimOverMin` (10), in steps of 0.01. `u_dim` on the card's items, the same path the shell's Home dim uses. |
| `appShift` | The card's whole graphics window walks the shell's ring (`orbitPx` 3, one pixel on one axis a beat), the black behind it shows in a strip of at most 3 px. |
| Back | Any key or pointer movement (`Care::input`), a real redraw, a hole appearing, a transition, a new front window, the saver, a switch turned off: brightness 1 and offset (0, 0) in the very plan that sees it, so the restore is the next frame (at most one idle tick, 100 ms, after a key). |
| Hole | Never shifted or dimmed while a hole is present. The shift is not held at an offset: the hole moves with the graphics but the video plane does not (its rectangle comes from the card at the origin), so the card goes to the origin the plan the hole appears in. |
| Direct plane | Idle and still-waiting, the card is a sole full-screen item at 1.0 and goes to the plane whole. Engaged, it is composed (`Plan::sole_fullscreen` needs the full rectangle at full brightness), only while the screen is static; a beat is a frame of its own that the output may call quiet (`Output::quiet_frame`). |

A small redraw (a caret, a clock, a progress bar) does not restore a dimmed app. A video drawn into the app's own surface (no hole) is commits: a redraw of 1/64 of the screen or more than 2 commits a second is movement. The compositor has no other playback report from an app; a playback state that an app reports through LG's services is not read.

`ctl status` `oledcare` gains `appDim` (`enabled`, `level`, `to`, `afterMin`), `appShift` (`enabled`, `offset`) and `app` (`window`, `stillSec`, `beats`, `engaged`). The journal says `oledcare: app window N still for S s: engaged (brightness L, offset (x, y))` and `oledcare: app window N put back (reason): brightness 1, offset (0, 0)`, once each per engagement.

Tokens in `payload/theme/theme.json`: `oledcare.{appDim false, appShift false, appDimAfterMin 3 (1-120), appDimTo 0.75 (0.7-1)}`, plus `boxDim` and `refreshGuard` (false) for the other two rows.

## Checks run

`cargo test` (the oledcare unit tests, 8 new; `tests/oledcare.rs` 5 new on the scene and the software renderer), `cargo check --features ui`, clippy, `python3 -m unittest tests.test_oledcare`, the Settings tests that touch the move (`tests.test_settings`, `tests.test_settings_coverage`), `tests/qmlhost/compile_all.py`.

## TV check (not run: no TV access)

Use an app whose still screens have no video: Netflix's profile picker or a Plex library page, not a page with an auto-playing preview.

1. `lumatv theme set '{"oledcare":{"appDim":true,"appShift":true,"appDimAfterMin":1,"dimOverMin":1,"stepSec":10}}'`. Within 2 s `lumatv-compositor ctl status` shows `oledcare.appDim.enabled` and `appShift.enabled` true.
2. Open the app, press nothing. `ctl status` every 10 s: `app.stillSec` counts up; `app.engaged` false until 60 s, `appDim.level` 1.0 and `appShift.offset` [0,0] until then; after it `level` falls to 0.75 in 0.01 steps within about a minute and `offset` changes one pixel on one axis per beat, never past 3. The journal has one `engaged` line. `ctl capture` before and after: the picture is darker and moved, a black strip of at most 3 px at the top or left edge.
3. Press a remote key: within 0.3 s `ctl status` shows `level` 1.0, `offset` [0,0], `app.engaged` false; the journal has `put back (key or pointer)`. Move the Magic Remote pointer instead: the same.
4. Engage again, then start a video (Netflix title, a Plex movie). `ctl windows` shows the video hole; `level` 1.0 and `offset` [0,0] from the moment it plays, and for 5 minutes of playback they never leave it (`app.engaged` false, no new `engaged` line). Pause: still a hole, so nothing engages while the video is up.
5. Engage, then change the page without a key (an app timer, a slideshow): back at the first redraw of 1/64 of the screen. A progress bar or a clock ticking: does not restore it.
6. Idle cost: engaged and still, `ctl wake reset`, wait 60 s, `ctl wake`: the tick stays near 10 a second, same as with the switches off. Before engaging (still < 60 s) the card goes to the plane whole: `ctl status` `buffers` shows the direct path, `renderUsMax` unchanged against the switches off. During interaction (keys in the app) the numbers match the switches off: nothing is composed that was not.
7. `tv-gate.sh frames` over the app with both switches on, engaged then keyed: no skipped refresh in the frames after the key.
8. With the saver up (`ctl shell` `saver` true) `app.engaged` false and level 1.0. Both switches off in Settings: level 1.0 and offset [0,0] within 2 s, engaged or not.
9. Settings › System › OLED care: the group under LG's rows has the five toggles in the order above; LumaTV screens is gone from Home & look › Screensaver; each toggle writes its key (`lumatv theme get`, the `oledcare` block).

Pass: 2 to 9 hold; then `appDim` and `appShift` ship on (the shipped values stay false until Main's word).
