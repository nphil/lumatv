// lumatv-mm: the process systemd starts as /usr/sbin/memchute while LumaTV's memory manager is switched on
// (tools/mm.sh binds it over LG's). It owns com.webos.memorymanager and answers every method its callers use from
// what it holds itself, so no caller ever waits on the helper: SAM waits on setRequiredMemory with no timeout and a
// late answer is a launch that hangs. Policy (closes, groups, swappiness, levels, the preload gate) is the helper's
// (helper/app/src/app_priority/mm.rs), told over a unix socket; docs/research/lg-memchute.md is the design.
//
// It is 32-bit and dynamically linked against the set's libluna-service2, because ls-hubd grants the name to the
// executable at LG's path and a 64-bit process cannot load LG's 32-bit libraries.
//
// On trouble it gives the name back in the same process: removes its bind mount (MNT_DETACH) and execs LG's memchute
// with the same arguments, so the unit, its dependants and the role stay LG's and nothing is restarted. LG's starts
// as on a respawn: /tmp/memchute_ready is left in place, which is what memchute-ready.sh's "Just Respawn" reads.
//
// JSON lines on the bridge socket (STATE/bridge.sock), one connection, from the helper:
//   {"kind":"pong","seq":N}
//   {"kind":"state","level":L,"prevLevel":L,"limit":B,"swapUsed":MB,"units":[...],"policy":{...}}
//   {"kind":"signal","name":"thresholdChanged"|"clearContainers","payload":P}
//   {"kind":"closing","id":APP,"appType":T}        pushed to getCloseAppId subscribers before SAM closes APP
//   {"kind":"outbound","uri":U,"payload":P}         a call made as com.webos.memorymanager (setPreloadStatus, closeMediaPipeline)
//   {"kind":"fallback","reason":R}                  give the name back to LG's (not counted as a failure)
//   {"kind":"bye"}                                   the helper is stopping: wait up to 60 s for it, then give back uncounted
// to the helper:
//   {"kind":"hello","pid":N}   {"kind":"ping","seq":N}   {"kind":"launch","sender":S,"payload":P}
//
// State files in STATE (/var/lib/lumatv/mm): enabled (written by tools/mm.sh on), failures ("<boot id> <uptime s>
// <reason>" lines), running ("<boot id>" while a run is live: one found at start from this boot is an unclean end and
// counts as a failure), last-fallback, crash/<time>-<n>/ records with crash/pending naming the newest unread one.
// Three failures within ten minutes of one boot write gave-up ("<boot id>"), and every start of that boot execs LG's at
// once; tools/mm.sh on (Try again) removes it.
//
// Usage: lumatv-mm [--state DIR] [--would-run | --probe]
//   --would-run read-only: "run" (exit 0) or "no: reason" (exit 3), what a start would decide now. Any other exit
//               means the binary cannot run here (a missing library), so tools/mm.sh never places the mount.
//   --probe     run from LG's path with LG's memchute still serving: exit 0 when the hub would give this executable
//               the name (it answers "already exists"), 2 when the role refuses it. ls-hubd checks the role first.
// SIGUSR1 hands back to LG's without counting a failure (tools/mm.sh off); SIGTERM ends a run cleanly.
#define _GNU_SOURCE
#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#include "../../logging/producer.h"

// ---- luna-service2 (public API, webosose/luna-service2 lunaservice.h) ----
typedef struct LSHandle LSHandle;
typedef struct LSMessage LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError { int error_code; char *message; const char *file; int line; const char *func; void *padding; unsigned long magic; } LSError;
typedef bool (*LSMethodFunction)(LSHandle *sh, LSMessage *msg, void *category_context);
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef struct { const char *name; LSMethodFunction function; int flags; } LSMethod;
typedef struct { const char *name; int flags; } LSSignal;
typedef struct GMainLoop GMainLoop;
bool LSErrorInit(LSError *e);
void LSErrorFree(LSError *e);
bool LSRegister(const char *name, LSHandle **sh, LSError *e);
bool LSUnregister(LSHandle *sh, LSError *e);
bool LSRegisterCategory(LSHandle *sh, const char *category, LSMethod *methods, LSSignal *signals, void *properties, LSError *e);
bool LSGmainAttach(LSHandle *sh, GMainLoop *loop, LSError *e);
bool LSCallOneReply(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e);
bool LSSignalSend(LSHandle *sh, const char *uri, const char *payload, LSError *e);
const char *LSMessageGetPayload(LSMessage *m);
const char *LSMessageGetMethod(LSMessage *m);
const char *LSMessageGetSender(LSMessage *m);
const char *LSMessageGetSenderServiceName(LSMessage *m);
bool LSMessageIsSubscription(LSMessage *m);
bool LSMessageReply(LSHandle *sh, LSMessage *m, const char *payload, LSError *e);
bool LSSubscriptionAdd(LSHandle *sh, const char *key, LSMessage *m, LSError *e);
bool LSSubscriptionReply(LSHandle *sh, const char *key, const char *payload, LSError *e);

