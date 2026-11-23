# Audio passthrough

Audio passthrough sends Dolby TrueHD, Atmos included, from an app that decodes for itself (Marquee) to a receiver over eARC, without LG's mixer touching it. It is off by default at Settings › Sound › Audio passthrough and works only on the TV software it was tested on (LG C4, release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`).

The route is the reusable `audiopass` crate, which has no LumaTV dependencies. The helper module `audio_passthrough` is the plugin around it: the switch and the page, LG's sound-output subscription, the monitor that carries a session through connecting, and the three calls a player makes. Everything that changes the TV's audio runs in the trusted helper and only while the plugin is on. The one exception is putting back what an earlier run changed (its journal), which also runs while the plugin is off. The page adds no animation and no long-dwell element.

Not yet accepted on the TV. The route was proven by hand (real TrueHD Atmos, the receiver showed Dolby Atmos); the plugin around it has only run against fakes.

## What a session does

The TV is stock between sessions. A session changes it only while a player streams, and every change is journalled first (`<helper data>/audio-passthrough/journal`, mode 0600).

1. **Arm.** The player holds LG's renderer (`/dev/lg/aren0`) and calls `arm`. The helper refuses unless the plugin is on, the screen is on, the firmware matches, LG's sound output is the receiver on eARC, the audio services are healthy, nothing is connected to any decoder input and the granted decoder is stopped. It then configures LG's PCM bypass converter for the renderer and un-mixes LG's eARC output words.
2. **Stream.** The player sends the carrier (below). The helper watches `bypassing` through the monitor.
3. **Connect.** After the bypass has held for 3 s the helper connects the granted decoder (ADEC) to the main output. If the kernel shows the input muted it waits 2.8 s, mutes, waits 0.7 s and unmutes; if the input was found unmuted it goes straight to audible. `audible` arrives about 6.7 s after `bypassing`.
4. **Watch.** While connected, LG changing the route (bypass off, mix words changed, converter gone, another decoder connected, the granted one disconnected, re-sourced, started decoding by LG's own input, or muted) ends the session after a 1 s grace. A player that rebuilds its renderer has 5 s to arm again. The watch reads only `/proc` and the kernel; the steps that do ask LG (connect, mute, unmute) are each followed by a fresh reading, so the sample a player is told about (`ageMs`) stays under its 300 ms limit however long LG takes to answer.
5. **Stop.** In this order: the player is told to stop sending, the input is muted, the mute is put back as it was found (only once the player has closed `/dev/lg/aren0`, waiting up to 2 s; otherwise the input stays muted and the page says so), the decoder is disconnected, LG's mix words are written back after a 0.3 s settle, the journal is removed, and the TV is read again and compared with what the session found. When the helper itself is stopping it skips the wait for the renderer and the comparison: the restore then takes about a second of the 4 s the helper allows, an input found unmuted stays muted, and `after` is not computed.

The restore runs on `release`, the player's death, route loss, the plugin being switched off, the TV's standby and the helper's exit. After a crash the journal is replayed at the next helper start, even with the plugin off. A panic in the monitor puts the TV back at once, not when the thread is started again 30 s to 30 min later. A restore the helper's exit cuts short leaves the journal, and the next start finishes it from what the TV reads then. A journal nobody has replayed blocks arming, so LG's original words are never overwritten.

## The decoder is the player's

The player names the decoder input (ADEC, 0 to 7) LG's resource manager granted its renderer, the `index` of its audio sink. The helper uses that one and never assumes ADEC1. It touches nothing an HDMI input could own: a session starts only while the audio service's connection list and the kernel's connected inputs are both empty and the granted decoder is not running. If LG connects another decoder, moves the granted one's source or starts it decoding (`Start=1`) during a session, the session ends and LG's input is left alone: its connection and mute are not touched (LG's mix words are still written back) and `after` says so. A journal replayed after a crash follows the same rule. `Connect` alone cannot say whose input it is: the kernel reads `Connect=HDMI_PORT2` on ADEC1 before, during and after a session.

## What the TV is compared with

