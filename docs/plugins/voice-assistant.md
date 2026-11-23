# Voice assistant

The TV's built-in microphone as a Home Assistant voice satellite. A helper plugin, off until the owner switches it on in Settings, Sound, Voice assistant; its page asks first and says what is listened to. While it is on and the TV is paired, the TV listens for the wake word itself and nothing is sent or kept before it. What is said after it goes to the owner's Assist pipeline in Home Assistant, which answers; the TV shows the turn and speaks the answer.

## Pairing

The page's **Pair** action opens a window of five minutes in which the TV announces itself as an ESPHome device (`lumatv-voice-<last six hex of its MAC>`, port 6055; `voiceAssistant.port` in helper.json changes it). In Home Assistant, Settings, Devices, Add integration, ESPHome (or the discovered device) adds it; Home Assistant sets the key, which the TV keeps in `/var/lib/lumatv/voice-assistant/key` (mode 0600) and the window closes. The device is then assigned an area (Living Room) and a pipeline in Home Assistant. **Forget** removes the key and closes the microphone. With neither the window nor a key nothing listens on the port, and an unpaired TV never opens the microphone.

## What it does

- Wake words: Okay Nabu (default) or Hey Jarvis, detected on the TV (`helper/voice`, the `lumavoice` engine). Sensitivity: strict, normal or lenient.
- The microphone is `hw:1,0` (two channels, S24_LE, opened at the chip's fixed 256-frame period and an 8192-frame buffer: with 2048 the driver's position runs past the buffer and the capture stops delivering after about 3 s). It is open only while the plugin is on and paired, and the engine drops the first 250 ms (the chip pops when it opens; the pop is gone by 40 ms). Its floor is about -74 dBFS.
- Quiet hours (default 01:00 to 07:00, changeable or off): the answer is shown and not spoken. The page's **Speak answers** switch and **Answer volume** apply the rest of the time; the level Home Assistant sets for the satellite follows the same setting. The TV's own audio, the receiver and the chimes are never touched.
- The screen is told by the `voice.assistant` topic: the microphone's state, and the turn's phase, what was heard and the answer. A turn of another Home Assistant assistant in the page's **Room** (default Living Room; empty shows none) is shown the same way while it listens and answers, read from Home Assistant through the Home Assistant link (`/var/lib/lumatv/ha/link.json`). The page's facts (status, microphone, Home Assistant, last request, other assistants) say how it stands and never what was said.
- If LG's audio processor resets while the microphone is open (`/sys/module/aud/parameters/aud_dsp_reset_count` rises), the plugin closes the microphone and says so until it is switched off and on.

## Overlay (panel care)

Everything the assistant draws is transient and takes the active theme's glass, type and colours, so nothing is brighter than the rest of the interface. The glow round the edges flows (or breathes with reduced motion) and never rests; the words plate and the answer card come with a turn and fade after at most 6 s and a fade of 400 ms; a turn is shown for 35 s at the longest. The helper drops a finished turn from the `voice.assistant` topic after 8 s, and one nothing ended after 40 s. Nothing stays on the screen while the assistant is idle.

## Self-check

`POST /v1/voice-assistant/selftest` (loopback only; answered only while `voiceAssistant.selftest` is `true` in helper.json) plays the clip `/var/lib/lumatv/voice-assistant/selftest.wav` (16 kHz mono 16-bit, a recorded wake phrase) to the engine in place of the microphone and answers `{passed, phase, heard, reply, error, tookMs}` when the turn ends. `helper/voice/tests/data/okay_nabu_time.wav` is such a clip.

## Limits

- Echo canceller on, measured on the TV with a Denon at volume 0.60: the TV's mixer capture `hw:1,2` is the reference. It accepts only 48 kHz but its frames come at twice that pace, so it is read as 96 kHz, and only its right channel carries the sound (the plugin pins it). The echo arrives 32 to 42 ms after the reference; the canceller locks about 2 to 3 s after the sound starts and takes out 12.6 to 13.5 dB over ten minutes of film audio (it drops the lock in quiet scenes and finds it again within 5 s). Film dialogue that says the wake phrase woke the raw microphone 3 times in 80 s and the cleaned stream 0 times; 10 minutes of film audio woke neither. It costs about 1% of one core on the TV (156 µs per 16 ms block, 430 µs at p99).
- Voice search on the remote's mic key reads the same microphone with its own `arecord` and finds it busy while the assistant holds it. The engine's `tap()` is the way to share it; the search does not use it yet.
- Not run on the TV at the time of writing: pairing, a real turn, the 120 Hz frame gate with the overlay, and a ten-minute watch of the cost.

Code: `helper/app/src/voice_assistant.rs` (plugin loop, page, facts, self-check), `voice_assistant/{pair,turn,dot}.rs` (announcement and key, the screen's document, the other assistants); screen: `rustui/ui/src/system/voice_assistant*.rs` and `settings/pages/voice_assistant.rs`. The wire to Home Assistant is LumaShow's `voice-host` and `voice-satellite` crates, by path.
