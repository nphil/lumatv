# opennow-streamer-transport

OpenNOW's `native/opennow-streamer/crates/opennow-streamer-transport` at
[a49b124c](https://github.com/OpenCloudGaming/OpenNOW/commit/a49b124ce31e87113fd4961f6a321fbf0ad1fdc5),
the revision the workspace pins, without `src/bin`. `Cargo.toml` is this workspace's: explicit
dependency versions, no dev-dependencies.

Upstream changes applied:

- [#1450](https://github.com/OpenCloudGaming/OpenNOW/pull/1450): FEC gap waits run side by side
  instead of one after another.

Local changes:

- SRTP rollover resync. Once more than 2^15 packets go unseen, the nearest-index guess lands a
  rollover low and every later packet fails authentication until the session ends; a 72 s
  in-stream menu that paused the receiver did this at 4K120, and a long enough outage does the
  same. A packet that fails at the guessed index is now retried up to three rollovers past the
  highest authenticated index (`SrtpReceiver::authenticate_ahead`), and only a packet that
  authenticates moves the replay window.
- A media timeout while packets arrive and fail authentication is reported as
  `NvstRecovery::AuthenticationFailed` rather than `Timeout`. The stats line counts both
  (`authFailed=`, `rocResync=`).
- A produced-frame stall requests a keyframe after 1 s instead of after the media timeout (8 s for
  GFN). Recovery still waits the media timeout.
