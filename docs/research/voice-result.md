# Voice search through Home Assistant Assist

Decided (PLAN, 2026-10-25): speech becomes text through Nitin's Home Assistant Assist pipeline, never LG's cloud, and the text fills LumaTV's search. Voice never runs a command.

## Resume here

Decided by Nitin (2026-10-25): the microphone is the TV's built-in one (`hw:1,0` through `arecord`; LG's voice services stay stopped) and the pipeline is **Local (Kokoro + Whisper)**.

Done on the offline host, committed:

- Helper: `voice.rs`, `voice_ws.rs`, `voice_store.rs`, the `base.voice` setting, `/v1/voice/*`, `lumatv-helper voice-link` / `voice-unlink`. The defaults are the decision: `voice.capture` is `arecord -q -D plughw:1,0 -t raw -f S16_LE -r 16000 -c 1` and `base.voice` `pipeline` is `01m3xamd8xzkdv52syapw0cmf2`. 16 end-to-end tests in `helper/app/tests/voice.rs` against a fake Home Assistant and a fake microphone; the whole helper suite passes.
- QML: the mic key path in `LumaHome.qml`, the voice bar's listening, working and failed states, the heard text typed into Search. ShellSlice's recogniser path (`voiceHeard`, `voicePicked`, `Match.voice`, the bar's options) is gone: nothing said can open an app or a page. A "Search by voice" switch sits in Settings, Remote & pointer, under Voice.
- Tests: `tests/test_voice_search.py` (7), the shell, panel, settings, scene-loader and volume host tests, `bun test tests/`.

Open, in this order:

1. Nitin creates the token and links it (steps below), then switches "Search by voice" on in Settings.
2. The TV check list (below). Nothing here has run on the TV.

## What Home Assistant does with the speech (read-only discovery, 2026-10-25)

`assist_pipeline/pipeline/list` on `192.168.1.146`:

| Pipeline | Speech-to-text | Local? |
|---|---|---|
| **Home Assistant Cloud** (the preferred one) | `stt.home_assistant_cloud` | **No.** Nabu Casa's cloud service; the audio leaves the house |
| Local (Kokoro + Whisper) | `stt.openai`, a Wyoming entry titled "openai" | Local by its name; where that server runs was not read (config entries do not show it) |
| Backup (local CPU) | `stt.faster_whisper` (Wyoming "Whisper") | Yes, on the local network |

Nitin chose **Local (Kokoro + Whisper)**, `01m3xamd8xzkdv52syapw0cmf2`, which is now the default of `base.voice`. Emptying the field makes the helper leave `pipeline` out of `assist_pipeline/run`, which is Home Assistant's preferred pipeline: the cloud one, so the audio would leave the house. Where the Wyoming "openai" server behind it runs was not read (config entries do not show it): the check list below tests that it answers.

## Where the microphone audio comes from (repo research, nothing opened on the set)

Two sources exist on this set. Nothing in the repo has read either.

**1. The Magic Remote's microphone, through LG's voice services.** The mic key (`Key_webOS_Voice`) is push-to-talk: LG's key chain tells `voiceconductor` `setInputEvent hold down` on press and `up` on release (`docs/research/surfaces/KeyEntryPoints.json`). The audio itself travels remote, `voiceinput_hidraw`, `voiceinput_preprocessor`, `voiceinput_sound`, `voiceinput` (`ps` snapshots in `docs/research/arch`; `hardware-census.md` notes that none of them holds an audio or hidraw node while the screen is on). The `lean` declutter profile stops all of them in its `voice` group (`declutter-result.md`, `payload/declutter/items.txt`: `voiceconductor`, `voiceinput`, `voiceinput_hidraw`, `voiceinput_preprocessor`, `voiceinput_sound`, `voiceinput_network`, `voiceclick`, `nlpmanager`, `tts`, the voice performer, the Alexa and ThinQ adapters, the Google Home runtime). Nothing in the repo documents a luna call that hands this audio to another program, so reading it would be reverse-engineering on the set. Turning the group back on costs about 30 MB resident for the five voiceinput services and `voiceconductor` (RSS from the `ps` snapshot: 4.4, 4.2, 4.0, 3.8, 3.5 and 2.6 MB), the 25 MB the preprocessor held in swap, and the services LG's own voice assistant needs (performer, Alexa adapter 16 MB with its Amazon connection, ThinQ). It also puts LG's voice back in charge of the mic key: the same hold would start LG's cloud assistant. **Not done; Nitin's decision.**

**2. The TV's own far-field microphone, straight from ALSA.** Card 1 `LGSIC` has capture device `hw:1,0` "WoV PDM Mic": two channels, S24_LE only, 8 to 48 kHz (`ambient-result.md` probed the capture devices' formats; `hardware-census.md`). The set reports `supportVoiceLongDistance: true` and has a "built-in mic is turned off" notice, so a microphone and a switch for it exist. No LG voice service is needed to open an ALSA device, and `arecord` with `libasound` is on the set. It was never opened: the census names the PDM and WoV streams as Nitin's decision, and the audio DSP has a reset counter that the mix-capture probe had to watch. Speaking at the remote while the TV listens from across the room is worse than speaking into the remote, and the TV's own playback is in the room [INFERENCE: whether LG's DSP removes it for this stream is unknown].

The helper does not choose. `helper.json` takes the microphone as a program that writes 16 kHz, 16-bit, mono, little-endian PCM to its standard output. For source 2 that would be:

```json
{"voice": {"capture": ["arecord", "-q", "-D", "plughw:1,0", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1"]}}
```

[INFERENCE: `plughw` converting S24_LE stereo 48 kHz to S16_LE mono 16 kHz is alsa-lib's plug plugin's job and is expected to work; it has not run on the set.] The program runs only while the mic key is held, plus 250 ms, and never longer than 20 s.

**Decided (Nitin, 2026-10-25): the TV's built-in microphone, `hw:1,0` through `arecord`.** The Magic Remote's microphone and LG's voice services are left as they are (stopped by `lean`). The mic key is still taken by LumaHome when voice search is ready, so LG's voice never sees it. Emptying `voice.capture` in `helper.json` takes the microphone away again.

## Nitin's steps

Home Assistant's long-lived tokens are not scoped: a token can do everything the user it belongs to can. So give voice search its own user.

1. In Home Assistant: Settings, People, Users, Add user. Name `lumatv-voice`, administrator off, "Can only log in from the local network" on.
2. Log in as `lumatv-voice` (a private window), open the profile (its name at the bottom left), Security, Long-lived access tokens, Create token, name `LumaTV voice`. Copy the token: it is shown once. [INFERENCE: a non-administrator may run `assist_pipeline/run`; if the first attempt answers with a permission error, the token has to come from an administrator's profile.]
3. On the TV, as root, link it. The token goes in on standard input, so it is in no argument list:

   ```sh
   /var/lib/lumatv/bin/lumatv-helper voice-link --data-dir /var/lib/lumatv --url http://192.168.1.146
   ```

   Paste the token, press Return and then Ctrl-D. The command prints the address it saved and never the token. `voice-unlink` removes it. The token and the address are saved together in `/var/lib/lumatv/voice/ha.json` (mode 0600): the token only ever goes to that address, and only on the home network (the helper refuses any name that resolves elsewhere). Nothing else can change either: not the settings, not the API, not an assistant.
4. Switch it on in Settings, Remote & pointer, Voice, "Search by voice". It is off until then. The row shows what is still missing (not linked, no microphone) once it is on.

The microphone and the pipeline need no step: they are the defaults. To change them, `voice.capture` goes in `/var/lib/lumatv/helper.json` (a program that writes 16 kHz, 16-bit, mono, little-endian PCM to its standard output) and `pipeline` is a setting (`POST /v1/settings/base.voice`).

## How it works

Hold the mic key on Home: the voice bar shows "Listening" and the mic indicator, the helper starts the microphone program and connects to Home Assistant (`ws://.../api/websocket`, token in the `auth` message), then `assist_pipeline/run` with `start_stage: "stt"`, `end_stage: "stt"`, `input.sample_rate: 16000`. Audio goes as binary messages whose first byte is the handler id Home Assistant gave in `run-start`; a message of only that byte ends the audio. Release the key: 250 ms more, the microphone closes, the bar says "One moment", `stt-end` brings the text, and Search opens with it typed in. The user then picks an answer with the remote as for any search. Home Assistant's silence detector may end the speech first, which closes the microphone as well. Back cancels at any time.

- **The helper's side** (`helper/app/src/voice.rs`): `GET /v1/voice`, `POST /v1/voice/{listen,stop,cancel,refresh}`; the `voice` topic is `{state: idle|listening|thinking|heard|error, session, ready, reason?, text?, error?: {code, message}}`. `ready` is false with a `reason` (`voice_off`, `voice_unlinked`, `link_unreadable`, `no_microphone`) until the setting, the link and the microphone are all there. Error codes: `microphone`, `unreachable`, `auth`, `lost`, `nothing_heard`, `stt`, `pipeline`, `timeout`.
- **Microphone is closed** when the request ends however it ends (the key, Home Assistant answering, Back, an error, the setting turned off, the helper stopping), by killing the program's whole process group. A key whose release is lost closes it after `voice.maxSeconds` (20 s).
- **Principle 0**: the routes are on the loopback API, which AI access never reaches (the MCP server is a separate handler, and a test checks no assistant tool mentions voice, the microphone, listening, speech or audio). The microphone is off until the owner switches it on and a root user has linked Home Assistant and named the microphone. Nothing acts on the words: they go into the search box and Search's own rules apply (OK on an answer is a person's key press). The token is never logged, published in a topic or an answer, or shown (`Debug` redacts it), and the words are not logged either (the log says only how many characters).
- **The mic key**: `LumaHome.key` takes `Key_webOS_Voice` only while Home is in front, the AI prompt is not up, and the helper says `ready`. Otherwise the key goes down LG's chain as before. A taken press means LG's voice never sees the key, so nothing of LG's listens even where its services are running.
- **TLS**: an `https://` address goes through the system's `openssl s_client`, as the weather does (LumaShow's `fetch`, the certificate and host name checked). The set's `openssl` may be 1.1.1 and the `s_client` flags need 3.x (see `fetcher.rs`); use the plain `http://` LAN address.

