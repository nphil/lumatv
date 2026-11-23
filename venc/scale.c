#include "scale.h"

#include <stdlib.h>
#include <string.h>

#if defined(__ARM_NEON) && defined(__aarch64__)
#include <arm_neon.h>
#define SCALE_VEC 1
#elif defined(SCALE_NEON_EMU)
/* the x86_64 host tests: the same vector code over portable stand-ins for the NEON it uses */
#include "neon_emu.h"
#define SCALE_VEC 1
#endif

#define LINE_PAD 64 /* bytes past a source row that the vector passes may load */
#ifdef SCALE_VEC
#define VB 16    /* bytes of an output row per vector block: 16 luma samples, or 8 Cb/Cr pairs */
#define VMAXC 18 /* the most source rows one output row takes, at the 16:1 limit scale_new allows */
#endif

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
    uint8_t *line;      /* one source row, copied with wide loads, LINE_PAD bytes of slack after it */
    uint32_t *rows[2];  /* two source rows already reduced horizontally, scaled by sw */
    long tag[2];        /* which source row each holds, -1 for none */
    uint32_t *acc;      /* the output row's sum over source rows, scaled by sw*sh */
    uint8_t *out;       /* the output row, padded to whole vector blocks */
    uint64_t recip;     /* 2^40 / (sw*sh) */
#ifdef SCALE_VEC
    int vec;            /* the vector passes run this plane (vec_init says when they cannot) */
    unsigned nblk;      /* vector blocks per output row */
    unsigned hk;        /* taps per output in the horizontal tables, 0 where that pass stays scalar */
    unsigned hwin;      /* table registers one block's source window takes: 2 or 4 */
    uint32_t *blo;      /* [nblk] where in the line each block's source window starts */
    uint8_t *tidx;      /* [nblk][hk][VB] the window byte each lane takes for each tap */
    uint16_t *tw;       /* [nblk][hk][VB] and its weight, 0 for a tap the output does not have */
    unsigned nslots;    /* horizontally reduced rows kept: the most source rows one output row takes, at least 2 */
    uint32_t *vrows;    /* [nslots][nblk*VB] */
    long vtag[VMAXC];   /* which source row each holds, -1 for none */
#endif
};

static void plane_free(struct plane *p)
{
    axis_free(&p->h);
    free(p->line);
    free(p->rows[0]);
    free(p->rows[1]);
    free(p->acc);
    free(p->out);
#ifdef SCALE_VEC
    free(p->blo);
    free(p->tidx);
    free(p->tw);
    free(p->vrows);
#endif
    memset(p, 0, sizeof(*p));
}

#ifdef SCALE_VEC
/* Sets up the vector passes of a plane, or leaves p->vec 0 where they cannot give the scalar result. The scalar last
 * step is floor((acc*recip + 2^39) / 2^40), which is (floor(acc*recip / 2^31) + 2^8) >> 9, and SQDMULH is that inner
 * floor for a signed 32-bit acc and recip: so recip must stay under 2^31 (planes over 512 samples) and acc, at most
 * 255*sw*sh, under 2^31 (scale_new's size limits keep it there). */
