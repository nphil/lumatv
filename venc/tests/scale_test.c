/* Tests of the scaler's vector code (scale.c): scale_run must give scale_run_ref's bytes exactly, for the sizes the
 * engine runs and the ones that stress the 16-byte blocks, on random and extreme pictures. test.sh runs it on the host,
 * where neon_emu.h stands in for NEON. build.sh builds it for the set too (build/arm64/scale_test, CPU only): there it
 * checks the real instructions against the scalar code and against what the emulation assumes of them. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(__ARM_NEON) && defined(__aarch64__)
#include <arm_neon.h>
#else
#include "../neon_emu.h"
#endif

#include "../scale.h"

static int failures;
#define CHECK(cond, ...) do { if (!(cond)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

static uint32_t rng = 0x2545F491u;
static uint32_t rnd(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

/* ---- what the vector code takes the instructions to do ---- */

/* the vector types are plain 16- and 8-byte values on both sides, so they go to and from arrays by memcpy */
#define TO(arr, v) do { __typeof__(v) t_ = (v); memcpy(arr, &t_, sizeof(t_)); } while (0)
#define FROM(type, arr) ({ type t_; memcpy(&t_, arr, sizeof(t_)); t_; })

static void test_instructions(void)
{
    uint8_t tab[64], idx[16], got[16];
    for (unsigned i = 0; i < 64; i++)
        tab[i] = (uint8_t)(200 - 3 * i);
    for (unsigned i = 0; i < 16; i++)
        idx[i] = (uint8_t)(i % 2 ? 31 - i : i);
    idx[3] = 32; /* past a two-register table: 0 */
    idx[5] = 255;
    vst1q_u8(got, vqtbl2q_u8(vld1q_u8_x2(tab), vld1q_u8(idx)));
    for (unsigned i = 0; i < 16; i++)
        CHECK(got[i] == (idx[i] < 32 ? tab[idx[i]] : 0), "tbl2 lane %u: %u", i, got[i]);
    for (unsigned i = 0; i < 16; i++)
        idx[i] = (uint8_t)(63 - 4 * i);
    idx[0] = 64;
    idx[1] = 100;
    vst1q_u8(got, vqtbl4q_u8(vld1q_u8_x4(tab), vld1q_u8(idx)));
    for (unsigned i = 0; i < 16; i++)
        CHECK(got[i] == (idx[i] < 64 ? tab[idx[i]] : 0), "tbl4 lane %u: %u", i, got[i]);

    /* widening does not extend the sign; the multiply-accumulates take the product at 32 bits and wrap there */
    uint8_t b[16] = { 255, 0, 128, 1, 254, 7, 9, 200, 255, 255, 3, 4, 5, 6, 77, 128 };
    uint16_t lo[8], hi[8];
    TO(lo, vmovl_u8(vget_low_u8(vld1q_u8(b))));
    TO(hi, vmovl_high_u8(vld1q_u8(b)));
    for (unsigned i = 0; i < 8; i++)
        CHECK(lo[i] == b[i] && hi[i] == b[8 + i], "widen lane %u: %u %u", i, lo[i], hi[i]);
    uint16_t ma[8] = { 65535, 65535, 1, 2, 3, 65535, 0, 7 }, mb[8] = { 65535, 2, 65535, 5, 7, 65535, 9, 11 };
    uint32_t acc[4] = { 0xFFFFFFFFu, 5, 6, 0x80000000u }, r[4];
    vst1q_u32(r, vmlal_u16(vld1q_u32(acc), vget_low_u16(vld1q_u16(ma)), vget_low_u16(vld1q_u16(mb))));
    for (unsigned i = 0; i < 4; i++)
        CHECK(r[i] == acc[i] + (uint32_t)ma[i] * mb[i], "umlal lane %u: %u", i, r[i]);
    vst1q_u32(r, vmlal_high_u16(vld1q_u32(acc), vld1q_u16(ma), vld1q_u16(mb)));
    for (unsigned i = 0; i < 4; i++)
        CHECK(r[i] == acc[i] + (uint32_t)ma[4 + i] * mb[4 + i], "umlal2 lane %u: %u", i, r[i]);
    uint32_t w[4] = { 0x80000000u, 0xFFFFFFFFu, 3, 0 };
    vst1q_u32(r, vmulq_n_u32(vld1q_u32(w), 2));
    CHECK(r[0] == 0 && r[1] == 0xFFFFFFFEu && r[2] == 6 && r[3] == 0, "mul by element: %u %u %u %u", r[0], r[1], r[2], r[3]);
    vst1q_u32(r, vmlaq_n_u32(vld1q_u32(acc), vld1q_u32(w), 3));
    for (unsigned i = 0; i < 4; i++)
        CHECK(r[i] == acc[i] + w[i] * 3, "mla lane %u: %u", i, r[i]);

    /* the last step: SQDMULH is floor(a*b / 2^31) (saturating at INT32_MIN squared), RSHRN adds half and shifts and keeps
     * the low 16 bits, SQXTUN clamps to a byte */
    int32_t qa[4] = { 0x40000000, 2147483647, INT32_MIN, 1000000000 }, qb[4] = { 0x40000000, 2147483647, INT32_MIN, 3 }, q[4];
    TO(q, vqdmulhq_s32(FROM(int32x4_t, qa), FROM(int32x4_t, qb)));
    CHECK(q[0] == 0x20000000 && q[1] == 2147483646 && q[2] == INT32_MAX && q[3] == 1, "sqdmulh: %d %d %d %d", q[0], q[1], q[2], q[3]);
    int32_t sa[4] = { 255, 256, 767, 768 }, sb[4] = { 130560, 130559, 0, 1 << 30 };
    int16_t n[8];
    TO(n, vrshrn_n_s32(FROM(int32x4_t, sa), 9));
    TO(n + 4, vrshrn_n_s32(FROM(int32x4_t, sb), 9));
    CHECK(n[0] == 0 && n[1] == 1 && n[2] == 1 && n[3] == 2, "rshrn: %d %d %d %d", n[0], n[1], n[2], n[3]);
    CHECK(n[4] == 255 && n[5] == 255 && n[6] == 0 && n[7] == 0, "rshrn: %d %d %d %d", n[4], n[5], n[6], n[7]);
    int16_t sat[8] = { -5, 300, 77, 255, 256, 0, 32767, -32768 };
    uint8_t nb[8];
    TO(nb, vqmovun_s16(FROM(int16x8_t, sat)));
    CHECK(nb[0] == 0 && nb[1] == 255 && nb[2] == 77 && nb[3] == 255 && nb[4] == 255 && nb[5] == 0 && nb[6] == 255 && nb[7] == 0,
          "sqxtun: %u %u %u %u %u %u %u %u", nb[0], nb[1], nb[2], nb[3], nb[4], nb[5], nb[6], nb[7]);

    /* together they are the scalar last step, (acc*recip + 2^39) >> 40: over areas up to the largest plane scale_new
     * takes, and sums at, just under and just over the rounding point of each result */
    int bad = 0;
    for (unsigned t = 0; t < 6000 && !bad; t++) {
        uint32_t area = 513 + rnd() % (3840 * 2176 - 512);
        uint64_t recip = ((1ull << 40) + area / 2) / area;
        int64_t a = t % 3 == 0 ? 255ll * area : t % 3 == 1 ? (int64_t)(rnd() % (255 * area + 1)) : (int64_t)(rnd() % 255) * area + area / 2 + (int)(rnd() % 3) - 1;
        uint8_t o[8], want = (uint8_t)(((uint64_t)a * recip + (1ull << 39)) >> 40);
        int32x4_t qq = vqdmulhq_s32(vreinterpretq_s32_u32(vdupq_n_u32((uint32_t)a)), vdupq_n_s32((int32_t)recip));
        int16x4_t h = vrshrn_n_s32(qq, 9);
        TO(o, vqmovun_s16(vcombine_s16(h, h)));
        bad = o[0] != want || o[7] != want;
        CHECK(!bad, "last step of %lld over %u samples: %u, scalar %u", (long long)a, area, o[0], want);
    }
}