The journal keeps the granted decoder's kernel line from before the session (`Open`, `Connect`, `Start`, `Gain`, `Mute`). After every restore the helper checks that the audio service lists no connection, the kernel has no input connected and the decoder's line equals the saved one. A difference is reported as `after` on the topic and on the page, not hidden. LG's input services must also have come through: no task may be stuck in the sound card's control lock (read from `/proc`, where nothing can hang; both wedges left `audiooutputd` and `extinput-integration` in uninterruptible sleep in `snd_ctl_elem_info`; a task counts only when it is still there 0.5 s after it was first seen, because the lock is also held for a moment in normal use), and the HDMI input service must answer a ping (its introspection call, 5 s). A reply of any kind counts; only silence is a fault. While either fails the plugin reports not ready (checked every 10 s while idle).

## What a player calls

All three are loopback only (403 otherwise).

| Request | Body | Answer |
|---|---|---|
| `POST /v1/audio-passthrough/arm` | `{pid, tail, adec}` | 200 `{armed, converter, reason, retry}` |
| `GET /v1/audio-passthrough/route` | | `{armed, phase, bypassing, mixed, converterPresent, connected, audible, reason, ageMs, sampledUptimeMs}` |
| `POST /v1/audio-passthrough/release` | `{pid}` | `{released}`; 409 `not_the_holder` for another pid |

- `pid` is the process holding the renderer. `tail` is the 8 lowercase hex digits after `lipsync_` in LG's audio graph for that renderer. `adec` is the granted decoder, 0 to 7. A missing or malformed field is 400.
- A refusal is an answer: `armed: false` with a `reason`, and `retry: true` when waiting may help. The player decodes in software then.
- `release` is synchronous: the TV is put back before it answers. The player calls it only after its sink is stopped or closed, so the exact unmute is safe.

### Feeding the carrier

The carrier is IEC 61937 bursts as 16-bit PCM, 8 channels at 192 kHz: one 61440-byte burst every 20 ms, the 16-bit words big-endian (`ffmpeg -spdif_flags be`; the proven bursts start `f8 72 4e 1f 00 16 ef f0`) while the caps say S16LE. Caps: `audio/x-raw, format=S16LE, layout=interleaved, rate=192000, channels=8, channel-mask=0xc3f, acmod=0, es=true, codec-str=mat, clock-x4=true, data-rate=192000, data-channels=8, data-bps=16`. The `pcm-multi-ch` property is on at caps, off for the first buffer and on from the second.

The player polls `GET route` and stops sending the moment `bypassing` is false or `ageMs` passes `validForMs` (300). It reports `audible` to nobody; the monitor owns the timing.

## The topic

`audio.passthrough` (`GET /v1/state/audio.passthrough`, also on `/v1/events`):

`schema` (1), `enabled`, `state`, `ready`, `reason`, `firmware` (`match`, `release`, `kernel`, `detail`), `output`, `digital`, `earc`, `words`, `codec`, `carrier` (`format`, `byteOrder`, `rate`, `channels`, `burstBytes`, `burstMs`), `session` (`pid`, `adec`, `tail`, `converter`, `phase`, `bypassing`, `mixed`, `converterPresent`, `connected`, `audible`, `why`), `end` (`why`, `detail`), `pending`, `after`, `sampledUptimeMs`, `validForMs` (300).

