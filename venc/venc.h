/* An H.264 encoder session on the C4's hardware encoder through /dev/venc, memory-to-memory: the caller
 * owns NV12 frames at known physical addresses (HMA buffers, or the video tap's capture buffers) and gets
 * one Annex B access unit back per frame, from the encoder's own output ring, with no CPU copy on the
 * way in and one memcpy's worth of bytes on the way out (the caller takes the bytes from the mapping). */
#ifndef LUMATV_VENC_H
#define LUMATV_VENC_H

#include <stddef.h>
#include <stdint.h>

#include "sysops.h"
#include "venc_abi.h"

struct venc_cfg {
    uint32_t width, height; /* NV12 frame size; width is the luma stride */
    uint32_t fps;           /* nominal frame rate the rate control plans for */
    uint32_t gop;           /* frames per I frame; 0 = one I frame then P forever */
    uint32_t kbps;          /* target bit rate; 0 = fixed QP (fixed_qp) */
    uint32_t fixed_qp;
    uint32_t profile; /* LX_VENC_H264_PROFILE_* */
    int cbr;          /* 1: CBR, 0: VBR (both through LX_VENC_IOW_ENC_SET_RC after create) */
};

struct venc_frame {
    const uint8_t *data; /* inside the ES mapping; valid until the next venc_encode */
    uint32_t size;
    int keyframe;    /* 1 for an I frame (SPS/PPS block included) */
    uint32_t hw_ms;  /* encoder's own clock for this frame, as reported */
};

struct venc {
    const struct sysops *sys;
    int fd;
    int id; /* encoder instance, -1 before create */
    LX_VENC_BUFFER_T buf;
    uint8_t *es; /* mapping of the ES (output) buffer, buf.ui32ESBufSize bytes */
    struct venc_cfg cfg;
    uint64_t frames; /* encoded so far */
};

/* Opens /dev/venc, passes the KADP gate, reads the buffer map and maps the output ring. No encoder
 * instance exists yet. Returns 0, or -1 with err. */
int venc_open(struct venc *v, const struct sysops *sys, char *err, size_t errlen);

/* Creates the H.264 encoder instance (one per fd). */
int venc_create(struct venc *v, const struct venc_cfg *cfg, char *err, size_t errlen);

/* Encodes the NV12 frame at paddr_y (luma) and paddr_c (interleaved chroma). out points into the ES
 * mapping. Returns 0, or -1 with err. */
int venc_encode(struct venc *v, uint32_t paddr_y, uint32_t paddr_c, struct venc_frame *out, char *err,
                size_t errlen);

int venc_set_bitrate(struct venc *v, uint32_t kbps, char *err, size_t errlen);
int venc_force_idr(struct venc *v, char *err, size_t errlen);

/* Destroys the instance (if any), unmaps, closes. Safe on an empty struct. */
void venc_close(struct venc *v);

/* Fills the magic the gate wants. Exposed for the tests. */
void venc_kadp_magic(LX_VENC_KADPCALL_PARAM_T *m);

#endif
