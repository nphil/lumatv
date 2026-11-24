# Dolby TrueHD & Atmos passthrough

Dolby TrueHD & Atmos passthrough sends Dolby TrueHD, Atmos included, from an app that decodes for itself (Marquee) to a receiver over eARC, without LG's mixer touching it. It is off by default at Settings › Sound › Dolby TrueHD & Atmos passthrough and works only on the TV software it was tested on (LG C4, release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`).

The route is the reusable `audiopass` crate, which has no LumaTV dependencies. The helper module `audio_passthrough` is the plugin around it: the switch and the page, LG's sound-output subscription, the monitor that carries a session through connecting, and the three calls a player makes. Everything that changes the TV's audio runs in the trusted helper and only while the plugin is on. The one exception is putting back what an earlier run changed (its journal), which also runs while the plugin is off. The page adds no animation and no long-dwell element.

Not yet accepted on the TV. The route was proven by hand (real TrueHD Atmos, the receiver showed Dolby Atmos); the plugin around it has only run against fakes.

## What a session does

The TV is stock between sessions. A session changes it only while a player streams, and every change is journalled first (`<helper data>/audio-passthrough/journal`, mode 0600).

1. **Arm.** The player holds LG's renderer (`/dev/lg/aren0`) and calls `arm`. The helper refuses unless the plugin is on, the screen is on, the firmware matches, LG's sound output is the receiver on eARC, the audio services are healthy, nothing is connected to any decoder input and the granted decoder is stopped. LG's own mixer input for the player's stream is the one connection allowed, and taken over first (below). The helper then configures LG's PCM bypass converter for the renderer.
   **Un-mix.** Once the player calls `flowing` (its sink's `pcm-multi-ch` is back on after the arm), or 1 s after the arm without that call, the helper un-mixes LG's eARC output words, provided they still read as the arm found them and nothing but the granted decoder is in use. LG re-applies its words when the sink's property is written: on the C4 an un-mix made before that write read mixed again within the second.
2. **Stream.** The player sends the carrier (below). The helper watches `bypassing` through the monitor.
3. **Connect.** At the first Pass-Through after the un-mix the helper connects the granted decoder (ADEC) to the main output. After a takeover of LG's mixer input it connects straight after the un-mix instead: the output codec follows the main audio input, which stays LG's mixer input after its disconnect, so Pass-Through shows only once the decoder is connected, and the 1 s grace for it starts at the connect. Once Pass-Through shows, an input the kernel reads muted gets a mute and, 0.5 s later, an unmute; one found unmuted is audible at once. `audible` follows the first Pass-Through by the connect's own time, or by about 0.6 s with the pair.
4. **Watch.** While connected, LG changing the route (bypass off, mix words changed, converter gone, another decoder connected, the granted one disconnected, re-sourced, started decoding by LG's own input, or muted) ends the session after a 1 s grace. LG putting its own mixed words back is the exception: the first time, while the granted decoder is the only one connected or decoding, the helper un-mixes them again; a second time, or with any other input in use, ends the session at once. A player that rebuilds its renderer has 5 s to arm again. The watch reads only `/proc` and the kernel, until the kernel shows LG's mixer input for the stream connected again (below). What a player polls does not come from the watch, whose steps hold the route while they wait on LG (a luna call, the `/proc` task scan, the journal write): during a session a reader thread reads the kernel every 50 ms and a sampler thread publishes the sample every 50 ms, and neither takes the route. A read that waits on LG's driver holds only the reader. The sample is stamped when it is published, so `ageMs` says whether the sampler is alive; a reading more than 200 ms old no longer says `bypassing`, and the reason says a read of LG's sound status has not come back.
5. **Stop.** In this order: the player is told to stop sending, the input is muted, the mute is put back as it was found (only once the player has closed `/dev/lg/aren0`, waiting up to 2 s; otherwise the input stays muted and the page says so), the decoder is disconnected, LG's mix words are written back after a 0.3 s settle, LG's mixer input is given back if the session took it, the journal is removed, and the TV is read again and compared with what the session found. When the helper itself is stopping it skips the wait for the renderer and the comparison: the restore then takes about a second of the 4 s the helper allows, an input found unmuted stays muted, and `after` is not computed.

The restore runs on `release`, the player's death, route loss, the plugin being switched off, the TV's standby and the helper's exit. After a crash the journal is replayed at the next helper start, even with the plugin off. A panic in the monitor puts the TV back at once, not when the thread is started again 30 s to 30 min later. A restore the helper's exit cuts short leaves the journal, and the next start finishes it from what the TV reads then. A journal nobody has replayed blocks arming, so LG's original words are never overwritten.

## The decoder is the player's

The player names the decoder input (ADEC, 0 to 7) LG's resource manager granted its renderer, the `index` of its audio sink. The helper uses that one and never assumes ADEC1. It touches nothing an HDMI input could own: a session starts only while the audio service's connection list and the kernel's connected inputs are both empty and the granted decoder is not running. If LG connects another decoder, moves the granted one's source or starts it decoding (`Start=1`) during a session, the session ends and LG's input is left alone: its connection and mute are not touched (LG's mix words are still written back) and `after` says so. A journal replayed after a crash follows the same rule. `Connect` alone cannot say whose input it is: the kernel reads `Connect=HDMI_PORT2` on ADEC1 before, during and after a session.

## LG's mixer input for the player's stream

On the C4 LG connects a mixer input for the player's own stream at the Load, before the player can arm: the player's PCM sink books a PCM mix channel with uMS (`PCMMC<n>`), and LG's audio daemon connects that channel's input (`AMIXER<n>`, `AMIXER4` as measured) to the main output with the eARC words mixed. The helper takes that input over only when LG's own records prove it carries the arming player's stream and nothing else plays sound:

- the kernel's `ConnectedAmixer` lists exactly one input besides LG's own 5, 6 and 7, and no decoder input;
- audiooutputd lists that input and nothing else;
- SAM's running list names the arming process (`processid`) as an app;
- uMS's active pipelines hold `PCMMC<n>` in exactly one pipeline, and no other app's pipeline holds a sound resource (`ADEC`, `PCMMC`, `MAIN_SOUND` and the like).

Anything else is refused as before, with nothing touched. The arm journals the input (its number and uMS pipeline) and then disconnects it as LG's own stop does (`audio/disconnect` with `{"source":"AMIXER<n>","sink":"MAIN"}`), until neither the kernel nor audiooutputd lists it. LG connecting it again during a session is proven again and taken over again, its words un-mixed again; three takeovers within 10 s end the session. Every restore, the journal replayed at the next start included, gives the input back while the stream it was taken for still plays (the player's process lives and uMS still has that pipeline holding the channel). After the stream has ended it stays disconnected, as LG's own stop leaves it.

