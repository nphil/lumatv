/* The reader's reduce: one poll's histograms (struct vth_zone, 256 bins a channel) to what the helper's wire frame carries
 * per area (ambient/src/wire.rs): the mean code value of each channel and the black share as q16, and the red channel's
 * pixel count. Integer-only; the output is bit-identical to the double-precision formula it replaced (reduce_test.c).
 *
 *   mean  = q16(sum(i * bins[i]) / n / 255)
 *   black = q16(min over r, g, b of (sum of bins[0..BLACK_CODE]) / n), 0 when any channel has no pixels
 *   q16(v) = (unsigned)(clamp(v, 0, 1) * 65535 + 0.5)
 */
#ifndef LUMATV_REDUCE_H
#define LUMATV_REDUCE_H

#include <stdint.h>

#include "vthist.h"

#define BLACK_CODE 20 /* zones.rs FrameParams::default: code values at or under this count as black */

struct zone_out {
    uint16_t mean[3]; /* r, g, b */
    uint16_t black;
    uint32_t count;   /* pixels in the red channel's histogram */
};

/* The fastest implementation this CPU runs (NEON where the kernel reports it, else the scalar one). */
void reduce_zones(const struct vth_zone *z, int n, struct zone_out *out);
const char *reduce_impl(void);

void reduce_zones_scalar(const struct vth_zone *z, int n, struct zone_out *out);
#ifdef LUMATV_NEON
void reduce_zones_neon(const struct vth_zone *z, int n, struct zone_out *out); /* reduce_neon.c: ARMv7-VE + NEON only */
#endif

/* The double-precision formulas, kept for the one case the integer rounding cannot decide (see reduce_core.h). */
unsigned reduce_ref_mean(uint32_t sum, uint32_t n);
unsigned reduce_ref_black(uint32_t dark, uint32_t n);

#endif