// ---- glib (only what the loop needs) ----
typedef struct GIOChannel GIOChannel;
typedef int gboolean;
typedef void *gpointer;
typedef gboolean (*GSourceFunc)(gpointer);
typedef gboolean (*GIOFunc)(GIOChannel *, int condition, gpointer);
enum { G_IO_IN = 1, G_IO_ERR = 8, G_IO_HUP = 16 };
GMainLoop *g_main_loop_new(void *context, gboolean running);
void g_main_loop_run(GMainLoop *loop);
GIOChannel *g_io_channel_unix_new(int fd);
unsigned g_io_add_watch(GIOChannel *ch, int condition, GIOFunc fn, gpointer data);
unsigned g_timeout_add(unsigned ms, GSourceFunc fn, gpointer data);

#define SERVICE "com.webos.memorymanager"
#define FAIL_LIMIT 3
#define FAIL_WINDOW_S 600.0
#define CRASH_KEEP 8
#define CRASH_LOG_LINES 300
#define CALLS_KEPT 20
// A helper restarting (an install) is back well inside these; one that is not is no memory manager.
#define HELPER_FIRST_S 20.0
#define HELPER_GONE_S 10.0
// A helper that said it is stopping (a restart, an install) may take longer, and its absence is no fault of the shim.
#define HELPER_BYE_S 60.0
#define PING_EVERY_S 5.0
// The helper sends its state at least every 5 s; older than this and the kernel's own numbers answer.
#define STATE_FRESH_S 10.0

static const char *state_dir = "/var/lib/lumatv/mm";
static const char *lg_binary = "/usr/sbin/memchute";
static const char *usable_path = "/sys/kernel/mm/low_mem_notify/usable_pages";
static const char *boot_id_path = "/proc/sys/kernel/random/boot_id";
static const char *uptime_path = "/proc/uptime";
static const char *log_directory = "/tmp/lumatv-logs";

static char **saved_argv;
static GMainLoop *loop;
static LSHandle *bus;
static int bridge_listen = -1, bridge_fd = -1;
static bool ever_connected, helper_bye;
static double started_at, gone_at, last_ping;
static unsigned long ping_seq, pong_seq;
static volatile sig_atomic_t switched_off;

static void logline(const char *fmt, ...) {
    char message[4000];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(message, sizeof message, fmt, ap);
    va_end(ap);
    lumatv_log("lumatv-mm", 6, "%s", message);
}

static double mono_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static const char *in_state(const char *name) {
    static char buf[4][600];
    static int i;
    i = (i + 1) % 4;
    snprintf(buf[i], sizeof buf[i], "%s/%s", state_dir, name);
    return buf[i];
}

static bool file_exists(const char *p) { struct stat st; return stat(p, &st) == 0; }

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

static void write_text(const char *p, const char *text) {
    char tmp[700];
    snprintf(tmp, sizeof tmp, "%s.new", p);
    FILE *f = fopen(tmp, "w");
    if (!f) { logline("write %s: %s", p, strerror(errno)); return; }
    fputs(text, f);
    fclose(f);
    if (rename(tmp, p) != 0) logline("rename %s: %s", p, strerror(errno));
}

static void mkdir_p(const char *path) {
    char tmp[600];
    snprintf(tmp, sizeof tmp, "%s", path);
    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') { *p = 0; mkdir(tmp, 0755); *p = '/'; }
    }
    mkdir(tmp, 0755);
}

static void boot_id(char *out, size_t n) {
    if (!read_first_line(boot_id_path, out, n) || !out[0]) snprintf(out, n, "unknown");
}

// Seconds since boot: failures are judged on it, since the wall clock may not be set yet early in a boot.
static double uptime_now(void) {
    char line[64];
    return read_first_line(uptime_path, line, sizeof line) ? strtod(line, NULL) : 0;
}

// ---- levels: LG's thresholds for this set (configd memorymanager, distro hd2160-normal), in 4 KB pages ----
// A level is entered when usable falls below its enter value and left when usable rises above its leave value; the
// gap between the two keeps a level from flapping. Reboot is never entered (its enter value is 0).
static const char *const level_names[] = { "normal", "medium", "low", "critical", "reboot" };
static const long enter_pages[] = { 0, 64000, 51200, 11706, 0 };
static const long leave_pages[] = { 0, 74453, 56320, 17033, 2978 };

static int level_step(int level, long usable) {
    while (level < 4 && usable < enter_pages[level + 1]) level++;
    while (level > 0 && usable > leave_pages[level]) level--;
    return level;
}

static long usable_pages(void) {
    char line[64];
    return read_first_line(usable_path, line, sizeof line) ? strtol(line, NULL, 10) : -1;
}

static long pages_mb(long pages) { return pages < 0 ? 0 : pages / 256; }

// The kernel's own level, kept current by the tick, for when the helper's word is stale.
static int local_level, local_prev;

static void local_update(void) {
    long u = usable_pages();
    if (u < 0) return;
    int next = level_step(local_level, u);
    if (next != local_level) { local_prev = local_level; local_level = next; }
}

// ---- what the helper last said ----
static struct {
    bool have;
    double at;
    char level[16], prev[16];
    bool limit;
    long swap_used;
    char *units, *policy;
} hs;

static bool state_fresh(void) { return hs.have && mono_now() - hs.at < STATE_FRESH_S; }

