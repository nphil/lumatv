# Cameras: the feeds that did not load, and the fill of the bars (2026-11-04)

Host work only: nothing here was run on the set. The TV and Scrypted were read, not changed (logs, the helper's state, Scrypted's device and stream
list); the streams were played from the workstation through the helper's own RTSP client.

## The feeds that did not load

Evidence is the helper's log on the TV from 00:25 to 00:33 EST, Scrypted's stream list, and each stream played from the workstation with the helper's
client (`rtspplay`: codec, profile, size, first picture, rate).

| Camera | What it is | What the TV did | Cause |
|---|---|---|---|
| Downstairs door (196) | Ring Doorbell Gen 2 through Scrypted's cloud Ring plugin: one WebRTC-sourced stream, no Rebroadcast or prebuffer on it | the session failed 200 ms after it started, twice (the helper dials once more); the log said only "failed" | the bridge answers with the one-shot address Scrypted opens for it (`rtsp://<scrypted host>:<port>`, no path, 25 s), and nothing answers on it from the LAN: the dial is refused in 82 ms from the workstation as well. Not a codec, a size or a decoder limit: no byte was read |
| Plant room (105) | TP-Link Tapo C225 over ONVIF: H.264 High 2688x1520, 25 fps declared (the SPS says 20), prebuffered | two starts refused at 00:26:34 and 00:26:35 with the bridge's 409 `offline`; the owner moved on within a second | Scrypted's `online` flag for it was false at that moment (the list the TV keeps, refreshed every 30 s, and Scrypted itself said online before and after; a 100 s sample showed no flapping). The stream is sound: from the workstation the first picture came in 38 ms at 28 pictures a second. The viewer then showed a black screen under "Starting" for good: the refusal was a 3 s toast, nothing asked again, and a prebuffered camera draws no stills |

Time to first picture and pace, from the helper's own lines on the TV (full view unless said): Cat Feeder 150 to 526 ms at 26 to 33 pictures a second; Tool Room (108)
224 ms at 32 a second; Back Door (103, 4:3 doorbell) 238 ms at 18.6; Gym (168) 284 ms but 5 pictures in 7.9 s; Front Door (104, the portrait doorbell) 1697 ms (1663 ms as a pop-up)
at 10.0 a second, the picture 480x640 under a stream declared 640x480.

Not resolved: Office (128, 3840x2160 H.265, its low stream 640x480 not prebuffered) had no picture in 5.7 s, Bird (106, H.264 High 1920x1080 20 fps, prebuffered) none in 3.9 s on its second
start, and Backyard (88, generic RTSP H.265, no size or rate declared) none in 2.2 s, while the owner pressed Right every 1 to 6 s; Bird's stream plays from the workstation at once. The sessions of 103,
104 and 105 were left after 0.3 to 1 s, too soon to say. The kernel's media log shows "video codec change on PAUSED state" on the video sink at the switches between a 1080p and a 640x480
stream [INFERENCE: the sink is re-set between codecs and sizes while a new session starts, which would slow the first picture after a switch]. The old log line could not tell; now it says
how long a session lasted with no picture and how many units the feeder took, refused and lost.

Also found, in the bridge (`/data/home/LumaShow/wt/ScryptedBridge`, not changed): the doorbells' native 2560x1920 streams report no codec, so the bridge counts their fit as
unknown and picks the known 640x480 sub stream, declared landscape for the portrait doorbell's 480x640 picture: the full view of both Reolink doorbells is that stream, soft at
full screen, and the portrait one's first picture takes 1.7 s against the 4:3 one's 0.24 s.

## What changed

