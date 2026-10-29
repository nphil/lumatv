// lumatv-sm: the process systemd starts as /usr/bin/surface-manager once LumaTV's compositor is live (track C4).
// It is 32-bit and dynamically linked against the set's own libluna-service2 and glib, because ls-hubd grants the
// compositor's bus names to the executable at that path and a 64-bit process cannot load LG's 32-bit libraries.
// Four duties (docs/research/compositor-c2-result.md §5 and §8, docs/research/cutover-result.md):
//
//   1. Decide (decide()). LumaTV runs only if the enabled flag is a persistent one or a session of this boot, the
//      bundle is whole, no escape is recorded, the firmware fingerprint equals the one `install` recorded, a persistent
//      enable also finds that fingerprint on verified.list (a build the gate passed on), and fewer than 2 consecutive
//      failures are on record (a start whose predecessor never settled or ended cleanly counts as one). Otherwise,
//      and on any later failure, fall back: remove the bind mount that put this file at /usr/bin/surface-manager and
//      exec LG's compositor at the same path, so the unit, the role and the exe name stay LG's. The unit's main
//      process becomes LG's compositor in place: nothing is restarted, and SAM is never touched.
//   2. Supervise. Spawn the 64-bit compositor through its private loader on LG's socket (/tmp/xdg/wayland-0),
//      heartbeat it over the bridge socket every 500 ms and fall back when it exits, misses two beats, draws frames
//      the panel never shows (6 s) or leaves no window mapped (45 s) while the set is on. The compositor can ask for
//      the fallback itself (the hold-Home escape), which also records the escape.
//   3. Hold Home. Independently of the compositor, read the remotes' evdev nodes: Home held 650 ms, or still held when
//      this process starts (a boot with Home held down), is the same escape. The way back to LG's therefore works
//      while the compositor is starting, wedged or blind to its input.
//   4. Bridge luna. Register the compositor's bus names, forward every call as a JSON line, send the compositor's
//      replies (subscriptions kept by token) and make its outbound calls.
//
// JSON lines on the bridge socket (compositor/crates/lumatv-compositor/src/luna.rs):
//   to the compositor:   {"kind":"inbound","token":N,"service":S,"method":M,"payload":P,"subscribe":B,"sender":S}
//                        {"kind":"inbound","token":N,...,"cancelled":true}   {"kind":"event","token":N,"payload":P}
//                        {"kind":"ping","seq":N}
//   from the compositor: {"kind":"reply","token":N,"payload":P}   {"kind":"outbound","token":N,"uri":U,"payload":P,"subscribe":B}
//                        {"kind":"cancel","token":N}   {"kind":"pong","seq":N}   {"kind":"fallback","reason":R}
//
// State files in --state DIR (default /var/lib/lumatv/compositor): enabled (empty or "persistent": every boot;
// "session <boot id>": this boot only), escaped, failures, fingerprint (recorded by install), verified.list (builds
// the gate passed on: "fingerprint <hex>" lines), running (written once a start is decided, removed after 120 s
// healthy or on SIGTERM or a deliberate fallback: a file found at start means the last run was cut short and counts
// as a failure), last-fallback, sm.log.
//
// Build: compositor/sm/build.sh (zig cc, arm-linux-gnueabi glibc 2.28, stub libraries for the link; the set's
// libraries at run time). Usage: lumatv-sm [--dry-run|--probe] [--state DIR] [--print-fingerprint]
//                                          [--would-run] [--home-held [MS]]
//   --dry-run   never touches the mount or execs LG's binary; registers the bus names with a ".lumatv" suffix and
//               runs the compositor offscreen on the private socket, for a test of the bridge beside LG's compositor
//               (the hub refuses the suffixed names, since no role file allows them: that part proves nothing).
//   --probe     asks the hub for the real names and exits: run from the real path (bind-mounted, LG's compositor
//               still running) it tells the role check from the name being taken, because ls-hubd checks the role
//               before it checks for a duplicate (luna-service2 hub.cpp _LSHubHandleRequestName). Exit 0 = every
//               name is allowed to this executable path (all answered "already exists"), 2 = at least one refused.
//   --print-fingerprint prints the firmware fingerprint the enable step records.
//   --would-run read-only: prints "run" (exit 0) or "no WORD: reason" (exit 3) as decide() would answer now. The early
//               boot hook asks it before it places the mount, so the decision has one implementation; any other
//               exit means this binary cannot run on this firmware (missing library, wrong ABI) and no mount is made.
//   --home-held [MS] exit 0 when a remote's Home key is down now and stays down for MS (default 700), else 1.
// The unit is Type=notify: READY=1 goes to systemd's socket once the compositor is connected (else the start
// times out and the unit is restarted), and /tmp/lsm-ready is touched, which is what webapp-mgr.service waits for.
#define _GNU_SOURCE
#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/input.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

// ---- luna-service2 (public API, webosose/luna-service2 lunaservice.h) ----
typedef struct LSHandle LSHandle;
typedef struct LSMessage LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError { int error_code; char *message; const char *file; int line; const char *func; void *padding; unsigned long magic; } LSError;
typedef bool (*LSMethodFunction)(LSHandle *sh, LSMessage *msg, void *category_context);
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef struct { const char *name; LSMethodFunction function; int flags; } LSMethod;
typedef struct GMainLoop GMainLoop;
bool LSErrorInit(LSError *e);
void LSErrorFree(LSError *e);
bool LSRegister(const char *name, LSHandle **sh, LSError *e);
bool LSUnregister(LSHandle *sh, LSError *e);
bool LSRegisterCategory(LSHandle *sh, const char *category, LSMethod *methods, void *signals, void *properties, LSError *e);
bool LSCategorySetData(LSHandle *sh, const char *category, void *user_data, LSError *e);
bool LSGmainAttach(LSHandle *sh, GMainLoop *loop, LSError *e);
bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e);
bool LSCallOneReply(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e);
bool LSCallCancel(LSHandle *sh, LSMessageToken token, LSError *e);
void LSMessageRef(LSMessage *m);
void LSMessageUnref(LSMessage *m);
const char *LSMessageGetPayload(LSMessage *m);
const char *LSMessageGetMethod(LSMessage *m);
const char *LSMessageGetSender(LSMessage *m);
const char *LSMessageGetSenderServiceName(LSMessage *m);
const char *LSMessageGetUniqueToken(LSMessage *m);
bool LSMessageIsSubscription(LSMessage *m);
bool LSMessageReply(LSHandle *sh, LSMessage *m, const char *payload, LSError *e);
bool LSSubscriptionAdd(LSHandle *sh, const char *key, LSMessage *m, LSError *e);
bool LSSubscriptionReply(LSHandle *sh, const char *key, const char *payload, LSError *e);
bool LSSubscriptionSetCancelFunction(LSHandle *sh, LSFilterFunc cancel, void *ctx, LSError *e);

// ---- glib (only what the loop needs) ----
typedef struct GIOChannel GIOChannel;
typedef int gboolean;
typedef void *gpointer;
typedef gboolean (*GSourceFunc)(gpointer);
typedef gboolean (*GIOFunc)(GIOChannel *, int condition, gpointer);
typedef void (*GChildWatchFunc)(int pid, int status, gpointer);
enum { G_IO_IN = 1, G_IO_PRI = 2, G_IO_OUT = 4, G_IO_ERR = 8, G_IO_HUP = 16 };
GMainLoop *g_main_loop_new(void *context, gboolean running);
void g_main_loop_run(GMainLoop *loop);
void g_main_loop_quit(GMainLoop *loop);
GIOChannel *g_io_channel_unix_new(int fd);
unsigned g_io_add_watch(GIOChannel *ch, int condition, GIOFunc fn, gpointer data);
unsigned g_timeout_add(unsigned ms, GSourceFunc fn, gpointer data);
unsigned g_child_watch_add(int pid, GChildWatchFunc fn, gpointer data);

// ---- configuration ----
static const char *state_dir = "/var/lib/lumatv/compositor";
static const char *lg_binary = "/usr/bin/surface-manager";
static const char *runtime_dir = "/tmp/xdg";
static const char *socket_name = "wayland-0";
static const char *input_dir = "/dev/input";
static const char *boot_id_path = "/proc/sys/kernel/random/boot_id";
static bool dry_run = false, probe = false;
static bool supervising = false;
static char **saved_argv;
static GMainLoop *loop;
static pid_t child_pid = 0;
static int bridge_listen = -1, bridge_fd = -1;
static unsigned long ping_seq = 0, pong_seq = 0;
static unsigned long inbound_token = 0;
static double started_mono;
static char running_path[600];
static FILE *logf;

static void logline(const char *fmt, ...) {
    char ts[32];
    time_t t = time(NULL);
    strftime(ts, sizeof ts, "%H:%M:%S", localtime(&t));
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "[lumatv-sm %s] ", ts);
    vfprintf(stderr, fmt, ap);
    fprintf(stderr, "\n");
    va_end(ap);
    if (logf) {
        va_start(ap, fmt);
        fprintf(logf, "[%s] ", ts);
        vfprintf(logf, fmt, ap);
        fprintf(logf, "\n");
        fflush(logf);
        va_end(ap);
    }
}

static char *path_in_state(const char *name) {
    static char buf[4][512];
    static int k;
    char *b = buf[k++ & 3];
    snprintf(b, 512, "%s/%s", state_dir, name);
    return b;
}

static bool file_exists(const char *p) { struct stat st; return stat(p, &st) == 0; }

static long read_number(const char *p, long dflt) {
    FILE *f = fopen(p, "r");
    if (!f) return dflt;
    long v = dflt;
    if (fscanf(f, "%ld", &v) != 1) v = dflt;
    fclose(f);
    return v;
}

// The refresh test mode (the compositor's `refresh.rs`, `ctl refresh 120`) sets the OSD's vblank divider to 1 and puts LG's 2 back
// when it exits; a compositor killed mid-run leaves 1, and LG's compositor would then get a page flip at every panel refresh.
#define VSYNC_DIVIDER "/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip"
static void restore_vsync_divider(void) {
    if (read_number(VSYNC_DIVIDER, 2) == 2) return;
    FILE *f = fopen(VSYNC_DIVIDER, "w");
    if (!f) return;
    fputs("2\n", f);
    fclose(f);
    logline("fallback: %s put back to 2 (the refresh test mode left 1)", VSYNC_DIVIDER);
}

static void write_text(const char *p, const char *text) {
    char tmp[600];
    snprintf(tmp, sizeof tmp, "%s.tmp", p);
    FILE *f = fopen(tmp, "w");
    if (!f) return;
    fputs(text, f);
    fclose(f);
    rename(tmp, p);
}

static void write_number(const char *p, long v) {
    char s[32];
    snprintf(s, sizeof s, "%ld\n", v);
    write_text(p, s);
}

// ---- fingerprint: the firmware files a replacement compositor depends on ----
// FNV-1a over "path:size:mtime" lines of the files LG could change in an update: the GPU driver, LG's Wayland
// server library, the compositor core and the release file. The compositor binary itself is checked by the early
// boot hook before it places the bind mount, since this process cannot see through the mount.
static const char *fingerprint_files[] = {
    "/etc/starfish-release",
    "/usr/lib/libmali.so.0.46.0",
    "/usr/lib/libwayland-webos-server.so.1.0.0",
    "/usr/lib/libWebOSCoreCompositor.so.0.0.1",
    "/usr/lib/libwayland-server.so.0",
    "/usr/lib/libluna-service2.so.3",
    NULL,
};

static void fingerprint(char out[17]) {
    uint64_t h = 1469598103934665603ULL;
    char line[600];
    for (int i = 0; fingerprint_files[i]; i++) {
        struct stat st;
        if (stat(fingerprint_files[i], &st) != 0) snprintf(line, sizeof line, "%s:missing\n", fingerprint_files[i]);
        else snprintf(line, sizeof line, "%s:%lld:%lld\n", fingerprint_files[i], (long long)st.st_size, (long long)st.st_mtime);
        for (const char *p = line; *p; p++) { h ^= (unsigned char)*p; h *= 1099511628211ULL; }
        if (!strcmp(fingerprint_files[i], "/etc/starfish-release")) {
            FILE *f = fopen(fingerprint_files[i], "r");
            if (f) { int c; while ((c = fgetc(f)) != EOF) { h ^= (unsigned char)c; h *= 1099511628211ULL; } fclose(f); }
        }
    }
    snprintf(out, 17, "%016llx", (unsigned long long)h);
}

// ---- clocks, state files and the start-up decision ----
// The wall clock is unset for the first seconds of a boot (systemd stamps early units in 2022) and jumps when time
// syncs, so every interval here is measured on CLOCK_MONOTONIC.
static double mono_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static bool read_first_line(const char *p, char *out, size_t n) {
    out[0] = 0;
    FILE *f = fopen(p, "r");
    if (!f) return false;
    bool ok = fgets(out, (int)n, f) != NULL;
    fclose(f);
    if (!ok) { out[0] = 0; return false; }
    out[strcspn(out, "\r\n")] = 0;
    return true;
}

static void current_boot_id(char *out, size_t n) {
    if (!read_first_line(boot_id_path, out, n) || !out[0]) snprintf(out, n, "unknown");
}

enum flag_kind { FLAG_OFF, FLAG_PERSISTENT, FLAG_SESSION, FLAG_STALE };

// The enabled flag. An empty file or "persistent" (what `touch enabled` made before sessions existed) holds across
// boots; "session <boot id>" holds in the boot it was made in only, so a gate or trial that was cut short can never
// turn into a boot-time enable.
static enum flag_kind read_flag(void) {
    char line[200];
    if (!file_exists(path_in_state("enabled"))) return FLAG_OFF;
    read_first_line(path_in_state("enabled"), line, sizeof line);
    if (!strncmp(line, "session", 7) && (line[7] == ' ' || line[7] == 0)) {
        char id[64];
        current_boot_id(id, sizeof id);
        return strcmp(line[7] ? line + 8 : "", id) == 0 ? FLAG_SESSION : FLAG_STALE;
    }
    return FLAG_PERSISTENT;
}

