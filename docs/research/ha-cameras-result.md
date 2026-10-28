# Cameras and detection pop-ups through Home Assistant (HaCameras, 2026-10-28)

Built on the offline host and committed; nothing has run on the TV. The switch is `cameras.enabled` in the helper's `helper.json`, off by
default, until the TV check below passes.

## Resume here

- **What it is.** The TV is one more LumaShow device to Home Assistant's **LumaShow integration** (`lumashow-ha`, the one Nitin installed
  for the Shows): the integration dials the TV (the Show's own `ha-bridge` crate, TCP 6054, announced as `_lumashow._tcp`), encrypted with a
  key derived from the TV's ESPHome pairing key, which is the Bluetooth proxy's (`lumatv-helper ble-link`). The TV holds no token and no
  address. Home Assistant pushes the cameras Nitin shares with the TV (signed picture addresses) and, with *Send camera events* on, what their
  detectors see. The pop-up is a glass card in the top right corner over whatever plays; OK expands it to the full screen, Back dismisses it,
  it goes by itself; a grid shows every shared camera.
- **Pieces.** `lumashow-ha` branch `tv-cameras` (forge `nphilip89/lumashow-ha`, not GitHub, no tag): modules `camera_events` and
  `camera_live` (`custom_components/lumashow/camera_events.py`, `camera_live.py`, protocol `docs/protocol.md` 10b and 10c). TV helper:
  `helper/app/src/cameras.rs` (the bridge, the pop-up policy, the picture route), `helper/app/src/live/` (the stream on the video plane),
  `config.rs` `CamerasConfig`. Shell: `panels/Cameras.qml`, `kit/CameraFrame.qml`, the overlay, the bus and Home's key routing. Mocks
  `docs/mocks/cameras/`. Tests below.
- **The LumaShow checkout moved.** The helper's path dependencies now point at the worktree `/data/home/LumaShow/wt/lumatv` (branch
  `tv-cameras`, from `g2-0809d` b008b004, 2026-10-23), because `/data/home/LumaShow/lumashow` is a 10-09 branch that predates `ha-bridge`.
  Nothing of LumaShow's is edited there. The plugin API there is level 2; **this TV keeps speaking level 1** (`session::TV_LEVEL`): the helper
  negotiates with `negotiate_with(min, target, 1, 1)`, holds a plugin to the permissions level 1 grants, and the roster no longer marks a
  plugin inert for a lease or a permission of a later level (the decoder, widgets): a plugin whose `min` is 1 runs without them. The HA
  plugin is now LumaShow's 0.2.0 (`api 1..2`, class L, `lease:decoder` and widgets it does not get here): the host tests show it still makes
  the lane's cards, the tiles, the doorbell and the pickers at level 1 (`tests/qmlhost/fixtures/ha-plugin.json` regenerated: two more
  Doorbell rows, *Doorbell camera* and *Live video camera*, which pick nothing on the TV; its icons are now `base.icons/home`, drawn as
  `house`). Its manifest changed, so **its approval code is a new one**: `plugin-consent` prints it, and Nitin approves it again with
  `plugin-enable` (ha-plugin-result.md, Nitin's steps).

Open, in this order: Nitin's steps (below), the TV check list (below), then the video-plane stream path (section "Not built").

## Nitin's steps

1. **Home Assistant: the integration update.** The module is on the branch `tv-cameras` of `lumashow-ha`; until it is merged and released,
   the TV gets no events (it still gets the cameras and their pictures from the released 0.1.0, and the pop-ups cannot fire). When Main says
   the release is out: HACS › LumaShow › *Download*, restart Home Assistant.
2. **Pair the TV once** (if the Bluetooth proxy is not linked yet): on the TV, as root, `lumatv-helper ble-link` (it prints the key once), then in
   Home Assistant add the ESPHome device *LumaTV Bluetooth* with that key. The LumaShow integration finds the TV by itself a moment after the
   camera bridge is on and reuses that key: *Discovered: LumaTV*, Add, nothing typed.
3. **Turn the bridge on**: `cameras.enabled: true` in `/var/lib/lumatv/helper.json` and restart the helper (the TV round's step 1 does this).
4. **Pick what the TV gets**: the LumaTV entry › *Configure* › *Pick by type* › Cameras (or by entity: `camera.exterior_front_door_camera`,
   `camera.front_doorbell_camera_fluent`, `camera.back_door_camera`, ...), then *Notifications, cameras and alarms* › **Send camera events** on,
   Save. Which kinds pop up is the TV's `cameras.popupKinds` (person, vehicle, animal, doorbell by default; package and motion are off).

Decide: whether *Share camera addresses* (RTSP, may carry a password) is turned on for the TV later, for the video-plane stream path.

## What Home Assistant has (read 2026-10-27, read-only)

| | |
|---|---|
| Cameras | 14 `camera.*`: Reolink's three (`camera.front_doorbell_camera_fluent`, `camera.back_door_camera_fluent`, `camera.office_camera_fluent`: the sub streams, `supported_features` 2, a stream), and Scrypted's (`camera.exterior_front_door_camera`, `camera.back_door_camera`, `camera.bird_camera`, `camera.exterior_backyard_camera`, `camera.office_camera`, `camera.office_tool_room_camera`, `camera.plant_room_camera`, `camera.downstairs_door_camera`, `camera.gym_camera` unavailable, `camera.plant_room_cat_feeder`: `supported_features` 0, a picture, no stream through Home Assistant). The front door is the same Reolink doorbell twice (Reolink's entry and Scrypted's). |
| Detectors on the camera's device | Reolink: `binary_sensor.<camera>_{person,vehicle,pet,package,visitor,motion}` (`device_class` motion/occupancy). Scrypted: `binary_sensor.<camera>_{person,people,vehicle,animal,package,face}_detected`, `_motion`, and `event.<camera>_object_detected` (`event_types` package, people, vehicle, person, animal, face, dog_cat; `event_type` at each event, with `score` and `zones`), `event.<camera>_doorbell` (`ring`). |
| Detectors elsewhere | Kestrel's `event.kestrel_<camera>_animal` (`event_types` bird, mammal, unidentified; attributes `species` "Domestic Cat", `camera` "Front Door Camera"). |
| Pictures | `/api/camera_proxy/<entity>`; the integration signs it (`authSig`, an hour). Scrypted answers from its prebuffer; Reolink from the camera's snapshot API. |
| Streams | Reolink: RTSP with credentials (sent only with *Share camera addresses* on). Scrypted's cameras: none through Home Assistant (Scrypted's own RTSP rebroadcast is what the Shows' wall dials; not reachable through the integration today). go2rtc is Home Assistant's built-in one (`source: system`, loopback only). |

## The design

### Transport and pairing

The Show's `ha-bridge` crate, unchanged, inside the helper (`cameras.rs`): `Bridge::start` with `KeySource` = the Bluetooth proxy's link key
(`ble_store`, `<data>/ble/proxy.json`, read at every connection, so `ble-unlink` ends the session), `DeviceInfo` name `lumatv-<six hex of the
MAC>`, friendly name *LumaTV*, `_lumashow._tcp` TXT as the Show's (`mac`, `proto`, `modules` with `camera_events` added). Bound to the TV's
private LAN address, never `0.0.0.0`. With the switch off or no key: no thread work beyond a 10 s look, no socket, no announcement. The
helper's HA plugin lease still runs on its own token link (the Show feeds its lease from the bridge; the TV's lease has no `Upstream` seam
yet: a follow-up, so that the plugin's lights and doorbell card run token-free too).

### Events: the `camera_events` module (integration side, additive)

A handler-owned module like `alarms`, so no change to the Rust bridge or to any Show: offered in `hello` only when the device's entry has
*Send camera events* on (off by default) and only while the device registered a handler. For each shared camera the integration watches the
`binary_sensor` and `event` entities of the camera's own device whose name or class says what they detect, and `event` entities elsewhere whose
`camera` attribute is the camera's name (`classify`: person, vehicle, animal, package, doorbell, motion; an event entity that announces many
classes is read at each event). A binary sensor fires when it turns on, an event entity when it has a new moment; two of one camera and one
kind within 2 s are one event. The message: `{"m":"camera_events","t":"event","camera":"camera.x","kind":"person","at":<epoch s>,"source":"binary_sensor.x","label":"Domestic Cat"?}`;
`list` says which kinds each camera can announce; `refresh` asks for it again. The device sees no detector it could not already see, and needs
no registry. Why not the `states` module: the TV would need the device registry to pair a sensor with a camera, which the protocol never gives
it; mapping in Home Assistant keeps the rule that the integration decides.

### Pop-ups: the policy (helper) and the card (shell)

`cameras.rs` `Popups`: which kinds (`popupKinds`), a cooldown per camera and kind (`cooldownSeconds` 30; a doorbell is never held back and stays
half as long again), `popupSeconds` 12. An accepted event is the hub event `cameras.event` and the retained topic `cameras.popup` (`id`, `camera`,
`name`, `kind`, `label`, `at`, `untilMs`), so a shell that starts late still shows a pop-up that is in date. The shell (`panels/Cameras.qml`, a
Resident of LumaOverlay, the layer that is up over Home and over every app) shows the card when the topic is new and in date and the host is
free; it waits (and re-looks every 500 ms) while a panel, a banner with buttons, a PIN prompt, an alert, a confirmation or the AI prompt is up
(Home says so through `Bus.homeBusy`), and shows it when they go if it is still in date. Back: `POST /v1/cameras/popup/clear` (every reader
sees it go). OK: the full view. The mute mark keeps to the left corner while the card is up.

Keys: over an app the overlay takes OK and Back only while a pop-up is up (the D-pad goes on to the app) and every key while the viewer is;
over Home, `LumaHome.keyAction` gives the key to the surface through `Bus.cameraKey` before its own layers, when no modal layer is open.

### Pictures: the snapshot path (built) and the stream path (not built)

The pop-up's preview, the full view and the grid draw `GET /v1/cameras/{id}/frame?w=&h=`: the helper fetches the camera's signed picture address
(`ha_link::picture::fetch_signed`, the Show's own: home network only, no token, no redirect, a size cap), decodes it through the hardware JPEG
block (`images.rs`, `lumatv-jpeg`, a BMP of the size asked) where the TV has one, else serves the JPEG, holds the newest picture `frameHoldMs`
(300) so a grid of tiles asking at once costs one fetch a camera, and fetches nothing unless the shell asks. `kit/CameraFrame.qml` asks for the
next picture `intervalMs` after the last arrived (700 ms single, 1200 ms a tile), two hidden Images taking turns and one rounded quad showing
the newest (no layer, no flash). No picture is written anywhere; the newest of each camera is in memory until the next.

What this gives: 1 to 3 pictures a second a camera, every camera, over Netflix and over an input alike, no decoder, no contention, nothing of
the app's playback touched. Smooth motion is the live path's (below): the full view on the video plane, the pictures as its fallback.

### Latency budget (event to pop-up)

| Stage | Expected | Basis |
|---|---|---|
| Detector to Home Assistant state | 0.2 to 1.5 s | the camera's or Scrypted's own (Reolink AI, Scrypted's detector); [INFERENCE] not measured here |
| Home Assistant to the TV (`camera_events.event`) | under 50 ms | one state change callback, one encrypted frame on the LAN; the module has no debounce beyond the 2 s hold |
| Helper policy to hub to shell | under 50 ms | the long poll wakes on the hub's condvar; measured host end to end: event to `cameras.popup` **under 1 s with a wide margin** (`helper/app/tests/cameras.rs` asserts < 1 s; it runs in tens of ms) |
| Shell: card on screen | 1 frame, 180 ms motion | a Resident, built at warm-up; `panels/Cameras.qml` host test: topic to `shownAt` under 600 ms including the poll |
| First picture in the card | 0.3 to 1 s | one fetch: Scrypted ~0.1-0.3 s, Reolink ~0.3-0.7 s [INFERENCE], plus the decode (~33 ms 1080p hardware, measured in `hwjpeg-result.md`) |

So the card with its words is expected within about a second of the event (the detector's own delay apart), and its picture within another.

### Privacy and safety

No frame is stored; the helper keeps the newest picture per camera in memory and clears them when the session ends. No picture address, stream
address or key is in a topic or a log (the test searches the topic for `authSig`). The TV never calls a camera service (`NEVER_CALLED` keeps
`camera`, `lock`, `cover`, `valve`, alarms and sirens), and the integration decides what is shared. The bridge binds the LAN address only and
refuses a wrong key (`a_wrong_key_is_refused...`).

## The live path (built offline; `cameras.live`, off by default)

`helper/app/src/live/` (`mod.rs` the session, `hls.rs` the playlists, `fmp4.rs` the demuxer, `gf.rs` the feeder's wire, `net.rs` the fetch)
and the integration's `camera_live` module (lumashow-ha `tv-cameras`, protocol §10c).

- **The stream, without the camera's password.** The full view asks the helper (`POST /v1/cameras/live {camera}`); the helper asks the
  integration (`camera_live.start`); the integration asks Home Assistant's own stream integration (`camera.async_request_stream(hass, entity,
  "hls")`), which pulls the camera's RTSP itself and serves **low-latency HLS of fragmented MP4** at a tokened address on Home Assistant's port
  (`camera_live.started {url}`). The TV never sees the camera's address; the token opens that stream and nothing else, on the home network
  (`net.rs` holds the fetch to `AddressPolicy`, plain HTTP: Home Assistant's internal address). Home Assistant ends the stream when nothing
  fetches it; the helper also says `stop`.
- **Into the video plane.** The helper fetches the playlist from its live edge (the last independent part), blocks on the next part where
  the server allows it (`_HLS_msn`/`_HLS_part`), demuxes each piece into whole access units in Annex B with the parameter sets before every
  key picture, and feeds them over the GFN feeder's socket protocol (`GF_LOAD` H.264/HEVC + `GF_VIDEO`, decode times as the timestamps: they
  rise with every unit, which the pipeline insists on; a stream with B pictures is shown a reorder delay later) to a `lumatv-gfn-feeder` it
  starts for the session (`--serve <sock> -w <cameras.feederWindow> --once -a <brand>.cameras`). The feeder owns the window on the video plane
  and LG's NDL DirectMedia pipeline, proven to 4K120 by the GFN work. One session at a time; a new camera ends the old one; the viewer closing
  unloads the pipeline and ends the feeder (`--once`), so nothing runs when nothing is shown.
- **The shell.** `panels/Cameras.qml`: the full view asks for the stream when it opens and at every camera switch, keeps the pictures until the
  helper's `cameras.live` says `playing` for that camera, then draws nothing where the picture was (the hole: the layer is transparent, the
  video plane shows through, the chrome stays above) and stops asking for pictures; Back asks `stop`. Where the live path is off, unavailable
  or failed (`reason` in words) the pictures stay: the full view always shows something.
- **Latency.** Home Assistant's LL-HLS parts are 1 s (`PART-TARGET`), so the stream arrives about 1 to 2 parts behind the camera plus the
  decoder's own delay: **about 2 to 3 s glass to glass [INFERENCE: the TV measures it]**, and smooth at the camera's full rate. Start: the
  stream integration's first segment (one key interval of the camera, 1 to 4 s) plus the feeder's load (about 1 s on the GFN path). The
  pictures cover the wait. Sub-second would need WebRTC (go2rtc through Home Assistant's `camera/webrtc/offer`), a DTLS/SRTP client the helper
  does not have: a later step, measured against what LL-HLS gives.
- **The grid, two designs, decided on the TV.** (a) *Pictures* (built): six tiles at 1 to 3 pictures a second each, one fetch a camera a
  second, no decoder. (b) *Sub-streams*: each tile a stream of its own through the same path at the camera's sub-stream resolution; it needs
  as many decoder sessions as tiles (the Show measured 6 to 8 on its own decoder; LG's media pipeline is unknown: `adec.p1 Open=1` was one
  session in round 9) and a feeder per tile placing its window at the tile's rectangle (`NDL_DirectVideoSetArea` or the exported window's
  geometry, as the saver's player moves its picture). The TV check below says whether a second session is given at all; with one, the grid
  stays on pictures and the full view is the live one.
- **Config.** `cameras.live` (false), `cameras.feeder` (empty: `<data>/bin/lumatv-gfn-feeder`), `cameras.feederWindow` (`card`; `overlay`,
  `screensaver` are the alternatives the TV check tries over an app), `cameras.feederLibrary` (empty: the feeder's default NDL library).
- **Host proof.** `helper/app/tests/cameras.rs` `the_full_view_plays_the_stream_on_the_video_plane_through_the_feeder_and_stops_clean`: a fake
  Home Assistant stream server (the master and a media playlist that grows a segment every 400 ms, the fixture's init segment and three
  one-second segments of 320x180 H.264 with B pictures, `helper/app/tests/fixtures/hls/`, made with ffmpeg), the fake integration answering
  `camera_live.start`, and **the real host build of the feeder (`gfn/feeder/build/host`) with the fake NDL library**: `cameras.live` goes
  asking, starting, playing, stopped; the fake library's own count says `played=30 shown=30 dropped=0` with **0 VIOLATION** lines (the order
  and the rising timestamps the real library wants), unloaded on the way out; a stream that does not end stops on `POST /v1/cameras/live/stop`
  with the feeder gone, the socket removed and the integration told; a refusal arrives in words. Unit tests: the demuxer on the fixture (init,
  Annex B, decode and presentation times, errors in words), the playlists (plain and low-latency, the live edge, the blocking reload, the
  address join), the feeder's wire (the structs' sizes against the header, frames both ways), the fetch (plain, chunked, https refused).
  The skipped case prints `skipped: the host feeder is not built` where `gfn/feeder/build.sh` has not run.

## Not built (next)

WebRTC through go2rtc (sub-second), the grid's sub-stream design (b) if the TV gives several sessions, HTTPS for the live path's fetch (Home
Assistant's internal address here is plain HTTP), the feeder's staging by `tools/lumatv` (the GFN app's own copy is used in the TV check).

## For LumaShow (not edited there; the exact proposal)

1. The Show's camera wall and doorbell card can take the same `camera_events` module: a `ModuleHandler` registered with `HaBridge::register_module`
   (as alarm sync is) that turns an `event` into the ringing card and, with the wall, into the tile that lights up. The module is additive and
   handler-owned, so a Show without the handler is never sent it.
2. `cameras.list` could carry the camera's `device` (Home Assistant's device id) and `kinds` once the built-in module moves to version 2; until
   then `camera_events.list` says the kinds.
3. The TV's `kit/CameraFrame.qml` pattern (two hidden pictures, one rounded quad) is the GPU answer where there is no decoder; the Show has one and
   keeps its live layer.

## Tests (host, all green 2026-10-28)

- `lumashow-ha`: `tests/test_camera_events.py` (classify; sources of a shared camera and one event once, the hold, the session's end; nothing
  for a device that does not share; the detectors follow the shared set; through the real hub and the fake Show: offered only when on, `list`,
  an `event`, `refresh`) and `tests/test_camera_live.py` (a start is answered with Home Assistant's own stream address, refusals in words,
  not offered without the option, through the channel); 253 passed, 2 skipped; ruff clean.
- Helper: `helper/app/tests/cameras.rs` (off and unpaired nothing listens; a fake integration dials the bridge, shares a camera, sends an
  event: pop-up within the budget, the picture fetched once for a burst and again after the hold, the cooldown, a doorbell, a kind not wanted,
  clear, the event stream, `bye` takes the cameras away; a wrong key refused and strange events ignored; **the live path end to end with the
  real host feeder and the fake NDL**); `cameras::tests` (the policy, the message, the announcement); `live::*::tests` (the demuxer, the
  playlists, the wire, the fetch); `helper/build.sh test` (clippy, 500+ tests).
- Shell: `tests/test_camera_popup.py` (the real overlay over an app and over Home with the fake helper: a pop-up within a moment, pictures of
  the preview's own size, gone at `untilMs`; the D-pad passes through, Back tells the helper; OK to full, every key, the grid's focus and
  sizes, Back closes; Home's keys through the bus; a pop-up waits for a busy host; nothing fetches once down; the full view asks for the live
  stream, is a hole while it plays, asks again at a switch and stops on close); `tests/qmlhost/compile_all.py` 143 files, 0 failed.
- Mocks: `docs/mocks/cameras/` (`node render.mjs`: popup, doorbell, full, grid; dark and light; 100 % and 200 % text; its check finds no text
  overflowing or off the safe edge).

## TV check list (not run; needs Main's slot and Nitin's steps 1 to 4)

Receiver muted through Home Assistant where sound could play; stop at the first failure and send back what is named.

1. `cameras.enabled: true`, helper restarted: `curl -s 127.0.0.1:18731/v1/cameras` says `paired: true`, `state: "waiting"`, `port: 6054`;
   `log-helper` has `cameras: the bridge listens at <lan ip>:6054`; from the workstation `avahi-browse -rt _lumashow._tcp` shows `lumatv-<six>`
   with `modules` ending `camera_events`.
2. Home Assistant discovers *LumaTV* (LumaShow integration) and connects with the ESPHome key: `/v1/cameras` `state: "connected"`, the TV's
   device page in Home Assistant shows *Connected*; `curl .../v1/state/cameras` lists the shared cameras with their `kinds` and no address.
3. Pictures: `curl -s -D - -o /tmp/f.bmp '127.0.0.1:18731/v1/cameras/camera.exterior_front_door_camera/frame?w=640&h=360'`: 200,
   `X-Luma-Decoder: hardware` (or `file` where the decoder is not staged), `X-Luma-Camera-Age-Ms`; a second call within 300 ms is served
   from memory. Time ten calls: the fetch time per camera (Scrypted, Reolink).
4. Event to pop-up over Home: `curl -s -X POST 127.0.0.1:18731/v1/cameras/event -d '{"camera":"camera.exterior_front_door_camera","kind":"person"}'`
   at a known time; a capture 0.3 s later shows the card top right; the log line `[Cameras] pop-up ...` stamps it. Then a real one: walk to the
   front door; measure from the detector's `last_changed` in Home Assistant to the card (target: under 1 s after the state change).
5. Over Netflix (playing) and over an HDMI input: the same two; the film keeps playing without a stutter (the frame-rate probe running);
   Right/Left still go to the app while the card is up; OK expands to the full view, Back closes it, the app is where it was; the mute mark
   stays in the left corner while the card is up.
6. The full view and the grid: the pictures arrive (2 to 3 a second, `X-Luma-Camera-Age-Ms` small), the name pill and hints fade after 2.5 s
   and come back with a key; the grid's focus moves, OK opens a tile, Back closes; `GET /v1/images` shows the hardware decoder's count rising.
7. Cost: helper CPU and RSS idle (bridge up, no pop-up), during a pop-up, during the grid of six (expected: a few percent of a core for the
   fetches and decodes; the compositor's frame gate unchanged with the card up: run `tools/lumatv` gate rows `quick` over Netflix with a card up).
8. Nothing left: Back on the pop-up, 2 s later `/__frames`-style check: the helper logs no fetch, `pidof lumatv-jpeg` ends after 30 s idle,
   `/v1/cameras/popup` topic `null`.
9. Busy: with a PIN prompt or an alert up, fire a test event: no card until the prompt goes, then the card if still in date (12 s).
10. Cooldown: two test events of the same camera and kind within 30 s: one card; a doorbell twice: two.
11. Switch off (`cameras.enabled: false`, restart): nothing listens on 6054, no announcement, Home Assistant's LumaTV entry says disconnected.

The live path (after 1 to 6 pass; Home Assistant's `stream` integration must be able to pull the camera: Reolink's Fluent streams are the
ones to try first; Scrypted's cameras have no stream through Home Assistant):

12. The feeder beside the helper: `cp /media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn/lumatv-gfn-feeder /var/lib/lumatv/bin/`
    (or `cameras.feeder` naming that path); `cameras.live: true`, restart; `/v1/cameras` says `live: true`.
13. Start time: on Home, open a Reolink camera full screen from the grid; stamp the key; `/v1/state/cameras.live` goes asking → starting →
    playing; the picture on the video plane replaces the stills (the hole); record key-to-playing (expected 2 to 5 s: Home Assistant's first
    segment plus the load) and the first `fed`/`dropped` after 30 s (dropped 0; `queue` figures in the feeder's stats file under
    `/tmp/lumatv-cameras/`).
14. Glass to glass: a clock (phone stopwatch) held in front of the camera, photographed beside the TV: the difference is the latency
    (expected 2 to 3 s with LL-HLS parts of 1 s; record Home Assistant's `stream:` `ll_hls` setting as found).
15. Chrome above the hole: the name pill, the live dot and the hints draw over the video; they fade and come back; Left/Right switch (the
    old session ends, the new one starts: record the gap); Back closes: `pidof lumatv-gfn-feeder` empty within 2 s, `cameras.live` `stopped`,
    no socket left in `/tmp/lumatv-cameras/`, Home Assistant's stream ends by itself within its idle time.
16. Second decoder session: with Netflix playing (then an HDMI input), open the full view. Either the stream plays over the film (record which
    `feederWindow` shows: `card`, `overlay`, `screensaver`; whether Netflix keeps playing; `adec` ports) or the load fails in words
    (`cameras.live` `failed`, the pictures stay): that answer decides the grid's design (b) and whether the live full view is Home-only.
17. Cost: CPU of the helper and the feeder while playing (expected: the feeder a few percent as in the GFN runs, the helper's fetch and demux
    under 5 % of a core), `CmaFree`/`MemAvailable` before, during and after three sessions; the compositor's frame gate with the hole up.
18. Off: `cameras.live: false`: the full view is pictures again; no feeder is ever started.

## Changelog line

Cameras through Home Assistant's LumaShow integration: event pop-ups (person, vehicle, animal, doorbell) in the top right over whatever plays, with
a live preview, OK to a full view and a grid of the shared cameras; the TV pairs as one more LumaShow device and holds no token. Off by default
(`cameras.enabled`) until the TV check passes.
