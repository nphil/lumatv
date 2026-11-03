# Camera pop-up: live video in the corner

## Current native acceptance — 2026-11-02

Lost Scrypted pairing was replaced with a TV-generated private device credential. Only its public verifier entered the trusted bridge UI; the certificate pin was matched through the trusted HTTPS management endpoint. The new LumaTV Living Room device connects to ten shared cameras. The obsolete LumaTV verifier was removed after live acceptance; the allowlist and master token remain unchanged. Gym is reported offline by Scrypted.

Home Cameras opens the existing native grid/full viewer. Actual DISPLAY captures and advancing feeder counters prove Backyard H265 1920×1080 full-screen live playback (fed95→122), not a still. Back stops the stream and releases its window. The popup preference was explicitly Off; enabling only `scrypted-cameras/popups.on` through the existing settings route restored it. A controlled event showed Plant Room H264 1280×720 live corner video (fed34→57); Back dismissed it. Startup lost=40 remained unchanged between samples: this is not zero-loss/universal smoothness proof. Existing allocator and fallback paths remain; no second viewer/decoder or compositor change.

Temporary test-event configuration was restored byte-for-byte, helper healthy, exact original picture context/backlight restored, actual standby and own lease release verified. Proof: `/data/home/tmp/lumatv-ha-cameras-20261102/`. Current UI/helper pins: `ha-plugin-result.md`. The rounds below are historical, not current ownership or instructions to replay old installs.

## Resume here

**Done** (all on forge/main; round 22 passed A2, A3 and D1 on `2138070f`: the pop-up's picture is in its box over Home at 26.4 a second of 25, the full view over Home is full screen with no re-placing):

- Pop-up on a video window: feeder `--ndl-app` and the idle allocator `b79dd141`, `df56b311`; helper and shell `f5227463`, `7cf6befc`; installer `0f207d4c`; QML stills `bc641553`.
- Full view as an app, previews by path, the texture grant, the card's place: `92aad301`, `0a26df49`, `ea267bc7`, `1e715ee6` (the pop-up and the cards 32 px from the top and right edges).
- **Round 21's storm and the units** (`21a560e3`): a window that is not full screen is asked for in the UI's 1920x1080, which videooutputd scales to the panel; it was sent in the panel's pixels. The re-place check is read a moment after a placement, full-screen aware, and bounded at two. Code: `compositor/crates/lumatv-compositor/src/{vwin.rs,server/foreign.rs,luna.rs}`, tests `tests/{vwin,protocol}.rs`.
- **The strip plays in the full view's window** (the commit after it): the full view's live asks for the video window (`for:"full"`, `window:"main"`, the decoder opened for Home); a recording or a clip from the strip (`POST /v1/cameras/playback`) went to the feeder's configured `card` window with the window's own app id, a second feeder process that videooutputd does not connect, beside a claim that has no window. They now name the same window (`window:"main"` when the compositor has video windows and the camera plays live), the helper keeps it for Play after a Pause (`Held.side`), and a seek reuses the pipeline and the picture on the plane. Host-tested (`tests/scrypted.rs` `the_full_views_recording_plays_where_its_live_does...`, with the old way as the control: it reloads; `cameras/app_tests.rs`); not yet seen on the TV (D4).

**Next**, in order (Round 22 owns the TV):

1. Install the build; D4 is the new reading (the strip in the full view over Home: Up, pick a recording or a clip, its picture shows in the whole screen, a second seek is quick, Pause and Play go on in place, Back goes to live); then D2 and D3 over YouTube (`homeMs`, `askedMs`, `firstLiveMs`, Back returns), the gate with a live card up (A5), A6 to A8. A2, A3 and D1 are done.
2. If the full view over Home is black or small after the switch: read `T ctl vwin` (`sinks[].fullScreen`, `displayOutput`, `replaced`) and the compositor journal for `left as it is`; the next suspects are a refusal of the full-screen placement (`Invalid Parameters` in `videooutput:` journal lines) and the old app's video still holding the scalers (a settle delay after Home comes forward).
3. The art saver's picture (the vplane's non-full-screen window) was doubled by the same bug and is now its designed size (1856x1024 inside the screen, drifting): look at it once; likewise any app that plays video in a window rather than full screen.
4. Nitin's go for `lumatv feeder install` where it is not installed, and a re-add of the device in Scrypted if the link says `refused`.

