/* Does the panel blank when the keeper returns it to 120 Hz? Samples what the kernel shows of a rate change, every 0.2 to 1 ms, and writes one CSV
 * after the run (the writing is off the sampling).
 *
 *   blank-probe SECONDS OUT.csv [IRQ_NUMBER]       (IRQ 104 is osd_irq on this set)
 *
 * E,lo_ns,hi_ns,increment   osd_irq (the OSD's vsync at the panel's own rate) changed: the increment fell between the start of the previous read and the
 *                           end of this one. A gap between two such rows longer than a frame is a scan that stopped.
 * S,t_ns,localmute,vFreq,black_out,connect_src
 *                           a row whenever one of the four changes: the video path's mute (/proc/lg/de/win0/status), the rate the display engine was
 *                           told, the panel link's black-out (/proc/lg/ovi/status) and the scaler's source (/proc/lgtv-driver/scaler/main.p0).
 * K,t_ns                    SIGUSR1 arrived: the caller's mark (the key, the stop of a clip), on the same clock as every other row.
 *
 * Clock: CLOCK_MONOTONIC, nanoseconds. Opens the three files read-only and writes nothing to the driver. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <sched.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

#define MAX_EDGES 800000
#define MAX_STATES 60000
#define MAX_MARKS 64

struct edge { uint64_t lo, hi; uint32_t inc; };
struct state { uint64_t t; char lm[16], vf[16], bo[40], src[24]; };

static uint64_t now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static volatile sig_atomic_t marks_used;
static volatile uint64_t marks[MAX_MARKS];

static void on_usr1(int sig)
{
    (void)sig;
    if (marks_used < MAX_MARKS) { marks[marks_used] = now_ns(); marks_used++; }
}

/* The sum of a per_cpu_count line ("12,34,0,56\n"). */
static int read_count(int fd, uint64_t *sum)
{
    char buf[512];
    ssize_t n = pread(fd, buf, sizeof buf - 1, 0);
    if (n <= 0) return -1;
    uint64_t total = 0, value = 0;
    for (ssize_t i = 0; i < n; ++i) {
        unsigned c = (unsigned char)buf[i];
        if (c >= '0' && c <= '9') value = value * 10 + (c - '0');
        else { total += value; value = 0; if (c == '\n') break; }
    }
    *sum = total;
    return 0;
}

/* The token after `key` (and the ':', '=' and blanks that follow it), up to the end of the line, with blanks inside made '_'. */
static void field(const char *buf, const char *key, char *out, size_t cap)
{
    out[0] = 0;
    const char *p = strstr(buf, key);
    if (!p) { snprintf(out, cap, "-"); return; }
    p += strlen(key);
    while (*p == ' ' || *p == ':' || *p == '=' || *p == '\t') ++p;
    size_t i = 0;
    while (*p && *p != '\n' && *p != '\r' && i + 1 < cap) { out[i++] = *p == ' ' ? '_' : *p; ++p; }
    while (i > 0 && out[i - 1] == '_') --i;
    out[i] = 0;
    if (!i) snprintf(out, cap, "-");
}

static ssize_t slurp(int fd, char *buf, size_t cap)
{
    ssize_t n = fd < 0 ? -1 : pread(fd, buf, cap - 1, 0);
    if (n < 0) n = 0;
    buf[n] = 0;
    return n;
}

