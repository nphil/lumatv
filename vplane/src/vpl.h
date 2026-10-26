/* lumatv-vplane: the video-plane player. Shared pieces: the log, the clock, and the parameters a start request carries.
 *
 * See docs/research/videoplane-saver-result.md for the design; each module's header says what it is for.
 */
#ifndef VPL_H
#define VPL_H

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

/* One line to stderr, one place: the service's log is that stream (vplane.sh redirects it to the state directory's vplane.log). The
 * same line is not repeated inside ten seconds, so a loop that keeps failing cannot fill the file. */
void vpl_say(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

/* CLOCK_MONOTONIC in milliseconds and microseconds. */
int64_t vpl_ms(void);
int64_t vpl_us(void);

/* Wall clock in milliseconds since the epoch (the dim curve counts from when the saver came up, which the requester says in wall time). */
int64_t vpl_wall_ms(void);

/* What a `start` or `update` request asks for. Defaults are the tokens of theme.json's `screensaver` (the requester sends its own). */
typedef struct {
    int everyMin;        /* a new print every this many minutes */
    int shuffle;         /* 1 bag of prints in random order, 0 in the order of the files */
    int dim;             /* 1 dims over time */
    double dimTo;        /* the brightness the dim ends at, 0..1 */
    int dimOverMin;      /* minutes the dim takes */
    int moveMs;          /* the drift takes a step this often */
    int fadeMs;          /* a change of print fades out and in, each this long */
    int ampX, ampY;      /* the drift's reach in pixels */
    int periodX, periodY;/* the drift's loop in seconds */
    int clock;           /* the corner clock is drawn */
    int twelveHour;      /* 12 hour face, no leading zero */
    int drift;           /* the picture drifts (the window moves) */
    int overlay;         /* 1 the window carries the veil and the clock, 0 it carries nothing */
    int caption;         /* the print's caption is drawn (the art mode's `caption` switch) */
    uint32_t ink;        /* the caption's artist name, 0xRRGGBB, and its alpha 0..255 (the QML's ink at 0.7) */
    int inkAlpha;
    uint32_t ink2;       /* the rest of the caption and the corner clock (the QML's ink2 at 0.7), and its alpha */
    int ink2Alpha;
    int64_t sinceMs;     /* wall time the saver came up (0: now) */
    int ttlMs;           /* the player stops by itself this long after the last request */
    char clip[64];       /* start with this print (empty: the next in the order) */
    int everyMs;         /* tests: a change of print this often (0: everyMin) */
    char clockAt[8];     /* tests: the clock shows this text instead of the time */
} vpl_params;

void vpl_params_default(vpl_params *p);

#endif
