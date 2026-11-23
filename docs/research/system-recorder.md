# System video recorder: routes on the LG C4

Requested 2026-11-23: a near-zero-cost hardware recorder for streamed and native gameplay, reusable later to stream the TV screen remotely. Research only; nothing has run on the TV.

## Tiers

1. **Game recorder (streamed games): bitstream tee.** The engine copies each encoded video access unit and its Opus audio, before the feeder decodes it, into a RAM ring (ordinary memory, capped at 128 MB: about 15 s at 65 Mbit/s, 100 s at 10 Mbit/s). A button saves the ring as Matroska (H.264, HEVC, AV1 and Opus all fit; AV1 in MPEG-TS is still a draft) and uploads it to BeastNAS. No re-encode, HDR10 tags kept, under 1 % of a core for the copy (estimate; the encrypted upload costs more and is to be measured). Tee point: `game/crates/lumagame-feeder/src/lib.rs:411`, shared by GeForce NOW (`gfn/engine/crates/opennow-streamer-platform/src/runtime.rs`) and Punktfunk; GeForce NOW's `recording.rs` stub (lines 33, 37) is the slot. The clip holds the game as sent: no LumaTV/LumaGame overlay, no TV picture processing. The external gamestream engine (Sunshine/Apollo) is not covered yet.
2. **Stills and LumaTV's own recorder.** LG's capture service (`executeOneShot`) takes whole-screen stills; calling it from LumaTV's role is untested. `compositor/crates/lumatv-compositor/src/screencap/` records LumaTV's own pages (720p30, 4 Mbit/s, capped 256 MiB and 5 min; video planes are black); its screenshot passed on the TV, its encoder has not run.
3. **Whole-screen video: tap → hardware encoder.** `/dev/video60` capture to `/dev/venc`, 1080p30 or 720p60, 10–25 ms per frame encode, 30–50 ms latency (estimates). Designed offline (`docs/research/venc-result.md`, `capture-result.md`), never run. The encoder shares a chip with the decoder, so heavy streams may drop frames while recording; the ambient-light tap and the recorder cannot both hold the tap.
4. **Remote view.** A relay on BeastNAS (WebRTC under a second, SRT a fraction of a second, RTSP 1–2 s; estimates) published through HomeLabber `expose_apply` (internal) at `<name>.nateshome.net`. Needs tier 2 or 3.

## Constraints

- Nothing is stored on the TV: clips go to BeastNAS; the ring is memory only. The contiguous pool the video chips use has 50–70 MB free, so the ring uses ordinary memory.
- Every new process sets `RLIMIT_CORE` to 0. A recording indicator dims or hides after a short dwell.
- Protected video (streaming apps, HDCP inputs) is black on every route and is not worked around.
- `/dev/video24` locked the TV until a cold boot: never opened. `/dev/lg/venc0` maps arbitrary physical memory: not used.
- Whole-TV audio has no mapped capture path; live TV audio is AC-3 on its own route (`docs/research/live-tv-capture.md`).

## TV tests, in order

1. GeForce NOW dry run of the tee into the RAM ring (counts only), a saved clip played on a PC, CPU and memory logged; one LG still from LumaTV's role.
2. Encoder test on `/dev/venc` (owner approval required).
3. Tap during GeForce NOW; LG continuous capture if the set supports it.
4. One-hour soak of tee and tap: drops, temperature, memory, 65 Mbit/s upload.