/* ---- scale_run against scale_run_ref ---- */

enum { P_RANDOM = 1, P_ZERO = 2, P_FULL = 4, P_CHECKER = 8, P_SPIKES = 16, P_BANDS = 32 };

/* rows of n bytes in a plane of the given stride; the padding between rows is junk that must make no difference.
 * P_BANDS is a band of rows of each of the other kinds, so one run takes in the edges between them. */
static void fill(uint8_t *p, unsigned stride, unsigned n, unsigned rows, int kind)
{
    uint32_t bits = 0;
    for (unsigned r = 0; r < rows; r++) {
        int k = kind == P_BANDS ? 1 << (r * 5 / rows) : kind;
        for (unsigned i = 0; i < stride; i++) {
            uint8_t v;
            if (!(i & 3))
                bits = rnd();
            if (i >= n)
                v = 0xA5;
            else if (k == P_ZERO)
                v = 0;
            else if (k == P_FULL)
                v = 255;
            else if (k == P_CHECKER)
                v = (i ^ r) & 1 ? 255 : 0;
            else if (k == P_SPIKES)
                v = (bits >> (i & 3) * 8) % 23 ? 0 : 255;
            else
                v = (uint8_t)(bits >> (i & 3) * 8);
            p[(size_t)r * stride + i] = v;
        }
    }
}

