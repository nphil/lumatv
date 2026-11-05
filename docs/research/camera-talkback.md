# Camera talkback and the TV's microphones

Research and design, 2026-11-05. No code was written and nothing was run on the TV. `[INFERENCE]` marks what no source in the repo or the public pages below states. Evidence keys: `TV:` measured on this set (OLED65C4PUA, webOS 10.2.1, Magic Remote `LGE MR24`, BLE), `SRC:` LG's published GPL tree under `/data/home/lgtv/research/lgsrc`, `PUB:` public pages (named inline), `REPO:` another file in this repo or in `/data/home/CameraIntercoms`, `/data/home/LumaShow`.

## Answer

| | Feasible for camera talkback | Why |
|---|---|---|
| Magic Remote microphone | Probably yes, unproven on this remote | Other people read the remote's voice reports straight from `hidraw` as mSBC (wideband SBC, 16 kHz) on a newer remote (MR25GB), and LG's `voiceinput` service can hand out the same audio as PCM. Nobody has read an MR24 stream; the codec on it is a guess until probe R1. |
| TV's built-in microphone (`hw:1,0`) | Yes, with half-duplex ducking | A plain ALSA capture; already the chosen voice-search source. Echo from the Denon is the problem, not access. |

Recommended path: talk with the **Magic Remote's microphone** (hold the mic key, the mouth is centimetres from the capsule, the receiver is a room away), behind one `MicSource` interface that the built-in microphone also implements, so talkback works on the built-in microphone the day the remote path fails its probe. The built-in microphone stays the source for the wake word. Camera audio is ducked for the whole press (half-duplex). The bridge gets one new, separately permitted capability (talk), because it is read-only today.

## 1. Magic Remote microphone

### What the repo shows

- The remote is `LGE MR24`, Bluetooth LE (`REPO: ble-result.md`). It sleeps and drops its link when laid down; on reconnect `lginput2` makes it a **uhid device** (`_uhid_add_device {MRCU:"LGE MR24"}`, `REPO: im-queue-result.md`), so its HID reports appear on a `/dev/hidraw*` node that exists only while the link is up. The first press after a wake was lost under LumaTV's compositor until `/IM_MQ_WEBOS` was answered (same file); whether that is fixed on the TV is not recorded.
- Push-to-talk is the key chain's: `Key_webOS_Voice` press tells `voiceconductor` `setInputEvent hold down`, release `up` (`REPO: surfaces/KeyEntryPoints.json`, `handleHotkeys:Key_webOS_Voice`). The Rust shell already receives the key as `Key::Mic` with press, repeat and release (`REPO: rustui/ui/src/kit/screen.rs`, `keys.conf` teaches `voice` by name).
- The audio travels remote, `voiceinput_hidraw`, `voiceinput_preprocessor`, `voiceinput_sound`, `voiceinput`, `voiceinput_network` (`REPO: arch/ps-01.txt`). By their names the first reads `hidraw`, the second prepares the samples [INFERENCE]. None holds a `hidraw` or audio node with the screen on (`REPO: hardware-census.md`).
- The `lean` profile stops the whole voice group; on the TV `voiceinput.service` is `masked` (`TV: round9/r11-2026-10-26/log.txt:220`). Nothing of LG's listens to the remote today.
- `hidraw` permissions: udev gives `hidraw*` mode 660, group `compositor` (`TV: gfn-input/tv-facts.txt`); root opens it. `hidraw0` is the LiteEMF USB dongle (`REPO: hardware-census.md`), so the remote is `hidraw1` or higher [INFERENCE: the uhid nodes in `gfn-input/udev-monitor-capture.txt` were `hidraw1`].

### What public sources show

