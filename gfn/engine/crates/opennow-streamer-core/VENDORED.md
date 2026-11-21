# opennow-streamer-core

OpenNOW's `native/opennow-streamer/crates/opennow-streamer-core` at
[a49b124c](https://github.com/OpenCloudGaming/OpenNOW/commit/a49b124ce31e87113fd4961f6a321fbf0ad1fdc5),
the revision the workspace pins. `Cargo.toml` is this workspace's: explicit dependency versions,
no dev-dependencies.

The crate is not a workspace member and its tests are not run here: they need upstream's desktop
platform crate with its `test-runtime` feature. It is checked by building the streamer and by
clippy, which skips non-members under `cargo clippy`; run it as
`RUSTC_WRAPPER=clippy-driver cargo check -p opennow-streamer-core` from `gfn/engine`.

Upstream changes applied:

- [#1451](https://github.com/OpenCloudGaming/OpenNOW/pull/1451): the encoded media queue holds 32
  frames instead of 8, room for the frames a repaired gap releases at once.
- [9d2e0fb](https://github.com/OpenCloudGaming/OpenNOW/commit/9d2e0fb4e7c12e7547a08eafea2f1bb311e40300)
  from [#1306](https://github.com/OpenCloudGaming/OpenNOW/pull/1306): ANNOUNCE returns the
  server's signed `video[0].videoSplitEncodeStripsPerFrame`, which upstream measured taking
  3840x2160@120 from about 96 to 120 fps. Ported by hand: the pin has `microphone_handoff` where
  the commit's parent did not.
- [ac89dd0](https://github.com/OpenCloudGaming/OpenNOW/commit/ac89dd0e6c94a7aee71b638c65b2f0aa6df9ef79)
  from [#1265](https://github.com/OpenCloudGaming/OpenNOW/pull/1265): ANNOUNCE writes
  `chromaFormat` as the official 4:4:4 flag (0 = 4:2:0, 1 = 4:4:4) instead of `chroma_format_idc`
  (1/3). Upstream's field logs: no 10-bit session (all HDR) reached a first video frame with the
  idc values, and the seat ended it with 0x800e8605. Touches `nvst_rtsp_color.rs`, its tests and
  the ANNOUNCE tests in `nvst_rtsp.rs`; applied as the commit has it.

Local changes, all in `build_announce` (`nvst_rtsp.rs`) and taken from what the official web client
(the 2026 play.geforcenow.com bundle LG's app wraps) sends from a TV, with ANNOUNCE tests for each:

- `video[0].mapRtpTimestampsToFrames` is no longer forced to 0, so the server's offer stands and
  RTP timestamps can name frames for the platform's playout pacing. The handoff log records the
  offer as `rtp_frame_stamps`.
- `video[0].dx9EnableHdr:1` always.
- Above 60 fps, frame-rate control (`vqos[0].dfc.*`) is on, except in dynamic streaming mode 1;
  at 120 fps it runs algorithm v2 with a 100 fps floor. Resolution control (`drc.enable:0`) stays
  off.
- At 120 fps, `fakeEncodeFps:120` asks for a fixed 120 fps encode whatever the game renders, with
  `fbcDynamicFpsGrabTimeoutMs:6` and `resControl.cpmRtc.decodeTimeThresholdMs:9`. DLSS frame
  generation otherwise reaches the TV as bursts of two frames.
- 10-bit HEVC gets `video[0].minQp:14`.