## TV check list (not run)

Each step is read-only or reversible; stop at the first surprise. The set stays on Home, Picture Mode `filmMaker`, and is left in Active Standby afterwards. Abort on a new `failed` unit, an `aud_dsp_reset_count` that moved, or a Mali frame rate under 59 fps.

1. **Before.** Mute `media_player.living_room_avr` through Home Assistant (unmute at the end). Record `cat /sys/module/aud/parameters/aud_dsp_reset_count` (it was 0), `ls -l /dev/snd/pcmC1D0c` (root:audio 0660), `cat /proc/asound/card1/pcm0c/info` and `systemctl list-units --state=failed`. Nothing may hold the device: `fuser /dev/snd/pcmC1D0c` prints nothing. LG's own "built-in mic" switch must be on in LG's Settings (the set has a "The built-in mic is turned off." notice); with it off the capture is expected to be silent [INFERENCE].
2. **Level test, receiver muted.** In a quiet room: `arecord -q -D plughw:1,0 -t raw -f S16_LE -r 16000 -c 1 -d 3 /tmp/mic-quiet.raw`. Expect 96000 bytes. Then again with someone speaking at the sofa's distance, into `/tmp/mic-speech.raw`. Peak of each: `od -An -t d2 -v FILE | tr -s ' ' '\n' | awk 'NF{a=$1<0?-$1:$1; if(a>m)m=a} END{print m}'`. Pass: speech peaks at least ten times higher than the quiet room, and the quiet room is not exactly 0. All zeros means the stream has no source or LG's mic switch is off; a full-scale square or a stall means the format is wrong (check `arecord -D hw:1,0 --dump-hw-params -d 1 /dev/null` for S24_LE stereo and the rate range). Play the speech file back on a PC if the numbers look right but the words must be checked (`aplay -t raw -f S16_LE -r 16000 -c 1`).
3. **The device after.** `aud_dsp_reset_count` unchanged, `dmesg | tail -20` has no `aud_bm ERR` or `auimc ERR` lines, the receiver's sound on unmute is as before.
4. **Home Assistant's side.** In Home Assistant, Settings, Devices & services: the Wyoming entries "openai" and "Whisper" show as loaded. Link the token (steps above). `curl -s http://127.0.0.1:18731/v1/voice` shows `"ready": false, "reason": "voice_off"` before the switch and `"ready": true` after it is turned on in Settings.
5. **One listen round trip, by hand.**
   ```sh
   curl -s -X POST -H 'X-Luma-Client: check' -H 'Content-Type: application/json' -d '{}' http://127.0.0.1:18731/v1/voice/listen
   # say "what time is it" at the sofa, about three seconds
   curl -s -X POST -H 'X-Luma-Client: check' -H 'Content-Type: application/json' -d '{}' http://127.0.0.1:18731/v1/voice/stop
   sleep 3; curl -s http://127.0.0.1:18731/v1/voice
   ```
   Pass: `state` is `heard` and `text` is the words (Whisper may be slow on the first request: up to 12 s before `timeout`). While it listens `pgrep arecord` finds one process; after `stop` and after the answer it finds none. `grep -c` of the token and of the words in `/var/lib/lumatv/log-helper` is 0.
