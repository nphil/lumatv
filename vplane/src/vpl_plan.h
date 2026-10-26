/* The saver's arithmetic, kept apart from everything that touches the TV so it can be tested to the digit: the drift, the dim, the fade's
 * easing, the order of the prints and the corner clock's text. Each function is the one the QML (panels/Screensaver.qml) has, with
 * the same numbers, so the video saver and the GPU saver move alike.
 */
#ifndef VPL_PLAN_H
#define VPL_PLAN_H

#include <stddef.h>
#include <stdint.h>

/* The drift in whole pixels at `t` seconds after the saver came up: round(ampX sin(2 pi t / periodX)), round(ampY sin(2 pi t / periodY)).
 * JavaScript's Math.round (halves go up), as Screensaver.qml's shift timer computes it. The caller samples it every `moveMs`. */
void vpl_drift(double t_sec, int ampX, int ampY, int periodX, int periodY, int *dx, int *dy);

/* The brightness the dim has reached after `minutes` in the saver: 1 down to `dimTo` over `dimOverMin`; 1 without `dim`. */
double vpl_level(double minutes, int dim, double dimTo, int dimOverMin);

/* Easing.InOutQuad over 0..1. */
double vpl_ease(double t);

/* The veil's alpha, 0..255, over a picture at `level` (the veil carries the dim: alpha = 1 - level) while a change of print is `into`
 * ms into its fade (out: 0..fadeMs rises to black, hold at black, in: falls back). `phase` is 0 not changing, 1 out, 2 hold, 3 in. */
int vpl_veil(double level, int phase, int64_t into_ms, int fadeMs);

/* A small generator (xorshift32): the shuffle must not depend on the C library's rand, so a test can pin an order. */
typedef struct { uint32_t s; } vpl_rng;
void vpl_rng_seed(vpl_rng *r, uint32_t seed);
uint32_t vpl_rng_next(vpl_rng *r);

/* The order of the prints, Screensaver.qml's nextIndex: in sequence it is the one after the last; shuffled, a bag holds every print once
 * in random order and a print never repeats before all have been shown, nor is the first of a new bag the one just shown. */
#define VPL_MAX_CLIPS 64
typedef struct {
    int n;
    int current;                 /* the print shown (-1 before the first) */
    int bag[VPL_MAX_CLIPS];
    int bag_len;
} vpl_order;

void vpl_order_init(vpl_order *o, int n);
int vpl_order_next(vpl_order *o, int shuffle, vpl_rng *r);   /* the index to show next, -1 without prints; does not move `current` */

/* The corner clock's text as Screensaver.qml's `Qt.formatTime(now, twelveHour ? "h:mm" : "HH:mm")` writes it. Qt reads "h" as 0-23 without a
 * leading zero unless the format has an AM/PM mark, so the "twelve hour" face of the GPU saver shows 17:05 at five past five, not 5:05; the
 * video saver draws the same text so the two look alike, and the defect (if it is one) is the QML's to fix in both. Returns its length. */
int vpl_clock_text(char *out, size_t cap, int hour, int minute, int twelve_hour_setting);

#endif
