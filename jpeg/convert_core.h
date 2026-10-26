/* The one pixel formula, shared by the scalar loop (convert.c) and the tails of the NEON one (convert_neon.c), so the two cannot
 * drift: r = (ymul * (y - yoff) + 32768 + rv * cr') >> 16 clamped to 0..255, and the same for g and b (cr' = cr - 128). */
#ifndef LUMATV_JPEG_CONVERT_CORE_H
#define LUMATV_JPEG_CONVERT_CORE_H

#include "convert.h"

static inline uint8_t clamp8(int32_t v) {
    return v < 0 ? 0 : v > 255 ? 255 : (uint8_t)v;
}

static inline void rgba_px(const struct yuv_coef *k, int32_t y, int32_t cb, int32_t cr, int ri, int bi, uint8_t *o) {
    int32_t yy = k->ymul * (y - k->yoff) + 32768;
    cb -= 128;
    cr -= 128;
    o[ri] = clamp8((yy + k->rv * cr) >> 16);
    o[1] = clamp8((yy - k->gu * cb - k->gv * cr) >> 16);
    o[bi] = clamp8((yy + k->bu * cb) >> 16);
    o[3] = 255;
}

/* One CbCr pair of the NV12 output: the average (rounded half up) of the (2 >> hshift) x (2 >> vshift) source chroma samples that
 * the output sample covers, clamped at the right and bottom edge (`sw` x `sh` is the chroma grid of the picture). */
static inline void nv12_chroma_px(const struct yuv_image *in, int sw, int sh, int x, int y, uint8_t *o) {
    const int nx = 2 >> in->hshift, ny = 2 >> in->vshift;
    unsigned sb = 0, sr = 0, n = 0;
    for (int j = 0; j < ny; j++) {
        int sy = y * ny + j;
        if (sy >= sh) break;
        const uint8_t *cbr = in->cb + (size_t)sy * in->cstride, *crr = in->cr + (size_t)sy * in->cstride;
        for (int i = 0; i < nx; i++) {
            int sx = x * nx + i;
            if (sx >= sw) break;
            sb += cbr[(size_t)sx * (size_t)in->cstep];
            sr += crr[(size_t)sx * (size_t)in->cstep];
            n++;
        }
    }
    o[0] = (uint8_t)((sb + n / 2) / n);
    o[1] = (uint8_t)((sr + n / 2) / n);
}

#endif
