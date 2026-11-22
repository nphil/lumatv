/* The sender-time mapping (src/gf_vclock.h) with explicit inputs: no clock, no sleeps. `--replay` reads `rtp hz local_ms gap` lines on stdin (one per
 * picture: the sender's RTP timestamp and rate, the stamp the picture got by default, 1 if a gap was reported before it) and prints `stamp epochs why`
 * for each; game/vrr-probe/replay_sendertime.py drives it with a capture. */
#undef NDEBUG
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "gf_vclock.h"

/* One plane as the feeder keeps it: the last stamp given is the floor of the next. */
typedef struct { gf_vclock c; int64_t floor; } plane;

static int64_t put(plane *p, uint32_t rtp, uint32_t hz, int64_t local) {
    int64_t ms = gf_vclock_map(&p->c, rtp, hz, local, p->floor);
    assert(ms >= p->floor);
    p->floor = ms;
    return ms;
}

static uint64_t rng(uint64_t *s) {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    return *s;
}

static void the_first_picture_is_stamped_where_it_would_be_by_default(void) {
    plane p = {0};
    assert(put(&p, 595241, 90000, 1219) == 1219);
    assert(p.c.epochs == 1 && p.c.why == GF_VCLOCK_WHY_FIRST);
    /* 750 ticks are 8.33 ms and 2250 are 25: rounded up, as in the offline replay of the Doom capture; the handover time no longer matters */
    assert(put(&p, 595241 + 750, 90000, 1220) == 1228 && p.c.why == GF_VCLOCK_WHY_NONE);
    assert(put(&p, 595241 + 2250, 90000, 1250) == 1244);
    assert(p.c.epochs == 1);
    plane behind = {.floor = 1300};
    assert(put(&behind, 7, 90000, 1219) == 1300);       /* never below the last stamp */
}

static void the_32_bit_wrap_is_an_ordinary_step(void) {
    plane p = {0};
    uint32_t rtp = 0xFFFFFFFFu - 100;
    assert(put(&p, rtp, 90000, 50) == 50);
    rtp += 750;
    assert(rtp == 649);
    assert(put(&p, rtp, 90000, 58) == 59);              /* 50 + ceil(8.33) */
    rtp += 750;
    assert(put(&p, rtp, 90000, 66) == 67);              /* 50 + ceil(16.67) */
    assert(p.c.epochs == 1);
}

static void the_intervals_of_a_doom_shaped_trace_are_the_senders_not_the_handovers(void) {
    /* 1202 pictures in steps of 290-1675 ticks (3.2-18.6 ms at 90 kHz), handed over 0-13 ms behind the sender's grid and never out of order; the 32-bit
     * wrap falls inside the trace. The Doom capture's own intervals are 3.222-18.611 ms. */
    plane p = {0};
    uint64_t s = 88172645463325252ull;
    uint32_t rtp = 0xFFFFFFFFu - 1295;
    int64_t ticks = 0, local = 0, last = 0, step = 0, lo = 0, hi = 0;
    int within = 0;
    for (int i = 0; i < 1202; i++) {
        int64_t grid = 1000 + gf_vclock_ms(ticks, 90000) + (int64_t)(rng(&s) % 14);
        local = grid > local ? grid : local;
        int64_t ms = put(&p, rtp, 90000, local);
        int64_t off = ms - local;
        if (i == 0) {
            lo = hi = off;
        } else {
            within += ((ms - last) * 90 - step < 90 && step - (ms - last) * 90 < 90);
            lo = off < lo ? off : lo;
            hi = off > hi ? off : hi;
        }
        last = ms;
        step = 290 + (int64_t)(rng(&s) % 1386);
        ticks += step;
        rtp += (uint32_t)step;
    }
    assert(within == 1201);                             /* every interval within 1 ms of the sender's */
    assert(p.c.epochs == 1);                            /* and one epoch: the handover jitter never reaches the bounds */
    assert(lo >= -13 && hi <= 13);
}

static void a_break_opens_an_epoch_anchored_at_the_larger_of_the_default_and_the_last_stamp(void) {
    plane p = {0};
    put(&p, 1000, 90000, 500);
    assert(put(&p, 1750, 90000, 509) == 509);
    gf_vclock_break(&p.c);                              /* a flush, a hold for loss or a reported gap */
    /* the default is behind the last stamp: the new epoch starts at the last stamp, not before it */
    assert(put(&p, 900000, 90000, 505) == 509 && p.c.epochs == 2 && p.c.why == GF_VCLOCK_WHY_FIRST);
    assert(put(&p, 900750, 90000, 510) == 518);
    gf_vclock_break(&p.c);
    assert(put(&p, 5, 90000, 800) == 800 && p.c.epochs == 3);
    /* a load starts afresh: no epoch, and the plane's own floor is back at zero */
    gf_vclock_reset(&p.c);
    p.floor = 0;
    assert(p.c.epochs == 0 && !p.c.open);
    assert(put(&p, 99, 90000, 3) == 3 && p.c.epochs == 1);
}