// ---- a minimal JSON scanner (top-level keys only), as lumatv-sm's ----
static bool json_value(const char *obj, const char *key, const char **start, size_t *len) {
    const char *p = obj;
    while (*p && *p != '{') p++;
    if (*p != '{') return false;
    p++;
    size_t klen = strlen(key);
    while (*p) {
        while (*p == ' ' || *p == ',' || *p == '\n' || *p == '\t' || *p == '\r') p++;
        if (*p != '"') return false;
        const char *k = ++p;
        while (*p && *p != '"') { if (*p == '\\' && p[1]) p++; p++; }
        size_t kl = (size_t)(p - k);
        if (*p) p++;
        while (*p == ' ' || *p == ':' || *p == '\n' || *p == '\t' || *p == '\r') p++;
        const char *v = p;
        int depth = 0;
        if (*p == '"') {
            p++;
            while (*p && *p != '"') { if (*p == '\\' && p[1]) p++; p++; }
            if (*p == '"') p++;
        } else if (*p == '{' || *p == '[') {
            do {
                if (*p == '"') { p++; while (*p && *p != '"') { if (*p == '\\' && p[1]) p++; p++; } }
                else if (*p == '{' || *p == '[') depth++;
                else if (*p == '}' || *p == ']') depth--;
                if (*p) p++;
            } while (*p && depth > 0);
        } else {
            while (*p && *p != ',' && *p != '}') p++;
        }
        if (kl == klen && !strncmp(k, key, klen)) {
            *start = v;
            *len = (size_t)(p - v);
            while (*len && (v[*len - 1] == ' ' || v[*len - 1] == '\n')) (*len)--;
            return true;
        }
    }
    return false;
}

static bool json_string(const char *obj, const char *key, char *out, size_t outsz) {
    const char *v; size_t n;
    if (!json_value(obj, key, &v, &n) || n < 2 || v[0] != '"') return false;
    size_t o = 0;
    for (size_t i = 1; i + 1 < n && o + 1 < outsz; i++) {
        char c = v[i];
        if (c == '\\' && i + 2 < n) { i++; c = v[i] == 'n' ? '\n' : v[i] == 't' ? '\t' : v[i]; }
        out[o++] = c;
    }
    out[o] = 0;
    return true;
}

static bool json_number(const char *obj, const char *key, long *out) {
    const char *v; size_t n;
    if (!json_value(obj, key, &v, &n) || n == 0) return false;
    if (*v == '"') v++;
    char *end;
    long x = strtol(v, &end, 10);
    if (end == v) return false;
    *out = x;
    return true;
}

static bool json_bool(const char *obj, const char *key) {
    const char *v; size_t n;
    return json_value(obj, key, &v, &n) && n >= 4 && !strncmp(v, "true", 4);
}

static void json_escape(const char *s, char *out, size_t outsz) {
    size_t o = 0;
    for (; *s && o + 3 < outsz; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') { out[o++] = '\\'; out[o++] = (char)c; }
        else if (c < 0x20) out[o++] = ' ';
        else out[o++] = (char)c;
    }
    out[o] = 0;
}

// A raw newline in valid JSON is whitespace between tokens: a space keeps the meaning and the line whole.
static void flatten(char *s) {
    for (; *s; s++) if (*s == '\n' || *s == '\r') *s = ' ';
}

// ---- the last calls, for a crash record ----
static struct call { double at; char method[32]; char sender[96]; } calls[CALLS_KEPT];
static int calls_next;

static void remember_call(const char *method, const char *sender) {
    struct call *c = &calls[calls_next++ % CALLS_KEPT];
    c->at = uptime_now();
    snprintf(c->method, sizeof c->method, "%s", method);
    snprintf(c->sender, sizeof c->sender, "%s", sender);
}

// ---- failures ----
// Failures of this boot within the window, from the file's "<boot id> <uptime> <reason>" lines.
static int recent_failures(void) {
    FILE *f = fopen(in_state("failures"), "r");
    if (!f) return 0;
    char id[64], line[512], fid[64];
    boot_id(id, sizeof id);
    double now = uptime_now();
    int n = 0;
    while (fgets(line, sizeof line, f)) {
        double at;
        if (sscanf(line, "%63s %lf", fid, &at) == 2 && !strcmp(fid, id) && now - at <= FAIL_WINDOW_S) n++;
    }
    fclose(f);
    return n;
}

// Appends one failure, keeping the newest few lines.
static void add_failure(const char *reason) {
    char lines[8][512];
    int n = 0;
    FILE *f = fopen(in_state("failures"), "r");
    if (f) {
        char line[512];
        while (fgets(line, sizeof line, f)) {
            if (n == 8) { memmove(lines[0], lines[1], sizeof lines[0] * 7); n = 7; }
            snprintf(lines[n++], sizeof lines[0], "%s", line);
        }
        fclose(f);
    }
    char id[64], text[8 * 512 + 512] = "", one[512], clean[300];
    boot_id(id, sizeof id);
    json_escape(reason, clean, sizeof clean);
    for (int i = (n == 8 ? 1 : 0); i < n; i++) strcat(text, lines[i]);
    snprintf(one, sizeof one, "%s %.0f %s\n", id, uptime_now(), clean);
    strcat(text, one);
    write_text(in_state("failures"), text);
    // The window alone would allow another try ten minutes on; "gave-up" holds the boot until the next one or `lumatv mm on`.
    if (recent_failures() >= FAIL_LIMIT) {
        snprintf(one, sizeof one, "%s\n", id);
        write_text(in_state("gave-up"), one);
    }
}

