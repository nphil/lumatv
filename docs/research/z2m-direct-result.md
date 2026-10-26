# Ambient light: Hue bulbs through Zigbee2MQTT straight from the TV (no Bifrost)

## Resume here

Question (Nitin, 2026-10-25): his Hue bulbs are paired to Zigbee2MQTT 2.14.2 (a Home Assistant app) and there is no Hue
bridge. Instead of installing Bifrost (a Hue bridge emulator that drives the bulbs through Zigbee2MQTT in Zigbee
entertainment mode) and streaming HueStream to it, can the TV do what Bifrost does, as an output of the helper's ambient
engine, without costing much?

Verdict: yes, and the TV's side is smaller than the HueStream path it replaces. What Bifrost sends to Zigbee2MQTT for a
frame is one JSON message of about 300 bytes (a `zclcommand` to one bulb, carrying every bulb's colour; the bulb
rebroadcasts it over Zigbee). The TV can publish that message itself over MQTT, 20 times a second, for a few microseconds
of CPU a frame and under 100 KB of memory. Everything else Bifrost does (pretending to be a bridge: HTTPS, the Hue app's
API, a certificate, mDNS, a DTLS server) exists only so that Hue clients can talk to it, and the TV does not need any of
it. The part that costs something, Zigbee2MQTT turning 20 messages a second into Zigbee frames, is the same whichever
program sends them. Findings with file and line citations are under "How Bifrost drives the bulbs"; the build is under
"What was built"; the steps and the check list for the set are at the end.

