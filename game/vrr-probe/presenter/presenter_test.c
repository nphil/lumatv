/* The presenter against a real HEVC decoder. Each ownership, selection and retirement edge case runs on decoded surfaces and is judged by the decoder's own
 * buffer counts (hevc_source.h); then three deadline traces are driven past three source cadences, with cookies returned at once and late. Deadline traces only: nothing here shows what a panel does.
 *   presenter_test STREAM.hevc STREAM.order            run everything
 *   presenter_test STREAM.hevc STREAM.order --report   also print the trace matrix
 *   presenter_test STREAM.hevc STREAM.order --print-shuffled
 */
#undef NDEBUG
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "hevc_source.h"
#include "vrr_presenter.h"
#include "vrr_trace.h"

#define MS 1000000LL

static const char *stream_path, *order_path;

/* ---- a presenter wired to a decoder, with every command and retirement recorded --------------------------------------------- */

enum { LOG = 128 };

typedef struct {
    hevc_source *src;
    vrr_presenter *p;
    int early_release;                  /* the faulty owner of the negative control: frees a dropped surface at the drop command */
    vrr_show shows[LOG];
    size_t nshows;
    vrr_drop drops[LOG];
    size_t ndrops;
    vrr_retirement rets[LOG];
    size_t nrets;
} rig;

static void rig_show(void *u, const vrr_show *c) {
    rig *r = u;
    assert(r->nshows < LOG);
    r->shows[r->nshows++] = *c;
    hevc_surface_read(r->src, c->handle);
}

static void rig_drop(void *u, const vrr_drop *c) {
    rig *r = u;
    assert(r->ndrops < LOG);
    r->drops[r->ndrops++] = *c;
    if (r->early_release) hevc_surface_drop_refs_early(r->src, c->handle);
}

static void rig_release(void *u, void *handle, const vrr_retirement *rt) {
    rig *r = u;
    assert(r->nrets < LOG);
    r->rets[r->nrets++] = *rt;
    hevc_surface_retire(r->src, handle);
}

static void rig_open(rig *r, unsigned capacity) {
    memset(r, 0, sizeof *r);
    int64_t pts[LOG];
    for (int i = 0; i < LOG; i++) pts[i] = i * 750;                    /* 120 pictures per second on the 90 kHz clock */
    char err[256];
    r->src = hevc_source_open(stream_path, order_path, pts, LOG, err, sizeof err);
    if (!r->src) {
        fprintf(stderr, "open: %s\n", err);
        exit(1);
    }
    vrr_config cfg = {capacity, rig_release, r, {r, rig_show, rig_drop}};
    r->p = vrr_new(&cfg);
    assert(r->p);
}

/* The next decoded picture, as a presenter picture of the presenter's current epoch. */
static hevc_surface *rig_pull(rig *r, vrr_picture *pic, int64_t ready_ns) {
    hevc_surface *s = NULL;
    int rc = hevc_source_next(r->src, &s);
    if (rc != 0) {
        fprintf(stderr, "decode: %d %s\n", rc, hevc_source_error(r->src));
        exit(1);
    }
    *pic = (vrr_picture){vrr_epoch(r->p), hevc_surface_id(s), hevc_surface_pts(s), {1, 90000}, ready_ns, s};
    return s;
}

static vrr_cookie rig_submit(rig *r, int64_t ready_ns) {
    vrr_picture pic;
    vrr_cookie c;
    rig_pull(r, &pic, ready_ns);
    int rc = vrr_submit(r->p, &pic, &c);
    assert(rc == VRR_OK);
    return c;
}

/* Pictures decoded and thrown away unpresented: the decoder keeps cycling its pool under whatever the presenter holds. */
static void rig_churn(rig *r, unsigned n) {
    for (unsigned i = 0; i < n; i++) {
        vrr_picture pic;
        hevc_surface_retire(r->src, rig_pull(r, &pic, 0));
    }
}

static vrr_state st(const rig *r, vrr_cookie c) { return vrr_cookie_state(r->p, c); }
static int same(vrr_cookie a, vrr_cookie b) { return a.slot == b.slot && a.gen == b.gen; }

static void oracle_clean(const hevc_oracle *o) {
    assert(o->recycled_live == 0 && o->freed_while_live == 0 && o->integrity_failures == 0 && o->release_ref_errors == 0);
    assert(o->double_retire == 0 && o->read_after_free == 0);
}

/* Closes the presenter and the decoder. Every surface must be back, and the decoder must have no buffer left. */
static hevc_oracle rig_close(rig *r) {
    unsigned leaked = vrr_free(r->p);
    assert(leaked == 0);
    hevc_oracle o;
    int64_t outstanding = hevc_source_close(r->src, &o);
    assert(outstanding == 0 && o.live == 0 && o.pictures == o.retired && o.allocated == o.freed);
    oracle_clean(&o);
    return o;
}

/* Shuts down and returns every cookie that is still outstanding. */
static void rig_finish(rig *r, int64_t now, const vrr_cookie *left, size_t n) {
    int owned = vrr_shutdown(r->p, now);
    assert(owned == (int)n);
    for (size_t i = 0; i < n; i++) {
        int rc = vrr_retire(r->p, left[i], now);
        assert(rc == VRR_OK);
    }
    assert(vrr_owned(r->p) == 0);
}

/* ---- B-frame reordering ------------------------------------------------------------------------------------------------------ */

static void test_bframe_reordering(void) {
    rig r;
    rig_open(&r, 8);
    size_t n = hevc_source_pictures(r.src);
    assert(n == 80);
    size_t reordered = 0;
    for (size_t au = 1; au < n; au++)
        if (hevc_source_au_display(r.src, au) < hevc_source_au_display(r.src, au - 1)) reordered++;
    assert(reordered >= 20);                                            /* decode order is not display order */

    uint64_t ids[80];
    vrr_cookie on_screen = {0, 0};
    for (size_t k = 0; k < n; k++) {
        vrr_picture pic;
        hevc_surface *s = rig_pull(&r, &pic, (int64_t)k * 8 * MS);
        assert(hevc_surface_display(s) == k && pic.pts == (int64_t)k * 750);
        ids[k] = pic.id;
        for (size_t j = 0; j < k; j++) assert(ids[j] != ids[k]);        /* every tag is unique, and none is the display index */
        assert(pic.id != k);
        vrr_cookie c;
        int rc = vrr_submit(r.p, &pic, &c);
        assert(rc == VRR_OK);
        assert(vrr_scan(r.p, (int64_t)k * 8 * MS + 1 * MS) == VRR_SCANNED_NEW);
        const vrr_show *show = &r.shows[r.nshows - 1];
        assert(show->kind == VRR_SHOW_NEW && same(show->cookie, c) && show->id == pic.id && show->pts == pic.pts);
        if (show->supersedes.gen) {
            assert(same(show->supersedes, on_screen));
            rc = vrr_retire(r.p, show->supersedes, (int64_t)k * 8 * MS + 2 * MS);
            assert(rc == VRR_OK);
        }
        on_screen = c;
    }
    hevc_surface *none;
    assert(hevc_source_next(r.src, &none) == 1);                        /* the stream is finished */
    rig_finish(&r, 700 * MS, &on_screen, 1);

    assert(r.nrets == n && vrr_get_stats(r.p)->dropped[VRR_DROP_SUPERSEDED] == 0);
    for (size_t k = 0; k < n; k++)                                      /* retired in display order, each with its own tag and timestamp */
        assert(r.rets[k].pts == (int64_t)k * 750 && r.rets[k].id == ids[k] && r.rets[k].outcome == VRR_DISPLAYED && r.rets[k].scans == 1);
    size_t lookahead = hevc_source_max_lookahead(r.src);
    assert(lookahead >= 2);                                             /* access units were fed ahead of the pictures that came out */
    hevc_oracle o = rig_close(&r);
    assert(o.pictures == n && o.handouts > o.distinct);                 /* the pool reused memory: the checks ran under pressure */
    printf("bframe_reordering ok (80 pictures, %zu reorderings, lookahead %zu access units)\n", reordered, lookahead);
}

