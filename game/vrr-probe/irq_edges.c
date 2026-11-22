/* Bracket changes to Linux IRQ accounting without enabling kernel tracing. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

struct edge { uint64_t lo, hi, count; };

static uint64_t now_ns(void)
{
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) < 0) { perror("clock_gettime"); exit(1); }
    return (uint64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static int read_count(int fd, uint64_t *sum)
{
    char buf[4096];
    ssize_t n = pread(fd, buf, sizeof buf, 0);
    if (n < 0) { perror("per_cpu_count"); return -1; }
    uint64_t total = 0, value = 0;
    int digits = 0;
    for (ssize_t i = 0; i < n; ++i) {
        unsigned c = (unsigned char)buf[i];
        if (c >= '0' && c <= '9') {
            if (value > (UINT64_MAX - (c - '0')) / 10) break;
            value = value * 10 + (c - '0');
            digits = 1;
        } else {
            if (!digits || UINT64_MAX - total < value) break;
            total += value;
            if (c == '\n' && i + 1 == n) { *sum = total; return 0; }
            if (c != ',') break;
            value = 0; digits = 0;
        }
    }
    fputs("incomplete or invalid per_cpu_count\n", stderr);
    return -1;
}

int main(int argc, char **argv)
{
    if (argc != 2 || !argv[1][0] || strlen(argv[1]) > 9 ||
        argv[1][strspn(argv[1], "0123456789")]) {
        fputs("usage: irq-edges IRQ_NUMBER\n", stderr); return 2;
    }
    char path[64];
    snprintf(path, sizeof path, "/sys/kernel/irq/%s/per_cpu_count", argv[1]);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) { perror(path); return 1; }
    struct edge edges[512];
    size_t used = 0;
    uint64_t start = now_ns(), prev_lo = start, prev, initial;
    if (read_count(fd, &prev) < 0) return 1;
    initial = prev;
    uint64_t hi = now_ns();
    while (hi - start < 2000000000) {
        const struct timespec pause = { .tv_nsec = 100000 };
        if (nanosleep(&pause, NULL) < 0 && errno != EINTR) { perror("nanosleep"); return 1; }
        uint64_t lo = now_ns(), count;
        if (lo - start >= 2000000000) break;
        if (read_count(fd, &count) < 0) return 1;
        hi = now_ns();
        if (count != prev) {
            if (count < prev || used == sizeof edges / sizeof *edges) {
                fputs("counter decreased or edge capacity exceeded; discard capture\n", stderr); return 1;
            }
            /* The accounting increment may occur inside either read. */
            edges[used++] = (struct edge){ prev_lo, hi, count };
        }
        prev_lo = lo; prev = count;
    }
    close(fd);
    printf("# clock=CLOCK_MONOTONIC irq=%s initial=%" PRIu64 " start_ns=%" PRIu64 " last_read_ns=%" PRIu64 "\n",
           argv[1], initial, start, hi);
    puts("previous_read_start_ns,current_read_end_ns,increment");
    prev = initial;
    for (size_t i = 0; i < used; ++i) {
        printf("%" PRIu64 ",%" PRIu64 ",%" PRIu64 "\n", edges[i].lo, edges[i].hi, edges[i].count - prev);
        prev = edges[i].count;
    }
    return fflush(stdout) == EOF;
}
