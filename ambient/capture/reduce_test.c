/* The reader's reduce against the double-precision code it replaced, on made-up histograms (the set's own dumps are not in
 * the repository), and its cost. Built by build.sh for the host (scalar integer path) and for the set's CPU (scalar and NEON
 * paths; run under qemu-arm, test.sh).
 *
 *   reduce_test [-n ZONES] [-s SEED]     compare every implementation with the reference, exit 1 on the first mismatch
 *   reduce_test -f DUMP                  compare on a recorded dump (ambient-probe -d: records of "VTH1", preset, areas, t_ms, areas * 0x604 bytes)
 *   reduce_test -b ITERATIONS [-i NAME]  time ITERATIONS polls of 16 areas per implementation (NAME: reference, scalar or neon only)
 *
 * Mismatch means any difference in any field of struct zone_out. The check is exact equality, not a tolerance.
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "reduce.h"

/* ---- the reference: the first version of the reader's reduce, verbatim (service.c at cdc9584) ------------------------- */

static unsigned ref_q16(double v) {
    if (v < 0) v = 0;
    if (v > 1) v = 1;
    return (unsigned)(v * 65535.0 + 0.5);
}

static void ref_channel(const uint16_t *bins, double *mean, uint32_t *count, double *black) {
    uint64_t n = 0, dark = 0;
    double sum = 0;
    for (int i = 0; i < VTH_BINS; i++) {
        n += bins[i];
        sum += (double)i * bins[i];
        if (i <= BLACK_CODE) dark += bins[i];
    }
    *count = (uint32_t)n;
    *mean = n ? sum / (double)n / 255.0 : 0.0;
    *black = n ? (double)dark / (double)n : 0.0;
}

static void ref_zones(const struct vth_zone *z, int n, struct zone_out *out) {
    for (int i = 0; i < n; i++) {
        double mr, mg, mb, br, bg, bb;
        uint32_t nr, ng, nb;
        ref_channel(z[i].r, &mr, &nr, &br);
        ref_channel(z[i].g, &mg, &ng, &bg);
        ref_channel(z[i].b, &mb, &nb, &bb);
        double black = (nr == 0 || ng == 0 || nb == 0) ? 0.0 : (br < bg ? (br < bb ? br : bb) : (bg < bb ? bg : bb));
        out[i].mean[0] = (uint16_t)ref_q16(mr);
        out[i].mean[1] = (uint16_t)ref_q16(mg);
        out[i].mean[2] = (uint16_t)ref_q16(mb);
        out[i].black = (uint16_t)ref_q16(black);
        out[i].count = nr;
    }
}

/* ---- histograms ------------------------------------------------------------------------------------------------------ */

static uint64_t rng_state = 88172645463325252ull;
static uint64_t rnd(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return rng_state;
}
static unsigned below(unsigned n) { return (unsigned)(((rnd() >> 32) * n) >> 32); } /* multiply-shift: no 64-bit divide on ARM */

static void add(uint16_t *bins, int code, uint32_t count) {
    uint32_t v = bins[code] + count;
    bins[code] = v > 65535u ? 65535u : (uint16_t)v;
}

#define KINDS 11
/* One channel of one area. The first kinds are what a picture looks like; the rest are the corners of the arithmetic. */
static void channel_of(uint16_t *bins, int kind) {
    memset(bins, 0, VTH_BINS * sizeof *bins);
    switch (kind) {
    case 0: { /* a smooth bump: a 1080p area is 129 600 pixels, a 4K one 518 400 */
        unsigned pixels = 20000u + below(500000u);
        int centre = (int)below(256), width = 2 + (int)below(60);
        for (unsigned p = 0; p < pixels; p += 7) {
            int c = centre + ((int)below((unsigned)width) - (int)below((unsigned)width)) + ((int)below(3) - 1);
            add(bins, c < 0 ? 0 : c > 255 ? 255 : c, 7);
        }
        break;
    }
    case 1: /* flat colour */
        add(bins, (int)below(256), 1 + below(600000u));
        break;
    case 2: { /* a few spikes, like a title card */
        int spikes = 1 + (int)below(5);
        for (int s = 0; s < spikes; s++) add(bins, (int)below(256), below(200000u));
        break;
    }
    case 3: /* letterbox bar: mostly black, some picture */
        add(bins, (int)below(BLACK_CODE + 1), 100000u + below(400000u));
        for (int s = 0; s < 40; s++) add(bins, (int)below(256), below(3000u));
        break;
    case 4: /* every bin random, up to the type's limit */
        for (int i = 0; i < VTH_BINS; i++) bins[i] = (uint16_t)rnd();
        break;
    case 5: /* every bin at the limit: n = 16 776 960, the largest the type allows */
        for (int i = 0; i < VTH_BINS; i++) bins[i] = 65535u;
        break;
    case 6: /* no pixels */
        break;
    case 7: /* tiny counts */
        for (int i = 0; i < VTH_BINS; i++) if (below(8) == 0) bins[i] = (uint16_t)below(4);
        break;
    case 8: /* all pixels at the edge of the black range */
        add(bins, below(2) ? BLACK_CODE : BLACK_CODE + 1, 1 + below(60000u));
        add(bins, below(2) ? 0 : 255, below(60000u));
        break;
    case 9: /* two bins, small counts: where values land exactly half way */
        add(bins, (int)below(256), 1 + below(64));
        add(bins, (int)below(256), 1 + below(64));
        break;
    default: /* a few bins at the limit: n near 2^23 and above, the 64-bit path */
        for (int s = 0; s < 130 + (int)below(130); s++) add(bins, (int)below(256), 65535u - below(8));
        break;
    }
}