// verified.list is what the gate passed on: blocks written by `cutover.sh firmware record`, of which only the
// "fingerprint <hex>" lines matter here (the hashes are checked by the shell side, which can read LG's own binary).
static bool verified_has(const char *fp) {
    FILE *f = fopen(path_in_state("verified.list"), "r");
    if (!f) return false;
    char line[256];
    bool found = false;
    while (!found && fgets(line, sizeof line, f)) {
        char tok[64] = "";
        if (!strncmp(line, "fingerprint ", 12) && sscanf(line + 12, "%63s", tok) == 1 && !strcmp(tok, fp)) found = true;
    }
    fclose(f);
    return found;
}

// One failure more than the file says while the last run's marker is still there: that run never settled or ended
// cleanly (power cut, kill, hang). A power cut on a black screen is the case this exists for.
static long failures_effective(void) {
    return read_number(path_in_state("failures"), 0) + (file_exists(path_in_state("running")) ? 1 : 0);
}

// At a real start: a leftover marker is counted for good and removed.
static void consume_unclean(void) {
    if (!file_exists(path_in_state("running"))) return;
    long f = read_number(path_in_state("failures"), 0) + 1;
    write_number(path_in_state("failures"), f);
    unlink(path_in_state("running"));
    logline("the previous run never settled or ended cleanly (power cut, kill or hang): failures now %ld", f);
}

static void mark_running(void) {
    char id[64], line[128];
    current_boot_id(id, sizeof id);
    snprintf(line, sizeof line, "%s %ld\n", id, (long)time(NULL));
    write_text(running_path, line);
}

static void forget_running(void) {
    if (running_path[0]) unlink(running_path);
}

// What a recovery after a fallback needs, and what the compositor cannot start without: all of it is checked here
// rather than found out by a crash.
static const char *bundle_files[] = { "bin/lumatv-compositor", "bin/recover-home.sh", "lib/ld-linux-aarch64.so.1", "lib/libmali-bifrost-g52-g24p0-wayland-gbm.so", NULL };

static const char *bundle_missing(void) {
    for (int i = 0; bundle_files[i]; i++) if (!file_exists(path_in_state(bundle_files[i]))) return bundle_files[i];
    return NULL;
}

struct verdict { bool run; char word[16]; char why[200]; };

static void verdict_no(struct verdict *v, const char *word, const char *fmt, ...) {
    v->run = false;
    snprintf(v->word, sizeof v->word, "%s", word);
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(v->why, sizeof v->why, fmt, ap);
    va_end(ap);
}

// Whether LumaTV may run now. Read-only (a leftover `running` marker counts as a failure here without being
// consumed), so the early boot hook can ask before it places the mount; the real start asks the same function.
static void decide(struct verdict *v) {
    char fp[17], want[64] = "";
    fingerprint(fp);
    FILE *f = fopen(path_in_state("fingerprint"), "r");
    if (f) { if (fscanf(f, "%63s", want) != 1) want[0] = 0; fclose(f); }
    enum flag_kind flag = read_flag();
    v->run = true;
    snprintf(v->word, sizeof v->word, "run");
    v->why[0] = 0;
    if (flag == FLAG_OFF) { verdict_no(v, "off", "not enabled"); return; }
    if (flag == FLAG_STALE) { verdict_no(v, "session", "the enable was a session of an earlier boot"); return; }
    if (file_exists(path_in_state("escaped"))) { verdict_no(v, "escaped", "escape recorded; re-enable from the tool"); return; }
    const char *missing = bundle_missing();
    if (missing) { verdict_no(v, "bundle", "the bundle is incomplete: %s is missing", missing); return; }
    if (strcmp(fp, want) != 0) { verdict_no(v, "firmware", "firmware fingerprint differs from the verified one"); return; }
    if (flag == FLAG_PERSISTENT && !verified_has(fp)) { verdict_no(v, "firmware", "firmware build is not on the verified list"); return; }
    if (failures_effective() >= 2) { verdict_no(v, "failures", "two consecutive failures"); return; }
}

// ---- Home held (the escape's second reader) ----
// The compositor's own escape (input.rs, 650 ms) needs the compositor running and reading the remote. This one
// needs neither: evdev nodes opened here, so a boot with Home held down, a compositor still starting and one
// wedged before it reads any key all end the same way. The network key devices (the SSAP keys of the TV's own app
// and of automations) are left out: this is a person's escape, at the TV.
#define HOLD_HOME_SECS 0.65
#define MAX_INPUTS 32

struct home { double down_at; };    // 0 while no Home key is down
static struct home home_state;

// LG's RF_HOME (network-input-service's key table): the code PlainHome reads as the Magic Remote's Home and perf/kb.py injects as Home
// through the SSAP key device; the kernel headers do not name it.
#define KEY_RF_HOME 773
static bool is_home_key(int code) { return code == KEY_HOME || code == KEY_HOMEPAGE || code == KEY_RF_HOME; }

// A key event of a remote: press and repeat start the hold, release ends it.
static void home_event(struct home *h, int code, int value, double now) {
    if (!is_home_key(code)) return;
    if (value != 0) { if (h->down_at == 0) h->down_at = now; }
    else h->down_at = 0;
}

static bool home_due(const struct home *h, double now) { return h->down_at != 0 && now - h->down_at >= HOLD_HOME_SECS; }

static bool name_is_network_keys(const char *name) {
    return strstr(name, "Smart Remote RCU Input") != NULL || strstr(name, "LGE Network Input") != NULL;
}

// The event node, read-only and non-blocking; -1 for one that does not open or is a network key device.
static int open_input(const char *path) {
    int fd = open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) return -1;
    char name[96] = "";
    if (ioctl(fd, EVIOCGNAME(sizeof name - 1), name) >= 0 && name_is_network_keys(name)) { close(fd); return -1; }
    return fd;
}

// Whether a Home key's bit is set in a key-state bitmap of `n` bytes (the answer of EVIOCGKEY): a bit past the answer's end is not read.
static bool home_bits_set(const unsigned char *bits, size_t n) {
    static const int homes[] = { KEY_HOME, KEY_HOMEPAGE, KEY_RF_HOME };
    for (size_t i = 0; i < sizeof homes / sizeof homes[0]; i++) {
        size_t byte = (size_t)homes[i] / 8;
        if (byte < n && ((bits[byte] >> (homes[i] % 8)) & 1)) return true;
    }
    return false;
}

// Whether a Home key is down on this device right now (the kernel keeps the state of every key it saw pressed). The buffer covers
// 1024 key codes, more than the header's KEY_MAX: LG's RF_HOME (773) lies above the mainline limit, and the kernel copies no more
// than it holds.
#define KEY_STATE_BYTES 128
_Static_assert(KEY_STATE_BYTES * 8 > KEY_RF_HOME && KEY_STATE_BYTES * 8 > KEY_HOMEPAGE, "the key-state buffer must reach every Home code");
static bool input_home_down(int fd) {
    unsigned char bits[KEY_STATE_BYTES];
    memset(bits, 0, sizeof bits);
    if (ioctl(fd, EVIOCGKEY(sizeof bits), bits) < 0) return false;
    return home_bits_set(bits, sizeof bits);
}

static int open_inputs_all(int *fds, int n) {
    DIR *d = opendir(input_dir);
    if (!d) return 0;
    int got = 0;
    struct dirent *e;
    while (got < n && (e = readdir(d))) {
        if (strncmp(e->d_name, "event", 5) != 0) continue;
        char path[300];
        snprintf(path, sizeof path, "%s/%s", input_dir, e->d_name);
        int fd = open_input(path);
        if (fd >= 0) fds[got++] = fd;
    }
    closedir(d);
    return got;
}

static bool home_down_any(const int *fds, int n) {
    for (int i = 0; i < n; i++) if (input_home_down(fds[i])) return true;
    return false;
}

// Home down now and still down `ms` milliseconds later (polled every 50 ms). Returns at once when it is not down.
static bool home_held_for(int ms) {
    int fds[MAX_INPUTS];
    int n = open_inputs_all(fds, MAX_INPUTS);
    bool held = home_down_any(fds, n);
    for (int waited = 0; held && waited < ms; waited += 50) { usleep(50000); held = home_down_any(fds, n); }
    for (int i = 0; i < n; i++) close(fds[i]);
    return held;
}

// ---- systemd and the rest of webOS ----
// READY=1 on $NOTIFY_SOCKET (a datagram socket, "@" for the abstract namespace), as LG's compositor does.
static void sd_notify_ready(void) {
    const char *path = getenv("NOTIFY_SOCKET");
    if (!path || !*path) { logline("no NOTIFY_SOCKET (not started by systemd)"); return; }
    int fd = socket(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return;
    struct sockaddr_un a = { .sun_family = AF_UNIX };
    size_t len = strlen(path);
    if (len >= sizeof a.sun_path) len = sizeof a.sun_path - 1;
    memcpy(a.sun_path, path, len);
    if (a.sun_path[0] == '@') a.sun_path[0] = 0;
    socklen_t alen = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + len);
    const char *msg = "READY=1\n";
    ssize_t n = sendto(fd, msg, strlen(msg), MSG_NOSIGNAL, (struct sockaddr *)&a, alen);
    logline("sd_notify READY=1 to %s: %s", path, n < 0 ? strerror(errno) : "sent");
    close(fd);
}

// /tmp/lsm-ready is the file lsm-ready.path watches and webapp-mgr.service's ConditionPathExists; LG's compositor
// has it made through `initctl emit lsm-ready` when its first frame is up.
static void emit_lsm_ready(void) {
    int fd = open("/tmp/lsm-ready", O_WRONLY | O_CREAT, 0644);
    if (fd >= 0) close(fd);
    if (access("/sbin/initctl", X_OK) == 0) {
        pid_t p = fork();
        if (p == 0) { execl("/sbin/initctl", "initctl", "emit", "--no-wait", "lsm-ready", (char *)NULL); _exit(127); }
        if (p > 0) waitpid(p, NULL, 0);
    }
    logline("lsm-ready emitted");
}

// After this process execs LG's compositor, WebAppMgr may have hit its start limit and the web Home cannot come
// back by itself; recover-home.sh (compositor/tools/tv-recover-home.sh) waits for LG's compositor and repairs
// that. Started as its own session so it outlives the exec and nothing of it reaches the unit's main process.
static void spawn_recovery(void) {
    char script[600], logpath[600];
    snprintf(script, sizeof script, "%s/bin/recover-home.sh", state_dir);
    snprintf(logpath, sizeof logpath, "%s/sm.log", state_dir);
    if (access(script, R_OK) != 0) { logline("no %s; LG's Home is not relaunched", script); return; }
    pid_t p = fork();
    if (p < 0) return;
    if (p == 0) {
        setsid();
        pid_t q = fork();
        if (q != 0) _exit(0);
        int null = open("/dev/null", O_RDWR);
        if (null >= 0) { dup2(null, 0); dup2(null, 1); dup2(null, 2); close(null); }
        for (int fd = 3; fd < 256; fd++) close(fd);
        unsetenv("NOTIFY_SOCKET");
        execl("/bin/sh", "sh", script, logpath, (char *)NULL);
        _exit(127);
    }
    waitpid(p, NULL, 0);
    logline("recover-home.sh started");
}

// ---- crash evidence (LPC 2026: a crash record shown at the next start) ----
// A fallback that is a problem (the compositor died, hung, drew nothing the panel showed) leaves a bundle where the next start can show it:
// <state>/../crash/<time>/ holds the reason, the compositor's last log lines, this process's own, the compositor's trace ring (a file in the
// runtime directory the compositor writes through a mapping, src/ring.rs, so it is whole after a SIGKILL or a signal from the GPU blob) and a few
// numbers of the system. At most CRASH_KEEP bundles are kept and each is a few hundred KiB at the most. `latest` names the newest and `pending` the
// newest one a problem made that nobody has looked at: the helper shows "LumaTV recovered from a problem" in Settings while it exists and removes it
// when the owner has read it. A hand-back the owner asked for (the hold-Home escape) leaves a bundle too, and no `pending`.
#define CRASH_KEEP 8
#define CRASH_LOG_LINES 200
#define CRASH_LOG_BYTES (64 * 1024)
#define RING_HEADER 64
#define RING_MAX (1024 * 1024)
static const char *crash_root = NULL;                    // tests; else <state>/../crash

static const char *crash_dir(void) {
    static char buf[600];
    if (crash_root) return crash_root;
    snprintf(buf, sizeof buf, "%s/../crash", state_dir);
    return buf;
}

static void mkdir_p(const char *path) {
    char tmp[600];
    snprintf(tmp, sizeof tmp, "%s", path);
    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') { *p = 0; mkdir(tmp, 0755); *p = '/'; }
    }
    mkdir(tmp, 0755);
}

// Copies the last `max_lines` lines of `src`, of its last `max_bytes` bytes at the most, to `dst`. False when `src` cannot be read.
static bool copy_tail(const char *src, const char *dst, int max_lines, size_t max_bytes) {
    FILE *f = fopen(src, "r");
    if (!f) return false;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    long start = size > (long)max_bytes ? size - (long)max_bytes : 0;
    fseek(f, start, SEEK_SET);
    char *buf = malloc(max_bytes + 1);
    if (!buf) { fclose(f); return false; }
    size_t n = fread(buf, 1, max_bytes, f);
    fclose(f);
    char *from = buf, *end = buf + n;
    if (start > 0) { char *nl = memchr(buf, '\n', n); from = nl ? nl + 1 : end; }   // the cut began inside a line
    char *scan = end;
    if (scan > from && scan[-1] == '\n') scan--;                                    // a last newline ends a line, it does not start one
    for (int seen = 0; scan > from;) {
        scan--;
        if (*scan == '\n' && ++seen == max_lines) { from = scan + 1; break; }
    }
    FILE *o = fopen(dst, "w");
    if (o) { fwrite(from, 1, (size_t)(end - from), o); fclose(o); }
    free(buf);
    return o != NULL;
}

static uint64_t le64(const unsigned char *p) {
    uint64_t v = 0;
    for (int i = 7; i >= 0; i--) v = v << 8 | p[i];
    return v;
}

