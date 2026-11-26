# System video recorder: routes on the LG C4

Requested 2026-11-23: a near-zero-cost hardware recorder for streamed and native gameplay, reusable later to stream the TV screen remotely. Research only; nothing has run on the TV.

## Tiers

1. **Game recorder (streamed games): bitstream tee.** The engine copies each encoded video access unit and its Opus audio, before the feeder decodes it, into a RAM ring (ordinary memory, capped at 128 MB: about 15 s at 65 Mbit/s, 100 s at 10 Mbit/s). A button saves the ring as fragmented MP4 (H.264, HEVC, AV1 and Opus all fit, and a truncated file still plays) and uploads it to BeastNAS. No re-encode, HDR10 tags kept, under 1 % of a core for the copy (estimate; the upload is plain HTTP and is to be measured). Tee point: `game/crates/lumagame-feeder/src/lib.rs:411`, shared by GeForce NOW (`gfn/engine/crates/opennow-streamer-platform/src/runtime.rs`) and Punktfunk; GeForce NOW's `recording.rs` stub (lines 33, 37) is the slot. The clip holds the game as sent: no LumaTV/LumaGame overlay, no TV picture processing. The external gamestream engine (Sunshine/Apollo) is not covered yet.
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

**Built.** `game/crates/lumagame-recorder`: a 128 MB cap in all (108 MB ring evicting whole GOPs, 4 MB hand-off, two 8 MB fragments, one for a clip and one for the live feed); the hand-off never blocks decode and counts its drops. Settings › Game recorder (off by default) with a receiver address puts a Recorder drawer in the in-stream menu: Save clip, Record, Long replay.

- Instant clip (long replay off, or the NAS not answering): the ring's last seconds from a keyframe, `PUT /clip?game=`.
- Long replay: the same fragmented MP4, a fragment per keyframe, goes to the receiver as one endless `PUT /live?session=`. The receiver keeps it in segments on `endurancessd/GameReplayBuffer` (8 GB quota, 6 GB cap, emptied at the stream's end and at start) and `POST /save` cuts the last 1, 5, 15 or 30 minutes at a fragment into `Stash/Game Recordings/<game>/<date time>.mp4`. A reach that does not fit the cap at the stream's bitrate is shortened by the receiver (6 GB is about 13 min at 65 Mbit/s and 5 min at 150) and the TV says so once.
- Record: `POST /record/start` writes the feed straight to the share from the next fragment; it stops on `POST /record/stop`, at the stream's end, when the TV stops sending, at the set length (2 h by default) or under 100 GB free.
- The feed is dropped, never waited for: when the receiver is behind by more than the ring or does not answer, the feed ends with one notice and is retried every 5 to 60 s; Save clip uses the ring meanwhile. A change of the stream's format ends the feed and opens a new one, which ends a running recording.

**Cut points.** A cloud stream may carry one keyframe for the whole session (GeForce NOW at 4K120 HEVC did), so the recorder asks for its own: `KeyframeRequest` is the engine's hook (GeForce NOW: the OpenNOW streamer's `MediaFeedback::RequestKeyframe`; Punktfunk: `NativeClient::request_keyframe`) and `lumagame-recorder/src/cut.rs` decides when, on a clock with no sleeps in its tests.

- Record start asks at once and the file starts at the keyframe that follows; if none arrives within 2 s it asks again, three times in all, and gives up at 15 s with "Record could not start: the game sent no keyframe".
- While the TV's memory or long replay is on, a keyframe is asked for every 5 s (setting: 5 s, 10 s, or Only when I save). A keyframe that arrives by itself restarts the timer, so a stream that sends its own costs nothing. Nothing is asked with the recorder off, nor within 3 s of decoder recovery or link trouble (a dropped or lost frame, a stalled gate, a keyframe the decoder itself asked for), and a feed that has fallen behind does not add to it.
- Cost, measured on the Windrose 4K120 HEVC HDR clip: an IDR is 18.8 kB against 4.6 kB for a P frame on a static screen, about three pictures of extra data, so 5 s is roughly 0.5 % more bitrate and 10 s half that. A moving game's IDRs are heavier (assumed up to 1.5 % at 5 s); [not measured on a moving game]. Only when I save asks once at the press, so a clip starts there and runs the instant-clip length forward (capped at length + 5 s and 176 MB).
- The ring evicts whole GOPs and never the newest one; a GOP too big for the ring loses its oldest units and the ring is not ready until the next keyframe. A clip starts at the newest keyframe at or before the reach, else the oldest held. With no keyframe held the drawer says "Waiting for a keyframe" (`ready` on the recorder event) and Save clip, asked for a keyframe, fails with "Save clip could not start: the game sent no keyframe to start from" if none comes. The receiver's cap never drops the chunk holding its newest keyframe.
- Cutting at SEI recovery points (gradual refresh) is not done: a stream that uses them has no frame the bitstream marks as decodable from nowhere without decoding its own references, and only requested IDRs are proven to start a clip.
- The Game recorder page (Settings › Picture and stream) holds the settings once for every integration, with the receiver address starting as `http://192.168.1.69:8795`; a profile saved before it keeps its first integration's values, and a saved `http://192.168.1.69:8765` (the receiver's own container, now gone) reads as the new address.

Plain HTTP on the LAN. The receiver is part of the LumaTV server (`nphilip89/lumatv-server` on the forge: `replay.go`, its Game recordings page and the Unraid template), which answers it only from the home network and the tailnet. Steam and Gamestream engines have no feeder: they need the feeder's tee in their own video path and the same recorder wiring as Punktfunk's session.
