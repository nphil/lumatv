#include "vpl_plan.h"

#include <math.h>
#include <stdio.h>

static int js_round(double x) { return (int)floor(x + 0.5); }

void vpl_drift(double t_sec, int ampX, int ampY, int periodX, int periodY, int *dx, int *dy) {
    const double two_pi = 6.283185307179586;
    *dx = periodX > 0 ? js_round(ampX * sin(two_pi * t_sec / periodX)) : 0;
    *dy = periodY > 0 ? js_round(ampY * sin(two_pi * t_sec / periodY)) : 0;
}

double vpl_level(double minutes, int dim, double dimTo, int dimOverMin) {
    if (!dim || dimOverMin <= 0) return 1.0;
    double k = minutes / dimOverMin;
    if (k < 0) k = 0;
    if (k > 1) k = 1;
    return 1.0 - (1.0 - dimTo) * k;
}

double vpl_ease(double t) {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) * (-2 * t + 2) / 2;
}

int vpl_veil(double level, int phase, int64_t into_ms, int fadeMs) {
    double base = 1.0 - level;
    double a = base;
    double k = fadeMs > 0 ? (double)into_ms / fadeMs : 1.0;
    if (phase == 1) a = base + (1.0 - base) * vpl_ease(k);
    else if (phase == 2) a = 1.0;
    else if (phase == 3) a = base + (1.0 - base) * (1.0 - vpl_ease(k));
    if (a < 0) a = 0;
    if (a > 1) a = 1;
    return (int)floor(a * 255 + 0.5);
}

void vpl_rng_seed(vpl_rng *r, uint32_t seed) {
    /* xorshift32 is poor for the first outputs of a small seed (a bag came out in order): scramble the seed and discard a few */
    uint32_t x = seed * 2654435761u;
    x ^= x >> 15;
    x *= 2246822519u;
    x ^= x >> 13;
    r->s = x ? x : 0x9E3779B9u;
    for (int i = 0; i < 8; i++) vpl_rng_next(r);
}

uint32_t vpl_rng_next(vpl_rng *r) {
    uint32_t x = r->s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    return r->s = x;
}

void vpl_order_init(vpl_order *o, int n) {
    o->n = n < 0 ? 0 : n > VPL_MAX_CLIPS ? VPL_MAX_CLIPS : n;
    o->current = -1;
    o->bag_len = 0;
}

int vpl_order_next(vpl_order *o, int shuffle, vpl_rng *r) {
    int n = o->n;
    if (n <= 0) return -1;
    if (!shuffle) return (o->current + 1) % n;
    if (o->bag_len == 0) {
        int b[VPL_MAX_CLIPS];
        for (int i = 0; i < n; i++) b[i] = i;
        for (int j = n - 1; j > 0; j--) {
            int k = (int)(vpl_rng_next(r) % (uint32_t)(j + 1));
            int t = b[j];
            b[j] = b[k];
            b[k] = t;
        }
        if (n > 1 && b[0] == o->current) {
            int t = b[0];
            b[0] = b[1];
            b[1] = t;
        }
        for (int i = 0; i < n; i++) o->bag[i] = b[i];
        o->bag_len = n;
    }
    int out = o->bag[0];
    for (int i = 1; i < o->bag_len; i++) o->bag[i - 1] = o->bag[i];
    o->bag_len--;
    return out;
}

int vpl_clock_text(char *out, size_t cap, int hour, int minute, int twelve_hour_setting) {
    if (twelve_hour_setting) {
        int h = hour % 12;
        return snprintf(out, cap, "%d:%02d", h == 0 ? 12 : h, minute);
    }
    return snprintf(out, cap, "%02d:%02d", hour, minute);
}
