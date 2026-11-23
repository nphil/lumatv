#include "scale.h"

#include <stdlib.h>
#include <string.h>

/* One axis of the reduction: output sample x covers source samples [x*src/dst, (x+1)*src/dst), the source sample p
 * covers [p, p+1). In units of 1/dst of a source sample every weight is an integer no larger than dst, and the weights
 * of one output sum to src. */
struct axis {
    unsigned *first; /* [dst] the first source sample an output takes */
    unsigned *count; /* [dst] how many */
    unsigned *off;   /* [dst] where its weights start in w */
    uint16_t *w;
};

static void axis_free(struct axis *a)
{
    free(a->first);
    free(a->count);
    free(a->off);
    free(a->w);
    memset(a, 0, sizeof(*a));
}

static int axis_init(struct axis *a, unsigned src, unsigned dst)
{
    unsigned taps = src / dst + 2; /* a span of src/dst samples touches at most this many */
    a->first = malloc(dst * sizeof(unsigned));
    a->count = malloc(dst * sizeof(unsigned));
    a->off = malloc(dst * sizeof(unsigned));
    a->w = malloc((size_t)dst * taps * sizeof(uint16_t));
    if (!a->first || !a->count || !a->off || !a->w) {
        axis_free(a);
        return -1;
    }
    unsigned n = 0;
    for (unsigned x = 0; x < dst; x++) {
        unsigned lo = x * src, hi = lo + src;
        unsigned p0 = lo / dst, p1 = (hi + dst - 1) / dst;
        a->first[x] = p0;
        a->count[x] = p1 - p0;
        a->off[x] = n;
        for (unsigned p = p0; p < p1; p++) {
            unsigned from = p * dst > lo ? p * dst : lo;
            unsigned to = (p + 1) * dst < hi ? (p + 1) * dst : hi;
            a->w[n++] = (uint16_t)(to - from);
        }
    }
    return 0;
}

/* One plane's working set: nch is 1 for luma and 2 for interleaved chroma (the two channels are scaled alike and the
 * output stays interleaved). Sizes are in samples of one channel. */
struct plane {
    unsigned nch, sw, sh, dw, dh;
    struct axis h;
    uint8_t *line;      /* one source row, copied with wide loads */
    uint32_t *rows[2];  /* two source rows already reduced horizontally, scaled by sw */
    long tag[2];        /* which source row each holds, -1 for none */
    uint32_t *acc;      /* the output row's sum over source rows, scaled by sw*sh */
    uint8_t *out;       /* the output row */
    uint64_t recip;     /* 2^40 / (sw*sh) */
};

static void plane_free(struct plane *p)
{
    axis_free(&p->h);
    free(p->line);
    free(p->rows[0]);
    free(p->rows[1]);
    free(p->acc);
    free(p->out);
    memset(p, 0, sizeof(*p));
}

static int plane_init(struct plane *p, unsigned nch, unsigned sw, unsigned sh, unsigned dw, unsigned dh)
{
    memset(p, 0, sizeof(*p));
    p->nch = nch;
    p->sw = sw;
    p->sh = sh;
    p->dw = dw;
    p->dh = dh;
    p->tag[0] = p->tag[1] = -1;
    p->recip = ((1ull << 40) + (uint64_t)sw * sh / 2) / ((uint64_t)sw * sh);
    p->line = malloc((size_t)sw * nch);
    p->rows[0] = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->rows[1] = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->acc = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->out = malloc((size_t)dw * nch);
    if (!p->line || !p->rows[0] || !p->rows[1] || !p->acc || !p->out || axis_init(&p->h, sw, dw) < 0) {
        plane_free(p);
        return -1;
    }
    return 0;
}

/* The horizontal reduction of one source line. Two or three source samples cover an output sample at the usual ratios
 * (2:1, 3:2), so those get straight-line code; nch is a constant at both call sites. */
static inline __attribute__((always_inline)) void hpass(const struct axis *a, const uint8_t *line, uint32_t *row, unsigned dw, const unsigned nch)
{
    for (unsigned x = 0; x < dw; x++) {
        const uint16_t *w = a->w + a->off[x];
        const uint8_t *q = line + (size_t)a->first[x] * nch;
        uint32_t *o = row + (size_t)x * nch;
        switch (a->count[x]) {
        case 2:
            for (unsigned c = 0; c < nch; c++)
                o[c] = (uint32_t)w[0] * q[c] + (uint32_t)w[1] * q[nch + c];
            break;
        case 3:
            for (unsigned c = 0; c < nch; c++)
                o[c] = (uint32_t)w[0] * q[c] + (uint32_t)w[1] * q[nch + c] + (uint32_t)w[2] * q[2 * nch + c];
            break;
        default:
            for (unsigned c = 0; c < nch; c++) {
                uint32_t sum = 0;
                for (unsigned k = 0; k < a->count[x]; k++)
                    sum += (uint32_t)w[k] * q[k * nch + c];
                o[c] = sum;
            }
        }
    }
}