static int vec_init(struct plane *p)
{
    const struct axis *a = &p->h;
    const unsigned per = VB / p->nch; /* outputs per block */
    p->nblk = (p->dw * p->nch + VB - 1) / VB;
    if (p->recip >= (1u << 31) || (uint64_t)p->sw * p->sh * 255 >= (1u << 31))
        return 0;
    unsigned maxc = 2;
    for (unsigned j = 0; j < p->dh; j++) {
        unsigned lo = j * p->sh;
        unsigned c = (lo + p->sh + p->dh - 1) / p->dh - lo / p->dh;
        if (c > maxc)
            maxc = c;
    }
    if (maxc > VMAXC)
        return 0;
    p->nslots = maxc;
    p->vrows = calloc((size_t)p->nslots * p->nblk * VB, sizeof(uint32_t));
    if (!p->vrows)
        return -1;
    p->vec = 1;

    /* The horizontal pass looks a block's source window up with TBL, which reaches 64 bytes: ratios of more than about
     * 3.8:1 keep the scalar pass. */
    unsigned taps = 0, span = 0;
    for (unsigned x = 0; x < p->dw; x++)
        if (a->count[x] > taps)
            taps = a->count[x];
    for (unsigned b = 0; b < p->nblk; b++) {
        unsigned x0 = b * per, x1 = x0 + per < p->dw ? x0 + per : p->dw;
        unsigned s = (a->first[x1 - 1] + a->count[x1 - 1] - a->first[x0]) * p->nch;
        if (s > span)
            span = s;
    }
    if (span > 4 * VB)
        return 0;
    p->blo = malloc((size_t)p->nblk * sizeof(uint32_t));
    p->tidx = calloc((size_t)p->nblk * taps, VB);
    p->tw = calloc((size_t)p->nblk * taps * VB, sizeof(uint16_t));
    if (!p->blo || !p->tidx || !p->tw)
        return -1;
    for (unsigned b = 0; b < p->nblk; b++) {
        unsigned x0 = b * per, lo = a->first[x0] * p->nch;
        p->blo[b] = lo;
        for (unsigned t = 0; t < VB; t++) {
            unsigned x = x0 + t / p->nch;
            if (x >= p->dw)
                break;
            for (unsigned k = 0; k < a->count[x]; k++) {
                size_t at = ((size_t)b * taps + k) * VB + t;
                p->tidx[at] = (uint8_t)((a->first[x] + k) * p->nch + t % p->nch - lo);
                p->tw[at] = a->w[a->off[x] + k];
            }
        }
    }
    p->hk = taps;
    p->hwin = span <= 2 * VB ? 2 : 4;
    return 0;
}
#endif

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
    p->line = calloc(1, (size_t)sw * nch + LINE_PAD);
    p->rows[0] = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->rows[1] = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->acc = malloc((size_t)dw * nch * sizeof(uint32_t));
    p->out = calloc(1, ((size_t)dw * nch + 15) & ~(size_t)15);
    if (!p->line || !p->rows[0] || !p->rows[1] || !p->acc || !p->out || axis_init(&p->h, sw, dw) < 0) {
        plane_free(p);
        return -1;
    }
#ifdef SCALE_VEC
    if (vec_init(p) < 0) {
        plane_free(p);
        return -1;
    }
#endif
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

#ifdef SCALE_VEC
/* The vector passes add the scalar passes' products in another order, which changes nothing: every sum is below 2^32
 * (at most 255*sw*sh), so the u32 arithmetic is exact in any order. */

/* One tap of a block: the 16 gathered source bytes widened and multiplied into the four sums by that tap's weights. */
static inline __attribute__((always_inline)) void hmac(uint32x4_t acc[4], uint8x16_t g, const uint16_t *wt)
{
    uint16x8_t lo = vmovl_u8(vget_low_u8(g)), hi = vmovl_high_u8(g);
    uint16x8_t wl = vld1q_u16(wt), wh = vld1q_u16(wt + 8);
    acc[0] = vmlal_u16(acc[0], vget_low_u16(lo), vget_low_u16(wl));
    acc[1] = vmlal_high_u16(acc[1], lo, wl);
    acc[2] = vmlal_u16(acc[2], vget_low_u16(hi), vget_low_u16(wh));
    acc[3] = vmlal_high_u16(acc[3], hi, wh);
}

/* hpass for the line in p->line, a block of 16 output bytes at a time: the k-th source sample of every output is looked
 * up in the block's source window (a table of 2 or 4 registers) and multiplied by its weight. nwin and taps are
 * constants where it matters, the 3:2 and 4:3 ratios (2 taps, a window of under 32 bytes). */