static bool gave_up_this_boot(void) {
    char line[128], id[64];
    if (!read_first_line(in_state("gave-up"), line, sizeof line)) return false;
    boot_id(id, sizeof id);
    return !strcmp(line, id);
}

// ---- the decision at start ----
static const char *decide(void) {
    if (!file_exists(in_state("enabled"))) return "LumaTV's memory manager is switched off";
    if (recent_failures() >= FAIL_LIMIT || gave_up_this_boot())
        return "three failures within ten minutes: LG's memory manager serves until the next boot";
    return NULL;
}

// ---- crash records ----
static const char *crash_dir(void) { return in_state("crash"); }

// The newest `max_lines` lines this program logged, from the collector's files.
static void copy_log_tail(const char *tag, const char *dst, int max_lines) {
    const size_t max_bytes = 96 * 1024;
    char *buf = malloc(max_bytes + 1);
    if (!buf) return;
    size_t n = 0;
    const char *names[] = { "previous.log", "current.log" };
    size_t tag_len = strlen(tag);
    for (int g = 0; g < 2; g++) {
        char path[700], line[4098];
        snprintf(path, sizeof path, "%s/%s", log_directory, names[g]);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        while (fgets(line, sizeof line, f)) {
            if (line[0] != '<' || !isdigit((unsigned char)line[1])) continue;
            char *message = strchr(line, '>');
            if (!message) continue;
            message++;
            if (strncmp(message, tag, tag_len) || strncmp(message + tag_len, ": ", 2)) continue;
            message += tag_len + 2;
            size_t size = strlen(message);
            if (size > max_bytes) continue;
            if (n + size > max_bytes) {
                size_t drop = n + size - max_bytes;
                while (drop < n && buf[drop - 1] != '\n') drop++;
                memmove(buf, buf + drop, n - drop);
                n -= drop;
            }
            memcpy(buf + n, message, size);
            n += size;
        }
        fclose(f);
    }
    char *from = buf, *end = buf + n, *scan = end;
    if (scan > from && scan[-1] == '\n') scan--;
    for (int seen = 0; scan > from;) {
        scan--;
        if (*scan == '\n' && ++seen == max_lines) { from = scan + 1; break; }
    }
    FILE *o = fopen(dst, "w");
    if (o) { fwrite(from, 1, (size_t)(end - from), o); fclose(o); }
    free(buf);
}

static int cmp_names(const void *a, const void *b) { return strcmp((const char *)a, (const char *)b); }

static void remove_record(const char *name) {
    char dir[700], file[900];
    snprintf(dir, sizeof dir, "%s/%s", crash_dir(), name);
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

// Keeps the newest `keep` records; their names sort by time.
static void rotate_records(int keep) {
    DIR *d = opendir(crash_dir());
    if (!d) return;
    static char names[64][64];
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d)) && n < 64) {
        if (e->d_name[0] == '.' || !strcmp(e->d_name, "pending") || !strcmp(e->d_name, "gave-up")) continue;
        snprintf(names[n++], 64, "%s", e->d_name);
    }
    closedir(d);
    qsort(names, (size_t)n, 64, cmp_names);
    for (int i = 0; i < n - keep; i++) remove_record(names[i]);
}

// reason.txt carries "summary:" and "last error:" lines, the fields the helper's notice reads (ssgw.rs `pending`).
static void crash_record(const char *reason) {
    char stamp[48], name[64] = "", dir[700], path[800];
    time_t t = time(NULL);
    struct tm tm;
    if (t > 1577836800) strftime(stamp, sizeof stamp, "%Y%m%d-%H%M%S", localtime_r(&t, &tm));
    else snprintf(stamp, sizeof stamp, "boot-%06.0f", uptime_now());
    mkdir_p(crash_dir());
    static int seq;
    for (int k = 0; k < 100; k++) {
        snprintf(name, sizeof name, "%s-%03d", stamp, ++seq);
        snprintf(dir, sizeof dir, "%s/%s", crash_dir(), name);
        if (mkdir(dir, 0755) == 0) break;
        if (errno != EEXIST || k == 99) return;
    }
    char id[64];
    boot_id(id, sizeof id);
    long u = usable_pages();
    snprintf(path, sizeof path, "%s/reason.txt", dir);
    FILE *f = fopen(path, "w");
    if (f) {
        fprintf(f, "summary: LumaTV's memory manager stopped and LG's took over\nlast error: %s\ntime: %s\nboot: %s\nuptime: %.0f s\n"
                   "level: %s (helper's: %s)\nusable: %ld MB\nfailures this boot: %d\n",
                reason, stamp, id, uptime_now(), level_names[local_level], hs.have ? hs.level : "none", pages_mb(u), recent_failures());
        fclose(f);
    }
    snprintf(path, sizeof path, "%s/calls.txt", dir);
    f = fopen(path, "w");
    if (f) {
        for (int i = 0; i < CALLS_KEPT; i++) {
            struct call *c = &calls[(calls_next + i) % CALLS_KEPT];
            if (c->method[0]) fprintf(f, "%.1f %s from %s\n", c->at, c->method, c->sender);
        }
        fclose(f);
    }
    snprintf(path, sizeof path, "%s/mm.log.tail", dir);
    copy_log_tail("lumatv-mm", path, CRASH_LOG_LINES);
    snprintf(path, sizeof path, "%s/pending", crash_dir());
    write_text(path, name);
    rotate_records(CRASH_KEEP);
    logline("crash record saved in %s/%s", crash_dir(), name);
}