## What the TV is compared with

The journal keeps the granted decoder's kernel line from before the session (`Open`, `Connect`, `Start`, `Gain`, `Mute`). After every restore the helper checks that the audio service lists no connection but LG's mixer input given back, the kernel has no input connected and the decoder's line equals the saved one. A difference is reported as `after` on the topic and on the page, not hidden. LG's input services must also have come through: no task may be stuck in the sound card's control lock (read from `/proc`, where nothing can hang; the wedges left `audiooutputd`, `amixer` and `extinput-integration` in uninterruptible sleep in the lock, in `snd_ctl_release` (the writer that closing a control file takes) or in `snd_ctl_elem_info` (a reader); a task counts only when it is still there 0.5 s after it was first seen, because the lock is also held for a moment in normal use), and the HDMI input service must answer a ping (its introspection call, 5 s). A reply of any kind counts; only silence is a fault. While either fails the plugin reports not ready (checked every 10 s while idle).

## What a player calls

All four are loopback only (403 otherwise).

| Request | Body | Answer |
|---|---|---|
| `POST /v1/audio-passthrough/arm` | `{pid, tail, adec}` | 200 `{armed, converter, reason, retry}` |
| `POST /v1/audio-passthrough/flowing` | `{pid}` | 200 `{unmixed, reason}`; a failed un-mix ends the session |
| `GET /v1/audio-passthrough/route` | | `{armed, phase, adec, bypassing, mixed, converterPresent, connected, audible, reason, ageMs, sampledUptimeMs}` |
| `POST /v1/audio-passthrough/release` | `{pid}` | `{released}`; 409 `not_the_holder` for another pid |

- `pid` is the process holding the renderer. `tail` is the 8 lowercase hex digits after `lipsync_` in LG's audio graph for that renderer. `adec` is the granted decoder, 0 to 7; `route` echoes the one the session holds (null without a session). A missing or malformed field is 400.
- A refusal is an answer: `armed: false` with a `reason`, and `retry: true` when waiting may help. The player decodes in software then.
- `release` is synchronous: the TV is put back before it answers. The player calls it only after its sink is stopped or closed, so the exact unmute is safe.

