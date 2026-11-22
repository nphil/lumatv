#include "vrr_presenter.h"

#include <stdlib.h>
#include <string.h>

typedef struct {
    vrr_state state;
    uint32_t gen;                       /* bumped at every allocation: a cookie of an earlier tenant never matches */
    vrr_picture pic;
    vrr_drop_reason reason;
    uint32_t scans;
    int64_t first_scan_ns, last_scan_ns, decided_ns;
} slot;

struct vrr_presenter {
    vrr_config cfg;
    slot *slots;
    unsigned *queue;                    /* slot indexes of the QUEUED pictures, oldest first, as a ring */
    unsigned q_head, q_len;
    unsigned owned;
    int cur;                            /* slot of the CURRENT picture, -1 when none */
    uint32_t epoch;
    int have_tb, have_pts, have_scan;
    vrr_timebase tb;
    int64_t last_pts, last_scan_ns;
    int closed, busy;
    vrr_stats stats;
};

const char *vrr_strerror(int rc) {
    static const char *const names[VRR_E_COUNT] = {
        "ok", "bad argument", "no handle", "stale epoch", "future epoch", "duplicate", "timebase changed",
        "out of order", "full", "closed", "stale cookie", "premature retirement", "re-entered",
    };
    return rc <= 0 && -rc < VRR_E_COUNT ? names[-rc] : "unknown";
}

vrr_presenter *vrr_new(const vrr_config *cfg) {
    if (!cfg || !cfg->release || cfg->capacity < 1 || cfg->capacity > VRR_MAX_CAPACITY) return NULL;
    vrr_presenter *p = calloc(1, sizeof *p);
    if (!p) return NULL;
    p->cfg = *cfg;
    p->slots = calloc(cfg->capacity, sizeof *p->slots);
    p->queue = calloc(cfg->capacity, sizeof *p->queue);
    if (!p->slots || !p->queue) {
        free(p->slots);
        free(p->queue);
        free(p);
        return NULL;
    }
    p->cur = -1;
    return p;
}

static int enter(vrr_presenter *p) {
    if (p->busy) return VRR_E_REENTRANT;
    p->busy = 1;
    return VRR_OK;
}

static int leave(vrr_presenter *p, int rc) {
    p->busy = 0;
    return rc;
}

static vrr_cookie cookie_of(const vrr_presenter *p, unsigned idx) {
    vrr_cookie c = {idx, p->slots[idx].gen};
    return c;
}

static int cookie_valid(const vrr_presenter *p, vrr_cookie c) {
    return c.slot < p->cfg.capacity && p->slots[c.slot].state != VRR_ST_NONE && p->slots[c.slot].gen == c.gen;
}

static unsigned queue_peek(const vrr_presenter *p, unsigned pos) { return p->queue[(p->q_head + pos) % p->cfg.capacity]; }

static unsigned queue_pop(vrr_presenter *p) {
    unsigned idx = p->queue[p->q_head];
    p->q_head = (p->q_head + 1) % p->cfg.capacity;
    p->q_len--;
    return idx;
}

static int same_timebase(vrr_timebase a, vrr_timebase b) { return (int64_t)a.num * b.den == (int64_t)b.num * a.den; }

