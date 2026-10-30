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

- Live only where no app plays video: Home (the wallpaper's window is taken for the card and returns afterwards) and idle apps. Over YouTube, Plex or any playing video the card shows stills, 8 a second, smoother than the QML card's one a second but not video.
- One live camera at a time; the full view and the pop-up share the one feeder session, the later one wins.
- Needs the feeder and its LS2 role on the set: `lumatv feeder install` (the owner's command; a role is a permission, so apply never writes it).
- Needs the Rust `popup` kind (`ctl shell rust +popup`); the QML card remains for sets without it and asks for stills at 8 a second.
- Whether the black rectangle seen in round 20 is gone, and the first-frame latency, are the TV check's to say.

## Settings and journal

Helper `cameras`: `popupLive` (true), `popupWindow` (`video`, the full view keeps `feederWindow`), `ndlApp` (`auto`: Home's id for the main window, the window's own for the sub one; `home`, `own` or an id), `testEvents` (false). Pop-up kinds follow the design (person, doorbell); a camera named like a pet or a cat has `animal` on too. The Home Assistant camera path is not in the helper (removed in `55fa00b5`); it is to come back as a source choice.

Helper log, one line per decision: `cameras: pop-up N for KIND on NAME (... source scrypted, S s[, a test][, live video where a window is free|, stills only])`, `cameras: KIND on NAME (...) makes no pop-up: <reason>`, `cameras: live for pop-up N on camera C: CODEC WxH at F a second declared, window ..., decoder opened for ...`, and at its end `... ended: first picture after X ms, N pictures in S s = F a second`. The feeder's own log is `<run dir>/feeder.log`. Compositor: `vwin: claim X's video is held at ... not ...: placed again (n)`.

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

B. Over YouTube (1080p30 playing). Send the same event.

1. `rust.ui.popup.live.window` is `refused` (or `waiting`, then `refused`); `T ctl vwin` claim `camera-popup` denied with `app-video` or `both-scalers`; the helper log has no `live for pop-up` line. The card shows stills (look at the corner for 10 s: about 8 a second, not one).
2. YouTube's picture and sound are unharmed: `world.appPlaying` 1, no stall, no switch to another app. Back clears the card; the claim list is empty.

C. Only if A passes and Nitin agrees to a glitch risk: `T ctl vwin apps`, repeat B with YouTube, and `cameras.ndlApp` `own` for one Home run. Stop at the first glitch of YouTube's picture, `T ctl vwin idle`, note the mode, app and frame rate.

Report: per step pass or fail, the POST to `video true` time, first picture ms, pictures a second against the declared rate, the two gate results, and the captures.

## Changelog line

Camera pop-up: the card plays the camera's live video in the corner on Home where a video window is free (stills at 8 a second over a playing app), with the feeder installed by `lumatv feeder install`; the idle video-window allocator no longer counts Home's idle element as an app's video.
