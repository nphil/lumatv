# Cameras through Scrypted: the design for the TV, shared with the Show (ScryptedDesign, 2026-10-28)

Design only; nothing here was built, and nothing was run on the TV. Evidence tags: `TV:` measured on this set (file named), `SHOW:` measured on a Show against Nitin's
Scrypted, `SRC:` source (a path in `koush/scrypted` at `50b8668ea8`, 2026-10-27, server 0.148.0, unless another repository is named), `PUB:` public documentation,
`LAN:` an unauthenticated read-only look at Nitin's Scrypted on 2026-10-28 (`GET /login`, which asks for no credentials, on three addresses, and TCP connects to three ports), `[INFERENCE]` not run.

## Resume here

**What this settles.** The TV's cameras come from Scrypted, not Home Assistant: live video at the camera's native size straight from Scrypted's RTSP rebroadcast onto the
video plane, a grid and the event pop-up from stills Scrypted's host makes, a timeline and an events and clips list from the NVR, and event pop-ups pushed the moment
Scrypted's detectors fire, with the events chosen per camera in the TV's own Settings.

**A Scrypted bridge and a Show plugin already exist; the TV reuses them.** Two LumaShow worktrees have them:

- `/data/home/LumaShow/wt/ScryptedBridge` (repository `lumashow-scrypted` on the forge, branch `main`, `eef9e55`): the **LumaShow bridge**, a TypeScript Scrypted plugin with an
  HTTP API v1 (health, cameras, recordings, events, clips, thumbnails, playback), its own pinned-certificate HTTPS port 10444, a hashed bearer token, a camera allow-list that
  starts empty, read-only by construction, reviewed. Host-tested with fakes; **not installed** (`LAN:` 192.168.1.69:10444 refuses).
- `/data/home/LumaShow/wt/lumatv` (branch `tv-cameras`, the checkout the helper already depends on): `plugins/scrypted` (Rust, the Show's camera provider, plugin API level 2,
  commit `19a2a142`), `crates/rtsp-client` (an RTSP client for "a Scrypted restream", H.264 and H.265 to Annex B access units), `crates/fetch` (`lan`: pinned-certificate
  requests) and `docs/camera-wall.md` (the Show's measurements against Nitin's Scrypted).

**Decisions.**

| # | Decision | Why |
|---|---|---|
| D1 | The Scrypted side is the existing bridge, extended to **v1.1** (additive): live stream addresses, stills, a push channel for events, event thumbnails, native-quality playback, `package` as a kind, one token per device | Scrypted offers a client an RPC session under a login, or a plugin inside Scrypted; the bridge is that plugin, already reviewed, and a login cannot be made without an admin (section 4.3) |
| D2 | The TV side is a **first-party helper module** (`scrypted`) that replaces `cameras.rs` and the Home Assistant sources of `live/`; it shares `rtsp-client`, the bridge's request and reader code and the options schema with the Show | The TV speaks plugin API level 1: no network, no decoder, 64 KiB frames, images only as RGBA or NV12 assets. A sandboxed plugin cannot carry video or pictures here |
| D3 | The full view plays the camera's **native** stream: Scrypted rebroadcast RTSP (TCP) to `rtsp-client` to the GFN feeder to LG's NDL on the video plane | Measured: the decoder takes 4K120 HEVC at 100 Mbit/s with no drop and starts in 0.18 to 0.36 s; Scrypted's prebuffer gives an instant start |
| D4 | The grid, the pop-up and the scrub bubbles are **stills** (JPEG, 4 to 8 a second a camera) made by the bridge, never one decoder a tile | LG's resource table and the Show's wall show decoders are the scarce part; a second NDL session on this set is unmeasured (check C13) |
| D5 | Timeline, events and clips use the bridge's recordings, events, clips, thumbnails and playback calls, with playback in the recording's own codec and size (`accept` limits) | Scrypted has no call that lists recorded ranges; the bridge probes, and the Show already built the strip on it |
| D6 | A link needs **no Scrypted login**: the TV makes its own secret, the bridge stores only its hash | No supported non-interactive way to make a Scrypted user exists; a user is also wider than the bridge's token (section 4.3) |
| D7 | The Home Assistant camera path is **removed**, not kept as a fallback | None of it is installed or has run on the TV; Scrypted's cameras in Home Assistant are the same cameras and have no stream there |

**Open, in this order:** Nitin's steps (4.2), then the first run against the real NVR (what only it can settle, 1.7), then the TV checks (6). Build slices in 5.

## 1. What Scrypted offers a client

### 1.1 Nitin's Scrypted

