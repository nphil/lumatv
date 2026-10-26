/* The conversions in NEON, for the set's Cortex-A76 running 32-bit code. Compiled as ARMv7-VE with NEON and hardware divide
 * (build.sh: the hard-float target, because the soft-float one cannot select NEON vector code; nothing here takes or returns a
 * floating-point value, so the calling convention is the same as the rest of the program's), and only called after convert.c
 * has found NEON and divide in the kernel's capability bits and checked the layout with convert_neon_ok().
 *
 * Both functions give the scalar loops' bytes: the RGBA arithmetic is the same 32-bit integer formula (convert_core.h) with a
 * saturating narrow in place of the clamp, and the NV12 averages are the same rounded sums, 16 pixels (RGBA) or 8 chroma samples
 * (NV12) at a time; the edges (the columns that do not fill a vector, the last chroma row of an odd height) go through the scalar
 * code's own helpers. convert_test.c compares them over every layout.
 */
#include <arm_neon.h>
#include <string.h>

#include "convert_core.h"

int convert_neon_ok(const struct yuv_image *in) {
    if (!in->cb || !in->cr || in->hshift < 0 || in->hshift > 1 || in->vshift < 0 || in->vshift > 1) return 0;
    if (in->cstep == 1) return 1;
    return in->cstep == 2 && (in->cr == in->cb + 1 || in->cb == in->cr + 1);
}

/* 8 pixels: y - yoff and the two chroma values minus 128, as signed 16-bit, to the three bytes. */
static inline void px8(const struct yuv_coef *k, int16x8_t y, int16x8_t cb, int16x8_t cr, uint8x8_t *r, uint8x8_t *g, uint8x8_t *b) {
    const int32x4_t half = vdupq_n_s32(32768);
    int16x4_t part[2][3] = {{vget_low_s16(y), vget_low_s16(cb), vget_low_s16(cr)}, {vget_high_s16(y), vget_high_s16(cb), vget_high_s16(cr)}};
    uint16x4_t ro[2], go[2], bo[2];
    for (int h = 0; h < 2; h++) {
        int32x4_t yy = vmlaq_n_s32(half, vmovl_s16(part[h][0]), k->ymul);
        int32x4_t u = vmovl_s16(part[h][1]), v = vmovl_s16(part[h][2]);
        ro[h] = vqmovun_s32(vshrq_n_s32(vmlaq_n_s32(yy, v, k->rv), 16));
        go[h] = vqmovun_s32(vshrq_n_s32(vmlsq_n_s32(vmlsq_n_s32(yy, u, k->gu), v, k->gv), 16));
        bo[h] = vqmovun_s32(vshrq_n_s32(vmlaq_n_s32(yy, u, k->bu), 16));
    }
    *r = vqmovn_u16(vcombine_u16(ro[0], ro[1]));
    *g = vqmovn_u16(vcombine_u16(go[0], go[1]));
    *b = vqmovn_u16(vcombine_u16(bo[0], bo[1]));
}

/* The chroma of 16 pixels starting at pixel x of a row, a byte per pixel (a chroma sample repeated for a subsampled axis). */
static inline void chroma16(const struct yuv_image *in, const uint8_t *cb, const uint8_t *cr, int x, uint8x16_t *u, uint8x16_t *v) {
    const int hs = in->hshift;
    const size_t at = (size_t)(x >> hs) * (size_t)in->cstep;
    if (in->cstep == 1) {
        if (hs) {
            uint8x8_t a = vld1_u8(cb + at), b = vld1_u8(cr + at);
            uint8x8x2_t za = vzip_u8(a, a), zb = vzip_u8(b, b);
            *u = vcombine_u8(za.val[0], za.val[1]);
            *v = vcombine_u8(zb.val[0], zb.val[1]);
        } else {
            *u = vld1q_u8(cb + at);
            *v = vld1q_u8(cr + at);
        }
        return;
    }
    /* semi-planar: CbCr pairs (or CrCb, when cr sits before cb) */
    const int swap = in->cb > in->cr;
    const uint8_t *base = swap ? cr : cb;
    if (hs) {
        uint8x8x2_t p = vld2_u8(base + at);
        uint8x8_t a = swap ? p.val[1] : p.val[0], b = swap ? p.val[0] : p.val[1];
        uint8x8x2_t za = vzip_u8(a, a), zb = vzip_u8(b, b);
        *u = vcombine_u8(za.val[0], za.val[1]);
        *v = vcombine_u8(zb.val[0], zb.val[1]);
    } else {
        uint8x16x2_t p = vld2q_u8(base + at);
        *u = swap ? p.val[1] : p.val[0];
        *v = swap ? p.val[0] : p.val[1];
    }
}