// A trace ring file's text in order, oldest line first, to `dst`: the format of src/ring.rs ("LTVRING1", the text area's size and the bytes ever written, both
// little endian, at 8 and 16, the text from 64 on). Once it has wrapped, the oldest line is left out (the bytes that began it were overwritten).
// False when `src` is not a ring.
static bool ring_to_text(const char *src, const char *dst) {
    FILE *f = fopen(src, "r");
    if (!f) return false;
    unsigned char hdr[RING_HEADER];
    if (fread(hdr, 1, RING_HEADER, f) != RING_HEADER || memcmp(hdr, "LTVRING1", 8) != 0) { fclose(f); return false; }
    uint64_t size = le64(hdr + 8), head = le64(hdr + 16);
    if (size == 0 || size > RING_MAX) { fclose(f); return false; }
    char *area = malloc((size_t)size), *out = malloc((size_t)size + 1);
    if (!area || !out || fread(area, 1, (size_t)size, f) != (size_t)size) { fclose(f); free(area); free(out); return false; }
    fclose(f);
    size_t n = 0;
    if (head <= size) { memcpy(out, area, (size_t)head); n = (size_t)head; }
    else {
        size_t at = (size_t)(head % size);
        memcpy(out, area + at, (size_t)size - at);
        memcpy(out + ((size_t)size - at), area, at);
        n = (size_t)size;
    }
    char *from = out, *end = out + n;
    if (head > size) { char *nl = memchr(out, '\n', n); from = nl ? nl + 1 : end; }
    FILE *o = fopen(dst, "w");
    if (o) { fwrite(from, 1, (size_t)(end - from), o); fclose(o); }
    free(area); free(out);
    return o != NULL;
}

// The few numbers of the system at the time: uptime, load, and the memory lines the stalls of the frame path are read against.
static void crash_system_text(const char *dst) {
    FILE *o = fopen(dst, "w");
    if (!o) return;
    char line[256];
    FILE *f = fopen("/proc/uptime", "r");
    if (f) { if (fgets(line, sizeof line, f)) fprintf(o, "uptime: %s", line); fclose(f); }
    f = fopen("/proc/loadavg", "r");
    if (f) { if (fgets(line, sizeof line, f)) fprintf(o, "loadavg: %s", line); fclose(f); }
    f = fopen("/proc/meminfo", "r");
    if (f) {
        while (fgets(line, sizeof line, f)) {
            if (!strncmp(line, "MemTotal:", 9) || !strncmp(line, "MemFree:", 8) || !strncmp(line, "MemAvailable:", 13) || !strncmp(line, "SwapFree:", 9) || !strncmp(line, "CmaFree:", 8)) fputs(line, o);
        }
        fclose(f);
    }
    fclose(o);
}

static void remove_bundle(const char *root, const char *name) {
    char dir[700], file[900];
    snprintf(dir, sizeof dir, "%s/%s", root, name);
    DIR *d = opendir(dir);
    if (d) {
        struct dirent *e;
        while ((e = readdir(d))) {
            if (e->d_name[0] == '.') continue;
            snprintf(file, sizeof file, "%s/%s", dir, e->d_name);
            unlink(file);
        }
        closedir(d);
    }
    rmdir(dir);
}

static int cmp_names(const void *a, const void *b) { return strcmp((const char *)a, (const char *)b); }

// Keeps the newest `keep` bundles (the names sort by time).
static void rotate_crash_bundles(int keep) {
    DIR *d = opendir(crash_dir());
    if (!d) return;
    static char names[64][64];
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d)) && n < 64) {
        if (e->d_name[0] == '.' || !strcmp(e->d_name, "latest") || !strcmp(e->d_name, "pending")) continue;
        snprintf(names[n++], 64, "%s", e->d_name);
    }
    closedir(d);
    qsort(names, (size_t)n, 64, cmp_names);
    for (int i = 0; i < n - keep; i++) remove_bundle(crash_dir(), names[i]);
}

// Saves the evidence of a fallback; the bundle's directory name goes to `name` ("" when none was made).
static void crash_bundle(const char *reason, bool problem, char *name, size_t namesz) {
    char dir[700], path[800], stamp[48];
    name[0] = 0;
    time_t t = time(NULL);
    if (t > 1577836800) strftime(stamp, sizeof stamp, "%Y%m%d-%H%M%S", localtime(&t));
    else snprintf(stamp, sizeof stamp, "boot-%06ld", (long)mono_now());     // a clock that is not set yet
    mkdir_p(crash_dir());
    // <time>-<n>: the time orders bundles across starts, n within one second (a name freed by rotation is never made again in that second).
    static int bundle_seq;
    for (int k = 0; k < 100; k++) {
        snprintf(name, namesz, "%s-%03d", stamp, ++bundle_seq);
        snprintf(dir, sizeof dir, "%s/%s", crash_dir(), name);
        if (mkdir(dir, 0755) == 0) break;
        if (errno != EEXIST || k == 99) { name[0] = 0; return; }
    }
    snprintf(path, sizeof path, "%s/reason.txt", dir);
    FILE *f = fopen(path, "w");
    if (f) {
        char boot[64] = "";
        current_boot_id(boot, sizeof boot);
        fprintf(f, "reason: %s\nkind: %s\ntime: %s\ncompositor uptime: %.0f s\nboot: %s\nfailures: %ld\n", reason, problem ? "problem" : "escape", stamp,
                mono_now() - started_mono, boot, read_number(path_in_state("failures"), 0));
        fclose(f);
    }
    snprintf(path, sizeof path, "%s/compositor.log.tail", dir);
    copy_tail(path_in_state("compositor.log"), path, CRASH_LOG_LINES, CRASH_LOG_BYTES);
    snprintf(path, sizeof path, "%s/sm.log.tail", dir);
    copy_tail(path_in_state("sm.log"), path, 60, 16 * 1024);
    snprintf(path, sizeof path, "%s/trace.txt", dir);
    char ring[600];
    snprintf(ring, sizeof ring, "%s/lumatv-trace.ring", runtime_dir);
    if (!ring_to_text(ring, path)) write_text(path, "no trace ring\n");
    snprintf(path, sizeof path, "%s/system.txt", dir);
    crash_system_text(path);
    snprintf(path, sizeof path, "%s/latest", crash_dir());
    write_text(path, name);
    if (problem) {
        snprintf(path, sizeof path, "%s/pending", crash_dir());
        write_text(path, name);
    }
    rotate_crash_bundles(CRASH_KEEP);
    logline("crash evidence saved in %s/%s", crash_dir(), name);
}

// ---- fallback to LG's compositor ----
static void release_for_exec(void);
static void fallback(const char *reason, bool count_failure) {
    logline("fallback: %s", reason);
    write_text(path_in_state("last-fallback"), reason);
    if (count_failure) write_number(path_in_state("failures"), read_number(path_in_state("failures"), 0) + 1);
    // Handing the screen over on purpose is not an unclean end: the next start must not count it a second time.
    forget_running();
    if (child_pid > 0) {
        kill(child_pid, SIGTERM);
        for (int i = 0; i < 20 && waitpid(child_pid, NULL, WNOHANG) == 0; i++) usleep(50000);
        kill(child_pid, SIGKILL);
        waitpid(child_pid, NULL, 0);
        child_pid = 0;
    }
    if (!dry_run && !probe) restore_vsync_divider();
    if (dry_run || probe) {
        logline("%s: would unmount %s and exec LG's compositor", dry_run ? "dry run" : "probe", lg_binary);
        exit(75);
    }
    // The evidence, with the compositor gone and its log and ring as it left them. Not for the decision at start-up: nothing had run.
    if (supervising) {
        char bundle[64];
        crash_bundle(reason, count_failure, bundle, sizeof bundle);
    }
    // A fallback after the unit started under this supervisor (not the decision at start-up, where nothing of
    // LG's was disturbed yet) needs WebAppMgr and Home brought back once LG's compositor is up.
    if (supervising) spawn_recovery();
    // The bind mount placed by the early hook: once it is gone, the path is LG's file again, and so is every later
    // restart of the unit until the next boot decides anew.
    if (umount2(lg_binary, MNT_DETACH) != 0 && errno != EINVAL) logline("umount %s: %s", lg_binary, strerror(errno));
    release_for_exec();
    execv(lg_binary, saved_argv);
    logline("exec %s: %s", lg_binary, strerror(errno));
    exit(1);
}

// ---- a minimal JSON scanner for the bridge's own messages (top-level keys only) ----
// Finds the value of top-level key `key` in object `obj`; returns the start and length of the raw value.
static bool json_value(const char *obj, const char *key, const char **start, size_t *len) {
    const char *p = obj;
    while (*p && *p != '{') p++;
    if (*p != '{') return false;
    p++;
    size_t klen = strlen(key);
    while (*p) {
        while (*p == ' ' || *p == ',' || *p == '\n') p++;
        if (*p == '}') return false;
        if (*p != '"') return false;
        const char *k = ++p;
        while (*p && *p != '"') { if (*p == '\\') p++; p++; }
        size_t kl = p - k;
        p++;
        while (*p == ' ' || *p == ':') p++;
        const char *v = p;
        // skip one value
        int depth = 0;
        if (*p == '"') {
            p++;
            while (*p && *p != '"') { if (*p == '\\') p++; p++; }
            if (*p == '"') p++;
        } else if (*p == '{' || *p == '[') {
            do {
                if (*p == '"') { p++; while (*p && *p != '"') { if (*p == '\\') p++; p++; } }
                else if (*p == '{' || *p == '[') depth++;
                else if (*p == '}' || *p == ']') depth--;
                p++;
            } while (*p && depth > 0);
        } else {
            while (*p && *p != ',' && *p != '}') p++;
        }
        if (kl == klen && !strncmp(k, key, klen)) { *start = v; *len = p - v; return true; }
    }
    return false;
}

static bool json_string(const char *obj, const char *key, char *out, size_t outsz) {
    const char *v; size_t n;
    if (!json_value(obj, key, &v, &n) || n < 2 || v[0] != '"') return false;
    size_t o = 0;
    for (size_t i = 1; i + 1 < n && o + 1 < outsz; i++) {
        char c = v[i];
        if (c == '\\' && i + 2 < n) {
            i++;
            c = v[i];
            if (c == 'n') c = '\n'; else if (c == 't') c = '\t';
        }
        out[o++] = c;
    }
    out[o] = 0;
    return true;
}

static bool json_number(const char *obj, const char *key, unsigned long *out) {
    const char *v; size_t n;
    if (!json_value(obj, key, &v, &n) || n == 0) return false;
    *out = strtoul(v, NULL, 10);
    return true;
}

static bool json_bool(const char *obj, const char *key) {
    const char *v; size_t n;
    return json_value(obj, key, &v, &n) && n >= 4 && !strncmp(v, "true", 4);
}

static void json_escape(const char *s, char *out, size_t outsz) {
    size_t o = 0;
    for (; *s && o + 2 < outsz; s++) {
        if (*s == '"' || *s == '\\') { out[o++] = '\\'; out[o++] = *s; }
        else if (*s == '\n') { out[o++] = '\\'; out[o++] = 'n'; }
        else out[o++] = *s;
    }
    out[o] = 0;
}

// ---- the bridge socket ----
static char inbuf[256 * 1024];
static size_t inlen;

// The bridge is one JSON object per line, and luna payloads arrive as their senders wrote them: com.webos.app.hdmi2's
// getCurrentHomeUIState came pretty-printed (set, 2026-10-25) and split into four bad lines. A raw newline in valid
// JSON can only be whitespace between tokens (inside strings it must be escaped), so a space keeps the meaning.
static void flatten(char *s) {
    for (; *s; s++) if (*s == '\n' || *s == '\r') *s = ' ';
}

// Calls that reach the bus names before the compositor has connected are kept in order and sent on connect. They
// used to be dropped and never answered; SAM's getForegroundAppInfo subscription, made when the name appears, is
// the likely casualty behind the 2026-10-25 gate's "Home stays in front".
#define MAX_PENDING 256
static char *pending[MAX_PENDING];
static int npending;

static void write_line(const char *line) {
    size_t n = strlen(line);
    struct iovec iov[2] = { { (void *)line, n }, { (void *)"\n", 1 } };
    struct msghdr mh = { 0 };
    mh.msg_iov = iov; mh.msg_iovlen = 2;
    if (sendmsg(bridge_fd, &mh, MSG_NOSIGNAL) < 0) logline("bridge send: %s", strerror(errno));
}

static void send_line(const char *line) {
    if (bridge_fd >= 0) { write_line(line); return; }
    if (npending == MAX_PENDING) { logline("bridge: %d lines waiting for the compositor; dropped one", npending); return; }
    pending[npending++] = strdup(line);
}

static void flush_pending(void) {
    if (npending) logline("bridge: %d call(s) that arrived before the compositor sent on", npending);
    for (int i = 0; i < npending; i++) { write_line(pending[i]); free(pending[i]); }
    npending = 0;
}

// ---- inbound luna calls, kept by token until the compositor answers ----
#define MAX_INBOUND 512
struct inbound { unsigned long token; LSHandle *sh; LSMessage *msg; bool subscription; bool answered; char key[32]; char unique[64]; };
static struct inbound inbounds[MAX_INBOUND];

static struct inbound *inbound_find(unsigned long token) {
    for (int i = 0; i < MAX_INBOUND; i++) if (inbounds[i].token == token) return &inbounds[i];
    return NULL;
}

static void inbound_free(struct inbound *in) {
    if (in->msg) LSMessageUnref(in->msg);
    memset(in, 0, sizeof *in);
}

struct bus { const char *name; LSHandle *sh; };
static struct bus buses[16];
static int nbuses;

// Before LG's compositor is exec'd in this process: every bus name this supervisor holds is given back and every
// descriptor above stderr is closed. Without it the luna-service2 sockets would survive the exec inside LG's
// process, ls-hubd would go on seeing their owner alive, and LG's own LSRegister of the same names would be
// refused. The unit is not socket-activated, so nothing above fd 2 is LG's to inherit.
static void release_for_exec(void) {
    for (int i = 0; i < nbuses; i++) {
        LSError e; LSErrorInit(&e);
        if (buses[i].sh && !LSUnregister(buses[i].sh, &e)) logline("LSUnregister %s: %s", buses[i].name, e.message ? e.message : "?");
        LSErrorFree(&e);
        buses[i].sh = NULL;
    }
    logline("released %d bus name(s); exec %s", nbuses, lg_binary);
    nbuses = 0;
    if (logf) { fclose(logf); logf = NULL; }
    long max = sysconf(_SC_OPEN_MAX);
    if (max < 0 || max > 65536) max = 4096;
    for (int fd = 3; fd < max; fd++) close(fd);
}

