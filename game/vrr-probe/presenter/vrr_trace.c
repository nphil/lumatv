#include "vrr_trace.h"

#include <stdio.h>
#include <string.h>

int64_t vrr_ticks_to_ns(int64_t ticks) { return (ticks * 200000 + 9) / 18; }       /* 1e9 / 90000 = 100000 / 9 ns per tick, rounded */

static void start(vrr_trace *t, const char *name) {
    memset(t, 0, sizeof *t);
    snprintf(t->name, sizeof t->name, "%s", name);
}

void vrr_trace_regular(vrr_trace *t, const char *name, size_t n, int64_t step_ticks) {
    start(t, name);
    t->n = n < VRR_TRACE_MAX ? n : VRR_TRACE_MAX;
    for (size_t i = 0; i < t->n; i++) t->ticks[i] = (int64_t)i * step_ticks;
}

void vrr_trace_intervals(vrr_trace *t, const char *name, const int64_t *interval_ticks, size_t n) {
    start(t, name);
    t->n = n + 1 < VRR_TRACE_MAX ? n + 1 : VRR_TRACE_MAX;
    int64_t at = 0;
    for (size_t i = 0; i < t->n; i++) {
        t->ticks[i] = at;
        if (i < n) at += interval_ticks[i];
    }
}

void vrr_trace_fixed_120(vrr_trace *t) { vrr_trace_regular(t, "fixed 120 Hz", 76, 750); }

void vrr_trace_fixed_13(vrr_trace *t) { vrr_trace_regular(t, "fixed 13 ms", 49, 13 * 90); }

/* random.Random(174).shuffle([10, 16] * 24), in milliseconds; test_presenter.py compares it with Python. */
static const int shuffled_ms[48] = {10, 10, 10, 16, 16, 10, 16, 10, 16, 10, 16, 16, 10, 16, 10, 16, 10, 10, 10, 10, 16, 16, 16, 10,
                                    10, 10, 16, 16, 10, 16, 16, 16, 10, 10, 10, 16, 16, 16, 16, 10, 16, 10, 10, 16, 16, 16, 10, 10};

void vrr_trace_shuffled(vrr_trace *t) {
    int64_t iv[48];
    for (int i = 0; i < 48; i++) iv[i] = shuffled_ms[i] * 90;
    vrr_trace_intervals(t, "shuffled 10/16 ms", iv, 48);
}

int vrr_trace_shuffled_ms(int i) { return shuffled_ms[i]; }

typedef struct {
    int64_t due;
    uint64_t seq;
    vrr_cookie cookie;
} pending;

typedef struct {
    const vrr_sim_cfg *cfg;
    const vrr_sim_hooks *hooks;
    vrr_sim_result *out;
    vrr_presenter *p;
    pending pend[VRR_TRACE_MAX];
    size_t npend;
    uint64_t seq;
    int overflow;
    vrr_cookie on_screen;
    int64_t now;
} sim;

static void schedule(sim *s, vrr_cookie c, int64_t due) {
    if (s->npend == VRR_TRACE_MAX) {
        s->overflow = 1;
        return;
    }
    s->pend[s->npend++] = (pending){due, s->seq++, c};
}

static size_t earliest(const sim *s) {
    size_t best = 0;
    for (size_t i = 1; i < s->npend; i++)
        if (s->pend[i].due < s->pend[best].due || (s->pend[i].due == s->pend[best].due && s->pend[i].seq < s->pend[best].seq)) best = i;
    return best;
}

static void deliver(sim *s, size_t i, int64_t now) {
    vrr_cookie c = s->pend[i].cookie;
    s->pend[i] = s->pend[--s->npend];
    s->now = now;
    if (vrr_retire(s->p, c, now) != VRR_OK) s->out->retire_errors++;
}

static void model_show(void *user, const vrr_show *cmd) {
    sim *s = user;
    if (s->hooks->shown) s->hooks->shown(s->hooks->user, cmd);
    if (cmd->kind != VRR_SHOW_NEW) return;
    if (cmd->supersedes.gen) schedule(s, cmd->supersedes, cmd->scan_ns + s->cfg->tail_ns);
    s->on_screen = cmd->cookie;
}

static void model_drop(void *user, const vrr_drop *cmd) {
    sim *s = user;
    if (s->hooks->dropped) s->hooks->dropped(s->hooks->user, cmd);
    schedule(s, cmd->cookie, s->now + s->cfg->drop_ns);
}

static void model_release(void *user, void *handle, const vrr_retirement *r) {
    sim *s = user;
    if (s->out->nlog < VRR_TRACE_MAX) s->out->log[s->out->nlog++] = *r;
    s->hooks->release(s->hooks->user, handle, r);
}

int vrr_sim_run(const vrr_sim_cfg *cfg, const vrr_trace *source, const vrr_trace *scans, const vrr_sim_hooks *hooks, vrr_sim_result *out) {
    if (!cfg || !source || !scans || !hooks || !hooks->make || !hooks->release || !out) return -1;
    memset(out, 0, sizeof *out);
    sim s = {.cfg = cfg, .hooks = hooks, .out = out};
    vrr_config pc = {cfg->capacity, model_release, &s, {&s, model_show, model_drop}};
    s.p = vrr_new(&pc);
    if (!s.p) return -1;

    int64_t ready[VRR_TRACE_MAX], scan_at[VRR_TRACE_MAX];
    for (size_t i = 0; i < source->n; i++) ready[i] = vrr_ticks_to_ns(source->ticks[i]) + cfg->decode_ns;
    for (size_t i = 0; i < scans->n; i++) scan_at[i] = cfg->delay_ns + vrr_ticks_to_ns(scans->ticks[i]);

    size_t next_pic = 0, next_scan = 0;
    int64_t end = 0;
    int rc = 0;
    for (;;) {
        int64_t t_retire = INT64_MAX, t_pic = INT64_MAX, t_scan = INT64_MAX;
        size_t due = 0;
        if (s.npend) {
            due = earliest(&s);
            t_retire = s.pend[due].due;
        }
        if (next_pic < source->n) t_pic = ready[next_pic];
        if (next_scan < scans->n) t_scan = scan_at[next_scan];
        int64_t t = t_retire < t_pic ? (t_retire < t_scan ? t_retire : t_scan) : (t_pic < t_scan ? t_pic : t_scan);
        if (t == INT64_MAX) break;
        end = t;
        s.now = t;
        if (t == t_retire) {
            deliver(&s, due, t);
        } else if (t == t_pic) {
            vrr_picture pic;
            if (hooks->make(hooks->user, next_pic, source->ticks[next_pic], t, &pic) != 0) {
                rc = -1;
                break;
            }
            pic.ready_ns = t;
            out->pictures++;
            int err = vrr_submit(s.p, &pic, NULL);
            if (err != VRR_OK) {
                out->refused++;
                if (hooks->refused) hooks->refused(hooks->user, &pic, err);
            }
            next_pic++;
        } else {
            vrr_scan(s.p, t);
            next_scan++;
        }
    }

    s.now = end;
    out->outstanding_at_shutdown = vrr_shutdown(s.p, end);
    while (s.npend) deliver(&s, earliest(&s), end);
    if (s.on_screen.gen && vrr_cookie_state(s.p, s.on_screen) != VRR_ST_NONE && vrr_retire(s.p, s.on_screen, end) != VRR_OK) out->retire_errors++;
    out->stats = *vrr_get_stats(s.p);
    out->leaked = vrr_free(s.p);
    return rc == 0 && !s.overflow ? 0 : -1;
}
