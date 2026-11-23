# Audio passthrough

Audio passthrough sends Dolby TrueHD, Atmos included, from an app that decodes for itself (Marquee) to a receiver over eARC, without LG's mixer touching it. It is off by default at Settings › Sound › Audio passthrough and works only on the TV software it was tested on (LG C4, release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`).

The route is the reusable `audiopass` crate, which has no LumaTV dependencies. The helper module `audio_passthrough` is the plugin around it: the switch and the page, LG's sound-output subscription, the monitor that carries a session through connecting, and the three calls a player makes. Everything that changes the TV's audio runs in the trusted helper and only while the plugin is on. The one exception is putting back what an earlier run changed (its journal), which also runs while the plugin is off. The page adds no animation and no long-dwell element.

Not yet accepted on the TV. The route was proven by hand (real TrueHD Atmos, the receiver showed Dolby Atmos); the plugin around it has only run against fakes.

## What a session does

The TV is stock between sessions. A session changes it only while a player streams, and every change is journalled first (`<helper data>/audio-passthrough/journal`, mode 0600).

1. **Arm.** The player holds LG's renderer (`/dev/lg/aren0`) and calls `arm`. The helper refuses unless the plugin is on, the screen is on, the firmware matches, LG's sound output is the receiver on eARC, the audio services are healthy, nothing is connected to any decoder input and the granted decoder is stopped. It then configures LG's PCM bypass converter for the renderer and un-mixes LG's eARC output words.
2. **Stream.** The player sends the carrier (below). The helper watches `bypassing` through the monitor.
3. **Connect.** After the bypass has held for 3 s the helper connects the granted decoder (ADEC) to the main output. If the kernel shows the input muted it waits 2.8 s, mutes, waits 0.7 s and unmutes; if the input was found unmuted it goes straight to audible. `audible` arrives about 6.7 s after `bypassing`.
4. **Watch.** While connected, LG changing the route (bypass off, mix words changed, converter gone, another decoder connected, the granted one disconnected, re-sourced or muted) ends the session after a 1 s grace. A player that rebuilds its renderer has 5 s to arm again.
5. **Stop.** In this order: the player is told to stop sending, the input is muted, the mute is put back as it was found (only once the player has closed `/dev/lg/aren0`, waiting up to 2 s; otherwise the input stays muted and the page says so), the decoder is disconnected, LG's mix words are written back after a 0.3 s settle, the journal is removed, and the TV is read again and compared with what the session found.

The restore runs on `release`, the player's death, route loss, the plugin being switched off, the TV's standby and the helper's exit. After a crash the journal is replayed at the next helper start, even with the plugin off. A journal nobody has replayed blocks arming, so LG's original words are never overwritten.

## The decoder is the player's

The player names the decoder input (ADEC, 0 to 7) LG's resource manager granted its renderer, the `index` of its audio sink. The helper uses that one and never assumes ADEC1. It touches nothing an HDMI input could own: a session starts only while the audio service's connection list and the kernel's connected inputs are both empty and the granted decoder is not running. If LG connects another decoder, or moves the granted one's source, during a session, the session ends and the other input is left alone.

## What the TV is compared with

The journal keeps the granted decoder's kernel line from before the session (`Open`, `Connect`, `Start`, `Gain`, `Mute`). After every restore the helper checks that the audio service lists no connection, the kernel has no input connected and the decoder's line equals the saved one. A difference is reported as `after` on the topic and on the page, not hidden. LG's HDMI input service (`extinputs.hdmi getStatus`) must also answer; if it does not, the plugin reports not ready until it does (asked every 10 s while idle).

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
- The audio stack is checked before every step: no audio task stuck in uninterruptible sleep, and the audio service answers.
- Sound-card writes run in a child process with a 5 s timeout. A hang stops all further changes until the helper restarts.
- Never read: `/proc/aaudio/instances/*/prop`, the sound card with `amixer`, `controlC0` with `ioctl`. The Denon is never contacted by this plugin; the page shows the receiver topic's sound mode when the Denon plugin is on.
- If the route is lost, LG's mix is restored and the state says `lost`. Audio is never left silent or noisy.

## Risks

- The audio stack has wedged twice around sessions (`audiooutputd` and `extinput-integration` stuck in uninterruptible sleep in `snd_ctl_elem_info`, 21:21 and 23:51 on 2026-11-23, after a TrueHD window and an HDMI input switch). The cause is not proven. The working inference is that connecting ADEC1 and toggling its mute left the audio service's view and the kernel's inconsistent before an HDMI input started. This is why the decoder is the granted one, why a session starts only on an idle audio output, why the restore is compared with the journal and why the HDMI service is asked afterwards.
- If the player still holds `/dev/lg/aren0` when it releases and the input was found unmuted, the input is left muted (the kernel differs from the journal) and `after` says so. It does not block the next session. A player that closes its sink first avoids it.
- The HDMI input service URI (`luna://com.webos.service.extinputs.hdmi/getStatus`) is not yet verified on the TV; one constant in `audiopass/src/health.rs`. If it does not answer there, the plugin holds itself not ready after the first session.
- The receiver hung twice in earlier windows; the cause is unknown.
- That disconnecting a decoder found unmuted leaves the kernel mute unchanged is inferred, not measured.

## Not yet tested on the TV

A carrier longer than 30 s; pause, seek and track switches; lip-sync; a real non-Atmos TrueHD stream; Marquee's own playback path; connecting while zeros flow; release on standby; a decoder other than ADEC1; the HDMI service probe; the page and the settings switch; `soundOutputDigital` (reported, never read from the TV, not required).

## Credit

The route is ported from [`josippapez/dts_restore_plus`](https://github.com/josippapez/dts_restore_plus) at `b8fa7db445fff52b1d5183d7a2cfc0cd4169b523` (LGPL-2.1-or-later): `lgroute-graph.h`, `lgroute-control.c` and `lgroute.c`. See `audiopass/NOTICE`.

## Firmware allowlist

Release `Rockhopper release 10.2.1-5202 (ponytail-paparoa)`, kernel `5.4.268-329.ptl4tv.2`, and the SHA-256 of `libgstlgaudiosink.so` (`61c269de…cc9d2`) and `libpf-1.0.so.1` (`e8a966b2…c36f`); the full digests are in `audiopass/src/firmware.rs`. A new firmware is added only after its route is proven by hand.