### Feeding the carrier

The carrier is IEC 61937 bursts as 16-bit PCM, 8 channels at 192 kHz: one 61440-byte burst every 20 ms, the 16-bit words big-endian (`ffmpeg -spdif_flags be`; the proven bursts start `f8 72 4e 1f 00 16 ef f0`) while the caps say S16LE. Caps: `audio/x-raw, format=S16LE, layout=interleaved, rate=192000, channels=8, channel-mask=0xc3f, acmod=0, es=true, codec-str=mat, clock-x4=true, data-rate=192000, data-channels=8, data-bps=16`. The `pcm-multi-ch` property is on at caps, off for the first buffer and on from the second. The player arms only once LG's sink holds that first buffer, and keeps holding it until the arm returns, as `lgroute.c` does: on the C4, words un-mixed while `pcm-multi-ch` was still on never brought `Pass-Through`.

The player polls `GET route` and stops sending the moment `bypassing` is false or `ageMs` passes `validForMs` (300). A live sampler's answer is at most about 100 ms old, 150 ms with one sample late; past 300 ms the helper has stopped publishing and nothing vouches for the route. It reports `audible` to nobody; the monitor owns the timing.

## The topic

`audio.passthrough` (`GET /v1/state/audio.passthrough`, also on `/v1/events`):

`schema` (1), `enabled`, `state`, `ready`, `reason`, `firmware` (`match`, `release`, `kernel`, `detail`), `output`, `digital`, `earc`, `words`, `codec`, `carrier` (`format`, `byteOrder`, `rate`, `channels`, `burstBytes`, `burstMs`), `session` (`pid`, `adec`, `mixer` (`AMIXER<n>` the session took over, or null), `tail`, `converter`, `phase`, `bypassing`, `mixed`, `converterPresent`, `connected`, `audible`, `why`), `end` (`why`, `detail`), `pending`, `after`, `sampledUptimeMs`, `validForMs` (300).

- `state`: `off`, `refused` (not the tested firmware), `not-ready` (with the `reason`), `ready`, `armed`, `lost` (the last session ended because LG changed the route; LG's own mix is playing). A firmware version or file that could not be read, and an audio driver that has not answered an earlier read yet, are `not-ready`, not `refused`; the next read looks again.
- `session.phase`: `armed`, `connected`, `unmuting`, `audible`, `ending`.
- `end.why`: `released`, `owner-exited`, `route-lost`, `disabled`, `standby`, `helper-exit`, `fault`, `recovered`.
- `pending` is set while a restore waits (the audio stack is stuck, the decoder will not disconnect, or the driver stopped answering after the words were written and they could not be read back); it is retried every 2 s and the journal stays until it finishes.
- `after` is null when the TV matches what the session found.

## Safety

- Only the exact firmware above is accepted; on any mismatch the plugin refuses and the page says why. A version or file that cannot be read is not a mismatch: the plugin says so, is not ready and reads it again. LG's files are never modified.
- The audio stack is checked before every step that changes LG's route: no task stuck in the sound card's control lock, and the audio service answers. A running session is watched from `/proc` only.
- Sound-card writes run in a child process with a 5 s timeout. A hang stops all further changes until the helper restarts. Reads and writes of LG's driver files (`/proc/aaudio`, `/proc/lgtv-driver`) run on a thread of their own with a 1.5 s deadline: a call that waits on the stuck lock is abandoned, so the route is never held for good, and until it returns every further call to those files fails at once (the page says the audio driver is busy) and the restore waits as pending. Any other file, the firmware's identity and the journal included, is read directly, so a stuck driver never shows as a firmware refusal.
- Never read: `/proc/aaudio/instances/*/prop`, the sound card with `amixer`, `controlC0` with `ioctl`. The Denon is never contacted by this plugin; the page shows the receiver topic's sound mode when the Denon plugin is on.
- If the route is lost, LG's mix is restored and the state says `lost`. Audio is never left silent or noisy.

## What it writes

- One file: the journal, `audio-passthrough/journal` in the helper's data directory. About 250 bytes, under 512 (a boot id, the process, two sets of words, the decoder number and its kernel line, and LG's mixer input the session took); it is rewritten whole through `journal.tmp` and deleted when the session is restored. Nothing is appended, so there is at most the journal and one temporary file.
- No state file, capture or cache of its own. The helper's log gets one line when a journal left by a crash is replayed at start, one when the page or the sound-output follower fails (the same message is logged once until the call works again), and one when a session ends: how often the player polled the route and the longest `ageMs` it was told, which the TV test reads from the log. The helper keeps the current and the previous log, size-capped.
- Its child processes (`amixer`, `luna-send`) and the `audiopass` command set their core file size to 0, so a crash leaves no dump. The helper's own core limit is the helper's rule, not this plugin's.

