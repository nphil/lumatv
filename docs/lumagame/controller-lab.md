# Controller lab

Settings › Controllers › Controller lab shows each pad as the kernel reports it, times its reports, sets the stick shaping the
engines apply, rumbles it, and lights the screen on a press so a phone video can time press to light. During a session it also
asks the engine for the legs of the round trip.

## Reading the pads

The lab reads the pads through `lumatv-gfn-pad` with `PadConfig::raw` on: every `SYN_REPORT` reaches `PadSink::raw` with the
kernel's evdev stamp and the stick and trigger values before any shaping. The kernel passes on only reports in which a value
changed, so a pad nobody touches sends nothing, and the rate is measured while a stick moves.

`lumatv_gfn_pad::lab::Timing` keeps 3 s of stamps. Intervals of 250 ms or more are pauses and stay out of the figures. It
reports the rate, p50/p95/p99/max of the interval, jitter (the mean change from one interval to the next), a histogram with edges
at 0.5, 1, 2, 4, 8, 16, 32 and 64 ms, and bursts: a gap of 20 ms or more followed by three or more reports each under 1 ms apart.
That is how a Bluetooth link in sniff mode delivers, holding reports between wake-ups (about 77 ms apart) and handing them over
together.

`lab::Drift` records where a stick settles once it has been still for 300 ms within 30 % of the centre: the last rest and the
worst, which is what the dead zone has to cover.

The live view follows the last-used pad. Button capture and the flash test keep pad presses out of navigation; holding B for
1 second or releasing the remote's Back leaves the test. Rumble left/right runs that motor for 500 ms on each connected pad.
During a stream the engine owns the pads, so the lab offers stream timing rather than local capture.

## Stick settings

The lab writes three of LumaGame's common settings, which the engines read under the same keys:

- `deadzone`: radial dead zone, 0–20 %, default 8.
- `anti-deadzone`: the smallest output once a stick leaves the dead zone, 0–30 %, default 0. Games with their own dead zone
  otherwise swallow the first part of the travel.
- `stick-curve`: `linear`, `precise` (t²) or `quick` (√t), default `linear`.

`lumatv_gfn_pad::shape` applies them in that order: the dead zone rescales the rest of the travel to 0–1, the curve bends it,
and the anti-deadzone lifts it to `a + (1 − a)·curve(t)`. With the defaults the result equals the previous radial dead zone.

| Engine | Applies the stick settings | How |
| --- | --- | --- |
| GFN | yes | The adapter writes `deadzone`, `antiDeadzone` and `curve` to the streamer's preferences file; the streamer's pads read it at start. |
| Punktfunk | yes | `settings::pad_config` builds the session's `PadConfig` from the settings. |
| Sunshine/Apollo (`gamestream`) | no | v0.1.1 starts its pads with `PadConfig::default()`: an 8 % dead zone, linear. |
| Native | no | A native game reads the pads itself. |

`gamestream` would apply them by adding the three fields to its `settings.json` schema and building its `PadConfig` from the
resolved values, as Punktfunk does.

## Flash test

The first frame after a button edge lights the screen, even when a queued release arrives in the same batch. It stays lit while
a button is held. The screen shows the frame counter and kernel press stamp; one log line per flash records
`lumagame: flash press_ns=<evdev ns> frame=<frame> shown_ns=<swap ns> delay_ms=<milliseconds>`.
The swap hands the frame to the compositor, not the panel. A 240 fps video (4.2 ms a frame) of the controller and screen measures
physical press-to-light; the log alone does not.

## Round trip

An engine with the `round-trip` feature answers `{"type":"round-trip","session":N}` with `{"type":"round-trip","session":N,
"trip":{…}}`. Every field is optional and reports what the engine measured last; the lab shows the ones present.

| Field | Meaning | GFN | Punktfunk | `gamestream` could use |
| --- | --- | --- | --- | --- |
| `presses`, `press_p50_ms`, `press_p99_ms` | Kernel stamp of a press until it leaves the TV, over the pad crate's window | The streamer's `latency` pad line, `send` stage (`handoff` when it has no send stamp) | Until `send_input` returns | The pad crate's `PadSink::latency`, which it already runs every second |
| `network_ms` | Network round trip | `roundTripTimeMs` (WebRTC) | QUIC RTT | `LiGetEstimatedRttInfo` (ENet) |
| `host_ms` | Host capture and encode | `hostProcessingLatencyMs` | The host's per-frame timing | `frameHostProcessingLatency` from each decode unit |
| `arrival_p50_ms`, `arrival_p95_ms` | Host capture until the frame arrived | — | Capture to received, only once a clock offset to the host is measured | — |
| `client_ms` | Client-side hand-off reported by the engine, not panel latency | `decodeLatencyMs` | Mean receive-to-decoded time | Its decode-unit time |

