# Camera video windows and the panel rate; Back resuming the app

## Resume here

**Panel left at 100 Hz after a camera view: the helper declares the stream's rate as one the panel keeps 120 Hz for (2026-11-05; host-tested, not yet on the set).**

A camera whose stream says 25 fps took the panel to 100 Hz at its first picture and left it there after the view closed (the Ring's full view and camera 240's 1080p25: `lumatv-tv-20261104b/gate/tvs2-120.out.txt`, round 23). The helper now writes 30 (60 for 50, 120 for 100) over the rate in the parameter sets of the key pictures it hands the feeder (`helper/app/src/live/rate.rs`, `live/mod.rs` `for_the_feeder`), so the decoder reports a rate the panel keeps 120 Hz for. The TV check is `compositor/tools/tv-camera-rate-probe.py`.

- Which rate the panel follows: the one the decoder reports for the stream, which comes from the stream's own parameter set (its VUI timing). Not the pace the pictures arrive at, and not the rate the video sink is told, which is the decoder's. Presenting 25 fps pictures at 30 by repeating them would leave the stream saying 25 and the panel at 100 Hz. Kernel (LG's GPL tree): `_vsc_check_fr` (`v4l2_vsc_event.c:2676-2718`) maps window 0's rate in tenths of a Hz, 245 to 255 and 495 to 505 (990 to 1011 halved first) to 50, which is 100 Hz, and every other rate to 60, 120 Hz; the rate is the decoder's register (`vp_kwrap.c:1382-1394`), which is the decoder's frame rate from `vrate.c` `select_rate_source` (`:264-362`): a fixed rate, else with both the container's rate (`CONF`) and the stream's (`CODEC`) the one nearer the pictures' average interval (the faster when they are within a fifth), with one of them that one, and only with neither the pictures' average interval (`PTS`). The NDL pipeline sets no container rate. Nothing in it reads the pace the pictures arrive at.
- TV, round 22 (`framerate-r22/vr-A.txt`, a 1920 by 1080 camera pop-up): `CONFIG 0`, `CODEC 25/1`, `PTS 60.00` (each picture's interval counted as exactly 1500 ticks, the default for a time that did not advance [INFERENCE: the NDL pictures' times do not advance it]), `STATUS rate_src CODEC 25/1`, window 0 `vFreq 250`: the decoder followed the stream while the pictures' own average read 60. TV 2026-11-05 (`lumatv-tv-20261104f/retime/play2`, `samples.jsonl`, `monitor.jsonl`): a 640 by 360 clip whose parameter set says 25: `vFreq 250`, the panel at 100.1 Hz and the sink told `frameRate` 25; a clip that says 30: `vFreq 300`, 120.3 Hz, `frameRate` 30. Both clips were fed at the rate they say, so the set cannot tell the stream's rate from the pace; the kernel can.
- Unconfirmed until `tv-camera-rate-probe.py rates` has run: that an H.264 stream whose parameter set says 30 or 60 fps keeps the panel at 120 Hz through the feeder. Seen on the set: 30 (the clip above, 120.3 Hz) and 59.94 from Live TV (another pipeline); a 60 fps H.264 file through the feeder, and 30 or 60 on a camera, not yet. What a server sending 30 or 60 needs: the stream's own parameter set (H.264 or H.265, VUI timing present) says 30 or 60 and not 25, 50 or 100 (24.5 to 25.5, 49.5 to 50.5 and 99 to 101 fps are the 100 Hz family), whatever the pace of the pictures; a copy of a 25 fps camera stream gets that with `-c copy -bsf:v h264_metadata=tick_rate=60/1` (H.265: `hevc_metadata=tick_rate=30/1`), and the helper does the same itself now.
- The streams on the set (helper log, TV 2026-11-04): the bridge declares 240 as 1920 by 1080 at 25, 105 as 2688 by 1520 at 25, 106 as 1080p at 20, 108 as 2560 by 1440 at 20, 128 as H.265 3840 by 2160, the others none; the pictures that arrived were 10 to 21 a second.
- What the helper writes: the camera's own `num_units_in_tick` stays and `time_scale` becomes 30 (60, 120) pictures a second of it, in an H.264 sequence parameter set, an H.265 one and an H.265 video parameter set that has timing, when the rate is in the kernel's family; every other stream, a set with no timing and anything the helper's own parser does not read back the same (the old rate, the same picture, the new rate) goes as the camera sent it. Only a stream on the main window is touched: a texture draws no hardware video, and the sub window is not window 0, the one `_vsc_check_fr` reads. One log line per stream: `cameras: live for full view on camera 240: the stream's parameter set says 25 pictures a second, which would take the panel to 100 Hz; the decoder is told 30`.
- Host tests (`live::rate`, `live::tests`): x264's and x265's parameter sets at 25 and 50 fps come out bit for bit as ffmpeg's `h264_metadata` and `hevc_metadata` bitstream filters write them (`tick_rate=60/1`, `30/1`, `120/1`, `60/1`), ten H.265 sets built bit by bit (sub-layers, scaling lists, PCM, reference sets predicted from each other, long-term pictures, a VUI with every part) are accepted by that filter and rewritten the same way, H.264 sets of every shape the writer takes (profiles, scaling matrices, the three picture order types, cropping, interlace, every optional VUI part) are walked to their timing, a cut or damaged set is never a panic, and only a key picture of a stream on the main window is touched.
- Untested on the set: that the decoder takes the rewritten parameter set (the probe's `clips` stage plays the same bytes from files and `camera` the real stream); how a 25 fps stream looks declared as 30 (the decoder's own tick is then 30 a second, a picture repeats one time in six [INFERENCE: the decoder shows each picture at its next tick and repeats the last when none came]; the latency is the tick's, up to 33 ms); a stream whose parameter set has no timing (left alone: if the decoder reports 25 for one by default it still goes to 100 Hz, and `decoder.status_fps` of the `camera` stage says so); an H.265 stream at 25 fps (camera 128, if its parameter sets carry timing).
- Not solved: an app's own 25 or 50 fps video (YouTube, Netflix) still leaves the panel at 100 Hz after it closes, until the next video's first picture. The kernel does nothing with no source, a write to `/proc/lg/de/fr_rate` is ignored (round 22; `1 60` with no source and with the Ring's stream connected), and a source's first picture at another rate is what every return seen on the set was (YouTube at 30 and 60 fps, a 20 fps camera pop-up in round 23, Live TV at srv 23538.483 s, `lumatv-tv-20261104c/evidence/ramlog-current-1736.log`). The compositor follows the panel's rate (`kms: the panel refreshes every 20.00 ms now`).
- Removed: the compositor's retime clip (`retime.rs`: a 30 fps black clip under Home that put the panel back, `ctl retime`, `--retime`, `tv-retime-probe.py`) and the helper's `framerate.rs` with its `frameRate.returnOnHome` key (its `1 60` write did nothing; a `frameRate` key in `helper.json` is refused now, as any unknown key is). On the set the clip brought 120 Hz back each time, 0.4 s after it began, and changed the picture: the whole-screen clip showed no picture-in-graphics line on the bus and the TV still took the picture mode to its game mode and the screen flickered (2026-11-05, `lumatv-tv-20261104f/retime/play2`; 2026-11-04, `lumatv-tv-20261104e/retime/`).
- Why the picture changes (the colour shift seen with the 25 fps test pop-up on 2026-11-04 is this too): LG's AV connector tells videooutputd that the NDL pipeline's video is a game stream (`setVideoData` `videoInfo`: `path: network`, `lowDelayMode: true`, `isGameMode: true`, `subScalerOccupied: true`), pqcontroller takes `dimension.dynamicRange: sdrALLM` and reads the game picture mode's settings (`picture$default.game.x`, backlight 90 where Filmmaker has 80). Every NDL video on the main window, a camera view, a pop-up there, the GFN stream, runs the TV in game mode while it plays. The starfish pipeline (`GF_PIPE smp`, an app's own video) sets no game mode (`gfn/feeder/src/gf_pipe.h`), and takes `videoFpsValue` as the container's rate: a lead for the camera views, untested and not part of this fix.
- Lead, not done: the pop-up's stream limits (`helper/app/src/scrypted/popup_stream.rs`: a window only for a low stream inside 1280 by 720 at up to 30, else inside 1920 by 1088 at up to 20, otherwise a texture with stills) were chosen to keep 25 fps off the panel (round 24) and can be loosened once the check below passes.
- The 2026-10-30 runs add a second effect for a small window on the main sink: pqcontroller takes it for picture-in-graphics (PIG), greys out six rows (`colorTemperature`, `whiteBalanceActive`, `eyeComfortMode`, `gamma`, `dynamicContrast`, `dynamicColor`) with `setSystemSettingDescActive`, and applies colour gamut 0 and local dimming 0 where the owner's Filmmaker Mode has `ColorGamut=1`, `LD=1`; a window of the whole screen does not (`resetSystemSettingDesc` instead):

| Window on the 3840 by 2160 panel | Run | PIG lines | Applied `CG` `LD` |
|---|---|---|---|
| 1440 by 810 at 312,486, `fullScreen` false | `multiview/tv-2026-10-30/luna-e1`, `luna-e2` (05:23:44) | yes, six rows greyed | 0 0 |
| the same window, the video ended | `luna-e2` (05:23:59) | none | 0 0 |
| 2880 by 2160 at 480,0 | `luna-e3-feeder-livetv-id` | no | 1 1 |
| 2880 by 980 at 624,1180 | `luna-yt-feeder` | no | 1 1 |
| 3840 by 2160, `fullScreen` | `luna-e2` (05:24:12), `luna-e2c-installed-feeder-long-video` | no | 1 1 |

TV check, `tv-camera-rate-probe.py` on the set as root (it never writes `fr_rate`, changes a setting or leaves a claim or a player; the header says how the two clips are made on the workstation and `selftest` runs its readers on the host). Needs the build with `live::rate` installed, Home in front at 120 Hz, no video:

1. `read`: Home in front, 120 Hz, no source, `/proc/lxvo/vdisp/vrate` readable, the feeder able to play a file, the RAM log and the remote's key device there.
2. `rates --native30 FILE --native60 FILE` (about 30 s): from 120 Hz, an H.264 clip made at 30 and one made at 60 (the header's recipe), each fed at its own rate as a full-screen claim for 6 s and watched for 6 s after the release. Pass for each: window 0 reads `vFreq` 300 (600), `vrate` `STATUS` is `CODEC 30/1` (`60/1`), the panel is at 120 Hz through it and after. This settles the rule above for the feeder path and what a 30 or 60 fps server stream can expect.
3. `clips --clip25 FILE --clip30 FILE`: from 120 Hz, the 30 clip (the 25 clip's pictures with a parameter set that says 30, fed at 25 a second) as a full-screen claim for 7 s. Pass: window 0 reads `vFreq` 300, `vrate` `STATUS` is `CODEC 30/1`, the panel stays at 120 Hz through it and after. The 25 clip then: `vFreq` 250, the panel at 100 Hz and held there after the release (the control: the TV still does it). The 30 clip again: back at 120 Hz. Nothing but black shows.
4. `camera --downs 2`: the Ring door camera's full view (Cameras, two rows down, OK, 16 s, Back). Pass: the helper's line above, `vrate` `STATUS` `CODEC 30/1`, `osd_irq` 120 a second through the view and for 9 s after Back, pictures fed and none refused. By eye: the picture is smooth, shows no more lag than before and nothing flashes. A camera whose stream is not 25 or 50 fps says `in_family` false and proves nothing: take another (`--downs`).
5. `all` runs `clips`, `rates` (when its two files are given) and `camera`. For H.265, camera 128 the same way if the helper's line shows its set says 25 or 50.

If step 3 passes and step 4 shows no re-declaration, the helper is not the build under test or the stream's set has no timing (`vrate` `CODEC` says which); if the panel still goes to 100 Hz with the line present, the decoder reads its rate from somewhere other than the parameter set for this stream, and `vrate` says from where. If step 2 fails, a stream that says 30 or 60 is not enough for the panel and the rule above is wrong; the panel's readings and `vrate` say what the decoder did.

**Round 26 (host-tested, not yet on the set): the full view over a playing app stays over it, on the sub window, and Back finds the video where it was.**

- Cause (round 22 D3, round 25): the full view opened over an app sent Home forward and took the main window with Home's decoder. The media stack gives the
  app's pipeline up when its main window goes (the policy action seen in round 21's `m1`), so YouTube came back with nothing to play and sat at its start page;
  `front_app` raising the card was right and could not bring a pipeline back.
- Now, with an app in front whose video plays (compositor topic `vwin`, `appsPlaying`, a decoder attached) and a camera that plays live, the view takes nothing
  from the app (`cameras/app.rs`: `beside`). Home does not come forward. The shell first asks the media controller whether the app's video plays
  (`com.webos.service.mediacontroller` `getMediaSessionId {appId}`, then `getMediaPlayStatus`: `PLAYSTATE_PLAYING` or anything but paused, stopped or none; no
  session or no answer in 700 ms counts as playing), and only then pauses it with its transport key (`POST /v1/appkey
  {app, key: pause}`; the compositor taps `KEY_PAUSE` 119 into the app while it has the keyboard focus, `server/webos_shell.rs` `app_key`), the claim `cameras-full`
  is made first for what the sub window takes (h264 1920x1088 at 30, the rectangle fitted to the camera's shape), and only when it is granted the helper is asked
  `POST /v1/cameras/live {for: full, window: sub}`: the best stream inside 1920x1088 at 30 (`bridge.rs` `live_sub`: quality as set, then lower; recordings and
  clips of the strip ask with the same `window` and the same limits), the feeder's pipeline opened for the app in front (`sub_video`), as the pop-up's. A
  refused or taken-back window (`idle` mode, `both-scalers`, `too-big`) leaves the view over the app with its stills and starts no decoder.
- Back closes the view and taps `KEY_PLAY` 207 into the app (`/v1/appkey {key: play}`) only if the view paused it; a video that was paused when the view opened
  stays paused (nothing is sent either way). Nothing is raised or launched, the app never left the front. The Home key and the screen going off leave the app paused
  where it is. An app that plays no video (YouTube at its home page, Netflix browsing) keeps the old route
  (Home forward, the main window, Back raises the card).
- Costs of this route: the full view over a playing app is at most 1080p at 30 (the sub window), not the camera's own 4K; the app's video is paused by a key
  that was not read off the remote on the set (119 and 207 are Linux's `KEY_PAUSE` and `KEY_PLAY`, in the compositor's keymap). Whether YouTube registers a media
  session with the controller on this set is not known; if it does not, every video counts as playing and Back plays one the owner had paused.

**TV check (round 26)**, receiver muted, `T ctl vwin apps` (the mode the sub window needs), a camera with a 1080p or smaller stream (103, 105, 108, 128), start
from Home:

1. YouTube, a video playing full screen. `T ctl vwin` `world.appPlaying` is 1. `T ctl key 119` once by hand: the video pauses (if it does not, try `T ctl key 164`
   and note which code YouTube takes, then set `KEY_PAUSE` and `KEY_PLAY` in `server/seat.rs`); `T ctl key 207` plays it again. Note the position in between.
   Also read the play state: `luna-send -n 1 luna://com.webos.service.mediacontroller/getMediaSessionId '{"appId":"youtube.leanback.v4"}'`, then
   `getMediaPlayStatus '{"mediaId":"<id>"}'` while it plays and while it is paused: `PLAYSTATE_PLAYING` / `PLAYSTATE_PAUSED` expected, `mediaId` empty means
   YouTube has no session and the check below cannot tell.