/* ---- hold until retirement --------------------------------------------------------------------------------------------------- */

static void test_hold_until_retirement(void) {
    rig r;
    rig_open(&r, 8);
    vrr_picture pa, pb, pc, pd;
    vrr_cookie a, b, c, d;
    hevc_surface *sa = rig_pull(&r, &pa, 0), *sb = rig_pull(&r, &pb, 0), *sc = rig_pull(&r, &pc, 0);
    assert(vrr_submit(r.p, &pa, &a) == VRR_OK && vrr_submit(r.p, &pb, &b) == VRR_OK && vrr_submit(r.p, &pc, &c) == VRR_OK);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);
    assert(st(&r, a) == VRR_ST_DROPPED && st(&r, b) == VRR_ST_DROPPED && st(&r, c) == VRR_ST_CURRENT);
    assert(r.ndrops == 2 && same(r.drops[0].cookie, a) && same(r.drops[1].cookie, b) && r.drops[0].reason == VRR_DROP_SUPERSEDED);
    assert(r.nrets == 0 && vrr_owned(r.p) == 3);                        /* a drop is not a retirement */

    rig_churn(&r, 20);                                                  /* the decoder keeps going while all three are held */
    hevc_surface *held[3] = {sa, sb, sc};
    for (int i = 0; i < 3; i++) {
        assert(hevc_surface_planes_freed(held[i]) == 0 && hevc_surface_refs(held[i]) >= 1);
        assert(hevc_surface_read(r.src, held[i]) == 0);
    }

    rig_pull(&r, &pd, 8 * MS);
    assert(vrr_submit(r.p, &pd, &d) == VRR_OK && vrr_scan(r.p, 9 * MS) == VRR_SCANNED_NEW);
    assert(same(r.shows[r.nshows - 1].supersedes, c) && st(&r, c) == VRR_ST_SUPERSEDED);
    assert(r.nrets == 0 && vrr_owned(r.p) == 4);                        /* the picture that was shown is held until its cookie returns */

    assert(vrr_retire(r.p, a, 10 * MS) == VRR_OK);                      /* a dropped picture comes back through the same door */
    assert(r.nrets == 1 && r.rets[0].outcome == VRR_DROPPED && r.rets[0].reason == VRR_DROP_SUPERSEDED && r.rets[0].scans == 0);
    assert(r.rets[0].first_scan_ns == -1 && r.rets[0].decided_ns == 1 * MS && r.rets[0].retire_ns == 10 * MS);
    assert(vrr_retire(r.p, b, 10 * MS) == VRR_OK && r.nrets == 2);
    assert(vrr_retire(r.p, c, 10 * MS) == VRR_OK);
    assert(r.nrets == 3 && r.rets[2].outcome == VRR_DISPLAYED && r.rets[2].scans == 1);
    assert(r.rets[2].first_scan_ns == 1 * MS && r.rets[2].decided_ns == 9 * MS && r.rets[2].retire_ns == 10 * MS);
    assert(vrr_owned(r.p) == 1 && st(&r, d) == VRR_ST_CURRENT);

    rig_finish(&r, 11 * MS, &d, 1);
    hevc_oracle o = rig_close(&r);
    assert(o.handouts > o.distinct);
    printf("hold_until_retirement ok\n");
}

/* ---- repeats ----------------------------------------------------------------------------------------------------------------- */

static void test_repeat_holds_one_surface(void) {
    rig r;
    rig_open(&r, 4);
    vrr_cookie a = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);
    for (int i = 2; i <= 5; i++) assert(vrr_scan(r.p, i * MS) == VRR_SCANNED_REPEAT);
    assert(r.nshows == 5 && r.shows[0].kind == VRR_SHOW_NEW);
    for (size_t i = 1; i < 5; i++) assert(r.shows[i].kind == VRR_SHOW_REPEAT && same(r.shows[i].cookie, a) && r.shows[i].handle == r.shows[0].handle);
    assert(vrr_get_stats(r.p)->shown == 1 && vrr_get_stats(r.p)->scans_repeat == 4);

    rig_churn(&r, 12);                                                  /* repeats go on while the decoder cycles its pool */
    assert(vrr_scan(r.p, 6 * MS) == VRR_SCANNED_REPEAT);
    assert(vrr_retire(r.p, a, 6 * MS) == VRR_E_PREMATURE);              /* on screen: protected from an early cookie */
    assert(r.nrets == 0 && st(&r, a) == VRR_ST_CURRENT);

    vrr_cookie b = rig_submit(&r, 6 * MS);
    assert(vrr_scan(r.p, 7 * MS) == VRR_SCANNED_NEW && st(&r, a) == VRR_ST_SUPERSEDED);
    assert(vrr_retire(r.p, a, 8 * MS) == VRR_OK);
    assert(r.nrets == 1 && r.rets[0].scans == 6 && r.rets[0].first_scan_ns == 1 * MS && r.rets[0].last_scan_ns == 6 * MS);  /* one surface, six scans, one retirement */
    assert(vrr_retire(r.p, a, 8 * MS) == VRR_E_STALE_COOKIE && r.nrets == 1);

    rig_finish(&r, 9 * MS, &b, 1);
    rig_close(&r);
    printf("repeat_holds_one_surface ok\n");
}

/* ---- duplicates -------------------------------------------------------------------------------------------------------------- */

