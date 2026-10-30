# Camera video windows and the panel rate; Back resuming the app

## Resume here

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
