/* ambient-probe: polls LG's libvtcapture histogram (16 picture areas straight from the video hardware) and reports
 * what comes back: per-area colour, update rate, call latency, CPU. Read-only to LG's state: it takes the capture
 * resource, reads, and always gives it back. 32-bit ARM, because LG's library is.
 *
 *   ambient-probe [-s SECS] [-r HZ] [-p PRESET | -S] [-c CSV] [-d DUMP] [-L LABEL] [-n]
 *
 *   -s SECS    run time per preset (default 10)
 *   -r HZ      polls per second (default 20, 1..240)
 *   -p PRESET  histogram preset 0..3 (default 0; 0 plain, 1 top/bottom bars skipped, 2 side bars skipped, 3 games)
 *   -S         sweep all four presets, SECS each
 *   -c CSV     one line per poll: t_ms, rc, changed, then r g b (0-255 mean, one decimal) per area
 *   -d DUMP    raw struct vth_zone arrays: the first poll of each preset, then one every 5 s (header: "VTH1",
 *              preset, areas, t_ms, then areas * 0x604 bytes)
 *   -L LABEL   free text echoed in the report
 *   -n         resolve the library and its four functions, print them, and exit without taking the capture
 *
 * Exit: always finalizes the histogram. SIGINT/SIGTERM/SIGHUP only raise a flag the poll loop checks every
 * period; SIGALRM is a watchdog (secs + 10 s, then a hard exit 5 s later); faults finalize once on a best-effort
 * basis and _exit. The library's policy callback (LG's resource manager taking the capture back) also stops
 * the run. Exit code 0 ok, 2 not allowed or no capture, 3 library problem, 4 watchdog.
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "vthist.h"

static vth_init_fn p_init;
static vth_preset_fn p_preset;
static vth_get_fn p_get;
static vth_fin_fn p_fin;

static void *g_handle;
static volatile sig_atomic_t g_stop, g_fin_done, g_policy, g_sig, g_in_fault;

static const int zones_for_preset[VTH_PRESETS] = {16, 16, 16, 14};

static uint64_t mono_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

/* ---- finalize, whatever happens ------------------------------------------------------------------------- */

static int finalize_once(void) {
    if (!g_handle || g_fin_done) return 0;
    g_fin_done = 1;
    void *h = g_handle;
    g_handle = NULL;
    return p_fin(h);
}

static void at_exit(void) { finalize_once(); }

static void on_stop(int sig) {
    g_sig = sig;
    g_stop = 1;
}

static void on_alarm(int sig) {
    (void)sig;
    if (g_stop) _exit(4); /* the stop flag was already set and the loop is still stuck: give up */
    g_stop = 1;
    alarm(5);
}

static void on_fault(int sig) {
    if (!g_in_fault) {
        g_in_fault = 1;
        if (g_handle && !g_fin_done) { /* not async-signal-safe: best effort, the process is dying anyway */
            g_fin_done = 1;
            p_fin(g_handle);
        }
    }
    _exit(128 + sig);
}

static bool policy_cb(void) { /* called from the library's thread */
    g_policy = 1;
    g_stop = 1;
    return false;
}

static void install_signals(void) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sigemptyset(&sa.sa_mask);
    sa.sa_handler = on_stop; /* no SA_RESTART: sleeps return EINTR at once */
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    sigaction(SIGPIPE, &sa, NULL);
    sa.sa_handler = on_alarm;
    sigaction(SIGALRM, &sa, NULL);
    sa.sa_handler = on_fault;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
    sigaction(SIGFPE, &sa, NULL);
    sigaction(SIGILL, &sa, NULL);
    sigaction(SIGABRT, &sa, NULL);
}

/* ---- statistics ----------------------------------------------------------------------------------------- */

static int cmp_u32(const void *a, const void *b) {
    uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
    return x < y ? -1 : x > y;
}

static uint32_t pct(uint32_t *v, size_t n, double p) { /* v sorted */
    if (!n) return 0;
    size_t i = (size_t)(p * (double)(n - 1) + 0.5);
    return v[i];
}

