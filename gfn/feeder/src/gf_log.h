/* lumatv-gfn-feeder: the log (stderr, one line each), the clock, and the JSON lines the probe and the serve mode write to stdout. */
#ifndef GF_LOG_H
#define GF_LOG_H

#include <stdint.h>

void gf_say(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
int64_t gf_ns(void);            /* CLOCK_MONOTONIC */
int64_t gf_ms(void);

/* vplane's vpl_wl.c (compiled into the feeder for the window) wants this name. */
int64_t vpl_ms(void);

/* /proc/meminfo's CmaFree and MemAvailable, and this process's RSS, in KB (0 when unreadable). */
void gf_meminfo(long *cma_kb, long *avail_kb, long *rss_kb);

#endif