- The full view's refused start (the 409) is a note over the camera's stills, with the cause in the helper's words, and "This camera is offline / Looking
  again every few seconds" for `offline`. A refusal that passes is asked again by itself after 3, 6 and 12 s and then every 30 s, eight times; a stream that
  drops after it played the same way. A camera that never played is not asked again by itself (a battery doorbell would wake for each ask), nor is a refusal that is
  the same next time. A refusal of a switch to another camera stops the session of the camera that was left, which kept playing unseen (it may bear on the open item
  "TV reboot in camera full view over YouTube": a second decoder alive beside the app's; not verified).
- The helper logs why a session failed (the words name no address) and what a session with no picture lasted; the view follows a stream whose own parameter
  set says another size; an unreachable stream reads "Scrypted's stream for this camera could not be reached".
- Regression tests: the Ring doorbell's answer shape (codec only, no size, not prebuffered, an address nobody answers at, audio with no codec) fails in words with no size made up and
  no pipeline loaded, and a following refusal reads "The camera is offline" in the helper's live state as well as in its answer (`helper/app/tests/scrypted.rs`); the model's refusal,
  retry, give-up, stale-refusal and stop-the-other-session cases (`app_tests.rs`).

## The fill of the bars

The owner's request: fill the bars of cameras that are not 16:9 (front door, back door, any doorbell or portrait camera) with a blurred copy of the live video, in the spirit of
Stashy's `LiveBlurBackdropView`.

**How the camera planes are composed.** The full view's video is the NDL decoder's output on a hardware video window the compositor places at the fitted
rectangle (`vwin`, `LiveClaim.rect`), under the UI plane; the layer is a hole over it. The UI never sees a decoded frame. A second window showing the same
stream is a second decode (one decoder output feeds one window; a second NDL session is unmeasured), so the first choice, a second plane, is out. The
frames cannot be read, so the second (a tiny sample of them) is out too. What the UI can read is what the grid already reads: the still the bridge makes from
the camera's low stream, which the helper holds and serves.

**What is used: the snapshot the grid already has, refreshed every 2 s.** `GET /v1/cameras/{id}/frame?w=32&h=18` (the helper decodes the bridge's 320 px JPEG
at one eighth, 40x23, a few kilobytes), asked for every 2 s while the video plays in a granted, placed window and only for a camera whose fit leaves bars of 16 px or
more. Every 2 s because the helper closes a camera's stills stream after 3 s of no ask: the bridge's ffmpeg for it stays the one it had. Nothing is decoded twice on the
TV, the video plane is not touched, and nothing is asked for a 16:9 camera, a hidden or covered view, or a note.

**What the CPU does with it.** The picture is taken to 48 px on its longest side by a sparse mean, blurred twice (3x3), dimmed and tinted, and only the part the bars
show is made again, at one pixel in 8 of the screen's (4 for a tile), by bilinear sampling with an eased fraction (a plain bilinear shows creases at this magnification).
The compositor scales it up. No GPU blur pass. The video plane's bars are two image layers either side of the window, each clipped to its bar; a still shows whole
over a wash of itself (one layer under it), made at most once a second; a grid tile whose picture is not 16:9 within 4 % shows the whole picture over a wash of itself, with the tile's
round corners, instead of its middle. The tile's tint is the theme's `scrim` role (dark and light follow the page); the full view's is the view's dark ink in both modes, since the video
is on black in both. `theme.cameras.fillBars` turns it all off.

The plane path (no granted window: the stream plays on LG's plane as it did) has no rectangle to fill around, and its bars stay black.

### Numbers (host, `lumatv-ui --host --virtual`, 120 Hz, the gate's model: 1.5 ms + 0.085 ms a layer, 8.3 ms ≈ 80 layers)

`tests/host_budget.py --surfaces cameras-grid,cameras-grid-nofill,cameras-full,cameras-full-nofill`; the app alone, no Home under it; the worst of three runs. The host's
software renderer is not the set's GPU, so the cost is the model's, from the layers drawn; the model's work in the worst frame never passed 5.0 ms (8.3 ms is the 120 Hz frame).

| Walk | Most layers in a frame, fill off | fill on | Frame cost (model), off | on | Over 8.3 ms |
|---|---|---|---|---|---|
| grid, opening (8 tiles, 3 not 16:9) | 39 | 39 | 4.8 ms | 4.8 ms | 0 frames |
| grid, stills at rest (5 a second a tile) | 16 | 21 | 2.9 ms | 3.3 ms | 0 |
| grid, focus walk | 33 | 38 | 4.3 ms | 4.7 ms | 0 |
| full view, a 3:4 doorbell playing in its window | 23 | 26 | 3.5 ms | 3.7 ms | 0 |
| full view, chrome gone / back | 23 | 26 | 3.5 ms | 3.7 ms | 0 |

The fill adds two layers beside the video (one more behind a still, one more for each tile that shows whole): at most +0.4 ms of the model's GPU cost in a frame, and the
worst frame is the grid opening in both. The commits the process applied took at most 0.1 ms in a frame, except at the openings (the full view's: 0.2 ms on average and 0.6 ms at
worst, the same with and without the fill).

CPU of the UI process for the wash (optimised build, host at load 12, so upper figures): a tile's wash from the 480x640 still, 0.5 ms once a second (0.05 % of a core); the full
view's bars from the 40x23 picture, 0.6 ms every 2 s; from a 1280x720 still, 0.75 ms. The first version took 7.5 ms and 19 ms (a full-area mean per pixel): at 120 Hz that
was a dropped frame for every tile that refreshed, so the mean is sparse now (one pixel in 16 to 64 of those under each of the tiny picture's) and the sampling is by axis. The helper's decode
of the wash picture is one eighth of a 320 px JPEG. On Scrypted's side a portrait camera in the full view holds one stills stream at 320 px and the grid's rate
(the design's estimate for a stream is 2 to 4 % of a core; not measured).

The picture path of a tile (cut, round the corners, paint) takes 0.2 to 0.4 ms on the host; this change rounds the corners in the cut picture instead of in a second copy of it, a saving
too small for the measurement to tell.

### Renders

The scenes are in `tests/rust_cameras_scenes.py` (`--only full-doorbell,full-wide,grid-shapes,full-offline`, dark and light; `LUMATV_UI_BIN` names the binary): `full-doorbell` (a 3:4 camera in its
window, the bars beside it), `full-wide` (4:3), `grid-shapes` (a doorbell, a 4:3 camera and a 16:9 one among the tiles), `full-offline` (the note for a camera Scrypted says is offline). The renders are not kept in git.

## TV steps (nothing here is accepted until it is checked on the set)

1. Install from `main` with the owner's go (the TV is not touched tonight).
2. Open the full view on Front Door (portrait): the bars are black for under a second (the tile's picture is the first wash), then a blur of the camera; Back Door (4:3): narrower bars.
   Open a 16:9 camera: no bars, and no `frame?w=32&h=18` in the helper's log.
3. Frame gate over the portrait full view, fill on and off (`docs/research/perf/gate.py`), and the video check: no frame dropped in motion; YouTube beside it unharmed.
4. Grid: the portrait tiles show the whole picture over a wash, the others as before; the focus lift and the scroll stay at the gate's numbers.
5. Downstairs door (bridge 0.2.1): the full view plays through the bridge's relay, in about 2 to 5 s when the doorbell is awake. When it is slow or asleep the note ("Scrypted sent no picture in time") shows over the stills, and "Try again" within 20 s joins the start. The tile and the backdrop show a snapshot, a new one a minute. Untested on the set.
6. Plant room: while Scrypted says it is offline, the note and the retries; when it is back, the video starts without a key.
7. Cycle the cameras quickly as the owner did and read the helper's log for the sessions that end with no picture (Bird, Office, Backyard): the new line says how long and what the feeder took.

## Open owner decisions

- Ring (downstairs door): resolved in the bridge, with no new service. The address Scrypted opens for the doorbell is a loopback listener for one client; the bridge (`lumashow-scrypted` main, 0.2.1) is that client, as Scrypted's HomeKit plugin is, and serves the stream on its own RTSP address (its docs, 12.3); its pictures are `takePicture` snapshots (12.5). From the workstation, through that address: first frame 1280x720, H.264 Constrained Baseline, 25 a second; the helper's RTSP client (`rtspplay`) got its first picture 1.1 s after PLAY. The doorbell's own start is Ring's: 0.6 to 3 s, about 10 s, or none within 20 s (5 of 24 starts connected on 2026-11-04). The TV gives up after about 7.5 s (the call's 3.5 s and the 4 s DESCRIBE reply), so a slow start needs "Try again" while the bridge's session still waits (20 s). Open: give the player the stream's description at once, so the TV waits for the picture (8 s) and not for the description (4 s), or a longer reply timeout for relayed live.
- Plant room: the bridge refuses a camera Scrypted marks offline before it tries its stream; trying anyway would start it through a flap. A bridge change.
- The doorbells' stream choice (above): resolved on the TV side. The full view asks for the main stream with no codec list when the listing names none (`bridge::accept_full`), and the Front Door's portrait main stream (1920x2560) fits the main window as 2560x1920 does (`Need::fits`). Its first-open failure ("an earlier write to the feeder failed", 2026-11-04 22:55 to 22:56 on the set) was that stream's claim ending as too big on the switch to it and the compositor closing the feeder's window; the writer now replaces a feeder that breaks under a session once. Untested on the set: the main window decoding a portrait 1920x2560 picture. If it refuses, `video.live` = Medium plays the 896x672 stream.