**Open questions**: whether videooutputd keeps a sink's old `displayOutput` after a `fullScreen` placement (the code treats `fullScreen` as the answer); first frame under 1 s over an app (`homeMs`, `askedMs`, `firstLiveMs` say where the time goes); what the texture costs the app in front.

**Artifacts**: renders `docs/mocks/host-rust/system/popup-live-*.jpg`; the units evidence `docs/research/multiview/tv-2026-10-30/luna-yt-feeder/journal-media.txt` and `luna-lgmv-both-playing/journal-media.txt` (`[doSetDisplayWindow]` against `[setWindow] output[...]`); the storm `docs/research/compositor/slot-2026-10-29-r21/d1-home.out`.

A Scrypted event (person, doorbell, animal on a pet camera) puts a card in the top-right corner. The picture box is the camera's live video on a spare video window where one is free, stills at 8 a second where none is. Back dismisses, OK opens the Cameras full view of the same camera, and the card ends at its own time. Design: `scrypted-cameras-design.md` 3.3 to 3.7, `cameras-ui-spec.md` "The pop-up". Renders: `docs/mocks/host-rust/system/popup-live-{dark,light}{,-text200}.jpg`.

## The order

1. The helper's policy makes the record (`cameras.popup`: `live` says whether it can play live video, `source` is `scrypted`). The shell draws the card with the event's picture, then stills.
2. The shell claims a video window for the picture box (`camera-popup`, kind `camera-popup`, posted as soon as the card is drawn). Nothing is decoded yet.
3. Only when the compositor grants the claim does the shell ask the helper for the stream (`POST /v1/cameras/live {camera, for:"popup", window}`). A decoder is never started beside an app's video.
4. The picture box becomes a transparent hole only when the stream plays, the claim is placed, videooutputd lists the video on a sink (`claims[].sink`) and 250 ms have passed. Before that, and whenever any of it fails, the stills stay.
5. Back, the card's own time, an alert or a refused window ends the stream (`POST /v1/cameras/live/stop`) and releases the claim. OK keeps the stream for the full view.

The card fades and does not slide: its glass is four clipped bands around the hole.

## Measured on the set (round 20, 2026-10-29)

