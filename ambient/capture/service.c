/* lumatv-capture: the 32-bit colour reader of LumaTV's ambient light, a long-running native service.
 *
 * It does one thing: while the helper (64-bit, helper/) wants a picture, it polls LG's libvtcapture histogram (16 picture
 * areas straight from the video hardware, the same API LG's Hue Sync uses), reduces every poll to one colour per area
 * (ambient/src/wire.rs, "AMB1") and writes that to the helper over a local unix socket. Nothing else leaves the process:
 * no network, no file, no bus call of its own, and the helper never hands these readings to AI access or the plugin broker.
 *
 *   lumatv-capture [-s SOCKET] [-r HZ] [-i IDLE_MS] [-b BACKOFF_MS] [-n]
 *
 *   -s SOCKET      the helper's capture socket (it exists only while a picture is wanted; default $LUMATV_CAPTURE_SOCKET)
 *   -r HZ          polls a second (default 10, 1..120)
 *   -i IDLE_MS     how often to look for the socket while nobody wants a picture (default 2000; one connect(), nothing
 *                  else runs: the library is not even loaded until the first connection)
 *   -b BACKOFF_MS  the wait after a capture that could not start or broke off (default 5000)
 *   -n             resolve the library's four functions, print them, exit
 *
 * Lifecycle (docs/research/ambient-result.md, "Helper-side design"):
 *   - idle: no socket, no library call, no bus connection, no /dev/video60;
 *   - the helper accepts, and tells the hardware preset first (one byte, 0..3; more bytes later when the picture changes
 *     from letterbox to full screen): initHistogram, setPresetHistogram, then a poll every period;
 *   - the capture is given back (finalizeHistogram) the moment the helper closes the connection, LG's resource manager takes
 *     it back (the policy callback), the polls fail for two seconds, or the process is stopped;
 *   - initHistogram returning NULL (LG's gate wants the developer flag or Hue Sync's command line; the bus wants a role file
 *     for this executable; the scaler is busy) is logged with the lines LG's library left in /var/log/messages, the
 *     connection is closed, and the next try comes after the back-off.
 *
 * The flag /var/luna/preferences/vt_devmode is never created here: that is the owner's act (docs/PLAN.md, Decided). The LS2
 * role that lets this executable register the library's bus client is installed with the package (payload/capture/).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#include "reduce.h"
#include "vthist.h"

/* ambient/src/wire.rs */
#define WIRE_HEADER 12
#define WIRE_ZONE 12
#define WIRE_MAX (WIRE_HEADER + VTH_MAX_ZONES * WIRE_ZONE)

static vth_init_fn p_init;
static vth_preset_fn p_preset;
static vth_get_fn p_get;
static vth_fin_fn p_fin;
static int g_lib_loaded;

static void *g_handle;
static volatile sig_atomic_t g_stop, g_fin_done, g_policy, g_in_fault;

static const int zones_for_preset[VTH_PRESETS] = {16, 16, 16, 14};

enum end { END_CLOSED, END_INIT, END_POLICY, END_FAILED, END_STOP };

