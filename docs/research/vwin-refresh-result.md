# Camera video windows and the panel rate; Back resuming the app

## Resume here

**Panel left at 100 Hz after a full view (2026-11-04, host-tested, not yet on the set).** The Ring's full view (25 fps) re-timed the panel and it stayed at 99.9 Hz after the view closed (`lumatv-tv-20261104b/gate/tvs2-120.out.txt`). The compositor now writes `1 60` to `fr_rate` once, 1.5 s after the last video window and source are gone with Home in front and the policy automatic (`refresh.rs` `Retime`); a video that is still playing, or one that returns, holds it back. This is the helper's request made from the compositor, and round 22 measured it ignored with no source: if `osd_irq` stays near 100 after the journal line `refresh: the panel was left at 100 Hz by a video; 120 Hz asked for`, only a playing 30 or 60 fps video restores 120 Hz and this write is to be replaced by one.

TV check: Home at 120 Hz, open the Ring full view, close it with Back. Pass: the journal line above 1.5 s after the view is gone, `osd_irq` about 121 a second, the compositor log `the panel refreshes every 8.33 ms now`; no line while the view is up.

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

**Now in the helper** (`framerate.rs`): the watcher also listens to `cameras.live`. When a camera stream starts to play with Home in front it
waits 1 s (the kernel re-times at the first frame), measures `osd_irq`, and at 100 Hz writes `1 60` (free run) with the source still connected,
then measures again. When a stream ends or fails, an ordinary check runs. Log lines: `frame rate: Home measures X Hz with a source connected to
the main window`, `frame rate: Home was left at X Hz; set back to 120 Hz (Y Hz measured)`. A 25 fps stream at 120 Hz is shown in a 5-5-5-5-4
cadence, fine for a camera. The check is limited to Home in front, so an app's own 25/50 fps video keeps LG's rate.

## TV check (Round22, after the frames gates)

1. From 120 Hz, a pop-up on Home. Pass: the helper log has `Home measures 10x Hz with a source connected` then `set back to 120 Hz`; the
   compositor log's `refreshes every` goes 20.00 to 16.67 ms (60 mode) about 1.5 s after the card appeared and stays; `osd_irq` 122 a second
   during and after; the card's picture is smooth.
2. If `wrote 1 60 ... output measures 100 Hz`: the firmware ignores the request even with a source; the manual reading while a card plays
   (`echo '1 60'` and `'0 60'` mid-playback) says which request, if any, works.
3. A second pop-up from 120 Hz: it goes to 100 Hz at the first frame and back within 2 s; the full view over Home likewise.
