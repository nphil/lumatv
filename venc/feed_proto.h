/* The wire between the compositor's screen recorder and `lumatv-venc feed` (compositor/crates/lumatv-compositor/src/screencap/encoder.rs
 * is the other side; a test there reads these numbers from this file).
 *
 * The recorder starts `lumatv-venc feed -w W -h H -r FPS -b KBPS -g GOP` with a pipe on each of stdin and stdout. Every record is a
 * 24-byte header, little-endian, then its bytes. One frame is in flight at a time.
 *
 *   child -> recorder, once, when the encoder is open:   "LRDY", version, width, height, 0, 0
 *   recorder -> child, per frame:   "LFRM", flags (bit 0: make this frame an IDR), pts_us (u64), size (NV12 bytes that follow), 0
 *   child -> recorder, per frame:   "LAUN", flags (bit 0: keyframe), pts_us (u64, the frame's), size (Annex B bytes that follow;
 *                                    0: the encoder made nothing of the frame), the encoder's own milliseconds for it
 *
 * The child ends when its stdin does: it closes the hardware and writes one JSON line about the run to stderr. Nothing else is
 * ever written to stdout in this mode. */
#ifndef LUMATV_FEED_PROTO_H
#define LUMATV_FEED_PROTO_H

#define FEED_MAGIC_FRAME "LFRM"
#define FEED_MAGIC_UNIT "LAUN"
#define FEED_MAGIC_READY "LRDY"
#define FEED_HEADER_LEN 24
#define FEED_VERSION 1
#define FEED_FLAG_KEY 1

#endif