static void test_duplicates(void) {
    rig r;
    rig_open(&r, 6);
    vrr_picture pa, pb, pd;
    hevc_surface *sa = rig_pull(&r, &pa, 0), *sb = rig_pull(&r, &pb, 0);
    vrr_cookie a;
    assert(vrr_submit(r.p, &pa, &a) == VRR_OK);

    vrr_picture again = pa;                                             /* a producer that sends the same picture twice */
    assert(vrr_submit(r.p, &again, NULL) == VRR_E_DUPLICATE);
    vrr_picture clash = pb;                                             /* another surface under an id the presenter holds */
    clash.id = pa.id;
    assert(vrr_submit(r.p, &clash, NULL) == VRR_E_DUPLICATE);
    vrr_picture alias = pa;                                             /* the held surface under a new id */
    alias.id = pb.id;
    alias.pts = pb.pts;
    assert(vrr_submit(r.p, &alias, NULL) == VRR_E_DUPLICATE);
    vrr_picture twin = pb;                                              /* another surface and id at the presentation time of a queued picture */
    twin.pts = pa.pts;
    assert(vrr_submit(r.p, &twin, NULL) == VRR_E_DUPLICATE);
    assert(vrr_owned(r.p) == 1 && vrr_get_stats(r.p)->rejected[-VRR_E_DUPLICATE] == 4 && vrr_get_stats(r.p)->submitted == 1);
    assert(hevc_surface_planes_freed(sa) == 0 && hevc_surface_read(r.src, sa) == 0);   /* the held surface is untouched */
    hevc_surface_retire(r.src, sb);                                     /* refused: B never left its caller */

    vrr_cookie c = rig_submit(&r, 2 * MS);
    assert(vrr_scan(r.p, 3 * MS) == VRR_SCANNED_NEW);                   /* shows C, drops A */
    assert(st(&r, a) == VRR_ST_DROPPED);
    vrr_picture dup_dropped = pa;                                       /* a dropped picture still owns its id until it retires */
    assert(vrr_submit(r.p, &dup_dropped, NULL) == VRR_E_DUPLICATE);
    assert(vrr_retire(r.p, a, 4 * MS) == VRR_OK);

    rig_pull(&r, &pd, 5 * MS);                                          /* no memory of a retired id: it may come round again */
    pd.id = pa.id;
    vrr_cookie d;
    assert(vrr_submit(r.p, &pd, &d) == VRR_OK);
    vrr_cookie left[2] = {c, d};
    rig_finish(&r, 6 * MS, left, 2);
    rig_close(&r);
    printf("duplicates ok\n");
}

/* ---- epochs ------------------------------------------------------------------------------------------------------------------ */

static void test_stale_and_future_epoch(void) {
    rig r;
    rig_open(&r, 6);
    rig_submit(&r, 0);
    vrr_cookie b = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* B on screen, A dropped */
    assert(vrr_flush(r.p, 1, 2 * MS) == VRR_OK && vrr_epoch(r.p) == 1);

    vrr_picture stale, future;
    hevc_surface *ss = rig_pull(&r, &stale, 3 * MS), *sf = rig_pull(&r, &future, 3 * MS);
    stale.epoch = 0;
    future.epoch = 2;
    assert(vrr_submit(r.p, &stale, NULL) == VRR_E_STALE_EPOCH);
    assert(vrr_submit(r.p, &future, NULL) == VRR_E_FUTURE_EPOCH);
    hevc_surface_retire(r.src, ss);                                     /* both refused: still their caller's */
    hevc_surface_retire(r.src, sf);
    assert(vrr_owned(r.p) == 2);

    assert(vrr_scan(r.p, 3 * MS) == VRR_SCANNED_STALE && st(&r, b) == VRR_ST_CURRENT);   /* the old picture stays held until a new epoch replaces it */
    vrr_cookie c = rig_submit(&r, 3 * MS);
    assert(vrr_scan(r.p, 4 * MS) == VRR_SCANNED_NEW && st(&r, b) == VRR_ST_SUPERSEDED);
    assert(vrr_retire(r.p, b, 5 * MS) == VRR_OK && r.rets[r.nrets - 1].epoch == 0);   /* retirement still works across the flush */
    assert(vrr_flush(r.p, 1, 6 * MS) == VRR_E_STALE_EPOCH && vrr_flush(r.p, 0, 6 * MS) == VRR_E_STALE_EPOCH);

    vrr_cookie dropped = r.drops[0].cookie;                             /* A, dropped at the first scan, was never returned */
    vrr_cookie left[2] = {dropped, c};
    rig_finish(&r, 7 * MS, left, 2);
    rig_close(&r);
    printf("stale_and_future_epoch ok\n");
}

/* ---- full queue -------------------------------------------------------------------------------------------------------------- */

static void test_full_queue(void) {
    rig r;
    rig_open(&r, 3);
    vrr_cookie a = rig_submit(&r, 0), b = rig_submit(&r, 0), c = rig_submit(&r, 0);
    vrr_picture pd;
    hevc_surface *sd = rig_pull(&r, &pd, 0);
    assert(vrr_submit(r.p, &pd, NULL) == VRR_E_FULL);                   /* refused: D stays with its caller, intact */
    assert(hevc_surface_planes_freed(sd) == 0 && hevc_surface_read(r.src, sd) == 0 && vrr_owned(r.p) == 3);

    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* C shown, A and B dropped, none returned yet */
    assert(vrr_submit(r.p, &pd, NULL) == VRR_E_FULL && vrr_owned(r.p) == 3);   /* awaiting retirement still takes room */
    assert(vrr_retire(r.p, a, 2 * MS) == VRR_OK && vrr_owned(r.p) == 2);
    vrr_cookie d;
    assert(vrr_submit(r.p, &pd, &d) == VRR_OK);                         /* the same picture goes in once there is room */
    assert(vrr_owned(r.p) == 3 && vrr_get_stats(r.p)->peak_owned == 3 && vrr_get_stats(r.p)->rejected[-VRR_E_FULL] == 2);
    assert(vrr_scan(r.p, 3 * MS) == VRR_SCANNED_NEW && same(r.shows[r.nshows - 1].cookie, d));   /* D was kept, not lost */

    vrr_cookie left[3] = {b, c, d};
    rig_finish(&r, 4 * MS, left, 3);
    rig_close(&r);
    printf("full_queue ok\n");
}

/* ---- flush ------------------------------------------------------------------------------------------------------------------- */