static void a_clock_that_steps_back_or_changes_rate_opens_an_epoch(void) {
    plane p = {0};
    put(&p, 100000, 90000, 1000);
    assert(put(&p, 100750, 90000, 1008) == 1009);
    /* an older picture: reordered, or the sender's clock restarted */
    assert(put(&p, 100000, 90000, 1016) == 1016 && p.c.why == GF_VCLOCK_WHY_STEP_BACK && p.c.epochs == 2);
    /* ...while the default stamp is behind the last one: held there */
    assert(put(&p, 99000, 90000, 1012) == 1016 && p.c.why == GF_VCLOCK_WHY_STEP_BACK && p.c.epochs == 3);
    assert(put(&p, 99100, 48000, 1024) == 1024 && p.c.why == GF_VCLOCK_WHY_RATE && p.c.epochs == 4);
    /* no clock rate, no sender time: the default stamp, and the next picture with one starts an epoch */
    assert(put(&p, 5, 0, 1030) == 1030 && p.c.why == GF_VCLOCK_WHY_NONE && !p.c.open && p.c.epochs == 4);
    assert(put(&p, 6, 90000, 1040) == 1040 && p.c.why == GF_VCLOCK_WHY_FIRST && p.c.epochs == 5);
}

static void a_jump_the_handover_time_does_not_share_is_not_followed_but_a_stall_it_shares_is(void) {
    plane p = {0};
    put(&p, 0, 90000, 1000);
    assert(put(&p, 750, 90000, 1009) == 1009);
    /* 10 s of sender time that did not pass here: the epoch restarts and the stamp does not leap */
    assert(put(&p, 750 + 900000, 90000, 1017) == 1017 && p.c.why == GF_VCLOCK_WHY_OFFSET);
    /* 0.5 s of sender time that did pass (a stall at the sender): the same epoch carries on */
    uint32_t epochs = p.c.epochs;
    assert(put(&p, 750 + 900000 + 45000, 90000, 1517) == 1517 && p.c.epochs == epochs);
    /* a picture handed over 100 ms behind a sender that moved 8.3 ms: behind its bound, so the stamp catches up to the default */
    assert(put(&p, 750 + 900000 + 45750, 90000, 1617) == 1617 && p.c.why == GF_VCLOCK_WHY_OFFSET);
}

static void a_sender_clock_that_drifts_is_held_to_the_bounds_and_never_goes_back(void) {
    /* 750 ticks of sender time for 8 ms (the sender 4% fast) or 9 ms (7% slow) of handover time, 2000 pictures each */
    for (int slow = 0; slow < 2; slow++) {
        plane p = {0};
        uint32_t rtp = 0;
        int64_t local = 1000, last = 0;
        for (int i = 0; i < 2000; i++) {
            int64_t ms = put(&p, rtp, 90000, local);
            assert(ms >= last);
            assert(ms - local <= GF_VCLOCK_AHEAD_MS && local - ms <= GF_VCLOCK_BEHIND_MS);
            last = ms;
            rtp += 750;
            local += slow ? 9 : 8;
        }
        assert(p.c.epochs > 1);
    }
}

static void random_breaks_jumps_and_steps_back_keep_the_stamps_rising_and_inside_the_bounds(void) {
    uint64_t s = 0x9E3779B97F4A7C15ull;
    for (int run = 0; run < 40; run++) {
        plane p = {0};
        uint32_t rtp = (uint32_t)rng(&s);
        int64_t local = 0, last = 0;
        for (int i = 0; i < 1500; i++) {
            uint64_t r = rng(&s);
            switch (r % 40) {
            case 0: gf_vclock_break(&p.c); break;
            case 1: rtp += (uint32_t)(r >> 8) % 4000000; break;     /* up to 44 s forward */
            case 2: rtp -= (uint32_t)(r >> 8) % 4000000; break;     /* or back */
            default: break;
            }
            rtp += 1 + (uint32_t)(r >> 40) % 1700;
            local += (int64_t)((r >> 20) % 20);
            int64_t ms = put(&p, rtp, 90000, local);
            assert(ms >= last && ms - local <= GF_VCLOCK_AHEAD_MS && local - ms <= GF_VCLOCK_BEHIND_MS);
            last = ms;
        }
    }
}

static int replay(void) {
    plane p = {0};
    unsigned long long rtp;
    unsigned hz, gap;
    long long local;
    while (scanf("%llu %u %lld %u", &rtp, &hz, &local, &gap) == 4) {
        if (gap) gf_vclock_break(&p.c);
        long long ms = put(&p, (uint32_t)rtp, hz, local);
        printf("%lld %u %d\n", ms, p.c.epochs, p.c.why);
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--replay")) return replay();
    the_first_picture_is_stamped_where_it_would_be_by_default();
    the_32_bit_wrap_is_an_ordinary_step();
    the_intervals_of_a_doom_shaped_trace_are_the_senders_not_the_handovers();
    a_break_opens_an_epoch_anchored_at_the_larger_of_the_default_and_the_last_stamp();
    a_clock_that_steps_back_or_changes_rate_opens_an_epoch();
    a_jump_the_handover_time_does_not_share_is_not_followed_but_a_stall_it_shares_is();
    a_sender_clock_that_drifts_is_held_to_the_bounds_and_never_goes_back();
    random_breaks_jumps_and_steps_back_keep_the_stamps_rising_and_inside_the_bounds();
    puts("vclock_test: 8 rules passed");
    return 0;
}