// ---- giving the name back ----
static void release_for_exec(void) {
    if (bus) {
        LSError e; LSErrorInit(&e);
        if (!LSUnregister(bus, &e)) logline("LSUnregister: %s", e.message ? e.message : "?");
        LSErrorFree(&e);
        bus = NULL;
    }
    // Without this the bus socket would live on inside LG's process and the hub would refuse LG's own register.
    long max = sysconf(_SC_OPEN_MAX);
    if (max < 0 || max > 65536) max = 4096;
    for (int fd = 3; fd < max; fd++) close(fd);
    lumatv_log_reset();
}

static void fallback(const char *reason, bool count) {
    logline("fallback: %s", reason);
    write_text(in_state("last-fallback"), reason);
    if (count) {
        add_failure(reason);
        crash_record(reason);
    }
    unlink(in_state("running"));
    // Once the mount is gone the path is LG's file again, and so is every restart of the unit until mm.sh binds it.
    if (umount2(lg_binary, MNT_DETACH) != 0 && errno != EINVAL) logline("umount %s: %s", lg_binary, strerror(errno));
    release_for_exec();
    execv(lg_binary, saved_argv);
    logline("exec %s: %s", lg_binary, strerror(errno));
    _exit(1);
}

// ---- the bridge ----
static bool bridge_broken;

static void write_line(const char *line) {
    if (bridge_fd < 0 || bridge_broken) return;
    size_t n = strlen(line);
    struct iovec iov[2] = { { (void *)line, n }, { (void *)"\n", 1 } };
    struct msghdr mh = { 0 };
    mh.msg_iov = iov;
    mh.msg_iovlen = 2;
    // Never blocking: a helper that stops reading must not stop the answers.
    ssize_t w = sendmsg(bridge_fd, &mh, MSG_NOSIGNAL | MSG_DONTWAIT);
    if (w < 0 || (size_t)w != n + 1) {
        logline("bridge send: %s", w < 0 ? strerror(errno) : "short write");
        bridge_broken = true;
        shutdown(bridge_fd, SHUT_RDWR);
    }
}

static void bridge_closed(void) {
    if (bridge_fd >= 0) close(bridge_fd);
    bridge_fd = -1;
    gone_at = mono_now();
    logline("bridge: the helper went away");
}

// ---- method answers ----
static void reply(LSHandle *sh, LSMessage *m, const char *payload) {
    LSError e; LSErrorInit(&e);
    if (!LSMessageReply(sh, m, payload, &e)) logline("LSMessageReply: %s", e.message ? e.message : "?");
    LSErrorFree(&e);
}

static bool subscribe(LSHandle *sh, LSMessage *m, const char *key) {
    if (!LSMessageIsSubscription(m)) return false;
    LSError e; LSErrorInit(&e);
    bool ok = LSSubscriptionAdd(sh, key, m, &e);
    if (!ok) logline("LSSubscriptionAdd %s: %s", key, e.message ? e.message : "?");
    LSErrorFree(&e);
    return ok;
}

static void current_levels(const char **level, const char **prev) {
    if (state_fresh()) { *level = hs.level; *prev = hs.prev; return; }
    local_update();
    *level = level_names[local_level];
    *prev = level_names[local_prev];
}