static void test_flush(void) {
    rig r;
    rig_open(&r, 6);
    vrr_cookie q[3];
    for (int i = 0; i < 3; i++) q[i] = rig_submit(&r, 10 * MS);        /* not ready: still queued */
    assert(vrr_flush(r.p, 1, 2 * MS) == VRR_OK);
    assert(r.ndrops == 3 && vrr_owned(r.p) == 3 && r.nrets == 0);       /* dropped, but their surfaces are not released until the cookies return */
    for (size_t i = 0; i < 3; i++) assert(same(r.drops[i].cookie, q[i]) && r.drops[i].reason == VRR_DROP_FLUSH && st(&r, q[i]) == VRR_ST_DROPPED);
    assert(vrr_get_stats(r.p)->dropped[VRR_DROP_FLUSH] == 3);

    vrr_picture n1, n2;
    rig_pull(&r, &n1, 3 * MS);
    n1.pts = 0;                                                         /* a new epoch starts its own clock: earlier timestamps are fine */
    n1.tb = (vrr_timebase){1, 1000};
    vrr_cookie c1;
    assert(vrr_submit(r.p, &n1, &c1) == VRR_OK);
    rig_pull(&r, &n2, 3 * MS);
    n2.pts = 1;
    assert(vrr_submit(r.p, &n2, NULL) == VRR_E_TIMEBASE);               /* and its first picture fixes the timebase */
    hevc_surface_retire(r.src, n2.handle);
    for (size_t i = 0; i < 3; i++) {
        assert(vrr_retire(r.p, q[i], 4 * MS) == VRR_OK);
        assert(r.rets[r.nrets - 1].outcome == VRR_DROPPED && r.rets[r.nrets - 1].reason == VRR_DROP_FLUSH && r.rets[r.nrets - 1].epoch == 0);
    }
    assert(vrr_owned(r.p) == 1);
    rig_finish(&r, 5 * MS, &c1, 1);
    rig_close(&r);
    printf("flush ok\n");
}

/* ---- a flush does not touch the surface being scanned ------------------------------------------------------------------------ */

static void test_flush_holds_the_scanned_surface(void) {
    rig r;
    rig_open(&r, 8);
    rig_submit(&r, 0);
    vrr_cookie b = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* B on screen, A dropped */
    hevc_surface *sb = r.shows[0].handle;
    vrr_cookie q1 = rig_submit(&r, 5 * MS), q2 = rig_submit(&r, 5 * MS);   /* old-epoch work, not ready yet */
    size_t shows = r.nshows;
    assert(vrr_flush(r.p, 1, 2 * MS) == VRR_OK);
    assert(st(&r, q1) == VRR_ST_DROPPED && st(&r, q2) == VRR_ST_DROPPED && r.drops[r.ndrops - 1].reason == VRR_DROP_FLUSH);
    assert(st(&r, b) == VRR_ST_CURRENT && vrr_owned(r.p) == 4);         /* the queued work is invalid; the picture being scanned is not queued work */

    for (int i = 3; i <= 6; i++) assert(vrr_scan(r.p, i * MS) == VRR_SCANNED_STALE);   /* nothing new to show: B is neither repeated nor selected again */
    assert(r.nshows == shows && vrr_get_stats(r.p)->scans_stale == 4 && vrr_get_stats(r.p)->scans_repeat == 0);
    assert(vrr_retire(r.p, b, 7 * MS) == VRR_E_PREMATURE && r.nrets == 0);   /* and no cookie can free what may still be scanned */
    rig_churn(&r, 20);
    assert(hevc_surface_planes_freed(sb) == 0 && hevc_surface_read(r.src, sb) == 0);

    vrr_cookie c = rig_submit(&r, 8 * MS);                              /* the new epoch's first picture is the verified replacement */
    assert(vrr_scan(r.p, 9 * MS) == VRR_SCANNED_NEW && same(r.shows[r.nshows - 1].supersedes, b) && st(&r, b) == VRR_ST_SUPERSEDED);
    assert(r.nrets == 0 && hevc_surface_planes_freed(sb) == 0);         /* replaced is not retired: its reads end when the backend says so */
    assert(vrr_retire(r.p, b, 10 * MS) == VRR_OK);
    assert(r.rets[0].outcome == VRR_DISPLAYED && r.rets[0].epoch == 0 && r.rets[0].scans == 1);
    for (size_t i = shows; i < r.nshows; i++) assert(r.shows[i].epoch == 1);   /* no show after the flush names an old-epoch picture */

    vrr_cookie left[4] = {r.drops[0].cookie, q1, q2, c};                /* A, dropped at the first scan, was never returned */
    rig_finish(&r, 11 * MS, left, 4);
    rig_close(&r);

    rig h;                                                              /* the handback also ends the hold */
    rig_open(&h, 4);
    vrr_cookie x = rig_submit(&h, 0);
    assert(vrr_scan(h.p, 1 * MS) == VRR_SCANNED_NEW && vrr_flush(h.p, 1, 2 * MS) == VRR_OK);
    assert(vrr_scan(h.p, 3 * MS) == VRR_SCANNED_STALE && st(&h, x) == VRR_ST_CURRENT);
    assert(vrr_shutdown(h.p, 4 * MS) == 1 && st(&h, x) == VRR_ST_SUPERSEDED);
    assert(vrr_retire(h.p, x, 5 * MS) == VRR_OK);
    assert(h.rets[0].outcome == VRR_DISPLAYED && h.rets[0].reason == VRR_DROP_SHUTDOWN && h.rets[0].epoch == 0 && h.rets[0].decided_ns == 4 * MS);
    rig_close(&h);
    printf("flush_holds_the_scanned_surface ok\n");
}

/* ---- selection follows presentation order ------------------------------------------------------------------------------------ */

/* The decoder calls back in the order of a hierarchical GOP, 0 4 2 1 3 | 8 6 5 7 | 11 9 10, each picture ready at its own instant, with scans in
 * between. The picture shown is the greatest pts that is ready, whatever came back last; one that arrives behind the picture on screen is dropped. */
