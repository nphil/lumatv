#include "gf_yuv.h"

#include <string.h>

#define MAX_EDGE 8192
#define MAX_HALVINGS 6

typedef struct {
    const uint8_t *p;
    int stride, w, h;
} plane;

typedef struct {
    int32_t i0, i1, f;          /* the two source columns of a destination column and the weight of the second, 0..256 */
} tap;

static int half_of(int v) { return (v + 1) / 2; }

/* How many 2x2 halvings keep the source at or above the destination on both sides. */
static int halvings(int sw, int sh, int dw, int dh) {
    int n = 0;
    while (n < MAX_HALVINGS && half_of(sw) >= dw && half_of(sh) >= dh) {
        sw = half_of(sw);
        sh = half_of(sh);
        n++;
    }
    return n;
}

static size_t align16(size_t v) { return (v + 15) & ~(size_t)15; }

/* The bytes the halved copies of the three planes take, all levels. */
static size_t levels_bytes(int sw, int sh, int n) {
    size_t total = 0;
    int w = sw, h = sh, cw = half_of(sw), ch = half_of(sh);
    for (int i = 0; i < n; i++) {
        w = half_of(w);
        h = half_of(h);
        cw = half_of(cw);
        ch = half_of(ch);
        total += align16((size_t)w * (size_t)h) + 2 * align16((size_t)cw * (size_t)ch);
    }
    return total;
}

size_t gf_yuv_work_size(int sw, int sh, int dw, int dh) {
    if (sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0 || sw > MAX_EDGE || sh > MAX_EDGE || dw > MAX_EDGE || dh > MAX_EDGE) return 0;
    int n = halvings(sw, sh, dw, dh);
    /* the halved planes, the three blended rows, the two column tables, the three sampled rows */
    return levels_bytes(sw, sh, n) + 3 * align16((size_t)sw + 16) + 2 * align16((size_t)dw * sizeof(tap)) + 3 * align16((size_t)dw + 16) + 64;
}

static void halve(const plane *s, uint8_t *d, int dstride, plane *out) {
    int dw = half_of(s->w), dh = half_of(s->h);
    for (int y = 0; y < dh; y++) {
        const uint8_t *r0 = s->p + (size_t)(2 * y) * (size_t)s->stride;
        const uint8_t *r1 = 2 * y + 1 < s->h ? r0 + s->stride : r0;
        uint8_t *o = d + (size_t)y * (size_t)dstride;
        int x = 0;
        for (; 2 * x + 1 < s->w; x++) o[x] = (uint8_t)((r0[2 * x] + r0[2 * x + 1] + r1[2 * x] + r1[2 * x + 1] + 2) >> 2);
        if (x < dw) o[x] = (uint8_t)((r0[2 * x] + r1[2 * x] + 1) >> 1);          /* an odd last column */
    }
    out->p = d;
    out->stride = dstride;
    out->w = dw;
    out->h = dh;
}

/* The source columns and weight of each destination column: the centre of the destination pixel mapped into the source, in 16.16, minus half a pixel. */
static void make_taps(tap *t, int dw, int sw) {
    for (int x = 0; x < dw; x++) {
        int64_t pos = (((int64_t)(2 * x + 1) * sw) << 15) / dw - 32768;
        if (pos < 0) pos = 0;
        int i0 = (int)(pos >> 16);
        int f = (int)((pos >> 8) & 0xff);
        if (i0 >= sw - 1) {
            i0 = sw - 1;
            f = 0;
        }
        t[x].i0 = i0;
        t[x].i1 = i0 + 1 < sw ? i0 + 1 : sw - 1;
        t[x].f = f;
    }
}

/* The two source rows blended by `fy` (0..256): into `row`, or the source row itself when the blend is the first of them. */
static const uint8_t *blend_row(const plane *p, int y0, int y1, int fy, uint8_t *row) {
    const uint8_t *a = p->p + (size_t)y0 * (size_t)p->stride;
    const uint8_t *b = p->p + (size_t)y1 * (size_t)p->stride;
    if (fy == 0 || y0 == y1) return a;
    int g = 256 - fy;
    for (int x = 0; x < p->w; x++) row[x] = (uint8_t)((a[x] * g + b[x] * fy + 128) >> 8);
    return row;
}

/* The row sampled at the destination's columns (`taps`), so that the conversion that follows runs over three plain arrays. */
static void sample_row(const uint8_t *row, const tap *t, int dw, uint8_t *out) {
    for (int x = 0; x < dw; x++) {
        const tap a = t[x];
        out[x] = (uint8_t)((row[a.i0] * (256 - a.f) + row[a.i1] * a.f + 128) >> 8);
    }
}