- `PUB: github.com/gprot42/webos-scripts-public/webos-voice-recorder` (MR25GB, rooted webOS): voice is HID report `0xFE`, 124 bytes: `[FE][seq][80][8b]` then two 60-byte air packets, each an H2 header (`0x01`, sequence byte `0x08/0x38/0xC8/0xF8`) and one **57-byte mSBC frame** (sync `0xAD 0x00 0x00`) plus a pad byte. Concatenated frames decode with `ffmpeg -f sbc`. Reports `0xF9` and `0xFB` are also treated as voice. One mSBC frame is 120 samples (7.5 ms at 16 kHz), a report 15 ms, about 67 reports a second, about 8.3 KB/s.
- Same source: the remote's BLE link drops about 2 s after idle; holding the mic key brings it up and starts the stream; the dump script opens the `hidraw` node read-only in parallel with `lginput2` (each opener gets its own report queue), and waits for the node to appear.
- `PUB: github.com/lorek123/lg-ha-voice` (rooted webOS): `luna://com.webos.service.voiceinput/startStreaming {deviceType:"remote", subscribe:true}` returns a Unix socket path that carries **raw PCM16, 16 kHz, mono** while the key is held, and a `voice stop` state from LG's silence detection. The mic key arrives as keycode 428 through `lginputhook`. This is LG's own decode of the same stream and needs the voice services running.

### How push-to-talk starts the stream

The remote's firmware starts streaming on the key (the key-down report and the first voice report are the remote's own, not a TV command) [INFERENCE from the two public sources: both only press the key and read]. No TV command was found that starts it.

### Can the TV read it itself?

Two routes, both unproven on the MR24:

| Route | What it takes | Verdict |
|---|---|---|
| A. Read `hidraw` in the helper, decode mSBC in Rust | root helper opens the remote's node while the key is held (no LG service involved, `voiceinput*` stay masked); a small SBC decoder (57-byte frames, fixed mSBC parameters) or `ffmpeg` if the TV has it (not recorded) | Recommended. Keeps the `lean` decision and Nitin's "LG's voice services stay stopped". |
| B. Un-mask `voiceinput`, call `startStreaming` | LG's stack runs again with its cloud-facing siblings (`voiceinput_network`, `voiceconductor`) | Fallback only. Undoes a `lean` group; LG's key chain would also see the key unless LumaHome takes it. |

Risks for A: the codec on MR24 may differ (probe R1 settles it from the raw reports); the node number is not fixed; the stream stalls if the link is asleep at the press (first words lost; mitigation is the preroll and a visible "Connecting" state, below).

## 2. Built-in microphone (`hw:1,0`)

- Card 1 `LGSIC`, capture device `WoV PDM Mic`: two channels, S24_LE only, 8-48 kHz, 256-sample periods (`TV: ambient-result.md`, `SRC: core/aud/alsa/lgsic_pdm.c`; how the formats were read is not recorded). The PDM DMA runs on the WoV block's bus (`fwloadwov_wovbus_addr` in the driver), so the ALSA stream is the WoV hardware's microphone.
- The helper's default is `arecord -q -D plughw:1,0 -t raw -f S16_LE -r 16000 -c 1` (`REPO: voice-result.md`); the conversion from S24 stereo 48 kHz is `plug`'s job [INFERENCE, never run].
- `/proc/asound/card1/pcm0c/info` and the PDM driver have VAD and noise-suppression registers; no userspace control for them is known.

## 3. Comparison

| | Remote mic (route A) | Built-in mic |
|---|---|---|
| Quality | Close-talk, 16 kHz mSBC, level set by the remote's own gain [INFERENCE] | Far-field PDM, 5 m or more from the sofa; the TV's speakers are off (`external_arc`), the Denon is the noise |
| Echo from the Denon | Small: the capsule is at the mouth, but a loud receiver still leaks in | Large: a loudspeaker a few metres away, no cancellation. LumaShow's own Show heard its wake word 0 of 6 times over its reply on the raw mic and 6 of 6 after its canceller (`REPO: LumaShow HANDOFF 10-19 17:00`) |
| Latency, ours | BLE connection interval plus 15 ms reports [INFERENCE: tens of ms]; plus the stall if the link was asleep | `arecord` period plus pipe, 5-20 ms [INFERENCE] |
| Gesture | Hold the key on the same device that is already in the hand | Same key; remote only carries the key press |
| Implementation | `hidraw` reader, mSBC decoder, node discovery, link-asleep handling: medium | One existing command line: small |
| Privacy | Live only while the key is held (the remote sends nothing otherwise) | The PDM is open for as long as the command runs |
| Standby / always on | No | Possible (wake word) |

