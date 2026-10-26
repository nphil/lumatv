#include "convert_core.h"

#include <string.h>

#ifdef LUMATV_NEON
#include <sys/auxv.h>
#endif

const struct yuv_coef YUV_FULL = {0, 65536, 91881, 22554, 46802, 116130};
const struct yuv_coef YUV_LIMITED = {16, 76309, 104597, 25674, 53278, 132201};

void yuv_to_rgba_scalar(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride) {
    const int ri = bgra ? 2 : 0, bi = bgra ? 0 : 2;
    for (int y = 0; y < in->height; y++) {
        const uint8_t *yr = in->y + (size_t)y * in->ystride;
        uint8_t *o = out + (size_t)y * out_stride;
        if (!in->cb) {
            for (int x = 0; x < in->width; x++) {
                uint8_t g = clamp8((k->ymul * ((int32_t)yr[x] - k->yoff) + 32768) >> 16);
                o[0] = o[1] = o[2] = g;
                o[3] = 255;
                o += 4;
            }
            continue;
        }
        size_t crow = (size_t)(y >> in->vshift) * in->cstride;
        const uint8_t *cbr = in->cb + crow, *crr = in->cr + crow;
        for (int x = 0; x < in->width; x++) {
            size_t c = (size_t)(x >> in->hshift) * (size_t)in->cstep;
            rgba_px(k, yr[x], cbr[c], crr[c], ri, bi, o);
            o += 4;
        }
    }
}

void yuv_to_nv12_scalar(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride) {
    for (int y = 0; y < in->height; y++)
        memcpy(out_y + (size_t)y * out_stride, in->y + (size_t)y * in->ystride, (size_t)in->width);
    int cw = (in->width + 1) / 2, ch = (in->height + 1) / 2;
    if (!in->cb) {
        for (int y = 0; y < ch; y++) memset(out_uv + (size_t)y * out_stride, 128, (size_t)cw * 2);
        return;
    }
    int hs = in->hshift, vs = in->vshift;
    if (hs == 1 && vs == 1 && in->cstep == 2 && in->cr == in->cb + 1) {
        for (int y = 0; y < ch; y++)
            memcpy(out_uv + (size_t)y * out_stride, in->cb + (size_t)y * in->cstride, (size_t)cw * 2);
        return;
    }
    /* One output chroma sample covers (2 >> hs) x (2 >> vs) source samples, clamped at the right and bottom edge. */
    int sw = (in->width + (1 << hs) - 1) >> hs, sh = (in->height + (1 << vs) - 1) >> vs;
    for (int y = 0; y < ch; y++) {
        uint8_t *o = out_uv + (size_t)y * out_stride;
        for (int x = 0; x < cw; x++) nv12_chroma_px(in, sw, sh, x, y, o + (size_t)x * 2);
    }
}

#ifdef LUMATV_NEON
/* arch/arm/include/uapi/asm/hwcap.h; convert_neon.c is built for ARMv7-VE, so the hardware divide must be reported as well. */
#define HWCAP_NEON (1u << 12)
#define HWCAP_IDIVA (1u << 17)
#define HWCAP_IDIVT (1u << 18)

static int have_neon(void) {
    static int cached = -1;
    if (cached < 0) {
        unsigned long caps = getauxval(AT_HWCAP);
        const unsigned long need = HWCAP_NEON | HWCAP_IDIVA | HWCAP_IDIVT;
        cached = (caps & need) == need;
    }
    return cached;
}

void yuv_to_rgba(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride) {
    if (in->cb && have_neon() && convert_neon_ok(in)) yuv_to_rgba_neon(in, k, bgra, out, out_stride);
    else yuv_to_rgba_scalar(in, k, bgra, out, out_stride);
}

void yuv_to_nv12(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride) {
    if (in->cb && have_neon() && convert_neon_ok(in)) yuv_to_nv12_neon(in, out_y, out_uv, out_stride);
    else yuv_to_nv12_scalar(in, out_y, out_uv, out_stride);
}

const char *convert_impl(void) { return have_neon() ? "neon" : "scalar"; }
#else
void yuv_to_rgba(const struct yuv_image *in, const struct yuv_coef *k, int bgra, uint8_t *out, size_t out_stride) {
    yuv_to_rgba_scalar(in, k, bgra, out, out_stride);
}

void yuv_to_nv12(const struct yuv_image *in, uint8_t *out_y, uint8_t *out_uv, size_t out_stride) {
    yuv_to_nv12_scalar(in, out_y, out_uv, out_stride);
}

const char *convert_impl(void) { return "scalar"; }
#endif
