/* The conversions against each other and against double precision, over every layout the service passes them: planar and
 * semi-planar (Cb first or Cr first), chroma subsampled 1 or 2 per axis, widths that do and do not fill a vector, odd heights,
 * strides with padding, both coefficient sets, RGBA and BGRA.
 *
 *   convert_test [-n ITERATIONS]
 *
 * The scalar loops are the reference for the NEON ones (this build's NEON functions exist only when LUMATV_NEON is defined:
 * the ARM build, run under qemu-arm or on the set); they must give the same bytes. The scalar RGBA is checked against the
 * double-precision formula within one level, and the scalar NV12 against its own definition (the rounded average of the cell).
 */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "convert.h"

static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint32_t rnd(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (uint32_t)(rng >> 16);
}

static int failures;
#define CHECK(cond, ...)                                                 \
    do {                                                                 \
        if (!(cond)) {                                                   \
            if (failures++ < 10) {                                       \
                printf("FAIL %s:%d: ", __FILE__, __LINE__);              \
                printf(__VA_ARGS__);                                     \
                printf("\n");                                            \
            }                                                            \
        }                                                                \
    } while (0)

static int clampi(double v) {
    int i = (int)floor(v + 0.5);
    return i < 0 ? 0 : i > 255 ? 255 : i;
}