static void test_selection_follows_presentation_order(void) {
    enum { N = 12, SUBMIT = 0, SCAN = 1 };
    static const struct { int op, arg; int64_t ms; } script[] = {      /* submit: picture, ready time; scan: expected result, start */
        {SUBMIT, 0, 0},  {SCAN, VRR_SCANNED_NEW, 1},
        {SUBMIT, 4, 2},  {SUBMIT, 2, 3},  {SCAN, VRR_SCANNED_NEW, 5},            /* 4 is shown though 2 came back last; 2 never is */
        {SUBMIT, 1, 6},  {SUBMIT, 3, 7},  {SCAN, VRR_SCANNED_REPEAT, 9},         /* both are behind the picture on screen: dropped on arrival */
        {SUBMIT, 8, 30}, {SUBMIT, 6, 10}, {SCAN, VRR_SCANNED_NEW, 11},           /* 8 is not ready and waits; 6 is shown */
        {SUBMIT, 5, 12}, {SUBMIT, 7, 13}, {SCAN, VRR_SCANNED_NEW, 14},           /* 5 is behind 6; 7 is shown */
        {SUBMIT, 11, 15}, {SUBMIT, 9, 16}, {SUBMIT, 10, 17}, {SCAN, VRR_SCANNED_NEW, 20},   /* 11 is shown; 9, 10 and the unready 8 are behind it */
        {SCAN, VRR_SCANNED_REPEAT, 40},                                           /* nothing ready: 11 repeats */
    };
    static const struct { vrr_outcome outcome; vrr_drop_reason reason; unsigned scans; } want[N] = {
        {VRR_DISPLAYED, VRR_DROP_SUPERSEDED, 1}, {VRR_DROPPED, VRR_DROP_LATE, 0},         {VRR_DROPPED, VRR_DROP_SUPERSEDED, 0},
        {VRR_DROPPED, VRR_DROP_LATE, 0},         {VRR_DISPLAYED, VRR_DROP_SUPERSEDED, 2}, {VRR_DROPPED, VRR_DROP_LATE, 0},
        {VRR_DISPLAYED, VRR_DROP_SUPERSEDED, 1}, {VRR_DISPLAYED, VRR_DROP_SUPERSEDED, 1}, {VRR_DROPPED, VRR_DROP_SUPERSEDED, 0},
        {VRR_DROPPED, VRR_DROP_SUPERSEDED, 0},   {VRR_DROPPED, VRR_DROP_SUPERSEDED, 0},   {VRR_DISPLAYED, VRR_DROP_SHUTDOWN, 2},
    };
    static const int shown_order[5] = {0, 4, 6, 7, 11};
    rig r;
    rig_open(&r, 16);
    vrr_picture pic[N];
    vrr_cookie ck[N];
    for (int k = 0; k < N; k++) rig_pull(&r, &pic[k], 0);               /* display order: pts 750 k */
    for (size_t i = 0; i < sizeof script / sizeof *script; i++) {
        if (script[i].op == SUBMIT) {
            pic[script[i].arg].ready_ns = script[i].ms * MS;
            assert(vrr_submit(r.p, &pic[script[i].arg], &ck[script[i].arg]) == VRR_OK);
        } else {
            assert(vrr_scan(r.p, script[i].ms * MS) == script[i].arg);
        }
    }
    const vrr_stats *s = vrr_get_stats(r.p);
    assert(s->shown == 5 && s->scans_new == 5 && s->scans_repeat == 2 && s->scans_underrun == 0 && s->scans_stale == 0);
    assert(s->dropped[VRR_DROP_SUPERSEDED] == 4 && s->dropped[VRR_DROP_LATE] == 3 && r.nshows == 7 && r.nrets == 0);
    size_t news = 0;
    int64_t last_pts = -1;
    for (size_t i = 0; i < r.nshows; i++) {                             /* the pictures shown, in order, rise in pts */
        if (r.shows[i].kind != VRR_SHOW_NEW) continue;
        assert(news < 5 && r.shows[i].id == pic[shown_order[news++]].id && r.shows[i].pts > last_pts);
        last_pts = r.shows[i].pts;
    }
    assert(news == 5);
    assert(hevc_surface_planes_freed(pic[1].handle) == 0 && hevc_surface_read(r.src, pic[1].handle) == 0);   /* a dropped picture stays loaned until its cookie returns */

    assert(vrr_shutdown(r.p, 41 * MS) == N);
    for (int k = N - 1; k >= 0; k--) assert(vrr_retire(r.p, ck[k], 42 * MS) == VRR_OK);    /* the backend may return them in any order */
    assert(r.nrets == N);
    for (int k = 0; k < N; k++) {
        const vrr_retirement *rt = NULL;
        for (size_t i = 0; i < r.nrets; i++)
            if (r.rets[i].id == pic[k].id) rt = &r.rets[i];
        assert(rt && rt->outcome == want[k].outcome && rt->reason == want[k].reason && rt->scans == want[k].scans);
        if (rt->reason == VRR_DROP_LATE) assert(rt->decided_ns == rt->ready_ns);       /* dropped the instant it arrived */
    }
    rig_close(&r);
    printf("selection_follows_presentation_order ok\n");
}

/* ---- shutdown ---------------------------------------------------------------------------------------------------------------- */

static void test_shutdown(void) {
    rig r;
    rig_open(&r, 8);
    vrr_cookie a = rig_submit(&r, 0), b = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* B on screen, A dropped */
    vrr_cookie c = rig_submit(&r, 2 * MS);
    assert(vrr_scan(r.p, 3 * MS) == VRR_SCANNED_NEW);                   /* C on screen, B displaced */
    vrr_cookie d = rig_submit(&r, 100 * MS), e = rig_submit(&r, 100 * MS);
    assert(st(&r, a) == VRR_ST_DROPPED && st(&r, b) == VRR_ST_SUPERSEDED && st(&r, c) == VRR_ST_CURRENT);
    assert(st(&r, d) == VRR_ST_QUEUED && st(&r, e) == VRR_ST_QUEUED && vrr_owned(r.p) == 5);

    assert(vrr_shutdown(r.p, 4 * MS) == 5);                             /* nothing is released by shutting down: five cookies are out */
    assert(r.nrets == 0 && vrr_get_stats(r.p)->dropped[VRR_DROP_SHUTDOWN] == 2);
    assert(st(&r, d) == VRR_ST_DROPPED && st(&r, c) == VRR_ST_SUPERSEDED);   /* the picture on screen will not be repeated again */
    vrr_picture late;
    hevc_surface *sl = rig_pull(&r, &late, 5 * MS);
    assert(vrr_submit(r.p, &late, NULL) == VRR_E_CLOSED && vrr_scan(r.p, 6 * MS) == VRR_E_CLOSED && vrr_flush(r.p, 1, 6 * MS) == VRR_E_CLOSED);
    hevc_surface_retire(r.src, sl);

    vrr_cookie order[5] = {c, e, a, d, b};
    for (int i = 0; i < 5; i++) {
        assert(vrr_retire(r.p, order[i], 7 * MS) == VRR_OK);
        assert(vrr_owned(r.p) == (unsigned)(4 - i));
    }
    assert(r.nrets == 5 && vrr_shutdown(r.p, 8 * MS) == 0);
    assert(vrr_retire(r.p, c, 8 * MS) == VRR_E_STALE_COOKIE);
    rig_close(&r);
    printf("shutdown ok\n");
}

/* ---- retirement after a drop, and cookies that must change nothing ----------------------------------------------------------- */

static void test_retire_after_drop_and_bad_cookies(void) {
    rig r;
    rig_open(&r, 2);
    vrr_cookie a = rig_submit(&r, 0), b = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* B shown, A dropped */
    assert(vrr_retire(r.p, a, 2 * MS) == VRR_OK && r.nrets == 1);
    assert(vrr_retire(r.p, a, 3 * MS) == VRR_E_STALE_COOKIE && r.nrets == 1);   /* retired after the drop: a second return releases nothing */

    vrr_picture pc;
    hevc_surface *sc = rig_pull(&r, &pc, 10 * MS);                      /* the new picture takes the slot A had */
    vrr_cookie c;
    assert(vrr_submit(r.p, &pc, &c) == VRR_OK && c.slot == a.slot && c.gen != a.gen);
    assert(vrr_retire(r.p, a, 4 * MS) == VRR_E_STALE_COOKIE);           /* a late cookie of the earlier tenant */
    assert(r.nrets == 1 && hevc_surface_planes_freed(sc) == 0 && hevc_surface_read(r.src, sc) == 0 && st(&r, c) == VRR_ST_QUEUED);

    assert(vrr_retire(r.p, c, 4 * MS) == VRR_E_PREMATURE);              /* queued: the backend cannot have it yet */
    assert(vrr_retire(r.p, b, 4 * MS) == VRR_E_PREMATURE);              /* on screen */
    vrr_cookie forged[3] = {{0, 0}, {99, 1}, {b.slot, b.gen + 7}};
    for (int i = 0; i < 3; i++) assert(vrr_retire(r.p, forged[i], 4 * MS) == VRR_E_STALE_COOKIE);
    assert(r.nrets == 1 && vrr_owned(r.p) == 2 && st(&r, b) == VRR_ST_CURRENT && st(&r, c) == VRR_ST_QUEUED);
    assert(vrr_get_stats(r.p)->retired_dropped == 1 && vrr_get_stats(r.p)->retired_displayed == 0);

    vrr_cookie left[2] = {b, c};
    rig_finish(&r, 11 * MS, left, 2);
    rig_close(&r);
    printf("retire_after_drop_and_bad_cookies ok\n");
}