static bool on_method(LSHandle *sh, LSMessage *msg, void *ctx) {
    struct bus *b = ctx;
    struct inbound *slot = inbound_find(0);
    if (!slot) {
        LSError e; LSErrorInit(&e);
        LSMessageReply(sh, msg, "{\"returnValue\":false,\"errorText\":\"too many pending calls\"}", &e);
        LSErrorFree(&e);
        return true;
    }
    LSMessageRef(msg);
    slot->token = ++inbound_token;
    slot->sh = sh;
    slot->msg = msg;
    slot->subscription = LSMessageIsSubscription(msg);
    snprintf(slot->key, sizeof slot->key, "t%lu", slot->token);
    const char *u = LSMessageGetUniqueToken(msg);
    snprintf(slot->unique, sizeof slot->unique, "%s", u ? u : "");
    if (slot->subscription) {
        LSError e; LSErrorInit(&e);
        if (!LSSubscriptionAdd(sh, slot->key, msg, &e)) { logline("LSSubscriptionAdd: %s", e.message ? e.message : "?"); slot->subscription = false; }
        LSErrorFree(&e);
    }
    const char *payload = LSMessageGetPayload(msg);
    const char *method = LSMessageGetMethod(msg);
    const char *sender = LSMessageGetSenderServiceName(msg);
    if (!sender || !*sender) sender = LSMessageGetSender(msg);
    char esender[256], emethod[128];
    json_escape(sender ? sender : "", esender, sizeof esender);
    json_escape(method ? method : "", emethod, sizeof emethod);
    // Who subscribes to what is the evidence a gate run needs (SAM's foreground subscription above all).
    if (slot->subscription) logline("subscription t%lu: %s/%s from %s%s", slot->token, b->name, method ? method : "?", sender ? sender : "?", bridge_fd < 0 ? " (compositor not connected yet: queued)" : "");
    size_t need = strlen(payload ? payload : "{}") + 512;
    char *line = malloc(need);
    snprintf(line, need, "{\"kind\":\"inbound\",\"token\":%lu,\"service\":\"%s\",\"method\":\"%s\",\"payload\":%s,\"subscribe\":%s,\"sender\":\"%s\"}",
             slot->token, b->name, emethod, payload && *payload ? payload : "{}", slot->subscription ? "true" : "false", esender);
    flatten(line);
    send_line(line);
    free(line);
    return true;
}

static bool on_cancel(LSHandle *sh, LSMessage *msg, void *ctx) {
    (void)sh; (void)ctx;
    const char *u = LSMessageGetUniqueToken(msg);
    for (int i = 0; i < MAX_INBOUND; i++) {
        if (inbounds[i].token && u && !strcmp(inbounds[i].unique, u)) {
            char line[256];
            snprintf(line, sizeof line, "{\"kind\":\"inbound\",\"token\":%lu,\"service\":\"\",\"method\":\"\",\"payload\":{},\"subscribe\":false,\"sender\":\"\",\"cancelled\":true}", inbounds[i].token);
            send_line(line);
            inbound_free(&inbounds[i]);
        }
    }
    return true;
}

static void reply_inbound(unsigned long token, const char *payload) {
    struct inbound *in = inbound_find(token);
    if (!in) return;
    LSError e; LSErrorInit(&e);
    if (!in->answered) {
        if (!LSMessageReply(in->sh, in->msg, payload, &e)) logline("LSMessageReply: %s", e.message ? e.message : "?");
        in->answered = true;
        if (!in->subscription) inbound_free(in);
    } else if (in->subscription) {
        if (!LSSubscriptionReply(in->sh, in->key, payload, &e)) logline("LSSubscriptionReply: %s", e.message ? e.message : "?");
    }
    LSErrorFree(&e);
}

// ---- outbound calls ----
#define MAX_OUTBOUND 256
struct outbound { unsigned long token; LSMessageToken ls; bool subscribe; };
static struct outbound outbounds[MAX_OUTBOUND];

static bool on_reply(LSHandle *sh, LSMessage *reply, void *ctx) {
    (void)sh;
    struct outbound *o = ctx;
    const char *payload = LSMessageGetPayload(reply);
    size_t need = strlen(payload ? payload : "{}") + 64;
    char *line = malloc(need);
    snprintf(line, need, "{\"kind\":\"event\",\"token\":%lu,\"payload\":%s}", o->token, payload && *payload ? payload : "{}");
    flatten(line);
    send_line(line);
    free(line);
    if (!o->subscribe) memset(o, 0, sizeof *o);
    return true;
}

static void make_call(unsigned long token, const char *uri, const char *payload, bool subscribe) {
    struct outbound *o = NULL;
    for (int i = 0; i < MAX_OUTBOUND; i++) if (!outbounds[i].token) { o = &outbounds[i]; break; }
    if (!o || nbuses == 0) return;
    o->token = token;
    o->subscribe = subscribe;
    LSError e; LSErrorInit(&e);
    bool ok = subscribe ? LSCall(buses[0].sh, uri, payload, on_reply, o, &o->ls, &e) : LSCallOneReply(buses[0].sh, uri, payload, on_reply, o, &o->ls, &e);
    if (!ok) {
        logline("LSCall %s: %s", uri, e.message ? e.message : "?");
        char line[256];
        snprintf(line, sizeof line, "{\"kind\":\"event\",\"token\":%lu,\"payload\":{\"returnValue\":false,\"errorText\":\"call failed\"}}", token);
        send_line(line);
        memset(o, 0, sizeof *o);
    }
    LSErrorFree(&e);
}

static void cancel_call(unsigned long token) {
    for (int i = 0; i < MAX_OUTBOUND; i++) {
        if (outbounds[i].token == token) {
            LSError e; LSErrorInit(&e);
            LSCallCancel(buses[0].sh, outbounds[i].ls, &e);
            LSErrorFree(&e);
            memset(&outbounds[i], 0, sizeof outbounds[i]);
        }
    }
}

// ---- messages from the compositor ----
static void handle_line(char *line) {
    char kind[32] = "";
    json_string(line, "kind", kind, sizeof kind);
    if (!strcmp(kind, "pong")) {
        json_number(line, "seq", &pong_seq);
    } else if (!strcmp(kind, "reply")) {
        unsigned long token = 0; const char *v; size_t n;
        if (json_number(line, "token", &token) && json_value(line, "payload", &v, &n)) {
            char *payload = strndup(v, n);
            reply_inbound(token, payload);
            free(payload);
        }
    } else if (!strcmp(kind, "outbound")) {
        unsigned long token = 0; const char *v; size_t n; char uri[512];
        if (json_number(line, "token", &token) && json_string(line, "uri", uri, sizeof uri) && json_value(line, "payload", &v, &n)) {
            char *payload = strndup(v, n);
            make_call(token, uri, payload, json_bool(line, "subscribe"));
            free(payload);
        }
    } else if (!strcmp(kind, "cancel")) {
        unsigned long token = 0;
        if (json_number(line, "token", &token)) cancel_call(token);
    } else if (!strcmp(kind, "fallback")) {
        char reason[256] = "requested";
        json_string(line, "reason", reason, sizeof reason);
        // The escape (hold Home) keeps LG's compositor until the owner re-enables LumaTV from the tool.
        write_text(path_in_state("escaped"), reason);
        fallback(reason, false);
    } else {
        logline("bridge: unknown message kind %s", kind);
    }
}

// The reason a child's end gives: its exit code or the signal that killed it.
static void child_end_reason(int status, char *out, size_t outsz) {
    if (WIFEXITED(status)) snprintf(out, outsz, "compositor exited with %d", WEXITSTATUS(status));
    else snprintf(out, outsz, "compositor killed by signal %d", WTERMSIG(status));
}

// The kernel closes a dying process's sockets before the process can be reaped, so the bridge's hang-up reaches this
// loop ahead of the child watch: a `kill -9` read "compositor closed the bridge socket" (gate 8's wd-crash). The child
// is given BRIDGE_REAP_MS to become reapable; `true` with its exit status as the reason when it was, `false` for a
// compositor still alive (it closed the socket itself, and the bridge's own words stay the reason).
#define BRIDGE_REAP_MS 300
static bool reap_child_within(int ms, char *reason, size_t reasonsz) {
    if (child_pid <= 0) return false;
    for (int waited = 0;; waited += 5) {
        int status;
        pid_t r = waitpid(child_pid, &status, WNOHANG);
        if (r == child_pid) {
            child_pid = 0;
            child_end_reason(status, reason, reasonsz);
            return true;
        }
        if (r < 0 || waited >= ms) return false;
        usleep(5000);
    }
}

static void bridge_gone(const char *what) {
    char reason[128];
    if (!reap_child_within(BRIDGE_REAP_MS, reason, sizeof reason)) snprintf(reason, sizeof reason, "%s", what);
    fallback(reason, true);
}

static gboolean on_bridge_readable(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)data;
    if (cond & (G_IO_HUP | G_IO_ERR)) { bridge_gone("compositor closed the bridge socket"); return 0; }
    ssize_t n = read(bridge_fd, inbuf + inlen, sizeof inbuf - inlen - 1);
    if (n <= 0) { bridge_gone("bridge read ended"); return 0; }
    inlen += n;
    inbuf[inlen] = 0;
    char *start = inbuf, *nl;
    while ((nl = memchr(start, '\n', inlen - (start - inbuf)))) {
        *nl = 0;
        handle_line(start);
        start = nl + 1;
    }
    size_t rest = inlen - (start - inbuf);
    memmove(inbuf, start, rest);
    inlen = rest;
    if (inlen == sizeof inbuf - 1) { logline("bridge: line too long"); inlen = 0; }
    return 1;
}

static gboolean on_bridge_accept(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)cond; (void)data;
    int fd = accept4(bridge_listen, NULL, NULL, SOCK_CLOEXEC);
    if (fd < 0) return 1;
    if (bridge_fd >= 0) { logline("bridge: second connection refused"); close(fd); return 1; }
    bridge_fd = fd;
    logline("bridge: compositor connected");
    flush_pending();
    g_io_add_watch(g_io_channel_unix_new(fd), G_IO_IN | G_IO_HUP | G_IO_ERR, on_bridge_readable, NULL);
    // The compositor is listening on LG's socket and the bus names are ours: the unit is up.
    if (!dry_run) { sd_notify_ready(); emit_lsm_ready(); }
    return 1;
}

static gboolean on_heartbeat(gpointer data) {
    (void)data;
    if (bridge_fd < 0) {
        if (mono_now() - started_mono > 20) fallback("compositor did not connect to the bridge within 20 s", true);
        return 1;
    }
    if (ping_seq > pong_seq + 2) { fallback("heartbeat: two beats missed", true); return 1; }
    char line[64];
    snprintf(line, sizeof line, "{\"kind\":\"ping\",\"seq\":%lu}", ++ping_seq);
    send_line(line);
    // Two minutes healthy: this start has proved itself, and a power cut after it is no failed boot.
    if (ping_seq == 240 && running_path[0]) { forget_running(); logline("settled: two minutes healthy"); }
    // Ten minutes healthy clears the failure count: a crash a week later is not "consecutive".
    if (ping_seq == 1200) write_number(path_in_state("failures"), 0);
    return 1;
}

static void on_child(int pid, int status, gpointer data) {
    (void)data;
    if (pid != child_pid) return;
    child_pid = 0;
    char reason[128];
    child_end_reason(status, reason, sizeof reason);
    fallback(reason, true);
}

// ---- the compositor's bus names and methods (docs/research/shell-architecture.md, 56 methods on 12 names) ----
#define M(n) { n, on_method, 0 }
static LSMethod m_surfacemanager[] = { M("closeByAppId"), M("getForegroundAppInfo"), M("captureCompositorOutput"), M("clearRecentsList"), M("closeLRUSurface"), M("getCurrentState"), M("getFocusedWindowInfo"), M("getRecentsAppList"), M("showInputPicker"), M("showQuickSettings"), M("showSpinner"), M("showIndicator"), M("getForegroundVideoWindow"), M("getWindowGroupInfo"), M("getForegroundWindowInfo"), M("getScreenVisible"), M("setScreenVisible"), M("getOverlayPlayVideoState"), M("getForegroundAppState"), M("changeMultiViewLayout"), M("setMultiViewLayout"), M("getLastInputType"), M("getMultiViewLayoutAnimationState"), M("setPinOverlayView"), M("getZoomStatus"), M("setZoom"), M("alertChoice"), M("setLaunchOrigin"), { 0 } };
static LSMethod m_recent[] = { M("listRecents"), M("removeAllRecents"), M("removeRecent"), M("requestScreenCapture"), { 0 } };
static LSMethod m_coverui[] = { M("open"), M("close"), M("getStatus"), { 0 } };
static LSMethod m_screensaver[] = { M("restartUserActivityTimer"), { 0 } };
static LSMethod m_hcap[] = { M("getHcapOpacity"), M("setHcapOpacity"), { 0 } };
static LSMethod m_systemui_manager[] = { M("launch"), M("closeByAppId"), M("getForegroundAppInfo"), { 0 } };
static LSMethod m_systemui[] = { M("closeAllSystemUI"), { 0 } };
static LSMethod m_home[] = { M("getCurrentHomeUIState"), { 0 } };
static LSMethod m_extended[] = { M("getTextureRenderingStatus"), M("setTextureRenderingStatus"), { 0 } };
static LSMethod m_magicnumber[] = { M("notifyChannelToRegister"), { 0 } };
static LSMethod m_input[] = { M("blockKeyInput"), M("cancelBlockKeyInput"), M("getKeyInputBlockState"), M("getForegroundAppKeymask"), M("registerAllowedKey"), { 0 } };
static LSMethod m_sysuicompmgr[] = { M("changeSystemUIView"), M("changeSystemUIViewStateTransition"), M("launch"), M("close"), { 0 } };

static struct { const char *name; LSMethod *methods; } bus_table[] = {
    { "com.webos.surfacemanager", m_surfacemanager },
    { "com.webos.service.recent", m_recent },
    { "com.webos.coverUI", m_coverui },
    { "com.webos.surfacemanager.screenSaver", m_screensaver },
    { "com.webos.surfacemanager.hcap", m_hcap },
    { "com.webos.service.systemUIManager", m_systemui_manager },
    { "com.webos.surfacemanager.systemui", m_systemui },
    { "com.webos.surfacemanager.home", m_home },
    { "com.webos.surfacemanager.extended", m_extended },
    { "com.webos.keyfilter.magicnumber", m_magicnumber },
    { "com.webos.surfacemanager.input", m_input },
    { "com.webos.service.sysuicompmgr", m_sysuicompmgr },
    { NULL, NULL },
};

