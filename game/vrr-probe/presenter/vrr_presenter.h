/* Decoded-picture ownership and scan accounting for internal 4K VRR. A HOST MODEL: nothing here runs on the TV, requests a scan or observes one.
 * The deadline traces in vrr_trace.h are inputs to it, not measurements of a panel.
 * Results and limits: docs/research/decoded-vrr-4k.md, "Decoded-picture presenter (host model)".
 *
 * The contract
 *   Picture       {epoch, id, pts + timebase, ready_ns, handle}. `handle` is the decoded surface in its native format and is opaque: the
 *                 presenter never reads, converts or copies through it. `id` names the picture for its whole life and (epoch, id) is unique
 *                 among the pictures the presenter owns. `ready_ns` is the local monotonic time the picture became usable. `pts` is the
 *                 presentation timestamp in `tb` ticks, on one timebase within an epoch. Pictures may arrive in any order, since a decoder's
 *                 callback order is not presentation order; no two queued pictures share a pts.
 *   Ownership     vrr_submit() == VRR_OK moves the handle to the presenter; any other result leaves it with the caller. The presenter calls
 *                 `release` for a handle exactly once, inside vrr_retire(), and nowhere else. Nothing else may free, reuse or overwrite
 *                 the surface in between.
 *   Retirement    Each accepted picture has one cookie, first shown to the backend in the show or drop command that names the picture. The
 *                 backend returns it with vrr_retire() only once no read of the surface can still happen, for a dropped picture as for a
 *                 displayed one, as a V4L2 capture buffer is requeued only when the sink gives its cookie back. The presenter infers no
 *                 retirement: the scanout and prefetch tail is unknown, so a replacing scan or its IRQ, a drop command, a timeout, a picture
 *                 count and the capacity authorise no reuse. A cookie is accepted once; a stale or repeated one, or one for a picture still
 *                 queued or on screen, changes nothing.
 *   Backpressure  `capacity` bounds the pictures owned, queued or awaiting retirement. At the bound vrr_submit() fails with VRR_E_FULL and
 *                 the picture stays with its caller; no surface is recycled to make room.
 *   Wire ACK      The compressed-input acknowledgement NVST sends for an access unit is a different fact. It has no entry point here and
 *                 cannot start, extend or end ownership. Feed or ACK events are no substitute for a retirement.
 *   Selection     vrr_scan(scan_ns) reports that a scan started. Of the queued pictures with ready_ns <= scan_ns the one with the greatest
 *                 pts is shown: the newest in presentation order within the epoch (low-latency play), not the last the decoder returned.
 *                 Queued pictures with a smaller pts are dropped, and so is one that arrives with a pts at or below the shown picture's,
 *                 so the shown pts never goes back. With nothing eligible the picture on screen is repeated (nothing on screen: an
 *                 underrun). Repeats keep one surface held; it is retired once, when a newer picture replaces it.
 *   Epoch         A discontinuity starts a new epoch. vrr_flush() invalidates the queued work of the old epoch (dropped, never shown) and
 *                 refuses older and not-yet-announced epochs from then on. It does not touch the surface still being scanned: that picture
 *                 stays owned until a verified replacement (the new epoch's first picture is shown, then its cookie returns the old one) or
 *                 a handback (vrr_shutdown(), then the cookie). Until then a scan selects nothing (VRR_SCANNED_STALE), so an old-epoch
 *                 picture is never shown anew.
 *   Handback      vrr_shutdown() drops what is queued and refuses new work. Cookies still outstanding must be returned by the backend as it
 *                 stops reading; vrr_free() reports any that were not, and does NOT release them: a surface the hardware may still
 *                 read is never recycled on a guess.
 *   Re-entry      The callbacks run inside the call that causes them and must not call back into the presenter: they queue the cookie and
 *                 return it later from the backend's own context. The presenter is single-threaded; the caller serialises it.
 *
 * Every time is an argument; the presenter reads no clock. Slots are allocated once, in vrr_new().
 */