static bool on_method(LSHandle *sh, LSMessage *msg, void *ctx) {
    (void)ctx;
    const char *method = LSMessageGetMethod(msg);
    const char *payload = LSMessageGetPayload(msg);
    const char *sender = LSMessageGetSenderServiceName(msg);
    if (!sender || !*sender) sender = LSMessageGetSender(msg);
    if (!method) method = "";
    if (!payload || !*payload) payload = "{}";
    if (!sender) sender = "";
    remember_call(method, sender);
    char out[8192];
    long usable = pages_mb(usable_pages());
    if (!strcmp(method, "setRequiredMemory")) {
        // First, always: SAM holds the launch until this answer.
        reply(sh, msg, "{\"returnValue\":true}");
        if (bridge_fd >= 0) {
            char esender[200];
            json_escape(sender, esender, sizeof esender);
            size_t need = strlen(payload) + 300;
            char *line = malloc(need);
            if (line) {
                snprintf(line, need, "{\"kind\":\"launch\",\"sender\":\"%s\",\"payload\":%s}", esender, payload);
                flatten(line);
                write_line(line);
                free(line);
            }
        }
    } else if (!strcmp(method, "getCurrentMemState")) {
        const char *level, *prev;
        current_levels(&level, &prev);
        snprintf(out, sizeof out, "{\"returnValue\":true,\"currentLevel\":\"%s\",\"usable\":%ld,\"prevLevel\":\"%s\"}", level, usable, prev);
        reply(sh, msg, out);
    } else if (!strcmp(method, "getCustomThreshold")) {
        // LG's refuses while an app it keeps preloads from (EfsTarget) is in front, the "limit" the helper reports.
        if (state_fresh() && hs.limit) {
            reply(sh, msg, "{\"errorCode\":9,\"returnValue\":false,\"errorText\":\"get custom threshold blocked\"}");
        } else {
            char threshold[32] = "";
            long t;
            if (!json_string(payload, "threshold", threshold, sizeof threshold) && json_number(payload, "threshold", &t)) snprintf(threshold, sizeof threshold, "%ld", t);
            char et[80];
            json_escape(threshold, et, sizeof et);
            bool sub = subscribe(sh, msg, "getCustomThreshold");
            snprintf(out, sizeof out, "{\"subscribed\":%s,\"returnValue\":true,\"threshold\":\"%s\",\"current\":\"%ld\"}", sub ? "true" : "false", et, usable);
            reply(sh, msg, out);
        }
    } else if (!strcmp(method, "getEFS")) {
        // Neither EFS swap nor hibernation exists here; the subscription is kept so emmcd's stays open.
        bool sub = subscribe(sh, msg, "getEFS");
        snprintf(out, sizeof out, "{\"subscribed\":%s,\"returnValue\":true,\"EfsInfo\":{\"activate\":false,\"type\":\"dynamic\",\"target\":false},\"HibernationInfo\":{\"activate\":false}}",
                 sub ? "true" : "false");
        reply(sh, msg, out);
    } else if (!strcmp(method, "getCloseAppId")) {
        if (!LSMessageIsSubscription(msg)) {
            reply(sh, msg, "{\"returnValue\":false,\"errorCode\":4,\"errorText\":\"Invalid Parameters Error\"}");
        } else {
            char type[40] = "", key[64];
            json_string(payload, "appType", type, sizeof type);
            snprintf(key, sizeof key, "close:%s", type);
            bool sub = subscribe(sh, msg, key);
            snprintf(out, sizeof out, "{\"returnValue\":true,\"subscribed\":%s}", sub ? "true" : "false");
            reply(sh, msg, out);
        }
    } else if (!strcmp(method, "getUnitList") || !strcmp(method, "getGroupInfo")) {
        const char *units = state_fresh() && hs.units ? hs.units : "[]";
        size_t need = strlen(units) + 200;
        char *big = malloc(need);
        if (big) {
            if (!strcmp(method, "getUnitList"))
                snprintf(big, need, "{\"returnValue\":true,\"usable_memory\":%ld,\"swapUsed\":%ld,\"unitList\":%s}", usable, hs.swap_used, units);
            else
                snprintf(big, need, "{\"returnValue\":true,\"groups\":%s}", units);
            reply(sh, msg, big);
            free(big);
        }
    } else if (!strcmp(method, "getPolicy")) {
        const char *policy = state_fresh() && hs.policy ? hs.policy : "{}";
        size_t need = strlen(policy) + 100;
        char *big = malloc(need);
        if (big) {
            snprintf(big, need, "{\"returnValue\":true,\"policy\":%s}", policy);
            reply(sh, msg, big);
            free(big);
        }
    } else if (!strcmp(method, "startMemNotifier") || !strcmp(method, "setMriHelper")) {
        reply(sh, msg, "{\"returnValue\":true}");
    } else {
        snprintf(out, sizeof out, "{\"returnValue\":false,\"errorCode\":-1,\"errorText\":\"not supported\"}");
        reply(sh, msg, out);
    }
    return true;
}

static bool on_out_reply(LSHandle *sh, LSMessage *m, void *ctx) {
    (void)sh;
    const char *p = LSMessageGetPayload(m);
    if (p && !strstr(p, "\"returnValue\":true")) logline("%s answered %.200s", (const char *)ctx, p);
    free(ctx);
    return true;
}

static bool valid_name(const char *s) {
    if (!*s) return false;
    for (; *s; s++) if (!isalnum((unsigned char)*s) && *s != '.' && *s != '_' && *s != '-') return false;
    return true;
}