/* One size pair on every kind of picture asked for; the vector passes are expected to be the ones in want (-1: any), and
 * come back. One scaler serves all the pictures, so what a run leaves behind must not matter to the next. Strides are
 * wider than the pictures, differ between luma and chroma, and the planes sit at odd addresses. */
static unsigned run_case(unsigned sw, unsigned sh, unsigned dw, unsigned dh, unsigned kinds, int want)
{
    struct scaler *s = scale_new(sw, sh, dw, dh);
    CHECK(s != NULL, "scaler %ux%u -> %ux%u", sw, sh, dw, dh);
    if (!s)
        return 0;
    unsigned got = scale_vector(s);
    CHECK(want < 0 || got == (unsigned)want, "%ux%u -> %ux%u: vector passes %#x, expected %#x", sw, sh, dw, dh, got, (unsigned)want);
    unsigned ys = sw + 6, cs = sw + 10, yd = dw + 4, cd = dw + 2;
    size_t ysz = (size_t)ys * sh, csz = (size_t)cs * (sh / 2), ydsz = (size_t)yd * dh, cdsz = (size_t)cd * (dh / 2);
    uint8_t *src = malloc(ysz + csz + 8), *ref = malloc(ydsz + cdsz + 8), *vec = malloc(ydsz + cdsz + 8);
    if (src && ref && vec)
        for (unsigned kind = 1; kind <= P_BANDS; kind <<= 1) {
            if (!(kinds & kind))
                continue;
            uint8_t *y = src + 1, *c = src + 1 + ysz + 2;
            fill(y, ys, sw, sh, kind);
            fill(c, cs, sw, sh / 2, kind);
            memset(ref, 0xEE, ydsz + cdsz + 8);
            memset(vec, 0xEE, ydsz + cdsz + 8);
            scale_run_ref(s, y, ys, c, cs, ref + 3, yd, ref + 3 + ydsz, cd);
            scale_run(s, y, ys, c, cs, vec + 3, yd, vec + 3 + ydsz, cd);
            if (memcmp(ref, vec, ydsz + cdsz + 8)) {
                size_t i = 0;
                while (ref[i] == vec[i])
                    i++;
                size_t at = i >= 3 ? i - 3 : 0;
                int luma = at < ydsz;
                CHECK(0, "%ux%u -> %ux%u, picture kind %d: first difference in %s row %zu byte %zu: %u, scalar %u", sw, sh, dw, dh, kind,
                      luma ? "luma" : "chroma", luma ? at / yd : (at - ydsz) / cd, luma ? at % yd : (at - ydsz) % cd, vec[i], ref[i]);
                break;
            }
        }
    free(src);
    free(ref);
    free(vec);
    scale_free(s);
    return got;
}

static void test_engine_sizes(void)
{
    const unsigned all = SCALE_VEC_LUMA_H | SCALE_VEC_LUMA_V | SCALE_VEC_CHROMA_H | SCALE_VEC_CHROMA_V;
    const unsigned down = SCALE_VEC_LUMA_V | SCALE_VEC_CHROMA_V;
    run_case(1920, 1080, 1280, 720, P_BANDS, all);
    run_case(1920, 1088, 1280, 720, P_BANDS, all); /* 1.51 down: two or three source rows */
    run_case(1280, 720, 960, 540, P_BANDS, all);
    run_case(1280, 720, 848, 480, P_BANDS, all); /* two or three source samples across */
    run_case(1280, 720, 640, 360, P_BANDS, all);
    run_case(1280, 720, 432, 240, P_BANDS, all); /* 2.96:1, three or four, a 64-byte window */
    run_case(1440, 1080, 960, 720, P_BANDS, all);
    run_case(704, 480, 640, 480, P_BANDS, all); /* a ratio of 1.1 across, none down */
    run_case(1280, 720, 854, 480, P_BANDS, all); /* widths of 53.4 blocks */
    run_case(960, 544, 240, 136, P_BANDS, all); /* 4:1 across: a window of exactly 64 bytes */
    run_case(960, 540, 224, 136, P_BANDS, down); /* a little more is past the table window */
    run_case(960, 540, 60, 34, P_BANDS, down); /* 16:1 across, 17 source rows an output */
    run_case(64, 48, 64, 48, P_RANDOM, 0); /* the identity is a copy */
}