// Registers every bus name. Live, any refusal is a fallback at once: the names are what SAM and the key filters
// talk to, and a compositor without them is a black panel for every app that waits on an answer.
static void register_buses(void) {
    for (int i = 0; bus_table[i].name; i++) {
        char name[128];
        snprintf(name, sizeof name, "%s%s", bus_table[i].name, dry_run ? ".lumatv" : "");
        LSError e; LSErrorInit(&e);
        LSHandle *sh = NULL;
        if (!LSRegister(name, &sh, &e)) {
            logline("LSRegister %s: %s", name, e.message ? e.message : "?");
            LSErrorFree(&e);
            if (!dry_run) { char why[200]; snprintf(why, sizeof why, "cannot register %s", name); fallback(why, true); }
            continue;
        }
        buses[nbuses].name = bus_table[i].name;
        buses[nbuses].sh = sh;
        if (!LSRegisterCategory(sh, "/", bus_table[i].methods, NULL, NULL, &e)) logline("LSRegisterCategory %s: %s", name, e.message ? e.message : "?");
        LSCategorySetData(sh, "/", &buses[nbuses], &e);
        LSSubscriptionSetCancelFunction(sh, on_cancel, NULL, &e);
        if (!LSGmainAttach(sh, loop, &e)) logline("LSGmainAttach %s: %s", name, e.message ? e.message : "?");
        LSErrorFree(&e);
        nbuses++;
        logline("registered %s (%d methods)", name, (int)({ int c = 0; while (bus_table[i].methods[c].name) c++; c; }));
    }
}

// --probe: one LSRegister per real name, the hub's verdict logged and printed, nothing kept.
static int probe_buses(void) {
    int refused = 0, taken = 0, got = 0;
    for (int i = 0; bus_table[i].name; i++) {
        LSError e; LSErrorInit(&e);
        LSHandle *sh = NULL;
        if (LSRegister(bus_table[i].name, &sh, &e)) {
            got++;
            printf("probe %s: registered (nobody holds it)\n", bus_table[i].name);
            logline("probe %s: registered", bus_table[i].name);
            LSUnregister(sh, &e);
        } else {
            const char *m = e.message ? e.message : "?";
            bool dup = strstr(m, "already exists") != NULL;
            if (dup) taken++; else refused++;
            printf("probe %s: %s -> %s\n", bus_table[i].name, m, dup ? "role allows it" : "REFUSED");
            logline("probe %s: %s", bus_table[i].name, m);
        }
        LSErrorFree(&e);
    }
    printf("probe: %d allowed (held by LG's compositor), %d free, %d refused\n", taken, got, refused);
    return refused ? 2 : 0;
}

// ---- the compositor child ----
// Extra compositor flags for a TV round (docs/research/hw-planes-result.md): the words of this file, appended to a live start's standard ones. It is in /tmp
// so that a flag cannot outlive the boot. A word the compositor does not take makes it refuse to start, which is any other failed start: LG's compositor
// comes back.
#define EXTRA_FLAGS_FILE "/tmp/lumatv/compositor.flags"
#define EXTRA_FLAGS_MAX 32
static char extra_text[1024];
static const char *extra_words[EXTRA_FLAGS_MAX];

static int read_extra_flags(void) {
    FILE *f = fopen(EXTRA_FLAGS_FILE, "r");
    if (!f) return 0;
    size_t len = fread(extra_text, 1, sizeof extra_text - 1, f);
    fclose(f);
    extra_text[len] = 0;
    int n = 0;
    for (char *w = strtok(extra_text, " \t\r\n"); w && n < EXTRA_FLAGS_MAX; w = strtok(NULL, " \t\r\n")) extra_words[n++] = w;
    return n;
}

static void spawn_compositor(void) {
    char loader[600], libdir[600], bin[600], blob[600], bridge[600], logpath[600];
    snprintf(loader, sizeof loader, "%s/lib/ld-linux-aarch64.so.1", state_dir);
    snprintf(libdir, sizeof libdir, "%s/lib", state_dir);
    snprintf(bin, sizeof bin, "%s/bin/lumatv-compositor", state_dir);
    snprintf(blob, sizeof blob, "%s/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so", state_dir);
    snprintf(bridge, sizeof bridge, "%s/bridge.sock", state_dir);
    snprintf(logpath, sizeof logpath, "%s/compositor.log", state_dir);
    mkdir(runtime_dir, 0700);
    // read here, in the parent: the child allocates nothing
    int extra = dry_run ? 0 : read_extra_flags();
    if (extra > 0) {
        char line[1100] = "";
        for (int i = 0; i < extra; i++) { strncat(line, i ? " " : "", sizeof line - strlen(line) - 1); strncat(line, extra_words[i], sizeof line - strlen(line) - 1); }
        logline("compositor flags from %s: %s", EXTRA_FLAGS_FILE, line);
    }
    pid_t pid = fork();
    if (pid < 0) { fallback("fork failed", true); return; }
    if (pid == 0) {
        int lf = open(logpath, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (lf >= 0) { dup2(lf, 1); dup2(lf, 2); close(lf); }
        setenv("XDG_RUNTIME_DIR", runtime_dir, 1);
        setenv("LUMATV_LUNA_BRIDGE", bridge, 1);
        setenv("LUMATV_LOG", "info", 0);
        const char *renderer = dry_run ? "gl" : "kms";
        const char *sock = dry_run ? "wayland-lumatv" : socket_name;
        const char *rt = dry_run ? "/tmp/lumatv" : runtime_dir;
        // Live, the compositor reads the remotes itself (--input): key routing and the hold-Home escape.
        // Buffers come from the driver's own mali_buffer_sharing: with the compositor's own global LG's 32-bit
        // libmali clients (Qt apps, Flutter Home, the input apps, Netflix) segfault right after binding it (set,
        // 2026-10-25 06:20: tvhotkey exit 139 with own, runs with driver); only WAM and LumaTV's clients survived.
        const char *av[20 + EXTRA_FLAGS_MAX];
        int ac = 0;
        av[ac++] = loader; av[ac++] = "--library-path"; av[ac++] = libdir; av[ac++] = bin;
        av[ac++] = "--runtime-dir"; av[ac++] = rt; av[ac++] = "--socket"; av[ac++] = sock;
        av[ac++] = "--renderer"; av[ac++] = renderer; av[ac++] = "--driver"; av[ac++] = blob;
        av[ac++] = "--buffers"; av[ac++] = "driver"; av[ac++] = "--card"; av[ac++] = "/dev/dri/card0";
        if (!dry_run) av[ac++] = "--input";
        for (int i = 0; i < extra; i++) av[ac++] = extra_words[i];
        av[ac] = NULL;
        execv(loader, (char *const *)av);
        _exit(127);
    }
    child_pid = pid;
    g_child_watch_add(pid, on_child, NULL);
    logline("compositor started: pid %d (%s)", pid, dry_run ? "dry run, private socket, offscreen" : "kms on /tmp/xdg/wayland-0");
}

static void open_bridge(void) {
    char path[600];
    snprintf(path, sizeof path, "%s/bridge.sock", state_dir);
    unlink(path);
    bridge_listen = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_un a = { .sun_family = AF_UNIX };
    snprintf(a.sun_path, sizeof a.sun_path, "%s", path);
    if (bind(bridge_listen, (struct sockaddr *)&a, sizeof a) < 0 || listen(bridge_listen, 1) < 0) fallback("bridge socket", true);
    chmod(path, 0600);
    g_io_add_watch(g_io_channel_unix_new(bridge_listen), G_IO_IN, on_bridge_accept, NULL);
}

// ---- the watchdog: what the compositor shows, not only that it answers ----
// A compositor can answer every ping and still leave the panel dead: frames drawn that no flip ever presents (the
// display driver wedged, a commit refused for good), or no window mapped at all while the set is on (WebAppMgr
// never brought Home back). Both are read from the compositor's control socket and its own counters (`status`:
// "drawn" and the summary's "flips"; `windows`: "mapped") and answered like a crash: a counted fallback to LG's.
// Whether the set is on comes from tvpower, so a boot into standby, where nothing may map for minutes, is not a
// black panel.
#define WD_FROZEN_SECS 6.0
#define WD_FROZEN_DRAWN 20
#define WD_BLANK_SECS 45.0

struct wd {
    bool have_prev;
    long prev_drawn, prev_flips;
    double anchor_t;                // the last moment the output was demonstrably fine
    long anchor_drawn;
    double blank_since;             // 0 = a window is mapped or the set is not on
};

struct wd_in {
    double now;
    bool have_flips;                // `status` gave both counters (the offscreen renderer has no flips)
    long drawn, flips;
    bool have_windows;
    int mapped;
    bool power_known, power_active;
};

enum wd_verdict { WD_OK, WD_FROZEN, WD_BLANK };

static enum wd_verdict wd_step(struct wd *w, const struct wd_in *in, char *why, size_t whysz) {
    if (in->have_flips) {
        // An idle screen draws nothing and a landing flip is progress: either one makes this moment the new
        // baseline. Frames drawn with no flip since the baseline are the freeze, once 20 of them and 6 s add up.
        if (!w->have_prev || in->drawn == w->prev_drawn || in->flips > w->prev_flips || (in->power_known && !in->power_active)) {
            w->anchor_t = in->now;
            w->anchor_drawn = in->drawn;
        } else if (in->drawn - w->anchor_drawn >= WD_FROZEN_DRAWN && in->now - w->anchor_t >= WD_FROZEN_SECS) {
            snprintf(why, whysz, "output frozen: %ld frames drawn and no page flip for %.0f s", in->drawn - w->anchor_drawn, in->now - w->anchor_t);
            return WD_FROZEN;
        }
        w->have_prev = true;
        w->prev_drawn = in->drawn;
        w->prev_flips = in->flips;
    }
    if (in->have_windows && in->power_known) {
        if (in->power_active && in->mapped == 0) {
            if (w->blank_since == 0) w->blank_since = in->now;
            else if (in->now - w->blank_since >= WD_BLANK_SECS) {
                snprintf(why, whysz, "nothing on the panel: no window mapped for %.0f s with the set on", in->now - w->blank_since);
                return WD_BLANK;
            }
        } else {
            w->blank_since = 0;
        }
    }
    return WD_OK;
}

// One command on the compositor's control socket: the reply line (newline cut) lands in out and its length is
// returned, -1 when the socket does not answer within timeout_ms. Everything is non-blocking and bounded: this
// loop also carries the heartbeat and every luna call, and must not wait on a compositor that is stuck.
static int control_query(const char *dir, const char *cmd, char *out, size_t outsz, int timeout_ms) {
    struct sockaddr_un a = { .sun_family = AF_UNIX };
    snprintf(a.sun_path, sizeof a.sun_path, "%s/lumatv-control", dir);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    if (connect(fd, (struct sockaddr *)&a, sizeof a) < 0) { close(fd); return -1; }
    char line[128];
    int ln = snprintf(line, sizeof line, "%s\n", cmd);
    if (send(fd, line, (size_t)ln, MSG_NOSIGNAL) != ln) { close(fd); return -1; }
    size_t got = 0;
    double end = mono_now() + timeout_ms / 1000.0;
    for (;;) {
        char *nl = memchr(out, '\n', got);
        if (nl) { *nl = 0; close(fd); return (int)(nl - out); }
        if (got + 1 >= outsz) { out[got] = 0; close(fd); return (int)got; }
        int left = (int)((end - mono_now()) * 1000.0);
        if (left <= 0) break;
        struct pollfd p = { .fd = fd, .events = POLLIN };
        if (poll(&p, 1, left) <= 0) break;
        ssize_t n = recv(fd, out + got, outsz - 1 - got, 0);
        if (n == 0) break;
        if (n < 0) { if (errno == EAGAIN || errno == EINTR) continue; break; }
        got += (size_t)n;
    }
    out[0] = 0;
    close(fd);
    return -1;
}

static bool scan_long_after(const char *text, const char *needle, long *out) {
    const char *p = strstr(text, needle);
    if (!p) return false;
    p += strlen(needle);
    char *end;
    long v = strtol(p, &end, 10);
    if (end == p) return false;
    *out = v;
    return true;
}

// `ctl status`: "stats":{..."drawn":N...} and, in the summary text, "flips N" (the KMS output's counter).
static void parse_status(const char *json, struct wd_in *in) {
    in->have_flips = scan_long_after(json, "\"drawn\":", &in->drawn) && scan_long_after(json, " flips ", &in->flips);
}

// `ctl windows`: one "mapped":true per window on the panel.
static int count_mapped(const char *json) {
    int n = 0;
    for (const char *p = json; (p = strstr(p, "\"mapped\":true")) != NULL; p += 13) n++;
    return n;
}

static struct wd wd;
static bool power_known, power_active;
static char power_state[40];
static LSMessageToken power_token;
static bool power_call_live;
static double power_asked = -100, last_scan = -100, last_report;
static bool progress_announced, progress_warned, seen_flips, seen_windows;
static char reply_buf[64 * 1024];

// tvpower's state, as the compositor itself subscribes to it (the same call through the same handle, so the role
// that lets the compositor ask lets this ask). Asked again every 10 s until a state arrives.
static bool on_power(LSHandle *sh, LSMessage *msg, void *ctx) {
    (void)sh; (void)ctx;
    const char *payload = LSMessageGetPayload(msg);
    char state[40];
    if (!payload || !json_string(payload, "state", state, sizeof state)) return true;
    if (!power_known || strcmp(state, power_state) != 0) logline("watchdog: the set is %s", state);
    snprintf(power_state, sizeof power_state, "%s", state);
    power_known = true;
    power_active = strcasecmp(state, "active") == 0;
    return true;
}

static void ask_power(void) {
    if (nbuses == 0) return;
    LSError e;
    LSErrorInit(&e);
    if (power_call_live) LSCallCancel(buses[0].sh, power_token, &e);
    power_call_live = LSCall(buses[0].sh, "luna://com.webos.service.tvpower/power2/getPowerState", "{\"subscribe\":true}", on_power, NULL, &power_token, &e);
    if (!power_call_live) logline("watchdog: asking tvpower for the power state: %s", e.message ? e.message : "?");
    LSErrorFree(&e);
    power_asked = mono_now();
}

// ---- the Home watcher's devices ----
static struct { int fd; char path[300]; } inputs[MAX_INPUTS];

static gboolean on_input(GIOChannel *ch, int cond, gpointer data) {
    (void)ch;
    int idx = (int)(intptr_t)data;
    int fd = inputs[idx].fd;
    if (fd < 0) return 0;
    bool gone = (cond & (G_IO_HUP | G_IO_ERR)) != 0;
    struct input_event ev[32];
    while (!gone) {
        ssize_t n = read(fd, ev, sizeof ev);
        if (n < 0) { if (errno != EAGAIN && errno != EINTR) gone = true; break; }
        if (n == 0) break;
        for (size_t i = 0; i < (size_t)n / sizeof ev[0]; i++)
            if (ev[i].type == EV_KEY) home_event(&home_state, ev[i].code, ev[i].value, mono_now());
        if ((size_t)n < sizeof ev) break;
    }
    // A device that goes (a remote's radio dropping) takes its key releases with it: a Home that was down on it is no hold.
    if (gone) { close(fd); inputs[idx].fd = -1; home_state.down_at = 0; return 0; }
    return 1;
}

// Opens the event nodes not yet watched, again every few seconds: a remote that pairs later is a new node. A Home
// key already down on a new node starts its hold.
static void scan_inputs(void) {
    DIR *d = opendir(input_dir);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (strncmp(e->d_name, "event", 5) != 0) continue;
        char path[300];
        snprintf(path, sizeof path, "%s/%s", input_dir, e->d_name);
        int slot = -1;
        bool known = false;
        for (int i = 0; i < MAX_INPUTS; i++) {
            if (inputs[i].fd >= 0 && !strcmp(inputs[i].path, path)) known = true;
            else if (inputs[i].fd < 0 && slot < 0) slot = i;
        }
        if (known) continue;
        if (slot < 0) break;
        int fd = open_input(path);
        if (fd < 0) continue;
        inputs[slot].fd = fd;
        snprintf(inputs[slot].path, sizeof inputs[slot].path, "%s", path);
        if (input_home_down(fd)) home_event(&home_state, KEY_HOMEPAGE, 1, mono_now());
        g_io_add_watch(g_io_channel_unix_new(fd), G_IO_IN | G_IO_ERR | G_IO_HUP, on_input, (gpointer)(intptr_t)slot);
    }
    closedir(d);
}

