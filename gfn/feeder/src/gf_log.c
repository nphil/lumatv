#define _GNU_SOURCE
#include "gf_log.h"

#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

void gf_say(const char *fmt, ...) {
    char line[512];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    fprintf(stderr, "gfn-feeder %lld: %s\n", (long long)gf_ms(), line);
    fflush(stderr);
}

int64_t gf_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int64_t gf_ms(void) { return gf_ns() / 1000000; }
int64_t vpl_ms(void) { return gf_ms(); }

void gf_meminfo(long *cma_kb, long *avail_kb, long *rss_kb) {
    *cma_kb = *avail_kb = *rss_kb = 0;
    FILE *f = fopen("/proc/meminfo", "r");
    if (f) {
        char k[64];
        long v;
        while (fscanf(f, "%63s %ld kB\n", k, &v) == 2) {
            if (!strcmp(k, "CmaFree:")) *cma_kb = v;
            else if (!strcmp(k, "MemAvailable:")) *avail_kb = v;
        }
        fclose(f);
    }
    f = fopen("/proc/self/statm", "r");
    if (f) {
        long size, res;
        if (fscanf(f, "%ld %ld", &size, &res) == 2) *rss_kb = res * (long)(sysconf(_SC_PAGESIZE) / 1024);
        fclose(f);
    }
}