/* the biggest sums: every sample 255 at the largest size scale_new takes */
static void test_extremes(void)
{
    run_case(3840, 2176, 240, 136, P_FULL, SCALE_VEC_LUMA_V | SCALE_VEC_CHROMA_V); /* 16:1, the biggest area: sums within 1% of 2^31 */
    run_case(32, 16, 16, 8, P_RANDOM | P_FULL, 0); /* under 512 samples a plane: the scalar code */
    run_case(26, 20, 20, 12, P_RANDOM | P_FULL, SCALE_VEC_LUMA_V | SCALE_VEC_LUMA_H); /* luma just over, chroma under */
}

/* ---- the sweeps: every pair of sizes in a range, and which tap counts they put through the vector code ---- */

/* source samples the output at x takes along an axis of sn samples scaled to dn */
static unsigned taps(unsigned sn, unsigned dn, unsigned x)
{
    return ((x + 1) * sn + dn - 1) / dn - x * sn / dn;
}

/* 1: two samples, 2: three, 4: four or more */
static unsigned tap_class(unsigned t)
{
    return t <= 2 ? 1 : t == 3 ? 2 : 4;
}

/* which classes share one 16-byte block of outputs (blocks), and follow each other in neighbouring outputs (steps) */
static unsigned char seen_blocks[8], seen_steps[8][8];

static void tally_blocks(unsigned sn, unsigned dn, unsigned per_block)
{
    for (unsigned b = 0; b < dn; b += per_block) {
        unsigned mix = 0;
        for (unsigned x = b; x < dn && x < b + per_block; x++)
            mix |= tap_class(taps(sn, dn, x));
        seen_blocks[mix] = 1;
    }
}

static void tally_steps(unsigned sn, unsigned dn)
{
    for (unsigned x = 0; x + 1 < dn; x++)
        seen_steps[tap_class(taps(sn, dn, x))][tap_class(taps(sn, dn, x + 1))] = 1;
}

/* the size of the other axis that leaves the chroma plane just over the 512 samples the vector code starts at */
static unsigned just_over(unsigned n)
{
    return (2048 / n + 2) & ~1u;
}

/* every pair of widths across a few blocks, with three rows in two down, and every pair of heights likewise */
static void test_sweeps(void)
{
    for (unsigned sw = 24; sw <= 96; sw += 2)
        for (unsigned dw = ((sw + 15) / 16 + 1) & ~1u; dw <= sw; dw += 2) {
            unsigned sh = just_over(sw);
            unsigned got = run_case(sw, sh, dw, sh / 3 * 2, P_RANDOM, -1);
            if (got & SCALE_VEC_LUMA_H && sw != dw)
                tally_blocks(sw, dw, 16);
            if (got & SCALE_VEC_CHROMA_H && sw != dw)
                tally_blocks(sw / 2, dw / 2, 8);
        }
    for (unsigned sh = 24; sh <= 96; sh += 2)
        for (unsigned dh = ((sh + 15) / 16 + 1) & ~1u; dh <= sh; dh += 2) {
            unsigned sw = just_over(sh) < 48 ? 48 : just_over(sh);
            unsigned got = run_case(sw, sh, sw / 3 * 2, dh, P_RANDOM, -1);
            if (got & SCALE_VEC_LUMA_V && sh != dh)
                tally_steps(sh, dh);
            if (got & SCALE_VEC_CHROMA_V && sh != dh)
                tally_steps(sh / 2, dh / 2);
        }
    for (unsigned sh = 24; sh <= 96; sh += 6)
        for (unsigned dh = ((sh + 15) / 16 + 1) & ~1u; dh < sh; dh += 2)
            run_case(80, sh, 80, dh, P_RANDOM, -1); /* only the vertical pass reduces */

    /* the mixes one ratio can make: two samples; three; four or more; two and three; three and four or more */
    static const unsigned mixes[] = { 1, 2, 4, 3, 6 };
    for (unsigned i = 0; i < sizeof(mixes) / sizeof(*mixes); i++)
        CHECK(seen_blocks[mixes[i]], "no block of outputs across with tap classes %#x", mixes[i]);
    static const unsigned steps[][2] = { { 1, 1 }, { 1, 2 }, { 2, 1 }, { 2, 2 }, { 2, 4 }, { 4, 2 }, { 4, 4 } };
    for (unsigned i = 0; i < sizeof(steps) / sizeof(*steps); i++)
        CHECK(seen_steps[steps[i][0]][steps[i][1]], "no neighbouring rows down with tap classes %u then %u", steps[i][0], steps[i][1]);
}

int main(void)
{
    test_instructions();
    test_engine_sizes();
    test_extremes();
    test_sweeps();
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    puts("scale_test: ok");
    return 0;
}