## Risks

- The audio stack has wedged twice around sessions (`audiooutputd`, `amixer` and `extinput-integration` stuck in uninterruptible sleep in `snd_ctl_release` or `snd_ctl_elem_info`): at 21:21 on 2026-11-22 (`research/truehd-from-marquee/WEDGE.md`) and again at about 23:51, after a TrueHD window at 23:28 and an HDMI2 input switch. The cause is not proven. The working inference is that connecting ADEC1 and toggling its mute left the audio service's view and the kernel's inconsistent before an HDMI input started. This is why the decoder is the granted one, why a session starts only on an idle audio output, why the restore is compared with the journal and why LG's input services are checked afterwards.
- The helper does not read the audio service's own mute flag for the decoder, only the kernel's. The service's flag is kept consistent by sending the same mute calls in the same order and is not compared after a restore; the service's connection list and the kernel's decoder line are.
- If the player still holds `/dev/lg/aren0` when it releases and the input was found unmuted, the input is left muted (the kernel differs from the journal) and `after` says so. It does not block the next session. A player that closes its sink first avoids it.
- LG's HDMI input service has no `getStatus` method (it answers "Unknown method"), so its liveness is its introspection call, `luna://com.webos.service.extinputs.hdmi/com/palm/luna/private/introspection`, which a healthy C4 answers with its method list (`getVRRInfo` among them). That it stays silent on a wedged TV is not measured here, which is why the stuck-task scan comes first. The call is the one constant `HDMI_INTROSPECTION` in `audiopass/src/health.rs`.
- Whether LG's own HDMI2 decode reads `Start=1` on the granted decoder is not measured on this TV. `Start` stayed 0 through every reading of our own session (`research/truehd-from-marquee/run/link10/probes.txt`: before, connected, unmuted, after), and the kernel's notes list it as the decoder's status. If LG's decode does not set it, an HDMI switch to the same source is seen only when it changes the connection, the source or the mute.
- The receiver hung twice in earlier windows; the cause is unknown.
- That disconnecting a decoder found unmuted leaves the kernel mute unchanged is inferred, not measured.

## Not yet tested on the TV

A carrier longer than 30 s; pause, seek and track switches; lip-sync; a real non-Atmos TrueHD stream; Marquee's own playback path; connecting while zeros flow; release on standby; the helper stopping mid-session (time of the restore); an HDMI input switched to during a session (`Start`); the longest `ageMs` of a streaming session (logged) with the reader and sampler threads; how long the `/proc` task scan and a luna call take on the TV; a wedge that shows only as `snd_ctl_release` (matched from `WEDGE.md`, not yet seen by the plugin); a decoder other than ADEC1; the HDMI service probe; the page and the settings switch; `soundOutputDigital` (reported, never read from the TV, not required). Taking LG's mixer input over: uMS's `PCMMC` entry for a Marquee Load is inferred from Marquee's own resource list (`VDEC0, PCMMC4`) and other apps' pipelines, not read during one; giving it back sends `audio/connect` without the `pipelineAttribute` LG's own connect carries.

## Credit

The route is ported from [`josippapez/dts_restore_plus`](https://github.com/josippapez/dts_restore_plus) at `b8fa7db445fff52b1d5183d7a2cfc0cd4169b523` (LGPL-2.1-or-later): `lgroute-graph.h`, `lgroute-control.c` and `lgroute.c`. See `audiopass/NOTICE`.

## Firmware allowlist

Release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`, and the SHA-256 of `libgstlgaudiosink.so` (`61c269de…cc9d2`) and `libpf-1.0.so.1` (`e8a966b2…c36f`); the full digests are in `audiopass/src/firmware.rs`. A new firmware is added only after its route is proven by hand.