int main(int argc, char **argv) {
    int iters = 3000;
    for (int i = 1; i + 1 < argc; i += 2)
        if (!strcmp(argv[i], "-n")) iters = atoi(argv[i + 1]);
    long cases = 0, neon_cases = 0;
    for (int it = 0; it < iters; it++) {
        int w = 1 + (int)(rnd() % (it % 7 == 0 ? 130 : 70)), h = 1 + (int)(rnd() % 9);
        int hs = (int)(rnd() & 1), vs = (int)(rnd() & 1), semi = (int)(rnd() & 1), swap = (int)(rnd() & 1);
        int ypad = (int)(rnd() % 4) * 16, cpad = (int)(rnd() % 3) * 2;
        size_t ystride = (size_t)w + (size_t)ypad;
        int sw = (w + (1 << hs) - 1) >> hs, sh = (h + (1 << vs) - 1) >> vs;
        size_t cstride = (size_t)sw * (semi ? 2 : 1) + (size_t)cpad;
        size_t ybytes = ystride * (size_t)h, cbytes = cstride * (size_t)sh * (semi ? 1 : 2);
        uint8_t *buf = malloc(ybytes + cbytes + 64);
        for (size_t i = 0; i < ybytes + cbytes + 64; i++) buf[i] = (uint8_t)rnd();
        struct yuv_image im;
        memset(&im, 0, sizeof im);
        im.y = buf;
        im.ystride = ystride;
        im.cstride = cstride;
        im.hshift = hs;
        im.vshift = vs;
        im.width = w;
        im.height = h;
        const uint8_t *c0 = buf + ybytes;
        if (semi) {
            im.cstep = 2;
            im.cb = swap ? c0 + 1 : c0;
            im.cr = swap ? c0 : c0 + 1;
        } else {
            im.cstep = 1;
            const uint8_t *c1 = c0 + cstride * (size_t)sh;
            im.cb = swap ? c1 : c0;
            im.cr = swap ? c0 : c1;
        }
        int gray = it % 11 == 10;
        if (gray) im.cb = im.cr = NULL;

        for (int kk = 0; kk < 2; kk++) {
            const struct yuv_coef *k = kk ? &YUV_LIMITED : &YUV_FULL;
            for (int bgra = 0; bgra < 2; bgra++) {
                size_t os = (size_t)w * 4 + 8;
                uint8_t *a = malloc(os * (size_t)h), *b = malloc(os * (size_t)h);
                memset(a, 0xAA, os * (size_t)h);
                memset(b, 0xAA, os * (size_t)h);
                yuv_to_rgba_scalar(&im, k, bgra, a, os);
                /* the double reference */
                for (int y = 0; y < h; y++)
                    for (int x = 0; x < w; x++) {
                        double yy = im.y[(size_t)y * ystride + (size_t)x], cb = 128, cr = 128;
                        if (im.cb) {
                            size_t c = (size_t)(y >> vs) * cstride + (size_t)(x >> hs) * (size_t)im.cstep;
                            cb = im.cb[c];
                            cr = im.cr[c];
                        }
                        double ys = k->yoff ? 1.164383 * (yy - 16) : yy;
                        double r = ys + (k->yoff ? 1.596027 : 1.402) * (cr - 128);
                        double g = ys - (k->yoff ? 0.391762 : 0.344136) * (cb - 128) - (k->yoff ? 0.812968 : 0.714136) * (cr - 128);
                        double bl = ys + (k->yoff ? 2.017232 : 1.772) * (cb - 128);
                        const uint8_t *p = a + (size_t)y * os + (size_t)x * 4;
                        int want[3] = {clampi(r), clampi(g), clampi(bl)};
                        for (int ch = 0; ch < 3; ch++) {
                            int got = p[bgra ? 2 - ch : ch];
                            CHECK(abs(got - want[ch]) <= 1, "scalar rgba off the double reference: %dx%d (%d,%d) ch %d got %d want %d", w, h, x, y, ch, got, want[ch]);
                        }
                        CHECK(p[3] == 255, "alpha %d", p[3]);
                    }
#ifdef LUMATV_NEON
                if (im.cb && convert_neon_ok(&im)) {
                    yuv_to_rgba_neon(&im, k, bgra, b, os);
                    for (int y = 0; y < h; y++)
                        CHECK(!memcmp(a + (size_t)y * os, b + (size_t)y * os, (size_t)w * 4) && b[(size_t)y * os + (size_t)w * 4] == 0xAA,
                              "neon rgba differs: %dx%d hs%d vs%d semi%d swap%d bgra%d row %d", w, h, hs, vs, semi, swap, bgra, y);
                    neon_cases++;
                }
#endif
                free(a);
                free(b);
                cases++;
            }
        }

        /* NV12 */
        int cw = (w + 1) / 2, ch = (h + 1) / 2;
        size_t ns = (size_t)w + 8;
        uint8_t *ya = malloc(ns * (size_t)(h + ch)), *yb = malloc(ns * (size_t)(h + ch));
        memset(ya, 0xAA, ns * (size_t)(h + ch));
        memset(yb, 0xAA, ns * (size_t)(h + ch));
        yuv_to_nv12_scalar(&im, ya, ya + ns * (size_t)h, ns);
        for (int y = 0; y < h; y++) CHECK(!memcmp(ya + (size_t)y * ns, im.y + (size_t)y * ystride, (size_t)w), "nv12 luma row %d", y);
        for (int y = 0; y < ch; y++)
            for (int x = 0; x < cw; x++) {
                const uint8_t *p = ya + ns * (size_t)h + (size_t)y * ns + (size_t)x * 2;
                if (!im.cb) {
                    CHECK(p[0] == 128 && p[1] == 128, "gray chroma");
                    continue;
                }
                unsigned sb = 0, sr = 0, n = 0;
                for (int j = 0; j < (2 >> vs); j++)
                    for (int i = 0; i < (2 >> hs); i++) {
                        int sy = y * (2 >> vs) + j, sx = x * (2 >> hs) + i;
                        if (sy >= sh || sx >= sw) continue;
                        size_t c = (size_t)sy * cstride + (size_t)sx * (size_t)im.cstep;
                        sb += im.cb[c];
                        sr += im.cr[c];
                        n++;
                    }
                CHECK(p[0] == (sb + n / 2) / n && p[1] == (sr + n / 2) / n, "nv12 chroma (%d,%d) of %dx%d hs%d vs%d", x, y, w, h, hs, vs);
            }
#ifdef LUMATV_NEON
        if (im.cb && convert_neon_ok(&im)) {
            yuv_to_nv12_neon(&im, yb, yb + ns * (size_t)h, ns);
            for (int y = 0; y < h + ch; y++)
                CHECK(!memcmp(ya + (size_t)y * ns, yb + (size_t)y * ns, (size_t)(y < h ? w : cw * 2)) && yb[(size_t)y * ns + (size_t)(y < h ? w : cw * 2)] == 0xAA,
                      "neon nv12 differs: %dx%d hs%d vs%d semi%d swap%d row %d", w, h, hs, vs, semi, swap, y);
        }
#endif
        free(ya);
        free(yb);
        free(buf);
    }
    printf("implementation chosen by the converters: %s; %ld rgba cases, %ld against the NEON functions\n", convert_impl(), cases, neon_cases);
    if (failures) {
        printf("%d failures\n", failures);
        return 1;
    }
    printf("converters agree\n");
    return 0;
}
