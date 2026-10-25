/* The reader's reduce, baseline ARMv7 (and the host): the scalar integer path, the double-precision reference formulas for
 * exact ties, and the choice of implementation. See reduce.h; the NEON path is reduce_neon.c. */
#include "reduce_core.h"

#ifdef LUMATV_NEON
#include <sys/auxv.h>
#endif

/* wire.rs q16 */
static unsigned q16(double v) {
    if (v < 0) v = 0;
    if (v > 1) v = 1;
    return (unsigned)(v * 65535.0 + 0.5);
}

/* The two formulas of the first version of this reader, operation for operation (sum is exact in a double). The integer
 * path calls them only when a value is exactly half way between two codes. This file is built without hardware floating
 * point and without contraction, so the results are IEEE round-to-nearest on every target. */
unsigned reduce_ref_mean(uint32_t sum, uint32_t n) { return q16((double)sum / (double)n / 255.0); }
unsigned reduce_ref_black(uint32_t dark, uint32_t n) { return q16((double)dark / (double)n); }

static void channel_scalar(const uint16_t *bins, struct chan *c) {
    uint32_t n = 0, sum = 0, dark = 0;
    for (int i = 0; i <= BLACK_CODE; i++) {
        n += bins[i];
        sum += (uint32_t)i * bins[i];
        dark += bins[i];
    }
    for (int i = BLACK_CODE + 1; i < VTH_BINS; i++) {
        n += bins[i];
        sum += (uint32_t)i * bins[i];
    }
    c->n = n;
    c->sum = sum;
    c->dark = dark;
}

void reduce_zones_scalar(const struct vth_zone *z, int n, struct zone_out *out) {
    for (int i = 0; i < n; i++) {
        struct chan r, g, b;
        channel_scalar(z[i].r, &r);
        channel_scalar(z[i].g, &g);
        channel_scalar(z[i].b, &b);
        rd_finish(&r, &g, &b, &out[i]);
    }
}

#ifdef LUMATV_NEON
/* arch/arm/include/uapi/asm/hwcap.h; the 32-bit kernel sets these from the CPU's ID registers. The NEON file is built for
 * ARMv7-VE (hardware divide in both instruction sets), so both must be reported. */
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

void reduce_zones(const struct vth_zone *z, int n, struct zone_out *out) {
    if (have_neon()) reduce_zones_neon(z, n, out);
    else reduce_zones_scalar(z, n, out);
}

const char *reduce_impl(void) { return have_neon() ? "neon" : "scalar"; }
#else
void reduce_zones(const struct vth_zone *z, int n, struct zone_out *out) { reduce_zones_scalar(z, n, out); }

const char *reduce_impl(void) { return "scalar"; }
#endif
