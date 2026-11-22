# opennow-streamer-transport

OpenNOW's `native/opennow-streamer/crates/opennow-streamer-transport` at
[a49b124c](https://github.com/OpenCloudGaming/OpenNOW/commit/a49b124ce31e87113fd4961f6a321fbf0ad1fdc5),
the revision the workspace pins, without `src/bin`. `Cargo.toml` is this workspace's: explicit
dependency versions, no dev-dependencies.

Upstream changes applied:

- [#1450](https://github.com/OpenCloudGaming/OpenNOW/pull/1450): FEC gap waits run side by side
  instead of one after another.
- [#1239](https://github.com/OpenCloudGaming/OpenNOW/pull/1239): the video keepalive state is
  renamed `UdpKeepaliveState`; the WebRTC bundle's NATT ping goes through it and takes an injectable send
  function (`run_nvst_webrtc_bundle(.., send_keepalive)`), with the same names and tests as
  upstream. A failed keepalive no longer ends the stream until the failure budget
  (`NvstVideoConfig::timeout`) runs out.
- [a9de957](https://github.com/OpenCloudGaming/OpenNOW/commit/a9de957a5902c54186e9d4b44b1562f0b6beb951)
  (main, 2026-11-11): the reading of the sender's per-picture prefix in `assembled_video_payload`
  (`nvst.rs`, lines 3311-3399) and its `SenderFrameTiming`, ported by hand as `nvst_frame_prefix.rs`
  for observation only. It records the capture time and the `0x200`, `0x201` and `0x202` records;
  the pin's own handling of the prefix (a fixed 20-byte skip for AV1) is untouched, and upstream's
  use of the values is not ported.

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
- Transient send failures on Linux. Upstream tolerates only Windows `WSAENOBUFS` (10055),
  `Interrupted` and `WouldBlock` (`udp_send_error_is_transient`). The Linux errnos EPERM (a
  firewall or conntrack drop), ENOMEM, EADDRNOTAVAIL, ENETDOWN, ENETUNREACH, ENOBUFS, EHOSTDOWN
  and EHOSTUNREACH are tolerated too, matched by raw code so a bare `ErrorKind` stays terminal.
  Upstream's Windows tests keep EPERM and the network-down codes terminal; here they are
  transient on Linux only.
- The failure budget moved into `SendFailureBudget` and every send site owns one, so a success at
  one site never refills another's. Besides the keepalive/NATT pings, the video STUN response and
  receiver report and the bundle ICE burst, str0m datagrams and PONG are covered: a transient
  failure drops that datagram, and a persistent or permanent one stops the stream with the
  original cause. Both receive loops take a second send function (`send_datagram`) for tests.
- Timing-capture recorders, observation only: no decision uses them. `nvst_sender_report.rs` counts
  the RTCP sender reports heard on each track and keeps the first and last readable one.
  `nvst_frame_prefix.rs` lists each video picture's sender prefix for 12 s (at most 2048 rows) once
  a timing capture is armed, and costs one atomic load per picture before that. The capture reads
  both through `NvstFeedbackState` (`sender_reports_json`, `frame_prefixes_json`).