struct mean3 { float r, g, b; uint32_t count; };

static float chan_mean(const uint16_t *bins, uint32_t *count) {
    uint64_t sum = 0, n = 0;
    for (int i = 0; i < VTH_BINS; i++) {
        sum += (uint64_t)i * bins[i];
        n += bins[i];
    }
    *count = (uint32_t)n;
    return n ? (float)((double)sum / (double)n) : 0.0f;
}

static void zone_mean(const struct vth_zone *z, struct mean3 *m) {
    uint32_t cr, cg, cb;
    m->r = chan_mean(z->r, &cr);
    m->g = chan_mean(z->g, &cg);
    m->b = chan_mean(z->b, &cb);
    m->count = cr;
}

static float luma(const struct mean3 *m) { return 0.2126f * m->r + 0.7152f * m->g + 0.0722f * m->b; }

struct cpu_snap { double proc_s; uint64_t wall_us; uint64_t busy, total; };

static struct cpu_snap cpu_now(void) {
    struct cpu_snap s;
    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    s.proc_s = (double)ru.ru_utime.tv_sec + (double)ru.ru_stime.tv_sec + (double)(ru.ru_utime.tv_usec + ru.ru_stime.tv_usec) / 1e6;
    s.wall_us = mono_us();
    s.busy = s.total = 0;
    FILE *f = fopen("/proc/stat", "r");
    if (f) {
        unsigned long long v[8] = {0};
        if (fscanf(f, "cpu %llu %llu %llu %llu %llu %llu %llu %llu", &v[0], &v[1], &v[2], &v[3], &v[4], &v[5], &v[6], &v[7]) >= 4) {
            for (int i = 0; i < 8; i++) s.total += v[i];
            s.busy = s.total - v[3] - v[4]; /* minus idle and iowait */
        }
        fclose(f);
    }
    return s;
}

/* ---- environment facts ---------------------------------------------------------------------------------- */

static void report_holders(void) {
    DIR *d = opendir("/proc");
    if (!d) return;
    int found = 0;
    struct dirent *e;
    while ((e = readdir(d))) {
        if (e->d_name[0] < '0' || e->d_name[0] > '9') continue;
        char p[64];
        snprintf(p, sizeof p, "/proc/%s/fd", e->d_name);
        DIR *fd = opendir(p);
        if (!fd) continue;
        struct dirent *f;
        while ((f = readdir(fd))) {
            if (f->d_name[0] == '.') continue;
            char l[96], t[128];
            snprintf(l, sizeof l, "%s/%s", p, f->d_name);
            ssize_t n = readlink(l, t, sizeof t - 1);
            if (n <= 0) continue;
            t[n] = 0;
            if (!strcmp(t, "/dev/video60")) {
                char cp[64], name[64] = "?";
                snprintf(cp, sizeof cp, "/proc/%s/comm", e->d_name);
                FILE *c = fopen(cp, "r");
                if (c) {
                    if (fgets(name, sizeof name, c)) name[strcspn(name, "\n")] = 0;
                    fclose(c);
                }
                printf("holder: /dev/video60 is open in pid %s (%s)\n", e->d_name, name);
                found++;
            }
        }
        closedir(fd);
    }
    closedir(d);
    if (!found) printf("holder: nobody has /dev/video60 open\n");
}

static off_t log_size(void) {
    struct stat st;
    return stat("/var/log/messages", &st) == 0 ? st.st_size : 0;
}

/* After a failed init: the only explanation the library gives is in PmLog context "capture". */
static void print_log_since(off_t from) {
    FILE *f = fopen("/var/log/messages", "r");
    if (!f) return;
    if (fseek(f, from, SEEK_SET) != 0) { fclose(f); return; }
    char line[512];
    int shown = 0;
    while (fgets(line, sizeof line, f) && shown < 24) {
        if (strstr(line, "capture") || strstr(line, "istogram") || strstr(line, "vtCapture")) {
            fputs("log: ", stdout);
            fputs(line, stdout);
            shown++;
        }
    }
    fclose(f);
}