2. A camera event over the video (`POST $H/v1/cameras/event {"camera":"Cat Feeder","kind":"animal"}`, `cameras.testEvents` on), OK on the card after 600 ms.
   Pass: compositor journal `appkey: pause sent to youtube.leanback.v4`, no `launchDefaultApp`, no `front:` line; the shell journal `cameras: the full view opens
   beside youtube.leanback.v4's video, which is paused`; `ctl vwin` has the claim `cameras-full` `slot sub`, then `placed`; `videooutput getStatus` has YouTube
   on `MAIN` unchanged and the feeder on `SUB0`; the camera fills the screen with the chrome over it; `osd_irq` stays about 121 (note it).
3. The helper log has `cameras: live for full view on camera N: ... decoder opened for youtube.leanback.v4` (the stream's size and rate are on that line, at
   most 1920x1088 at 30) and no `the sub window was granted with ... in front` line. Up/Up/Up the strip: a recording and a clip play in the same window with no new claim.
3b. Paused first: pause YouTube by hand, then open the view. Pass: no `appkey:` line at all (shell journal `appWasPaused` true in `ctl shell`), and Back
   leaves YouTube paused.
4. Back. Pass: `appkey: play sent`, the view is gone in under 300 ms, `ctl vwin` has no claim, YouTube is playing the same video a moment on from where it
   stopped (note the time on screen before and after), not at its home page.