static int submit_locked(vrr_presenter *p, const vrr_picture *pic, vrr_cookie *cookie) {
    if (p->closed) return VRR_E_CLOSED;
    if (!pic || pic->tb.num <= 0 || pic->tb.den <= 0) return VRR_E_ARG;
    if (!pic->handle) return VRR_E_NO_HANDLE;
    if (pic->epoch < p->epoch) return VRR_E_STALE_EPOCH;
    if (pic->epoch > p->epoch) return VRR_E_FUTURE_EPOCH;
    for (unsigned i = 0; i < p->cfg.capacity; i++) {
        const slot *s = &p->slots[i];
        if (s->state != VRR_ST_NONE && (s->pic.handle == pic->handle || (s->pic.epoch == pic->epoch && s->pic.id == pic->id))) return VRR_E_DUPLICATE;
    }
    if (p->have_tb && !same_timebase(p->tb, pic->tb)) return VRR_E_TIMEBASE;
    if (p->have_pts && pic->pts <= p->last_pts) return VRR_E_ORDER;
    if (p->owned == p->cfg.capacity) return VRR_E_FULL;

    unsigned idx = 0;
    while (p->slots[idx].state != VRR_ST_NONE) idx++;                  /* owned < capacity: a free slot exists */
    slot *s = &p->slots[idx];
    s->state = VRR_ST_QUEUED;
    s->gen++;
    s->pic = *pic;
    s->reason = VRR_DROP_SUPERSEDED;
    s->scans = 0;
    s->first_scan_ns = s->last_scan_ns = s->decided_ns = -1;
    p->queue[(p->q_head + p->q_len++) % p->cfg.capacity] = idx;
    p->owned++;
    p->have_tb = p->have_pts = 1;
    p->tb = pic->tb;
    p->last_pts = pic->pts;
    p->stats.submitted++;
    if (p->owned > p->stats.peak_owned) p->stats.peak_owned = p->owned;
    if (cookie) *cookie = cookie_of(p, idx);
    return VRR_OK;
}

int vrr_submit(vrr_presenter *p, const vrr_picture *pic, vrr_cookie *cookie) {
    if (!p) return VRR_E_ARG;
    int rc = enter(p);
    if (rc != VRR_OK) return rc;
    rc = submit_locked(p, pic, cookie);
    if (rc != VRR_OK) p->stats.rejected[-rc]++;
    return leave(p, rc);
}

static void drop_front(vrr_presenter *p, vrr_drop_reason reason, int64_t now_ns) {
    unsigned idx = queue_pop(p);
    slot *s = &p->slots[idx];
    s->state = VRR_ST_DROPPED;
    s->reason = reason;
    s->decided_ns = now_ns;
    p->stats.dropped[reason]++;
    if (p->cfg.backend.drop) {
        vrr_drop cmd = {cookie_of(p, idx), s->pic.epoch, s->pic.id, reason, s->pic.handle};
        p->cfg.backend.drop(p->cfg.backend.user, &cmd);
    }
}

static void show(vrr_presenter *p, vrr_show_kind kind, unsigned idx, vrr_cookie supersedes, int64_t scan_ns) {
    if (!p->cfg.backend.show) return;
    const slot *s = &p->slots[idx];
    vrr_show cmd = {kind, cookie_of(p, idx), supersedes, s->pic.epoch, s->pic.id, s->pic.pts, s->pic.tb, scan_ns, s->pic.handle};
    p->cfg.backend.show(p->cfg.backend.user, &cmd);
}

static int scan_locked(vrr_presenter *p, int64_t scan_ns) {
    if (p->closed) return VRR_E_CLOSED;
    if (p->have_scan && scan_ns <= p->last_scan_ns) return VRR_E_ORDER;
    p->have_scan = 1;
    p->last_scan_ns = scan_ns;
    p->stats.scans++;

    int pick = -1;                                                      /* the newest picture that is ready */
    for (unsigned pos = 0; pos < p->q_len; pos++)
        if (p->slots[queue_peek(p, pos)].pic.ready_ns <= scan_ns) pick = (int)pos;
    if (pick >= 0) {
        for (int n = 0; n < pick; n++) drop_front(p, VRR_DROP_SUPERSEDED, scan_ns);     /* older than what is shown: never shown */
        unsigned idx = queue_pop(p);
        slot *s = &p->slots[idx];
        vrr_cookie supersedes = {0, 0};
        if (p->cur >= 0) {
            slot *old = &p->slots[p->cur];
            old->state = VRR_ST_SUPERSEDED;
            old->decided_ns = scan_ns;
            supersedes = cookie_of(p, (unsigned)p->cur);
        }
        s->state = VRR_ST_CURRENT;
        s->scans = 1;
        s->first_scan_ns = s->last_scan_ns = scan_ns;
        p->cur = (int)idx;
        p->stats.scans_new++;
        p->stats.shown++;
        show(p, VRR_SHOW_NEW, idx, supersedes, scan_ns);
        return VRR_SCANNED_NEW;
    }
    if (p->cur >= 0) {
        slot *s = &p->slots[p->cur];
        s->scans++;
        s->last_scan_ns = scan_ns;
        p->stats.scans_repeat++;
        if (s->pic.epoch != p->epoch) p->stats.repeats_stale_epoch++;
        show(p, VRR_SHOW_REPEAT, (unsigned)p->cur, (vrr_cookie){0, 0}, scan_ns);
        return VRR_SCANNED_REPEAT;
    }
    p->stats.scans_underrun++;
    return VRR_SCANNED_UNDERRUN;
}