/* ---- the run -------------------------------------------------------------------------------------------- */

struct opts {
    int secs, hz, preset;
    bool sweep, dry;
    const char *csv, *dump, *label;
};

static void write_dump(FILE *df, int preset, int zones, uint64_t t_ms, const struct vth_zone *z) {
    if (!df) return;
    uint32_t hdr[4] = {0x31485456u /* "VTH1" */, (uint32_t)preset, (uint32_t)zones, (uint32_t)t_ms};
    fwrite(hdr, sizeof hdr, 1, df);
    fwrite(z, sizeof *z, (size_t)zones, df);
    fflush(df);
}

static int run_preset(const struct opts *o, int preset, FILE *csv, FILE *dump) {
    int rc = p_preset(g_handle, preset);
    if (rc != 0) {
        printf("setPresetHistogram(%d) returned %d\n", preset, rc);
        return rc;
    }
    int zones = zones_for_preset[preset];
    size_t cap = (size_t)o->secs * (size_t)o->hz + 64;
    uint32_t *lat = malloc(cap * sizeof *lat);
    uint32_t *gap = malloc(cap * sizeof *gap);
    struct vth_zone *buf = calloc(VTH_MAX_ZONES, sizeof *buf);
    struct vth_zone *prev = calloc(VTH_MAX_ZONES, sizeof *prev);
    if (!lat || !gap || !buf || !prev) {
        puts("out of memory");
        return 3;
    }
    size_t nlat = 0, ngap = 0;
    unsigned rc_count[32] = {0};
    unsigned long calls = 0, ok = 0, changed = 0;
    float lmin[VTH_MAX_ZONES], lmax[VTH_MAX_ZONES];
    for (int i = 0; i < VTH_MAX_ZONES; i++) { lmin[i] = 1e9f; lmax[i] = -1e9f; }
    struct mean3 last[VTH_MAX_ZONES];
    memset(last, 0, sizeof last);
    uint32_t last_count[VTH_MAX_ZONES] = {0};
    uint32_t last_peak[VTH_MAX_ZONES] = {0};
    bool have_prev = false;
    uint64_t t_start = mono_us(), t_last_change = 0, t_next = t_start, t_report = t_start + 1000000u, t_dump = t_start;
    uint64_t period_us = 1000000u / (uint64_t)o->hz;
    unsigned sec_calls = 0, sec_changed = 0;
    uint32_t sec_lat_max = 0;
    struct cpu_snap c0 = cpu_now();
    bool first = true;
    uint64_t t_end = t_start + (uint64_t)o->secs * 1000000u;
    printf("--- preset %d: %d areas, %d s at %d Hz ---\n", preset, zones, o->secs, o->hz);

    while (!g_stop && mono_us() < t_end) {
        uint64_t t0 = mono_us();
        int r = p_get(g_handle, buf);
        uint64_t t1 = mono_us();
        calls++;
        sec_calls++;
        rc_count[r < 31 ? r : 31]++;
        uint32_t dt = (uint32_t)(t1 - t0);
        if (nlat < cap) lat[nlat++] = dt;
        if (dt > sec_lat_max) sec_lat_max = dt;
        if (r == 0) {
            ok++;
            bool chg = !have_prev || memcmp(buf, prev, (size_t)zones * sizeof *buf) != 0;
            if (chg) {
                changed++;
                sec_changed++;
                if (t_last_change && ngap < cap) gap[ngap++] = (uint32_t)(t1 - t_last_change);
                t_last_change = t1;
                memcpy(prev, buf, (size_t)zones * sizeof *buf);
                have_prev = true;
            }
            for (int z = 0; z < zones; z++) {
                struct mean3 m;
                zone_mean(&buf[z], &m);
                last[z] = m;
                last_count[z] = m.count;
                uint32_t pk = 0;
                for (int i = 0; i < VTH_BINS; i++) if (buf[z].r[i] > pk) pk = buf[z].r[i];
                last_peak[z] = pk;
                float l = luma(&m);
                if (l < lmin[z]) lmin[z] = l;
                if (l > lmax[z]) lmax[z] = l;
            }
            if (csv) {
                fprintf(csv, "%llu,%d,%d", (unsigned long long)((t1 - t_start) / 1000u), r, chg);
                for (int z = 0; z < zones; z++) fprintf(csv, ",%.1f,%.1f,%.1f", last[z].r, last[z].g, last[z].b);
                fputc('\n', csv);
            }
            if (first || t1 >= t_dump) {
                write_dump(dump, preset, zones, (t1 - t_start) / 1000u, buf);
                t_dump = t1 + 5000000u;
                first = false;
            }
        } else if (csv) {
            fprintf(csv, "%llu,%d,0\n", (unsigned long long)((t1 - t_start) / 1000u), r);
        }
        if (t1 >= t_report) {
            printf("t=%3ds calls=%u changed=%u call_us_max=%u rc0=%u\n", (int)((t1 - t_start) / 1000000u), sec_calls, sec_changed,
                   sec_lat_max, rc_count[0]);
            fflush(stdout);
            sec_calls = sec_changed = 0;
            sec_lat_max = 0;
            t_report += 1000000u;
        }
        t_next += period_us;
        uint64_t now = mono_us();
        if (t_next <= now) { t_next = now; continue; } /* behind: poll again at once, no catch-up burst */
        struct timespec ts;
        uint64_t abs = t_next; /* CLOCK_MONOTONIC absolute */
        ts.tv_sec = (time_t)(abs / 1000000u);
        ts.tv_nsec = (long)(abs % 1000000u) * 1000;
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts, NULL); /* EINTR on a stop signal: the loop test ends it */
    }

    struct cpu_snap c1 = cpu_now();
    double wall = (double)(c1.wall_us - c0.wall_us) / 1e6;
    qsort(lat, nlat, sizeof *lat, cmp_u32);
    qsort(gap, ngap, sizeof *gap, cmp_u32);
    printf("summary preset %d: wall=%.2fs calls=%lu ok=%lu changed=%lu (%.1f/s) rc:", preset, wall, calls, ok, changed,
           wall > 0 ? (double)changed / wall : 0.0);
    for (int i = 0; i < 32; i++) if (rc_count[i]) printf(" %d x%u", i, rc_count[i]);
    printf("\n  call latency us: p50=%u p90=%u p99=%u max=%u\n", pct(lat, nlat, 0.5), pct(lat, nlat, 0.9), pct(lat, nlat, 0.99), nlat ? lat[nlat - 1] : 0);
    printf("  change interval us: n=%zu p50=%u p90=%u p99=%u max=%u\n", ngap, pct(gap, ngap, 0.5), pct(gap, ngap, 0.9), pct(gap, ngap, 0.99), ngap ? gap[ngap - 1] : 0);
    double dproc = c1.proc_s - c0.proc_s;
    double dsys = c1.total > c0.total ? 100.0 * (double)(c1.busy - c0.busy) / (double)(c1.total - c0.total) : 0.0;
    printf("  cpu: probe %.2f%% of one core (%.3f s in %.2f s), system busy %.1f%% over the window\n", wall > 0 ? 100.0 * dproc / wall : 0.0, dproc, wall, dsys);
    printf("  area   r      g      b    luma   count  peak  luma_min..max\n");
    for (int z = 0; z < zones; z++)
        printf("  %2d  %6.1f %6.1f %6.1f %6.1f %7u %5u  %6.1f..%.1f\n", z, last[z].r, last[z].g, last[z].b, luma(&last[z]), last_count[z], last_peak[z],
               lmin[z] > 1e8f ? 0.0f : lmin[z], lmax[z] < -1e8f ? 0.0f : lmax[z]);
    free(lat); free(gap); free(buf); free(prev);
    return 0;
}