5. The Home key from the view instead of Back: Home, YouTube paused, no `appkey: play`.
6. `T ctl vwin idle` and repeat 2: the claim is denied `app-video`, the view shows stills over YouTube, YouTube is paused and Back plays it. A 4K60 video
   (`both-scalers`) the same.
7. Over an app with no video (YouTube's start page, Netflix browse): the old route, Home forward and the main window, Back raises the card.

**Round 25 (host-tested, not yet on the set): a pop-up with Home in front is a texture, and a replaced card keeps its claim.**

- Round 24 showed that the sub window passes over an app and that Home in front, where the helper's fallback played the pop-up on the main window, took
  the panel to 100 Hz (camera 240, low stream 1280x720 declared 25). With Home in front the shell now claims `texture_only` for the pop-up
  (`system/popup.rs` `claim`: `!app_front`), so no hardware window is ever opened over Home; over an app the claim is as before and gets `SUB0`. The
  helper's fallback (a sub window granted while Home turned out to be in front) is unchanged. The full view is unchanged.
- The release ~100 ms after a grant when a new card replaced the old one (real events on camera 103): the helper's `cameras.live` topic keeps the last
  words of the previous stream, and the replaced card's own stop made it say `stopped` (or `playing`, then `stopped`) for the same camera right after the
  new card asked; the new card took that for its stream failing, gave up (`Phase::Given`), and the shell let go of the claim while the helper's new
  stream went on. The topic now carries `purpose` (`pop-up 41`, `full view`; kept by a stopped view), and a card reads only the states of its own
  purpose (`popup_live.rs` `topic`; an older helper that says none keeps the old reading).

**TV check (round 25)**, receiver muted, `popups.on` and `cameras.testEvents` on, start from Home at 120 Hz (`osd_irq` about 121):

1. Camera 240 pop-up on Home (`POST $H/v1/cameras/event {"camera":"Cat Feeder","kind":"animal"}`): compositor journal `vwin: claim camera-popup
   (camera-popup) granted a texture (no window: texture-only)`, `ctl vwin` claim `slot texture`; `osd_irq` stays about 121 during and after, no
   `the panel refreshes every 20.00 ms now`; the card's picture moves (`ctl shell` popup `live.video` true, `slot texture`).
2. Cameras 128, 105, 108 and 103 the same way on Home: textures, 120 Hz kept. Note the helper's `ended:` line for each (first picture, pictures a second).
3. Card replacement: camera 103 events (real, or two test events a few seconds apart on the same camera, and once on two cameras): the second card's claim
   is granted once and not released (`vwin: claim camera-popup released` only when the card goes), the video plays to the end of the card, the helper log
   has one `live for pop-up N` for each card and the `cameras.live` topic's `purpose` is `pop-up N` of the card on screen.
4. Pop-up over YouTube (1080p playing): `SUB0` beside YouTube as in round 24 (`slot sub`, osd_irq 121, YouTube untouched); then Home to the front while the
   card is up: the claim turns into a texture and back again.
5. Full view of camera 240 over Home (native, unchanged): note `osd_irq`.

**Pop-ups no longer reach the panel at 25 fps (round 24, host-tested, not yet on the set).** Round 23 measured it: 1080p declared 25 (camera 240) takes
120 Hz to 100 Hz and nothing written to `fr_rate` brings it back; 640x480 at 10, 1280x720 declared 25 and streams declared 20 keep 120 Hz (a 20 fps
pop-up even restores it). So the pop-up never plays a stream the panel would follow:

- The pop-up's window stream is always the camera's low stream, whatever `video.live` says (the full view keeps `video.live`, native by default).
  `helper/app/src/scrypted/popup_stream.rs` asks the bridge's `light` (Scrypted's low-resolution destination, else the smallest stream that fits)
  inside 1280x720 at up to 30; if the camera has none, inside 1920x1088 at up to 20. Both are inside SUB0's limit (1920x1088 at 30). A `501` or an
  answer that is not calm after all goes to the next step; with none the call is refused `501 no_calm_stream`. The helper logs each step and the chosen
  stream: `cameras: pop-up stream for camera N: the low stream h264 WxH at F a second, up to 1280x720`.