static uint64_t cpu_ns(clockid_t id) {
    struct timespec ts;
    return clock_gettime(id, &ts) == 0 ? (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec : 0;
}

static uint64_t mono_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

/* ---- log: state changes only, one line each; the same line is not repeated inside ten seconds ------------------------- */

static void say(const char *fmt, ...) {
    static char last[256];
    static uint64_t last_us;
    char line[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    uint64_t now = mono_us();
    if (strcmp(line, last) == 0 && now - last_us < 10000000u) return;
    snprintf(last, sizeof last, "%s", line);
    last_us = now;
    time_t t = time(NULL);
    struct tm tm;
    char stamp[16] = "";
    if (localtime_r(&t, &tm)) strftime(stamp, sizeof stamp, "%H:%M:%S", &tm);
    // The log is appended to by this process for as long as it runs (capture.sh rotates it at a start only), and a failing session
    // writes about nine lines every backoff: past this much the file is emptied (a no-op where stderr is not a file).
    static size_t wrote;
    wrote += strlen(stamp) + strlen(line) + 18;
    if (wrote > 256 * 1024) {
        if (ftruncate(STDERR_FILENO, 0) != 0) { /* not a file */ }
        wrote = 0;
    }
    fprintf(stderr, "%s lumatv-capture: %s\n", stamp, line);
    fflush(stderr);
}

static off_t log_size(void) {
    struct stat st;
    return stat("/var/log/messages", &st) == 0 ? st.st_size : 0;
}

/* What LG's library (context "capture") and ls-hubd said while the call ran: the only place the reason is. */
static void say_log_since(off_t from) {
    FILE *f = fopen("/var/log/messages", "r");
    if (!f) return;
    if (fseeko(f, from, SEEK_SET) == 0) {
        char line[512];
        for (int shown = 0; shown < 8 && fgets(line, sizeof line, f);) {
            if (strstr(line, "capture") || strstr(line, "LSHUB") || strstr(line, "vtCapture")) {
                line[strcspn(line, "\n")] = 0;
                say("  log: %.200s", line);
                shown++;
            }
        }
    }
    fclose(f);
}

/* ---- the library, and giving the capture back whatever happens ----------------------------------------------------- */

static int load_lib(void) {
    if (g_lib_loaded) return 1;
    void *lib = dlopen("libvtcapture.so.1", RTLD_NOW);
    if (!lib) {
        say("dlopen libvtcapture.so.1 failed: %s", dlerror());
        return 0;
    }
    p_init = (vth_init_fn)dlsym(lib, "vtCapture_initHistogram");
    p_preset = (vth_preset_fn)dlsym(lib, "vtCapture_setPresetHistogram");
    p_get = (vth_get_fn)dlsym(lib, "vtCapture_getHistogram");
    p_fin = (vth_fin_fn)dlsym(lib, "vtCapture_finalizeHistogram");
    if (!p_init || !p_preset || !p_get || !p_fin) {
        say("libvtcapture.so.1 lacks a histogram function");
        return 0;
    }
    g_lib_loaded = 1;
    return 1;
}

static int finalize_once(void) {
    if (!g_handle || g_fin_done) return 0;
    g_fin_done = 1;
    void *h = g_handle;
    g_handle = NULL;
    return p_fin(h);
}

static bool policy_cb(void) { /* called from the library's own thread: LG's resource manager wants the capture back */
    g_policy = 1;
    return false;
}

static void on_stop(int sig) {
    (void)sig;
    g_stop = 1;
}

static void on_alarm(int sig) { /* the watchdog: re-armed every turn of every loop, so it fires only when a call hangs */
    (void)sig;
    if (g_stop) _exit(4);
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

static void install_signals(void) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sigemptyset(&sa.sa_mask);
    sa.sa_handler = on_stop; /* no SA_RESTART: poll and nanosleep return EINTR at once */
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    sa.sa_handler = SIG_IGN;
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

static void nap_ms(unsigned ms) { /* interrupted by a signal: the loops test g_stop */
    struct timespec ts = {(time_t)(ms / 1000u), (long)(ms % 1000u) * 1000000L};
    alarm(30);
    nanosleep(&ts, NULL);
}

/* ---- one poll, reduced to the helper's frame ------------------------------------------------------------------------ */

static void put16(uint8_t *p, unsigned v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
}

static void put32(uint8_t *p, uint32_t v) {
    put16(p, v & 0xffffu);
    put16(p + 2, v >> 16);
}

static size_t build_frame(uint8_t *out, const struct vth_zone *z, int preset, uint32_t t_ms) {
    int n = zones_for_preset[preset];
    struct zone_out red[VTH_MAX_ZONES];
    reduce_zones(z, n, red);
    memcpy(out, "AMB1", 4);
    out[4] = (uint8_t)preset;
    out[5] = (uint8_t)n;
    out[6] = out[7] = 0;
    put32(out + 8, t_ms);
    memset(out + WIRE_HEADER, 0, (size_t)n * WIRE_ZONE);
    for (int i = 0; i < n; i++) {
        /* the library numbers the areas; a number outside the preset is read as the position */
        unsigned id = z[i].area_id < (unsigned)n ? z[i].area_id : (unsigned)i;
        uint8_t *o = out + WIRE_HEADER + (size_t)id * WIRE_ZONE;
        put16(o, red[i].mean[0]);
        put16(o + 2, red[i].mean[1]);
        put16(o + 4, red[i].mean[2]);
        put16(o + 6, red[i].black);
        put32(o + 8, red[i].count);
    }
    return (size_t)WIRE_HEADER + (size_t)n * WIRE_ZONE;
}

/* ---- the connection to the helper ----------------------------------------------------------------------------------- */

static int connect_helper(const char *path) {
    struct sockaddr_un a;
    if (strlen(path) >= sizeof a.sun_path) return -1;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    memset(&a, 0, sizeof a);
    a.sun_family = AF_UNIX;
    strcpy(a.sun_path, path);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) != 0) {
        close(fd);
        return -1;
    }
    struct timeval tv = {0, 200000}; /* a helper that stops reading cannot stall the poll loop for long */
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    return fd;
}

/* One capture for as long as the helper wants it. Returns why it ended. */
static enum end session(int fd, int hz) {
    int preset = 0;
    struct pollfd pfd = {fd, POLLIN, 0};
    /* the helper tells the preset first; a helper that has nothing to say yet means preset 0 */
    if (poll(&pfd, 1, 1000) > 0 && (pfd.revents & (POLLIN | POLLHUP))) {
        unsigned char b[16];
        ssize_t n = recv(fd, b, sizeof b, 0);
        if (n == 0) return END_CLOSED;
        for (ssize_t i = 0; i < n; i++) if (b[i] < VTH_PRESETS) preset = b[i];
    }
    if (g_stop) return END_STOP;
    if (!load_lib()) return END_INIT;

    g_policy = 0;
    g_fin_done = 0;
    off_t log0 = log_size();
    uint64_t t0 = mono_us();
    alarm(30);
    g_handle = p_init(policy_cb);
    if (!g_handle) {
        say("initHistogram returned NULL after %llu ms: not allowed (the developer flag or Hue Sync's command line), "
            "no bus role for this executable, or the scaler is busy", (unsigned long long)((mono_us() - t0) / 1000u));
        say_log_since(log0);
        return END_INIT;
    }
    int rc = p_preset(g_handle, preset);
    if (rc != 0) {
        say("setPresetHistogram(%d) returned %d", preset, rc);
        finalize_once();
        return END_FAILED;
    }
    say("capturing: preset %d, %d areas, %d polls a second (init %llu ms, reduce %s)", preset, zones_for_preset[preset], hz,
        (unsigned long long)((mono_us() - t0) / 1000u), reduce_impl());

    struct vth_zone *buf = calloc(VTH_MAX_ZONES, sizeof *buf);
    if (!buf) {
        finalize_once();
        return END_FAILED;
    }
    uint8_t frame[WIRE_MAX];
    uint64_t period = 1000000u / (uint64_t)hz, next = mono_us(), bad_since = 0;
    enum end why = END_CLOSED;
    /* What a poll costs, for the capture's last line: process CPU over the whole capture (the library's own threads
     * included) and this thread's CPU inside each of the three steps of a poll. */
    uint64_t cost_proc0 = cpu_ns(CLOCK_PROCESS_CPUTIME_ID), cost_wall0 = mono_us();
    uint64_t cost_get = 0, cost_reduce = 0, cost_send = 0, cost_get_wall = 0;
    unsigned polls = 0, frames = 0;
    while (!g_stop && !g_policy) {
        alarm(30);
        uint64_t now = mono_us();
        if (now < next) { /* the helper's control bytes are heard while waiting for the next poll */
            int wait = (int)((next - now + 999u) / 1000u);
            pfd.revents = 0;
            int r = poll(&pfd, 1, wait);
            if (r < 0 && errno != EINTR) { why = END_FAILED; break; }
            if (r > 0) {
                unsigned char b[16];
                ssize_t n = recv(fd, b, sizeof b, MSG_DONTWAIT);
                if (n == 0 || (n < 0 && errno != EAGAIN && errno != EINTR)) { why = END_CLOSED; break; }
                int want = preset;
                for (ssize_t i = 0; i < n; i++) if (b[i] < VTH_PRESETS) want = b[i];
                if (want != preset) {
                    if (p_preset(g_handle, want) != 0) { say("setPresetHistogram(%d) failed", want); why = END_FAILED; break; }
                    say("preset %d -> %d", preset, want);
                    preset = want;
                }
            }
            continue;
        }
        next += period;
        if (next <= now) next = now + period; /* behind: poll at once, never a catch-up burst */

        uint64_t c0 = cpu_ns(CLOCK_THREAD_CPUTIME_ID), w0 = mono_us();
        int r = p_get(g_handle, buf);
        uint64_t c1 = cpu_ns(CLOCK_THREAD_CPUTIME_ID);
        cost_get += c1 - c0;
        cost_get_wall += mono_us() - w0;
        polls++;
        if (r == 0) {
            bad_since = 0;
            size_t len = build_frame(frame, buf, preset, (uint32_t)(mono_us() / 1000u));
            uint64_t c2 = cpu_ns(CLOCK_THREAD_CPUTIME_ID);
            ssize_t sent = send(fd, frame, len, MSG_NOSIGNAL);
            cost_reduce += c2 - c1;
            cost_send += cpu_ns(CLOCK_THREAD_CPUTIME_ID) - c2;
            frames++;
            if (sent != (ssize_t)len) { why = END_CLOSED; break; }
        } else if (r == 11 || r == 13) { /* EAGAIN, ENODATA: nothing new, or no signal on the input right now */
            bad_since = 0;
        } else {
            if (!bad_since) bad_since = now;
            if (now - bad_since > 2000000u) { say("getHistogram failed with %d for two seconds", r); why = END_FAILED; break; }
        }
    }
    free(buf);
    if (polls) {
        double wall_s = (double)(mono_us() - cost_wall0) / 1e6, proc_ms = (double)(cpu_ns(CLOCK_PROCESS_CPUTIME_ID) - cost_proc0) / 1e6;
        say("cost: %u polls (%u frames) in %.1f s, process CPU %.1f ms = %.2f %% of a core, %.3f ms a poll; per poll: "
            "getHistogram %.3f ms CPU (%.3f ms wall), reduce %.3f ms, send %.3f ms",
            polls, frames, wall_s, proc_ms, wall_s > 0 ? proc_ms / 10.0 / wall_s : 0.0, proc_ms / polls,
            (double)cost_get / 1e6 / polls, (double)cost_get_wall / 1e3 / polls, frames ? (double)cost_reduce / 1e6 / frames : 0.0,
            frames ? (double)cost_send / 1e6 / frames : 0.0);
    }
    if (g_policy) why = END_POLICY;
    else if (g_stop) why = END_STOP;
    int frc = finalize_once();
    say("capture ended (%s), finalize rc=%d", why == END_CLOSED ? "the helper no longer wants a picture" : why == END_POLICY ? "LG's resource manager took it back"
        : why == END_STOP ? "stopped" : "failed", frc);
    return why;
}

static void usage(void) {
    fputs("usage: lumatv-capture [-s SOCKET] [-r HZ] [-i IDLE_MS] [-b BACKOFF_MS] [-n]\n", stderr);
}

int main(int argc, char **argv) {
    const char *socket_path = getenv("LUMATV_CAPTURE_SOCKET");
    int hz = 10, idle_ms = 2000, backoff_ms = 5000, dry = 0, ch;
    while ((ch = getopt(argc, argv, "s:r:i:b:nh")) != -1) {
        switch (ch) {
        case 's': socket_path = optarg; break;
        case 'r': hz = atoi(optarg); break;
        case 'i': idle_ms = atoi(optarg); break;
        case 'b': backoff_ms = atoi(optarg); break;
        case 'n': dry = 1; break;
        default: usage(); return 1;
        }
    }
    if (socket_path && !*socket_path) socket_path = NULL;
    if (hz < 1 || hz > 120 || idle_ms < 10 || backoff_ms < 10 || (!dry && !socket_path)) {
        usage();
        return 1;
    }
    if (dry) {
        if (!load_lib()) return 3;
        printf("library: init=%p preset=%p get=%p finalize=%p\n", (void *)p_init, (void *)p_preset, (void *)p_get, (void *)p_fin);
        return 0;
    }
    install_signals();
    say("waiting for %s (%d polls a second when a picture is wanted)", socket_path, hz);
    while (!g_stop) {
        alarm(30);
        int fd = connect_helper(socket_path);
        if (fd < 0) {
            nap_ms((unsigned)idle_ms);
            continue;
        }
        enum end why = session(fd, hz);
        close(fd);
        finalize_once();
        switch (why) {
        case END_CLOSED: nap_ms(50); break;                       /* the helper may want a picture again at once */
        case END_POLICY: nap_ms(2000 < (unsigned)backoff_ms ? 2000 : (unsigned)backoff_ms); break;
        case END_STOP: break;
        default: nap_ms((unsigned)backoff_ms); break;
        }
    }
    finalize_once();
    return 0;
}
