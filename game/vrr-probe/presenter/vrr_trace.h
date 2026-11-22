/* Deadline traces and the backend model that drive the presenter offline.
 *
 * A trace is a list of instants on the 90 kHz clock the stream uses. One list says when scans start, another when source pictures are due.
 * The model backend returns each cookie `lag_ns` (plus a seeded random 0..`jitter_ns`) after the event that could end the reads of its
 * surface: the scan that replaced the picture began, the drop was commanded, or the handback began. No real lag is known, so it is a scenario
 * input to sweep, not an assumption: the presenter never reads it and no event or count in a run releases a surface by itself. With jitter
 * the cookies return out of order.
 *
 * This is NOT a panel and not a measurement: the scan instants are given, not caused by the model, and nothing here shows what a display does.
 */
#ifndef VRR_TRACE_H
#define VRR_TRACE_H

#include <stddef.h>
#include <stdint.h>

#include "vrr_presenter.h"

enum { VRR_TRACE_MAX = 128, VRR_TICKS_PER_SECOND = 90000 };

typedef struct {
    char name[32];
    size_t n;
    int64_t ticks[VRR_TRACE_MAX];
} vrr_trace;

int64_t vrr_ticks_to_ns(int64_t ticks);                                                         /* nearest nanosecond */

/* `n` instants, `step` ticks apart, from 0. */
void vrr_trace_regular(vrr_trace *t, const char *name, size_t n, int64_t step_ticks);
/* `n` intervals make n + 1 instants, from 0. */
void vrr_trace_intervals(vrr_trace *t, const char *name, const int64_t *interval_ticks, size_t n);

/* The three traces of the 4K VRR stimulus, each about 624 ms: 120 Hz (76 instants), the 24 x 10 ms and 24 x 16 ms intervals in the order
 * random.Random(174).shuffle([10, 16] * 24) gives (49), and 48 x 13 ms (49). */
void vrr_trace_fixed_120(vrr_trace *t);
void vrr_trace_shuffled(vrr_trace *t);
void vrr_trace_fixed_13(vrr_trace *t);
int vrr_trace_shuffled_ms(int i);       /* the i-th interval of the shuffled trace in ms (0..47), for checking the table against Python */

typedef struct {
    int64_t decode_ns;      /* a picture is ready this long after its source time */
    int64_t delay_ns;       /* the first scan comes this long after the first source time */
    int64_t lag_ns;         /* the backend returns a cookie this long after the event that could end its reads, plus ... */
    int64_t jitter_ns;      /* ... a seeded random 0..jitter_ns for each cookie */
    unsigned capacity;      /* the presenter's bound on pictures owned (backpressure only), at most VRR_TRACE_MAX */
} vrr_sim_cfg;

/* What the run needs from the world. `make` builds source picture k (due at its trace instant, ready at `ready_ns`); `refused` takes
 * a refused picture's handle back; `release` is the presenter's release. `shown` and `dropped` see each command before the model answers it. */
typedef struct {
    void *user;
    int (*make)(void *user, size_t k, int64_t pts, int64_t ready_ns, vrr_picture *pic);
    void (*refused)(void *user, const vrr_picture *pic, int err);
    vrr_release_fn release;
    void (*shown)(void *user, const vrr_show *cmd);
    void (*dropped)(void *user, const vrr_drop *cmd);
} vrr_sim_hooks;

typedef struct {
    vrr_stats stats;
    size_t pictures;                    /* source pictures offered to the presenter */
    size_t refused;
    unsigned retire_errors;             /* vrr_retire() results other than VRR_OK */
    int outstanding_at_shutdown;        /* cookies the handback had still to see returned */
    unsigned leaked;                    /* pictures still owned at vrr_free() */
    size_t nlog;
    vrr_retirement log[VRR_TRACE_MAX];  /* in the order they retired */
} vrr_sim_result;

/* Events at the same instant run in the order retire, submit, scan. Returns 0, or -1 when `make` or the configuration failed. */
int vrr_sim_run(const vrr_sim_cfg *cfg, const vrr_trace *source, const vrr_trace *scans, const vrr_sim_hooks *hooks, vrr_sim_result *out);

#endif
