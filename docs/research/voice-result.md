# Voice search through Home Assistant Assist

Decided (PLAN, 2026-10-25): speech becomes text through Nitin's Home Assistant Assist pipeline, never LG's cloud, and the text fills LumaTV's search. Voice never runs a command.

## Resume here

Done on the offline host, committed:

- Helper: `voice.rs`, `voice_ws.rs`, `voice_store.rs`, the `base.voice` setting, `/v1/voice/*`, `lumatv-helper voice-link` / `voice-unlink`. 15 end-to-end tests in `helper/app/tests/voice.rs` against a fake Home Assistant and a fake microphone; the whole helper suite passes.
- QML: the mic key path in `LumaHome.qml`, the voice bar's `working` and `failed` states, the heard text typed into Search. 5 host tests in `tests/test_voice_search.py`; the 12 shell, 13 panel, 3 scene-loader and 2 volume tests still pass; scenes `28-voice-working`, `28-voice-failed` and `09-search-keyboard` (now lit by the helper's `ready`, not `theme.voice.enabled`) were rendered in both materials.

Open, in this order:

1. Nitin decides the microphone source (next section). Until `voice.capture` names one, voice search reports `no_microphone` and opens nothing.
2. Nitin creates the token and links it (steps below).
3. On the set: nothing here has run on the TV. First check is `arecord` reading the chosen device at 16 kHz and a `voice-link` against the real Home Assistant.

## What Home Assistant does with the speech (read-only discovery, 2026-10-25)

`assist_pipeline/pipeline/list` on `192.168.1.146`:

| Pipeline | Speech-to-text | Local? |
|---|---|---|
| **Home Assistant Cloud** (the preferred one) | `stt.home_assistant_cloud` | **No.** Nabu Casa's cloud service; the audio leaves the house |
| Local (Kokoro + Whisper) | `stt.openai`, a Wyoming entry titled "openai" | Local by its name; where that server runs was not read (config entries do not show it) |
| Backup (local CPU) | `stt.faster_whisper` (Wyoming "Whisper") | Yes, on the local network |

So with no pipeline chosen the words go to Nabu Casa, not to LG. If the words must stay in the house, `base.voice` has a `pipeline` field: set it to `01m3xamd8xzkdv52syapw0cmf2` (Local, Kokoro + Whisper) or `01m3yddanfdmg4epdmbmaht8kb` (Backup, local CPU). The helper sends it as `pipeline` in `assist_pipeline/run`. Nitin's choice: the default is the preferred pipeline, which is the cloud one.

## Where the microphone audio comes from (repo research, nothing opened on the set)

Two sources exist on this set. Nothing in the repo has read either.

**1. The Magic Remote's microphone, through LG's voice services.** The mic key (`Key_webOS_Voice`) is push-to-talk: LG's key chain tells `voiceconductor` `setInputEvent hold down` on press and `up` on release (`docs/research/surfaces/KeyEntryPoints.json`). The audio itself travels remote, `voiceinput_hidraw`, `voiceinput_preprocessor`, `voiceinput_sound`, `voiceinput` (`ps` snapshots in `docs/research/arch`; `hardware-census.md` notes that none of them holds an audio or hidraw node while the screen is on). The `lean` declutter profile stops all of them in its `voice` group (`declutter-result.md`, `payload/declutter/items.txt`: `voiceconductor`, `voiceinput`, `voiceinput_hidraw`, `voiceinput_preprocessor`, `voiceinput_sound`, `voiceinput_network`, `voiceclick`, `nlpmanager`, `tts`, the voice performer, the Alexa and ThinQ adapters, the Google Home runtime). Nothing in the repo documents a luna call that hands this audio to another program, so reading it would be reverse-engineering on the set. Turning the group back on costs about 30 MB resident for the five voiceinput services and `voiceconductor` (RSS from the `ps` snapshot: 4.4, 4.2, 4.0, 3.8, 3.5 and 2.6 MB), the 25 MB the preprocessor held in swap, and the services LG's own voice assistant needs (performer, Alexa adapter 16 MB with its Amazon connection, ThinQ). It also puts LG's voice back in charge of the mic key: the same hold would start LG's cloud assistant. **Not done; Nitin's decision.**

