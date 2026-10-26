/* The wire between the stream engine (64-bit, gfn/engine, crate opennow-streamer-platform's feeder.rs) and the feeder (this, 32-bit):
 * a Unix stream socket carrying frames of `gf_frame` (8 bytes) followed by `length` bytes of payload. Everything is little-endian and
 * laid out so that a Rust `#[repr(C)]` struct of the same fields matches byte for byte; the engine's feeder.rs tests assert the sizes.
 *
 * Why a socket and not a library: LG's media library is 32-bit only and the engine is 64-bit (docs/research/opennow-result.md, "Facts
 * the NDL path imposes"). At 100 Mbit/s the stream is 12.5 MB/s, far below a Unix socket's rate.
 */
#ifndef GF_PROTO_H
#define GF_PROTO_H

#include <stdint.h>

#define GF_PROTO_VERSION 1
#define GF_MAX_PAYLOAD (8u << 20)   /* one access unit at 100 Mbit/s is under a megabyte; eight is the hard stop */

enum gf_msg {
    /* engine -> feeder */
    GF_LOAD = 1,        /* gf_load */
    GF_VIDEO = 2,       /* gf_media_head then the access unit */
    GF_AUDIO = 3,       /* gf_media_head then one Opus packet, or interleaved S16LE PCM */
    GF_FLUSH = 4,       /* flush the render buffer (after a stall or a keyframe request) */
    GF_UNLOAD = 5,
    GF_STATE = 6,       /* uint8: 1 foreground, 0 background */
    GF_HDR = 7,         /* gf_hdr: static metadata, before or after LOAD (after LOAD it is applied at once) */
    GF_PING = 8,        /* int64 echo */
    /* feeder -> engine */
    GF_LOADED = 16,     /* int32 ok (1), then text (the pipeline's answer or the error) */
    GF_EVENT = 17,      /* gf_event then text */
    GF_ACCEPT = 18,     /* gf_accept: one video unit went into the pipeline */
    GF_DROP = 19,       /* gf_drop: one video unit did not (the engine asks for a keyframe) */
    GF_STATUS = 20,     /* gf_status, once a second while loaded */
    GF_ERROR = 21,      /* int32 code then text; the feeder is unusable afterwards */
    GF_PONG = 22,       /* int64 echo, int64 feeder CLOCK_MONOTONIC ns */
    GF_HELLO = 23,      /* gf_hello: first message from the feeder on connect */
};

typedef struct { uint8_t type, flags; uint16_t reserved; uint32_t length; } gf_frame;

typedef struct {
    uint32_t version;       /* GF_PROTO_VERSION */
    uint32_t library;       /* 1 the real NDL library loaded, 0 no library (the feeder cannot show anything) */
    uint32_t pid;
    uint32_t window;        /* 1 the feeder owns a window, 0 it was given a window id */
    char window_id[32];     /* "_Window_Id_N" */
    char library_name[64];
} gf_hello;

typedef struct {
    uint32_t displayPrimariesX0, displayPrimariesY0, displayPrimariesX1, displayPrimariesY1, displayPrimariesX2, displayPrimariesY2;
    uint32_t whitePointX, whitePointY;
    uint32_t maxDisplayMasteringLuminance, minDisplayMasteringLuminance;
    uint32_t maxContentLightLevel, maxPicAverageLightLevel;
    uint32_t transferCharacteristics, colorPrimaries, matrixCoeffs;
} gf_hdr;                   /* NDL_DIRECTVIDEO_HDR_INFO_T, 60 bytes; primaries in ST 2086 order: 0 green, 1 blue, 2 red */

enum { GF_AUDIO_NONE = 0, GF_AUDIO_OPUS = 1, GF_AUDIO_PCM_S16LE = 2 };

typedef struct {
    uint32_t codec;         /* GF_CODEC_H264 1, GF_CODEC_H265 2, GF_CODEC_AV1 4 */
    uint32_t width, height, fps;
    uint32_t audio;         /* GF_AUDIO_* */
    uint32_t channels, sample_rate;
    uint32_t hdr;           /* 1: apply `hdr_info` after the load */
    gf_hdr hdr_info;
    int32_t drop_threshold; /* NDL_DirectVideoSetFrameDropThreshold after the load; -1 leaves the library's default */
    int32_t pts_unit;       /* what the PTS fields of VIDEO/AUDIO are divided by to reach the library's unit: see gf_pts_unit */
} gf_load;                  /* 100 bytes */

enum gf_pts_unit { GF_PTS_NS = 1, GF_PTS_US = 1000, GF_PTS_MS = 1000000 };

typedef struct {
    int64_t pts_ns;         /* the engine's clock, CLOCK_MONOTONIC-like, nanoseconds; the feeder divides by gf_load.pts_unit */
    uint8_t key;            /* video: 1 for an IDR */
    uint8_t pad[7];
} gf_media_head;            /* 16 bytes */

typedef struct { int32_t type; int32_t pad; int64_t num; } gf_event;           /* NDL's load callback (type, numValue), text follows */

typedef struct { int64_t pts_ns; int32_t queue; int32_t wait_us; } gf_accept;   /* queue: render buffer length before the feed; wait: time spent on a full buffer */

enum { GF_DROP_FULL = 1, GF_DROP_NOT_LOADED = 2, GF_DROP_REFUSED = 3 };
typedef struct { int64_t pts_ns; int32_t reason; int32_t pad; } gf_drop;

typedef struct {
    uint64_t video_fed, video_dropped, audio_fed, audio_dropped;
    uint64_t full_retries;      /* feeds that had to wait at least once */
    uint32_t queue_min, queue_max, queue_mean_x100, queue_p99;    /* render buffer length samples of the last second */
    uint32_t wait_max_us, wait_p99_us;                            /* time spent waiting on a full buffer */
    uint32_t buffer_low, buffer_full;                             /* events 44/46 and 45 in the last second */
    uint32_t errors;
    uint32_t loaded, playing;                                     /* 1 once the pipeline said so */
    uint32_t cma_free_kb, mem_available_kb;                       /* /proc/meminfo, the probe's memory watch */
    uint32_t rss_kb;
    uint32_t audio_avail;                                         /* NDL_DirectAudioGetAvailableBufferSize, or 0 */
} gf_status;                /* 104 bytes */

#endif