static void handle_line(char *line) {
    char kind[32] = "";
    json_string(line, "kind", kind, sizeof kind);
    const char *v; size_t n;
    if (!strcmp(kind, "pong")) {
        long seq;
        if (json_number(line, "seq", &seq)) pong_seq = (unsigned long)seq;
    } else if (!strcmp(kind, "state")) {
        json_string(line, "level", hs.level, sizeof hs.level);
        json_string(line, "prevLevel", hs.prev, sizeof hs.prev);
        hs.limit = json_bool(line, "limit");
        long swap = 0;
        hs.swap_used = json_number(line, "swapUsed", &swap) ? swap : 0;
        free(hs.units); hs.units = NULL;
        free(hs.policy); hs.policy = NULL;
        if (json_value(line, "units", &v, &n) && n && *v == '[') hs.units = strndup(v, n);
        if (json_value(line, "policy", &v, &n) && n && *v == '{') hs.policy = strndup(v, n);
        hs.have = hs.level[0] != 0;
        hs.at = mono_now();
    } else if (!strcmp(kind, "signal")) {
        char name[40] = "", uri[200];
        if (json_string(line, "name", name, sizeof name) && (!strcmp(name, "thresholdChanged") || !strcmp(name, "clearContainers"))
            && json_value(line, "payload", &v, &n) && bus) {
            char *payload = strndup(v, n);
            snprintf(uri, sizeof uri, "luna://" SERVICE "/com/webos/memory/%s", name);
            LSError e; LSErrorInit(&e);
            if (!LSSignalSend(bus, uri, payload, &e)) logline("LSSignalSend %s: %s", name, e.message ? e.message : "?");
            LSErrorFree(&e);
            free(payload);
        }
    } else if (!strcmp(kind, "closing")) {
        char id[200] = "", type[40] = "", eid[256], key[64], out[400];
        if (json_string(line, "id", id, sizeof id) && valid_name(id) && bus) {
            json_string(line, "appType", type, sizeof type);
            json_escape(id, eid, sizeof eid);
            snprintf(out, sizeof out, "{\"returnValue\":true,\"id\":\"%s\"}", eid);
            LSError e; LSErrorInit(&e);
            snprintf(key, sizeof key, "close:%s", type);
            LSSubscriptionReply(bus, key, out, &e);
            if (type[0]) LSSubscriptionReply(bus, "close:", out, &e);
            LSErrorFree(&e);
        }
    } else if (!strcmp(kind, "outbound")) {
        char uri[300] = "";
        if (json_string(line, "uri", uri, sizeof uri) && !strncmp(uri, "luna://", 7) && json_value(line, "payload", &v, &n) && bus) {
            char *payload = strndup(v, n);
            LSError e; LSErrorInit(&e);
            char *what = strdup(uri);
            if (!LSCallOneReply(bus, uri, payload, on_out_reply, what, NULL, &e)) {
                logline("LSCall %s: %s", uri, e.message ? e.message : "?");
                free(what);
            }
            LSErrorFree(&e);
            free(payload);
        }
    } else if (!strcmp(kind, "bye")) {
        helper_bye = true;
        logline("bridge: the helper is stopping");
    } else if (!strcmp(kind, "fallback")) {
        char reason[200] = "the helper asked";
        json_string(line, "reason", reason, sizeof reason);
        fallback(reason, false);
    } else {
        logline("bridge: unknown kind %s", kind);
    }
}

static char inbuf[256 * 1024];
static size_t inlen;

static gboolean on_bridge_readable(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)data;
    ssize_t n = (cond & G_IO_IN) ? read(bridge_fd, inbuf + inlen, sizeof inbuf - inlen - 1) : 0;
    if (n < 0 && (errno == EAGAIN || errno == EINTR)) return 1;
    if (n <= 0) { bridge_closed(); return 0; }
    inlen += (size_t)n;
    inbuf[inlen] = 0;
    char *start = inbuf, *nl;
    while ((nl = memchr(start, '\n', inlen - (size_t)(start - inbuf)))) {
        *nl = 0;
        handle_line(start);
        start = nl + 1;
    }
    size_t rest = inlen - (size_t)(start - inbuf);
    memmove(inbuf, start, rest);
    inlen = rest;
    if (inlen == sizeof inbuf - 1) { logline("bridge: line too long"); inlen = 0; }
    return 1;
}

static void bridge_attach(int fd) {
    bridge_fd = fd;
    bridge_broken = false;
    inlen = 0;
    ping_seq = pong_seq = 0;
    last_ping = mono_now();
    ever_connected = true;
    helper_bye = false;
    char line[64];
    snprintf(line, sizeof line, "{\"kind\":\"hello\",\"pid\":%ld}", (long)getpid());
    write_line(line);
    logline("bridge: helper connected");
}

static gboolean on_bridge_accept(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)cond; (void)data;
    int fd = accept4(bridge_listen, NULL, NULL, SOCK_CLOEXEC | SOCK_NONBLOCK);
    if (fd < 0) return 1;
    // A second helper (an old one still ending) is refused; the live one reconnects once its old link is gone.
    if (bridge_fd >= 0) { logline("bridge: second connection refused"); close(fd); return 1; }
    bridge_attach(fd);
    g_io_add_watch(g_io_channel_unix_new(fd), G_IO_IN | G_IO_HUP | G_IO_ERR, on_bridge_readable, NULL);
    return 1;
}

// Once a second: the switch, the helper's presence and heartbeat, and the kernel's level.
static const char *tick_verdict(double now) {
    if (switched_off) return "switched off";
    if (bridge_broken && bridge_fd >= 0) bridge_closed();
    if (bridge_fd < 0) {
        if (!ever_connected && now - started_at > HELPER_FIRST_S) return "the helper did not connect within 20 s";
        if (ever_connected && helper_bye && now - gone_at > HELPER_BYE_S) return "the helper stopped and did not come back within 60 s";
        if (ever_connected && !helper_bye && now - gone_at > HELPER_GONE_S) return "the helper was gone for 10 s";
        return NULL;
    }
    if (now - last_ping >= PING_EVERY_S) {
        if (ping_seq > pong_seq + 1) return "the helper missed two heartbeats";
        last_ping = now;
        char line[64];
        snprintf(line, sizeof line, "{\"kind\":\"ping\",\"seq\":%lu}", ++ping_seq);
        write_line(line);
    }
    return NULL;
}