## 4. Push-to-talk in the camera full view

Behaviour, in the style of `cameras-ui-spec.md`:

- **Key.** The mic key held in the full view, only when the camera is marked `talk` (section 5), the owner has switched Camera talkback on, and the helper says `ready`. Not offered in the grid, the pop-up or the sheet. Back while talking ends it. The hint glyph `Key::Mic` appears with the other hints only for a talk-capable camera.
- **States** (the pill lives with the name and info pills, theme tokens only):
  1. Key down: the pill says "Connecting" and the chrome stays up (never fades while talking). Camera audio is ducked now (section 6). The microphone starts; audio is buffered in the helper (at most 1 s).
  2. The bridge says `ready` (the camera's speaker path is open): the pill says "Talking" with a live dot and a level bar from the helper's own samples, the buffer is sent. This is the honest moment; the camera may still play it up to 2 s later (doorbells, `REPO: CAMERA_TALKBACK.md`), so the words say what the TV does, not that the visitor hears it.
  3. Key up: 400 ms more, the microphone closes, the pill returns to "Live". The duck ends after the tail (section 6).
- **Limits.** 30 s a press, then the microphone closes and a toast says so (LumaShow's turn limit; `voice.maxSeconds` plays this role for search). A lost key release or a remote that stops sending for 1.5 s ends it the same way. A press under 200 ms does nothing (a tap on a mic key must not open a microphone).
- **Failure** is a toast of one line for 3 s, as the refusal table: "Can't talk to this camera" (not talk-capable, bridge refused), "Scrypted isn't linked", "The camera didn't answer". Nothing retries by itself; a talk is a person's act.
- **Principle 0.** The microphone opens only on the key, only in the full view, never from the AI/MCP side (the routes are loopback; a test must assert no tool mentions talk or the microphone, as for voice). Off by default: `base.talkback`, a switch in Settings, Home & look, Cameras, and a per-camera permission on the bridge. The wake word never starts a talk.

## 5. Audio path to Scrypted

```
remote mic -> hidraw -> mSBC decode ----\
                                         >-- MicSource: 16 kHz S16LE mono, 20 ms (640 B)
built-in mic -> arecord ----------------/        |
                                                 v   helper (talk session, one at a time)
                       HTTPS WebSocket, pinned certificate, bearer token (same as the bridge's other calls)
                                                 v   LumaShow bridge (Scrypted plugin)
              loopback TCP server  <-  ffmpeg input  <-  FFmpegInput object  ->  camera.startIntercom(media)
                                                 v   camera-intercom plugin / Ring plugin -> the camera's own protocol
```

- **Scrypted interface.** `Intercom`: `startIntercom(media: MediaObject)` and `stopIntercom()` (`REPO: ScryptedForks/scrypted/sdk/types`). The bridge calls it on the camera device itself, as HomeKit does, never through WebRTC; the WebRTC talkback bug and the "WebCodec Player" caveat in `CAMERA_TALKBACK.md` therefore do not apply. A camera is talk-capable only if its `interfaces` include `Intercom`; the plugin already advertises it honestly (a dead capability flag was the original bug there).
- **Media object.** The bridge listens on 127.0.0.1 on an ephemeral port, writes the PCM it receives, and hands Scrypted `mediaManager.createFFmpegMediaObject({ url, inputArguments: ['-f','s16le','-ar','16000','-ac','1','-i','tcp://127.0.0.1:<port>'] })`. Both consumers read `inputArguments` and use them as ffmpeg's input (the Ring plugin pushes its output arguments after `inputArguments.slice()`; `camera-intercom/src/mixin.ts` does the same). [INFERENCE: untested with this input form; the plugin's self-test and the `tools/intercom-lab` bench exist to try it.] The plugin adds its own warm-up (300 ms of silence), lead (120 ms on wired cameras, 250 ms on wifi) and pacing, measured at 109-114 ms of ours, so the bridge sends audio as it arrives and adds no pacing of its own.
- **Transport helper to bridge.** A WebSocket `GET /v1/talk?camera=<id>`, bearer token in the header, binary frames of 640 bytes (20 ms, 32 KB/s), text frames for control: `{"t":"ready"}`, `{"t":"end"}`, `{"t":"error","code":...}`. The helper already has a WebSocket client (`voice_ws.rs`); the bridge's HTTP layer has no upgrade path yet. Rejected: a chunked `POST` body (the helper's HTTP client has no streaming upload; a WebSocket also lets camera audio come down the same socket later, `scrypted-bridge-api.md` section 11 lists audio as not in v1).
- **Bridge rules** (new, and the first write capability of a read-only bridge, so a security-review item): a second allow-list tick, "Allow talking to this camera", per camera, off by default; `GET /v1/cameras` gains `talk: true` only when ticked and `Intercom` is present; one talk at a time; `stopIntercom` on end, error, socket close, token change or camera untick; 30 s hard cap and a 3 s no-data timeout whatever the helper says (LumaShow's audiod limits); the audio is never stored or logged.
- **Ring specifics** (`REPO: ScryptedForks/scrypted/plugins/ring/src/camera.ts`):
  - `startIntercom` throws "not in call" unless the camera has an active Ring session. The session exists only while a live stream is being served through the Ring plugin (the bridge's one-client relay of a Ring doorbell, commit `e6b74cb` in `lumashow-scrypted`, is that session). Talk therefore needs live video playing in the full view; the bridge refuses with `not_live` otherwise.
  - Ring's session times out after 120 s (`STREAM_TIMEOUT`) unless refreshed; a talk that outlives it ends with the stream. [INFERENCE: whether the bridge's refresh keeps it alive longer is not read.]
  - Ring's output is `pcm_mulaw`, 8 kHz, SRTP into Ring's audio port, and the speaker is activated on the first packet; the ceiling is 8 kHz speech whatever the TV sends, as the Tapo path (`CAMERA_TALKBACK.md`).
- **Other cameras** (all through `@nphil/camera-intercom`, `CAMERA_TALKBACK.md`): doorbells Baichuan ADPCM 16 kHz with about 2.2 s device-side delay; Tapo 8 kHz A-law; Office Reolink ADPCM; Foscam proprietary; the cat feeder ONVIF backchannel. The Gym Foscam has no working microphone, so talk there is one-way.

## 6. Echo and half-duplex with the Denon

Camera audio does not play on the TV today: the bridge hands out the muted Rebroadcast address and `audio` is `null` (`REPO: scrypted-cameras-design.md` 1.4 and `Audio is off`). The rules below apply to the day it is switched on, and the receiver-mute rule applies already (a movie playing while talking).

Why the loop matters: the doorbells are full duplex with **no echo cancellation** (our tone measured at -10.6 dBFS in the Front Door's own microphone, `CAMERA_TALKBACK.md`), and their speaker delay is about 2.2 s. Camera audio on the Denon, heard by a microphone and sent back, would repeat each phrase every few seconds.

- **Duck on key-down, before the microphone opens.** Camera audio gain goes to zero in the TV's own player (instant, ours). Rejected as the only duck: muting the receiver, because the Denon's command path is slower than a key press [INFERENCE: 100-300 ms, to measure, probe R9].
- **Receiver mute (default on, `talkback.muteReceiver`).** If the receiver is unmuted and something is playing, the helper mutes it through its existing telnet path and restores the old state at the end, including on every error path. Audio the microphone hears before the mute is confirmed is dropped, not sent (up to about 300 ms).
- **Tail.** The duck holds 600 ms after key-up for room decay and the receiver's delay; a camera that echoes the user's words back after its own delay (doorbells) is not covered by a short tail and is accepted, as in the vendors' own apps.
- **No echo cancellation in v1.** The mix capture `hw:1,2` is a clean digital reference at source level (`TV: ambient-result.md`, peak 0.2441 for a 0.244 tone, before volume and the receiver's mute), so a canceller like LumaShow's (16.6 dB at 1 m, section 7) is possible later for the wake word. For talkback the delay through ARC and the Denon, plus room reverberation, is unmeasured [INFERENCE: tens to hundreds of ms]; the half-duplex rule does not depend on it.
- The remote microphone sits at the mouth and needs the duck far less, but the loop above is acoustic and exists for either source, so the duck is the same.

## 7. Wake word on the built-in microphone

Decision input from Nitin (2026-11-05): local wake-word detection on the TV's far-field microphone, in the manner of his Echo Dots and the Show (Home Assistant Assist, Wyoming-satellite style). Nothing here is built.

### Cost of continuous capture

- **CPU.** The TV has four A76 at 1.4 GHz, `performance` governor; LG's PM offlines CPU3 at Home and about 2.7 of 3 online cores idle (`REPO: hardware-census.md`). On the Show's A53 at 1.3 GHz, scalar, microWakeWord is 1.12 % of one core (front end 0.47 %, one model 0.66 %; a second model on the same front end +0.7 %), 3.3 % for the whole voice path with the echo canceller (`REPO: LumaShow docs/voice-architecture.md` 4.1 and HANDOFF 10-17 19:35). An A76 does more work a cycle, so the share is expected well under that [INFERENCE: not measured]. `arecord` with `plug` conversion at 50 reads a second is under 1 % of a core on the host (`REPO: ambient-result.md`; the TV's figure is unmeasured). Budget: under 1.5 % of one core, in a thread or process at idle priority, so no frame-loop cost; the frame gate still has to be measured with it running before anything is claimed.
- **Audio graph and DSP.** Opening a capture adds a peer to LG's graph. `hw:1,2` opened and read cleanly on 2026-10-25 with `aud_dsp_reset_count` 0; `hw:1,3` produced `aud_bm ERR` and `auimc ERR` kernel lines on close (`REPO: ambient-result.md`). The PDM stream is the WoV block's own DMA, never opened on this set (see section 10); an hours-long open, and open across the screen turning off, are unknown. Guards: `aud_dsp_reset_count` unmoved, no new `failed` unit, no new `aud_bm`/`auimc` lines, Mali at 59 fps or more.
- **Standby.** Always listening in Active Standby keeps the application processor awake 50 times a second (LumaShow measured 280 wake-ups a second for its microphone reads, 0.70 % of a core). Default: the wake word runs only while the screen is on; listening in Active Standby is a separate decision after the standby power and wake-up probe (R6).
- **Network.** None while idle with the on-device first stage; 32 KB/s to the server only for the wake-up audio.

### Can LG's WoV wake pipeline be reused?

No, it stays off. `fwload_wov.c` loads LG's closed images (`trigger`, `gram`, `gram2-6`, `net`) into the WoV DSP at fixed addresses; the phrase is LG's ("Hi, LG" in `voice-framework`'s strings) and its trigger is consumed by LG's voice services, which `lean` stops [INFERENCE for the event path: not in the source read]. There is no interface for a model of ours (`REPO: hardware-census.md`: "no ABI"). Keeping it off means leaving the voice group masked and not starting `voiceinput`; whether the WoV firmware keeps running while our capture holds the PDM is unknown (probe R6 reads the DSP node, no microphone). The WoV VAD registers are not reachable from userspace and are not needed: a 1 % model does not need a gate.

### On-device first stage versus streaming everything

| | On-device microWakeWord, then a second opinion (the Show's design) | Stream everything to openWakeWord (Wyoming satellite's default) |
|---|---|---|
| Audio leaving the TV | Only after a wake, preceded by a 1 s RAM preroll | Every sample, always, 256 kbit/s |
| Accuracy | 91 % recall at about 0 false accepts an hour with both stages (Hey Jarvis, synthetic clips; 84 % at 0.19 an hour for the first stage alone) | openWakeWord alone 99.5 % at 0 (same clips); real rooms are worse for both |
| TV cost | about 1 % of a core (estimate above) | arecord plus a socket |
| Server dependence | Works with the server down (first stage), second opinion needs it | Dead without the server |
| Matches the Show | Yes | Matches stock `wyoming-satellite` |

Recommended: the Show's design. Stage 1 is microWakeWord in the helper by reusing LumaShow's pure-Rust `crates/wake-word` (no C compiler, bit-equal front end, one front end for any number of models; models Apache-2.0 from ESPHome's collection); stage 2 is Home Assistant's own wake stage on the preroll, which `voice.rs`'s `assist_pipeline/run` can start from `wake_word` instead of `stt` (`start_stage` and `end_stage` already pass through) [INFERENCE: HA's `wake_word_settings` details to be read before building], or the Wyoming openWakeWord container on the Unraid. No recordings are kept; nothing is written to a log.

Open decisions: the wake phrase (Nitin's Dots and the Shows answer to "Hey Jarvis"; the Show yields to a Dot's HA satellite and arbitrates Show-to-Show with a 22-byte UDP claim, `LumaShow docs/voice-architecture.md` section 14; the TV needs the same rule or its own phrase), and the acoustic problem below.

Echo over a movie is the hard part: the PDM hears the Denon. Without cancellation the wake word will miss while sound plays (the Show's 0 of 6 raw against 6 of 6 cancelled is the only measurement). The reference exists (`hw:1,2`, section 6); a canceller is a second piece of work, sized after probe R5 gives the delay and the level. Until then: the wake word is armed only when the TV's sound output is silent or below a level, or the first release ships "awake and quiet only".

## 8. Mic sharing

One owner. ALSA `hw` capture is single-open [INFERENCE; probe R7 opens a second reader to see], so a mic hub in the helper is the only code that spawns the capture; today `voice.rs` spawns `voice.capture` per request and would collide with an always-on wake capture.

| Consumer | Source | Priority | While it runs |
|---|---|---|---|
| Camera talkback | remote (own `hidraw` reader) or hub | 1 | wake scoring paused, preroll cleared, duck applied |
| Voice search (key held) | hub | 2 | wake scoring paused |
| Wake word | hub, continuous | 3 | -- |

- Talkback on the remote does not touch the PDM, so it does not need the hub; it still pauses wake scoring and ducks, so camera speech is never scored or sent to Home Assistant.
- A pause is a gap in the audio: the wake runtime resets its streaming state after any gap (LumaShow rule).
- The hub reports one state, `capture: {open, consumers: [wake|voice|talk], since}`, on the `voice` topic. The shell and the tests read that; nothing reads `pgrep`.
- Voice search through the key keeps working with the wake word off: the hub then opens the capture on demand and closes it after, exactly as the helper does today.

## 9. Indicator and off switch

- **Indicator.** A small persistent mic mark on every surface (status area, one glyph, theme tokens) for as long as `capture.open` is true, a stronger state for a voice turn (the existing voice bar) and for talking (the pill above). The mark is driven by the hub's own child process, and checked independently by reading `/proc/asound/card1/pcm0c/sub0/status` (`RUNNING` while open, a read that opens nothing). A mark that is wrong in either direction fails the acceptance check.
- **Switches, each default off.** "Search by voice" (exists), "Wake word" (new, Settings, Remote & pointer, Voice), "Camera talkback" (new, Settings, Home & look, Cameras). Wake word on requires Search by voice on and the Home Assistant link. A quick-panel tile "Microphone off" kills the hub at once and leaves the switches as they are until turned on again.
- **Off is the process gone**, not audio ignored: the capture child is killed with its process group (as `voice.rs` does), the mark disappears, and `pgrep arecord` is empty. Emptying `voice.capture` in `helper.json` stays the hard off for every built-in-microphone consumer; LG's own "built-in mic" switch is a second one (`voiceinput` strings: "The built-in mic is turned off/on"), whose effect on a capture with LG's services masked is unknown (probe R4).

## 10. Has `voice.capture` / `arecord` run on the TV?

The repo shows no run, and cannot show a negative:

- `docs/research/voice-result.md` (2026-10-25) states nothing in it has run on the TV and lists the TV check list as not run. The helper's tests (`helper/app/tests/voice.rs`, 15) use a fake Home Assistant and a fake microphone.
- A search of the repo, the research dumps and `/data/home/tmp/receipts/lumatv-historical-stage.TyUJ9M4D` for `plughw:1,0`, `/v1/voice`, `pcmC1D0c` and `voice.capture` finds sources, tests, the voice documents and the check list only: no TV log, no `arecord` output, no dump.
- The helper's own log file was removed later (`76650710`, `base.logging`), so a run by the owner or in an unrecorded slot would leave nothing the receipts contain. What the TV itself holds settles it (probe R0 reads `/var/lib/lumatv/voice/ha.json`, `GET /v1/voice` and `base.voice`, none of which opens a microphone).
- What did run: `arecord -D hw:1,2` (the mix capture) on 2026-10-25, 05:19-05:28 EDT, S16_LE 48 kHz stereo, `aud_dsp_reset_count` 0, so `arecord` and `libasound` work on the set; `hw:1,3` was opened and stalled (`REPO: ambient-result.md`). The capture formats of `hw:1,0` are in the same file; how they were read is not recorded.
- `lumatv voice on|off|status` in the TV logs is the restyle of LG's Search app (`8576ea65`, `docs/research/round9/r11-2026-10-26/log.txt`), not a microphone.

## 11. Effort

| Piece | Effort | Notes |
|---|---|---|
| Probes R0-R2 (remote node, codec, latency) | S | decide the route |
| `MicSource` + hub refactor of `voice.rs` | M | needed before wake word or talk |
| Remote `hidraw` reader + mSBC decoder | M | the codec is checked by R1 first |
| Helper talk session + `talkback` setting + pill/hint in the full view | M | UI follows `cameras-ui-spec.md` |
| Bridge: `/v1/talk`, permission, `Intercom` call, self-test | M | the bridge's first write capability; review it |
| Ducking (player gain, receiver mute with restore) | S-M | camera audio playback is a separate task |
| Wake word first stage (crate reuse), second stage, arbitration or own phrase | M-L | echo handling is the open part |
| Indicator, switches, microphone-off tile | S | |

Order: R0-R2, hub, talkback on the built-in microphone (works without any decode), remote source, wake word.

## 12. TV probes

Each follows the existing guards: Home foreground, Picture Mode `filmMaker`, receiver muted through Home Assistant first and unmuted at the end, abort on a new `failed` unit, a moved `aud_dsp_reset_count`, a new `aud_bm`/`auimc` kernel line or Mali under 59 fps, the set left in Active Standby.

**No microphone opened (no approval needed beyond the slot):**

- **R0.** Inventory: `ls -l /dev/hidraw*`; for each node `cat /sys/class/hidraw/hidrawN/device/uevent` and `xxd /sys/class/hidraw/hidrawN/device/report_descriptor` (finds the MR24's node and its vendor report ids and sizes, while the remote is awake); `fuser /dev/hidraw*`; `luna-send -n 1 luna://com.webos.service.mrcu/getConnectionStatus '{}'`; `systemctl status voiceinput*`; `ls -l /var/lib/lumatv/voice/`; `curl -s http://127.0.0.1:18731/v1/voice` and `/v1/settings/base.voice`; `cat /sys/module/aud/parameters/aud_dsp_reset_count`; `cat /proc/asound/card1/pcm0c/info`; `ls /proc/aaudio/`.
- **R6a.** Wake-runtime cost on recorded audio (a file, no microphone): the LumaShow `wakeeval` build for aarch64 over a speech clip, CPU time on the TV.
- **R9.** Receiver mute and restore latency through the helper's telnet path, timed from the command to the receiver's status.

**Open a microphone: each needs Nitin's OK, asked at the moment of the probe:**

- **R1. Remote, raw reports (NEEDS NITIN'S OK: opens the Magic Remote microphone).** With the node from R0: `timeout 12 cat /dev/hidrawN > /tmp/mr-raw.bin` while Nitin holds the mic key and says a sentence; read-only, voice services stay masked. Offline: report ids, lengths, the `0xFE` and mSBC sync pattern, `ffmpeg -f sbc` decode, level. Settles the codec.
- **R2. Remote, timing (NEEDS NITIN'S OK: opens the Magic Remote microphone).** The same read with timestamps (`python3` or `evtest`-style reader with `CLOCK_MONOTONIC`): key-down to first voice report, key-up to last, and the first press after 60 s of the remote asleep (the lost-first-press case). A reader that stops after 1.5 s without a report.
- **R3. Remote through LG's service (NEEDS NITIN'S OK: opens the Magic Remote microphone; changes the `lean` voice group).** Only if R1 fails: start `voiceinput` and call `startStreaming {deviceType:"remote"}`; stop it and re-apply `lean`'s voice group after.
- **R4. Built-in microphone, levels (NEEDS NITIN'S OK: opens the TV microphone).** The existing check list steps 1 and 2 of `voice-result.md`: `arecord -D hw:1,0 --dump-hw-params -d 1 /dev/null`, then `plughw:1,0` for 3 s of a quiet room and 3 s of speech at the sofa, 16 kHz, peaks compared, the guard values before and after; with LG's "built-in mic" switch on and, separately, off.
- **R5. Built-in microphone with the Denon playing (NEEDS NITIN'S OK: opens the TV microphone).** The receiver at Nitin's usual level, 15 s of a known file, `arecord -D plughw:1,0` and, at the same time, `arecord -D hw:1,2` (the mix: not a microphone); offline cross-correlation gives the Denon-and-room delay and the echo level against speech at the sofa. Then 10 s of speech over the same playback.
- **R6. Continuous capture, cost (NEEDS NITIN'S OK: opens the TV microphone).** `arecord -D plughw:1,0` to `/dev/null` for 30 min on Home and 30 min with the screen off in Active Standby: `/proc/<pid>/stat` CPU, `/proc/interrupts` and `vmstat` wake-up rate before, during, after, `aud_dsp_reset_count`, `dmesg`, `/proc/aaudio/dsp3`, Mali fps, and the standby wall-power reading if a meter is attached. Second half only with a separate OK for standby capture.
- **R7. Single-open check (NEEDS NITIN'S OK: opens the TV microphone).** While R6 runs, a second `arecord -D plughw:1,0 -d 1`: expect `Device or resource busy`.
- **R8. Key path in the full view (no microphone while the remote switch is off; with it on NEEDS NITIN'S OK).** In the camera full view hold the mic key: confirm press, repeat and release reach the shell and LG's voice bar does not appear. Run first with Search by voice off (key goes down LG's chain, the services are masked, nothing records) to read the events from the compositor log.

## 13. Decisions for Nitin

1. Talk with the Magic Remote microphone (recommended, after R1) or the built-in microphone from the first release.
2. Camera talkback switch and a per-camera permission on the bridge: the bridge stops being read-only.
3. Wake phrase and arbitration with the Dots and Shows; wake word awake-only or also in Active Standby.
4. Whether camera audio playback on the Denon is wanted at all (talk works one-way without it).
