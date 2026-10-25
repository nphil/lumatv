/* Shared by reduce.c (baseline ARMv7, the scalar path) and reduce_neon.c (ARMv7-VE with hardware divide): the part of the
 * reduce that is the same for both, as static inline so each file compiles it for its own instruction set. Include once
 * per translation unit.
 *
 * Ranges (bins are uint16, 256 of them): n <= 256 * 65535 = 16 776 960 and sum <= 255 * n = 4 278 124 800, so every
 * per-channel total fits uint32_t; no 64-bit accumulator is needed.
 */
#ifndef LUMATV_REDUCE_CORE_H
#define LUMATV_REDUCE_CORE_H

#include "reduce.h"

struct chan {
    uint32_t n;    /* pixels */
    uint32_t sum;  /* sum of code * bins[code] */
    uint32_t dark; /* pixels at or under BLACK_CODE */
};

/* q16 of e / (255 n), exactly rounded half up: floor((514 e + n) / (2 n)). 65535 / 255 = 257, so 257 e / n is the value
 * before rounding. The numerator overflows 32 bits, so e = qe * n + re first and 514 * re + n (< 515 n) is divided: this
 * is 32-bit as long as 515 n < 2^32, i.e. for every zone of a real picture (a 8K zone has 2 M pixels). `tie` is set when
 * the value is exactly x.5, the one place where the double-precision reference can fall either way (its three roundings
 * are off by about 1e-11 at most, while any other value lies at least 1 / (2 n) >= 3e-8 from a rounding boundary). */
static inline uint32_t rd_round257(uint32_t e, uint32_t n, int *tie) {
    if (n < 8000000u) {
        uint32_t qe = e / n, re = e - qe * n, m = 514u * re + n, d = 2u * n, q = m / d;
        *tie = m == q * d;
        return 257u * qe + q;
    }
    uint64_t m = 514ull * e + n, d = 2ull * n, q = m / d;
    *tie = m == q * d;
    return (uint32_t)q;
}

static inline uint16_t rd_mean(const struct chan *c) {
    if (!c->n) return 0;
    int tie;
    uint32_t q = rd_round257(c->sum, c->n, &tie);
    return (uint16_t)(tie ? reduce_ref_mean(c->sum, c->n) : q);
}

/* The black share is the smallest of the three channels' dark / n; ratios are compared by cross-multiplication (exact),
 * and 65535 d / n is 257 (255 d) / n. */
static inline uint16_t rd_black(const struct chan *r, const struct chan *g, const struct chan *b) {
    if (!r->n || !g->n || !b->n) return 0;
    const struct chan *m = r;
    if ((uint64_t)g->dark * m->n < (uint64_t)m->dark * g->n) m = g;
    if ((uint64_t)b->dark * m->n < (uint64_t)m->dark * b->n) m = b;
    int tie;
    uint32_t q = rd_round257(255u * m->dark, m->n, &tie); /* dark <= 21 * 65535, so 255 * dark fits */
    return (uint16_t)(tie ? reduce_ref_black(m->dark, m->n) : q);
}

static inline void rd_finish(const struct chan *r, const struct chan *g, const struct chan *b, struct zone_out *o) {
    o->mean[0] = rd_mean(r);
    o->mean[1] = rd_mean(g);
    o->mean[2] = rd_mean(b);
    o->black = rd_black(r, g, b);
    o->count = r->n;
}

#endif