static bool load_lib(void) {
    void *lib = dlopen("libvtcapture.so.1", RTLD_NOW);
    if (!lib) {
        printf("dlopen libvtcapture.so.1 failed: %s\n", dlerror());
        return false;
    }
    p_init = (vth_init_fn)dlsym(lib, "vtCapture_initHistogram");
    p_preset = (vth_preset_fn)dlsym(lib, "vtCapture_setPresetHistogram");
    p_get = (vth_get_fn)dlsym(lib, "vtCapture_getHistogram");
    p_fin = (vth_fin_fn)dlsym(lib, "vtCapture_finalizeHistogram");
    if (!p_init || !p_preset || !p_get || !p_fin) {
        printf("missing symbol(s): init=%p preset=%p get=%p fin=%p\n", (void *)p_init, (void *)p_preset, (void *)p_get, (void *)p_fin);
        return false;
    }
    return true;
}

static void usage(void) {
    fputs("usage: ambient-probe [-s SECS] [-r HZ] [-p PRESET | -S] [-c CSV] [-d DUMP] [-L LABEL] [-n]\n", stderr);
}

int main(int argc, char **argv) {
    struct opts o = {10, 20, 0, false, false, NULL, NULL, ""};
    int ch;
    while ((ch = getopt(argc, argv, "s:r:p:Sc:d:L:nh")) != -1) {
        switch (ch) {
        case 's': o.secs = atoi(optarg); break;
        case 'r': o.hz = atoi(optarg); break;
        case 'p': o.preset = atoi(optarg); break;
        case 'S': o.sweep = true; break;
        case 'c': o.csv = optarg; break;
        case 'd': o.dump = optarg; break;
        case 'L': o.label = optarg; break;
        case 'n': o.dry = true; break;
        default: usage(); return 1;
        }
    }
    if (o.secs < 1 || o.secs > 3600 || o.hz < 1 || o.hz > 240 || o.preset < 0 || o.preset >= VTH_PRESETS) {
        usage();
        return 1;
    }

    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("ambient-probe label=\"%s\" pid=%d secs=%d hz=%d %s\n", o.label, getpid(), o.secs, o.hz, o.sweep ? "sweep" : "single preset");
    if (!load_lib()) return 3;
    printf("library: init=%p preset=%p get=%p finalize=%p\n", (void *)p_init, (void *)p_preset, (void *)p_get, (void *)p_fin);
    struct stat st;
    printf("dev flag /var/luna/preferences/vt_devmode: %s\n", stat("/var/luna/preferences/vt_devmode", &st) == 0 ? "present" : "absent");
    report_holders();
    if (o.dry) return 0;

    install_signals();
    atexit(at_exit);
    alarm((unsigned)(o.secs * (o.sweep ? VTH_PRESETS : 1) + 10));

    FILE *csv = o.csv ? fopen(o.csv, "w") : NULL;
    FILE *dump = o.dump ? fopen(o.dump, "wb") : NULL;

    off_t log0 = log_size();
    uint64_t t0 = mono_us();
    g_handle = p_init(policy_cb);
    uint64_t t1 = mono_us();
    if (!g_handle) {
        printf("initHistogram returned NULL after %llu ms: not allowed, capture busy (code 11) or no driver\n", (unsigned long long)((t1 - t0) / 1000u));
        print_log_since(log0);
        return 2;
    }
    printf("initHistogram ok in %llu ms, handle %p\n", (unsigned long long)((t1 - t0) / 1000u), g_handle);

    int rc = 0;
    if (o.sweep) {
        for (int p = 0; p < VTH_PRESETS && !g_stop && !rc; p++) rc = run_preset(&o, p, csv, dump);
    } else {
        rc = run_preset(&o, o.preset, csv, dump);
    }

    uint64_t f0 = mono_us();
    int frc = finalize_once();
    printf("finalizeHistogram rc=%d in %llu ms%s%s\n", frc, (unsigned long long)((mono_us() - f0) / 1000u), g_policy ? " (stopped by LG policy action)" : "",
           g_sig ? " (stopped by signal)" : "");
    if (csv) fclose(csv);
    if (dump) fclose(dump);
    return rc ? 3 : (g_policy ? 2 : 0);
}
