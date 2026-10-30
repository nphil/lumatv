# Input previews: what is on each input, in the picker

## Resume here

On the set since 2026-10-29 (Round21): the helper's routes, the capture and the picker with Live TV's picture ("Live TV · just now", "1 min ago") work. Switch: Settings > Inputs & devices > Show input previews (`inputs.previews` of theme.json, on by default); its row was not drawn until the scene-budget fix (`settings-scene-budget-result.md`).

Probe, Live TV in front, 480 by 270: `VIDEO` takes PNG (median 237 ms, 200 KB), JPG (187 ms, 35 KB), BMP and RGB (about 170 ms); `DISPLAY` takes them too (it has the UI in it). 320 by 180 to 960 by 540 take 180 to 337 ms. 40 captures 500 ms apart: the call's median 199 ms, p90 222, max 233; the capture service 7.7 % of one core (39 ms of CPU a capture). The helper's three pictures were 24 KB each, the folder 0700, the files 0600, `/tmp/lumatv-previews` empty. HDMI 2 with no source: `VIDEO` is refused in every format (`REFUSED_PERMISSION_DENIED`, errorCode 12) and `DISPLAY` gives the TV's own screen, so the helper's `no_picture` is right and the method stays `VIDEO`. At 2 a second the cost is the 8 % of a core and a 200 ms call: `LIVE_MS` stays 500.

Next: the TV check below over an HDMI input that has a picture (steps 2 to 9) and over a protected one (step 6).

## What it does

- The input picker's tile for the input in front is the **live picture** of that input while the picker is up (a new picture every 500 ms); every other tile that has been seen before is its **last picture**, a little dimmed, with its age in the state line ("Connected · 7 min ago", "In front · 2 min ago", "yesterday"). The live tile says "In front · Live".
- A tile with no picture (nothing connected, never seen, the pictures came back black) keeps its icon, in the same shape.
- The tiles are 16:10 (332 by 209 at 100 % text) so a 16:9 picture loses a twentieth of its width at each side; the sheet is as tall as its tiles need (321 instead of the mock's 404) and still rests 64 from the bottom. The words are on the picture over a scrim baked into its pixels; the port's name and the sound mark sit on black chips. The focused picture lifts 3 % about its centre with a lit ring and halo, as a camera's tile does.
- The last pictures are taken without the picker: after an input comes to the front (3 s later; if the picture is still black, again 6 s later, up to three times), then every 30 s while it stays and the shell is quiet (no panel, Settings, saver or camera view up), as it goes from the front (Home, an app or another input; a try, for the video plane may already be gone), and once more as the screen goes off. So one is there when the person leaves, or the TV goes off.
- With the switch off nothing is taken, nothing is shown, and everything kept is deleted.

## Decisions and why

- **The source is LG's capture service through the helper**, method `VIDEO` (the video plane alone: LumaTV's own sheet is never in it; `DISPLAY` is the blend with the UI), PNG at 480 by 270 (`luna-send ... com.webos.service.capture/executeOneShot`, the call the gate makes by hand). The shell cannot make the call: the compositor's luna allow-list for the shell does not include the capture service, and widening it would put the video picture behind every screen of the shell. The compositor's own screenshots never read the video plane (`capture-result.md`) and the app switcher never samples an input (`app-switcher-result.md`); this feature is the one door to an input's picture, in one process, behind the owner's switch.
- **No hardware window for the live tile.** The mixer hole (`layer-deep-result.md` § 8) shows the full-size video plane through a rectangle: a tile would show the part of the picture that lies behind it, not the picture shrunk into it, and the hole cuts every pixel of the UI inside it, so no label or scrim can sit on it. A scaled main video window for an HDMI input is not on offer (`vwin` hands out spare windows for LumaTV's own streams, never an app's or an input's). A capture, shrunk once into the tile, is the whole picture with the words on it and costs no register write. If the probes show the capture is too dear, the live rate comes down; the window would be worth building only with a scaled video window under it.
- **Nothing protected is read and nothing black is kept.** LG's service refuses or blanks what is protected. A capture with fewer than 0.5 % of its pixels brighter than 16 in 255, or that the service answers with `REFUSED...`, is no picture: the one kept before stays (the tile shows it with its age), and with none the tile keeps its icon. Nothing here touches HDCP, a DRM path or the video tap. (A film that is black for a moment keeps its older picture: the price of the rule.)
- **Pictures stay on the TV.** One JPEG of at most 480 by 270 per input in `/var/lib/lumatv/previews/` (folder 0700, files 0600, overwritten by the next, forgotten after 30 days); the raw capture is a file in `/tmp/lumatv-previews/` deleted as soon as it is read. Nothing is uploaded. Per TV, not per profile (`inputs` is not in `profile-scope.json`). The helper reads the switch itself (theme.json) before it captures or serves, so a stale shell cannot take a picture the owner turned off; the first request that finds it off deletes what was kept (a hand edit of theme.json leaves nothing behind), and the shell, told so (`previews_disabled`, or `enabled: false` in the index), forgets its pictures and shows icons.
- **Cheap.** One capture at a time, 250 ms apart at the least; after three failures in a row none for a minute (doubling to ten); live pictures stay in the helper's memory and go to the flash at most every 30 s. The shell decodes a picture only when the picker is still: not while the sheet slides in or for 450 ms after a key (a decode of some milliseconds inside the focus's lift is a dropped frame); it waits and keeps the newest.
- **Last pictures when the shell is quiet.** The timers wait while a panel, Settings, the saver, the display lab or the camera view is up and try again 5 s later.
- **The row keeps one height** whether a picture has come yet or not (the height is the larger of the picture shape and the icon layout), so a picture arriving moves nothing but that tile.