int vrr_scan(vrr_presenter *p, int64_t scan_ns) {
    if (!p) return VRR_E_ARG;
    int rc = enter(p);
    if (rc != VRR_OK) return rc;
    return leave(p, scan_locked(p, scan_ns));
}

static int retire_locked(vrr_presenter *p, vrr_cookie cookie, int64_t now_ns) {
    if (!cookie_valid(p, cookie)) return VRR_E_STALE_COOKIE;
    slot *s = &p->slots[cookie.slot];
    if (s->state == VRR_ST_QUEUED || s->state == VRR_ST_CURRENT) return VRR_E_PREMATURE;
    vrr_retirement r = {s->pic.epoch, s->pic.id, s->pic.pts, s->pic.tb, s->state == VRR_ST_DROPPED ? VRR_DROPPED : VRR_DISPLAYED,
                        s->reason, s->scans, s->pic.ready_ns, s->first_scan_ns, s->last_scan_ns, s->decided_ns, now_ns};
    void *handle = s->pic.handle;
    if (r.outcome == VRR_DROPPED) p->stats.retired_dropped++;
    else p->stats.retired_displayed++;
    s->state = VRR_ST_NONE;
    memset(&s->pic, 0, sizeof s->pic);
    p->owned--;
    p->cfg.release(p->cfg.release_user, handle, &r);
    return VRR_OK;
}

int vrr_retire(vrr_presenter *p, vrr_cookie cookie, int64_t now_ns) {
    if (!p) return VRR_E_ARG;
    int rc = enter(p);
    if (rc != VRR_OK) return rc;
    return leave(p, retire_locked(p, cookie, now_ns));
}

int vrr_flush(vrr_presenter *p, uint32_t new_epoch, int64_t now_ns) {
    if (!p) return VRR_E_ARG;
    int rc = enter(p);
    if (rc != VRR_OK) return rc;
    if (p->closed) return leave(p, VRR_E_CLOSED);
    if (new_epoch <= p->epoch) return leave(p, VRR_E_STALE_EPOCH);
    while (p->q_len) drop_front(p, VRR_DROP_FLUSH, now_ns);
    p->epoch = new_epoch;
    p->have_tb = p->have_pts = 0;
    return leave(p, VRR_OK);
}

int vrr_shutdown(vrr_presenter *p, int64_t now_ns) {
    if (!p) return VRR_E_ARG;
    int rc = enter(p);
    if (rc != VRR_OK) return rc;
    if (!p->closed) {
        p->closed = 1;
        while (p->q_len) drop_front(p, VRR_DROP_SHUTDOWN, now_ns);
        if (p->cur >= 0) {                                              /* no scan will repeat it again */
            p->slots[p->cur].state = VRR_ST_SUPERSEDED;
            p->slots[p->cur].decided_ns = now_ns;
            p->cur = -1;
        }
    }
    return leave(p, (int)p->owned);
}

unsigned vrr_owned(const vrr_presenter *p) { return p->owned; }

vrr_state vrr_cookie_state(const vrr_presenter *p, vrr_cookie cookie) { return cookie_valid(p, cookie) ? p->slots[cookie.slot].state : VRR_ST_NONE; }

uint32_t vrr_epoch(const vrr_presenter *p) { return p->epoch; }

const vrr_stats *vrr_get_stats(const vrr_presenter *p) { return &p->stats; }

unsigned vrr_free(vrr_presenter *p) {
    if (!p) return 0;
    unsigned abandoned = p->owned;
    free(p->slots);
    free(p->queue);
    free(p);
    return abandoned;
}
