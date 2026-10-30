/* A picture of 8-bit 4:2:0 planes scaled and converted to XRGB8888: what the feeder paints into its window when a camera's video is shown as a
 * texture (gf_swdec.c decodes it, the compositor draws the window's buffer in the claim's rectangle). Plain C, integer maths, no dependencies.
 *
 * A source more than twice the destination is first halved by a 2x2 box (as often as it takes to stay at or above the destination), then sampled
 * bilinearly: a 1280x720 stream into a 480x270 box is one halving and a bilinear pass, which is the cost of the picture, and the box filter is
 * what keeps a fence or a roof edge from shimmering. */
#ifndef GF_YUV_H
#define GF_YUV_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
    const uint8_t *plane[3];    /* Y, U, V */
    int stride[3];              /* bytes */
    int width, height;          /* the luma plane's; each chroma plane is ceil(width/2) x ceil(height/2) */
    int full_range;             /* 0 limited (16..235, chroma 16..240), 1 full (0..255) */
    int bt709;                  /* 0 BT.601, 1 BT.709 */
} gf_yuv;

/* Scratch bytes gf_yuv_to_xrgb needs for a source of sw x sh going to dw x dh: the halved copies and the sampling tables. */
size_t gf_yuv_work_size(int sw, int sh, int dw, int dh);

/* Writes the picture at dw x dh into `dst` (words 0xFFRRGGBB, `dstride` words per row). `work` is gf_yuv_work_size bytes, 16-byte aligned.
 * 0, or -1 for a source or destination that is empty or larger than 8192 on a side. */
int gf_yuv_to_xrgb(const gf_yuv *src, uint32_t *dst, int dw, int dh, int dstride, void *work);

#endif
