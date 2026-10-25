// lumatv-sm: the process systemd starts as /usr/bin/surface-manager once LumaTV's compositor is live (track C4).
// It is 32-bit and dynamically linked against the set's own libluna-service2 and glib, because ls-hubd grants the
// compositor's bus names to the executable at that path and a 64-bit process cannot load LG's 32-bit libraries.
// Three duties, in order (docs/research/compositor-c2-result.md §5 and §8):
//
//   1. Decide. LumaTV runs only if /var/lib/lumatv/compositor/enabled exists, the firmware fingerprint equals the
//      verified one, no escape is recorded, and fewer than 2 consecutive failures are. Otherwise, and on any later
//      failure, fall back: remove the bind mount that put this file at /usr/bin/surface-manager and exec LG's
//      compositor at the same path, so the unit, the role and the exe name stay LG's.
//   2. Supervise. Spawn the 64-bit compositor through its private loader on LG's socket (/tmp/xdg/wayland-0),
//      heartbeat it over the bridge socket every 500 ms, and fall back when it exits or misses two beats. The
//      compositor can ask for the fallback itself (the hold-Home escape), which also records the escape.
//   3. Bridge luna. Register the compositor's bus names, forward every call as a JSON line, send the compositor's
//      replies (subscriptions kept by token) and make its outbound calls.
//
// JSON lines on the bridge socket (compositor/crates/lumatv-compositor/src/luna.rs):
//   to the compositor:   {"kind":"inbound","token":N,"service":S,"method":M,"payload":P,"subscribe":B,"sender":S}
//                        {"kind":"inbound","token":N,...,"cancelled":true}   {"kind":"event","token":N,"payload":P}
//                        {"kind":"ping","seq":N}
//   from the compositor: {"kind":"reply","token":N,"payload":P}   {"kind":"outbound","token":N,"uri":U,"payload":P,"subscribe":B}
//                        {"kind":"cancel","token":N}   {"kind":"pong","seq":N}   {"kind":"fallback","reason":R}
//
// Build: compositor/sm/build.sh (zig cc, arm-linux-gnueabi glibc 2.28, stub libraries for the link; the set's
// libraries at run time). Usage: lumatv-sm [--dry-run|--probe] [--state DIR] [--print-fingerprint]
//   --dry-run   never touches the mount or execs LG's binary; registers the bus names with a ".lumatv" suffix and
//               runs the compositor offscreen on the private socket, for a test of the bridge beside LG's compositor
//               (the hub refuses the suffixed names, since no role file allows them: that part proves nothing).
//   --probe     asks the hub for the real names and exits: run from the real path (bind-mounted, LG's compositor
//               still running) it tells the role check from the name being taken, because ls-hubd checks the role
//               before it checks for a duplicate (luna-service2 hub.cpp _LSHubHandleRequestName). Exit 0 = every
//               name is allowed to this executable path (all answered "already exists"), 2 = at least one refused.
//   --print-fingerprint prints the firmware fingerprint the enable step records.
// The unit is Type=notify: READY=1 goes to systemd's socket once the compositor is connected (else the start
// times out and the unit is restarted), and /tmp/lsm-ready is touched, which is what webapp-mgr.service waits for.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
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
static bool dry_run = false, probe = false;
static bool supervising = false;
static char **saved_argv;
static GMainLoop *loop;
static pid_t child_pid = 0;
static int bridge_listen = -1, bridge_fd = -1;
static unsigned long ping_seq = 0, pong_seq = 0;
static unsigned long inbound_token = 0;
static time_t started_at;
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

