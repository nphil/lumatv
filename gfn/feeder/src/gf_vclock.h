/* The sender's picture times as the player's PTS (opt-in: the engine sends GF_VIDEO_CLOCK; docs/research/decoded-vrr-4k.md, "Sender time as the
 * player's PTS").
 *
 * By default a picture is stamped with the time the engine handed it over, so pacing and network jitter shape the intervals the player sees: in the
 * Doom 4K120 capture 625 of 1201 play() intervals matched the sender's within 1 ms, where the sender's RTP clock matches all 1201. With a sender
 * stamp the picture is stamped
 *
 *     anchor_ms + ceil(ticks * 1000 / clock_hz)
 *
 * ticks being the sender's RTP time since the first picture of the epoch (signed 32-bit steps, so the wrap is no special case) and anchor_ms the stamp
 * that picture would have had by default: the first picture stays where it always was, on the timeline the audio plane is stamped on.
 *
 * Audio shares no clock with the video: no RTCP sender report, NTP time or server timestamp relates their RTP clocks, and audio is stamped on the TV's
 * clock at feed time. The TV's own clock at an epoch's first picture is the only anchor between the planes. A sender clock that drifts from the TV's
 * (100 ppm is 6 ms a minute) would walk the video away from the audio without limit, so the offset from the default stamp is held inside
 * [-BEHIND, +AHEAD]; a picture that lands outside opens a new epoch. The Doom trace stayed within -13..+1 ms of the default over its 10 s.
 *
 * An epoch also ends at a break (a hold for loss, a flush, a gap the engine reported) and when the RTP clock steps back or changes rate. A new epoch
 * is anchored at the larger of the default stamp and the last stamp given, so stamps never decrease, across epochs included. A load starts afresh.
 *
 * Pure arithmetic: no clock, no I/O, no allocation. tests/vclock_test.c drives it with explicit inputs.
 */
#ifndef GF_VCLOCK_H
#define GF_VCLOCK_H

#include <stdint.h>

#define GF_VCLOCK_AHEAD_MS 30       /* the sender's timeline may run this far past the default stamp... */
#define GF_VCLOCK_BEHIND_MS 50      /* ...or this far behind it: a held picture waits up to 1.5 periods, 25 ms at 60 fps */

enum { GF_VCLOCK_WHY_NONE = 0, GF_VCLOCK_WHY_FIRST, GF_VCLOCK_WHY_STEP_BACK, GF_VCLOCK_WHY_RATE, GF_VCLOCK_WHY_OFFSET };

typedef struct {
    int open;                   /* an epoch is running */
    uint32_t hz, last_rtp;      /* its tick rate and the previous picture's RTP timestamp */
    int64_t ticks, anchor_ms;   /* sender ticks since the epoch's first picture; that picture's stamp */
    uint32_t epochs;            /* opened since the load */
    int why;                    /* GF_VCLOCK_WHY_*: what opened an epoch at the last picture; _NONE if none did */
} gf_vclock;

static inline void gf_vclock_reset(gf_vclock *c) { *c = (gf_vclock){0}; }

/* The next picture opens a new epoch. */
static inline void gf_vclock_break(gf_vclock *c) { c->open = 0; }

static inline int64_t gf_vclock_ms(int64_t ticks, uint32_t hz) { return (ticks * 1000 + (int64_t)hz - 1) / (int64_t)hz; }

/* The stamp for a picture with sender timestamp `rtp` at `hz` ticks a second. `local_ms` is the stamp it would have by default, `floor_ms` the last
 * stamp given on this plane; the result is never below `floor_ms`. A picture without a clock rate has no sender time: it gets the default. */
static inline int64_t gf_vclock_map(gf_vclock *c, uint32_t rtp, uint32_t hz, int64_t local_ms, int64_t floor_ms) {
    int64_t start = local_ms > floor_ms ? local_ms : floor_ms;
    int why = GF_VCLOCK_WHY_FIRST;
    c->why = GF_VCLOCK_WHY_NONE;
    if (hz == 0) {
        c->open = 0;
        return start;
    }
    if (c->open) {
        int32_t step = (int32_t)(rtp - c->last_rtp);
        if (c->hz != hz) {
            why = GF_VCLOCK_WHY_RATE;
        } else if (step < 0) {
            why = GF_VCLOCK_WHY_STEP_BACK;
        } else {
            int64_t ticks = c->ticks + step;
            int64_t ms = c->anchor_ms + gf_vclock_ms(ticks, hz);
            int64_t off = ms - local_ms;
            if (off <= GF_VCLOCK_AHEAD_MS && off >= -GF_VCLOCK_BEHIND_MS) {
                c->ticks = ticks;
                c->last_rtp = rtp;
                return ms > floor_ms ? ms : floor_ms;
            }
            why = GF_VCLOCK_WHY_OFFSET;
        }
    }
    c->open = 1;
    c->hz = hz;
    c->last_rtp = rtp;
    c->ticks = 0;
    c->anchor_ms = start;
    c->epochs++;
    c->why = why;
    return start;
}

#endif