Status: built and tested offline against a fake MQTT broker (no traffic to Nitin's broker, Zigbee2MQTT or lights was
sent, and no credential was read or made); nothing has run on the TV or the lights. The frames the helper sends are
byte-identical to what Bifrost's own code packs for the same colours (Bifrost's `hue` crate was built and run here to
produce the expected values: `helper/app/tests/fixtures/bifrost-vectors.jsonl`, generator in "Ground truth"). Gate on
2026-10-25: `cargo fmt --check`, `cargo clippy -p lumatv-helper --all-targets -D warnings` and `cargo test -p
lumatv-helper --no-fail-fast` all green (the aarch64 release build is the main gate's, `helper/build.sh check`).

## How Bifrost drives the bulbs

Source: `github.com/chrivers/bifrost`, `master` at `12d9e37` (2025-06-02, the newest commit on 2026-10-25; the `dev`
branch is older). Zigbee2MQTT `2.14.2` (`5c0c1c6`), zigbee-herdsman-converters `v26.115.1` (`9dabbe4`) and
zigbee-herdsman `v10.10.0` (`3cb89ad`) are the versions 2.14.2 pins (`package.json:60-61`).

### The path from a Hue client to a bulb

1. A Hue client (Hue Sync, the helper's `hue.rs`) opens a DTLS stream to Bifrost's UDP 2100 and sends HueStream packets.
   Bifrost reads them (`src/server/entertainment.rs:194-258`): the first packet names the area; every packet becomes a
   `BackendRequest::EntertainmentFrame`. The receive buffer is deliberately 512 bytes so that packets are dropped rather
   than queued when the backend cannot keep up (`:283-290`).
2. The z2m backend throttles to `streaming_fps` (default 20, `src/backend/z2m/mod.rs:93,102`; `backend_event.rs:434-437`:
   a frame that arrives before the interval has passed is dropped, not delayed) and packs one Zigbee frame
   (`src/backend/z2m/entertainment.rs:132-141`).
3. The frame is sent to Zigbee2MQTT over its **frontend WebSocket** (`ws://<z2m>:8080`, `src/backend/z2m/mod.rs:165-170`,
   `websocket.rs:27-73`) as the JSON `{"topic": "<friendly name>/set", "payload": {...}}`. Zigbee2MQTT's frontend
   turns every such message into exactly what an MQTT publish to `<base_topic>/<friendly name>/set` is
   (`lib/extension/frontend.ts:163-168`: `this.mqtt.onMessage(`${base}/${topic}`, payload)`), so an MQTT client that
   publishes the same payload to `zigbee2mqtt/<friendly name>/set` is indistinguishable from Bifrost to Zigbee2MQTT.
4. Zigbee2MQTT's `publish` extension finds the converter for the key `zclcommand` (`lib/extension/publish.ts:91-280`),
   which is zigbee-herdsman-converters' generic `zcl_command` (`src/converters/toZigbee.ts:255-275`), added to every
   device definition (`src/index.ts:423-437`). It calls herdsman's `Endpoint.zclCommand(cluster, command, payload,
   options, ..., frametype)` (`src/controller/model/endpoint.ts:1154-1209`), which looks the command up in the cluster
   definition, builds the ZCL frame and sends it as a unicast to that device with the APS ack the adapter waits for
   (`options.timeout`, Bifrost sets 100 ms, bounds that wait).
5. Cluster `0xFC01` is zigbee-herdsman-converters' custom cluster `manuSpecificPhilips3` (manufacturer code `0x100B`,
   Signify), with commands 1, 2, 3, 4 and 7 that each take one `data` buffer
   (`src/lib/philips.ts:296-311`); `philipsModernExtend.light(...)` adds it to every Hue light definition
   (`src/lib/philips.ts:757-760`), so every Hue bulb Zigbee2MQTT supports accepts these commands. The byte array in the
   JSON is written as is (`buffaloZcl.ts:972-975`, `buffalo.ts:235-243`).

### The frame (cluster 0xFC01, command 1)

`crates/hue/src/zigbee/stream.rs:65-71,142-156` and `entertainment.rs:36-69,200-234`:

```text
counter    u32 LE   frame counter, +1 per frame, continues across stop/start
smoothing  u16 LE   fade time per frame: 0xFFFF = 2.56 s (SMOOTHING_MAX_MICROS, "determined experimentally"); Hue clients use 0x0400;
                    Bifrost sets it to the frame interval (backend_event.rs:412-417, stream.rs:100-111): 20 fps -> 50 ms -> 0x0500 = 1280
then 7 bytes per light (record):
  addr       u16 LE   the bulb's Zigbee network (short) address; for a gradient strip, nwk + segment index (a virtual address set by command 7)
  field      u16 LE   (brightness << 5) | mode; brightness 11 bits, 1..=2047; mode 0b01011 (0x0B) for a bulb, 0b00000 for a strip segment
  raw        3 bytes  x and y quantised to 12 bits each (xy.rs:108-119): x12 = x * 4095 / 0.7347, y12 = y * 4095 / 0.8264 (lib.rs:30-31);
                      bytes: x12 & 0xFF, (x12 >> 8) | ((y12 & 0xF) << 4), y12 >> 4
```

Colour (`src/backend/z2m/entertainment.rs:73-83`): the HueStream 16-bit RGB is cut to 8 bits by `/256`
(`crates/hue/src/stream.rs:215-221`), converted by `XY::from_rgb` (`xy.rs:33-50`): linear, no gamma, through the
wide-gamut matrix (`colorspace.rs:132-146,170-182`: `X = 0.7164 r + 0.1010 g + 0.1468 b`, `Y = 0.2587 r + 0.7247 g +
0.0166 b`, `Z = 0.0512 g + 0.7740 b`), `x = X / (X+Y+Z)`, `y = Y / (X+Y+Z)`; brightness is `Y` divided by the largest
`Y` that still fits the gamut at that (x, y) (`find_maximum_y`, `colorspace.rs:150-159`: ten rounds of dividing by the
largest RGB component), times 255, capped at 255; then `(bright / 255 * 2047).clamp(1, 2047)`. Black gives `0/0` and
falls to the D50 white point at brightness 1 (`xy.rs:45-49`), so "off" in a frame is the dimmest level, not zero.

Where it goes: Bifrost resolves every channel of the area to a device and address (`backend_event.rs:374-405`:
`addrs[friendly_name] = [nwk + member.index, ...]`), then sends **every frame to one device only, the first channel's**
(`backend_event.rs:409-410`, `entertainment.rs:140`: `send_entertainment_frame(&self.target, ...)`). The frame carries
the records of all the lights; the bulb that receives it repeats it in broadcast mode for the others
(`doc/hue-zigbee-clusters.md:59-60`). One Zigbee message per frame, however many bulbs. That other bulbs do get their
records is shown by PR #160/#161 (closed, not merged): several bulbs in one area each lit up, with the colours assigned
in the wrong order, because Bifrost maps channel N to the Nth address in friendly-name order (`entertainment.rs:36-52,
78`: `self.modes[channel % len]`) rather than by membership. For the helper this does not matter: it builds the records
from its own light list.

Frame size: `6 + 7 n` bytes of ZCL payload plus a 5-byte ZCL header. Hue's own frames carry up to 10 lights
(`doc/hue-zigbee-clusters.md:202-203`); PR #165 (closed, a personal fork) reports `MESSAGE_TOO_LONG` on an Ember
coordinator with two 7-segment strips in one area (14 records = 104 bytes). The helper caps a frame at 10 records.

### Setting up and tearing down entertainment mode

Start (`src/backend/z2m/entertainment.rs:102-120`), once per stream, per device in friendly-name order:

1. `{"write": {"cluster": 64513, "payload": {"5": {"manufacturerCode": 4107, "type": 32, "value": 254}}}}` to
   `<name>/set`: attribute 5 of 0xFC01, the "light balance" for entertainment mode, set to 0xFE = full
   (`entertainment.rs:55-66`; `doc/hue-zigbee-clusters.md:268-279`: 1..0xFE, fails on bulb firmware 1.76.11, works on
   1.122.2). In Zigbee2MQTT 2.14.2 this reaches herdsman's `Endpoint.write` (`endpoint.ts:493-514`) with the cluster's own
   manufacturer code; the `manufacturerCode` inside the attribute object is ignored, harmlessly.
2. For a device with more than one segment: command 7 with `count u16 BE, then addr u16 LE each`
   (`entertainment.rs:109-114`; `crates/hue/src/zigbee/entertainment.rs:151-160`; `doc:200-257`).
3. Then `stop_stream` (`entertainment.rs:117`, yes, at the end of start): command 3 `[0x00, 0x01, counter u32 LE]` to
   every device (`:122-130`; `stream.rs:128-140`), "synchronize entertainment zone": it tells the bulbs the counter the
   frames will continue from (`doc:140-151`).

Stop (`backend_event.rs:443-469`, after the DTLS stream ends, `server/entertainment.rs:304-316`): the same command 3 to
every device, and the counter is kept for the next start. Bifrost does not restore the bulbs' state; what a bulb shows
after command 3 (its state from before the stream, or the last frame) is not documented in Bifrost and not verified
here. The helper sends a black frame twice before the command, as the Hue path does, and then puts each bulb back to the
state Zigbee2MQTT last reported for it (see "What was built").

Rate: 20 frames a second by default; Bifrost's reference says 10, 20 or 30 fps look good for 30 fps content, 60 worse,
and even 5 acceptable (`doc/config-reference.md:116-146`). Field reports: streams that stop on their own are the Zigbee
network not keeping up, the advice is a lower `streaming_fps` or a better coordinator (issue #155, comments of
2026-01-15). One frame is lost per stop on Bifrost's side (the first is used only to find the area,
`server/entertainment.rs:201-206`).

Zigbee2MQTT requirements (`src/backend/z2m/zclcommand.rs:146-154`; PR #104): zigbee-herdsman-converters 23.0.0 or later,
i.e. Zigbee2MQTT 2.1.1, with 2.1.3 recommended. 2.14.2 is far past that. Nothing else is needed from it: no external
converter, no option, no group. The MQTT broker it uses is enough; the frontend WebSocket Bifrost uses is not needed.

The bulb list: `zigbee2mqtt/bridge/devices` is published retained (`lib/extension/bridge.ts:889`) with
`friendly_name`, `ieee_address`, `network_address`, `definition.vendor/model` and each endpoint's input clusters
(`:829-888`), and republished whenever a device joins, leaves, is renamed or changes its network address
(`:110-181`). A subscriber gets the current list at once and every change after, which is how the helper keeps the
addresses in its frames right without ever asking.

### Ground truth

Bifrost's `hue` crate (`12d9e37`, with chrono's `serde` feature switched on in its `Cargo.toml` so the crate builds
alone) was run with this example; its output is `helper/app/tests/fixtures/bifrost-vectors.jsonl` and the helper's
unit tests compare their records, frames, reset and segment map to it byte for byte.

```rust
// crates/hue/examples/vectors.rs; the per-light lines are src/backend/z2m/entertainment.rs:73-83 verbatim.
let (xy, bright) = Rgb16 { r, g, b }.to_xy();
let brightness = (bright / 255.0 * 2047.0).clamp(1.0, 2047.0) as u16;
let raw = xy.to_quant();
HueEntFrameLightRecord::new(addr, brightness, LightRecordMode::Device, raw).pack()
// plus EntertainmentZigbeeStream::new(counter).frame(records), .reset(), HueEntSegmentConfig::new(addrs).pack(),
// and duration_to_smoothing(1 s / fps) for 10, 20, 25, 30, 50 and 60.
```

## Feasibility on the TV

The TV's part is an MQTT client that publishes one small message per frame and keeps a subscription to one retained
topic. For `n` bulbs a frame is `6 + 7n` data bytes written as decimal numbers in JSON: 4 bulbs make a 297-byte payload
(measured), 20 a second is 6 KB/s, 25 a second 7.4 KB/s. Building one (the matrix and the ten gamut rounds per light,
the packing, the JSON) costs 8.2 us on this x86-64 workstation in a release build (`cargo test --release`, 20 000
frames of 4 bulbs); on the set's A76 two to three times that, 20-25 us, plus one `write` on a TCP socket [INFERENCE: the
set was not measured]. At 25 a second that is under 0.1 % of a core, less than the HueStream path, which also encrypts
each packet (AES-GCM) and keeps a DTLS session. Memory: one TCP socket, a receive buffer that grows to the size of the
retained device list (a few KB per device; 100 devices about 300 KB, read once per connection) and the light table; no
TLS state. The thread sleeps between frames, exactly like the Hue output thread, and a still picture is sent once a
second, not at the frame rate (an unchanged frame is not resent early).

Latency, TV to bulb: MQTT publish on the LAN to the broker and on to Zigbee2MQTT, 1-3 ms; Zigbee2MQTT and the
coordinator, about 5-15 ms; one Zigbee hop plus the receiving bulb's rebroadcast, 5-10 ms; then the bulb's own fade of
one frame interval (50 ms at 20 fps, by Bifrost's smoothing trick). Against the Bifrost route the TV saves one hop
(DTLS to Bifrost, Bifrost to the WebSocket) and one process; the Zigbee part, which dominates, is the same. End to end
the same picture-to-light figure as the helper's Hue path is expected, minus a few milliseconds [INFERENCE: not
measured].

What Bifrost does that the TV does not need: the Hue bridge emulation (HTTP on 80, HTTPS on 443 with a certificate tied to
a MAC address, the CLIP v2 resource model, the v1 API, the event stream), mDNS/SSDP/UPnP discovery, the DTLS server, the
entertainment-area model the Hue app edits, scenes, groups and the whole device import, `state.yaml`. The TV needs: the
MQTT client, the bulb list from `bridge/devices`, the frame encoder, the start and stop sequences, and Nitin's list of
which bulbs sit where.

What stays the same whichever program sends: Zigbee2MQTT's work per frame (JSON parse, converter, ZCL frame, serial to
the coordinator, waiting for the bulb's ack with a 100 ms cap) and the Zigbee mesh. Bifrost's users run this at 20 fps
day in, day out; the limits reported are the mesh (issue #155) and frame size (PR #165), both of which apply to the TV
identically. The `publish` extension logs every failed `zclcommand` (`publish.ts:263-268`): a bulb that misses acks
makes Zigbee2MQTT's log noisy at 20 lines a second, as it would under Bifrost.

Decision: build the direct output. Bifrost stays the fallback for anything that wants a Hue bridge (the Hue app, Hue Sync
on a phone), and the helper's Hue path still works against it; nothing is removed.

## What was built

See "Resume here" for status. Modules in `helper/app/src/`:

- `ambient_z2m_frame.rs`: the encoder. `record(addr, [r, g, b] 16-bit, mode)` reproduces Bifrost's conversion
  (`f64`, the same matrix, the same ten rounds, the same casts), `frame(counter, smoothing, records)`, `reset(counter)`,
  `segment_map(addrs)`, `smoothing_for(interval)`, and the JSON payloads `zclcommand(command, data)` and
  `light_balance(0xFE)` in exactly Bifrost's shape (`zclcommand.rs:157-172`, `entertainment.rs:56-65`). Unit tests
  check every vector of `bifrost-vectors.jsonl` and Bifrost's own unit-test values (`2211ebffaabbcc`, the frame of
  `hue_ent_frame`, the segment map `00 04 01 00 ...`, the reset `00 01 22 11 00 00`).
- `ambient_z2m_mqtt.rs`: a small MQTT 3.1.1 client on `std::net::TcpStream` (`#![forbid(unsafe_code)]` as the rest):
  CONNECT with clean session, user name and password, keep-alive; CONNACK return codes in words that never carry the
  password; PUBLISH QoS 0 (a frame that is late is worse than a frame that is lost, which is also why Bifrost drops
  rather than queues); SUBSCRIBE/SUBACK; PINGREQ/PINGRESP; an incremental parser that takes PUBLISHes of any size
  (the retained device list can be hundreds of KB) from partial reads; DISCONNECT. No third-party crate.
- `ambient_z2m_store.rs`: `<data dir>/ambient/z2m.json`, mode 0600: broker address (a private IPv4 address), port, user
  name, password (`Secret`, `Debug` says `<redacted>`), base topic, frames a second, and the lights: Zigbee2MQTT friendly
  name, where it sits (picture coordinates, from a word or `x,y`), segments. Written by `lumatv-helper z2m-link` (the
  password on standard input, never an argument), removed by `z2m-unlink`; nothing else writes it.
- `ambient_z2m.rs`: the output thread, the twin of `hue.rs` with the same `Demand`/`Status`/`Event` types. The lights
  (one engine light per bulb, or per segment of a strip, placed from the saved link) are told to ambient light as soon
  as the thread starts. `Stream`: connect (CONNECT, then `bridge/devices` and each light's state topic subscribed to),
  wait for the retained device list (5 s at most), resolve the names to network addresses, remember each bulb's last
  reported state, send the start sequence (light balance to each, a segment map for a strip, then command 3 with the
  counter to each), then the latest frame at most `fps` times a second and the same frame again after a second of
  silence, to the first light's topic, with the smoothing set to the interval; the connection is read every 250 ms, so
  a bulb's new network address in a later `bridge/devices` is in the next frame. `Off`: black twice, command 3 to each
  bulb, then each bulb's remembered state back (`{"state": "OFF"}`, or `ON` with its brightness and the colour of its
  colour mode) so the room is as it was; then the connection is closed. `Discover` (asked only if the lights were
  never told): connect, read the list, check the names, disconnect. A lost connection, a refused publish or an
  unknown name retries after 5 s, then 10, up to 60, with the words in the `ambient` topic's `problem` prefixed
  `Zigbee2MQTT:`; a new connection always waits for a fresh device list. `probe(link)` does the connect-and-resolve
  once for `z2m-link`. The frame counter starts at 0 when the helper starts and runs on across streams, as Bifrost's.
- `ambient.rs`: the entertainment output is now either the Hue bridge or Zigbee2MQTT (`Ent`), chosen by which link exists;
  with both saved, Zigbee2MQTT is used and the topic's `problem` says so. Topic fields `z2m` and `z2mState` next to `hue`
  and `hueState`. `cli.rs`: `z2m-link`, `z2m-unlink`; `api.rs`: `POST /v1/ambient/refresh` reloads both links.

Tests (`helper/app/tests/z2m.rs`, a fake broker in the test itself on 127.0.0.1, 7 tests, all passing on 2026-10-25
with the rest of the helper's suite: 335 unit tests, and the hue, daemon and other integration tests):

- `the_still_colour_starts_the_bulbs_with_bifrosts_frames_and_the_screen_going_off_puts_them_back`: the saved user name
  and password in CONNECT, the three subscriptions, `write` of attribute 5 then command 3 with counter 0 to each bulb,
  frames to the first bulb only with both bulbs' records byte-identical to `ambient_z2m_frame::record` (which the unit
  tests hold to Bifrost's bytes), smoothing 1280, counters 0, 1, 2, ..., the whole `zclcommand` payload equal to
  Bifrost's shape, a still colour resent 1-3 times in 2.3 s, nothing published to any other topic; screen off: two black
  frames, command 3 with the running counter, `{"state":"OFF"}` to the bulb that was off and `{"state":"ON",
  "brightness":120,"color_temp":366}` to the one that was on, state `idle`; screen on: the start sequence again with the
  counter carried on; the helper's exit: command 3 and the restore again; the password in no topic and no log line.
- `the_pictures_colours_reach_each_bulb_by_where_it_sits_at_the_frame_rate`: frames on the capture socket with red on
  the left and blue on the right put a red xy on the left bulb's record and a blue one on the right bulb's, at 12-22
  frames a second against the engine's 30 (the link's 20); when the picture stops the lights fade, black is sent and the
  stream ends with command 3 and the restore.
- `a_bulb_zigbee2mqtt_does_not_know_is_an_error_until_the_list_names_it`: `Zigbee2MQTT: ... no device named "TV right";
  its Hue lights are: TV left`, nothing published; the list republished with the bulb: streaming on the next try.
- `a_bulb_that_rejoined_with_a_new_address_gets_it_in_the_next_frames`: a pushed `bridge/devices` with a new address:
  the next frames carry it, on the same connection, with no new start.
- `a_broker_that_refuses_the_password_is_reported_in_words_that_do_not_carry_it`: CONNACK 4 is "bad user name or
  password"; nothing subscribed or published; the password in no topic and no log.
- `a_dropped_connection_is_reported_and_the_stream_started_again`: every client dropped: "the stream ended", then a
  second connection, the start sequence with the counter the next frame carries, counters only ever going up.
- `z2m_link_checks_the_lights_against_the_device_list_and_saves_the_password_privately`: an unknown light is refused
  with the Hue lights the list has and nothing is saved; the right lights are saved (mode 0600, the password in the
  file and in no output), the credentials were used, nothing was published; a refused password is the broker's words;
  `z2m-unlink`; the usage errors (no light, a bad place, a public address).

## Nitin's steps

Keys are made and entered by Nitin only; nothing here reads Home Assistant, Zigbee2MQTT or the broker on his behalf.
`H=/var/lib/lumatv/bin/lumatv-helper`, on the TV as root over the Homebrew Channel's ssh.

1. In Home Assistant, Settings, Apps, Mosquitto broker (or whichever broker Zigbee2MQTT uses): the broker's LAN address
   is the Home Assistant host (`192.168.1.146`), port 1883. Make a user for the TV: a Home Assistant user `lumatv`
   (Settings, People, Users; "can only log in from the local network" is fine; Mosquitto accepts Home Assistant users),
   or a `logins:` entry in the Mosquitto app's options. Write the password down for the next step; it is typed once.
2. In Zigbee2MQTT, note the friendly names of the bulbs that should follow the TV (Devices page), and for each where it
   stands seen from the sofa: `left`, `right`, `top`, `bottom`, `top-left`, `top-right`, `bottom-left`, `bottom-right`,
   `behind` (the centre), or `x,y` in picture coordinates (0,0 top left of the picture, 1,1 bottom right; outside the
   range is beside the set).
3. On the TV, with the password on standard input (it never goes on the command line):
   `printf '%s' '<password>' | $H z2m-link --data-dir /var/lib/lumatv --host 192.168.1.146 --user lumatv --light 'Hue TV left=left' --light 'Hue TV right=right'`
   (`--port`, `--base-topic` if not `zigbee2mqtt`, `--fps` 1..60, default 20; a gradient strip is `<name>=<place>:<segments>`).
   The first `--light` is the bulb every frame is sent to (it repeats them to the others), so name the one nearest the
   coordinator or the most reliable first. The command connects, reads the device list, checks that every named light is
   a device Zigbee2MQTT knows (it prints the Hue lights it found if one is missing, so names can be copied), saves the
   link with mode 0600 and tells a running helper. It sends nothing to any light. The command checks the names only, not
   that the device is a Hue light: for any other bulb Zigbee2MQTT has no command 1 on cluster 0xFC01 and logs
   `Publish 'set' 'zclcommand' ... failed` for every frame (herdsman `getClusterCommand`, `UNSUP_COMMAND`).
4. Switch ambient light on in Settings, Ambient light. `$H z2m-unlink --data-dir /var/lib/lumatv` removes the link and the
   password. The Hue link (`hue-link`) is unaffected; if both exist, Zigbee2MQTT is the one used.

## TV and lights check list (not run)

1. `z2m-link` on the set: `ok: true`, each light with its `network_address`; `/var/lib/lumatv/ambient/z2m.json` is mode
   0600 and the password appears in no log (`journalctl`-equivalent: the helper's log file) and no topic.
2. Zigbee2MQTT's log at the start of a stream: for each bulb a `write` of `manuSpecificPhilips3` attribute 5 (an
   error here on old bulb firmware is expected and harmless), then `command3`, then `command1` lines at the frame rate on
   the first bulb only, no `Publish 'set' 'zclcommand' ... failed` lines in steady state.
3. The room: still colour on, every named bulb lit with the same colour within a second of enabling; in Picture mode, a
   red-left/blue-right test picture (or a film) moves the left and right bulbs apart; bulbs placed `behind` take the
   average. If only the first bulb follows, the rebroadcast is not reaching the others: try the other bulb as the first in
   the list (`--light` order), then a lower `--fps`.
4. Screen off: all bulbs go to how they were before (off if they were off) within two seconds. If they stay at the last
   colour, the restore did not apply: check the Zigbee2MQTT log for the `/set` after `command3`.
5. Rate: Zigbee2MQTT's log shows no backlog warnings and the bulbs keep up for 10 minutes of a film at 20 fps; if the
   stream stops by itself or the log fills with timeouts, re-run `z2m-link` with `--fps 10`.
6. Helper CPU with the stream open (`top` on the set): the helper's total under the Hue path's, well under 1 % of a core
   for the output.
7. Latency: a flash on the screen to the bulb (phone slow-motion), expected around 100-150 ms with `delay_ms` 0.

## Open

- Whether a bulb restores its own state after command 3 (then the helper's restore is redundant but harmless) or keeps
  the last frame (then the restore is what makes the room right). Only the set can show it.
- Gradient strips (segment mode and command 7) are encoded as Bifrost does and untested on any strip; Bifrost itself
  lists 3-segment strips as "perhaps working".
- MQTT over TLS to the broker is not built: the broker is on the LAN and plain 1883 is what Zigbee2MQTT and every LAN
  device use with it. If Nitin wants 8883, the TLS child `hue_rest` uses could carry it.
