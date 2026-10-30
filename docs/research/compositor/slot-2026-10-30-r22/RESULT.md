# Round 22 (TV slot 2026-10-30)

## Resume here

- Installed now: forge 4bc7a078 = 740048bf (third install, 05:15); b421f486 (frames fix) is being built and installed next, then both frames gates re-run (60 Hz and 120 Hz, `--only frames --frames-quick [--refresh 120]`; read `panel_refresh` per surface first). Weather redesign 740048bf is NOT installed yet: rebuild from HEAD, install, run docs/research/weather-redesign-result.md 'TV check (round 22)'.
- First install: forge 5afc7e41 (worktree `/tmp/lumatv-r22`, staged as `/data/home/tmp/lumatv-next`; build script `/tmp/r22-build/build22.sh`). LumaTV compositor persistent and live, helper running, Smart home listener on (6054, HA's LumaShow entry connected).
- Newer commits (21a560e3 video windows in UI px, e81474d6 sub window) are not installed yet; rebuild and reinstall when Frames120 pushes or at 05:00, then run CameraPopup's A2/A3 and D1, SubWindow's M2, and the frames gate at 60 and 120 Hz.
- Marquee 0.9.3: done (below). The receiver stays muted until the end of the night; unmute to 0.52 at the end.
- Changed on the set for the night, to put back at the end: picture backlight (filmMaker/default) 0, original 80 (the 68 seen at the start was the OLED care dim in progress); `/var/lib/lumatv/helper.json` gained `cameras.testEvents` true (original kept as `helper.json.before-r22`).
- Install procedure that worked: `cutover.sh disable` on the set, wait for "recover: Home on the panel" plus 60 s, the tv-install grant (the MCP call times out client-side at 30 s but the script keeps running; read `/tmp/lumatv-install-out/log.txt`), then `cutover.sh install` and `cutover.sh enable` over ssh (install refuses while the compositor is mounted).

## Results on 5afc7e41

| # | Check | Result |
|---|---|---|
| 1 | Install, hashes, persistent | pass: helper sha, compositor and ui md5 equal the build; flag persistent, live |
| 1 | Home: avatar, icons, weather | pass (`home1.png`) |
| 2 | Port 6054 and HA entry | pass: listening; entry 01M627CQ3HVVVFM62HQEYVFE57 `loaded`, helper log "the LumaShow integration connected"; the grant's script printed `lumatv: not found`, helper restarted by hand |
| 3a | Weather card on Home | 43,264 px per frame, 1 rect, ~75 % of frames (60 per s); the doc's "about 31 k px" is 40 % lower than measured |
| 3b | Weather card, Home covered (Browser in front) | pass: 5 frames in 9.6 s, all `ui 0 draws 0 layers`, 0 card draws |
| 3c | Weather view | pass: OK on the card opens it (`wx-now.png`), Down walks Now to Radar |
| 3d | Radar first frame | 1.4 s from the Down key to the first frame (page shows "Loading the radar" first); map and frame fade in by 2.3 s |
| 3e | Radar loop | pass: frames alternate FORECAST and RADAR (`wx-radar-sheet.png`), a step about every 0.56 s |
| 3f | Location | Atlanta: the helper's base.weather zip 30311 (33.723, -84.4702), unchanged |
| 3g | App icons on the dock | pass: distinct hue and glyph per app (`home1.png`) |
| 3h | Camera pop-up corner | pass: card top edge at y=32, right edge at x=1888 (32 px from the corner); the picture box was black (round 21's videooutputd units bug, fixed in 21a560e3, not installed) |
| 3i | Toast | bottom centre (`toast1.png`); only a test change in the blocker test moved, no placement code of toast or volume changed |
| 3j | Volume bar | not run: a volume key would change the receiver |
| - | Frames120 ring probe | `frames120-settings-ring.json` (notes `r22-settings-open`, `r22-settings-steps`) |

## Marquee 0.9.3 (installed by grant 667050c5, 03:31)

| Check | Result |
|---|---|
| Launcher icon and name | pass: new pink marquee-lights tile, label "Marquee" (`marquee-dock.png` shows the stale PLX tile, `marquee-focus.png` the new one). The dock keeps the old icon until the Rust shell restarts (`ctl shell restart`): same path, cached image |
| Bazarr import | the script's file is owned by root, the app (uid 5365) refused it ("not owned by this app") and could not delete it; `chown 5365:jailer /tmp/marquee-bazarr-import.json` and a restart imported it ("bazarr: link imported from file") |
| Settings > Subtitles preview | pass (`mq-subtitles.png`); Sample scene cycled Daylight to Night and back, the preview followed |
| Settings > Integrations > Bazarr | pass: address bazarr.nateshome.net, key as dots, Test connection "Connected · Bazarr 1.6.2. Sonarr 4.0.20.3014 and Radarr 6.4.4.10685 are linked" (`mq-bazarr-test.png`) |
| 30 opens and closes | pass: 30 of 30 reached the foreground, Home came back, each close logged `egl exit: unbound=1 surface ... destroyed=true context ... destroyed=true`, 0 crashes (`marquee-30-cycles.log`; the crash log grows 94 bytes per start, build ids only) |

Remote keys: `kb.py` presses did not reach Marquee under LumaTV's compositor (the Rust shell took them fine); `ctl key <evdev code>` did (UP 103, DOWN 108, LEFT 105, RIGHT 106, OK 28, BACK 412), so the Marquee walk used that.

## Round 2 on forge 2138070f (installed 04:14: 21a560e3 video windows in UI px, e81474d6 sub window, 9e100ea4 and 85ac5a6d frames)

| Check | Result |
|---|---|
| Frames gate 60 Hz `--only frames --frames-quick` | FAIL: dropped 0 everywhere but apps-grid 1; burst_over20 home-dpad 2, quick 8, picker 5, settings-scroll 10, apps-grid 1, home-widgets 0; toast burst_end 1999 ms (`gate/frames60q-verdict.txt`) |
| Frames gate 120 Hz | FAIL: dropped home-dpad 169, quick 33, picker 39, toast 38, settings-scroll 44, apps-grid 521, home-widgets (idle Home) 1205; burst_over20 567 to 1949 (`gate/frames120q-verdict.txt`) |
| A2 pop-up window | pass: claim camera-popup 1348,145 520x292, sink MAIN displayOutput 2696,290,1040,584 (the claim twice), fullScreen false |
| A3 picture in the card's box | pass: the camera's picture is in the box over Home (`popup-home-a.png`, `popup-home-b2.png`); POST to `video true` 0.98 s (250 ms polling) |
| A4 stream | first picture 151 to 293 ms; 26.4 to 26.8 pictures a second against 25 declared; no `placed again` line, `replaced` empty |
| D1 full view over Home | pass: claim cameras-full 0,0 1920x1080, sink displayOutput 0,0,3840,2160, fullScreen true, `replaced` empty, no `placed again`; first picture 152 ms; Back returns to Home with no claim (`full-home.png`) |
| Dock with the Marquee icon | `dock-marquee.png` |
| M2 sub window (SubWindow's script) | the sub sink was asked for once (`current [2:MainVideo], new [4:SubVideo]`) and videooutputd put the connect back on MAIN (`videoSinkType main_video`); SUB0 never connected; files in `docs/research/multiview/tv-2026-10-30/m2/` |

## Round 3 on forge 4bc7a078 (740048bf weather redesign, installed 05:15)

| Check | Result |
|---|---|
| A1 room on Home (clear night) | layers 121 of 256, groups 38 (`wx2-home2.png`) |
| A3 card cost | 1 rect of 103,680 to 121,856 px per frame, 44 draws; covered Home (Browser in front): 0 card draws, 10 frames in 10 s, all `ui 0 draws` |
| B1 view | OK on the card opens it in 0.21 s; layers 83, groups 38; ring 590 k px, 1 rect, GPU mean 3.2 ms max 3.7 ms (`wx2-now.png`, 10-day list) |
| B2 radar | full-screen map, FORECAST chip, loop steps every 0.5 s, layers 64; GPU mean 6.1 ms max 7.0 ms per frame at 60 Hz, full-screen damage (`wx2-radar.png`) |
| B3 details | layers 91 (`wx2-details.png`) |
| Not run | rain reading (the sky is clear), A4 gate with rain, A5 OLED drift, A6 and B4 at 200 % text |
| D2 over YouTube | OK on the card: homeMs 728, askedMs 728, firstLiveMs 728 ms; helper bridge answered in 1 ms; first picture after 419 ms, 30.4 a second; `cameras: the full view takes the front from youtube.leanback.v4` is in `shell-rust.log`, not in compositor.log |
| D3 Back | `cameras: Back brings youtube.leanback.v4 back`; YouTube is in front again at its home page (restarted, the video did not resume), claims empty, `cameras.switched` false |
| A6 Back on a live card | card gone in 0.18 s (ctl poll), claims empty, `free` 2, `ended:` line written; a second event within 30 s is held by the cooldown, one after it works |
| A8 untouched card | claim granted and released 11.99 s later |
| D4 probe | Up shows the chrome, the third Up opens the strip (timeline with Live, -10 s, Pause, +10 s, Event buttons; `d4-up3.png`); Left x4 then OK closes the strip and stays on live, no playback line in the helper log; DOWN, RIGHT, OK from the strip went to the camera grid |
| Panel refresh | the compositor log shows `kms: the panel refreshes every 20.00 ms` as the base after the 120 Hz run (16.68 ms only while a video window is up); `ctl refresh 60` did not restore 16.68 ms. Frames120 found the same and fixed the gate and the idle redraws in b421f486 |