- videooutputd connects a video only for an app id on SAM's foreground list. The feeder's window id (`org.nphil.lumatv.cameras`) is not on it: claim granted, window placed, sink never connected. The feeder now opens its decoder for another app id than its window's (`--ndl-app`, `GF_APP`): `com.webos.app.home` over Home connected MAIN (appId home, 1280x720@30).
- The "SDL_webOSSetExportedWindow: no video device" lines are not the fault; they also appear in the working vplane and GeForce NOW runs.
- **The "parked at 3840,2160" video was a window asked for in the wrong units.** videooutputd takes a window's `displayOutput` in the UI's 1920x1080 and scales it to the panel itself (x2 on the C4); only a full-screen video (`fullScreen`, 3840x2160) is in the panel's pixels. Its own log shows it: LG's Multi View is asked for 12,273 944x531 (`[doSetDisplayWindow]`) and applies 24,546 1888x1062 (`[setWindow] output[...]`); the feeder's window asked for 312,590 1440x602 in the panel's pixels was applied at 624,1180 2880x1204 (`docs/research/multiview/tv-2026-10-30/luna-yt-feeder/journal-media.txt`). The compositor sent panel pixels, so every window but a full-screen one was twice as large and, for a corner window, off the panel: the black rectangle of round 20 (2560,1440 960x540 held as 3840,2160 with no size) and the storm of round 21 (the pop-up's 1316,247 520x292 held as 3840,988 0x1168, which the full view's claim was then compared with). It now sends a window in the UI's pixels (`ui_rect`), full screen in the panel's. The camera pop-up's small video over Home was therefore not in its box before this [INFERENCE: round 21 read `video true` and the sink, not the picture]; whether it is now is the TV check's to say (A3).
- **Re-placing is a safety net, not a loop.** A claim whose video is on a sink somewhere else than it says is placed again at most twice (`REPLACE_MAX`), no sooner than 0.7 s after the video was last placed and first seen on the sink (the check runs at every frame and the status may predate the placement), then left with one journal line (`left as it is`). A sink that says `fullScreen` for a claim that covers the output is where it should be whatever rectangle it held before. `ctl vwin` has `replaced` (the counts) and each sink's `fullScreen`.
- An NDL pipeline holds both scalers (`subScalerOccupied`, `lowDelayMode`). A second feeder kills the first (errorCode 600), and the foreground app's own id takes its MAIN away (YouTube stopped). Live video next to a playing app is therefore not offered: `ctl vwin` defaults to `idle` on the set (stills over apps), `apps` is an experiment.
- `idle` denied every claim with `app-video` because an idle exported element of LG Home was counted as an app's video. Only the front non-Home window's elements that play, or are about to, count now; `ctl vwin` lists them (`elements`).

## Limits