#ifndef VRR_PRESENTER_H
#define VRR_PRESENTER_H

#include <stddef.h>
#include <stdint.h>

typedef struct { int32_t num, den; } vrr_timebase;      /* seconds per tick = num / den */

typedef struct {
    uint32_t epoch;
    uint64_t id;
    int64_t pts;
    vrr_timebase tb;
    int64_t ready_ns;
    void *handle;
} vrr_picture;

/* Names one accepted picture for its whole life. A zero value (gen 0) is never issued. */
typedef struct { uint32_t slot, gen; } vrr_cookie;

enum {
    VRR_OK = 0,
    VRR_E_ARG = -1,                 /* missing picture, bad timebase, bad configuration */
    VRR_E_NO_HANDLE = -2,           /* no surface to own: an ACK alone does not make a picture */
    VRR_E_STALE_EPOCH = -3,         /* older than the presenter's epoch (flush: not newer than it) */
    VRR_E_FUTURE_EPOCH = -4,        /* newer than the epoch announced by vrr_flush() */
    VRR_E_DUPLICATE = -5,           /* the (epoch, id) or the handle is already owned, or a queued picture has the same pts */
    VRR_E_TIMEBASE = -6,            /* the timebase changed within the epoch */
    VRR_E_ORDER = -7,               /* scan_ns not increasing */
    VRR_E_FULL = -8,                /* capacity reached, pictures awaiting retirement included: backpressure, the picture stays with the caller */
    VRR_E_CLOSED = -9,              /* after vrr_shutdown() */
    VRR_E_STALE_COOKIE = -10,       /* not outstanding: unknown, already retired, or from an earlier tenant of the slot */
    VRR_E_PREMATURE = -11,          /* the picture is still queued or on screen */
    VRR_E_REENTRANT = -12,          /* called from inside one of its own callbacks */
    VRR_E_COUNT = 13
};

const char *vrr_strerror(int rc);

typedef enum { VRR_ST_NONE = 0, VRR_ST_QUEUED, VRR_ST_CURRENT, VRR_ST_SUPERSEDED, VRR_ST_DROPPED } vrr_state;

/* SUPERSEDED: a queued picture with a smaller pts than the one shown. LATE: arrived at or below the shown pts. */
typedef enum { VRR_DROP_SUPERSEDED = 0, VRR_DROP_FLUSH, VRR_DROP_SHUTDOWN, VRR_DROP_LATE, VRR_DROP_COUNT } vrr_drop_reason;
typedef enum { VRR_SHOW_NEW = 1, VRR_SHOW_REPEAT } vrr_show_kind;

/* Scan commands to the backend. The handle is a loan, valid until the cookie is returned. */
typedef struct {
    vrr_show_kind kind;
    vrr_cookie cookie;
    vrr_cookie supersedes;          /* NEW: the picture this scan replaces (zero when none). Naming it authorises nothing: its cookie comes back when its reads have ceased */
    uint32_t epoch;
    uint64_t id;
    int64_t pts;
    vrr_timebase tb;
    int64_t scan_ns;
    void *handle;
} vrr_show;

typedef struct {
    vrr_cookie cookie;              /* returned only after every possible read of the surface has ceased */
    uint32_t epoch;
    uint64_t id;
    vrr_drop_reason reason;
    void *handle;
} vrr_drop;

/* THE PLUG POINT for a hardware backend. It is a description, not an implementation: no TV call exists that provides it. NDL's compressed-input
 * call returns no picture, handle or retirement, and the stock decoder's buffers are not reachable from the app jail (see the doc). The only
 * implementation is the deadline-trace model in vrr_trace.c. Outbound, the presenter commands the scan; inbound, the backend calls vrr_scan() when a
 * scan starts and vrr_retire() for every cookie. A backend that can start a scan on demand needs one more call, to ask for the next scan when a
 * picture is ready. None is known to exist, so none is declared here. */
