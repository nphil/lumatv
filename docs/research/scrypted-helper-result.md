# Cameras through Scrypted, the TV helper (ScryptedHelper, 2026-10-29)

Slice B of `scrypted-cameras-design.md`: built and host-tested; nothing has run on the TV and nothing has run against Nitin's Scrypted (the bridge is not installed).
The Home Assistant camera path is gone from the helper.

## Resume here

- **Off until linked.** No link file, no connection: the helper looks at `<data>/scrypted/link.json` every 5 s and nothing else exists (no feeder, no stills, no push channel).
  Linked and awake, it keeps one kept-alive HTTPS connection for the calls and one for the push channel; asleep, it closes both and wakes only when the screen topic changes.
- **The link** is `lumatv-helper scrypted-link --brand B --address 192.168.1.69:10444 --pin <fingerprint> --new [--device LumaTV]`: the TV makes its own token (`lsb_` and 32
  random bytes as base64url), saves it in the 0600 file and prints one JSON object with only the verifier (the token's SHA-256) and the device name. `--check` connects and prints
  `{"ok":true,"connected":true,"cameras":N,"message":"connected: N cameras"}` or the refusal in words (untrusted pin, refused token, not answering). Run again with other flags it changes only
  what is given and keeps the token (`--pin` after the bridge made a new certificate); `scrypted-unlink` removes the file. Only an IP literal on the home network is taken.
- **Transport.** `rustls` 0.23 with the `ring` provider; the certificate is trusted by the SHA-256 of its DER and by nothing else, and a different one ends the handshake before a byte of
  the TV's own is sent (the state is `untrusted`, the bridge's log shows no request). The signature of the handshake is still checked by webpki.
- **States** (`cameras.state`): `unlinked`, `connecting`, `ready`, `offline` (the last list is kept; also standby: "The TV is asleep"), `refused` (the token was refused: no cameras shown), `untrusted`.
  A refused or untrusted link is tried again every 30 s, an offline one after 1, 2, 4, 8, 16, 30 s.
- **Live** (`live/`): `POST /v1/cameras/live` asks the bridge for the stream, then the session thread leases the GFN feeder, loads the pipeline with what the bridge says the stream is
  while the RTSP reader dials, holds the first pictures 150 ms and starts at the last random access point, so the catch-up from Scrypted's prebuffer is at most one key interval. The
  feeder process is kept 120 s after the last stream (`cameras.feederIdleSeconds`) and ends with the helper's connection (`--once`). A seek, a pause and the return to live of the same
  camera reuse the pipeline and the picture on the plane; another camera or size loads again. Stop returns at once and unloads, tears RTSP down and tells the bridge off the caller's path.
  The queue holds 15 pictures once caught up and drops to the next key picture past that; a stream behind by more than 500 ms for a second is dialled again once; quiet for 8 s ends the session in words.