- A camera whose last stream list (`cameras` topic, `streams`; `popupVideo` says `window` or `texture`) has no such stream has its pop-up record say
  `textureOnly: true`; the card then claims `texture_only` (compositor `vwin`: no window is offered, the MultiView CPU texture is granted; the claim is
  denied when `ctl vwin texture off`, the screen is off or vwin is off, and the card shows stills). The decoder rate never reaches the panel.
  The texture takes only H.264 up to 1280x720 at 30, so a camera whose only stream is 1080p has stills, not video, in its pop-up. `ctl vwin claim ID
  camera-popup X Y W H texture-only` makes the same claim by hand.
- The bridge does not transcode live streams (only playback), so there is no "transcode at 20 fps" step; the bridge's own limits do the choosing.
- Which camera has which stream is not known off the set: the link token lives on the TV. The helper now lists it, `GET /v1/cameras` on the helper
  (`cameras[].streams`, `.popupVideo`); copy it into this note at the check below.

**TV check (round 24)**, pop-ups on, `cameras.testEvents` on, from 120 Hz, receiver muted:

1. `GET /v1/cameras` on the helper: write down each camera's `streams` and `popupVideo`. Camera 240 (1080p at 25 only?) should say `texture` unless it has a
   sub-stream.
2. Camera 240 pop-up on Home: compositor journal `vwin: claim camera-popup (camera-popup) granted a texture (no window: texture-only)` if `texture`, or
   a window with the helper line `the low stream ... up to 1280x720`. Pass: `osd_irq` stays about 121 a second during and after, the compositor logs no
   `the panel refreshes every 20.00 ms now`, and the card's picture moves.
