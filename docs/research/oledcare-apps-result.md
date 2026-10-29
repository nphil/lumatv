# OLED care for apps: a still app dims and drifts

Built and host-tested; both switches off until their TV check passes. Switches: `theme.oledcare.appDim` and `theme.oledcare.appShift`, each its own and independent of `oledcare.enabled` (LumaTV's own screens). Settings › System › OLED care, under LG's rows, group "LumaTV panel care" with five toggles in this order: LumaTV screens, Dim still apps, Shift apps, Dim bright still areas, Protect panel refresh. The last two write `boxDim` and `refreshGuard` for the modules that read them. Mocks: `docs/mocks/oled-care-apps/` (dark and light, 100 % and 200 % text, the help sheet). The compositor reads `theme/theme.json` at the idle tick, at most every 2 s.

## What it does

The card in front (`WindowType::Card`, not Home's stub, not a covering overlay such as LG's Settings) is *still* when none of these happened for `appDimAfterMin` (3): a key or pointer movement; a change of its picture over 1/64 of the screen; a video hole on its surface or its group; a transition. What is a change is told by the box dim's grid (`boxdim-result.md`): a commit's damage marks the cells that may have changed and a GPU sample of the picture says whether 36 or more did, so an app that commits the same frame again is still however often it does (LG's browser and Plex every 4 to 5 s, round 16). The grid runs while `appDim`, `appShift` or `boxDim` is on. Then, at beats of its own (`stepSec`, shared with the orbit):

| | Rule |
|---|---|
| `appDim` | Eases from 1 down to `appDimTo` (0.75, never below 0.7) over `dimOverMin` (10), in steps of 0.01. `u_dim` on the card's items, the same path the shell's Home dim uses. |
| `appShift` | The card's whole graphics window walks the shell's ring (`orbitPx` 3, one pixel on one axis a beat), the black behind it shows in a strip of at most 3 px. |
| Back | Any key or pointer movement (`Care::input`), a hole appearing, a transition, a new front window, the saver, a switch turned off: brightness 1 and offset (0, 0) in the very plan that sees it, so the restore is the next frame (at most one idle tick, 100 ms, after a key). A change of its picture: in the plan after the sample that sees it, which an engaged card's damage gets right after the frame that drew it (the first commit after a quiet stretch; the next ones in a run 250 ms apart), so a new picture shows dimmed for a frame. |
| Hole | Never shifted or dimmed while a hole is present. The shift is not held at an offset: the hole moves with the graphics but the video plane does not (its rectangle comes from the card at the origin), so the card goes to the origin the plan the hole appears in. |
| Direct plane | Idle and still-waiting, the card is a sole full-screen item at 1.0 and goes to the plane whole. Engaged, it is composed (`Plan::sole_fullscreen` needs the full rectangle at full brightness), only while the screen is static; a beat is a frame of its own that the output may call quiet (`Output::quiet_frame`). |

A small change (a caret, a clock, a progress bar) does not restore a dimmed app. A video drawn into the app's own surface (no hole) is a picture that changes at every sample: never still. The sample is taken where the window places the card, the shift taken away and no dim in it, so the care's own dim and drift are never a change. The compositor has no other playback report from an app; a playback state that an app reports through LG's services is not read.

`ctl status` `oledcare` gains `appDim` (`enabled`, `level`, `to`, `afterMin`), `appShift` (`enabled`, `offset`) and `app` (`window`, `stillSec`, `beats`, `engaged`). The journal says `oledcare: app window N still for S s: engaged (brightness L, offset (x, y))` and `oledcare: app window N put back (reason): brightness 1, offset (0, 0)`, once each per engagement.

Tokens in `payload/theme/theme.json`: `oledcare.{appDim false, appShift false, appDimAfterMin 3 (1-120), appDimTo 0.75 (0.7-1)}`, plus `boxDim` and `refreshGuard` (false) for the other two rows.

## Checks run

`cargo test` (the oledcare unit tests; `tests/oledcare.rs` on the scene and the software renderer, among them the same picture committed again and a caret leaving an engaged app, a new picture putting it back the frame after and within 250 ms inside a run of commits; `tests/boxdim.rs`, the same picture committed from a swap chain every 4 to 5 s for 2.5 minutes engaging the app features), `cargo check --features ui`, clippy, `python3 -m unittest tests.test_oledcare`, the Settings tests that touch the move (`tests.test_settings`, `tests.test_settings_coverage`), `tests/qmlhost/compile_all.py`.

## TV check (not run: no TV access)

Use an app whose still screens have no video: Netflix's profile picker, a Plex library page, LG's browser on a static page (round 16's case: these commit the same frame again every 4 to 5 s), not a page with an auto-playing preview.

1. `lumatv theme set '{"oledcare":{"appDim":true,"appShift":true,"appDimAfterMin":1,"dimOverMin":1,"stepSec":10}}'`. Within 2 s `lumatv-compositor ctl status` shows `oledcare.appDim.enabled` and `appShift.enabled` true.
2. Open the app, press nothing. `ctl status` every 10 s: `app.stillSec` counts up and never falls back while hands are off (round 16: it fell to 0 every 4 to 5 s); `oledcare.boxDim.samples` grows by about one per burst of commits. `app.engaged` false until 60 s, `appDim.level` 1.0 and `appShift.offset` [0,0] until then; after it `level` falls to 0.75 in 0.01 steps within about a minute and `offset` changes one pixel on one axis per beat, never past 3. The journal has one `engaged` line and no `put back` line while hands are off. `ctl capture` before and after: the picture is darker and moved, a black strip of at most 3 px at the top or left edge.
3. Press a remote key: within 0.3 s `ctl status` shows `level` 1.0, `offset` [0,0], `app.engaged` false; the journal has `put back (key or pointer)`. Move the Magic Remote pointer instead: the same.
4. Engage again, then start a video (Netflix title, a Plex movie). `ctl windows` shows the video hole; `level` 1.0 and `offset` [0,0] from the moment it plays, and for 5 minutes of playback they never leave it (`app.engaged` false, no new `engaged` line). Pause: still a hole, so nothing engages while the video is up.
5. Engage, then change the page without a key (an app timer, a slideshow, a new `target` sent to the browser): put back within 300 ms of the change (`put back (redrawn)`, `oledcare.boxDim.checks` +1), the new page dimmed for one frame at most. A progress bar or a clock ticking: does not restore it.
6. Idle cost: engaged and still, `ctl wake reset`, wait 60 s, `ctl wake`: the tick stays near 10 a second, same as with the switches off. Before engaging (still < 60 s) the card goes to the plane whole: `ctl status` `buffers` shows the direct path, `renderUsMax` unchanged against the switches off. During interaction (keys in the app) the numbers match the switches off: nothing is composed that was not.
7. `tv-gate.sh frames` over the app with both switches on, engaged then keyed: no skipped refresh in the frames after the key.
8. With the saver up (`ctl shell` `saver` true) `app.engaged` false and level 1.0. Both switches off in Settings: level 1.0 and offset [0,0] within 2 s, engaged or not.
9. Settings › System › OLED care: the group under LG's rows has the five toggles in the order above; LumaTV screens is gone from Home & look › Screensaver; each toggle writes its key (`lumatv theme get`, the `oledcare` block).

Pass: 2 to 9 hold; then `appDim` and `appShift` ship on (the shipped values stay false until Main's word).

## TV round 2026-10-28 (round 16)

`64e015d`, `appDim` and `appShift` on with `appDimAfterMin` 1: FAIL, never engaged. `app.stillSec` fell back to 0 every 4 to 5 s on Plex's library, LG's browser start page and a static `data:` page: those clients commit the same frame again in bursts of 1 to 12 every 4 to 5 s with the whole surface damaged, and both the redraw rule (damage over 1/64 of the screen) and the commit-rate rule (over 2 a second) took that for movement. Fixed since: a change is a change of the picture, as the box dim's samples see it; the commit-rate rule is gone. The pointer put-back held (0.03 s). Steps 2 to 8 are to be run again.
