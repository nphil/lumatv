/* YUV planes from the hardware decoder to the two outputs the shell wants: NV12 and RGBA/BGRA. Plain integer C, no
 * floating point (the set's program is soft-float). */
#ifndef LUMATV_JPEG_CONVERT_H
#define LUMATV_JPEG_CONVERT_H

#include <stddef.h>
#include <stdint.h>

/* The planes of one decoded image, as the hardware's buffer holds them. */
struct yuv_image {
    const uint8_t *y;
    const uint8_t *cb, *cr; /* NULL for gray; for a semi-planar buffer cb = base and cr = base + 1, cstep 2 */
    size_t ystride, cstride;
    int cstep;              /* bytes between two chroma samples of one row: 1 planar, 2 semi-planar */
    int hshift, vshift;     /* chroma is subsampled by 1 << shift in each axis */
    int width, height;      /* the pixels to convert (a crop of what the buffer holds) */
};

/* Studio-to-RGB coefficients, 16.16: r = (ymul * (y - yoff) + rv * (cr - 128)) >> 16, and the same for the others. */
struct yuv_coef {
    int32_t yoff, ymul, rv, gu, gv, bu;
};
extern const struct yuv_coef YUV_FULL;    /* JFIF: Y and C use 0..255 */
extern const struct yuv_coef YUV_LIMITED; /* BT.601 studio swing: Y 16..235, C 16..240 */

/* `out` is width * 4 bytes a row at `out_stride`; bgra swaps the first and third byte. These use the NEON implementation where the
 * kernel reports it; both give the same bytes (convert_test.c). */
void yuv_to_rgba(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride);

/* `out_y` is height rows of `out_stride` bytes, `out_uv` is (height + 1) / 2 rows of the same stride, each holding
 * (width + 1) / 2 CbCr pairs. A chroma sample is averaged over the 4:2:0 cell it covers when the source is not 4:2:0
 * already; gray gives 128. */
void yuv_to_nv12(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride);

const char *convert_impl(void); /* "neon" or "scalar" */

void yuv_to_rgba_scalar(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride);
void yuv_to_nv12_scalar(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride);
#ifdef LUMATV_NEON
/* convert_neon.c: ARMv7-VE + NEON only, and only for an image that has chroma in a layout convert_neon_ok() takes (planar, or
 * semi-planar with Cb and Cr adjacent; chroma subsampled by 1 or 2 per axis). */
int convert_neon_ok(const struct yuv_image *in);
void yuv_to_rgba_neon(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride);
void yuv_to_nv12_neon(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride);
#endif

#endif
