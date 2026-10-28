/* A frame capture from the C4's display pipeline through /dev/video60 (the video tap): NV12 frames at the
 * size asked for, in the driver's own physically contiguous buffers, with the physical address of every
 * buffer known up front so the encoder can read them in place.
 *
 * Guards: every call that can block is under the watchdog (watchdog.h); a buffer is dequeued only after
 * poll says a frame is ready and only while at least one buffer is queued; a buffer is requeued only if
 * this session dequeued it; buffer sizes and physical addresses are checked (non-zero, aligned, inside a
 * listed HMA pool when /proc/hma/status is readable) before anything is mapped or streamed. */
#ifndef LUMATV_VTCAP_H
#define LUMATV_VTCAP_H

#include <stdint.h>

#include "sysops.h"
#include "vt_abi.h"

#define VTCAP_MAX_BUFS 6 /* the driver indexes 0..5 (v4l2_vt.c, vt_get_buffer_address) */

struct vtcap_frame {
    int index;
    uint32_t paddr_y, paddr_c;
    const uint8_t *y, *c; /* CPU mappings of the two planes */
    uint32_t y_len, c_len;
    uint64_t ts_us; /* driver timestamp */
    uint32_t sequence;
};

struct vtcap {
    const struct sysops *sys;
    int fd;
    uint32_t width, height, nbuf;
    enum v4l2_ext_capture_location location;
    int streaming;
    uint8_t *plane[VTCAP_MAX_BUFS][2];
    uint32_t plane_len[VTCAP_MAX_BUFS][2];
    uint32_t paddr_y[VTCAP_MAX_BUFS], paddr_c[VTCAP_MAX_BUFS];
    uint8_t queued[VTCAP_MAX_BUFS]; /* 1 while the driver holds the buffer */
    uint32_t nqueued;
    uint32_t *lowmem; /* a page below 4 GiB for the physical-address query (64-bit callers) */
    uint32_t stride;  /* from PLANE_INFO, 0 if the query failed */
    const char *pool[VTCAP_MAX_BUFS]; /* the HMA pool each buffer's luma lies in, or "?" */
};

/* Opens the tap at the given point (vt_abi.h), asks for width x height (width a multiple of 16) and
 * nbuf buffers (1..6; LG uses 5), divides the display rate by divide (1 = every frame, 2 = every other),
 * maps the buffers, reads and checks their physical addresses and queues them. Streaming starts only
 * with vtcap_start, so a probe can stop here. */
int vtcap_open(struct vtcap *c, const struct sysops *sys, enum v4l2_ext_capture_location location, uint32_t width,
               uint32_t height, uint32_t nbuf, uint32_t divide, char *err, size_t errlen);

int vtcap_start(struct vtcap *c, char *err, size_t errlen);

/* Waits up to timeout_ms for a frame. Returns 0 with f filled, 1 on timeout, 2 when the frame was refused
 * because the content is protected or muted (EACCES/ENODATA: nothing captured, try again later), -1 on
 * error (also when nothing is queued or streaming has not started). */
int vtcap_dequeue(struct vtcap *c, struct vtcap_frame *f, int timeout_ms, char *err, size_t errlen);

/* Gives a dequeued buffer back to the driver. */
int vtcap_requeue(struct vtcap *c, int index, char *err, size_t errlen);

/* Stops streaming, frees the buffers, closes. Safe on an empty struct. */
void vtcap_close(struct vtcap *c);

/* Read-only: the capability record (no buffers, no streaming). */
int vtcap_capability(const struct sysops *sys, struct v4l2_ext_capture_capability_info *cap, char *err, size_t errlen);

const char *vtcap_location_name(enum v4l2_ext_capture_location l);
int vtcap_location_parse(const char *s, enum v4l2_ext_capture_location *l);

#endif
