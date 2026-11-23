/* The wire between the transcode engine (helper/xcode, the other side; its tests read the golden bytes in
 * tests/xcode_test.c) and `lumatv-venc xcode`, the child that owns the TV's video hardware for one job: access
 * units in, H.264 access units out, through the hardware decoder (vdec.c), an optional CPU downscale (scale.c)
 * and the hardware encoder (venc.c). One child per job; ending the job is ending the child.
 *
 * The engine starts `lumatv-venc xcode` with a pipe on each of stdin and stdout; stderr is a log, one line a
 * line. Every record is a 24-byte header, little-endian, then `size` payload bytes. The same header is the
 * access-unit file format of tools/es2au.py (what `lumatv-venc vdec` reads).
 *
 *   engine -> child, once, first:   "XCFG", 0, 0, size 64, 0                              + struct xc_cfg
 *   child -> engine, once:          "XRDY", 0, 0, size 24, 0                              + struct xc_ready
 *   engine -> child, per picture:   "XAUN", flags (bit 0: random access point), pts (the engine's clock, handed back
 *                                    with the H.264 unit made of this picture), size, 0    + one MPEG-2 or H.264 access unit
 *   child -> engine, per picture:   "XOUT", flags (bit 0: IDR, SPS and PPS included), pts (of its picture), size,
 *                                    the encoder's own tenths of a millisecond for it      + one Annex B access unit
 *   engine -> child, any time:      "XIDR" (the next unit is an IDR), "XBPS" (aux: new target in kbit/s)
 *   child -> engine, any time:      "XEVT", 0, 0, size (32, then error text), 0           + struct xc_event
 *
 * Pictures that are dropped to lower the rate, and pictures the decoder could not decode, have no unit.
 * Units come in the order the pictures are shown, which is not the order the access units went in for MPEG-2
 * with B pictures. The child ends when its stdin does (the pictures still inside are finished first), on SIGTERM
 * (at once), or on its own failure after an error event; the exit status says which (XC_EXIT_*). Nothing else is
 * ever written to stdout in this mode. */
#ifndef LUMATV_XCODE_PROTO_H
#define LUMATV_XCODE_PROTO_H

#include <stdint.h>

#define XC_HEADER_LEN 24
#define XC_VERSION 1

#define XC_CONFIG "XCFG"
#define XC_READY "XRDY"
#define XC_AU "XAUN"
#define XC_OUT "XOUT"
#define XC_IDR "XIDR"
#define XC_BITRATE "XBPS"
#define XC_EVENT "XEVT"

#define XC_FLAG_KEY 1 /* XAUN: random access point; XOUT: IDR */

#define XC_CODEC_MPEG2 1
#define XC_CODEC_H264 2

#define XC_CFG_CBR 1 /* xc_cfg.flags */

/* the exit status of the child */
#define XC_EXIT_OK 0
#define XC_EXIT_CONFIG 2   /* the configuration or the stream of records was not valid */
#define XC_EXIT_SETUP 3    /* the hardware would not open, or there was no memory for it */
#define XC_EXIT_DECODER 4  /* the decoder failed or took an access unit it could not */
#define XC_EXIT_STALLED 5  /* access units went in and no picture came out in time */
#define XC_EXIT_ENCODER 6  /* the encoder failed */
#define XC_EXIT_SHAPE 7    /* the stream changed picture size: a new job is needed */
#define XC_EXIT_WEDGED 75  /* a call into the hardware did not return (watchdog.h): the hardware may be stuck */

/* the 24-byte record header, little-endian (every device this runs on is) */
struct xc_head {
    char magic[4];
    uint32_t flags;
    uint64_t pts;
    uint32_t size; /* payload bytes after the header */
    uint32_t aux;
} __attribute__((packed));

/* xc_event.kind */
#define XC_EVT_STATS 1 /* once a second: x = access units in, y = units out, a = pictures dropped, b = last decode ms, c = last encode tenths of ms */
#define XC_EVT_FIRST 2 /* the first picture: a = width, b = height (decoded), c = milliseconds since the first access unit */
#define XC_EVT_ERROR 3 /* a = the exit status that follows; the payload after the struct is text */

struct xc_cfg {
    uint32_t version;    /* XC_VERSION */
    uint32_t codec;      /* XC_CODEC_* */
    uint32_t src_w, src_h; /* the largest picture the station sends (0: 1920x1088); sizes the picture buffers */
    uint32_t out_w, out_h; /* the picture to encode: 0, 0 is the decoded size, otherwise the decoded size scaled down to it (never up) */
    uint32_t keep_every; /* encode every Nth picture, 1 for all (0 is 1) */
    uint32_t fps;        /* the output rate the rate control plans for */
    uint32_t kbps;       /* the video bit rate to aim for */
    uint32_t gop;        /* pictures between IDRs; 0: one IDR, then none until XIDR */
    uint32_t flags;      /* XC_CFG_* */
    uint32_t profile;    /* LX_VENC_H264_PROFILE_* */
    uint32_t ncap;       /* picture buffers (0: 6) */
    uint32_t nout;       /* bitstream buffers (0: 4) */
    uint32_t out_size;   /* bytes of the largest access unit (0: 1.5 MiB) */
    uint32_t reserved;
};

struct xc_ready {
    uint32_t version;
    uint32_t max_w, max_h; /* the picture buffers' size */
    uint32_t out_size;     /* bytes one access unit may be */
    uint32_t ncap;
    uint32_t hma_bytes;    /* what the child holds of the TV's contiguous memory for the picture buffers */
};

struct xc_event {
    uint32_t kind; /* XC_EVT_* */
    uint32_t a, b, c;
    uint64_t x, y;
};

_Static_assert(sizeof(struct xc_head) == XC_HEADER_LEN, "the header is 24 bytes on the wire");
_Static_assert(sizeof(struct xc_cfg) == 64, "xc_cfg is 64 bytes on the wire");
_Static_assert(sizeof(struct xc_ready) == 24, "xc_ready is 24 bytes on the wire");
_Static_assert(sizeof(struct xc_event) == 32, "xc_event is 32 bytes on the wire");

/* the largest access unit or text the child takes in one record */
#define XC_MAX_PAYLOAD (4u << 20)

#endif