/* Source row s reduced horizontally, from the cache or from the source. Rows are asked for in order, so the one that
 * is kept is the latest. */
static const uint32_t *plane_row(struct plane *p, const uint8_t *src, unsigned stride, long s)
{
    int slot = p->tag[0] == s ? 0 : p->tag[1] == s ? 1 : -1;
    if (slot >= 0)
        return p->rows[slot];
    slot = p->tag[0] < p->tag[1] ? 0 : 1;
    memcpy(p->line, src + (size_t)s * stride, (size_t)p->sw * p->nch);
    uint32_t *row = p->rows[slot];
    if (p->nch == 1)
        hpass(&p->h, p->line, row, p->dw, 1);
    else
        hpass(&p->h, p->line, row, p->dw, 2);
    p->tag[slot] = s;
    return row;
}

static void plane_run(struct plane *p, const uint8_t *src, unsigned sstride, uint8_t *dst, unsigned dstride)
{
    unsigned n = p->dw * p->nch;
    p->tag[0] = p->tag[1] = -1;
    for (unsigned j = 0; j < p->dh; j++) {
        unsigned lo = j * p->sh, hi = lo + p->sh; /* in units of 1/dh of a source row */
        unsigned s0 = lo / p->dh, s1 = (hi + p->dh - 1) / p->dh;
        memset(p->acc, 0, (size_t)n * sizeof(uint32_t));
        for (unsigned s = s0; s < s1; s++) {
            unsigned from = s * p->dh > lo ? s * p->dh : lo;
            unsigned to = (s + 1) * p->dh < hi ? (s + 1) * p->dh : hi;
            const uint32_t *row = plane_row(p, src, sstride, (long)s);
            uint32_t ov = to - from;
            for (unsigned i = 0; i < n; i++)
                p->acc[i] += ov * row[i];
        }
        for (unsigned i = 0; i < n; i++)
            p->out[i] = (uint8_t)(((uint64_t)p->acc[i] * p->recip + (1ull << 39)) >> 40);
        memcpy(dst + (size_t)j * dstride, p->out, n);
    }
}

struct scaler {
    unsigned sw, sh, dw, dh;
    struct plane luma, chroma;
};

struct scaler *scale_new(unsigned sw, unsigned sh, unsigned dw, unsigned dh)
{
    if (!sw || !sh || !dw || !dh || ((sw | sh | dw | dh) & 1) || sw > 3840 || sh > 2176 || dw > sw || dh > sh || dw * 16 < sw || dh * 16 < sh)
        return NULL;
    struct scaler *s = calloc(1, sizeof(*s));
    if (!s)
        return NULL;
    s->sw = sw;
    s->sh = sh;
    s->dw = dw;
    s->dh = dh;
    if (plane_init(&s->luma, 1, sw, sh, dw, dh) < 0 || plane_init(&s->chroma, 2, sw / 2, sh / 2, dw / 2, dh / 2) < 0) {
        scale_free(s);
        return NULL;
    }
    return s;
}

void scale_run(struct scaler *s, const uint8_t *sy, unsigned sy_stride, const uint8_t *sc, unsigned sc_stride, uint8_t *dy,
               unsigned dy_stride, uint8_t *dc, unsigned dc_stride)
{
    if (s->sw == s->dw && s->sh == s->dh) {
        for (unsigned j = 0; j < s->sh; j++)
            memcpy(dy + (size_t)j * dy_stride, sy + (size_t)j * sy_stride, s->sw);
        for (unsigned j = 0; j < s->sh / 2; j++)
            memcpy(dc + (size_t)j * dc_stride, sc + (size_t)j * sc_stride, s->sw);
        return;
    }
    plane_run(&s->luma, sy, sy_stride, dy, dy_stride);
    plane_run(&s->chroma, sc, sc_stride, dc, dc_stride);
}

void scale_free(struct scaler *s)
{
    if (!s)
        return;
    plane_free(&s->luma);
    plane_free(&s->chroma);
    free(s);
}