- **Pop-ups.** The push channel (`GET /v1/events/stream`, resumed with `Last-Event-ID`) feeds `cameras.events` (the last 30, as sent, plus `seq`) and the policy of
  `scrypted/policy.rs` (a pure function): pops up when on, the kind is in the camera's list (default person and doorbell), phase `new`, the camera and kind not within the cooldown (a
  doorbell ignores it and stays half as long again), and the event is not older than 30 s (a reconnect's replay). An `update` re-publishes the same card id with new words. Only cameras the TV shows are heard.

## What the shell reads beyond design 3.4

| Where | Addition |
|---|---|
| `cameras.popup` | `thumbnail`, `shownAt` (helper clock), `overApps` (the page's `popups.over_apps`: the shell hides the card over a playing app when false), `test` |
| `cameras.live` | `paused`; `startedAt` and `positionMs` are `null` while live; `reason` never carries an address |
| `GET /v1/cameras/{id}/frame` | 200 only for a still newer than `cameras.frameHoldMs` (10 s), else `404 no_picture`, `503 not_connected` or `503 busy` (nine cameras at once); the first ask waits up to 700 ms for a first picture |
| `POST .../live`, `.../playback` | the bridge is asked first and a refusal is `409 {error: {code, message}}` with its word, leaving the running session alone; the last of two quick calls plays (`superseded`) |
| `POST /v1/cameras/live/pause {paused}` | holds live or a recording (the dial ends, the picture stays, `positionMs` freezes); `false` dials the recording at the rate it had, 500 ms before the key picture the held one was decoded from (live by the TV's clock of its soonest picture, plus the bridge's skew) |
| `POST /v1/plugins/scrypted-cameras/actions/forget` | deletes the file and clears every topic at once |
| `POST /v1/cameras/event {camera, kind}` | only with `cameras.testEvents: true`, else `403` |

Playback position is `started_at + stream time x rate` `[INFERENCE: check C10]`.

## Config

`cameras.live` (now `true`: the link is the switch), `feeder`, `feederWindow`, `feederLibrary`, `feederIdleSeconds` (120), `frameHoldMs` (10000), `maxPictureBytes`, `testEvents` (`false`).
Removed: `enabled`, `port`, `relay`, `popupKinds`, `popupSeconds`, `cooldownSeconds` (an old `helper.json` that sets one, such as the TV's `{"cameras": {"enabled": true}}`, is read all the same: they are ignored and one log line says so; any other unknown key is still refused).
The Cameras page is the first-party plugin `scrypted-cameras` (page rebuilt at a higher revision when the shown cameras change; `notify.<id>` values stay when a camera leaves).

## Removed and changed

`cameras.rs`, `live/{relay,hls,fmp4,net,sps}.rs`, the `ha-bridge` dependency, `tests/cameras.rs` and its HLS fixtures. `live/gf.rs` stays; `live/mod.rs` is the RTSP session. New dependencies: `rustls`
(ring, std, tls12) and `rtsp-client` (by path, from the LumaShow `tv-cameras` checkout). `helper/build.sh` builds `ring` for aarch64 musl with zig cc through a shim that drops the cc crate's
`--target=` (zig rejects it). The helper's connection threads have 1 MiB stacks: an unoptimised build needs more than 256 KiB to describe the page and to handshake.
`scrypted/bridge.rs` is an SDK-free copy of the Show's `plugins/scrypted/src/bridge.rs` requests and readers (the Show's takes the plugin SDK's types) until the shared crate of design 3.2 exists.

## Tests (host)

`helper/app/src/scrypted/*` and `live/mod.rs` unit tests (pin, token, store, policy, SSE and multipart readers, HTTP framing, bridge requests, page, queue and lag rules) and
`helper/app/tests/scrypted.rs` (29 tests): the whole helper against a fake bridge over real TLS with a pinned certificate (`tests/support/bridge.rs`), a fake RTSP camera and, where
`sh gfn/feeder/build.sh` has built it, the real host feeder on the fake NDL library (0 `VIOLATION` lines). They cover off until linked, ready, wrong pin (nothing sent), refused and back, offline and
back, standby, forget, pop-up policy end to end, resume and reset of the push channel, one stills stream a camera, the timeline routes and refusals, live start, stop, switch, refused start,
playback, pause and resume, end of recording, a clip, quiet and dead streams.

## TV checks of the design (6) that apply

C1 link (`scrypted-link`, wrong pin, removed device), C2 the bridge's self-test, C3 and C4 stills and grid (rates 3, 5, 8), C5 and C6 event to pop-up and policy, C8 to C11 live start, latency, playback and
switch (`cameras.live` carries `fed`, `dropped`, `lost`, `delayMs`), C12 and C13 feeder window over an app and a second session, C14 open and close, C15 cost and standby, C16 hardware JPEG, C17 failure, C18 off.
`cameras.testEvents` must be `true` for C5. The helper's `live` is `true` by default; `cameras.live: false` leaves stills only.

## Not built

Audio. The shared policy crate and the shared bridge crate (the Show's follow-up). The timeline and cards routes of the Rust shell's crate (not asked once the shell slice stopped).