int main(int argc, char **argv)
{
    if (argc < 3 || argc > 4) { fputs("usage: blank-probe SECONDS OUT.csv [IRQ_NUMBER]\n", stderr); return 2; }
    double seconds = atof(argv[1]);
    const char *irq = argc == 4 ? argv[3] : "104";
    if (seconds <= 0 || seconds > 600 || strspn(irq, "0123456789") != strlen(irq) || strlen(irq) > 9) { fputs("bad arguments\n", stderr); return 2; }
    struct rlimit nocore = { 0, 0 };
    setrlimit(RLIMIT_CORE, &nocore);
    struct sched_param sp = { .sched_priority = 20 };
    if (sched_setscheduler(0, SCHED_FIFO, &sp) < 0) fputs("note: no real-time priority, the wake-ups may be late\n", stderr);
    signal(SIGUSR1, on_usr1);

    char path[64];
    snprintf(path, sizeof path, "/sys/kernel/irq/%s/per_cpu_count", irq);
    int fd_irq = open(path, O_RDONLY | O_CLOEXEC);
    if (fd_irq < 0) { perror(path); return 1; }
    int fd_win = open("/proc/lg/de/win0/status", O_RDONLY | O_CLOEXEC);
    int fd_ovi = open("/proc/lg/ovi/status", O_RDONLY | O_CLOEXEC);
    int fd_scl = open("/proc/lgtv-driver/scaler/main.p0", O_RDONLY | O_CLOEXEC);
    if (fd_win < 0 || fd_ovi < 0) fputs("note: the mute or black-out file is not readable\n", stderr);

    struct edge *edges = malloc(sizeof *edges * MAX_EDGES);
    struct state *states = malloc(sizeof *states * MAX_STATES);
    if (!edges || !states) { fputs("out of memory\n", stderr); return 1; }
    size_t ne = 0, ns = 0;
    char *buf = malloc(65536);
    if (!buf) return 1;

    uint64_t start = now_ns(), prev_lo = start, prev = 0, initial = 0, hi = start;
    if (read_count(fd_irq, &prev) < 0) { perror("per_cpu_count"); return 1; }
    initial = prev;
    struct timespec real;
    clock_gettime(CLOCK_REALTIME, &real);
    const uint64_t end = start + (uint64_t)(seconds * 1e9);
    struct state last = { 0 };
    strcpy(last.lm, "?");
    unsigned round = 0;
    for (;;) {
        const struct timespec pause = { .tv_sec = 0, .tv_nsec = 200000 };
        nanosleep(&pause, NULL);
        uint64_t lo = now_ns(), count;
        if (lo >= end) break;
        if (read_count(fd_irq, &count) < 0) { perror("per_cpu_count"); return 1; }
        hi = now_ns();
        if (count != prev) {
            if (count < prev || ne == MAX_EDGES) { fputs("counter decreased or edge capacity exceeded\n", stderr); break; }
            edges[ne++] = (struct edge){ prev_lo, hi, (uint32_t)(count - prev) };
        }
        prev_lo = lo; prev = count;
        /* about every millisecond: the mute and the rate told, the black-out; every fourth time that, the scaler's source */
        if (++round % 5 == 0 && ns < MAX_STATES) {
            struct state now = { 0 };
            now.t = hi;
            slurp(fd_win, buf, 65536);
            field(buf, "localmute", now.lm, sizeof now.lm);
            field(buf, "vFreq", now.vf, sizeof now.vf);
            slurp(fd_ovi, buf, 65536);
            field(buf, "lvds.black_out", now.bo, sizeof now.bo);
            if (round % 20 == 0) { slurp(fd_scl, buf, 65536); field(buf, "connect_src", now.src, sizeof now.src); }
            else strcpy(now.src, last.src[0] ? last.src : "-");
            if (strcmp(now.lm, last.lm) || strcmp(now.vf, last.vf) || strcmp(now.bo, last.bo) || strcmp(now.src, last.src)) {
                states[ns++] = now;
                last = now;
            }
        }
    }
    FILE *out = fopen(argv[2], "w");
    if (!out) { perror(argv[2]); return 1; }
    fprintf(out, "# clock=CLOCK_MONOTONIC irq=%s initial=%" PRIu64 " start_ns=%" PRIu64 " last_read_ns=%" PRIu64 " real_start_s=%ld\n", irq, initial, start, hi, (long)real.tv_sec);
    fputs("kind,a_ns,b_ns_or_value,c,d,e\n", out);
    for (size_t i = 0; i < ne; ++i) fprintf(out, "E,%" PRIu64 ",%" PRIu64 ",%u,,\n", edges[i].lo, edges[i].hi, edges[i].inc);
    for (size_t i = 0; i < ns; ++i) fprintf(out, "S,%" PRIu64 ",%s,%s,%s,%s\n", states[i].t, states[i].lm, states[i].vf, states[i].bo, states[i].src);
    for (int i = 0; i < marks_used && i < MAX_MARKS; ++i) fprintf(out, "K,%" PRIu64 ",,,,\n", marks[i]);
    return fclose(out) == EOF;
}