GFN keeps the legs from every `streamer.changed`, whether or not stats are on. Punktfunk measures them only while stats are on,
which they are while the overlay is open.

An engine without the feature gets no request, and the lab says the engine does not support it. An engine built before the
request existed answers `unsupported`, which the lab shows the same way; `gamestream` v0.1.1 is one.

## GameSir Cyclone 2

Settings › Controllers › GameSir Cyclone 2 reads the active hardware profile through the trusted helper's
`/v1/gaming/gamesir` route. Gaming must be enabled in LumaTV. The app jail cannot reliably open hidraw after a receiver changes
mode: its device nodes are static copies. The helper discovers the vendor configuration interface anew on every request.

USB and X-input receiver mode expose report rate (250/500/1000 Hz), left/right rumble strength (0–4), D-pad diagonal lock,
X-input A/B and X/Y swap, and left/right inner stick dead zones (0–20%). Each change preserves unrelated profile bytes and requires
matching read-back. A failure keeps the last confirmed reading visible and requires Refresh before another write. Hardware
settings persist on the controller, separately from the engines' stick shaping. No firmware-update command is exposed.

In DS4 mode the page directs switching to X-input by holding View+Menu for 2 seconds, then refreshing. Whether these hardware
settings also apply after switching back to DS4 remains untested.

If the TV's xpad driver does not know the receiver, its documented dynamic-ID value is `3537 102a ff` (ADC: `3537 100b ff`) for
`/sys/bus/usb/drivers/xpad/new_id`. This is documentation only: neither the app nor helper writes driver IDs or changes modes.

## Leadjoy Xeno

Settings › Controllers › Leadjoy Xeno uses the same gated helper. Discovery requires USB `4131:3519` or `4131:8008`,
the FF80 unnumbered 64-byte vendor interface, an `Xeno` product reply and valid logo lighting. This identifies the Xeno family,
not uniquely the Plus model. Only the controller's reported profile count is offered.

Pages expose single-colour logo RGB/brightness, controller mode, polling, step precision, dual output, D-pad and stick-direction
options, profile naming/activation, gamepad remapping/turbo, macro assignment, documented stick controls/curves/direction
bindings, triggers, motion control with gesture thresholds, motion-gesture actions and grip rumble. Section, profile,
sub-configuration, side, button, gesture, macro, step and press/release selectors only read; they never activate or overwrite
a profile. Writes change one field in a fresh complete object and require read-back, preserving unknown fields, curve length,
macros and hidden trigger-rumble ranges. Failed writes leave the last confirmed values visible and block further changes until
Refresh. A mode change re-enumerates the controller, so a missing read-back after it asks for Refresh instead of claiming success.

Macros are global: the page edits one step at a time of a simple macro (input, key, interval, duration), copies or removes a
step, creates a one-step macro by name and deletes a macro after a confirmation. Advanced macros are only deletable.

Calibration, macro recording (c174/c175), reset/save/reload and sub-configuration triggers answer with an ACK only, so they
stay out of the UI. Keyboard and mouse button-mapping targets, gesture-threshold actions, the motion `fs` flag, more than two
motion trigger keys, motion accessories (c220–c224), FPS settings (c130–c135) and the remaining stick parameters (`rjc`, `mse`) await complete vendor
contracts. Experimental c191/c192 lighting stays out of the UI; the logo has one colour and brightness, not a pixel editor.
Hardware persistence timing and all real-pad behaviour remain untested.

Needs TV: live report timing and Bluetooth bursts; rumble; physical flash video; engine stick shaping and round-trip values;
200% text and lab rendering cost at 120 Hz; GameSir hidraw access, real writes/read-back, receiver binding and cross-mode persistence;
Leadjoy discovery, every setting and read-back on a real pad, including the mode-switch reconnect.