## Interfaces

| Where | What |
|---|---|
| Helper | `GET /v1/previews` (`{previews: {enabled, items: {key: {at, w, h, bytes}}, size, method, stats}}`; `stats` has taken, kept, blank, failed, msAvg, msMax, last), `GET /v1/previews/{key}` (the JPEG, `X-Luma-At`), `POST /v1/previews/capture` `{key, why}` (`why`: live, open, arrive, periodic, leaving, standby), `DELETE /v1/previews[/{key}]`. Keys: `livetv`, `hdmi1` to `hdmi9`. Failures: 400 `bad_request`, 403 `previews_disabled`, 409 `screen_off` or `no_picture`, 429 `busy` or `too_soon`, 502 `capture_failed`, 503 `backing_off`, 504 `timeout` |
| `helper.json` | `previews: {method: "VIDEO" or "DISPLAY", width: 480, height: 270, quality: 72, deadlineMs: 4000, minGapMs: 250, keepDays: 30}` |
| theme.json | `inputs.previews` (bool, on unless false) |
| Shell | `host/previews.rs` (the model and its timers), `panels/shot_tile.rs` (the picture dressed for a tile: cut, scrim, rounded; the lifted tile), `panels/picker_surface.rs`, Settings > Inputs & devices (a switch), tags `0x5500_0000..` |
| Harness | `lumatv-ui --host` state has `panels.previews` (what is held, what is asked); the picker's geometry has `picture`, `live`, `age`, `rev` per tile |

## TV check (needs Main; step 0 is done for Live TV)

0. Done over Live TV and over HDMI 2 with no source (Resume here). Open: `sh tv-previews.sh capture hdmi-plain` over an HDMI input with a picture, and `capture hdmi-hdcp` over a protected one: `VIDEO` should give the input's picture with no LumaTV UI in it (`DISPLAY` has the UI), and a protected picture an error text or black, which is never kept.
1. With the helper from this commit installed: `curl -s 127.0.0.1:18731/v1/previews` says `enabled: true`, `items: {}`; `sh tv-previews.sh helper` over an input takes three pictures (`ok`, `at`, a JPEG under 60 KB), `ls -l /var/lib/lumatv/previews` shows 0700 and 0600 and `ls /tmp/lumatv-previews` is empty.
2. Over HDMI 2 press the Input key: the picker's HDMI 2 tile is its picture within a second and says "In front · Live"; the picture follows the film (two a second); the other tiles show icons until each input has been in front.
3. Switch to HDMI 3 with OK, watch 10 s, open the picker again: HDMI 3 is live, HDMI 2 is its last picture with an age.
4. With the picker closed and a film playing, wait 70 s: `GET /v1/previews` `stats.taken` rose by about 3 (arrival, then every 30 s); `lumatv-compositor ctl status` `stats.uiDrawn` did not move (a capture draws no UI frame); no judder in the film at a capture (`ctl refresh motion` before and after).
5. Live cost: with the picker open over a film for 20 s, `top -H` for `captureservice`, the helper and `lumatv-ui`; the film behind the sheet shows no stutter; the first key after opening moves the focus smoothly (the decode waits for it).
6. Protected: an Apple TV title that is protected in front: the helper answers `409 no_picture`, the tile keeps its last picture with its age (or its icon), and the file on the flash is not black.
7. Standby: power off with an input in front: `GET /v1/previews` item `at` is within a second of the screen going off, or unchanged when the picture was already gone.
8. Settings > Inputs & devices > Show input previews off: the tiles are icons in the mock's shape at the next opening, `/var/lib/lumatv/previews` holds no `.jpg`, `stats.taken` stops, the helper log has the `DELETE`. On again: pictures return from the next opening.
9. The video gate with previews on (`python3 compositor/tools/tv_gate.py --only video --video-apps youtube,netflix,plex`): its rules hold (nothing over the video composes between captures).

Pass marks: 2 to 5 and 8 on their words; 6 and 7 as written; 9 as the gate says.

## Not done

- The hardware window (above).
- Pictures of apps (the switcher's business, which never reads the video plane) and of inputs other than Live TV and HDMI 1 to 9.
- A profile's own pictures, an expiry shorter than 30 days, a "Clear pictures" row (turning the switch off deletes them).

## Tests

- Helper (`app/src/previews.rs`, 19): key allow-list, the call and the size asked, a small JPEG back, the raw file deleted, black and refused pictures keep the last one, the switch, 0700/0600 and one file per input, restart and expiry, a bad file on the flash, live pictures to the flash at most every 30 s, one at a time and the gap, backing off, screen off, shrinking, bad requests, deleting, the stats, `luna-send`'s answer and the camera's deadline; `tests/ipc.rs` counts the routes.
- Shell (`host/previews.rs`, 22; `panels/shot_tile.rs`, 4; `panels/picker_surface.rs`, 11 more; `settings/pages/inputs.rs`, 1): the model's asks, timers and holds, the dressed picture and its contrast, the picker's shape with and without pictures, in place repaints, layers, the lint at both text sizes in both looks, the focus lift; `tests/test_rust_panels.py` runs the picker on the host against the helper stood in (`picker_scene`) and writes renders (`docs/mocks/host-rust/panels/picker-previews-*`).

## Changelog line (for Main)

The input picker shows what is on each input: a live picture for the input in front and the last picture, with its age, for the others (LG's capture service through the helper, video plane only, small JPEGs kept on the TV, nothing black or protected kept); Settings > Inputs & devices > Show input previews (on) turns it off and deletes them; TV check in `docs/research/input-preview-result.md`.