static void row_taps(const plane *p, int dy, int dh, int *y0, int *y1, int *fy) {
    int64_t pos = (((int64_t)(2 * dy + 1) * p->h) << 15) / dh - 32768;
    if (pos < 0) pos = 0;
    int i0 = (int)(pos >> 16);
    int f = (int)((pos >> 8) & 0xff);
    if (i0 >= p->h - 1) {
        i0 = p->h - 1;
        f = 0;
    }
    *y0 = i0;
    *y1 = i0 + 1 < p->h ? i0 + 1 : p->h - 1;
    *fy = f;
}

static inline int clamp8(int v) { return v < 0 ? 0 : v > 255 ? 255 : v; }

int gf_yuv_to_xrgb(const gf_yuv *src, uint32_t *dst, int dw, int dh, int dstride, void *work) {
    if (!src || !dst || !work || src->width <= 0 || src->height <= 0 || dw <= 0 || dh <= 0 || src->width > MAX_EDGE || src->height > MAX_EDGE || dw > MAX_EDGE ||
        dh > MAX_EDGE || dstride < dw)
        return -1;
    plane pl[3] = {
        {src->plane[0], src->stride[0], src->width, src->height},
        {src->plane[1], src->stride[1], half_of(src->width), half_of(src->height)},
        {src->plane[2], src->stride[2], half_of(src->width), half_of(src->height)},
    };
    uint8_t *w = work;
    int n = halvings(src->width, src->height, dw, dh);
    for (int i = 0; i < n; i++) {
        for (int c = 0; c < 3; c++) {
            int cw = half_of(pl[c].w), ch = half_of(pl[c].h);
            halve(&pl[c], w, cw, &pl[c]);
            w += align16((size_t)cw * (size_t)ch);
        }
    }
    uint8_t *row[3];
    for (int c = 0; c < 3; c++) {
        row[c] = w;
        w += align16((size_t)src->width + 16);
    }
    tap *tl = (tap *)w;                       /* luma columns */
    w += align16((size_t)dw * sizeof(tap));
    tap *tc = (tap *)w;                       /* chroma columns */
    w += align16((size_t)dw * sizeof(tap));
    uint8_t *sy = w, *su = sy + align16((size_t)dw + 16), *sv = su + align16((size_t)dw + 16);
    make_taps(tl, dw, pl[0].w);
    make_taps(tc, dw, pl[1].w);

    /* 16.16 coefficients: Y scale, then the chroma terms of R, G (two) and B */
    int yoff = src->full_range ? 0 : 16;
    int ys, rv, gu, gv, bu;
    if (src->full_range) {
        ys = 65536;
        if (src->bt709) rv = 103206, gu = 12276, gv = 30679, bu = 121609;
        else rv = 91881, gu = 22554, gv = 46802, bu = 116130;
    } else {
        ys = 76309;
        if (src->bt709) rv = 117489, gu = 13975, gv = 34925, bu = 138438;
        else rv = 104597, gu = 25675, gv = 53279, bu = 132201;
    }
    for (int y = 0; y < dh; y++) {
        const uint8_t *r[3];
        for (int c = 0; c < 3; c++) {
            int y0, y1, fy;
            row_taps(&pl[c], y, dh, &y0, &y1, &fy);
            r[c] = blend_row(&pl[c], y0, y1, fy, row[c]);
        }
        uint32_t *o = dst + (size_t)y * (size_t)dstride;
        const uint8_t *ry = r[0];
        if (pl[0].w != dw) {                  /* a luma row at the destination's width is the destination's: no sampling */
            sample_row(r[0], tl, dw, sy);
            ry = sy;
        }
        sample_row(r[1], tc, dw, su);
        sample_row(r[2], tc, dw, sv);
        for (int x = 0; x < dw; x++) {
            int Y = ry[x], U = su[x], V = sv[x];
            int yy = (Y - yoff) * ys + 32768;
            int u = U - 128, v = V - 128;
            int r = clamp8((yy + rv * v) >> 16);
            int g = clamp8((yy - gu * u - gv * v) >> 16);
            int bl = clamp8((yy + bu * u) >> 16);
            o[x] = 0xFF000000u | ((uint32_t)r << 16) | ((uint32_t)g << 8) | (uint32_t)bl;
        }
    }
    return 0;
}