void yuv_to_rgba_neon(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride) {
    const int ri = bgra ? 2 : 0, bi = bgra ? 0 : 2;
    const uint8x8_t yoff = vdup_n_u8((uint8_t)k->yoff), c128 = vdup_n_u8(128), alpha = vdup_n_u8(255);
    for (int y = 0; y < in->height; y++) {
        const uint8_t *yr = in->y + (size_t)y * in->ystride;
        const size_t crow = (size_t)(y >> in->vshift) * in->cstride;
        const uint8_t *cbr = in->cb + crow, *crr = in->cr + crow;
        uint8_t *o = out + (size_t)y * out_stride;
        int x = 0;
        for (; x + 16 <= in->width; x += 16) {
            uint8x16_t yv = vld1q_u8(yr + x), u, v;
            chroma16(in, cbr, crr, x, &u, &v);
            uint8x8x4_t px[2];
            for (int h = 0; h < 2; h++) {
                uint8x8_t y8 = h ? vget_high_u8(yv) : vget_low_u8(yv);
                uint8x8_t u8 = h ? vget_high_u8(u) : vget_low_u8(u), v8 = h ? vget_high_u8(v) : vget_low_u8(v);
                uint8x8_t r, g, b;
                px8(k, vreinterpretq_s16_u16(vsubl_u8(y8, yoff)), vreinterpretq_s16_u16(vsubl_u8(u8, c128)),
                    vreinterpretq_s16_u16(vsubl_u8(v8, c128)), &r, &g, &b);
                px[h].val[0] = bgra ? b : r;
                px[h].val[1] = g;
                px[h].val[2] = bgra ? r : b;
                px[h].val[3] = alpha;
            }
            vst4_u8(o + (size_t)x * 4, px[0]);
            vst4_u8(o + (size_t)x * 4 + 32, px[1]);
        }
        for (; x < in->width; x++) {
            size_t c = (size_t)(x >> in->hshift) * (size_t)in->cstep;
            rgba_px(k, yr[x], cbr[c], crr[c], ri, bi, o + (size_t)x * 4);
        }
    }
}

/* The sums over the source samples one output chroma sample covers, from one source chroma row, for 8 output samples at output
 * column x: u16 lanes, at most 2 samples wide. */
static inline void sums8(const struct yuv_image *in, const uint8_t *cb, const uint8_t *cr, int x, uint16x8_t *sb, uint16x8_t *sr) {
    const int nx = 2 >> in->hshift;
    const size_t at = (size_t)x * (size_t)nx * (size_t)in->cstep;
    if (in->cstep == 1) {
        if (nx == 2) {
            *sb = vpaddlq_u8(vld1q_u8(cb + at));
            *sr = vpaddlq_u8(vld1q_u8(cr + at));
        } else {
            *sb = vmovl_u8(vld1_u8(cb + at));
            *sr = vmovl_u8(vld1_u8(cr + at));
        }
        return;
    }
    const int swap = in->cb > in->cr;
    const uint8_t *base = swap ? cr : cb;
    if (nx == 2) {
        uint8x16x2_t p = vld2q_u8(base + at);
        *sb = vpaddlq_u8(swap ? p.val[1] : p.val[0]);
        *sr = vpaddlq_u8(swap ? p.val[0] : p.val[1]);
    } else {
        uint8x8x2_t p = vld2_u8(base + at);
        *sb = vmovl_u8(swap ? p.val[1] : p.val[0]);
        *sr = vmovl_u8(swap ? p.val[0] : p.val[1]);
    }
}

void yuv_to_nv12_neon(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride) {
    for (int y = 0; y < in->height; y++) memcpy(out_y + (size_t)y * out_stride, in->y + (size_t)y * in->ystride, (size_t)in->width);
    const int hs = in->hshift, vs = in->vshift;
    const int cw = (in->width + 1) / 2, ch = (in->height + 1) / 2;
    const int sw = (in->width + (1 << hs) - 1) >> hs, sh = (in->height + (1 << vs) - 1) >> vs;
    const int nx = 2 >> hs, ny = 2 >> vs;
    for (int y = 0; y < ch; y++) {
        uint8_t *o = out_uv + (size_t)y * out_stride;
        int x = 0;
        if (y * ny + ny <= sh) { /* every source row of this output row exists */
            for (; (x + 8) * nx <= sw; x += 8) {
                uint16x8_t sb, sr;
                sums8(in, in->cb + (size_t)(y * ny) * in->cstride, in->cr + (size_t)(y * ny) * in->cstride, x, &sb, &sr);
                if (ny == 2) {
                    uint16x8_t b2, r2;
                    sums8(in, in->cb + (size_t)(y * ny + 1) * in->cstride, in->cr + (size_t)(y * ny + 1) * in->cstride, x, &b2, &r2);
                    sb = vaddq_u16(sb, b2);
                    sr = vaddq_u16(sr, r2);
                }
                uint8x8x2_t pair;
                if (nx * ny == 4) { /* (sum + 2) / 4 */
                    pair.val[0] = vqmovn_u16(vrshrq_n_u16(sb, 2));
                    pair.val[1] = vqmovn_u16(vrshrq_n_u16(sr, 2));
                } else if (nx * ny == 2) { /* (sum + 1) / 2 */
                    pair.val[0] = vqmovn_u16(vrshrq_n_u16(sb, 1));
                    pair.val[1] = vqmovn_u16(vrshrq_n_u16(sr, 1));
                } else {
                    pair.val[0] = vqmovn_u16(sb);
                    pair.val[1] = vqmovn_u16(sr);
                }
                vst2_u8(o + (size_t)x * 2, pair);
            }
        }
        for (; x < cw; x++) nv12_chroma_px(in, sw, sh, x, y, o + (size_t)x * 2);
    }
}