3. Cameras 128, 105 and 108 as well (they kept 120 Hz before; they must still, and now play their low stream).
4. A 240 pop-up over YouTube (a texture beside the app's video, as MultiView does) and the full view of 240 over Home (native: it may still go to 100 Hz;
   write down what `osd_irq` says, the full view is not changed here).
5. If a window's low stream still moves the panel (`osd_irq` about 100): the declared rate is not the SPS's. Set `ctl vwin texture on` and make
   that camera's pop-up a texture by hand with the `texture-only` claim, and note the stream's real rate.

The watcher below (round 22) and its check list are superseded for pop-ups: it cannot restore 120 Hz while a 25 fps stream is connected.

**Back: done and passed on the set** (round 22, `888dc431`). The full view's Back used `applicationManager/launch {id}`; YouTube has
`handlesRelaunch` false, so the launch reloaded it at its start page. Back now posts `/v1/front {app}` and the compositor raises the app's own
card (`server/webos_shell.rs` `front_app`: Home minimized, the card fullscreen, SAM's subscribers told). Journal `front: youtube.leanback.v4
(window 10) raised to the front, not launched again`; YouTube was in front on the same page (before/after screenshots identical).

**Panel rate: cause known, prevention not found, restore in progress.**

- Cause: the kernel sets the output rate from the decoder's rate at a timing change of the main window (`_vsc_check_fr`: 25/50 gives 100 Hz).
  The camera stream is 25 fps: `/proc/lxvo/vdisp/vrate` reads `rate_src CODEC 25/1`, `use_fixed 0`, win0 `vFreq 250`, `connect_src VDEC` for
  the pop-up's 11 s (`framerate-r22/vr-A.txt`); the compositor logs `the panel refreshes every 20.00 ms now (16.67 before)` 0.8 s after the claim.
  The NDL load carries no frame rate (`gf_ndl.c`: width, height, codec); the 25 comes from the stream's SPS.
- A frame-drop threshold after the load (`NDL_DirectVideoSetFrameDropThreshold`, 2), which would make the decoder a direct video with a fixed
  rate of 60 (`vo.c`, `vdisp_core.c`), did NOT do that on the set: `use_fixed 0` with the threshold loaded (`888dc431`, since removed). Only
  `lowdelaymode 1` was set.
- Nothing written with no source connected restores 120 Hz: `1 60`, `0 50`, `0 60`, `1 50`, `1 60` to `/proc/lg/de/fr_rate`, each counted 1.2 s
  later, left `osd_irq` at 102 a second (`framerate-r22/vr-B.txt`). What restores it is a 30/60 fps video playing for a few seconds (YouTube),
  the kernel's own `0 60` at that video's first frame. The firmware therefore seems to take a rate request only while a video window is
  connected [INFERENCE; the next reading tests it].
- The helper's return-to-120 watcher never fired for a pop-up (Home stays in front, so SAM's foreground subscription says nothing).

**Was in the helper** (`framerate.rs`, removed 2026-11-04: its `1 60` write did nothing, and `retime.rs` replaces it): the watcher also listened to `cameras.live`.
When a camera stream started to play with Home in front it waited 1 s (the kernel re-times at the first frame), measured `osd_irq`, and at 100 Hz wrote `1 60`
(free run) with the source still connected, then measured again. When a stream ended or failed, an ordinary check ran. Log lines: `frame rate: Home measures X Hz
with a source connected to the main window`, `frame rate: Home was left at X Hz; set back to 120 Hz (Y Hz measured)`. A 25 fps stream at 120 Hz is shown in a
5-5-5-5-4 cadence, fine for a camera. The check was limited to Home in front, so an app's own 25/50 fps video kept LG's rate.

## TV check (Round22, after the frames gates)

1. From 120 Hz, a pop-up on Home. Pass: the helper log has `Home measures 10x Hz with a source connected` then `set back to 120 Hz`; the
   compositor log's `refreshes every` goes 20.00 to 16.67 ms (60 mode) about 1.5 s after the card appeared and stays; `osd_irq` 122 a second
   during and after; the card's picture is smooth.
2. If `wrote 1 60 ... output measures 100 Hz`: the firmware ignores the request even with a source; the manual reading while a card plays
   (`echo '1 60'` and `'0 60'` mid-playback) says which request, if any, works.
3. A second pop-up from 120 Hz: it goes to 100 Hz at the first frame and back within 2 s; the full view over Home likewise.