6. **Failure shapes.** With the speech-to-text server stopped the answer is an `error` with code `stt` or `timeout`, in words, and `pgrep arecord` is empty. With a wrong token, code `auth`.
7. **The key.** On Home, hold the mic key, say "netflix", release. Pass: the bar shows Listening, then One moment, then Search opens with "netflix" typed in and nothing launches (Netflix only starts if OK is pressed on its answer); LG's voice bar does not appear. Back during listening closes the bar and `pgrep arecord` is empty.
8. **After.** Unmute the receiver; leave the set in Active Standby. If the TV mic's capture disturbs LG's audio graph, turn the switch off in Settings (the helper then never starts `arecord`) and empty `voice.capture`.

## Left as found, for Main

- `docs/research/shell-slice-result.md` still says nothing starts the voice bar and lists the old recogniser calls and scenes `28-voice-heard` and `28-voice-results`; the mock shots of those two stay in `docs/mocks/shots` as design history.
- No Settings control for the pipeline or for linking Home Assistant: both are command-line or API steps on purpose (a link is a key action, and nothing a page can reach may change where the audio goes).
- Qt's `XMLHttpRequest` in the QML host reports status 0 and no body for a 409, so the voice bar names the refusal from the helper's `reason` on the `voice` topic. [INFERENCE: the TV's Qt 5.12 does the same; the host runs the same Qt.]

## Tests

- Helper: `cd helper && cargo test --offline` (everything, 310 unit and all integration tests; the voice ones are `--test voice`, 15 tests), `cargo clippy --offline --all-targets -- -D warnings`, `cargo fmt --check`.
- QML: `python3 -m unittest discover -s tests -p 'test_voice_search.py'` (7), `test_shell_surfaces.py` (12), `test_panels.py` (13), `test_settings.py`; `TZ=America/New_York bun test tests/`.
- Scenes: `python3 tests/qmlhost/scenes.py --out /tmp/voice-shots --only 09-search-keyboard,28-voice-listening,28-voice-working,28-voice-failed,28-mic-indicator`.