| Fact | Value | Evidence |
|---|---|---|
| Host | Docker container `scrypted` (image `ghcr.io/koush/scrypted:nvidia-legacy`) on BeastNAS, LAN address 192.168.1.69; host networking is the stock compose default `[INFERENCE]` | `LAN:` `addresses ["https://192.168.1.69:10443"]`, `hostname "BeastNAS"`; the image as deployed, per Main |
| Doors | https 10443 (self-signed certificate), http 11080 (reachable from the workstation); 10444, the bridge's port, refuses | `LAN:`; ports `SRC: server/src/server-settings.ts` |
| Name | `https://cctv.nateshome.net` (Caddy at 192.168.1.71, valid certificate) answers `/login` the same | `LAN:` |
| Users | `hasLogin: true`: users exist | `LAN:` |
| Plugins | Scrypted NVR (device id 130), Rebroadcast, ONNX detection on the GPU, MQTT (202), Kestrel (Nitin's own plugin), Events Recorder 0.0.52 (a community plugin Kestrel patches) | `SRC: /data/home/Kestrel/scrypted-plugin/src/main.ts`, `README.md` |
| NVR | volume `/NVR`, "Video Retention (Days)" 3 by default, H.265 recordings on some cameras | `SRC: Kestrel README, src/clipfiles.ts:6-11` |
| Cameras | eight with sub streams played through Rebroadcast by a Show: Front Door 480x640 H.264, Tool Room 640x360, Back Door 640x480, Backyard 768x432 H.265, Plant Room 1280x720, Bird 640x360, Office 640x360, Cat Feeder 1280x720; 10 to 25 pictures a second | `SHOW: docs/camera-wall.md` (2026-10-19) |
| Load | Scrypted's container used 31 to 58 % of one of 16 cores serving 8 to 12 sub-stream clients | `SHOW:` same |

The main streams' sizes are not recorded anywhere yet; the bridge's `/v1/cameras` `native` field and its self-test report them (1.7).

### 1.2 Ways in, and what each credential can do

| Way | What it is | Source |
|---|---|---|
| Login, Basic over https | `GET /login` with `Authorization: Basic user:password-or-token` answers `authorization` (a signed token), `queryToken`, and the user's static `token` | `server/src/scrypted-server-main.ts:176-191,714-741` |
| User token | 16 random bytes, made with the account and replaced only when the password changes; accepted as the Basic password and as `Authorization: Bearer <token>` or `?scryptedToken=`; never expires | `server/src/services/users.ts` (`setScryptedUserPassword`), `scrypted-server-main.ts:289-295,333-338` |
| Signed login token | `POST /login` with `maxAge`; `{u,a,t,d}` signed with a per-start salt; at most one year | `server/src/usertoken.ts`, `scrypted-server-main.ts:223-238,597-646` |
| Client library | `connectScryptedClient({baseUrl, pluginId: "@scrypted/core", username, password})`, engine.io to `/endpoint/@scrypted/core/engine.io/api`, then Scrypted's RPC (device proxies, `listen`, media conversion) | `packages/client/src/index.ts`, README |
| CLI | `scrypted <device>[@host] <method> <args>`; `scrypted login` prompts for a password and keeps `{username, token}` in `~/.scrypted/login.json` | `packages/cli/src/main.ts` |
| Env admin | `SCRYPTED_ADMIN_USERNAME`, `SCRYPTED_ADMIN_TOKEN`, `SCRYPTED_ADMIN_ADDRESS` of the container: an admin session with no password | `scrypted-server-main.ts:167-174,259-287` |
| Plugin | inside Scrypted: the whole SDK, no login, runs as the plugin | `sdk/types/src/types.input.ts` |

An admin is a user without an ACL id (`services/users.ts`: `admin: !user.aclId`). A non-admin sees only what its ACL names: per device, the interfaces, methods and
properties allowed (`server/src/plugin/acl.ts`); the stock ACL page grants whole interfaces by name (`plugins/core/src/user.ts`, `sdk/src/acl.ts`). Non-admin NVR use goes
through the NVR's own "Scrypted NVR Users" extension and per-camera permissions (`PUB: docs.scrypted.app/maintenance/access.html`). Every plugin HTTP route outside `/public/`
needs only *a* logged-in user; the plugin sees `username` and `aclId` and must enforce them (`server/src/plugin/plugin-http.ts:89-115`). A Rust client of the RPC would have to
speak engine.io plus Scrypted's own object-proxy protocol (`server/src/rpc.ts`, 31 KB); nothing public documents it as an API.

### 1.3 Devices

`systemManager.getSystemState()` lists every device with its `interfaces`. For cameras (`SRC: sdk/types/src/types.input.ts`):

| Interface | Gives | Notes |
|---|---|---|
| `VideoCamera` | `getVideoStreamOptions()` (streams: id, name, codec, size, bitrate, fps, `destinations`), `getVideoStream({destination, id, prebuffer, container, route})` | a camera |
| `Camera` | `takePicture({reason, picture, timeout})` | see 1.4: cached up to 20 s |
| `VideoRecorder` | `getRecordingStream({startTime, duration, playbackRate})`, `getRecordingStreamThumbnail(time, {resize})`, `getRecordingStreamOptions()`, `recordingActive` | added by the NVR as a mixin (`device.mixins` holds the NVR's id); **no call lists recorded ranges** |
| `EventRecorder` | `getRecordedEvents({startTime, endTime, count})` | events after the fact |
| `VideoClips` | `getVideoClips`, `getVideoClip`, `getVideoClipThumbnail`, **`removeVideoClips`** | the Events Recorder provides it; granting the interface grants the delete |
| `ObjectDetector` | event `ObjectsDetected` (`detections[]`: `className`, `score`, `id` of the tracked object, `label`, `labelScore`, `boundingBox`, `zones`, `movement`; `detectionId`, `timestamp`), `getDetectionInput(detectionId)` (the picture) | the NVR's smart detections |
| `MotionSensor`, `BinarySensor` | `motionDetected`, `binaryState` (a doorbell press on a `Doorbell` device) | property events |

Ids are Scrypted's numeric device ids, stable while the device exists. Times are epoch milliseconds (an event's `timestamp` sometimes arrives in seconds:
`SRC: Kestrel src/main.ts` `normalizedTime`).

### 1.4 Live video

- **Rebroadcast** (`plugins/prebuffer-mixin/src/main.ts`) keeps a connection to each camera stream it is asked to prebuffer and serves one RTSP server (port a setting). Each stream has
  a path of 8 random bytes (`:81-87`) and a second, audio-less one; there is **no user or password**, the path is the key. The camera's settings show
  `rtsp://localhost:<port>/<path>` (`:461-478`, keys `rtspRebroadcastUrl`, `rtspRebroadcastMutedUrl`): the host is substituted. `getVideoStream` instead opens a one-client listener
  on 0.0.0.0 with its own token (`:1018-1029`) and answers `urls` rewritten to the server's addresses (`local-addresses.ts`).
- **Streams and destinations.** Per camera Scrypted maps `local`, `remote`, `medium-resolution`, `low-resolution`, `local-recorder`, `remote-recorder` to the camera's streams
  (`ResponseMediaStreamOptions.destinations`); a request by destination follows what Nitin configured, so the TV never guesses a stream name.
- **Prebuffer.** Ten seconds of the enabled streams (`prebufferDurationMs`, `:30`). An external RTSP client is replayed from the **first** random access point inside the last
  `max(4 s, key interval)` (`:1231`, `common/src/rtp-codec.ts:237-256`), at line speed: a burst of up to 4 s of old pictures before live (`SHOW:` about 2 s: 40 pictures at 20 a second).
  A stream that is not prebuffered is dialled cold: the camera's next key picture, 0.4 to 4.1 s (`SHOW:`: Front Door 1.2 s, Back Door 1.9 s, Office 4.1 s, the rest 0.4 to 0.5 s);
  warm 0.14 to 0.9 s. Prebuffering is a per-camera setting ("Prebuffered Streams"); by default the high stream is prebuffered and the sub streams are not (`PUB: plugins/prebuffer-mixin/README.md`).
- **Stills.** `takePicture` goes through the Snapshot plugin, which may return an image up to 20 s old for `reason: "periodic"`, 10 s by default, none for `"event"`
  (`plugins/snapshot/src/main.ts:359-383`). A live grid therefore does not use it.
- **WebRTC** (`plugins/webrtc`, `RTCSignalingChannel`) is for browsers. Not used: on a home network it saves nothing over RTSP on TCP, needs ICE, DTLS and SRTP beside the NDL
  feed (which wants Annex B access units either way), and adds a jitter buffer.

### 1.5 Recordings

- The recording stream starts at the key picture **at or before** the requested start and ends exactly at the end of the window; a request for 12 s gave 13.4, 14.4 and 15.9 s for
  starts 0, 1 and 2.5 s later; with a `duration` it is served about 20 times faster than real time; a time with no footage answers `recording_unavailable`; it is RTSP on the
  Scrypted host; the audio of one camera is damaged (timestamps squashed into the first 0.02 s) (`SRC: Kestrel src/clipfiles.ts:6-11,24`, `src/main.ts:907-941`).
- The SDK can seek a running recording stream (`getRecordingStream(options, previousStream)`); the bridge's v1 starts a new session instead (`bridge docs/design.md` 5).
- Recorded ranges: the bridge **probes** by asking for a 32-pixel thumbnail in each cell of a grid (15 s to 30 min cells, at most 96 a call, cached 6 h recorded and 5 min not)
  (`bridge docs/design.md` 4, `docs/scrypted-bridge-api.md` 5.3).
- The NVR plugin is closed source (`@scrypted/nvr` 0.12.110 on npm: a `MixinProvider` with `HttpRequestHandler`); only the SDK interfaces above are a contract.

### 1.6 Events

`systemManager.listenDevice(id, ScryptedInterface.ObjectDetector, callback)` delivers each detection update as it happens (`SRC: Kestrel src/main.ts:418`). One tracked
object produces many updates; `detections[].id` names it. The classes Nitin's Scrypted yields are `person`, `people`, `vehicle`, `animal` (with a `label` such as a species),
`package`, `face`, `dog_cat`, `motion`, plus a doorbell press as `BinarySensor` (`SRC: ha-cameras-result.md` "What Home Assistant has", Kestrel `onDetection`). The bridge's
class table has no `package`, `people` or `dog_cat` yet (`bridge src/features/mapping.ts`): they become `other`.

### 1.7 What only a real NVR can settle

The bridge was built without access to Nitin's Scrypted; its design lists nine guesses (`bridge docs/design.md` 6). This design adds these. The bridge's self-test settings button
reads each one, read-only, and prints `FIXTURE` lines to replace the invented test data; Nitin presses it once after the install.

| # | Question | Decides |
|---|---|---|
| 1 | The rebroadcast address of each stream: the key and layout of `rtspRebroadcastUrl` per stream, or `getVideoStream`'s one-client address | how `live` finds addresses (3.3) |
| 2 | Which streams are prebuffered, and each camera's main and sub size and codec | live start time; whether the TV can take the main stream (limit 4096x2304) |
| 3 | `getRecordedEvents`: present, shape, delay after an event | the strip's ticks and backfill |
| 4 | Delay from an event to the `listen` callback; the class names; whether `getDetectionInput(detectionId)` returns a picture at once | pop-up delay; thumbnails |
| 5 | The first-picture time of `getRecordingStream`, `playbackRate`, and whether it seeks in place | scrub release to picture |
| 6 | Whether Scrypted's ffmpeg can produce stills of the sub stream cheaply (CPU a camera) | grid cost |

## 2. The playback path on the C4

### 2.1 What the set has

| Fact | Value | Evidence |
|---|---|---|
| NDL DirectMedia, one session, load to playing | 176 to 359 ms for 1080p60 H.264, 1080p120, 1440p120, 4K60 and 4K120 H.265 at 50, 80 and 100 Mbit/s; 0 dropped, no `BufferFull`; render queue 0 to 3 pictures, p99 2 to 3; a 300 s run at 4K120 80 Mbit/s with 0 dropped | `TV: docs/research/gfn/tv-2026-10-26/log.txt` |
| The video plane under the UI | a window that exports its video element puts the picture on the plane, placed by the exported rectangle, with its own caption and clock drawn over it; the camera panel's own hole under the shell's chrome is host-tested only | `TV:` the saver's player, `videoplane-saver-result.md` (Active state); `ha-cameras-result.md` for the panel |
| Codec limits | H.264 up to 4096x2304 at 60, 50 Mbit/s; H.265 up to 4096x2304 at 120, 50 Mbit/s; a multiview sub window 1920x1088 at 30 | `TV: gfn/tv-2026-10-26/device_codec_capability_config.json` |
| Platform resources | VDEC 4, G1_H264 8, HEVC cores 4, AV1 2, main scaler 1, sub scaler 1, graphic scalers 4, **image decoder 1**, display managers 3 | `TV: gfn/tv-2026-10-26/umediaserver_resource_config.txt` |
| Second NDL session | **not measured**: every probe loaded one session; the second-session question is check C13 | `ha-cameras-result.md` check 16 |
| JPEG | the hardware block (`lumatv-jpeg`) is built and **not run on the TV**; libjpeg on the CPU decodes 70 to 120 MPix/s (census), so a 480x270 still takes 1 to 2 ms `[INFERENCE]` | `hwjpeg-result.md`, `hardware-census.md` |
| CPU | 89 % idle at Home | `hardware-census.md` |

### 2.2 The full view: native, live

```
camera --RTSP--> Scrypted (Rebroadcast, 10 s prebuffer) --RTSP/TCP--> helper: rtsp-client (Annex B access units)
   --unix socket--> lumatv-gfn-feeder (32-bit, one NDL session) --> LG decoder --> video plane; the shell's chrome above the hole
```

- **Stream choice.** The shell asks for a quality (`native`, `balanced`, `light`), the bridge maps it to Scrypted's destinations (`local`, `medium-resolution`,
  `low-resolution`) and answers the address with the stream's codec, size and rate. `native` is the highest stream the set takes: at most 4096x2304 at 60 and 50 Mbit/s (LG's declared
  cap; HEVC at 100 Mbit/s ran clean in the GFN probe); the TV sends these limits in an `accept` object and the bridge picks the highest stream inside them (a 7680-wide camera falls to its next stream).