/* ---- a cookie that never comes back ------------------------------------------------------------------------------------------ */

static void test_lost_cookie_is_a_reported_leak(void) {
    rig r;
    rig_open(&r, 4);
    vrr_cookie a = rig_submit(&r, 0);
    vrr_cookie b = rig_submit(&r, 0);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW);                   /* B shown, A dropped: the backend loses A's cookie */
    vrr_cookie c = rig_submit(&r, 2 * MS);
    assert(vrr_scan(r.p, 3 * MS) == VRR_SCANNED_NEW && vrr_retire(r.p, b, 4 * MS) == VRR_OK);

    assert(vrr_shutdown(r.p, 5 * MS) == 2);                             /* A and C */
    assert(vrr_retire(r.p, c, 5 * MS) == VRR_OK && vrr_owned(r.p) == 1 && st(&r, a) == VRR_ST_DROPPED);
    unsigned leaked = vrr_free(r.p);
    assert(leaked == 1);                                                /* reported, and A's surface was not released on a guess */
    const hevc_oracle *o = hevc_source_oracle(r.src);
    assert(o->live == 1 && r.nrets == 2);
    hevc_oracle final;
    assert(hevc_source_close(r.src, &final) == 3);                      /* the decoder's own counts: A's three plane buffers are still out */
    assert(final.live == 1 && final.pictures - final.retired == 1);
    printf("lost_cookie_is_a_reported_leak ok\n");
}

/* ---- the oracle must catch a faulty owner ------------------------------------------------------------------------------------ */

static void test_oracle_catches_early_release(void) {
    rig r;
    rig_open(&r, 40);
    r.early_release = 1;
    hevc_surface *surf[30];
    for (int i = 0; i < 30; i++) {
        vrr_picture pic;
        surf[i] = rig_pull(&r, &pic, 0);
        assert(vrr_submit(r.p, &pic, NULL) == VRR_OK);
    }
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_NEW && r.ndrops == 29);   /* the faulty owner frees all 29 at the drop command */
    rig_churn(&r, 10);                                                  /* the decoder reuses what it was given back */
    unsigned unreadable = 0;
    for (int i = 0; i < 29; i++)
        if (hevc_surface_read(r.src, surf[i]) != 0) unreadable++;
    const hevc_oracle *o = hevc_source_oracle(r.src);
    assert(o->recycled_live > 0);                                       /* memory handed out while a cookie for it was outstanding */
    assert(o->freed_while_live > 0);                                    /* a plane buffer reached count 0 while its surface was live */
    assert(o->read_after_free == 29 && unreadable > 0 && o->integrity_failures == unreadable);

    for (size_t i = 0; i < r.ndrops; i++) {
        int rc = vrr_retire(r.p, r.drops[i].cookie, 2 * MS);
        assert(rc == VRR_OK);
    }
    assert(vrr_shutdown(r.p, 3 * MS) == 1);
    assert(vrr_retire(r.p, r.shows[0].cookie, 3 * MS) == VRR_OK);
    unsigned leaked = vrr_free(r.p);
    assert(leaked == 0 && hevc_source_close(r.src, NULL) == 0);
    printf("oracle_catches_early_release ok (%u of 29 early-freed surfaces corrupted or recycled)\n", unreadable);
}

/* ---- the wire ACK is not a surface ------------------------------------------------------------------------------------------- */

static void test_ack_is_not_ownership(void) {
    rig r;
    rig_open(&r, 4);
    /* What the compressed-input ACK says about an access unit: its id and timestamp, and nothing to own. */
    vrr_picture ack_only = {0, 7, 750, {1, 90000}, 0, NULL};
    assert(vrr_submit(r.p, &ack_only, NULL) == VRR_E_NO_HANDLE);
    assert(vrr_owned(r.p) == 0 && vrr_get_stats(r.p)->submitted == 0 && vrr_get_stats(r.p)->rejected[-VRR_E_NO_HANDLE] == 1);
    assert(vrr_scan(r.p, 1 * MS) == VRR_SCANNED_UNDERRUN);              /* nothing to show for an ACK alone */
    rig_close(&r);
    printf("ack_is_not_ownership ok\n");
}

/* ---- deadline traces --------------------------------------------------------------------------------------------------------- */

/* The backend's cookie lag is unknown, so the traces run with none and with a long, jittered one to show that no decision depends on it. Capacity is a
 * backpressure bound and no reuse rule: 64 is more than the matrix can fill, and STARVED fills three on purpose. */
static const vrr_sim_cfg PROMPT = {.decode_ns = 2 * MS, .delay_ns = 4 * MS, .lag_ns = 0, .jitter_ns = 0, .capacity = 64};
static const vrr_sim_cfg LATE = {.decode_ns = 2 * MS, .delay_ns = 4 * MS, .lag_ns = 10 * MS, .jitter_ns = 40 * MS, .capacity = 64};
static const vrr_sim_cfg STARVED = {.decode_ns = 2 * MS, .delay_ns = 4 * MS, .lag_ns = 10 * MS, .jitter_ns = 40 * MS, .capacity = 3};

typedef struct { hevc_source *src; } cellctx;

static int cell_make(void *u, size_t k, int64_t pts, int64_t ready_ns, vrr_picture *pic) {
    hevc_surface *s;
    if (hevc_source_next(((cellctx *)u)->src, &s) != 0 || hevc_surface_pts(s) != pts || hevc_surface_display(s) != k) return -1;
    *pic = (vrr_picture){0, hevc_surface_id(s), hevc_surface_pts(s), {1, 90000}, ready_ns, s};
    return 0;
}
static void cell_refused(void *u, const vrr_picture *pic, int err) {
    (void)err;
    hevc_surface_retire(((cellctx *)u)->src, pic->handle);
}
static void cell_release(void *u, void *handle, const vrr_retirement *rt) {
    (void)rt;
    hevc_surface_retire(((cellctx *)u)->src, handle);
}
static void cell_shown(void *u, const vrr_show *cmd) { hevc_surface_read(((cellctx *)u)->src, cmd->handle); }