- With Home in front the pop-up is always a texture (the shell's claim is `texture_only`): Home's decoder is on the main window, where a 25 a second stream takes the panel to 100 Hz. Over an app it plays on the sub window (`SUB0`).

- Hardware video only where no app plays video: Home (the wallpaper's window is taken for the card and returns afterwards) and idle apps. Over YouTube, Plex, a game or a 120 fps stream the compositor's texture fallback (MultiView's: the feeder decodes the camera's low stream, at most 1280x720 at 30, and the compositor draws it into the card's box; `ctl vwin texture off` to switch it off) carries the card as live video; with it off, or where the compositor refuses it, the card shows stills, 8 a second (4 where an app holds both scalers), smoother than the QML card's one a second but not video. What the texture costs the app in front is the TV check's to say (B).
- A pop-up's window plays only the camera's low stream (at most 1280x720 at 30, else 1920x1088 at 20: the streams the panel keeps 120 Hz with, `helper/app/src/scrypted/popup_stream.rs`); a camera without one is claimed as a texture only (`texture_only`), never a window. The full view is unchanged. See `vwin-refresh-result.md`.
- One live camera at a time; the full view and the pop-up share the one feeder session, the later one wins.
- Needs the feeder and its LS2 role on the set: `lumatv feeder install` (the owner's command; a role is a permission, so apply never writes it).
- Needs the Rust `popup` kind (`ctl shell rust +popup`); the QML card remains for sets without it and asks for stills at 8 a second.
- Whether the black rectangle seen in round 20 is gone, and the first-frame latency, are the TV check's to say.

## Settings and journal

Helper `cameras`: `popupLive` (true), `popupWindow` (`video`, the full view keeps `feederWindow`), `ndlApp` (`auto`: Home's id for the main window, the window's own for the sub one; `home`, `own` or an id), `testEvents` (false). Pop-up kinds follow the design (person, doorbell); a camera named like a pet or a cat has `animal` on too. The Home Assistant camera path is not in the helper (removed in `55fa00b5`); it is to come back as a source choice.

Back from the full view raises the app (`POST /v1/front`, journal `front: ...`) instead of launching it, which reloaded YouTube at its start page. A 25 fps camera in the main window re-times the panel to 100 Hz; the helper writes the rate back while it plays: `vwin-refresh-result.md`.

Helper log, one line per decision: `cameras: pop-up N for KIND on NAME (... source scrypted, S s[, a test][, live video where a window is free|, stills only])`, `cameras: KIND on NAME (...) makes no pop-up: <reason>`, `cameras: live for pop-up N on camera C: CODEC WxH at F a second declared, window ..., decoder opened for ...`, and at its end `... ended: first picture after X ms, N pictures in S s = F a second`. The feeder's own log is `<run dir>/feeder.log`. Compositor: `vwin: claim X's video is held at ... not ...: placed again (n)`.

## Previews by path

A preview takes the best stream that stays smooth where it plays, and takes nothing from the app in front.

| Path | Where | Stream | Picture |
|---|---|---|---|
| Video window, main or sub | Home, idle apps (the compositor granted the claim) | the camera's own, inside 1920x1088 at 30 (`accept_popup`) | live video at the camera's rate |
| Texture / CPU (MultiView's, `slot: "texture"`) | no hardware window | the low stream, H.264 up to 1280x720 at 30 (`accept_low`, quality `light`) | live, drawn by the compositor into the claim's rectangle; the hole needs no sink |
| Stills | over a playing app (`app-video`, `no-window`) | stills at the box's size | 8 a second (`&fps=8`) |
| Stills, heavy | an app holds both scalers (`both-scalers`: a game at 120 Hz, 4K video) | stills | 4 a second (`&fps=4`) |

The grid's tiles (and so the picker's) keep the owner's pace over Home (Smooth 125, Balanced 200, Light 333 ms) and are asked for no more than twice a second over an app (`OVER_APP_TILE_MS` 500). The helper takes `quality` (`native`, `balanced`/`medium`, `light`/`low`) on `POST /v1/cameras/live`, and `window: "texture"` on `for: "popup"`.

## The full view is an app of its own

OK on a card or a tile of a camera that plays live, with the compositor's video windows on and an app in front, takes the front the way an app switch does: LG Home is launched (`launchDefaultApp`), the app goes to the background, and the view is already up on the card's own picture (the card's last still seeds the view's, so the first frame has a picture). Home is the one app LG's video path connects a decoder's video for here, so the stream is asked for only once Home is in front: `POST /v1/cameras/live {camera, for:"full", window:"main"}`, the camera's own stream up to 4K at its own rate (the owner's `quality`, native by default). The picture is a hole over the main window once the stream plays, the claim is placed and videooutputd lists the video on a sink. Back closes the view and launches the app again (`launch {id}`); the Home key, the screen going off and a view that was opened over Home take nothing back. Without the compositor's windows, with `cameras.liveOverApps` on, or for a camera that does not play live, the view stays over the app on stills as before.

The time is read in the shell (`ctl shell`, `cameras.firstLiveMs`: from the OK that opened the view to the first live picture) and in the helper's log (`live for full view ...: the bridge answered in X ms`, then `ended: first picture after Y ms`). The start inside the helper was already parallel (the feeder is leased and the decoder loaded, 0.2 to 0.4 s, while the stream is dialled and Scrypted's prebuffer replays); what is new is the order at the app switch, and the stream from the pop-up that is kept for the view when it is the same stream.

Known risks: the old app's video may still hold the scalers when the decoder loads right after Home comes forward (then `cameras.live` fails: OK asks again, and a settle delay after the foreground change is the first thing to add); videooutputd may not connect a video whose decoder loaded before Home was in front (this order avoids it). Home is the anchor because it is the one app LG's video path is known to connect a decoder's video for here; the card app (`org.nphil.lumatv.card`, `cardapp-result.md`: a real SAM app with its own card window, in the switcher) is the alternative anchor if launching Home proves unreliable: the shell would launch it instead and the feeder open its decoder for its id, with no change to the view itself.

## TV check (round 22)

`T` runs a command on the set, `H` is the helper's loopback base URL as the earlier probes use it.

Setup:

1. Build `sh gfn/feeder/build.sh`, then the ipk as usual, and install it with the round's wrapper.
2. `T lumatv feeder install`, then `T lumatv feeder status`: `installed` and `role` true. Without Nitin's word for the role, the round-20 temporary role file for `<data dir>/bin/lumatv-gfn-feeder` stands in (`ls-control scan-volatile-dirs`, `scan-services` after).
3. `T ctl shell rust +popup +cameras`, `T ctl vwin idle`. Helper settings: `cameras.testEvents` true, the rest as shipped.
4. `T lumatv-helper scrypted-link --check` must say `connected: 10 cameras`. `refused` means Nitin has to add the device again in Scrypted; stop there.

A. Home, live. Home in front.

1. `T curl -s -X POST $H/v1/cameras/event -d '{"camera":"Cat Feeder","kind":"animal"}'` answers `popup:true`.
2. Poll `T ctl shell` every 250 ms for 6 s: `rust.ui.popup.live` goes `phase asked` to `playing`, `window carried`, `slot main`, `sink MAIN`, then `video true`. Note the time from the POST to `video true`. `T ctl vwin`: claim `camera-popup` granted, placed, `sink` set; the sink's `displayOutput` is the claim's rectangle twice (the picture box at 1316,247 520x292 is 2632,494 1040x584).
3. `T shot popup-home`: `popup-home-video.png` has the camera's picture in the card's box (this is the first time it is asked for in the right units: if the box is black and `displayOutput` is not twice the claim, say what the sink holds and its `fullScreen`), `popup-home-display.png` the card's glass with the box clear. Look at it moving for 10 s (the camera's own motion, not a still).
4. After the card ends, the helper log's `ended:` line: first picture ms, pictures a second against the camera's declared rate (smooth: at least 90 % of it). The compositor's journal has no `placed again` line for it, or two at most and then `left as it is` (`T ctl vwin` `replaced`).
5. Frame gate: `python3 docs/research/perf/gate.py --surfaces quick --input-key 241 --tag popup-base` over Home with no card, then again with an event sent 1 s before (`--tag popup-live`). Both PASS; the card must not cost the compositor frames.
6. Back on a live card after 600 ms: card gone in under 300 ms; `T ctl vwin` has no claim and `free` is back to 2 (the wallpaper returns within 3 s); the helper log has the stream's `ended:` line. A second event after that works (no stuck feeder).
7. OK on a live card: the Cameras full view of the same camera opens and keeps playing, no `live/stop` between.
8. An event left alone ends by itself at `lastsMs`, and the claim goes with it.

B. Over YouTube (1080p30 playing). Send the same event. With the texture fallback on (`ctl vwin texture on`, the default) no hardware window is given over an app's video, so the card plays the camera's low stream decoded by the feeder and drawn by the compositor:

1. `rust.ui.popup.live` has `window carried`, `slot texture`, `video true`; `T ctl vwin` claim `camera-popup` granted with `slot texture`, `sink texture`; the helper log has `live for pop-up N ...: h264 WxH ... window texture` with a low-resolution stream (at most 1280x720 at 30). The corner moves (the camera's own rate, not a still).
2. YouTube's picture and sound are unharmed, measured against 30 s without the card: its frame counter (`world.appPlaying` 1, no stall), the compositor's frame gate (`docs/research/perf/gate.py --surfaces quick --tag popup-texture`), and the feeder's CPU (`top -b -n 1` while the card is up). Any lost frame of the app: `T ctl vwin texture off` and repeat; the card then shows stills (`window refused`, `app-video`, 8 a second) and the app must be whole. Back clears the card, the claim goes and the feeder stops.
3. Over a game or a 120 fps stream (both scalers held): the same, and the same two readings; if the app drops frames with the texture on, `texture off` and the card shows stills at 4 a second (`both-scalers`).

C. Only if A passes and Nitin agrees to a glitch risk: `T ctl vwin apps`, repeat B with YouTube, and `cameras.ndlApp` `own` for one Home run. Stop at the first glitch of YouTube's picture, `T ctl vwin idle`, note the mode, app and frame rate.

D. The full view as an app (the time is OK to the first live picture, target under 1 s).

1. From Home, OK on a tile of a 4K camera (Cameras tile in the quick panel, then OK), and OK on a live card over Home (the same sink, its window changing from the card's box to the whole screen): `T ctl shell` `rust.ui.cameras.firstLiveMs`, the helper log `live for full view on camera C: CODEC WxH at F ...` (native size and rate) and `ended: first picture after Y ms ... F a second`, the capture `T shot full-home` (`-video.png` has the camera at native size, `-display.png` the chrome). The compositor journal and `T ctl vwin`: no `placed again` line for `cameras-full` (or two at most, then one `left as it is`), `replaced` empty or at most 2, the sink's `fullScreen` true and its `displayOutput` 0,0 3840x2160 (say what it holds if not). Report the resolution, the rate against the declared one and the three times.
2. Over YouTube (1080p playing), an event, then OK on the card after 600 ms: the journal has `cameras: the full view takes the front from <YouTube's id>`, Home is in front within a second (`T ctl shell` `rust.ui.cameras.homeMs`: OK to Home in front; `askedMs`: OK to the stream asked for, the same moment), then the stream plays. Report `homeMs`, `askedMs`, the helper's `the bridge answered in X ms`, `first picture after Y ms`, `firstLiveMs` (the sum, the target under 1 s) and the picture the first frame had (the card's).
3. Back: the journal has `cameras: Back brings <the id> back`, the app is in front again (its own state is the app's to say: note whether YouTube resumed or restarted), `T ctl vwin` has no claim, the feeder stops, and `T ctl shell` `cameras.switched` is false. The Home key from the view: Home, no app launched.
4. The strip in the full view over Home (a camera with recordings, such as Front Door): Up opens the strip, pick a moment in it (or a clip from the sheet below it) and OK. Expected: the recording fills the screen on the same window and sink as the live picture (`T ctl vwin` still one claim `cameras-full`, one sink at 0,0 3840x2160 `fullScreen` true; the capture `T shot strip-rec` `-video.png` shows the recording, not the live picture); the helper log has `live for full view recording on camera C: ... window video, decoder opened for com.webos.app.home` and no second feeder process (`ps | grep lumatv-gfn-feeder`: one), `feeder.log` has no new `loaded:` line with `ndl app org.nphil.lumatv.cameras`. A second seek a few seconds on is quick (report the time from OK to the new picture, from `cameras.live` `positionMs` moving) and shows no black frame. Pause (OK on the playing recording) holds the picture, Play goes on from there in place (no `placed again`, no reload: one `loaded:` line in all), and Back from the strip goes to live (`for:"full"` again, the same window). The same for a 1080p camera. If the picture is black or the live picture shows under the recording: say which processes and claims `ctl vwin` lists and what `ctl shell` `rust.ui.cameras` says (`hole`, `window`, `expect`).
5. A first-open failure after the switch (`cameras.live` `failed`, or `SUB_SCALER ... released` in `/var/log/messages`): note the time since Home came forward.

Report: per step pass or fail, the POST to `video true` time, first picture ms, pictures a second against the declared rate, the two gate results, and the captures.

## Changelog line

Camera pop-up: the card plays the camera's live video in the corner on Home where a video window is free (stills at 8 a second over a playing app, 4 where an app holds both scalers), with the feeder installed by `lumatv feeder install`; the idle video-window allocator no longer counts Home's idle element as an app's video. The Cameras full view is an app of its own: OK from a card or a tile over an app brings Home forward, plays the camera's native stream on the main window, and Back returns to the app; its strip's recordings and clips play in that same window and pipeline (they went to a second, card-window feeder before). A video window that is not full screen is asked for in the UI's own pixels, which videooutputd scales to the panel (it was asked for in the panel's, twice as large and off the screen for a corner window), and a video that stays where it is after two placements is left alone (it was placed again twelve times).