static gboolean on_home_timer(gpointer data) {
    (void)data;
    if (home_due(&home_state, mono_now())) {
        write_text(path_in_state("escaped"), "hold Home 650 ms (supervisor)");
        fallback("hold Home 650 ms (supervisor)", false);
    }
    return 1;
}

// Every 2 s: new input nodes, tvpower's state when it is not known, then the compositor's counters.
static gboolean on_progress(gpointer data) {
    (void)data;
    double now = mono_now();
    if (now - last_scan >= 6) { last_scan = now; scan_inputs(); }
    if (nbuses > 0 && !power_known && now - power_asked >= 10) ask_power();
    // A heartbeat already late is the heartbeat's to judge; this asks the compositor nothing meanwhile.
    if (bridge_fd < 0 || ping_seq > pong_seq + 1) return 1;
    struct wd_in in = { .now = now, .power_known = power_known, .power_active = power_active };
    if (control_query(runtime_dir, "status", reply_buf, sizeof reply_buf, 150) >= 0) parse_status(reply_buf, &in);
    if (control_query(runtime_dir, "windows", reply_buf, sizeof reply_buf, 150) >= 0 && strstr(reply_buf, "\"windows\":[")) { in.have_windows = true; in.mapped = count_mapped(reply_buf); }
    // Said once both inputs have been read (a single slow answer is not a rule that is off); said otherwise after a
    // minute, so that a watchdog that cannot see what it needs shows in the log and not only in what it never catches.
    if (in.have_flips) seen_flips = true;
    if (in.have_windows) seen_windows = true;
    if (!progress_announced && seen_flips && seen_windows) {
        progress_announced = true;
        logline("watchdog: progress rules on (frozen output watched, empty panel watched)");
    } else if (!progress_announced && !progress_warned && now - started_mono > 60) {
        progress_warned = true;
        logline("watchdog: progress rules not fully on after a minute (frozen output %s, empty panel %s)", seen_flips ? "watched" : "off: no flip counter in `status`", seen_windows ? "watched" : "off: no window list");
    }
    char why[160];
    enum wd_verdict v = wd_step(&wd, &in, why, sizeof why);
    if (v != WD_OK) { fallback(why, true); return 1; }
    if (now - last_report >= 300) {
        last_report = now;
        logline("watchdog: drawn %ld flips %ld mapped %d set %s", in.drawn, in.flips, in.mapped, power_known ? power_state : "unknown");
    }
    return 1;
}

static void on_term(int sig) {
    (void)sig;
    // Stopped on purpose (systemctl restart, shutdown): not an unclean end.
    if (running_path[0]) unlink(running_path);
    if (child_pid > 0) kill(child_pid, SIGTERM);
    _exit(0);
}

// Removes one stale log from the last runs: the file is appended to by every start and every watchdog line.
static void rotate_log(void) {
    struct stat st;
    char p[600], old[620];
    snprintf(p, sizeof p, "%s/sm.log", state_dir);
    if (stat(p, &st) == 0 && st.st_size > 256 * 1024) {
        snprintf(old, sizeof old, "%s.old", p);
        rename(p, old);
    }
}

#ifdef LUMATV_SM_TEST
// The JSON scanner against lines as serde_json writes them on the compositor's side.
static int failures_seen;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #c); failures_seen++; } } while (0)
static void test_json(void) {
    const char *reply = "{\"kind\":\"reply\",\"token\":11,\"payload\":{\"returnValue\":true,\"foregroundAppInfo\":[{\"appId\":\"x,}\",\"params\":{}}],\"subscribed\":true}}";
    char kind[32]; unsigned long tok = 0; const char *v; size_t n;
    CHECK(json_string(reply, "kind", kind, sizeof kind) && !strcmp(kind, "reply"));
    CHECK(json_number(reply, "token", &tok) && tok == 11);
    CHECK(json_value(reply, "payload", &v, &n) && n == strlen("{\"returnValue\":true,\"foregroundAppInfo\":[{\"appId\":\"x,}\",\"params\":{}}],\"subscribed\":true}"));
    CHECK(!json_bool(reply, "subscribe"));
    const char *out = "{\"kind\":\"outbound\",\"token\":7,\"uri\":\"luna://com.webos.service.videooutput/display/setDisplayWindow\",\"payload\":{\"displayOutput\":{\"x\":160,\"y\":0,\"width\":960,\"height\":540},\"context\":\"_ctx\",\"fullScreen\":false},\"subscribe\":true}";
    char uri[256];
    CHECK(json_string(out, "uri", uri, sizeof uri) && !strcmp(uri, "luna://com.webos.service.videooutput/display/setDisplayWindow"));
    CHECK(json_bool(out, "subscribe"));
    CHECK(json_value(out, "payload", &v, &n) && v[0] == '{' && v[n - 1] == '}' && n == 94);
    const char *pong = "{\"kind\":\"pong\",\"seq\":42}";
    CHECK(json_number(pong, "seq", &tok) && tok == 42);
    const char *esc = "{\"kind\":\"fallback\",\"reason\":\"hold \\\"Home\\\" 650 ms\"}";
    char reason[64];
    CHECK(json_string(esc, "reason", reason, sizeof reason) && !strcmp(reason, "hold \"Home\" 650 ms"));
    char fp[17]; fingerprint(fp); CHECK(strlen(fp) == 16);
    char e[64]; json_escape("a\"b\\c\n", e, sizeof e); CHECK(!strcmp(e, "a\\\"b\\\\c\\n"));
    char pretty[] = "{\"kind\":\"inbound\",\"payload\":{\n    \"subscribe\": true\r\n}}";
    flatten(pretty);
    CHECK(!strchr(pretty, '\n') && !strchr(pretty, '\r'));
    CHECK(json_value(pretty, "payload", &v, &n) && v[0] == '{' && v[n - 1] == '}');
    // Lines sent before the compositor connects arrive, in order, once it does.
    send_line("{\"kind\":\"inbound\",\"token\":1}");
    send_line("{\"kind\":\"inbound\",\"token\":2}");
    CHECK(npending == 2);
    int sv[2];
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0);
    bridge_fd = sv[0];
    flush_pending();
    char got[128] = "";
    ssize_t r = read(sv[1], got, sizeof got - 1);
    CHECK(r > 0 && !strcmp(got, "{\"kind\":\"inbound\",\"token\":1}\n{\"kind\":\"inbound\",\"token\":2}\n"));
    CHECK(npending == 0);
    close(sv[0]); close(sv[1]); bridge_fd = -1;
}

// ---- the decision, the markers, the watchdog, the control socket and the Home hold ----
static char tdir[256];
static char made[16][256];
static int nmade;

static void put(const char *name, const char *text) {
    char p[400];
    snprintf(p, sizeof p, "%s/%s", tdir, name);
    FILE *f = fopen(p, "w");
    if (f) { fputs(text, f); fclose(f); }
}

static void drop(const char *name) { char p[400]; snprintf(p, sizeof p, "%s/%s", tdir, name); unlink(p); }

static bool has(const char *name) { char p[400]; snprintf(p, sizeof p, "%s/%s", tdir, name); return file_exists(p); }

static void slurp(const char *name, char *out, size_t n) {
    char p[400];
    snprintf(p, sizeof p, "%s/%s", tdir, name);
    read_first_line(p, out, n);
}

// A state directory with a whole bundle, the current fingerprint recorded and listed as verified, and this boot's id.
static void fresh_state(void) {
    snprintf(tdir, sizeof tdir, "/tmp/lumatv-sm-test-XXXXXX");
    CHECK(mkdtemp(tdir) != NULL);
    state_dir = tdir;
    if (nmade < 16) snprintf(made[nmade++], sizeof made[0], "%s", tdir);
    char p[400];
    snprintf(p, sizeof p, "%s/bin", tdir); mkdir(p, 0755);
    snprintf(p, sizeof p, "%s/lib", tdir); mkdir(p, 0755);
    for (int i = 0; bundle_files[i]; i++) put(bundle_files[i], "x");
    char fp[17], line[64];
    fingerprint(fp);
    snprintf(line, sizeof line, "%s\n", fp);
    put("fingerprint", line);
    snprintf(line, sizeof line, "build test\nfingerprint %s\nend\n", fp);
    put("verified.list", line);
    snprintf(p, sizeof p, "%s/boot_id", tdir);
    FILE *f = fopen(p, "w");
    if (f) { fputs("BOOT-A\n", f); fclose(f); }
    boot_id_path = strdup(p);
}

