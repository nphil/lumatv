# Round 24 (TV slot 2026-10-30, SubWindow2)

Installed forge 4307d7d3 (ca633c8d, the sub window for the app in front, and 4307d7d3, pop-up streams): helper sha256 44c645a8644d156e..., compositor md5 f9b375ee... equal to the build, persistent, live. Receiver muted during the checks.

## Sub window beside an app (ca633c8d)

| Check | Result |
|---|---|
| Test pop-up over YouTube, camera 105 | the sub window granted; `SUB0` 1280x720 connected for `youtube.leanback.v4` beside YouTube's 1920x1080 at 24 on `MAIN`, full screen and unchanged; the feeder's connector `current [2:MainVideo], new [4:SubVideo]`, `videooutputd` `set winMode(pip) automatically, videoSinkName(SUB0)`; no policy action; first picture 150 ms; osd_irq 121 before, during and after (`multiview/tv-2026-10-30/r24/popup-105-over-youtube-sub0.png`) |
| Real person events over YouTube, camera 103 (640x480 at 15) | the same: sub window, `SUB0`, decoder opened for YouTube's id, 16 pictures a second |
| Camera 240 over YouTube | `SUB0`, 1280x720 at 25 (`popup-240-over-youtube-sub0.png`) |
| Full view | camera 103 from the Cameras grid: `cameras-full` granted the main window, 2560x1920 on `MAIN`, placed, panel 120 Hz |

## Pop-up streams (4307d7d3, `docs/research/vwin-refresh-result.md`)

`GET /v1/cameras` (`multiview/tv-2026-10-30/r24/cameras.json`):

| Camera | Streams | popupVideo |
|---|---|---|
| 88 Backyard | native H.265 1920x1080; two streams without a size | texture |
| 103 Back Door | H.264 2560x1920, 896x672, 640x480 | window |
| 104 Front Door | H.264 1920x2560, 896x672, 640x480 | window |
| 105 Plant Room | H.264 2688x1520 at 25, 1280x720 at 25 | window |
| 106 Bird | H.264 1920x1080 at 20, 640x360 at 20 | window |
| 108 Tool Room | H.264 2560x1440 at 20, 640x360 at 20 | window |
| 128 Office | H.265 3840x2160, 896x672, 640x480 | window |
| 168 Gym | none (offline) | texture |
| 196 Downstairs Door | H.264 without a size | texture |
| 240 Plant Room Cat Feeder | H.264 1920x1080 at 25, 1280x720 at 25 | window |

Camera 240 on Home, from 120 Hz: the low stream (1280x720 declared 25) on the main window; osd_irq 119, then 100 to 101 from the second second to after the pop-up; the compositor logged `the panel refreshes every 20.00 ms now (16.67 before)`, the helper `wrote 1 60 to fr_rate with Home in front at 99.9 Hz, but the output measures 99.9 Hz`. Fail: a 25 a second stream re-times the panel at 1280x720 too.

Cameras 108, 128 and 105 on Home afterwards started from 100 Hz and stayed there; not a clean reading. Real person events on camera 103 replaced the card every few seconds during the round, and each replacement released the new card's claim within about 100 ms while its video went on playing (`vwin: claim camera-popup released` right after the grant).

Camera 240 over YouTube took `SUB0` beside YouTube's video (not a texture); osd_irq 121.

Verdict: the sub window passes; the pop-up streams check fails, so `popups.on` is left off.

## End state

LumaTV persistent and live on 4307d7d3, Home in front, panel 120 Hz (osd_irq 121), `popups.on` off, `cameras.testEvents` off (`helper.json` as before the round), YouTube closed, receiver unmuted at 0.52.
