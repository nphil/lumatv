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

No local changes.