// ---- fallback to LG's compositor ----
static void fallback(const char *reason, bool count_failure) {
    logline("fallback: %s", reason);
    write_text(path_in_state("last-fallback"), reason);
    if (count_failure) write_number(path_in_state("failures"), read_number(path_in_state("failures"), 0) + 1);
    if (child_pid > 0) {
        kill(child_pid, SIGTERM);
        for (int i = 0; i < 20 && waitpid(child_pid, NULL, WNOHANG) == 0; i++) usleep(50000);
        kill(child_pid, SIGKILL);
        waitpid(child_pid, NULL, 0);
        child_pid = 0;
    }
    if (dry_run || probe) {
        logline("%s: would unmount %s and exec LG's compositor", dry_run ? "dry run" : "probe", lg_binary);
        exit(75);
    }
    // A fallback after the unit started under this supervisor (not the decision at start-up, where nothing of
    // LG's was disturbed yet) needs WebAppMgr and Home brought back once LG's compositor is up.
    if (supervising) spawn_recovery();
    // The bind mount placed by the early hook: once it is gone, the path is LG's file again, and so is every later
    // restart of the unit until the next boot decides anew.
    if (umount2(lg_binary, MNT_DETACH) != 0 && errno != EINVAL) logline("umount %s: %s", lg_binary, strerror(errno));
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

static void send_line(const char *line) {
    if (bridge_fd < 0) return;
    size_t n = strlen(line);
    struct iovec iov[2] = { { (void *)line, n }, { (void *)"\n", 1 } };
    struct msghdr mh = { 0 };
    mh.msg_iov = iov; mh.msg_iovlen = 2;
    if (sendmsg(bridge_fd, &mh, MSG_NOSIGNAL) < 0) logline("bridge send: %s", strerror(errno));
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
    size_t need = strlen(payload ? payload : "{}") + 512;
    char *line = malloc(need);
    snprintf(line, need, "{\"kind\":\"inbound\",\"token\":%lu,\"service\":\"%s\",\"method\":\"%s\",\"payload\":%s,\"subscribe\":%s,\"sender\":\"%s\"}",
             slot->token, b->name, emethod, payload && *payload ? payload : "{}", slot->subscription ? "true" : "false", esender);
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

static gboolean on_bridge_readable(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)data;
    if (cond & (G_IO_HUP | G_IO_ERR)) { fallback("compositor closed the bridge socket", true); return 0; }
    ssize_t n = read(bridge_fd, inbuf + inlen, sizeof inbuf - inlen - 1);
    if (n <= 0) { fallback("bridge read ended", true); return 0; }
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
    g_io_add_watch(g_io_channel_unix_new(fd), G_IO_IN | G_IO_HUP | G_IO_ERR, on_bridge_readable, NULL);
    // The compositor is listening on LG's socket and the bus names are ours: the unit is up.
    if (!dry_run) { sd_notify_ready(); emit_lsm_ready(); }
    return 1;
}

static gboolean on_heartbeat(gpointer data) {
    (void)data;
    if (bridge_fd < 0) {
        if (time(NULL) - started_at > 20) fallback("compositor did not connect to the bridge within 20 s", true);
        return 1;
    }
    if (ping_seq > pong_seq + 2) { fallback("heartbeat: two beats missed", true); return 1; }
    char line[64];
    snprintf(line, sizeof line, "{\"kind\":\"ping\",\"seq\":%lu}", ++ping_seq);
    send_line(line);
    // Ten minutes healthy clears the failure count: a crash a week later is not "consecutive".
    if (ping_seq == 1200) write_number(path_in_state("failures"), 0);
    return 1;
}

static void on_child(int pid, int status, gpointer data) {
    (void)data;
    if (pid != child_pid) return;
    child_pid = 0;
    char reason[128];
    if (WIFEXITED(status)) snprintf(reason, sizeof reason, "compositor exited with %d", WEXITSTATUS(status));
    else snprintf(reason, sizeof reason, "compositor killed by signal %d", WTERMSIG(status));
    fallback(reason, true);
}

// ---- the compositor's bus names and methods (docs/research/shell-architecture.md, 56 methods on 12 names) ----
#define M(n) { n, on_method, 0 }
static LSMethod m_surfacemanager[] = { M("closeByAppId"), M("getForegroundAppInfo"), M("captureCompositorOutput"), M("clearRecentsList"), M("closeLRUSurface"), M("getCurrentState"), M("getFocusedWindowInfo"), M("getRecentsAppList"), M("showInputPicker"), M("showQuickSettings"), M("showSpinner"), M("showIndicator"), M("getForegroundVideoWindow"), M("getWindowGroupInfo"), M("getForegroundWindowInfo"), M("getScreenVisible"), M("setScreenVisible"), M("getOverlayPlayVideoState"), M("getForegroundAppState"), M("changeMultiViewLayout"), M("setMultiViewLayout"), M("getLastInputType"), M("getMultiViewLayoutAnimationState"), M("setPinOverlayView"), M("getZoomStatus"), M("setZoom"), M("alertChoice"), { 0 } };
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
static void spawn_compositor(void) {
    char loader[600], libdir[600], bin[600], blob[600], bridge[600], logpath[600];
    snprintf(loader, sizeof loader, "%s/lib/ld-linux-aarch64.so.1", state_dir);
    snprintf(libdir, sizeof libdir, "%s/lib", state_dir);
    snprintf(bin, sizeof bin, "%s/bin/lumatv-compositor", state_dir);
    snprintf(blob, sizeof blob, "%s/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so", state_dir);
    snprintf(bridge, sizeof bridge, "%s/bridge.sock", state_dir);
    snprintf(logpath, sizeof logpath, "%s/compositor.log", state_dir);
    mkdir(runtime_dir, 0700);
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
        execl(loader, loader, "--library-path", libdir, bin, "--runtime-dir", rt, "--socket", sock, "--renderer", renderer, "--driver", blob, "--buffers", "own", "--card", "/dev/dri/card0", (char *)NULL);
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

#ifdef LUMATV_SM_TEST
// The JSON scanner against lines as serde_json writes them on the compositor's side.
static int failures_seen;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #c); failures_seen++; } } while (0)
int main(void) {
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
    printf("lumatv-sm json tests: %s\n", failures_seen ? "FAILED" : "ok");
    return failures_seen ? 1 : 0;
}
#else
int main(int argc, char **argv) {
    saved_argv = argv;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--dry-run")) dry_run = true;
        else if (!strcmp(argv[i], "--probe")) probe = true;
        else if (!strcmp(argv[i], "--state") && i + 1 < argc) state_dir = argv[++i];
        else if (!strcmp(argv[i], "--print-fingerprint")) { char fp[17]; fingerprint(fp); printf("%s\n", fp); return 0; }
    }
    mkdir(state_dir, 0755);
    logf = fopen(path_in_state("sm.log"), "a");
    started_at = time(NULL);
    signal(SIGPIPE, SIG_IGN);
    if (probe) {
        char exe[256] = "?";
        ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
        if (n > 0) exe[n] = 0;
        logline("probe from %s", exe);
        printf("probe from %s\n", exe);
        return probe_buses();
    }

    // 1. Decide.
    char fp[17];
    fingerprint(fp);
    char want[64] = "";
    FILE *f = fopen(path_in_state("fingerprint"), "r");
    if (f) { if (fscanf(f, "%63s", want) != 1) want[0] = 0; fclose(f); }
    long failures = read_number(path_in_state("failures"), 0);
    logline("start: enabled %d fingerprint %s (verified %s) failures %ld escaped %d dry-run %d", file_exists(path_in_state("enabled")), fp, want[0] ? want : "none", failures, file_exists(path_in_state("escaped")), dry_run);
    if (!dry_run) {
        if (!file_exists(path_in_state("enabled"))) fallback("not enabled", false);
        if (file_exists(path_in_state("escaped"))) fallback("escape recorded; re-enable from the tool", false);
        if (strcmp(fp, want) != 0) fallback("firmware fingerprint differs from the verified one", false);
        if (failures >= 2) fallback("two consecutive failures", false);
    }

    // 2 and 3. Supervise and bridge. From here a fallback also brings LG's Home back (spawn_recovery).
    supervising = true;
    loop = g_main_loop_new(NULL, 0);
    open_bridge();
    register_buses();
    spawn_compositor();
    g_timeout_add(500, on_heartbeat, NULL);
    g_main_loop_run(loop);
    return 0;
}
#endif