static inline __attribute__((always_inline)) void hpass_vec(const struct plane *p, uint32_t *row, const unsigned nwin, const unsigned taps)
{
    for (unsigned b = 0; b < p->nblk; b++) {
        const uint8_t *win = p->line + p->blo[b];
        const uint8_t *ix = p->tidx + (size_t)b * taps * VB;
        const uint16_t *wt = p->tw + (size_t)b * taps * VB;
        uint32x4_t acc[4] = { vdupq_n_u32(0), vdupq_n_u32(0), vdupq_n_u32(0), vdupq_n_u32(0) };
        if (nwin == 2) {
            const uint8x16x2_t t = vld1q_u8_x2(win);
            for (unsigned k = 0; k < taps; k++)
                hmac(acc, vqtbl2q_u8(t, vld1q_u8(ix + k * VB)), wt + k * VB);
        } else {
            const uint8x16x4_t t = vld1q_u8_x4(win);
            for (unsigned k = 0; k < taps; k++)
                hmac(acc, vqtbl4q_u8(t, vld1q_u8(ix + k * VB)), wt + k * VB);
        }
        for (unsigned i = 0; i < 4; i++)
            vst1q_u32(row + (size_t)b * VB + 4 * i, acc[i]);
    }
}

/* The line in p->line reduced horizontally into row. */
static void hrow_vec(const struct plane *p, uint32_t *row)
{
    if (!p->hk) {
        if (p->nch == 1)
            hpass(&p->h, p->line, row, p->dw, 1);
        else
            hpass(&p->h, p->line, row, p->dw, 2);
    } else if (p->hwin == 2 && p->hk == 2) {
        hpass_vec(p, row, 2, 2);
    } else if (p->hwin == 2) {
        hpass_vec(p, row, 2, p->hk);
    } else {
        hpass_vec(p, row, 4, p->hk);
    }
}

/* The output row from c horizontally reduced rows, 16 bytes at a time with the sums kept in registers. The last step
 * is plane_run's, as vec_init derives: SQDMULH, then the +2^8 and >>9 as one rounding narrow. The result is at most 255
 * (the scalar step's cast to a byte never wraps), so the saturating narrow never acts. c is a constant for 1 and 2. */
static inline __attribute__((always_inline)) void vpass_vec(const uint32_t *const *rows, const uint32_t *ov, const unsigned c, const unsigned nblk, const int32x4_t recip, uint8_t *out)
{
    for (unsigned b = 0; b < nblk; b++) {
        const size_t o = (size_t)b * VB;
        uint32x4_t acc[4];
        for (unsigned i = 0; i < 4; i++)
            acc[i] = vmulq_n_u32(vld1q_u32(rows[0] + o + 4 * i), ov[0]);
        for (unsigned s = 1; s < c; s++)
            for (unsigned i = 0; i < 4; i++)
                acc[i] = vmlaq_n_u32(acc[i], vld1q_u32(rows[s] + o + 4 * i), ov[s]);
        int16x4_t h[4];
        for (unsigned i = 0; i < 4; i++)
            h[i] = vrshrn_n_s32(vqdmulhq_s32(vreinterpretq_s32_u32(acc[i]), recip), 9);
        vst1q_u8(out + o, vcombine_u8(vqmovun_s16(vcombine_s16(h[0], h[1])), vqmovun_s16(vcombine_s16(h[2], h[3]))));
    }
}

/* plane_run on the vector passes: the same rows in the same order, but a ring of nslots reduced rows, so an output row
 * of more than two source rows still finds them all. */
