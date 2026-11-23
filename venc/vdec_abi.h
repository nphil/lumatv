/* LG video-decoder ABI: /dev/video21 "lgsoc-vdec-decoder" on the C4, a V4L2 stateful memory-to-memory
 * decoder (multi-planar OUTPUT = the compressed stream, one access unit per buffer; CAPTURE = decoded pictures).
 *
 * Source: LG's published O22 tree, kdriver/core/vdec/ipc/local/v4l2_vdec.c (webOS 25, newer than the set;
 * what this set's 5.4.268 kernel really offers is what `lumatv-venc vdeccaps` prints) and
 * v4l2_common.h for the extension controls. The behaviours this code relies on, from that source:
 *   - STREAMON on either queue starts the decoder (vdec_api_open: a decoder channel from the video MCU and a
 *     6 MB code-picture buffer from the "cpb" pool); closing the fd stops it and frees both.
 *   - OUTPUT buffers are copied into that buffer at QBUF time by a worker, so an OUTPUT buffer is free again as
 *     soon as it is dequeued; a buffer with bytesused 0 is dropped; V4L2_DEC_CMD_STOP drains and ends the
 *     CAPTURE queue with a buffer flagged V4L2_BUF_FLAG_LAST and no payload.
 *   - CAPTURE buffers (NV12) are filled by the graphics DMA from the decoded picture at the picture's own
 *     size (cropped width x height, stride = width, chroma plane directly behind luma): the driver resizes the
 *     CAPTURE format to every new picture size and, when V4L2_EVENT_SOURCE_CHANGE is subscribed, queues the
 *     event and holds pictures until the caller has re-armed the CAPTURE queue (STREAMOFF, S_FMT, new
 *     buffers, STREAMON). Without the subscription a larger picture would overrun the buffers, so the
 *     subscription is not optional.
 *   - Timestamps ride through the decoder (vb2 timestamp, copied OUTPUT -> CAPTURE); bit 62 of the
 *     nanosecond value is reserved by the driver.
 *   - The CAPTURE queue wants at least 2 buffers (V4L2_CID_MIN_BUFFERS_FOR_CAPTURE).
 * Never opened here: /dev/video22 (the decoder's display sink), /dev/video20 (live), /dev/video24 (the V4L2
 * encoder: its mmap wrote kernel memory in the first slot; docs/research/venc-result.md). */
#ifndef LUMATV_VDEC_ABI_H
#define LUMATV_VDEC_ABI_H

#include <linux/videodev2.h>
#include <stdint.h>

#define VDEC_DEV "/dev/video21"
#define VDEC_RES_INFO_NAME "res info"

/* LG's extension controls (v4l2-ext-media-vda-vea.h). The set's driver numbers them from
 * V4L2_CID_USER_BASE + 0x9000 (vdeccaps, 2026-11-23: 'res info' 0x00989001, 'svp' 0x00989003, 'app type'
 * 0x00989004, 'decoded size' 0x00989008); the published tree says 0x8700, so a build for another firmware
 * should look the control up by name (QUERY_EXT_CTRL) the way vdec_start does. */
#define VDEC_CID_BASE (V4L2_CID_USER_BASE + 0x9000)
#define VDEC_CID_EXT_RES_INFO (VDEC_CID_BASE + 1)
#define VDEC_CID_EXT_SVP (VDEC_CID_BASE + 3)
#define VDEC_CID_EXT_APP_TYPE (VDEC_CID_BASE + 4)
#define VDEC_CID_EXT_DECODED_SIZE (VDEC_CID_BASE + 8)
#define VDEC_CID_EXT_UNDECODED_SIZE (VDEC_CID_BASE + 9)

/* V4L2_CID_EXT_RES_INFO payload (a pointer control of type 0x1001); natural alignment, 56 bytes as LG's
 * v4l2_ext_resource_info (the driver's elem_size) */
struct vdec_res_info {
    uint8_t core_type[32];
    uint32_t video_port;
    uint32_t max_width, max_height; /* the largest picture the stream will carry */
    struct v4l2_fract max_frame_rate;
    uint8_t hfr;
};
_Static_assert(sizeof(struct vdec_res_info) == 56, "vdec_res_info is 56 bytes");

#endif
