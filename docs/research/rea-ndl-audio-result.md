# NDL audio: the multichannel query and getting 5.1 out

Read-only session 2026-11-15 14:01-14:20 on the OLED65C4 (O22, webOS 10.2.1, firmware 33.22.52), TV in Active
Standby with the receiver (Denon AVR-S760H on HDMI 2) off. Sound receipts in `rea/`; NDL libraries and Ghidra output
under `/data/home/tmp/lumagame-rea-20261115/` (kept out of git). **Measured** = read from the TV or LG's code;
**Inferred** = not observed.

## `NDL_DirectAudioSupportMultiChannel` (measured, decompiled)

`int NDL_DirectAudioSupportMultiChannel(int *isSupported)` returns 0 or -1; the answer goes only through
`*isSupported`, and only when it returns 0:

| `*isSupported` | Meaning |
|---|---|
| 0 | not supported |
| 1 | no connected device takes multichannel PCM |
| 2 | the device takes it, but Digital Sound Out is not Pass Through |
| 3 | multichannel PCM will play |

It returns -1 with `*isSupported` untouched when the implementation library is not loaded, before
`NDL_DirectMediaInit`, or before the player has its audio part. The traps: reading the return value as the answer
(0 then looks like "not supported"), reading `*isSupported` after a -1, and mixing in
`NDL_DirectAudioRegisterCallback`, whose codes 0/1/2 mean 1/2/3 above and which never reports "not supported".

The query makes three synchronous Luna calls:

1. `com.webos.service.audio/getSoundOutput`: `soundOutput` must be `external_optical`, `external_arc` or
   `external_arc_sound_mode_sync`, otherwise 0.
2. settingsservice `sound`: `soundOutputDigital` `auto` or `passThrough` sets a provisional 1, anything else a
   provisional 0; only `passThrough` can reach 3.
3. `com.webos.service.arccontroller/getARCState`: unless `arcActivation` is `eARC`, the provisional answer stands.
   An `advancedEarcCapability` entry with `audioCodingType: "PCM"` and `maxChannelNumber` above 5 gives 3 under
   Pass Through, otherwise 2.

Optical and plain ARC therefore never reach 2 or 3. A failed Luna call in step 1 or 2 makes the query return -1;
a failed `getARCState` leaves the provisional answer.

## What NDL plays

- PCM S16LE interleaved: mono, stereo, `6-channel`. Opus. MP3. No AC-3, E-AC-3 or DTS input, so a compressed
  surround bitstream cannot pass through NDL.
- Opus without a `streamHeader` must be 2 or 6 channels ("Not support audio type, OPUS %dch" otherwise); 6 channels
  get LG's default header: OpusHead v1, mapping family 1, 4 streams, 2 coupled, mapping 0,1,4,5,2,3. A caller's own
  `streamHeader` replaces it and skips the channel check; whether 8-channel Opus then plays is untested.

## This TV now (measured)

`soundOutput` `tv_speaker` (last `external_arc`), `soundOutputDigital` `passThrough`, `eArcSupport` `on`,
`arcActivation` `None`. The stored eARC capability of the receiver lists PCM 8 channels, DD, DD+, TrueHD/MAT and
DTS-HD/DTS:X, with Atmos. With the receiver on over eARC the query should answer 3 (inferred: not read with the
receiver on).

## Measured elsewhere

punktfunk on an LG G5 with a Denon AVC-X3800H (client-webos `docs/NOTES.md`): NDL accepted and played 6-channel PCM,
but the receiver reported 2.0; Opus 5.1 through NDL reached the receiver as 5.1. Gating 5.1 on the query dropped
streams to stereo on default TV settings.

## LumaGame path

- 5.1: hand the stream's Opus 5.1 to NDL as Opus with 6 channels (`gf_ndl.c` already passes `GF_AUDIO_OPUS` with a
  channel count), with LG's default header, or the stream's own `streamHeader` when its layout differs. Decoding to
  6-channel PCM is the weaker route.
- Call the query after `NDL_DirectMediaInit`, read `*isSupported` only on a 0 return, and use it for the stats
  overlay and logs, not to refuse 5.1; webOS folds the layout down where the output cannot carry it (punktfunk, G5).
- 7.1: no PCM mode exists; the only candidate is 8-channel Opus with the stream's own header, untested. Bitstream
  passthrough is not available through NDL.

Untested on this TV: what the AVR-S760H shows for 6-channel PCM and for Opus 5.1 from NDL, the query's answer with
the receiver on, and 8-channel Opus.