- **Start.** The link is warm (one keep-alive connection and the push channel), so `POST /v1/cameras/live` costs one round trip to the bridge, then DESCRIBE, SETUP, PLAY (three round trips,
  under 30 ms `[INFERENCE]`); the replay burst (2 to 4 s of video, 5 to 10 MB at 20 Mbit/s) arrives in 0.1 to 0.3 s and the feeder loads in 0.18 to 0.36 s in parallel. The helper starts feeding at
  the **last** random access point of the burst's first 150 ms, so the catch-up is at most one key interval of pictures (decoded faster than they play; the ES player shows each on arrival),
  a quick forward run of a fraction of a second. **Expected first live picture 0.6 to 1.0 s after OK** `[INFERENCE: check C9]`. A stream that is not prebuffered waits for the camera's
  next key picture (up to 4 s measured on sub streams): the bridge reports `prebuffered` per stream and the Cameras page says so (3.7).
- **Keeping live.** The feed queue holds at most 15 pictures; past that the helper drops to the next key picture instead of queueing; a stream whose RTP clock runs more than 500 ms
  behind the wall clock is re-dialled once. A stream quiet for 8 s ends with a reason in words.
- **Switching camera** (Left and Right): unload and load again, 0.2 to 0.4 s measured for the load, plus the burst: **0.5 to 0.9 s** `[INFERENCE: C11]`. The server's prebuffer is the
  look-ahead; the helper does not dial neighbours.
- **Latency.**

| Stage | Expected | Basis |
|---|---|---|
| Camera capture to encoded packets | 100 to 300 ms | the camera's own `[INFERENCE]` |
| Camera to Scrypted, and Scrypted's copy | under 10 ms | Rebroadcast copies packets, no decode (`prebuffer-mixin`, `createRtspParser` stream copy) |
| Scrypted to the TV, RTSP on TCP | under 5 ms | LAN `[INFERENCE]` |
| `rtsp-client`: packets to one access unit | under 1 ms | an access unit ends at the marker packet (`crates/rtsp-client/src/depay.rs`) |
| Helper to feeder | about 1 ms median | host-measured on the same path, `ha-cameras-result.md` |
| NDL queue, decode, scan-out | 1 to 3 pictures and a refresh: 50 to 100 ms at 30 a second | `TV:` queue 0 to 3 at 120 a second |
| **Glass to glass** | camera's 100 to 300 ms plus 60 to 150 ms: **0.2 to 0.45 s** | `[INFERENCE: check C9]` |