- `state`: `off`, `refused` (not the tested firmware), `not-ready` (with the `reason`), `ready`, `armed`, `lost` (the last session ended because LG changed the route; LG's own mix is playing).
- `session.phase`: `armed`, `bypassing`, `connected`, `unmuting`, `audible`, `ending`.
- `end.why`: `released`, `owner-exited`, `route-lost`, `disabled`, `standby`, `helper-exit`, `fault`, `recovered`.
- `pending` is set while a restore waits (the audio stack is stuck, or the decoder will not disconnect); it is retried every 2 s and the journal stays until it finishes.
- `after` is null when the TV matches what the session found.

## Safety

- Only the exact firmware above is accepted; on any mismatch the plugin refuses and the page says why. LG's files are never modified.
- The audio stack is checked before every step that changes LG's route: no task stuck in the sound card's control lock, and the audio service answers. A running session is watched from `/proc` only.
- Sound-card writes run in a child process with a 5 s timeout. A hang stops all further changes until the helper restarts. Reads and writes of LG's driver files run on a thread of their own with a 1.5 s deadline: a call that waits on the stuck lock is abandoned, so the route is never held for good, and until it returns every further call fails at once and the restore waits as pending.
- Never read: `/proc/aaudio/instances/*/prop`, the sound card with `amixer`, `controlC0` with `ioctl`. The Denon is never contacted by this plugin; the page shows the receiver topic's sound mode when the Denon plugin is on.
- If the route is lost, LG's mix is restored and the state says `lost`. Audio is never left silent or noisy.

## What it writes

- One file: the journal, `audio-passthrough/journal` in the helper's data directory. About 250 bytes (a boot id, the process, two sets of words, the decoder number and its kernel line); it is rewritten whole through `journal.tmp` and deleted when the session is restored. Nothing is appended, so there is at most the journal and one temporary file.
- No state file, capture or cache of its own. The helper's log gets one line when a journal left by a crash is replayed at start, one when the page or the sound-output follower fails (the same message is logged once until the call works again), and one when a session ends: how often the player polled the route and the longest `ageMs` it was told, which the TV test reads from the log. The helper keeps the current and the previous log, size-capped.
- Its child processes (`amixer`, `luna-send`) and the `audiopass` command set their core file size to 0, so a crash leaves no dump. The helper's own core limit is the helper's rule, not this plugin's.

## Risks

- The audio stack has wedged twice around sessions (`audiooutputd` and `extinput-integration` stuck in uninterruptible sleep in `snd_ctl_elem_info`): at 21:21 on 2026-11-22 (`research/truehd-from-marquee/WEDGE.md`) and again at about 23:51, after a TrueHD window at 23:28 and an HDMI2 input switch. The cause is not proven. The working inference is that connecting ADEC1 and toggling its mute left the audio service's view and the kernel's inconsistent before an HDMI input started. This is why the decoder is the granted one, why a session starts only on an idle audio output, why the restore is compared with the journal and why LG's input services are checked afterwards.
- The helper does not read the audio service's own mute flag for the decoder, only the kernel's. The service's flag is kept consistent by sending the same mute calls in the same order and is not compared after a restore; the service's connection list and the kernel's decoder line are.
- If the player still holds `/dev/lg/aren0` when it releases and the input was found unmuted, the input is left muted (the kernel differs from the journal) and `after` says so. It does not block the next session. A player that closes its sink first avoids it.
- LG's HDMI input service has no `getStatus` method (it answers "Unknown method"), so its liveness is its introspection call, `luna://com.webos.service.extinputs.hdmi/com/palm/luna/private/introspection`, which a healthy C4 answers with its method list (`getVRRInfo` among them). That it stays silent on a wedged TV is not measured here, which is why the stuck-task scan comes first. The call is the one constant `HDMI_INTROSPECTION` in `audiopass/src/health.rs`.
- Whether LG's own HDMI2 decode reads `Start=1` on the granted decoder is not measured on this TV. `Start` stayed 0 through every reading of our own session (`research/truehd-from-marquee/run/link10/probes.txt`: before, connected, unmuted, after), and the kernel's notes list it as the decoder's status. If LG's decode does not set it, an HDMI switch to the same source is seen only when it changes the connection, the source or the mute.
- The receiver hung twice in earlier windows; the cause is unknown.
- That disconnecting a decoder found unmuted leaves the kernel mute unchanged is inferred, not measured.

## Not yet tested on the TV

A carrier longer than 30 s; pause, seek and track switches; lip-sync; a real non-Atmos TrueHD stream; Marquee's own playback path; connecting while zeros flow; release on standby; the helper stopping mid-session (time of the restore); an HDMI input switched to during a session (`Start`); the longest `ageMs` of a streaming session (logged); a decoder other than ADEC1; the HDMI service probe; the page and the settings switch; `soundOutputDigital` (reported, never read from the TV, not required).

## Credit

The route is ported from [`josippapez/dts_restore_plus`](https://github.com/josippapez/dts_restore_plus) at `b8fa7db445fff52b1d5183d7a2cfc0cd4169b523` (LGPL-2.1-or-later): `lgroute-graph.h`, `lgroute-control.c` and `lgroute.c`. See `audiopass/NOTICE`.

## Firmware allowlist

Release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`, and the SHA-256 of `libgstlgaudiosink.so` (`61c269de…cc9d2`) and `libpf-1.0.so.1` (`e8a966b2…c36f`); the full digests are in `audiopass/src/firmware.rs`. A new firmware is added only after its route is proven by hand.
