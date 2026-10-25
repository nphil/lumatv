/* The reader's reduce in NEON, for the set's Cortex-A76 running 32-bit code. Compiled as ARMv7-VE with NEON and hardware
 * divide (build.sh: the hard-float target, because the soft-float one cannot select NEON vector code; nothing here takes
 * or returns a floating-point value, so the calling convention is the same as the rest of the program's), and only ever
 * called after reduce.c has found NEON and divide in the kernel's capability bits.
 *
 * All 256 bins of every channel are needed (the mean and the pixel count depend on each one), so the work is the sweep
 * itself: 12 288 bins a poll, 8 to a vector load, read once in address order from the buffer the library has just filled
 * (the three channels of an area are contiguous, 512 bytes each). Per 8 bins: one pairwise-add-accumulate for the count
 * and two widening multiply-accumulates against the code values for the sum. 32-bit lanes cannot overflow (reduce_core.h).
 */
#include <arm_neon.h>

#include "reduce_core.h"

static inline void channel_neon(const uint16_t *bins, struct chan *c) {
    const uint16x8_t step8 = vdupq_n_u16(8), step16 = vdupq_n_u16(16);
    static const uint16_t first[8] = {0, 1, 2, 3, 4, 5, 6, 7};
    uint16x8_t idx0 = vld1q_u16(first), idx1 = vaddq_u16(idx0, step8);
    uint32x4_t n0 = vdupq_n_u32(0), n1 = n0, s0 = n0, s1 = n0;
    for (int i = 0; i < VTH_BINS; i += 16) {
        uint16x8_t v0 = vld1q_u16(bins + i), v1 = vld1q_u16(bins + i + 8);
        n0 = vpadalq_u16(n0, v0);
        n1 = vpadalq_u16(n1, v1);
        s0 = vmlal_u16(s0, vget_low_u16(v0), vget_low_u16(idx0));
        s1 = vmlal_u16(s1, vget_low_u16(v1), vget_low_u16(idx1));
        s0 = vmlal_u16(s0, vget_high_u16(v0), vget_high_u16(idx0));
        s1 = vmlal_u16(s1, vget_high_u16(v1), vget_high_u16(idx1));
        idx0 = vaddq_u16(idx0, step16);
        idx1 = vaddq_u16(idx1, step16);
    }
    /* codes 0..BLACK_CODE: two whole vectors and the first five lanes of the third */
    static const uint16_t keep[8] = {0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0, 0, 0};
    _Static_assert(BLACK_CODE == 20, "the mask keeps codes 16..20 of the third vector");
    uint32x4_t d = vpaddlq_u16(vld1q_u16(bins));
    d = vpadalq_u16(d, vld1q_u16(bins + 8));
    d = vpadalq_u16(d, vandq_u16(vld1q_u16(bins + 16), vld1q_u16(keep)));

    uint32x4_t n4 = vaddq_u32(n0, n1), s4 = vaddq_u32(s0, s1);
    uint32x2_t ns = vpadd_u32(vadd_u32(vget_low_u32(n4), vget_high_u32(n4)), vadd_u32(vget_low_u32(s4), vget_high_u32(s4)));
    uint32x2_t dd = vadd_u32(vget_low_u32(d), vget_high_u32(d));
    c->n = vget_lane_u32(ns, 0);
    c->sum = vget_lane_u32(ns, 1);
    c->dark = vget_lane_u32(vpadd_u32(dd, dd), 0);
}

void reduce_zones_neon(const struct vth_zone *z, int n, struct zone_out *out) {
    for (int i = 0; i < n; i++) {
        struct chan r, g, b;
        channel_neon(z[i].r, &r);
        channel_neon(z[i].g, &g);
        channel_neon(z[i].b, &b);
        rd_finish(&r, &g, &b, &out[i]);
    }
}