static void test_decide(void) {
    struct verdict v;
    fresh_state();
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "off") && !strcmp(v.why, "not enabled"));
    put("enabled", "");                                   // what `touch enabled` made: persistent
    decide(&v);
    CHECK(v.run && !strcmp(v.word, "run"));
    put("enabled", "persistent\n");
    decide(&v);
    CHECK(v.run);
    put("escaped", "hold Home 650 ms");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "escaped") && !strcmp(v.why, "escape recorded; re-enable from the tool"));
    drop("escaped");
    // a bundle with a part missing is refused before anything is started
    drop("bin/recover-home.sh");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "bundle") && strstr(v.why, "recover-home.sh"));
    put("bin/recover-home.sh", "x");
    // the firmware: another fingerprint than the one `install` recorded, and a build that is not on the verified list
    put("fingerprint", "0000000000000000\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "firmware") && !strcmp(v.why, "firmware fingerprint differs from the verified one"));
    char fp[17], line[96];
    fingerprint(fp);
    snprintf(line, sizeof line, "%s\n", fp);
    put("fingerprint", line);
    put("verified.list", "build other\nfingerprint 1111111111111111\nend\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "firmware") && !strcmp(v.why, "firmware build is not on the verified list"));
    drop("verified.list");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "firmware"));
    // a session enable of this boot runs on a build that was never verified; one of an earlier boot does not
    put("enabled", "session BOOT-A\n");
    decide(&v);
    CHECK(v.run);
    put("enabled", "session BOOT-B\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "session"));
    put("enabled", "session\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "session"));
    // the verified list: several blocks, the fingerprint on any of them
    snprintf(line, sizeof line, "# list\nbuild a\nfingerprint 1111111111111111\nend\nbuild b\nfingerprint %s\nend\n", fp);
    put("verified.list", line);
    put("enabled", "");
    decide(&v);
    CHECK(v.run);
    CHECK(verified_has(fp) && !verified_has("2222222222222222") && !verified_has(""));
    // failures: two on record, or one and a run that never ended
    put("failures", "1\n");
    decide(&v);
    CHECK(v.run);
    put("running", "BOOT-A 1\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "failures") && !strcmp(v.why, "two consecutive failures"));
    CHECK(failures_effective() == 2);
    drop("running");
    put("failures", "2\n");
    decide(&v);
    CHECK(!v.run && !strcmp(v.word, "failures"));
    // decide() reads and never writes
    put("failures", "1\n");
    put("running", "BOOT-A 1\n");
    decide(&v);
    char buf[64];
    slurp("failures", buf, sizeof buf);
    CHECK(!strcmp(buf, "1") && has("running"));
}

static void test_running_marker(void) {
    fresh_state();
    snprintf(running_path, sizeof running_path, "%s/running", tdir);
    char buf[64];
    consume_unclean();                                    // nothing left over: nothing counted
    CHECK(!has("failures"));
    mark_running();
    CHECK(has("running"));
    slurp("running", buf, sizeof buf);
    CHECK(!strncmp(buf, "BOOT-A ", 7));
    consume_unclean();                                    // the last run never settled or ended: one failure, marker gone
    slurp("failures", buf, sizeof buf);
    CHECK(!strcmp(buf, "1") && !has("running"));
    mark_running();
    forget_running();                                     // a clean end or a deliberate fallback leaves nothing to count
    CHECK(!has("running"));
    consume_unclean();
    slurp("failures", buf, sizeof buf);
    CHECK(!strcmp(buf, "1"));
    running_path[0] = 0;
}

static void test_watchdog(void) {
    struct wd w;
    struct wd_in in;
    char why[160];
    // frames drawn and landing: fine for as long as it lasts
    memset(&w, 0, sizeof w);
    for (int i = 0; i < 60; i++) {
        memset(&in, 0, sizeof in);
        in.now = 100 + i * 2.0; in.have_flips = true; in.drawn = 1000 + i * 120; in.flips = 1000 + i * 120;
        CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    }
    // an idle screen draws nothing and flips nothing, for as long as it likes
    memset(&w, 0, sizeof w);
    for (int i = 0; i < 300; i++) {
        memset(&in, 0, sizeof in);
        in.now = 100 + i * 2.0; in.have_flips = true; in.drawn = 5000; in.flips = 5000;
        CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    }
    // frames drawn and no flip: the third sample after the last good one is 6 s on
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    in.have_flips = true; in.now = 100; in.drawn = 500; in.flips = 500;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 102; in.drawn = 530;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 104; in.drawn = 560;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 106; in.drawn = 590;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_FROZEN);
    CHECK(strstr(why, "output frozen") && strstr(why, "90 frames") && strstr(why, "6 s"));
    // a flip that lands in between is the all-clear
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    in.have_flips = true; in.now = 100; in.drawn = 500; in.flips = 500;
    wd_step(&w, &in, why, sizeof why);
    in.now = 104; in.drawn = 560;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 106; in.drawn = 590; in.flips = 501;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 110; in.drawn = 650;
    CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);    // the clock restarted at the flip
    // a trickle below 20 frames is no freeze, a set that is off is not frozen, and a screen without a flip counter is not asked
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    in.have_flips = true; in.now = 0; in.drawn = 10; in.flips = 10;
    wd_step(&w, &in, why, sizeof why);
    for (int i = 1; i <= 9; i++) { in.now = i * 2.0; in.drawn += 1; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK); }
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    in.have_flips = true; in.power_known = true; in.power_active = false; in.now = 0; in.drawn = 10; in.flips = 10;
    wd_step(&w, &in, why, sizeof why);
    for (int i = 1; i <= 9; i++) { in.now = i * 2.0; in.drawn += 100; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK); }
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    for (int i = 1; i <= 9; i++) { in.now = i * 2.0; in.drawn += 100; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK); }
    // nothing mapped while the set is on: 45 s, restarted by a window, by the set going off or by not knowing
    memset(&w, 0, sizeof w);
    memset(&in, 0, sizeof in);
    in.have_windows = true; in.power_known = true; in.power_active = true; in.mapped = 0;
    in.now = 1000; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 1044; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 1046; CHECK(wd_step(&w, &in, why, sizeof why) == WD_BLANK);
    CHECK(strstr(why, "nothing on the panel") && strstr(why, "46 s"));
    memset(&w, 0, sizeof w);
    in.now = 2000; wd_step(&w, &in, why, sizeof why);
    in.now = 2040; in.mapped = 1; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 2041; in.mapped = 0; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 2080; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);          // 39 s since the window left
    in.now = 2087; CHECK(wd_step(&w, &in, why, sizeof why) == WD_BLANK);
    memset(&w, 0, sizeof w);
    in.now = 3000; in.mapped = 0; wd_step(&w, &in, why, sizeof why);
    in.now = 3030; in.power_active = false; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 3100; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);          // a standby of any length
    in.now = 3101; in.power_active = true; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);   // the 45 s start again at the wake
    in.now = 3140; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);
    in.now = 3147; CHECK(wd_step(&w, &in, why, sizeof why) == WD_BLANK);
    memset(&w, 0, sizeof w);
    in.power_known = false; in.now = 4000; wd_step(&w, &in, why, sizeof why);
    in.now = 4500; CHECK(wd_step(&w, &in, why, sizeof why) == WD_OK);          // tvpower never answered: no guess
}

static void test_status_parsing(void) {
    // the shape of `ctl status` and `ctl windows` on the set (docs/research/compositor/slot-2026-10-25-1606/gate)
    const char *status = "{\"buffers\":\"Driver\",\"check\":{\"bad\":0,\"ok\":0,\"unread\":0},\"clients\":[{\"bound\":[\"wl_compositor\"],\"cmd\":\"/usr/bin/WebAppMgr --flips 9\",\"pid\":3856,\"seconds\":156,\"uid\":505}],\"focus\":2,\"ok\":true,\"output\":\"kms\","
        "\"stats\":{\"buffers\":0,\"callbacks\":3885,\"commits\":3890,\"drawn\":3902,\"frameImports\":3,\"prepared\":4,\"releases\":3881,\"renderUsLast\":252,\"renderUsMax\":11134,\"requests\":15720,\"skipped\":0,\"ticks\":9618},"
        "\"summary\":\"uptime 159.8s clients 3 (total 8) surfaces 5 windows 2 requests 15720 commits 3890 buffers 0 ticks 9618 drawn 3902 callbacks 3885 releases 3881 skipped 0 gl errors 0 render last 252 us max 11134 us prepared 4 frame imports 3 check ok 0 bad 0 unread 0 flips 3902 late 4 lost 5197 direct 0 odd stamps 7\"}";
    struct wd_in in;
    memset(&in, 0, sizeof in);
    parse_status(status, &in);
    CHECK(in.have_flips && in.drawn == 3902 && in.flips == 3902);
    // the offscreen renderer's summary has no flips: nothing to compare
    const char *offscreen = "{\"stats\":{\"drawn\":17,\"ticks\":3},\"summary\":\"uptime 2.0s clients 1 ticks 3 drawn 17 check ok 0 bad 0 unread 0\"}";
    memset(&in, 0, sizeof in);
    parse_status(offscreen, &in);
    CHECK(!in.have_flips);
    memset(&in, 0, sizeof in);
    parse_status("", &in);
    CHECK(!in.have_flips);
    const char *windows = "{\"groups\":[],\"ok\":true,\"video\":[],\"windows\":[{\"appId\":\"com.webos.app.home\",\"id\":2,\"keyMask\":4286578682,\"mapped\":true,\"position\":[0,0]},{\"appId\":\"com.webos.app.hdmi2\",\"id\":3,\"keyMask\":1,\"mapped\":false,\"position\":[0,0]}]}";
    CHECK(count_mapped(windows) == 1);
    CHECK(count_mapped("{\"ok\":true,\"windows\":[]}") == 0);
    CHECK(count_mapped("{\"windows\":[{\"mapped\":true},{\"mapped\":true},{\"mapped\":false}]}") == 2);
}

static pid_t serve(const char *dir, int mode) {
    // mode 0: answer at once; 1: answer in two pieces 80 ms apart; 2: accept and stay silent; 3: a reply with no newline
    char path[300];
    struct sockaddr_un a = { .sun_family = AF_UNIX };
    snprintf(path, sizeof path, "%s/lumatv-control", dir);
    unlink(path);
    snprintf(a.sun_path, sizeof a.sun_path, "%s", path);
    int ls = socket(AF_UNIX, SOCK_STREAM, 0);
    CHECK(ls >= 0 && bind(ls, (struct sockaddr *)&a, sizeof a) == 0 && listen(ls, 4) == 0);
    pid_t p = fork();
    if (p == 0) {
        int c = accept(ls, NULL, NULL);
        char cmd[64] = "";
        ssize_t n = c >= 0 ? read(c, cmd, sizeof cmd - 1) : -1;
        if (n > 0) cmd[n] = 0;
        if (mode == 0) dprintf(c, "{\"ok\":true,\"cmd\":\"%.*s\"}\n", (int)strcspn(cmd, "\n"), cmd);
        else if (mode == 1) { dprintf(c, "{\"ok\":true,"); usleep(80000); dprintf(c, "\"x\":1}\n"); }
        else if (mode == 2) usleep(1500000);
        else if (mode == 3) { for (int i = 0; i < 40; i++) dprintf(c, "0123456789"); usleep(300000); }
        _exit(0);
    }
    close(ls);
    return p;
}

static void test_control_query(void) {
    fresh_state();
    char out[512];
    CHECK(control_query(tdir, "status", out, sizeof out, 150) == -1);          // nobody listens
    pid_t p = serve(tdir, 0);
    int n = control_query(tdir, "status", out, sizeof out, 500);
    CHECK(n > 0 && !strcmp(out, "{\"ok\":true,\"cmd\":\"status\"}"));
    waitpid(p, NULL, 0);
    p = serve(tdir, 1);
    n = control_query(tdir, "windows", out, sizeof out, 500);
    CHECK(n > 0 && !strcmp(out, "{\"ok\":true,\"x\":1}"));                     // a reply that arrives in two pieces
    waitpid(p, NULL, 0);
    p = serve(tdir, 2);
    double t0 = mono_now();
    n = control_query(tdir, "status", out, sizeof out, 150);
    double took = mono_now() - t0;
    CHECK(n == -1 && took >= 0.14 && took < 0.5);                             // a stuck compositor costs one bounded wait
    kill(p, SIGKILL);
    waitpid(p, NULL, 0);
    p = serve(tdir, 3);
    char small[64];
    n = control_query(tdir, "status", small, sizeof small, 500);
    CHECK(n == 63 && strlen(small) == 63);                                     // a reply with no newline fills the buffer and stops
    kill(p, SIGKILL);
    waitpid(p, NULL, 0);
}

static void test_home(void) {
    struct home h = { 0 };
    CHECK(!home_due(&h, 100));
    home_event(&h, KEY_HOMEPAGE, 1, 100.0);
    CHECK(!home_due(&h, 100.0) && !home_due(&h, 100.6) && home_due(&h, 100.65) && home_due(&h, 105));
    home_event(&h, KEY_HOMEPAGE, 2, 100.5);                                    // a repeat does not restart the hold
    CHECK(home_due(&h, 100.65));
    home_event(&h, KEY_HOMEPAGE, 0, 100.7);
    CHECK(!home_due(&h, 105));                                                // a release ends it
    home_event(&h, KEY_ENTER, 1, 200.0);
    CHECK(!home_due(&h, 205));                                                // other keys do not count
    home_event(&h, KEY_HOME, 1, 300.0);                                       // the older code counts as well
    CHECK(home_due(&h, 300.7));
    home_event(&h, KEY_HOME, 0, 300.3);
    CHECK(!home_due(&h, 301));                                                // a tap is not a hold
    home_event(&h, KEY_RF_HOME, 1, 400.0);                                    // LG's RF_HOME counts as well
    CHECK(!home_due(&h, 400.6) && home_due(&h, 400.7) && home_due(&h, 405));
    home_event(&h, KEY_RF_HOME, 2, 400.5);                                    // a repeat does not restart it
    CHECK(home_due(&h, 400.7));
    home_event(&h, KEY_RF_HOME, 0, 400.8);
    CHECK(!home_due(&h, 405));
    home_event(&h, KEY_RF_HOME, 1, 500.0);
    home_event(&h, KEY_RF_HOME, 0, 500.3);
    CHECK(!home_due(&h, 501));                                                // a tap is not a hold
    // the key-state bitmap of EVIOCGKEY: each of the three Home codes is found, other keys' bits are not Home, and a bit past the
    // end of a short answer is not read
    unsigned char bits[128];
    memset(bits, 0, sizeof bits);
    CHECK(!home_bits_set(bits, sizeof bits));
    bits[KEY_ENTER / 8] |= 1 << (KEY_ENTER % 8);
    bits[KEY_MENU / 8] |= 1 << (KEY_MENU % 8);
    CHECK(!home_bits_set(bits, sizeof bits));
    int homes[] = { KEY_HOME, KEY_HOMEPAGE, KEY_RF_HOME };
    for (size_t i = 0; i < sizeof homes / sizeof homes[0]; i++) {
        unsigned char one[128];
        memset(one, 0, sizeof one);
        one[homes[i] / 8] |= 1 << (homes[i] % 8);
        CHECK(home_bits_set(one, sizeof one));
        CHECK(!home_bits_set(one, (size_t)homes[i] / 8));                     // the answer ends before the key's byte
        CHECK(home_bits_set(one, (size_t)homes[i] / 8 + 1));
    }
    CHECK(name_is_network_keys("Smart Remote RCU Input") && name_is_network_keys("LGE Network Input"));
    CHECK(!name_is_network_keys("LGE RCU") && !name_is_network_keys("LGE M-RCU - Builtin [0]"));
    // no event nodes, or ones that are no input devices: Home is not held, and nothing is waited for
    fresh_state();
    char dir[320];
    snprintf(dir, sizeof dir, "%s/input", tdir);
    mkdir(dir, 0755);
    input_dir = strdup(dir);
    double t0 = mono_now();
    CHECK(!home_held_for(700) && mono_now() - t0 < 0.1);
    put("input/event0", "not a device");
    put("input/mouse0", "not an event node");
    int fds[MAX_INPUTS];
    CHECK(open_inputs_all(fds, MAX_INPUTS) == 1);
    CHECK(!input_home_down(fds[0]));
    close(fds[0]);
    t0 = mono_now();
    CHECK(!home_held_for(700) && mono_now() - t0 < 0.1);
}

// What the watcher reads from a device: Home's press and release in the layout of this build's input_event, other keys
// ignored, and a device that goes taking a Home held on it with it.
static void test_input_reads(void) {
    for (int i = 0; i < MAX_INPUTS; i++) inputs[i].fd = -1;
    memset(&home_state, 0, sizeof home_state);
    int p[2];
    CHECK(pipe2(p, O_NONBLOCK) == 0);
    inputs[0].fd = p[0];
    struct input_event ev[2];
    memset(ev, 0, sizeof ev);
    ev[0].type = EV_KEY; ev[0].code = KEY_ENTER; ev[0].value = 1;
    ev[1].type = EV_KEY; ev[1].code = KEY_HOMEPAGE; ev[1].value = 1;
    CHECK(write(p[1], ev, sizeof ev) == (ssize_t)sizeof ev);
    CHECK(on_input(NULL, G_IO_IN, (gpointer)(intptr_t)0) == 1);
    CHECK(home_state.down_at != 0);
    ev[0].code = KEY_HOMEPAGE; ev[0].value = 0;
    CHECK(write(p[1], ev, sizeof ev[0]) == (ssize_t)sizeof ev[0]);
    CHECK(on_input(NULL, G_IO_IN, (gpointer)(intptr_t)0) == 1 && home_state.down_at == 0);
    ev[0].code = KEY_RF_HOME; ev[0].value = 1;                          // LG's RF_HOME is Home on a remote as well
    CHECK(write(p[1], ev, sizeof ev[0]) == (ssize_t)sizeof ev[0]);
    CHECK(on_input(NULL, G_IO_IN, (gpointer)(intptr_t)0) == 1 && home_state.down_at != 0);
    ev[0].value = 0;
    CHECK(write(p[1], ev, sizeof ev[0]) == (ssize_t)sizeof ev[0]);
    CHECK(on_input(NULL, G_IO_IN, (gpointer)(intptr_t)0) == 1 && home_state.down_at == 0);
    ev[0].code = KEY_HOMEPAGE;
    ev[0].value = 2;                                                    // a repeat is the key still down
    CHECK(write(p[1], ev, sizeof ev[0]) == (ssize_t)sizeof ev[0]);
    CHECK(on_input(NULL, G_IO_IN, (gpointer)(intptr_t)0) == 1 && home_state.down_at != 0);
    CHECK(on_input(NULL, G_IO_HUP, (gpointer)(intptr_t)0) == 0);        // the device is gone with Home down on it
    CHECK(inputs[0].fd == -1 && home_state.down_at == 0);
    close(p[1]);
}