static void plane_run_vec(struct plane *p, const uint8_t *src, unsigned sstride, uint8_t *dst, unsigned dstride)
{
    const int32x4_t recip = vdupq_n_s32((int32_t)p->recip);
    const size_t rowlen = (size_t)p->nblk * VB;
    const unsigned n = p->dw * p->nch;
    const uint32_t *rows[VMAXC];
    uint32_t ov[VMAXC];
    for (unsigned i = 0; i < p->nslots; i++)
        p->vtag[i] = -1;
    for (unsigned j = 0; j < p->dh; j++) {
        unsigned lo = j * p->sh, hi = lo + p->sh; /* in units of 1/dh of a source row */
        unsigned s0 = lo / p->dh, s1 = (hi + p->dh - 1) / p->dh;
        for (unsigned s = s0; s < s1; s++) {
            unsigned slot = s % p->nslots;
            uint32_t *row = p->vrows + slot * rowlen;
            if (p->vtag[slot] != (long)s) {
                memcpy(p->line, src + (size_t)s * sstride, (size_t)p->sw * p->nch);
                hrow_vec(p, row);
                p->vtag[slot] = (long)s;
            }
            unsigned from = s * p->dh > lo ? s * p->dh : lo;
            unsigned to = (s + 1) * p->dh < hi ? (s + 1) * p->dh : hi;
            rows[s - s0] = row;
            ov[s - s0] = to - from;
        }
        switch (s1 - s0) {
        case 1:
            vpass_vec(rows, ov, 1, p->nblk, recip, p->out);
            break;
        case 2:
            vpass_vec(rows, ov, 2, p->nblk, recip, p->out);
            break;
        default:
            vpass_vec(rows, ov, s1 - s0, p->nblk, recip, p->out);
        }
        memcpy(dst + (size_t)j * dstride, p->out, n);
    }
}
#endif

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

/* One plane by the vector passes where it has them and vec asks, else by the scalar ones. */
static void plane_go(struct plane *p, const uint8_t *src, unsigned sstride, uint8_t *dst, unsigned dstride, int vec)
{
#ifdef SCALE_VEC
    if (vec && p->vec) {
        plane_run_vec(p, src, sstride, dst, dstride);
        return;
    }
#endif
    (void)vec;
    plane_run(p, src, sstride, dst, dstride);
}

static void scale_go(struct scaler *s, const uint8_t *sy, unsigned sy_stride, const uint8_t *sc, unsigned sc_stride, uint8_t *dy,
                     unsigned dy_stride, uint8_t *dc, unsigned dc_stride, int vec)
{
    if (s->sw == s->dw && s->sh == s->dh) {
        for (unsigned j = 0; j < s->sh; j++)
            memcpy(dy + (size_t)j * dy_stride, sy + (size_t)j * sy_stride, s->sw);
        for (unsigned j = 0; j < s->sh / 2; j++)
            memcpy(dc + (size_t)j * dc_stride, sc + (size_t)j * sc_stride, s->sw);
        return;
    }
    plane_go(&s->luma, sy, sy_stride, dy, dy_stride, vec);
    plane_go(&s->chroma, sc, sc_stride, dc, dc_stride, vec);
}

void scale_run(struct scaler *s, const uint8_t *sy, unsigned sy_stride, const uint8_t *sc, unsigned sc_stride, uint8_t *dy,
               unsigned dy_stride, uint8_t *dc, unsigned dc_stride)
{
    scale_go(s, sy, sy_stride, sc, sc_stride, dy, dy_stride, dc, dc_stride, 1);
}

void scale_run_ref(struct scaler *s, const uint8_t *sy, unsigned sy_stride, const uint8_t *sc, unsigned sc_stride, uint8_t *dy,
                   unsigned dy_stride, uint8_t *dc, unsigned dc_stride)
{
    scale_go(s, sy, sy_stride, sc, sc_stride, dy, dy_stride, dc, dc_stride, 0);
}

unsigned scale_vector(const struct scaler *s)
{
    unsigned m = 0;
#ifdef SCALE_VEC
    if (s->sw != s->dw || s->sh != s->dh) {
        m |= s->luma.vec ? SCALE_VEC_LUMA_V : 0;
        m |= s->luma.hk ? SCALE_VEC_LUMA_H : 0;
        m |= s->chroma.vec ? SCALE_VEC_CHROMA_V : 0;
        m |= s->chroma.hk ? SCALE_VEC_CHROMA_H : 0;
    }
#else
    (void)s;
#endif
    return m;
}

void scale_free(struct scaler *s)
{
    if (!s)
        return;
    plane_free(&s->luma);
    plane_free(&s->chroma);
    free(s);
}