static void zone_of(struct vth_zone *z, int index) {
    z->area_id = below(10) == 0 ? 99u : (uint32_t)index;
    int base = (int)below(KINDS);
    if ((base == 4 || base == 5 || base == 10) && below(4)) base = (int)below(4); /* the huge ones stay a minority */
    channel_of(z->r, below(4) ? base : (int)below(KINDS));
    channel_of(z->g, below(4) ? base : (int)below(KINDS));
    channel_of(z->b, below(4) ? base : (int)below(KINDS));
}

/* Exactly half way between two codes: the reference can go either way there, the integer rounding must hand over to it. */
static int is_tie(uint64_t e, uint64_t n) { return n && (514 * e + n) % (2 * n) == 0; }

static unsigned exact_round(uint64_t e, uint64_t n) { return (unsigned)((514 * e + n) / (2 * n)); }

struct stats {
    uint64_t zones, ties, ties_where_plain_rounding_differs, huge;
};

static void tally(const struct vth_zone *z, const struct zone_out *want, struct stats *st) {
    const uint16_t *ch[3] = {z->r, z->g, z->b};
    uint64_t dark[3], n[3];
    for (int c = 0; c < 3; c++) {
        uint64_t sum = 0;
        n[c] = dark[c] = 0;
        for (int i = 0; i < VTH_BINS; i++) {
            n[c] += ch[c][i];
            sum += (uint64_t)i * ch[c][i];
            if (i <= BLACK_CODE) dark[c] += ch[c][i];
        }
        if (n[c] >= 8000000u) st->huge++;
        if (is_tie(sum, n[c])) {
            st->ties++;
            st->ties_where_plain_rounding_differs += exact_round(sum, n[c]) != want->mean[c];
        }
    }
    if (n[0] && n[1] && n[2]) {
        int m = 0;
        for (int c = 1; c < 3; c++) if (dark[c] * n[m] < dark[m] * n[c]) m = c;
        if (is_tie(255 * dark[m], n[m])) {
            st->ties++;
            st->ties_where_plain_rounding_differs += exact_round(255 * dark[m], n[m]) != want->black;
        }
    }
    st->zones++;
}

static int differs(const struct zone_out *a, const struct zone_out *b) {
    return memcmp(a->mean, b->mean, sizeof a->mean) || a->black != b->black || a->count != b->count;
}

typedef void (*impl_fn)(const struct vth_zone *, int, struct zone_out *);
struct impl {
    const char *name;
    impl_fn fn;
};

static int collect(struct impl *list) {
    int n = 0;
    list[n++] = (struct impl){"scalar", reduce_zones_scalar};
#ifdef LUMATV_NEON
    if (strcmp(reduce_impl(), "neon") == 0) list[n++] = (struct impl){"neon", reduce_zones_neon};
    else puts("note: this CPU does not report NEON and hardware divide; the NEON path is not run");
#endif
    return n;
}