typedef struct {
    void *user;
    void (*show)(void *user, const vrr_show *cmd);          /* optional */
    void (*drop)(void *user, const vrr_drop *cmd);          /* optional */
} vrr_scan_backend;

typedef enum { VRR_DISPLAYED = 1, VRR_DROPPED } vrr_outcome;

/* What happened to one picture, handed over with its surface. Times are -1 when the event never happened. */
typedef struct {
    uint32_t epoch;
    uint64_t id;
    int64_t pts;
    vrr_timebase tb;
    vrr_outcome outcome;
    vrr_drop_reason reason;         /* why its hold ended: the drop reason; for a displayed picture SUPERSEDED (replaced by a newer scan) or SHUTDOWN (handback) */
    uint32_t scans;                 /* scans that showed it: 1 + repeats; 0 when dropped */
    int64_t ready_ns, first_scan_ns, last_scan_ns;
    int64_t decided_ns;             /* a newer picture's scan began, or the drop or handback was decided */
    int64_t retire_ns;
} vrr_retirement;

/* Frees the surface. Called once per accepted picture, from vrr_retire() only. */
typedef void (*vrr_release_fn)(void *user, void *handle, const vrr_retirement *r);

typedef struct {
    unsigned capacity;              /* pictures owned at once, queued or awaiting retirement: 1..VRR_MAX_CAPACITY. A bound for backpressure, never a reuse rule */
    vrr_release_fn release;         /* required */
    void *release_user;
    vrr_scan_backend backend;
} vrr_config;

enum { VRR_MAX_CAPACITY = 1024 };

typedef struct {
    uint64_t submitted;                         /* accepted */
    uint64_t rejected[VRR_E_COUNT];             /* by error code, as -code */
    uint64_t shown;                             /* pictures with at least one scan */
    uint64_t dropped[VRR_DROP_COUNT];
    uint64_t scans, scans_new, scans_repeat, scans_stale, scans_underrun;
    uint64_t retired_displayed, retired_dropped;
    unsigned peak_owned;
} vrr_stats;

typedef struct vrr_presenter vrr_presenter;

vrr_presenter *vrr_new(const vrr_config *cfg);

/* `cookie` may be NULL. A picture that is refused keeps its handle with the caller. */
int vrr_submit(vrr_presenter *p, const vrr_picture *pic, vrr_cookie *cookie);

/* STALE: after a flush, with no new-epoch picture to show; the old surface stays owned and is not selected. */
enum { VRR_SCANNED_NEW = 1, VRR_SCANNED_REPEAT, VRR_SCANNED_UNDERRUN, VRR_SCANNED_STALE };

/* A scan started at `scan_ns`, strictly later than the last one. VRR_SCANNED_* or an error. */
int vrr_scan(vrr_presenter *p, int64_t scan_ns);

/* The backend no longer reads the picture the cookie names. Releases its surface. */
int vrr_retire(vrr_presenter *p, vrr_cookie cookie, int64_t now_ns);

/* A discontinuity: `new_epoch` must be greater than the current one. */
int vrr_flush(vrr_presenter *p, uint32_t new_epoch, int64_t now_ns);

/* The handback to stock playback. Returns the number of pictures still owned: the cookies the backend has yet to return. */
int vrr_shutdown(vrr_presenter *p, int64_t now_ns);

/* Pictures owned now; a cookie's state (VRR_ST_NONE when it is not outstanding); the epoch; the counters. */
unsigned vrr_owned(const vrr_presenter *p);
vrr_state vrr_cookie_state(const vrr_presenter *p, vrr_cookie cookie);
uint32_t vrr_epoch(const vrr_presenter *p);
const vrr_stats *vrr_get_stats(const vrr_presenter *p);

/* Frees the presenter and returns how many pictures were still owned. Those surfaces are abandoned unreleased: a nonzero result is a leak. */
unsigned vrr_free(vrr_presenter *p);

#endif