**2. The TV's own far-field microphone, straight from ALSA.** Card 1 `LGSIC` has capture device `hw:1,0` "WoV PDM Mic": two channels, S24_LE only, 8 to 48 kHz (`ambient-result.md` probed the capture devices' formats; `hardware-census.md`). The set reports `supportVoiceLongDistance: true` and has a "built-in mic is turned off" notice, so a microphone and a switch for it exist. No LG voice service is needed to open an ALSA device, and `arecord` with `libasound` is on the set. It was never opened: the census names the PDM and WoV streams as Nitin's decision, and the audio DSP has a reset counter that the mix-capture probe had to watch. Speaking at the remote while the TV listens from across the room is worse than speaking into the remote, and the TV's own playback is in the room [INFERENCE: whether LG's DSP removes it for this stream is unknown].

The helper does not choose. `helper.json` takes the microphone as a program that writes 16 kHz, 16-bit, mono, little-endian PCM to its standard output. For source 2 that would be:

```json
{"voice": {"capture": ["arecord", "-q", "-D", "plughw:1,0", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1"]}}
```

[INFERENCE: `plughw` converting S24_LE stereo 48 kHz to S16_LE mono 16 kHz is alsa-lib's plug plugin's job and is expected to work; it has not run on the set.] The program runs only while the mic key is held, plus 250 ms, and never longer than 20 s.

**Decision for Nitin:** (a) the TV's microphone through `arecord` (nothing of LG's re-enabled; first try is read-only for 3 s with the receiver muted and the DSP reset counter checked), (b) the Magic Remote's microphone through LG's voice services (re-enables the `voice` group, needs a luna interface to reverse), or (c) neither for now. Until he picks, nothing opens a microphone.

## Nitin's steps

Home Assistant's long-lived tokens are not scoped: a token can do everything the user it belongs to can. So give voice search its own user.

