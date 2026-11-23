/* A decoder session on the C4's hardware video decoder through /dev/video21 (vdec_abi.h): a compressed
 * access unit in, one NV12 picture out in an HMA buffer at a known physical address, ready for the
 * encoder (venc.h) with no CPU copy of the pixels.
 *
 * Order: vdec_open -> vdec_start (= vdec_setup: formats, bitstream buffers, picture buffers; then
 * vdec_streamon) -> vdec_feed / vdec_get_picture / vdec_put_picture ... -> vdec_close. A picture size the
 * stream did not announce is handled inside vdec_get_picture (the driver's source-change handshake: the
 * picture queue is cycled; the buffers were made for the largest size) and reported through the picture's
 * width and height. */
#ifndef LUMATV_VDEC_H
#define LUMATV_VDEC_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "hma.h"
#include "sysops.h"
#include "vdec_abi.h"

#define VDEC_MAX_OUT 8 /* bitstream buffers */
#define VDEC_MAX_CAP 12 /* picture buffers */

struct vdec_cfg {
    uint32_t codec;       /* V4L2_PIX_FMT_MPEG2 or V4L2_PIX_FMT_H264 */
    uint32_t max_w, max_h; /* the largest picture the stream carries (sizes the picture buffers) */
    uint32_t nout;        /* bitstream buffers (2..VDEC_MAX_OUT) */
    uint32_t ncap;        /* picture buffers (3..VDEC_MAX_CAP) */
    uint32_t out_size;    /* bytes per bitstream buffer; must hold the largest access unit */
    const char *pool;     /* HMA pool for the picture buffers */
};

struct vdec_pic {
    int index;            /* the CAPTURE buffer; hand back with vdec_put_picture */
    uint32_t paddr_y, paddr_c;
    const uint8_t *y, *c; /* CPU mapping of the planes (reading is slow unless the buffer is cached) */
    uint32_t width, height, stride;
    uint64_t seq;         /* the sequence number given to vdec_feed for the access unit this picture came from */
    int last;             /* the end-of-stream marker after vdec_drain: no pixels */
    int error;            /* the decoder flagged the picture as not decoded */
};

struct vdec {
    const struct sysops *sys;
    int fd;
    struct vdec_cfg cfg;
    int streaming;
    int res_info_ok;   /* the decoder accepted the largest-picture hint */
    uint32_t capsize;  /* bytes per picture buffer (the CAPTURE format's sizeimage at the largest size) */
    uint32_t w, h;     /* the picture size the CAPTURE queue is armed for */
    struct {
        uint8_t *map;
        size_t len;
        int queued;
    } out[VDEC_MAX_OUT];
    uint32_t nout;
    struct {
        struct hma_buf mem;
        int dmabuf; /* fd handed to the driver, -1 when none */
        int queued;
        int held;   /* with the caller between vdec_get_picture and vdec_put_picture */
    } cap[VDEC_MAX_CAP];
    uint32_t ncap;
    uint64_t fed, pictures;
    unsigned source_changes, out_errors, cap_errors;
};

/* Opens /dev/video21 and checks it is the decoder. Returns 0, or -1 with err. */
int vdec_open(struct vdec *d, const struct sysops *sys, char *err, size_t errlen);

/* Read-only report of what the node offers (formats, frame sizes, controls, default formats), one line each,
 * to out; no buffers, no streaming. Returns 0, or -1 with err. */
int vdec_report(struct vdec *d, FILE *out, char *err, size_t errlen);

/* Sets the formats and makes both queues' buffers; nothing is started and the video MCU is not asked for
 * anything yet. Returns 0, or -1 with err (the session is then closed). */
int vdec_setup(struct vdec *d, const struct vdec_cfg *cfg, char *err, size_t errlen);

/* Starts both queues: the first STREAMON is where the driver takes a decoder channel from the video MCU. */
int vdec_streamon(struct vdec *d, char *err, size_t errlen);

/* vdec_setup, then vdec_streamon. */
int vdec_start(struct vdec *d, const struct vdec_cfg *cfg, char *err, size_t errlen);

/* Queues one access unit with a sequence number that comes back on its picture. Waits up to timeout_ms for a
 * free bitstream buffer. Returns 0, 1 when no buffer came free in time, or -1 with err. */
int vdec_feed(struct vdec *d, const uint8_t *au, size_t len, uint64_t seq, int timeout_ms, char *err, size_t errlen);

/* Waits up to timeout_ms for a picture. Returns 1 with *pic filled, 0 on timeout, or -1 with err. */
int vdec_get_picture(struct vdec *d, struct vdec_pic *pic, int timeout_ms, char *err, size_t errlen);

/* Hands a picture's buffer back to the decoder. */
int vdec_put_picture(struct vdec *d, int index, char *err, size_t errlen);

/* Ends the stream: the decoder flushes every picture it holds, then answers with a pic->last picture. */
int vdec_drain(struct vdec *d, char *err, size_t errlen);

/* Throws away everything in flight (a new stream follows: a channel change, a seek) and starts again with
 * both queues empty. Pictures the caller still holds stay valid until put back. */
int vdec_flush(struct vdec *d, char *err, size_t errlen);

/* Stops streaming, frees every buffer, closes. Safe on an empty struct. */
void vdec_close(struct vdec *d);

#endif