static void test_log_rotation(void) {
    fresh_state();
    put("sm.log", "short\n");
    rotate_log();
    CHECK(has("sm.log") && !has("sm.log.old"));
    char *big = malloc(300 * 1024);
    memset(big, 'x', 300 * 1024 - 1);
    big[300 * 1024 - 1] = 0;
    put("sm.log", big);
    free(big);
    rotate_log();
    CHECK(!has("sm.log") && has("sm.log.old"));
}

// The bridge's hang-up of a compositor that crashed: the reason is the reaped status, not the socket. A child holding
// one end of a socketpair is killed with SIGKILL (the gate's wd-crash), the hang-up is seen first, then the reaping.
static pid_t bridge_child(int sv[2], int how) {
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0);
    pid_t p = fork();
    if (p == 0) {
        close(sv[0]);
        if (how == 1) close(sv[1]);                                      // alive, its end of the bridge closed
        if (how == 2) _exit(3);
        for (;;) pause();
    }
    close(sv[1]);
    return p;
}

static bool hung_up(int fd) {
    struct pollfd p = { .fd = fd, .events = POLLIN };
    return poll(&p, 1, 2000) == 1 && (p.revents & (POLLHUP | POLLIN));
}

static void test_bridge_gone(void) {
    int sv[2];
    char reason[128] = "";
    child_pid = bridge_child(sv, 0);
    kill(child_pid, SIGKILL);
    CHECK(hung_up(sv[0]));
    CHECK(reap_child_within(BRIDGE_REAP_MS, reason, sizeof reason));
    CHECK(!strcmp(reason, "compositor killed by signal 9") && child_pid == 0);
    close(sv[0]);

    reason[0] = 0;
    child_pid = bridge_child(sv, 2);
    CHECK(hung_up(sv[0]));
    CHECK(reap_child_within(BRIDGE_REAP_MS, reason, sizeof reason));
    CHECK(!strcmp(reason, "compositor exited with 3") && child_pid == 0);
    close(sv[0]);

    // Alive after closing its end: not reaped, the reason left to the bridge's words, the child still supervised.
    reason[0] = 0;
    pid_t alive = bridge_child(sv, 1);
    child_pid = alive;
    CHECK(hung_up(sv[0]));
    CHECK(!reap_child_within(50, reason, sizeof reason));
    CHECK(child_pid == alive && reason[0] == 0);
    kill(alive, SIGKILL);
    waitpid(alive, NULL, 0);
    child_pid = 0;
    close(sv[0]);
}

// ---- the crash bundle: the reason, the last log lines, the trace ring in order, rotation ----
static char big[32768];

static void put_raw(const char *path, const char *text) {
    FILE *f = fopen(path, "w");
    if (f) { fputs(text, f); fclose(f); }
}

static void slurp_path(const char *p, char *out, size_t n) {
    FILE *f = fopen(p, "r");
    size_t r = 0;
    if (f) { r = fread(out, 1, n - 1, f); fclose(f); }
    out[r] = 0;
}

static int count_lines(const char *s) { int n = 0; for (; *s; s++) if (*s == '\n') n++; return n; }

// A trace ring file as src/ring.rs writes it: "LTVRING1", the area's size and the bytes ever written (little endian), text from 64 on, five-byte lines t000...
static void make_ring(const char *path, size_t area, int lines) {
    unsigned char *img = calloc(1, RING_HEADER + area);
    memcpy(img, "LTVRING1", 8);
    uint64_t head = 0;
    for (int i = 0; i < lines; i++) {
        char l[16];
        int n = snprintf(l, sizeof l, "t%03d\n", i);
        for (int k = 0; k < n; k++) img[RING_HEADER + (head++ % area)] = (unsigned char)l[k];
    }
    for (int i = 0; i < 8; i++) { img[8 + i] = (unsigned char)((uint64_t)area >> (8 * i)); img[16 + i] = (unsigned char)(head >> (8 * i)); }
    FILE *f = fopen(path, "w");
    if (f) { fwrite(img, 1, RING_HEADER + area, f); fclose(f); }
    free(img);
}

static int count_bundles(const char *root) {
    int n = 0;
    DIR *d = opendir(root);
    struct dirent *e;
    while (d && (e = readdir(d))) if (e->d_name[0] != '.' && strcmp(e->d_name, "latest") && strcmp(e->d_name, "pending")) n++;
    if (d) closedir(d);
    return n;
}

static void test_crash_bundle(void) {
    fresh_state();
    char root[400], file[600], dir[800], name[64], first[64], text[512];
    snprintf(root, sizeof root, "%s/crash", tdir);
    crash_root = strdup(root);
    runtime_dir = tdir;
    started_mono = mono_now() - 42;
    FILE *f = fopen(path_in_state("compositor.log"), "w");
    for (int i = 1; i <= 300; i++) fprintf(f, "[srv %d.000] info line %d\n", i, i);
    fclose(f);
    put("sm.log", "[00:00:01] start\n[00:00:02] registered com.webos.x\n");
    snprintf(file, sizeof file, "%s/lumatv-trace.ring", tdir);
    make_ring(file, 64, 20);                              // 100 bytes written into 64: wrapped, the newest twelve and a half lines are there

    crash_bundle("compositor killed by signal 11", true, name, sizeof name);
    snprintf(first, sizeof first, "%s", name);
    CHECK(name[0] != 0);
    snprintf(dir, sizeof dir, "%s/%s", root, name);
    snprintf(file, sizeof file, "%s/reason.txt", dir); slurp_path(file, text, sizeof text);
    CHECK(strstr(text, "reason: compositor killed by signal 11\n") && strstr(text, "kind: problem\n") && strstr(text, "compositor uptime: 42 s\n") && strstr(text, "boot: BOOT-A"));
    snprintf(file, sizeof file, "%s/compositor.log.tail", dir); slurp_path(file, big, sizeof big);
    CHECK(count_lines(big) == CRASH_LOG_LINES);
    CHECK(!strncmp(big, "[srv 101.000] info line 101\n", 28) && strstr(big, "info line 300\n") != NULL);
    snprintf(file, sizeof file, "%s/sm.log.tail", dir); slurp_path(file, text, sizeof text);
    CHECK(strstr(text, "registered com.webos.x") != NULL);
    snprintf(file, sizeof file, "%s/trace.txt", dir); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, "t008\nt009\nt010\nt011\nt012\nt013\nt014\nt015\nt016\nt017\nt018\nt019\n"));    // in order; the line the wrap cut is left out
    snprintf(file, sizeof file, "%s/system.txt", dir); slurp_path(file, text, sizeof text);
    CHECK(text[0] != 0);
    snprintf(file, sizeof file, "%s/latest", root); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, name));
    snprintf(file, sizeof file, "%s/pending", root); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, name));

    // A hand-back the owner asked for leaves a bundle and no `pending` of its own: the first problem is still the one waiting.
    crash_bundle("hold Home 650 ms", false, name, sizeof name);
    CHECK(name[0] != 0 && strcmp(name, first) != 0);
    snprintf(file, sizeof file, "%s/%s/reason.txt", root, name); slurp_path(file, text, sizeof text);
    CHECK(strstr(text, "kind: escape\n") != NULL);
    snprintf(file, sizeof file, "%s/latest", root); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, name));
    snprintf(file, sizeof file, "%s/pending", root); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, first));

    // Only the newest CRASH_KEEP are kept, bounded in size, and the newest is the last made.
    for (int i = 0; i < 12; i++) crash_bundle("heartbeat: two beats missed", true, name, sizeof name);
    CHECK(count_bundles(root) == CRASH_KEEP);
    snprintf(file, sizeof file, "%s/%s", root, first);
    CHECK(!file_exists(file));
    snprintf(file, sizeof file, "%s/%s/reason.txt", root, name);
    CHECK(file_exists(file));
    snprintf(file, sizeof file, "%s/pending", root); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, name));

    // No ring, no log: the bundle still says why.
    snprintf(file, sizeof file, "%s/lumatv-trace.ring", tdir); unlink(file);
    unlink(path_in_state("compositor.log"));
    crash_bundle("compositor exited with 3", true, name, sizeof name);
    snprintf(file, sizeof file, "%s/%s/trace.txt", root, name); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, "no trace ring\n"));
    snprintf(file, sizeof file, "%s/%s/reason.txt", root, name); slurp_path(file, text, sizeof text);
    CHECK(strstr(text, "compositor exited with 3") != NULL);
    // A file that is no ring is no ring.
    snprintf(file, sizeof file, "%s/lumatv-trace.ring", tdir); put_raw(file, "not a ring at all, just text, and long enough to hold a header of sixty-four bytes");
    crash_bundle("compositor exited with 4", true, name, sizeof name);
    snprintf(file, sizeof file, "%s/%s/trace.txt", root, name); slurp_path(file, text, sizeof text);
    CHECK(!strcmp(text, "no trace ring\n"));
    crash_root = NULL;
    runtime_dir = "/tmp/xdg";
}

int main(void) {
    test_json();
    test_decide();
    test_running_marker();
    test_watchdog();
    test_status_parsing();
    test_control_query();
    test_home();
    test_input_reads();
    test_bridge_gone();
    test_log_rotation();
    test_crash_bundle();
    for (int i = 0; i < nmade; i++) { char cmd[300]; snprintf(cmd, sizeof cmd, "rm -rf %s", made[i]); if (system(cmd) != 0) failures_seen++; }
    printf("lumatv-sm json tests: %s\n", failures_seen ? "FAILED" : "ok");
    return failures_seen ? 1 : 0;
}
#else
int main(int argc, char **argv) {
    saved_argv = argv;
    bool would_run = false, home_held = false;
    int home_ms = 700;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--dry-run")) dry_run = true;
        else if (!strcmp(argv[i], "--probe")) probe = true;
        else if (!strcmp(argv[i], "--state") && i + 1 < argc) state_dir = argv[++i];
        else if (!strcmp(argv[i], "--print-fingerprint")) { char fp[17]; fingerprint(fp); printf("%s\n", fp); return 0; }
        else if (!strcmp(argv[i], "--would-run")) would_run = true;
        else if (!strcmp(argv[i], "--home-held")) { home_held = true; if (i + 1 < argc && isdigit((unsigned char)argv[i + 1][0])) home_ms = atoi(argv[++i]); }
    }
    // The two questions the early boot hook asks before it mounts anything: read-only, no log, no state written.
    if (would_run) {
        struct verdict v;
        decide(&v);
        if (v.run) { printf("run\n"); return 0; }
        printf("no %s: %s\n", v.word, v.why);
        return 3;
    }
    if (home_held) return home_held_for(home_ms) ? 0 : 1;
    mkdir(state_dir, 0755);
    rotate_log();
    logf = fopen(path_in_state("sm.log"), "a");
    started_mono = mono_now();
    signal(SIGPIPE, SIG_IGN);
    if (probe) {
        char exe[256] = "?";
        ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
        if (n > 0) exe[n] = 0;
        logline("probe from %s", exe);
        printf("probe from %s\n", exe);
        return probe_buses();
    }

    // 1. Decide. The start line says what the decision starts from (failures counting a run that never settled), and a
    // run that never settled or ended cleanly is counted after it, so that the line stays in this start's own part of the log.
    char fp[17];
    fingerprint(fp);
    char want[64] = "";
    FILE *f = fopen(path_in_state("fingerprint"), "r");
    if (f) { if (fscanf(f, "%63s", want) != 1) want[0] = 0; fclose(f); }
    long failures = dry_run ? read_number(path_in_state("failures"), 0) : failures_effective();
    logline("start: enabled %d fingerprint %s (verified %s) failures %ld escaped %d dry-run %d flag %d on-verified-list %d", file_exists(path_in_state("enabled")), fp, want[0] ? want : "none", failures, file_exists(path_in_state("escaped")), dry_run, (int)read_flag(), verified_has(fp));
    if (!dry_run) consume_unclean();
    if (!dry_run) {
        struct verdict v;
        decide(&v);
        if (!v.run) fallback(v.why, false);
        // Home held down at this moment (a boot with the key held, a restart under a hand on it) is the escape
        // before anything of LumaTV's has started: nothing of LG's has been disturbed, so it is a plain hand-back.
        if (home_held_for(700)) {
            write_text(path_in_state("escaped"), "hold Home at start");
            fallback("hold Home at start", false);
        }
        snprintf(running_path, sizeof running_path, "%s", path_in_state("running"));
        mark_running();
        signal(SIGTERM, on_term);
        signal(SIGINT, on_term);
    }

    // 2 to 4. Supervise, hold Home, bridge. From here a fallback also brings LG's Home back (spawn_recovery).
    supervising = true;
    for (int i = 0; i < MAX_INPUTS; i++) inputs[i].fd = -1;
    loop = g_main_loop_new(NULL, 0);
    open_bridge();
    register_buses();
    spawn_compositor();
    g_timeout_add(500, on_heartbeat, NULL);
    if (!dry_run) {
        last_report = mono_now();
        scan_inputs();
        g_timeout_add(100, on_home_timer, NULL);
        g_timeout_add(2000, on_progress, NULL);
    }
    g_main_loop_run(loop);
    return 0;
}
#endif