static gboolean on_tick(gpointer data) {
    (void)data;
    local_update();
    const char *why = tick_verdict(mono_now());
    // Neither the switch nor a helper that said goodbye is the shim failing.
    if (why) fallback(why, !switched_off && !helper_bye);
    return 1;
}

static LSMethod methods[] = {
    { "setRequiredMemory", on_method, 0 }, { "getCurrentMemState", on_method, 0 }, { "getCustomThreshold", on_method, 0 },
    { "getEFS", on_method, 0 }, { "getCloseAppId", on_method, 0 }, { "getUnitList", on_method, 0 },
    { "getGroupInfo", on_method, 0 }, { "getPolicy", on_method, 0 }, { "startMemNotifier", on_method, 0 },
    { "setMriHelper", on_method, 0 }, { "sendLowMemPopupTest", on_method, 0 }, { 0 },
};
static LSSignal signals_table[] = { { "thresholdChanged", 0 }, { "clearContainers", 0 }, { 0 } };

static void on_term(int sig) {
    (void)sig;
    // Stopped on purpose (systemctl restart, shutdown): not an unclean end.
    unlink(in_state("running"));
    _exit(0);
}

static void on_usr1(int sig) { (void)sig; switched_off = 1; }

// A "running" marker from this boot means the last run ended without a word (a crash, a SIGKILL).
static void consume_unclean(void) {
    char line[128], id[64];
    if (!read_first_line(in_state("running"), line, sizeof line)) return;
    unlink(in_state("running"));
    boot_id(id, sizeof id);
    if (strcmp(line, id)) return;
    logline("the previous run ended uncleanly");
    add_failure("the previous run ended uncleanly (a crash or a kill)");
    crash_record("the previous run ended uncleanly (a crash or a kill)");
}

static int probe_bus(void) {
    LSError e; LSErrorInit(&e);
    LSHandle *sh = NULL;
    int rc;
    if (LSRegister(SERVICE, &sh, &e)) {
        printf("probe %s: registered (nobody holds it)\n", SERVICE);
        LSUnregister(sh, &e);
        rc = 0;
    } else {
        const char *m = e.message ? e.message : "?";
        rc = strstr(m, "already exists") ? 0 : 2;
        printf("probe %s: %s -> %s\n", SERVICE, m, rc ? "REFUSED" : "role allows it");
    }
    LSErrorFree(&e);
    return rc;
}

static void open_bridge(void) {
    const char *path = in_state("bridge.sock");
    unlink(path);
    bridge_listen = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    struct sockaddr_un a = { .sun_family = AF_UNIX };
    snprintf(a.sun_path, sizeof a.sun_path, "%s", path);
    if (bridge_listen < 0 || bind(bridge_listen, (struct sockaddr *)&a, sizeof a) < 0 || listen(bridge_listen, 2) < 0) fallback("the bridge socket could not be made", true);
    chmod(path, 0600);
    g_io_add_watch(g_io_channel_unix_new(bridge_listen), G_IO_IN, on_bridge_accept, NULL);
}

#ifdef LUMATV_MM_TEST
#include "test.c"
#else
int main(int argc, char **argv) {
    saved_argv = argv;
    bool would_run = false, probe = false;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--state") && i + 1 < argc) state_dir = argv[++i];
        else if (!strcmp(argv[i], "--would-run")) would_run = true;
        else if (!strcmp(argv[i], "--probe")) probe = true;
    }
    if (probe) return probe_bus();
    if (would_run) {
        const char *no = decide();
        if (no) { printf("no: %s\n", no); return 3; }
        printf("run\n");
        return 0;
    }
    started_at = mono_now();
    mkdir_p(state_dir);
    consume_unclean();
    const char *no = decide();
    if (no) fallback(no, false);
    char id[64];
    boot_id(id, sizeof id);
    write_text(in_state("running"), id);
    signal(SIGTERM, on_term);
    signal(SIGUSR1, on_usr1);
    signal(SIGPIPE, SIG_IGN);
    long u = usable_pages();
    if (u >= 0) local_level = local_prev = level_step(0, u);
    loop = g_main_loop_new(NULL, 0);
    LSError e; LSErrorInit(&e);
    if (!LSRegister(SERVICE, &bus, &e)) {
        char why[300];
        snprintf(why, sizeof why, "cannot register %s: %s", SERVICE, e.message ? e.message : "?");
        LSErrorFree(&e);
        fallback(why, true);
    }
    if (!LSRegisterCategory(bus, "/", methods, NULL, NULL, &e)) logline("LSRegisterCategory /: %s", e.message ? e.message : "?");
    LSErrorFree(&e); LSErrorInit(&e);
    if (!LSRegisterCategory(bus, "/com/webos/memory", NULL, signals_table, NULL, &e)) logline("LSRegisterCategory /com/webos/memory: %s", e.message ? e.message : "?");
    LSErrorFree(&e); LSErrorInit(&e);
    if (!LSGmainAttach(bus, loop, &e)) { LSErrorFree(&e); fallback("the bus could not join the main loop", true); }
    LSErrorFree(&e);
    open_bridge();
    g_timeout_add(1000, on_tick, NULL);
    logline("serving %s (pid %ld)", SERVICE, (long)getpid());
    g_main_loop_run(loop);
    return 0;
}
#endif