typedef struct {
    const vrr_sim_cfg *cfg;
    const vrr_trace *scans, *source;
    vrr_sim_result res;
    hevc_oracle o;
    double held_p50, held_max;                          /* ms, ready until a newer picture's scan or a drop decided its end (handback excluded) */
    double ierr_mean, ierr_max;                         /* ms, change between first scans against change between source times, consecutive displayed pictures */
    uint64_t dropped;
} cell;

static int cmp_i64(const void *a, const void *b) {
    int64_t x = *(const int64_t *)a, y = *(const int64_t *)b;
    return (x > y) - (x < y);
}

static int cmp_first_scan(const void *a, const void *b) {
    int64_t x = (*(const vrr_retirement *const *)a)->first_scan_ns, y = (*(const vrr_retirement *const *)b)->first_scan_ns;
    return (x > y) - (x < y);
}

/* The displayed pictures in the order they were first shown. The log is in the order cookies came back, which late cookies scramble. */
static size_t displayed(const cell *c, const vrr_retirement **out) {
    size_t n = 0;
    for (size_t i = 0; i < c->res.nlog; i++)
        if (c->res.log[i].outcome == VRR_DISPLAYED) out[n++] = &c->res.log[i];
    qsort(out, n, sizeof *out, cmp_first_scan);
    return n;
}

static void summarise(cell *c) {
    int64_t held[VRR_TRACE_MAX];
    size_t n = 0;
    for (size_t i = 0; i < c->res.nlog; i++) {
        const vrr_retirement *r = &c->res.log[i];
        if (r->reason != VRR_DROP_SHUTDOWN) held[n++] = r->decided_ns - r->ready_ns;      /* the handback is not a deadline decision */
    }
    assert(n > 0);
    qsort(held, n, sizeof *held, cmp_i64);
    c->held_p50 = (double)held[n / 2] / 1e6;
    c->held_max = (double)held[n - 1] / 1e6;

    const vrr_retirement *shown[VRR_TRACE_MAX];
    size_t m = displayed(c, shown);
    double sum = 0, worst = 0;
    for (size_t i = 1; i < m; i++) {
        double e = fabs((double)((shown[i]->first_scan_ns - shown[i - 1]->first_scan_ns) - (vrr_ticks_to_ns(shown[i]->pts) - vrr_ticks_to_ns(shown[i - 1]->pts)))) / 1e6;
        sum += e;
        if (e > worst) worst = e;
    }
    c->ierr_mean = m > 1 ? sum / (double)(m - 1) : 0;
    c->ierr_max = worst;
    for (int k = 0; k < VRR_DROP_COUNT; k++) c->dropped += c->res.stats.dropped[k];
}

static void run_cell(const vrr_sim_cfg *cfg, const vrr_trace *scans, const vrr_trace *source, cell *c) {
    memset(c, 0, sizeof *c);
    c->cfg = cfg;
    c->scans = scans;
    c->source = source;
    char err[256];
    cellctx ctx;
    ctx.src = hevc_source_open(stream_path, order_path, source->ticks, source->n, err, sizeof err);
    if (!ctx.src) {
        fprintf(stderr, "open: %s\n", err);
        exit(1);
    }
    vrr_sim_hooks hooks = {&ctx, cell_make, cell_refused, cell_release, cell_shown, NULL};
    int rc = vrr_sim_run(cfg, source, scans, &hooks, &c->res);
    assert(rc == 0);
    int64_t outstanding = hevc_source_close(ctx.src, &c->o);
    assert(outstanding == 0);
    summarise(c);
}

static void check_cell(const cell *c) {
    const vrr_sim_result *res = &c->res;
    const vrr_stats *s = &res->stats;
    assert(res->retire_errors == 0 && res->leaked == 0);
    assert(res->pictures == c->source->n && s->submitted + res->refused == res->pictures && res->nlog == s->submitted);
    assert(s->shown + c->dropped == s->submitted);                      /* every picture is shown or dropped, once */
    assert(s->retired_displayed == s->shown && s->retired_dropped == c->dropped);     /* and every one is retired, once */
    assert(s->scans == c->scans->n && s->scans == s->scans_new + s->scans_repeat + s->scans_stale + s->scans_underrun && s->scans_new == s->shown);
    assert(s->peak_owned >= 1 && s->peak_owned <= c->cfg->capacity);
    oracle_clean(&c->o);
    assert(c->o.outstanding == 0 && c->o.live == 0 && c->o.pictures == res->pictures && c->o.pictures == c->o.retired);
    assert(c->o.allocated == c->o.freed && c->o.handouts > c->o.distinct);
    int64_t last_retire = 0;
    for (size_t i = 0; i < res->nlog; i++) {
        const vrr_retirement *r = &res->log[i];
        assert(r->ready_ns <= r->decided_ns && r->decided_ns <= r->retire_ns && r->retire_ns >= last_retire);
        if (r->outcome == VRR_DISPLAYED) assert(r->ready_ns <= r->first_scan_ns && r->first_scan_ns <= r->last_scan_ns && r->last_scan_ns <= r->decided_ns && r->scans >= 1);
        else assert(r->scans == 0 && r->first_scan_ns == -1);
        last_retire = r->retire_ns;
    }
}

/* Intervals, in ns, between the first scans of consecutive displayed pictures. */
static size_t visible_intervals(const cell *c, int64_t *out) {
    const vrr_retirement *shown[VRR_TRACE_MAX];
    size_t m = displayed(c, shown), n = 0;
    for (size_t i = 1; i < m; i++) out[n++] = shown[i]->first_scan_ns - shown[i - 1]->first_scan_ns;
    return n;
}

static void print_report(const cell cells[3][3][2]) {
    printf("\ndeadline traces x source cadences (decode %lld ms, first scan +%lld ms, capacity %u; cookies back at once, or %lld ms + 0-%lld ms later and out of order)\n",
           (long long)(PROMPT.decode_ns / MS), (long long)(PROMPT.delay_ns / MS), PROMPT.capacity, (long long)(LATE.lag_ns / MS), (long long)(LATE.jitter_ns / MS));
    printf("%-18s %-18s %4s %5s %8s %11s %-16s %-24s %s\n", "scans", "source", "pics", "shown", "dropped", "new/rep/und", "held ms p50/max", "interval err ms mean/max",
           "peak owned none/late");
    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++) {
            const cell *c = &cells[i][j][0], *late = &cells[i][j][1];
            const vrr_stats *s = &c->res.stats;
            char sr[24];
            snprintf(sr, sizeof sr, "%llu/%llu/%llu", (unsigned long long)s->scans_new, (unsigned long long)s->scans_repeat, (unsigned long long)s->scans_underrun);
            printf("%-18s %-18s %4zu %5llu %8llu %11s %7.2f/%6.2f %15.2f/%5.2f %14u/%u\n", c->scans->name, c->source->name, c->res.pictures,
                   (unsigned long long)s->shown, (unsigned long long)c->dropped, sr, c->held_p50, c->held_max, c->ierr_mean, c->ierr_max, s->peak_owned,
                   late->res.stats.peak_owned);
        }
    uint64_t handouts = 0, distinct = 0, pictures = 0;
    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++)
            for (int k = 0; k < 2; k++) {
                handouts += cells[i][j][k].o.handouts;
                distinct += cells[i][j][k].o.distinct;
                pictures += cells[i][j][k].o.pictures;
            }
    printf("decoder: %llu pictures decoded over 18 runs, %llu plane-0 buffers handed out over %llu distinct addresses; recycled while live 0, freed while live 0, bad reads 0, outstanding 0\n",
           (unsigned long long)pictures, (unsigned long long)handouts, (unsigned long long)distinct);
}