static int compare(long zones) {
    struct impl impls[2];
    int nimpl = collect(impls);
    struct vth_zone *z = calloc(VTH_MAX_ZONES, sizeof *z);
    struct zone_out want[VTH_MAX_ZONES], got[VTH_MAX_ZONES];
    struct stats st = {0, 0, 0, 0};
    long bad = 0;
    for (long done = 0; done < zones; done += VTH_MAX_ZONES) {
        for (int i = 0; i < VTH_MAX_ZONES; i++) zone_of(&z[i], i);
        ref_zones(z, VTH_MAX_ZONES, want);
        for (int i = 0; i < VTH_MAX_ZONES; i++) tally(&z[i], &want[i], &st);
        for (int k = 0; k < nimpl; k++) {
            memset(got, 0xa5, sizeof got);
            impls[k].fn(z, VTH_MAX_ZONES, got);
            for (int i = 0; i < VTH_MAX_ZONES; i++) {
                if (!differs(&got[i], &want[i])) continue;
                if (bad++ < 5)
                    printf("MISMATCH %s zone %ld: got %u %u %u black %u count %u, reference %u %u %u black %u count %u\n", impls[k].name,
                           done + i, got[i].mean[0], got[i].mean[1], got[i].mean[2], got[i].black, got[i].count, want[i].mean[0],
                           want[i].mean[1], want[i].mean[2], want[i].black, want[i].count);
            }
        }
    }
    free(z);
    printf("%ld zones (%" PRIu64 " with a channel of 8 M pixels or more, the 64-bit path) through", (long)st.zones, st.huge);
    for (int k = 0; k < nimpl; k++) printf(" %s", impls[k].name);
    printf(": %ld mismatches; %" PRIu64 " values exactly half way, %" PRIu64 " of them where plain exact rounding would differ from the reference\n",
           bad, st.ties, st.ties_where_plain_rounding_differs);
    return bad ? 1 : 0;
}

static uint64_t gcd(uint64_t a, uint64_t b) { return b ? gcd(b, a % b) : a; }

/* A channel of n pixels whose sum of codes is s, on two neighbouring codes (n up to 131 070: a 1080p area has 129 600). */
static int channel_with_sum(uint16_t *bins, uint64_t n, uint64_t s) {
    uint64_t i = s / n, r = s % n;
    if (n - r > 65535u || r > 65535u) return 0;
    memset(bins, 0, VTH_BINS * sizeof *bins);
    bins[i] = (uint16_t)(n - r);
    if (r) bins[i + 1] = (uint16_t)r;
    return 1;
}

/* A channel of n pixels, d of them at code 0 and the rest at code 255. */
static int channel_with_dark(uint16_t *bins, uint64_t n, uint64_t d) {
    if (d > 65535u || n - d > 65535u) return 0;
    memset(bins, 0, VTH_BINS * sizeof *bins);
    bins[0] = (uint16_t)d;
    bins[255] = (uint16_t)(n - d);
    return 1;
}

/* Histograms built to land exactly half way between two codes, in the mean (red channel) and in the black share (all three
 * channels, so whichever is the smallest is one). The random kinds seldom get there, and where they do the reference mostly
 * agrees with plain rounding, but not always (4.4 % of the half-way means with n up to 3 M), so this is the test that the
 * hand-over to the double formulas is needed and right. */
static int ties(long target) {
    struct impl impls[2];
    int nimpl = collect(impls);
    struct vth_zone *z = calloc(1, sizeof *z);
    struct zone_out want, got;
    struct stats st = {0, 0, 0, 0};
    long bad = 0, built = 0;
    while (built < target) {
        memset(z, 0, sizeof *z);
        if (built % 2 == 0) { /* mean */
            uint64_t n = 2 * (1 + below(65535)), n2 = n / 2, s;
            if (n2 % 257 == 0) s = (2 * (uint64_t)below(100000) + 1) * (n2 / 257);
            else s = (2 * (uint64_t)below(255) + 1) * n2;
            if (s > 255 * n || !channel_with_sum(z->r, n, s)) continue;
            channel_of(z->g, 1 + (int)below(3)); /* cheap kinds: the mean under test is the red one */
            channel_of(z->b, 1 + (int)below(3));
        } else { /* black */
            uint64_t m = 2 * (uint64_t)below(65535) + 1, step = 131070 / gcd(m, 131070), n = step * (1 + below((unsigned)(131070 / step))), d;
            d = m * n / 131070;
            if (!channel_with_dark(z->r, n, d) || !channel_with_dark(z->g, n, d) || !channel_with_dark(z->b, n, d)) continue;
        }
        built++;
        ref_zones(z, 1, &want);
        tally(z, &want, &st);
        for (int k = 0; k < nimpl; k++) {
            impls[k].fn(z, 1, &got);
            if (differs(&got, &want)) {
                if (bad++ < 5) printf("MISMATCH %s half-way zone %ld\n", impls[k].name, built);
            }
        }
    }
    free(z);
    printf("half-way construction: %ld zones with %" PRIu64 " exact half-way values through", built, st.ties);
    for (int k = 0; k < nimpl; k++) printf(" %s", impls[k].name);
    printf(": %ld mismatches; plain exact rounding would have differed from the reference in %" PRIu64 " of them\n", bad,
           st.ties_where_plain_rounding_differs);
    return bad ? 1 : 0;
}