- **Audio** is off. The recorder's audio is unreliable on at least one camera (1.5) and the bridge's playback carries none. Later: G.711 first (PCM goes through the feeder's existing path),
  AAC through the set's own 32-bit libavcodec.
- **Not used:** `lumatv-vplane` (it plays a fixed looped clip for the saver and the wallpaper) and the Home Assistant relay and HLS sources; the GFN feeder takes live access units.

### 2.3 The grid, the pop-up and the scrub bubbles: stills

One decoder a tile does not fit this TV: the image decoder is 1, the platform's own multiview shows a main and one sub window (the scaler counts, `[INFERENCE]`), and a second NDL session is
unproven. So tiles are pictures. The bridge makes them from each camera's **low-resolution** stream with Scrypted's ffmpeg (decode, `fps=N`, `scale=W:-2`, JPEG), shared by every reader
of a camera, started on the first reader and stopped 3 s after the last:

- `GET /v1/stills?camera=&w=&fps=` is a `multipart/x-mixed-replace` stream (3.3); the helper holds one connection a camera while the shell asks and keeps the newest decoded picture, served at
  `GET /v1/cameras/{id}/frame` as today (the shell's `kit/CameraFrame.qml`, two hidden pictures and one quad, is unchanged).
- Grid defaults: 480x270, 5 a second a tile (the Show's own tiles run 10 to 25 a second on a decoder). Settings offer Light (3), Balanced (5), Smooth (8) `[INFERENCE: C4 sets the ceiling]`.
- Cost: 8 tiles at 5 a second and 15 to 25 KB are 0.6 to 1 MB/s; the TV decodes 40 stills a second in about 8 % of a core on libjpeg; Scrypted's host spends about 2 to 4 % of a core a camera
  `[INFERENCE: checks C3 and C4]`.
- Upgrades if Nitin wants fluid tiles, in order of cost: (a) the bridge composes one **mosaic** stream (xstack, hardware encoder where Scrypted's ffmpeg has one) and the TV plays it as one
  NDL session with its focus ring and names drawn above; (b) several NDL sessions with a hole each, only if C13 shows they run; (c) software decode in a 32-bit process on LG's libavcodec.
  None is built in v1.

### 2.4 Over an app, and on Home

The pop-up and the grid are stills, so they sit over a playing app with no second decoder and nothing of the app's playback touched (`ha-cameras-result.md`, "Pictures"; measured by C5). The **live** full view takes
the video plane: from Home it plays at once; over a playing app it opens in stills (1280x720, 8 a second) and offers "Live video" only if C12 shows the feeder window plays over the app, else leaving the
app is the way to live.

Open and close budgets: the panel is in the first frame with the last tiles (cached in memory, at most 16 cameras at 150 KB); stills and the link are warm because the push channel and one keep-alive
connection stay up while the TV is awake (closed in standby); Back removes the layer in one frame, and the RTSP teardown, the NDL unload and the feeder's end (120 s idle, so a second open skips the
process start) happen off the UI path; from a pop-up over an app, Back is one frame and the app is where it was, because nothing of the app was touched.

### 2.5 Timeline playback

`POST /v1/cameras/playback {camera, at}` gives an RTSP address from the bridge (recorder stream, played through ffmpeg `-re` and the bridge's own RTSP server; v1 copies when the recording fits
the Show and re-encodes otherwise). For the TV the bridge copies **any** codec and size inside `accept`, so a 4K H.265 recording arrives as recorded; the helper plays it through the same
`rtsp-client`, feeder and hole as live (`mode: "playback"`). While the strip is dragged the bubble shows the bridge's thumbnail of that moment; **release** starts a new session at the time (the
bridge's v1 seek); the first picture is at or up to a key interval before the time and the playhead runs from the bridge's `started_at` on the stream's RTP clock. Expected 0.7 to 2.5 s from
release to picture (the bridge's guess, `[INFERENCE: C10]`). Speeds 1, 2 and 4 are the bridge's `rate`. A pause stops reading and holds the last picture; Play re-dials at the held time.

## 3. Architecture

### 3.1 The shape

```
Scrypted (BeastNAS)                                         TV helper (Rust, root)                                    Shell (QML)
  Rebroadcast RTSP (prebuffer) ---------- RTSP/TCP --------> scrypted::live  (rtsp-client -> feeder -> NDL)              panels/Cameras.qml
  NVR: VideoRecorder, EventRecorder, VideoClips                scrypted::client (HTTPS, pin) <--+                          pop-up (overlay card), grid,
  LumaShow bridge v1.1 :10444 <-- HTTPS (pinned) + SSE ----->  scrypted::events, stills, playback, policy                    full view, timeline, events
        (live, stills, events/stream,                          scrypted::link (CLI), settings page, routes and topics --->
         recordings, events, clips, thumbnails, playback)
```

Rejected, with the reason:

| Alternative | Why not |
|---|---|
| The Scrypted client API (RPC) from the TV | needs a Scrypted login that can do everything, and an engine.io plus RPC implementation in Rust; the bridge is a plugin with a narrower key |
| A Scrypted user for the TV | cannot be made non-interactively, and is wider than the token (4.3) |
| A sandboxed LumaShow plugin on the TV | level 1: no network, no decoder, 64 KiB frames, images only as RGBA; the Show's plugin works because level 2 adds `net.lan` and `camera.provider` |
| Home Assistant as the source | its Scrypted cameras have no stream (`supported_features` 0); its events are a second hop; nothing of it has run |
| WebRTC | see 1.4 |
| One NDL session a tile | unmeasured, and the platform's multiview is two windows (2.1) |
| fMP4 or HLS from Scrypted | seconds behind (segments), a segmenter on the server, and nothing RTSP does not give; the Home Assistant path's 2 to 3 s is the same cost |

### 3.2 What is shared with the Show

| Piece | Where | Use |
|---|---|---|
| The bridge (TypeScript) | `lumashow-scrypted` | both devices; v1.1 is additive, the Show ignores what it does not know (`docs/scrypted-bridge-api.md` 3 and 10) |
| `rtsp-client` | LumaShow `crates/rtsp-client` | live and playback on both: `Session::connect`, `read` gives `Frame {annexb, pts_us, random_access}` |
| The bridge's requests and readers | LumaShow `plugins/scrypted/src/bridge.rs` (pure, 444 lines; depends on `lumashow_sdk::camera` types) | the TV helper links it as a library at first; a crate without the SDK types is the proper home (LumaShow follow-up, below) |
| `fetch::lan` | LumaShow `crates/fetch` | the Show's brokered, pinned requests; the TV needs in-process TLS (3.6) |
| The options schema and event policy | this document 3.5 (settings-schema format 1) | the TV's page now; the Show's plugin page later |
| Behaviour of the scrub bar | `camera-wall.md` "Recordings and clips in the full view" (Live pill, back 10 s, play and pause, forward 10 s, speed 1x 2x 4x, spans 15 min 1 h 6 h 24 h, ticks, hatched gaps, bubble, clips sheet) | the TV builds its own equivalent in its own look |

**For LumaShow (not edited here; the proposal).** (1) Move `bridge.rs` into a crate that does not depend on `lumashow-sdk`; both `plugin-scrypted` and the TV use it.
(2) The Show's plugin can adopt `live` (no typed rebroadcast addresses), `events/stream` (its doorbell card and the wall's lit tile) and the `package` kind through one new plugin message,
`camera.event`, additive in level 2. (3) The mosaic, if built, helps the Show's wall too (one decoder instead of six).

### 3.3 The bridge, v1.1 (the contract between slices A and B)

All additive to `docs/scrypted-bridge-api.md`; a client ignores what it does not know; `GET /v1/health` lists each in `features` with a version: `live: 1`, `stills: 1`, `stream: 1`,
`event_thumbnails: 1`, `devices: 1`. Conventions as v1 (snake_case, epoch milliseconds, the error body, `Authorization: Bearer`, home network only, pinned certificate).

**Cameras** (`GET /v1/cameras?scope=all`): without `scope` the answer is as v1 (cameras with a recorder). With `scope=all` every ticked camera with `VideoCamera` is listed and `features` gains
`live`, `stills`, `stream`. Each camera gains `room`, `type` (`camera` or `doorbell`), `online` and `streams`:

```json
{"id": "273", "name": "Front Door", "room": "Entry", "type": "doorbell", "online": true,
 "features": ["live", "stills", "stream", "recordings", "thumbnails", "playback", "events", "clips"],
 "streams": [{"quality": "native", "codec": "h265", "width": 3840, "height": 2160, "fps": 15, "prebuffered": true},
             {"quality": "balanced", "codec": "h264", "width": 1280, "height": 720, "fps": 15, "prebuffered": false},
             {"quality": "light", "codec": "h264", "width": 640, "height": 360, "fps": 10, "prebuffered": false}]}
```

`room` is `null` when Scrypted has none; `streams` is empty for an offline camera or one that did not describe its streams within 1.5 s; `recording_active`, `earliest` and `native` are `null` for a camera that does not record.

**Live** (`POST /v1/live {"camera": "273", "quality": "native", "accept": {"codecs": ["h264","h265"], "max_width": 4096, "max_height": 2304, "max_fps": 60}}`):

```json
{"rtsp": "rtsp://192.168.1.69:34135/9c1f0e5a2b7d4c36", "camera": "273", "quality": "native",
 "video": {"codec": "h265", "width": 3840, "height": 2160, "fps": 15}, "audio": null, "prebuffered": true, "expires_in_ms": 60000}
```

The address is the camera's Rebroadcast address read from the camera's settings (the muted one, without audio, when there is one: then `audio` is `null`) with its host replaced by the address the client reached the bridge on; a camera
without Rebroadcast gets Scrypted's one-client address from `getVideoStream` (host swapped the same way; it waits 30 s for its one client). `expires_in_ms` is 60000 for the first, 25000 for the second. It is a capability: never in a log,
a topic or an error. A stream outside `accept` is never chosen (a 4K camera falls to its next stream); when none fits the answer is `501 not_supported`. `video` and `audio` members the camera does not say are left out.
`404 camera_not_found`, `409 offline`, `501 not_supported`, `502 upstream_error`, `400`.

**Stills.** `GET /v1/still?camera=&w=&h=` is one fresh JPEG (at most 400 KB; the camera's own snapshot, else a picture of its stream; `Cache-Control: no-store`). `GET /v1/stills?camera=&w=&fps=` (`w` 160 to 1280, default 480; `fps` 1 to 10,
default 5) is `multipart/x-mixed-replace; boundary=lumashow`, each part `Content-Type: image/jpeg`, `Content-Length` and `X-Captured-At: <epoch ms>`; the first picture is awaited before the answer starts, so a camera that gives none is an
ordinary error answer; at most 8 cameras and 12 readers at once (`429 busy`); a reader is dropped after 10 s without reading. **Streaming answers (stills and events) have no `Content-Length` and are not chunked: the body runs to the end of the
connection (`Connection: close`).**

**Events, pushed.** `GET /v1/events/stream[?cameras=273,274&kinds=person,doorbell]` is `text/event-stream` (`kinds` default: all but `motion`; `cameras` at most 32; 4 clients at once, `429 busy`). `hello` is first on every connection;
`Last-Event-ID` resumes from the last 200 events (10 minutes); an id the bridge cannot continue from (older, from another run, not a number) answers `event: reset` before `hello`; a `: ping` comment every 15 s. The `id:` line is the message's number.

```
event: hello
data: {"now": 1792609200000, "seq": 4020}

id: 4021
event: event
data: {"id": "e4021", "camera": "273", "name": "Front Door", "kind": "person", "label": "person", "classes": ["person"], "score": 0.92, "at": 1792609200123, "phase": "new", "thumbnail": true, "recording": true}
```

`kind` is one of `person`, `vehicle`, `animal`, `package`, `doorbell`, `motion`, `other` (v1's six plus `package`; `face` and `people` map to `person`, `dog_cat` and `bird` to `animal`,
`package` to `package`); `label` is a name the recorder gave (a species, a person) else the class; `score` is left out when the detector gave none; `phase` is `new` the first time a tracked object is seen, `update` when the same object
gains a label or a score 0.1 better within 10 s of the first sighting (never a new pop-up), `end` when it leaves (not reported for 6 s, or the detector reported nothing for the camera). **`id` is the story's: the `new` event's id in all three
phases, and the id of its picture.** A doorbell press is a `new` only (a second within 2 s is the same press); a `motion` story is ended by its sensor. The bridge listens to `ObjectDetector`, `BinarySensor` and, only when asked in `kinds`,
`MotionSensor`. `GET /v1/event-thumbnail?id=e4021&w=&h=` is a JPEG taken when the event arrived (`getDetectionInput`, else a fresh still); the bridge keeps the last 100 for 30 minutes (`404 event_not_found` when it is unknown, expired or of an unticked camera, `502 upstream_error` when the camera gave none).

**Playback with limits.** `POST /v1/playback`, `/v1/clip-playback` and `/v1/playback/seek` accept the same `accept` object (a seek keeps the session's limits unless it names new ones); with it the bridge copies any recording or clip inside it
(H.264 or H.265) and encodes the rest to H.264 inside it (the rate thinned to `max_fps`), and the answer's `video` and `mode` say what the client gets; without it nothing changes (the Show's 1280x720 H.264). The SDP carries H.265 parameter
sets (`a=rtpmap:96 H265/90000`, `sprop-vps`, `sprop-sps`, `sprop-pps`). A session belongs to the token that made it.

**Devices.** The settings page gains a **Devices** list (name, added, last used, remove) and an "Add a device" form (name, SHA-256 verifier as 64 hexadecimal digits; at most 8); the existing "Make a token" stays for the Show.
`TokenStore` verifies a presented token against every verifier in constant time. Removing a device ends its streams at once (playback, pictures, events), as deleting the token does (`design.md` 3). Connections that showed a valid token: 16 per address,
32 in all; keep-alive only when the client sends `Connection: keep-alive`.

Also in v1.1: `package` in the class table (`mapping.ts`), `/v1/health` `device` (the name the token belongs to, `null` for the Show's), the errors `409 offline` and `404 event_not_found`, `selftest` lines for the six questions of 1.7. The finished text is
`docs/scrypted-bridge-api.md` section 12 of the bridge repo (branch `bridge-v1.1`).

### 3.4 The helper's contract to the shell (the contract between slices B and C)

The shell keeps its names where it can: the topics `cameras`, `cameras.popup`, `cameras.live` and the routes under `/v1/cameras`; the Home Assistant specifics go.

**Topics** (retained, as today):

```
cameras:        {"state": "unlinked|connecting|ready|offline|refused|untrusted", "reason": "words", "now_skew_ms": 12, "rev": 7,
                 "cameras": [{"id": "273", "name": "Front Door", "room": "Entry", "type": "doorbell", "online": true,
                              "live": true, "recordings": true, "events": true, "clips": true, "since": 1792500000000,
                              "native": {"codec": "h265", "width": 3840, "height": 2160, "fps": 15}, "prebuffered": true}]}
cameras.popup:  {"id": 41, "event": "e4021", "camera": "273", "name": "Front Door", "kind": "person", "label": "person", "at": 1792609200123,
                 "untilMs": 1792609212200, "seq": 4021, "thumbnail": true, "shownAt": 1792609200400, "overApps": true, "test": false}
cameras.live:   {"camera": "273", "mode": "live|playback|clip", "state": "idle|asking|starting|playing|failed|stopped", "reason": "words",
                 "codec": "h265", "width": 3840, "height": 2160, "fps": 15, "startedAt": 1792605903000, "positionMs": 1792605910250, "rate": 1,
                 "fed": 900, "dropped": 0, "lost": 0, "delayMs": 4, "window": "card", "paused": false, "since": 1792609200500}
cameras.events: {"events": [ ...the last 30 pushed events, newest first, as the bridge sent them... ]}
```

`positionMs` is the wall-clock moment of the picture on the plane in a playback (4 updates a second) and `startedAt` is the bridge's `started_at`; both are `null` in live. `paused` is true while a recording is held (the last picture stays,
nothing is dialled). The pop-up record says `thumbnail` (a picture of the moment is there), `shownAt` (when it was published), `overApps` (the owner's `popups.over_apps`, which the shell enforces) and `test` (a test event). Nothing in a topic, route answer or log carries the token,
a stream address or the bridge's address after the link is made.

**Routes** (loopback API, JSON, times in epoch milliseconds):

| Route | Does |
|---|---|
| `GET /v1/cameras` | the `cameras` topic's value |
| `GET /v1/cameras/{id}/frame?w=&h=` | the newest still as a BMP (hardware or libjpeg decode), `X-Luma-Camera-Age-Ms`, `X-Luma-Decoder`; the helper opens the bridge's stills stream for the camera while asked, closes it 3 s after the last ask; `404 no_picture` (none yet, or older than `cameras.frameHoldMs`), `503 not_connected`, `429 busy` |
| `GET /v1/cameras/{id}/recordings?from=&to=` | the bridge's ranges: `ranges`, `unknown`, `complete`, `step_ms` |
| `GET /v1/cameras/{id}/events?from=&to=&count=` | ticks of the span, oldest first: `at`, `kind`, `label`, `clip` |
| `GET /v1/cameras/{id}/clips?from=&to=&count=` | clips, newest first: `id`, `at`, `duration`, `kind`, `label`, `description`, `has_thumbnail`, `has_video` |
| `GET /v1/cameras/{id}/thumbnail?at=&w=&h=` and `/clip-thumbnail?clip=&w=&h=` | a picture of a recorded moment or a clip |
| `GET /v1/cameras/events/{event}/thumbnail?w=&h=` | the picture of a pushed event |
| `POST /v1/cameras/live {camera, quality?}` | starts live; the state is the `cameras.live` topic; one session, a new camera ends the old one |
| `POST /v1/cameras/playback {camera, at, rate?}` and `{camera, clip}` | starts a recording or a clip |
| `POST /v1/cameras/live/pause {paused}` | holds a recording (the picture stays, the dial ends) or plays it on from where it stopped |
| `POST /v1/cameras/live/stop` | stops live or playback (the pipeline unloads, the feeder ends after 120 s idle) |
| `POST /v1/cameras/popup/clear` | the pop-up is dismissed (every reader sees it go) |
| `POST /v1/cameras/event {camera, kind}` | a test event through the policy (off unless `cameras.testEvents` is on) |
| `GET` and `POST /v1/settings/scrypted-cameras`, `GET /v1/plugins/scrypted-cameras/choices/{cameras or kinds}`, `POST /v1/plugins/scrypted-cameras/actions/forget` | the Cameras page (3.5) and its Forget button, through the routes the Home Assistant page uses |

`GET /v1/cameras/{id}/frame` takes its `w` and `h` from the tile the shell draws, so no picture is scaled in the shell. A refusal is the bridge's `error.code` with its words
(`no_recording`, `camera_not_found`, `busy`, `timeout`, `not_supported`, `offline`), HTTP 409 on the start routes, and leaves live as it was. The bridge keeps one playback session for each camera and ends the old one as it takes
the call for another, so the end of a recording's stream while the next is asked for is not an end of recording; a refused seek then says why in `cameras.live` (`failed`). The end of a recording is `state: stopped`, reason "End of recording".

Answers are the bridge's own v1 JSON with times unchanged (`docs/scrypted-bridge-api.md` 5.3 to 5.5); the shell reads these three:

```json
{"camera": "273", "from": 1792605600000, "to": 1792609200000, "step_ms": 60000, "complete": false, "probed": 5,
 "ranges": [{"from": 1792605600000, "to": 1792607400000}, {"from": 1792607700000, "to": 1792609200000}], "unknown": [{"from": 1792607400000, "to": 1792607700000}]}
```

```json
{"camera": "273", "truncated": false, "events": [
  {"at": 1792605903000, "kind": "person", "label": "person", "classes": ["person"], "clip": {"id": "c1.WyIx...", "from": 1792605900000, "to": 1792605930000}},
  {"at": 1792606720000, "kind": "motion"}]}
```

```json
{"camera": "273", "truncated": false, "clips": [
  {"id": "c1.WyIx...", "at": 1792605900000, "duration": 30000, "kind": "person", "label": "person", "description": "Person at the front door", "has_thumbnail": true, "has_video": true}]}
```

**The pop-up.** `cameras.popup` is a record for the overlay system's card of class `camera` (`overlays-result.md`: 560 wide, two cards at most, held while a PIN, alert or keyboard is up); the Cameras
surface supplies the words and the picture and does no placing of its own (the card's place is the overlay layout's). OK opens the full view, Back dismisses, the arrows stay the app's (`takes` OK and
Back only while a pop-up is up); the card shows these two keys as glyphs (`assets/glyphs`), never as words. A doorbell stays half as long again, is never held back by the cooldown, and merges with the Home
Assistant plugin's doorbell record of the same ring (`popupShown(kind)`, as now). An `update` re-publishes the same `id` with new words, and the card up changes its words. The card shows the event's
thumbnail at once, then the camera's stills at 4 a second for as long as the card is up (the helper opens the stills stream for that camera only then).

**The full view and the strip** (Lucent TV rules: chrome over full-screen media fades after 2.5 s of no key and never blocks Back; the first key only reveals it; the way out never depends on it;
honest states; one button size per screen; no sentence of instruction, hints are glyphs; checked at 100 % and 200 % text). With the chrome up: Left and Right go to the previous and next camera, Up opens
the strip (Live, play, back 10 s, forward 10 s, previous and next event, rate, span, ticks, gaps, bubble), Left and Right there scrub (hold repeats, committed on release), OK commits the cursor or plays
and pauses, Down opens the events and clips sheet, Back goes Live, then closes. Cells the bridge calls `unknown` draw as recorded and dim; only a confirmed gap is hatched. Over a playing app the full view is
stills (1280x720, 8 a second) and the strip, sheet and playback are not offered unless C12 shows the feeder window plays over the app. The screens are the Rust shell's: `cameras-ui-spec.md` has their behaviour
and timings, `docs/mocks/cameras-scrypted/` their pictures.

### 3.5 Options and the event policy

The owner key is `scrypted-cameras`, drawn by the existing `settings/PluginRows.js` (it draws a read-only line, a switch, a number, a choice and an ordered pick; a secret or an address is never typed
there; the forget action is drawn as the Home Assistant page draws "Forget link"). One page, rebuilt at a higher `revision` when the camera list changes (the `notify` fields are generated from it).

```json
{"format": 1, "owner": "scrypted-cameras", "revision": 1, "title": "Cameras", "sections": [
  {"id": "link", "title": "Scrypted", "fields": [
    {"key": "link.status", "label": "Connection", "kind": {"type": "read-only", "value": {"capability": "plugin.scrypted-cameras.link"}}},
    {"key": "link.forget", "label": "Forget link", "kind": {"type": "action", "action": "forget", "confirmation": "destructive"}}]},
  {"id": "show", "title": "Cameras", "fields": [
    {"key": "show", "label": "Show cameras", "kind": {"type": "ordered-set", "options": {"provider": "cameras"}, "max_items": 32},
     "default": {"type": "ordered", "value": []}, "help": "Empty shows every camera Scrypted shares, in Scrypted's order.", "apply": "on-commit"}]},
  {"id": "popups", "title": "Pop-ups", "fields": [
    {"key": "popups.on", "label": "Event pop-ups", "kind": {"type": "bool"}, "default": {"type": "bool", "value": true}, "apply": "on-commit"},
    {"key": "popups.seconds", "label": "Stay for", "kind": {"type": "integer", "min": 5, "max": 60, "step": 1, "unit": "s"}, "default": {"type": "integer", "value": 12}, "apply": "on-commit"},
    {"key": "popups.cooldown", "label": "Same event again after", "kind": {"type": "integer", "min": 0, "max": 600, "step": 5, "unit": "s"}, "default": {"type": "integer", "value": 30}, "apply": "on-commit"},
    {"key": "popups.over_apps", "label": "Over playing apps", "kind": {"type": "bool"}, "default": {"type": "bool", "value": true}, "apply": "on-commit"}]},
  {"id": "notify", "title": "Notify for", "fields": [
    {"key": "notify.273", "label": "Front Door", "kind": {"type": "ordered-set", "options": {"provider": "kinds"}, "max_items": 8},
     "default": {"type": "ordered", "value": ["person", "doorbell"]}, "apply": "on-commit"}]},
  {"id": "video", "title": "Video", "fields": [
    {"key": "video.live", "label": "Live quality", "kind": {"type": "choice", "options": {"inline": [
       {"id": "native", "label": "Full"}, {"id": "balanced", "label": "Medium"}, {"id": "light", "label": "Low"}]}},
     "default": {"type": "choice", "value": "native"}, "apply": "on-commit"},
    {"key": "video.grid", "label": "Grid", "kind": {"type": "choice", "options": {"inline": [
       {"id": "light", "label": "Light"}, {"id": "balanced", "label": "Balanced"}, {"id": "smooth", "label": "Smooth"}]}},
     "default": {"type": "choice", "value": "balanced"}, "apply": "on-commit"}]}]}
```

The `notify` section has one `ordered-set` per camera (the page example shows one; 14 cameras make 14 of the 128 fields allowed). The provider `kinds` lists `person`, `vehicle`, `animal`, `package`,
`doorbell`, `motion` (the bridge's kinds; `other` never notifies). Defaults: person and doorbell for every camera; the rest opt-in (Nitin's cameras see birds and cats all day: `animal` on by default
would be a pop-up a minute).

**Policy** (a pure function in the shared crate, tested alone): an event pops up when the pop-ups are on, its kind is in its camera's `notify` list, `phase` is `new`, the camera and kind have not
popped within `popups.cooldown` (a doorbell ignores it), the shell is free (Home's `homeBusy`: a PIN, an alert, a panel, the AI prompt: it waits and shows the card if still within `popups.seconds`),
and, over a playing app, `popups.over_apps` is on. An `update` changes the words of a card already up. The event is also kept in `cameras.events` whatever the policy says.

An event older than 30 s (what a reconnect replays) is listed and never shown; an event of a camera the TV does not show is ignored; `popups.over_apps` (default on) is published as `overApps` in the pop-up record, and the
shell lets the card show over a playing app only when it is on.

### 3.6 The link and its secrets

`lumatv-helper scrypted-link` and `scrypted-unlink` are the only writers, as `ha-link` is (`ha_store.rs`): the secret never appears in an argument, a log, a topic or a route. The file is
`<data>/scrypted/link.json`, mode 0600 in a 0700 directory: `{address, pin, token, device}`; no settings route, plugin or key press can write it; Settings can only forget it.

1. `scrypted-link --brand $B --address 192.168.1.69:10444 --pin <fingerprint> --new` makes 32 random bytes on the TV, keeps them as the token (`lsb_` and base64url, the bridge's own shape), and prints
   only the **verifier** (the token's SHA-256, 64 hexadecimal digits) and the device name. The token never leaves the TV.
2. Nitin adds a device in the bridge's settings with that verifier. Only a hash and a fingerprint, neither a secret, cross between the two screens.
3. `scrypted-link --check` connects, prints `connected: N cameras` (or the refusal in words) and nothing else. Every command prints one JSON object (`code`, `message`, and the `verifier` after `--new`); the device name
   defaults to `LumaTV` (`--device`). While a link file exists the Cameras page shows the device name and the verifier (the facts `device` and `verifier` of `plugin.scrypted-cameras`, read-only rows `link.device` and `link.verifier`), so
   a verifier that was missed on the screen is read again without making a new token.

**Transport.** HTTPS to the bridge with the pin as the only trust (a SHA-256 of the one certificate; no chain, name or date), in the helper's own process: keep-alive, a push channel and a stills
stream need a connection that stays, which `fetch::lan`'s one `openssl s_client` child a request does not give. The helper has no in-process TLS today (`fetcher.rs`: `openssl` or `curl`
children). **`rustls` with a certificate verifier that compares the pin** is the way: the GFN engine, built for this target (`cargo zigbuild --target aarch64-unknown-linux-gnu.2.36`), already depends on
`rustls`, `ring` and `rustls-webpki` (`gfn/engine/Cargo.lock`), and the bridge's certificate was made for webpki (`CA:FALSE`, `serverAuth`; `design.md` 6 item 9). If it proves too heavy,
the fallback is the bridge's optional second door over Scrypted's plain `http://192.168.1.69:11080/endpoint/@lumashow/scrypted-bridge/public/v1/...`, the same token, a wider door that the consent text says
(`docs/scrypted-bridge-api.md` 9). The address must be a private one; the helper refuses any other. RTSP is plain TCP to a random-path address, as the Show plays it.

### 3.7 Honest states

`unlinked` (say how to link), `connecting`, `ready`, `offline` (the last list is kept, tiles show the last good still with its age and dim; never an invented picture), `refused` (the bridge refused the
token: "Scrypted refused this TV"), `untrusted` (the certificate is not the pinned one: nothing was sent), a camera `online: false`, a stream not prebuffered ("opens in a few seconds"), a recording gap
("No recording at that time", three seconds, nothing else changes). Every state is a word in `reason` that carries no address or token.

## 4. Migration

### 4.1 The Home Assistant camera path goes

Removed (clean cutover): in `helper/app/src/cameras.rs` the Home Assistant bridge, its `camera_events` handling and the signed-picture fetch (the pop-up policy and the frame cache are rewritten on the new
source); `live/relay.rs`, `live/hls.rs`, `live/fmp4.rs`, `live/net.rs` and the relay and HLS runs of `live/mod.rs` (`Player`, `Feeder` and the feeding loop stay and take the RTSP source); the helper's
`ha-bridge` dependency (only `cameras.rs` and `live/mod.rs` use it); `tests/cameras.rs`; from `CamerasConfig` the keys `enabled`, `port`, `relay` and `popupKinds` (the link is the switch; `popupSeconds` and
`cooldownSeconds` become the page's values); the shell's Home Assistant wording. In `lumashow-ha` the `tv-cameras` branch's `camera_events` and `camera_live` modules are never released: steps 1 to 5 of
`ha-cameras-result.md` are cancelled (no HACS update, no `ble-link` for the cameras' sake; the Bluetooth proxy keeps its own purpose). What stays: the Home Assistant plugin (lights, doorbell card),
`live/gf.rs` and `live/sps.rs`, the feeder, `images.rs`, `kit/CameraFrame.qml`, the overlay system and `CamerasConfig`' `frameHoldMs`, `maxPictureBytes`, `live` and the three feeder keys. The Show's Home
Assistant path is untouched.

No fallback: a Scrypted that is down has no cameras in Home Assistant either (the same cameras come from it), and a second source is a second set of faults; the Reolink cameras' own Home
Assistant entities stay in Home Assistant for what Nitin does there.

Config (`cameras` in `helper.json`): `frame_hold_ms` (a still older than this is no picture, 10000), `max_picture_bytes`, `live` (default true: the link is the switch; false keeps stills only), `feeder`, `feeder_window`,
`feeder_library`, `feeder_idle_seconds` (120), `test_events` (false). The keys `enabled`, `port`, `relay`, `popup_kinds`, `popup_seconds` and `cooldown_seconds` are gone; a `helper.json` that still has them (the TV's own is `{"cameras": {"enabled": true}}`) is read all the same: they are ignored and one log line says so.
`build.sh` needs zig for the target (ring's C); `.cargo/config.toml` turns UBSan off for that C.

### 4.2 Nitin's steps

Plain words, in order. Nothing runs by itself; each line is one thing to do.

1. **Install the bridge in Scrypted** (once; the bridge's README has the commands). One computer with Node 22, or Unraid's terminal with Docker: build `out/plugin.zip`, `npx scrypted login 192.168.1.69`
   (his Scrypted user name and password, typed at the prompt, kept on that computer), `npm run scrypted-deploy 192.168.1.69`. This opens a new listening port, 10444, on the home network only.
   Needs bridge v1.1 for the TV's live view, grid and pop-ups; v1.0 serves only the timeline.
2. In Scrypted, **Plugins, LumaShow bridge**: tick the cameras the TV may see (leave out private ones); copy the **certificate fingerprint** shown there. Press **Run the self-test** once.
3. On the TV, over SSH as root: `H=/var/lib/lumatv/bin/lumatv-helper; B=/media/developer/apps/usr/palm/applications/org.nphil.lumatv/brand.json`, then
   `$H scrypted-link --brand $B --address 192.168.1.69:10444 --pin <the fingerprint> --new`. It prints a **verifier** (a hash, not a secret).
4. In the bridge's settings, **Devices, Add a device**: name `LumaTV`, paste the verifier.
5. On the TV: `$H scrypted-link --brand $B --check`. It says `connected: N cameras`.
6. Settings, Home & look, **Cameras**: pick which events pop up for each camera (the default is person and doorbell).
7. Recommended in Scrypted: turn on **Prebuffered Streams** for the low-resolution stream of each camera in the grid, so the grid starts in under a second (otherwise up to 4 s).

To take access away: remove the device in the bridge's settings (the TV says "Scrypted refused this TV" and every stream it started ends at once), or Forget link on the TV, or untick a camera.

### 4.3 Creating LumaTV's Scrypted login

**There is none to create.** LumaTV does not log in to Scrypted. It holds one device token of the LumaShow bridge, which can read recordings, events, stills and streams of the cameras Nitin
ticked and nothing else. That is the least-privilege account: read-only by construction (no call to `removeVideoClips`, `deleteRecordingStream` or `setRecordingActive` exists in its code), an allow-list that
starts empty, home network only, a hashed token revocable in one click.

**No supported non-interactive way to make a Scrypted user exists:**

1. A user is made by `UsersService.addUser(username, password, aclId)` (`server/src/services/users.ts`), reached only through `systemManager.getComponent('users')`
   (`server/src/runtime.ts:360`), an admin session, or by the core plugin's "Scrypted User" device creator (`plugins/core/src/user.ts`, `UsersCore.createDevice`). The doors that need no existing
   credential are the first user of an empty server (`scrypted-server-main.ts:649-657`) and `SCRYPTED_ADMIN_USERNAME` at container creation (`:167-174`); the stock compose file sets neither it nor
   `SCRYPTED_ADMIN_TOKEN` (`install/docker/docker-compose.yml`), and setting one means recreating the container, which is Nitin's own act.
2. The database is LevelDB (`server/src/level.ts`) held open by the running server: a write from `docker exec` is impossible while it runs and unsupported.
3. The CLI and the client library log in with a typed password (`packages/cli/src/main.ts` `doLogin`, `readline-sync`). So no `docker exec scrypted ...` command, and so no HomeLabber
   `run_command`, can make the account without an admin password or token that Nitin types or holds.

**Why a Scrypted user would be the wrong tool.** A non-admin user with `Default Access` off and no interfaces holds no device, but may call any plugin's authenticated HTTP route that does not check
`aclId` (`plugin-http.ts:89`); the stock ACL page grants whole interfaces, and `VideoClips` includes `removeVideoClips`; NVR access needs the NVR Users extension and per-camera grants; and the
user's token never expires. The bridge's token is narrower on every count.

**What does need Nitin, and cannot be a command:** step 1 of 4.2 (installing a plugin is his act, with his own Scrypted login typed at a prompt, so it is not a `run_command` either) and step 4 (adding a
device in the bridge's settings). Nothing secret passes through any screen, chat, command line or agent: the TV makes its secret and shows a hash.

**If Nitin wants a Scrypted account anyway** (for something else): Management Console, Users, add user, name, password, Administrator off; open the user, turn Default Access off, leave Interfaces empty.
That account holds no device and is enough only to call a plugin's authenticated route. For an RPC client (nothing here uses one) the minimum would be Default Access on (the core and WebRTC engine.io
endpoints), per camera `<id>#VideoCamera`, `#Camera`, `#ObjectDetector`, `#MotionSensor`, `#BinarySensor`, `#VideoRecorder` and `#EventRecorder` (and `#VideoClips`, which carries the delete), and the NVR
Users extension with each camera ticked in the NVR's permissions; the ACL page grants by interface, never by method. It is not used by this design. The shortest path for LumaTV is steps 1, 2 and 4 of 4.2.

### 4.4 Principle 0

| Action | Who | Why |
|---|---|---|
| Install the bridge (a plugin and a new LAN port, 10444, plus random RTSP ports while a playback runs) | Nitin | installing an integration; a network entrance (home network only, never a forward, never public) |
| Tick cameras, add or remove a device, enable prebuffer | Nitin | accounts and camera settings |
| `scrypted-link` and `scrypted-unlink` | Nitin | writes a key; no agent, route or plugin can |
| The bridge's reading of Scrypted | the bridge | read-only: no delete, no setting, no user, no other device |
| Camera frames | the TV | in memory only, no file, cleared when the link is forgotten; no AI or MCP tool carries a picture, an address or a token |

The TV never holds a camera's own address or password, and holds no Scrypted account. Stream addresses are capabilities: never in a topic, route answer or log.

## 5. Build plan

Three slices, parallel from the contracts in 3.3 (A to B) and 3.4 (B to C). Each builds against a fake of its neighbour, and a fourth, later, is the Show's.

| Slice | What | Size | Needs |
|---|---|---|---|
| **A: bridge v1.1** (TypeScript, `lumashow-scrypted`) | modules `live`, `stills` (ffmpeg per camera, shared, ref-counted), `stream` (events, with the tracked-object rule and the 200-event ring), `event_thumbnails`, `devices` (`TokenStore` with many verifiers, the settings form); `cameras?scope=all` with `streams` and `prebuffered`; `mapping.ts` (`package`, `people`, `dog_cat`); `accept` limits and H.265 in `src/playback` and its SDP; selftest lines; `docs/scrypted-bridge-api.md` v1.1 | **M**: about 1,300 lines and 1,200 of tests | the contract 3.3; the unverified items of 1.7 are read by its self-test |
| **B: TV helper** (Rust, `helper/app/src/scrypted/`) | `link` (store and CLI), `client` (rustls with the pin, keep-alive, SSE and multipart readers), the bridge's request and reader code (an SDK-free copy of the Show's; slice D shares it later), `events` and `policy`, `stills`, `live` and `playback` (`rtsp-client` into the existing `Player` and `Feeder`), the routes and topics of 3.4, the options page and its providers, removal of 4.1, a fake bridge for tests | **L**: about 4,000 lines and 1,500 of tests | the contract 3.3 (fake) and the repository dependency on `rtsp-client`; `rustls` added to the helper |
| **C: TV shell** (the Rust shell; no QML) | the screens of `cameras-ui-spec.md` on the topics and routes of 3.4: pop-up content for the overlay card, grid, full view over the hole, strip and bubble, events and clips sheet, states of 3.7, the Settings page from the schema of 3.5; mocks in `docs/mocks/cameras-scrypted/` (done: every view, dark and light, 100 % and 200 % text) | **M to L** | the contract 3.4; Lucent TV (`skill://design-language`) |
| D: the Show (later, LumaShow) | the shared crate, `camera.event`, `live`; the mosaic for the wall | **S to M** | A deployed |

Order: A and B start together (B's fake bridge follows 3.3); C starts from 3.4. Integration needs A installed (Nitin's 4.2 steps 1 to 5). The slices own distinct files; Main edits the changelog.

**Risks, in order.** (1) The bridge has never run against the real NVR (1.7): its self-test fixtures come first and cost one run. (2) H.265 and sizes up to 4K through the bridge's RTSP ingest and server,
written for H.264 up to 1280x720: slice A's first task; if the NVR's recordings do not pass as they are, the bridge re-encodes to H.264 at the recording's size instead (quality kept, codec changed).
(3) `rustls` on the TV, its size and build: fallback the plain second door (3.6). (4) The feeder window over an app and a second NDL session (C12, C13) decide only the optional paths of 2.3 and 2.4.
(5) The stills' cost on Scrypted's host (C3, C4): lower the rate, or build the mosaic.

## 6. TV checks

Not run; needs Main's slot and Nitin's steps 1 to 5. Receiver muted through Home Assistant before any playback; stop at the first failure and send back what is named.

**Latency, glass to glass.** A millisecond clock (a page on a second screen redrawing at 60 Hz, or a counter on an LED strip) held in the camera's view; a phone at 1/250 s photographs the clock and the TV in
one frame; the difference of the two readings is the delay. Ten shots a case, median and worst. The software parts are logged apart so the budget can be attributed: `cameras.live.delayMs`
(arrival to the feeder's write), the feeder's stats file (queue, `playing_ms`), the stills' `X-Captured-At` against the TV's clock (both NTP; the bridge's `health.now` gives the skew). Events: the
event's `at` against the helper's `shownAt`, and one real walk past the door.

- **C1 Link.** Steps 3 to 5 of 4.2: `/v1/cameras` says `ready` and lists the cameras; a wrong pin is `untrusted` with nothing sent (the bridge's log shows no request); a removed device is `refused`.
- **C2 Self-test fixtures.** The bridge's self-test text saved: the answers to 1.7, the main and sub sizes of every camera, the prebuffered streams.
- **C3 Stills.** `GET /v1/cameras/{id}/frame?w=480&h=270` five times a second for 60 s on each camera: `X-Luma-Camera-Age-Ms`, fetch time, helper CPU, the bridge's cost a camera.
- **C4 Grid.** Eight tiles at Light, Balanced, Smooth: pictures a second a tile, the compositor's frame gate over the grid (`tools/lumatv` gate rows), Scrypted's CPU.
- **C5 Event to pop-up.** `POST /v1/cameras/event`, then a real person: detector time to the card; the card in its place (top right, from y 134), over Home and over Netflix, the film steady (the frame probe).
- **C6 Policy.** Person on and vehicle off on one camera: only the person pops up; two within 30 s are one card; a doorbell twice is two; a busy host holds the card.
- **C7 Quality.** The pop-up picture, the grid tile and the full still of one camera look right at their three sizes in both themes.
- **C8 Live start.** From Home, OK on a camera: time to the first live picture for the main stream of each camera, prebuffered and not (expected 0.6 to 1.0 s, and up to 4 s); `cameras.live` goes asking, starting, playing; `fed`, `dropped`, `lost`.
- **C9 Latency.** Glass to glass at `native`, five minutes a camera: expected 0.2 to 0.45 s; the largest main stream (4K H.265 if any) and a 1080p H.264 one. A camera's own on-screen clock shows seconds on the camera's clock, so it only checks the order of magnitude.
- **C10 Playback.** Release of the scrub knob to the first picture (expected 0.7 to 2.5 s), the picture's recording time against `started_at`, a gap ("No recording at that time"), 1x, 2x, 4x, a clip.
- **C11 Switch.** Left and Right between cameras of different codec and size: the gap (expected 0.5 to 0.9 s); the old session gone (the feeder ends within 2 minutes of Back; no socket left).
- **C12 Over an app.** With Netflix and then an HDMI input playing, open the full view: the stills path always; whether the feeder window `card`, `overlay` or `screensaver` plays live over the app and what Netflix does
  (this decides "Live video" in 2.4).
- **C13 Second decoder session.** Two feeders at once on Home (then over an app): whether the second loads, and the `adec` and `vdec` counts, CMA before, during and after (decides upgrade b of 2.3).
- **C14 Open and close.** Cameras from Home and from a pop-up: the panel in the first frame with the last tiles; Back to Home and to an app within one frame; live torn down off the UI path (the frame gate: no refresh lost).
- **C15 Cost.** Helper CPU and RSS idle (linked, no one watching), with the grid, with live; the link while the TV is in standby (it must close) and on wake (it reconnects).
- **C16 Hardware JPEG.** `lumatv-jpeg` against libjpeg for a 480x270 still: `decode_us`, CPU; the hardware block is used only if it wins.
- **C17 Failure.** Scrypted stopped: `offline`, the last tiles dimmed with their age, no invented picture; back: it reconnects by itself within 30 s. The bridge stopped, the pin wrong, a camera offline.
- **C18 Off.** `scrypted-unlink`: no connection, no file, the page says unlinked, no feeder ever started.

## Changelog line

Cameras through Scrypted: live video at the camera's own size from Scrypted's rebroadcast on the video plane, a grid and event pop-ups from stills, a timeline and events and clips list from
the NVR, event pop-ups chosen per camera, linked with a device token the TV makes itself (no Scrypted login); replaces the Home Assistant camera path.