/* Pictures arrive faster than late cookies come back: the presenter refuses at its bound and recycles nothing. */
static void test_backpressure_never_recycles(void) {
    static vrr_trace t;
    static cell c;
    vrr_trace_fixed_120(&t);
    run_cell(&STARVED, &t, &t, &c);
    check_cell(&c);                                                     /* includes the decoder's own counts: no live surface was recycled */
    const vrr_sim_result *res = &c.res;
    const vrr_stats *s = &res->stats;
    assert(res->refused > 0 && s->peak_owned == STARVED.capacity);
    assert(s->rejected[-VRR_E_FULL] == res->refused && s->submitted + res->refused == res->pictures);
    printf("backpressure_never_recycles ok (capacity %u, cookies %lld-%lld ms late: %zu of %zu pictures refused, %llu shown)\n", STARVED.capacity,
           (long long)(STARVED.lag_ns / MS), (long long)((STARVED.lag_ns + STARVED.jitter_ns) / MS), res->refused, res->pictures, (unsigned long long)s->shown);
}

static void test_trace_matrix(int report) {
    static vrr_trace scans[3], source[3];
    vrr_trace_fixed_120(&scans[0]);
    vrr_trace_shuffled(&scans[1]);
    vrr_trace_fixed_13(&scans[2]);
    source[0] = scans[0];
    source[1] = scans[1];
    source[2] = scans[2];
    assert(scans[0].n == 76 && scans[1].n == 49 && scans[2].n == 49);
    assert(scans[1].ticks[48] == 624 * 90 && scans[2].ticks[48] == 624 * 90);
    const vrr_sim_cfg *const backend[2] = {&PROMPT, &LATE};
    static cell cells[3][3][2];                                         /* scan trace, source, cookies at once or late */
    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++)
            for (int k = 0; k < 2; k++) {
                run_cell(backend[k], &scans[i], &source[j], &cells[i][j][k]);
                check_cell(&cells[i][j][k]);
                assert(cells[i][j][k].res.refused == 0);
            }

    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++) {                                   /* a late, out-of-order cookie changes no decision, only how long surfaces stay owned */
            const cell *none = &cells[i][j][0], *late = &cells[i][j][1];
            assert(none->res.stats.shown == late->res.stats.shown && none->dropped == late->dropped);
            assert(none->res.stats.scans_repeat == late->res.stats.scans_repeat && none->res.stats.scans_underrun == late->res.stats.scans_underrun);
            assert(none->held_p50 == late->held_p50 && none->held_max == late->held_max && none->ierr_mean == late->ierr_mean && none->ierr_max == late->ierr_max);
            assert(none->res.stats.peak_owned < late->res.stats.peak_owned);
        }

    for (int i = 0; i < 3; i++) {                                       /* a trace driven by its own cadence needs no drop, repeat or underrun */
        const vrr_stats *s = &cells[i][i][0].res.stats;
        assert(cells[i][i][0].dropped == 0 && s->scans_repeat == 0 && s->scans_underrun == 0 && s->shown == scans[i].n);
        assert(cells[i][i][0].ierr_mean == 0 && cells[i][i][0].ierr_max == 0);
    }
    const vrr_stats *s = &cells[0][2][0].res.stats;                     /* 120 Hz scans, 13 ms pictures: a repeat for every scan without a new picture */
    assert(cells[0][2][0].dropped == 0 && s->shown == 49 && s->scans_repeat == 27 && s->scans_underrun == 0);
    s = &cells[0][1][0].res.stats;                                      /* 120 Hz scans, 10/16 ms pictures */
    assert(cells[0][1][0].dropped == 0 && s->shown == 49 && s->scans_repeat == 27 && s->scans_underrun == 0);
    s = &cells[2][0][0].res.stats;                                      /* 13 ms scans, 120 Hz pictures: one shown per scan, the rest dropped */
    assert(s->shown == 49 && cells[2][0][0].dropped == 27 && s->scans_repeat == 0 && s->scans_underrun == 0);
    s = &cells[1][0][0].res.stats;
    assert(s->shown == 49 && cells[1][0][0].dropped == 27 && s->scans_repeat == 0 && s->scans_underrun == 0);

    int64_t iv[VRR_TRACE_MAX];                                          /* the shuffled picture times on the fixed grid change only on 8 1/3 or 16 2/3 ms */
    size_t n = visible_intervals(&cells[0][1][1], iv);                  /* (with the late cookies: the intervals are the same) */
    assert(n == 48);
    for (size_t i = 0; i < n; i++) assert(llabs(iv[i] - 25 * MS / 3) <= 2 || llabs(iv[i] - 50 * MS / 3) <= 2);
    n = visible_intervals(&cells[1][1][1], iv);                         /* and the shuffled trace on its own times reproduces them exactly */
    assert(n == 48);
    for (size_t i = 0; i < n; i++) assert(iv[i] == vrr_trace_shuffled_ms((int)i) * MS);

    printf("trace_matrix ok (9 cells x 2 cookie lags)\n");
    if (report) print_report((const cell(*)[3][2])cells);
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s STREAM.hevc STREAM.order [--report | --print-shuffled]\n", argv[0]);
        return 2;
    }
    stream_path = argv[1];
    order_path = argv[2];
    if (argc > 3 && !strcmp(argv[3], "--print-shuffled")) {
        for (int i = 0; i < 48; i++) printf("%d%c", vrr_trace_shuffled_ms(i), i == 47 ? '\n' : ' ');
        return 0;
    }
    test_bframe_reordering();
    test_hold_until_retirement();
    test_repeat_holds_one_surface();
    test_duplicates();
    test_stale_and_future_epoch();
    test_full_queue();
    test_flush();
    test_flush_holds_the_scanned_surface();
    test_selection_follows_presentation_order();
    test_shutdown();
    test_retire_after_drop_and_bad_cookies();
    test_lost_cookie_is_a_reported_leak();
    test_oracle_catches_early_release();
    test_ack_is_not_ownership();
    test_backpressure_never_recycles();
    test_trace_matrix(argc > 3 && !strcmp(argv[3], "--report"));
    return 0;
}