/* A recorded dump from the set (ambient-probe -d), every area of every record. */
static int replay(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) {
        perror(path);
        return 2;
    }
    struct impl impls[2];
    int nimpl = collect(impls);
    struct vth_zone *z = calloc(VTH_MAX_ZONES, sizeof *z);
    struct zone_out want[VTH_MAX_ZONES], got[VTH_MAX_ZONES];
    uint32_t hdr[4];
    long records = 0, bad = 0;
    while (fread(hdr, sizeof hdr, 1, f) == 1) {
        if (hdr[0] != 0x31485456u || hdr[2] > VTH_MAX_ZONES || fread(z, sizeof *z, hdr[2], f) != hdr[2]) {
            fprintf(stderr, "%s: record %ld is not a VTH1 record\n", path, records);
            return 2;
        }
        ref_zones(z, (int)hdr[2], want);
        for (int k = 0; k < nimpl; k++) {
            impls[k].fn(z, (int)hdr[2], got);
            for (uint32_t i = 0; i < hdr[2]; i++) bad += differs(&got[i], &want[i]);
        }
        records++;
    }
    fclose(f);
    free(z);
    printf("%s: %ld records, %ld mismatches\n", path, records, bad);
    return bad || !records ? 1 : 0;
}

/* ---- cost ------------------------------------------------------------------------------------------------------------ */

static double now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
}

#define POLLS 8 /* distinct polls cycled through, so one poll's worth of data is not all the cache sees */

static void bench(long iterations, const char *only) {
    struct impl impls[3];
    int n = 0;
    impls[n++] = (struct impl){"reference", ref_zones};
    n += collect(impls + 1);
    struct vth_zone *z = calloc((size_t)POLLS * VTH_MAX_ZONES, sizeof *z);
    for (int p = 0; p < POLLS * VTH_MAX_ZONES; p++) { /* a picture-like bump in each channel, 130 000 pixels: a 1080p area */
        z[p].area_id = (uint32_t)(p % VTH_MAX_ZONES);
        uint16_t *ch[3] = {z[p].r, z[p].g, z[p].b};
        for (int c = 0; c < 3; c++) {
            int centre = (int)below(256), width = 4 + (int)below(40);
            for (int s = 0; s < 260; s++) {
                int code = centre + (int)below((unsigned)width) - (int)below((unsigned)width);
                add(ch[c], code < 0 ? 0 : code > 255 ? 255 : code, 500);
            }
        }
    }
    struct zone_out out[VTH_MAX_ZONES];
    for (int k = 0; k < n; k++) {
        if (only && strcmp(only, impls[k].name)) continue;
        double t0 = now_ns();
        for (long i = 0; i < iterations; i++) impls[k].fn(z + (i % POLLS) * VTH_MAX_ZONES, VTH_MAX_ZONES, out);
        double per = (now_ns() - t0) / (double)iterations;
        printf("%-20s %10.3f us a poll of 16 areas (%ld polls)\n", impls[k].name, per / 1e3, iterations);
    }
    free(z);
}

int main(int argc, char **argv) {
    long zones = 400000, bench_n = 0, tie_target = 20000;
    const char *only = NULL;
    int ch;
    while ((ch = getopt(argc, argv, "n:s:b:t:i:f:")) != -1) {
        switch (ch) {
        case 'n': zones = atol(optarg); break;
        case 's': rng_state ^= (uint64_t)atoll(optarg) * 0x9e3779b97f4a7c15ull; break;
        case 'b': bench_n = atol(optarg); break;
        case 't': tie_target = atol(optarg); break;
        case 'i': only = optarg; break;
        case 'f': return replay(optarg);
        default: fputs("usage: reduce_test [-n ZONES] [-t HALFWAYS] [-s SEED] | -f DUMP | -b ITERATIONS [-i reference|scalar|neon]\n", stderr); return 2;
        }
    }
    if (bench_n) {
        bench(bench_n, only);
        return 0;
    }
    printf("implementation chosen by reduce_zones: %s\n", reduce_impl());
    int rc = compare(zones);
    rc |= ties(tie_target);
    puts(rc ? "FAILED" : "reduce: identical to the reference");
    return rc;
}