1. In Home Assistant: Settings, People, Users, Add user. Name `lumatv-voice`, administrator off, "Can only log in from the local network" on.
2. Log in as `lumatv-voice` (a private window), open the profile (its name at the bottom left), Security, Long-lived access tokens, Create token, name `LumaTV voice`. Copy the token: it is shown once. [INFERENCE: a non-administrator may run `assist_pipeline/run`; if the first attempt answers with a permission error, the token has to come from an administrator's profile.]
3. On the TV, as root, link it. The token goes in on standard input, so it is in no argument list:

   ```sh
   /var/lib/lumatv/bin/lumatv-helper voice-link --data-dir /var/lib/lumatv --url http://192.168.1.146
   ```

   Paste the token, press Return and then Ctrl-D. The command prints the address it saved and never the token. `voice-unlink` removes it. The token and the address are saved together in `/var/lib/lumatv/voice/ha.json` (mode 0600): the token only ever goes to that address, and only on the home network (the helper refuses any name that resolves elsewhere). Nothing else can change either: not the settings, not the API, not an assistant.
4. Choose the microphone in `/var/lib/lumatv/helper.json` (above, after the decision) and restart the helper.
5. Switch voice search on, which it is not by default:

   ```sh
   curl -s -X POST -H 'X-Luma-Client: nitin' -H 'Content-Type: application/json' \
     -d '{"changes":[{"key":"enabled","value":{"type":"bool","value":true}}]}' \
     http://127.0.0.1:18731/v1/settings/base.voice
   ```

   (Add `{"key":"pipeline","value":{"type":"text","value":"01m3yddanfdmg4epdmbmaht8kb"}}` to keep the words in the house.)

There is no Settings page for it yet; the switch is the API call above.

## How it works

Hold the mic key on Home: the voice bar shows "Listening" and the mic indicator, the helper starts the microphone program and connects to Home Assistant (`ws://.../api/websocket`, token in the `auth` message), then `assist_pipeline/run` with `start_stage: "stt"`, `end_stage: "stt"`, `input.sample_rate: 16000`. Audio goes as binary messages whose first byte is the handler id Home Assistant gave in `run-start`; a message of only that byte ends the audio. Release the key: 250 ms more, the microphone closes, the bar says "One moment", `stt-end` brings the text, and Search opens with it typed in. The user then picks an answer with the remote as for any search. Home Assistant's silence detector may end the speech first, which closes the microphone as well. Back cancels at any time.

- **The helper's side** (`helper/app/src/voice.rs`): `GET /v1/voice`, `POST /v1/voice/{listen,stop,cancel,refresh}`; the `voice` topic is `{state: idle|listening|thinking|heard|error, session, ready, reason?, text?, error?: {code, message}}`. `ready` is false with a `reason` (`voice_off`, `voice_unlinked`, `link_unreadable`, `no_microphone`) until the setting, the link and the microphone are all there. Error codes: `microphone`, `unreachable`, `auth`, `lost`, `nothing_heard`, `stt`, `pipeline`, `timeout`.
- **Microphone is closed** when the request ends however it ends (the key, Home Assistant answering, Back, an error, the setting turned off, the helper stopping), by killing the program's whole process group. A key whose release is lost closes it after `voice.maxSeconds` (20 s).
- **Principle 0**: the routes are on the loopback API, which AI access never reaches (the MCP server is a separate handler, and a test checks no assistant tool mentions voice, the microphone, listening, speech or audio). The microphone is off until the owner switches it on and a root user has linked Home Assistant and named the microphone. Nothing acts on the words: they go into the search box and Search's own rules apply (OK on an answer is a person's key press). The token is never logged, published in a topic or an answer, or shown (`Debug` redacts it), and the words are not logged either (the log says only how many characters).
- **The mic key**: `LumaHome.key` takes `Key_webOS_Voice` only while Home is in front, the AI prompt is not up, and the helper says `ready`. Otherwise the key goes down LG's chain as before. A taken press means LG's voice never sees the key, so nothing of LG's listens even where its services are running.
- **TLS**: an `https://` address goes through the system's `openssl s_client`, as the weather does (LumaShow's `fetch`, the certificate and host name checked). The set's `openssl` may be 1.1.1 and the `s_client` flags need 3.x (see `fetcher.rs`); use the plain `http://` LAN address.

## Left as found, for Main

- `LumaHome.voiceHeard(words)` and `Match.voice` (ShellSlice's recogniser API, with the voice bar's `heard` state and its options) are not on the new path and nothing calls them. They can launch an app when the words name one, which is a command, so they should go (with the `28-voice-heard` and `28-voice-results` scenes and the shell test that uses them) once Nitin confirms the search box is the only result.
- No Settings page switches voice search on; the API call above does. A page would also show `ready` and its `reason`.
- `docs/research/shell-slice-result.md` still says nothing starts the voice bar.
- Qt's `XMLHttpRequest` in the QML host reports status 0 and no body for a 409, so the voice bar names the refusal from the helper's `reason` on the `voice` topic. [INFERENCE: the TV's Qt 5.12 does the same; the host runs the same Qt.]

## Tests

- Helper: `cd helper && cargo test --offline` (everything, 310 unit and all integration tests; the voice ones are `--test voice`, 15 tests), `cargo clippy --offline --all-targets -- -D warnings`, `cargo fmt --check`.
- QML: `python3 -m unittest discover -s tests -p 'test_voice_search.py'` (5), `test_shell_surfaces.py` (12), `test_panels.py` (13).
- Scenes: `python3 tests/qmlhost/scenes.py --out /tmp/voice-shots --only 09-search-keyboard,28-voice-listening,28-voice-working,28-voice-failed,28-mic-indicator`.
