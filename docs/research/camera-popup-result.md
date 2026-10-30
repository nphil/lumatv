# Camera pop-up: live video in the corner

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
- After connecting, videooutputd parked the video at 3840x2160 although the compositor's placement had been accepted. The compositor now compares the sink's destination with the claim and places it again (up to 12 times, 200 ms apart).
- An NDL pipeline holds both scalers (`subScalerOccupied`, `lowDelayMode`). A second feeder kills the first (errorCode 600), and the foreground app's own id takes its MAIN away (YouTube stopped). Live video next to a playing app is therefore not offered: `ctl vwin` defaults to `idle` on the set (stills over apps), `apps` is an experiment.
- `idle` denied every claim with `app-video` because an idle exported element of LG Home was counted as an app's video. Only the front non-Home window's elements that play, or are about to, count now; `ctl vwin` lists them (`elements`).

## Limits

- Hardware video only where no app plays video: Home (the wallpaper's window is taken for the card and returns afterwards) and idle apps. Over YouTube, Plex, a game or a 120 fps stream the compositor's texture fallback (MultiView's: the feeder decodes the camera's low stream, at most 1280x720 at 30, and the compositor draws it into the card's box; `ctl vwin texture off` to switch it off) carries the card as live video; with it off, or where the compositor refuses it, the card shows stills, 8 a second (4 where an app holds both scalers), smoother than the QML card's one a second but not video. What the texture costs the app in front is the TV check's to say (B).
- One live camera at a time; the full view and the pop-up share the one feeder session, the later one wins.
- Needs the feeder and its LS2 role on the set: `lumatv feeder install` (the owner's command; a role is a permission, so apply never writes it).
- Needs the Rust `popup` kind (`ctl shell rust +popup`); the QML card remains for sets without it and asks for stills at 8 a second.
- Whether the black rectangle seen in round 20 is gone, and the first-frame latency, are the TV check's to say.

## Settings and journal

Helper `cameras`: `popupLive` (true), `popupWindow` (`video`, the full view keeps `feederWindow`), `ndlApp` (`auto`: Home's id for the main window, the window's own for the sub one; `home`, `own` or an id), `testEvents` (false). Pop-up kinds follow the design (person, doorbell); a camera named like a pet or a cat has `animal` on too. The Home Assistant camera path is not in the helper (removed in `55fa00b5`); it is to come back as a source choice.

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

## TV check (round 21)

`T` runs a command on the set, `H` is the helper's loopback base URL as the earlier probes use it.

Setup:

1. Build `sh gfn/feeder/build.sh`, then the ipk as usual, and install it with the round's wrapper.
2. `T lumatv feeder install`, then `T lumatv feeder status`: `installed` and `role` true. Without Nitin's word for the role, the round-20 temporary role file for `<data dir>/bin/lumatv-gfn-feeder` stands in (`ls-control scan-volatile-dirs`, `scan-services` after).
3. `T ctl shell rust +popup +cameras`, `T ctl vwin idle`. Helper settings: `cameras.testEvents` true, the rest as shipped.
4. `T lumatv-helper scrypted-link --check` must say `connected: 10 cameras`. `refused` means Nitin has to add the device again in Scrypted; stop there.

A. Home, live. Home in front.

1. `T curl -s -X POST $H/v1/cameras/event -d '{"camera":"Cat Feeder","kind":"animal"}'` answers `popup:true`.
2. Poll `T ctl shell` every 250 ms for 6 s: `rust.ui.popup.live` goes `phase asked` to `playing`, `window carried`, `slot main`, `sink MAIN`, then `video true`. Note the time from the POST to `video true`. `T ctl vwin`: claim `camera-popup` granted, placed, `sink` set.
3. `T shot popup-home`: `popup-home-video.png` has the camera's picture in the card's box, `popup-home-display.png` the card's glass with the box clear. Look at it moving for 10 s (the camera's own motion, not a still).
4. After the card ends, the helper log's `ended:` line: first picture ms, pictures a second against the camera's declared rate (smooth: at least 90 % of it), no `placed again` storm (at most 2).
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

1. From Home, OK on a tile of a 4K camera (Cameras tile in the quick panel, then OK): `T ctl shell` `rust.ui.cameras.firstLiveMs`, the helper log `live for full view on camera C: CODEC WxH at F ...` (native size and rate) and `ended: first picture after Y ms ... F a second`, the capture `T shot full-home` (`-video.png` has the camera at native size, `-display.png` the chrome). Report the resolution, the rate against the declared one and the three times.
2. Over YouTube (1080p playing), an event, then OK on the card after 600 ms: the journal has `cameras: the full view takes the front from <YouTube's id>`, Home is in front within a second (`T ctl shell` `rust.ui.cameras.homeMs`: OK to Home in front; `askedMs`: OK to the stream asked for, the same moment), then the stream plays. Report `homeMs`, `askedMs`, the helper's `the bridge answered in X ms`, `first picture after Y ms`, `firstLiveMs` (the sum, the target under 1 s) and the picture the first frame had (the card's).
3. Back: the journal has `cameras: Back brings <the id> back`, the app is in front again (its own state is the app's to say: note whether YouTube resumed or restarted), `T ctl vwin` has no claim, the feeder stops, and `T ctl shell` `cameras.switched` is false. The Home key from the view: Home, no app launched.
4. The camera's strip in the full view (Up, then a clip) plays and Back from it goes to live; the same for a 1080p camera, where the card's stream (up to 1080p30) is already the camera's own.
5. A first-open failure after the switch (`cameras.live` `failed`, or `SUB_SCALER ... released` in `/var/log/messages`): note the time since Home came forward.

Report: per step pass or fail, the POST to `video true` time, first picture ms, pictures a second against the declared rate, the two gate results, and the captures.

## Changelog line

Camera pop-up: the card plays the camera's live video in the corner on Home where a video window is free (stills at 8 a second over a playing app, 4 where an app holds both scalers), with the feeder installed by `lumatv feeder install`; the idle video-window allocator no longer counts Home's idle element as an app's video. The Cameras full view is an app of its own: OK from a card or a tile over an app brings Home forward, plays the camera's native stream on the main window, and Back returns to the app.
