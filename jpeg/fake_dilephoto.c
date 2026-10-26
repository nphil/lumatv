/* A stand-in for LG's libdile_photo.so.1 for the host tests (soname libdile_photo.so.1, the three functions lumatv-jpeg calls).
 *
 * It reads the JPEG's own header (size and chroma sampling) and "decodes" it to a synthetic picture in the layout the real
 * library's contract describes (dilejpeg.h): a buffer malloc'd for (w >> shift) * (h >> shift) * 3 bytes of the dimensions the
 * caller gave, Y plane then chroma, the pixel_format code of the chroma sampling and the interleaved flag, dst.width and
 * dst.height set. The picture is a fixed function of the position, so the tests recompute it:
 *     Y(x, y) = (3x + 5y + 7) & 255      Cb(cx, cy) = (11cx + 3cy + 40) & 255      Cr(cx, cy) = (5cx + 13cy + 200) & 255
 * over the buffer's own grid (x, y in the reported size, cx, cy in the chroma grid). What the real block puts in the planes is
 * not known until the TV slot; this checks lumatv-jpeg's handling of every layout it accepts.
 *
 * Environment (read at each call):
 *   FAKE_JPEG_SEMI=0|1   chroma planar (0) or interleaved (1, default)
 *   FAKE_JPEG_ALIGN=N    Y row bytes aligned to N (default 1), FAKE_JPEG_VALIGN=N plane rows aligned to N (default 1)
 *   FAKE_JPEG_PAD=1      report the size rounded up to a multiple of 16, as a macroblock-padding block would
 *   FAKE_JPEG_FLOOR=1    report floor(dim / 2^shift) instead of the ceiling
 *   FAKE_JPEG_OPEN_FAIL=1, FAKE_JPEG_DECODE_FAIL=N (return N), FAKE_JPEG_BAD_FORMAT=1 (an unknown pixel_format code)
 *   FAKE_JPEG_DELAY_US=N sleep inside the decode
 *   FAKE_JPEG_LOG=PATH   append one line per Open, Close and Decode
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "dilejpeg.h"

static unsigned envu(const char *name, unsigned dflt) {
    const char *v = getenv(name);
    return v && *v ? (unsigned)strtoul(v, NULL, 10) : dflt;
}

static void note(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void note(const char *fmt, ...) {
    const char *path = getenv("FAKE_JPEG_LOG");
    if (!path) return;
    FILE *f = fopen(path, "a");
    if (!f) return;
    __builtin_va_list ap;
    __builtin_va_start(ap, fmt);
    vfprintf(f, fmt, ap);
    __builtin_va_end(ap);
    fputc('\n', f);
    fclose(f);
}

static int g_open;

int DILE_JPEG_Open(void) {
    note("open");
    if (envu("FAKE_JPEG_OPEN_FAIL", 0)) return 1;
    g_open = 1;
    return 0;
}

int DILE_JPEG_Close(void) {
    note("close");
    g_open = 0;
    return 0;
}

static size_t align_up(size_t v, size_t a) {
    return a > 1 ? (v + a - 1) / a * a : v;
}

int DILE_JPEG_Decode(const struct dile_jpeg_buffer *src, struct dile_jpeg_buffer *dst, int scaling) {
    if (!src || !dst || !src->data) return 2;
    if (!g_open) return 1;
    unsigned fail = envu("FAKE_JPEG_DECODE_FAIL", 0);
    note("decode scaling=%d", scaling);
    if (fail) return (int)fail;
    unsigned delay = envu("FAKE_JPEG_DELAY_US", 0);
    if (delay) usleep(delay);
    unsigned shift = scaling >= 1 && scaling <= 4 ? (unsigned)scaling - 1 : 0;

    /* the JPEG's frame header */
    const uint8_t *p = src->data;
    size_t n = src->size, i = 2;
    unsigned w = 0, h = 0, nf = 0, hv = 0;
    while (i + 4 <= n) {
        if (p[i] != 0xff) return 1;
        unsigned m = p[i + 1];
        size_t len = ((size_t)p[i + 2] << 8) | p[i + 3];
        if (m >= 0xc0 && m <= 0xc2) {
            h = ((unsigned)p[i + 5] << 8) | p[i + 6];
            w = ((unsigned)p[i + 7] << 8) | p[i + 8];
            nf = p[i + 9];
            hv = p[i + 11];
            break;
        }
        i += 2 + len;
    }
    if (!w || !h) return 1;
    unsigned hs = 0, vs = 0;
    if (nf == 3) {
        hs = (hv >> 4) == 2;
        vs = (hv & 15) == 2;
    }
    unsigned semi = envu("FAKE_JPEG_SEMI", 1);
    uint32_t pf = nf == 1 ? DILE_JPEG_PF_GRAY
                : hs && vs ? (semi ? DILE_JPEG_PF_YUV420_SEMI : DILE_JPEG_PF_YUV420_PLANAR)
                : hs       ? (semi ? DILE_JPEG_PF_YUV422H_SEMI : DILE_JPEG_PF_YUV422H_PLANAR)
                : vs       ? (semi ? DILE_JPEG_PF_YUV224_SEMI : DILE_JPEG_PF_YUV224_PLANAR)
                           : (semi ? DILE_JPEG_PF_YUV444_SEMI : DILE_JPEG_PF_YUV444_PLANAR);
    if (envu("FAKE_JPEG_BAD_FORMAT", 0)) pf = 11;

    unsigned ow = envu("FAKE_JPEG_FLOOR", 0) ? w >> shift : (w + (1u << shift) - 1) >> shift;
    unsigned oh = envu("FAKE_JPEG_FLOOR", 0) ? h >> shift : (h + (1u << shift) - 1) >> shift;
    if (envu("FAKE_JPEG_PAD", 0)) {
        ow = (ow + 15) & ~15u;
        oh = (oh + 15) & ~15u;
    }

    /* the library's own allocation */
    size_t alloc = (size_t)(src->width >> shift) * (src->height >> shift) * 3;
    uint8_t *buf = malloc(alloc ? alloc : 1);
    if (!buf) return 1;
    memset(buf, 0xA5, alloc);

    size_t a = envu("FAKE_JPEG_ALIGN", 1), va = envu("FAKE_JPEG_VALIGN", 1);
    size_t ystride = align_up(ow, a), yrows = align_up(oh, va);
    size_t cw = (ystride + (1u << hs) - 1) >> hs, crows = (yrows + (1u << vs) - 1) >> vs;
    size_t cstride = semi ? cw * 2 : cw;
    size_t need = ystride * yrows + (nf == 1 ? 0 : semi ? cstride * crows : 2 * cstride * crows);
    if (need > alloc) {
        free(buf);
        return 1;
    }
    for (unsigned y = 0; y < oh; y++)
        for (unsigned x = 0; x < ow; x++) buf[y * ystride + x] = (uint8_t)(3 * x + 5 * y + 7);
    if (nf != 1) {
        uint8_t *cb = buf + ystride * yrows;
        uint8_t *cr = semi ? cb + 1 : cb + cstride * crows;
        size_t step = semi ? 2 : 1;
        for (size_t cy = 0; cy < ((oh + (1u << vs) - 1) >> vs); cy++)
            for (size_t cx = 0; cx < ((ow + (1u << hs) - 1) >> hs); cx++) {
                cb[cy * cstride + cx * step] = (uint8_t)(11 * cx + 3 * cy + 40);
                cr[cy * cstride + cx * step] = (uint8_t)(5 * cx + 13 * cy + 200);
            }
    }
    dst->data = buf;
    dst->size = (uint32_t)alloc;
    dst->width = ow;
    dst->height = oh;
    dst->pixel_format = pf;
    return 0;
}
