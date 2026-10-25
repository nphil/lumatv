// lumatv-card: the program SAM starts for the card app (appinfo.json "main"). It is 32-bit and dynamically linked against the set's own
// libluna-service2 and glib, for the reason `lumatv-sm` is: the luna-service2 library is 32-bit, ls-hubd gives a bus name to the
// executable it has a role for, and SAM talks to its native apps over the bus. The app itself is 64-bit (bin64/lumatv-card-app, the
// toolkit's, on the Mali blob), so this program starts it and stands between it and SAM. docs/research/cardapp-result.md has the account.
//
// What SAM does with a native app (X4 capture, docs/research/arch/x4, PlxNative / Netflix / the browser):
//   - starts `<main> '<json>'`: {"@system_native_app":true,"reason":"undefined"|"preload","interfaceVersion":2,"event":"launch",
//     "interfaceMethod":"registerApp","appId":...,"parameters":{...}}
//   - the app registers a bus name equal to its app id and calls luna://com.webos.applicationManager/registerApp
//     {"appId":...,"subscribe":true}; SAM answers {"event":"registered"} and then pushes {"event":"relaunch","reason":...,"appId":...,
//     "parameters":{...}} (the app is launched again while it runs) and {"event":"close","reason":...} (the app is to end).
//
// What this program does:
//   1. Starts the app through gfx64/launch.sh (the 64-bit loader, the heap shim and the Mali blob of this package) on a socket pair, which
//      is descriptor 3 of the app: one JSON object a line. A launch that is a preload (reason "preload" or parameters.snapshot) starts the
//      app with --hidden.
//   2. Registers as the app id and subscribes to SAM. If the hub refuses the name, or SAM refuses the call, the app still runs and is told
//      ({"kind":"sam","registered":false}); nothing is retried.
//   3. Tells the app what SAM pushes: {"kind":"relaunch","reason":R,"parameters":{...}} and {"kind":"close","reason":R}. A close that the
//      app has not answered by ending in 4 s becomes SIGTERM to its process group, and SIGKILL 2 s after that.
//   4. Ends with the app: cancels the subscription, unregisters, exits with the app's status (128 + signal when it was killed).
// SIGTERM to this program (SAM, a stop) goes to the app's process group the same way. The app also ends when this program goes (the
// channel closes, and its launcher shell has PR_SET_PDEATHSIG).
//
// What the app tells it ({"kind":"ready"|"state"|"page"|"exit"}) is only logged: the log is the TV step list's evidence.
//
// Environment, all optional: LUMATV_CARD_LOG (log file, default /tmp/lumatv-card.log), LUMATV_CARD_HOME (the app directory; default the
// parent of this executable's directory), XDG_RUNTIME_DIR and WAYLAND_DISPLAY (defaults /tmp/xdg and wayland-0, LG's socket). Test hooks:
// LUMATV_CARD_CHILD (a program to run in place of launch.sh and the app; it gets the app's arguments), LUMATV_CARD_KILL_MS (the 4 s).
//
// Build: rustui/app/stub/build.sh (zig cc, arm-linux-gnueabi glibc 2.28, stub libraries for the link; the set's libraries at run time).
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
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

// ---- luna-service2 (public API, webosose/luna-service2 lunaservice.h; the calls lumatv-sm already makes on the set) ----
typedef struct LSHandle LSHandle;
typedef struct LSMessage LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError { int error_code; char *message; const char *file; int line; const char *func; void *padding; unsigned long magic; } LSError;
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef struct GMainLoop GMainLoop;
bool LSErrorInit(LSError *e);
void LSErrorFree(LSError *e);
bool LSRegister(const char *name, LSHandle **sh, LSError *e);
bool LSUnregister(LSHandle *sh, LSError *e);
bool LSGmainAttach(LSHandle *sh, GMainLoop *loop, LSError *e);
bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e);
bool LSCallCancel(LSHandle *sh, LSMessageToken token, LSError *e);
const char *LSMessageGetPayload(LSMessage *m);

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
unsigned g_unix_signal_add(int signum, GSourceFunc fn, gpointer data);

#define DEFAULT_APP_ID "org.nphil.lumatv.card"
#define SAM_REGISTER "luna://com.webos.applicationManager/registerApp"

static GMainLoop *loop;
static FILE *logf;
static char app_id[200] = DEFAULT_APP_ID;
static pid_t child_pid;
static int channel = -1;
static LSHandle *sh;
static LSMessageToken sam_token;
static bool sam_call, sam_answered, closing;
static unsigned kill_ms = 4000;
static int child_status = -1;

static void logline(const char *fmt, ...) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    char head[48];
    snprintf(head, sizeof head, "[lumatv-card %ld.%03ld] ", (long)ts.tv_sec, ts.tv_nsec / 1000000);
    char body[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(body, sizeof body, fmt, ap);
    va_end(ap);
    fprintf(stderr, "%s%s\n", head, body);
    if (logf) {
        fprintf(logf, "%s%s\n", head, body);
        fflush(logf);
    }
}

// ---- a minimal JSON scanner (the one of lumatv-sm: top-level keys, raw values) ----
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
        if (c == '\\' && i + 2 < n) { i++; c = v[i]; if (c == 'n') c = '\n'; else if (c == 't') c = '\t'; }
        out[o++] = c;
    }
    out[o] = 0;
    return true;
}

static bool json_true(const char *obj, const char *key) {
    const char *v; size_t n;
    return json_value(obj, key, &v, &n) && n >= 4 && !strncmp(v, "true", 4);
}

// A string as the body of a JSON string.
static void json_escape(const char *s, char *out, size_t outsz) {
    size_t o = 0;
    for (; *s && o + 7 < outsz; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') { out[o++] = '\\'; out[o++] = c; }
        else if (c == '\n') { out[o++] = '\\'; out[o++] = 'n'; }
        else if (c < 0x20) o += snprintf(out + o, outsz - o, "\\u%04x", c);
        else out[o++] = c;
    }
    out[o] = 0;
}

// ---- the channel to the app ----
static void to_app(const char *line) {
    if (channel < 0) return;
    size_t n = strlen(line);
    char buf[2048];
    if (n + 2 > sizeof buf) { logline("channel: message too long, dropped"); return; }
    memcpy(buf, line, n);
    buf[n++] = '\n';
    ssize_t w = send(channel, buf, n, MSG_NOSIGNAL | MSG_DONTWAIT);
    if (w != (ssize_t)n) logline("channel: write of %zu bytes gave %zd (%s)", n, w, w < 0 ? strerror(errno) : "short");
}

static char inbuf[4096];
static size_t inlen;

static gboolean on_app_says(GIOChannel *ch, int cond, gpointer data) {
    (void)ch; (void)data;
    ssize_t n = read(channel, inbuf + inlen, sizeof inbuf - inlen - 1);
    if (n <= 0) {
        if (n < 0 && (errno == EAGAIN || errno == EINTR)) return 1;
        close(channel);
        channel = -1;
        return 0;
    }
    inlen += n;
    inbuf[inlen] = 0;
    char *start = inbuf, *nl;
    while ((nl = memchr(start, '\n', inlen - (start - inbuf)))) {
        *nl = 0;
        logline("app: %s", start);
        start = nl + 1;
    }
    size_t rest = inlen - (start - inbuf);
    memmove(inbuf, start, rest);
    inlen = rest;
    if (inlen == sizeof inbuf - 1) inlen = 0;
    (void)cond;
    return 1;
}

// ---- stopping the app ----
static void signal_app(int sig) {
    if (child_pid > 0) kill(-child_pid, sig);
}

static gboolean kill_hard(gpointer data) {
    (void)data;
    if (child_pid > 0) { logline("the app is still running: SIGKILL"); signal_app(SIGKILL); }
    return 0;
}

static gboolean kill_soft(gpointer data) {
    (void)data;
    if (child_pid > 0) {
        logline("the app did not end in %u ms: SIGTERM", kill_ms);
        signal_app(SIGTERM);
        g_timeout_add(2000, kill_hard, NULL);
    }
    return 0;
}

static gboolean on_sigterm(gpointer data) {
    (void)data;
    logline("SIGTERM: stopping the app");
    signal_app(SIGTERM);
    g_timeout_add(3000, kill_hard, NULL);
    return 0;
}

// ---- SAM ----
static void tell_sam_state(bool registered) {
    to_app(registered ? "{\"kind\":\"sam\",\"registered\":true}" : "{\"kind\":\"sam\",\"registered\":false}");
}

static bool on_sam(LSHandle *h, LSMessage *msg, void *ctx) {
    (void)h; (void)ctx;
    const char *payload = LSMessageGetPayload(msg);
    if (!payload) payload = "{}";
    char event[40] = "", reason[80] = "";
    json_string(payload, "event", event, sizeof event);
    json_string(payload, "reason", reason, sizeof reason);
    if (!strcmp(event, "registered")) {
        sam_answered = true;
        logline("SAM: registered");
        tell_sam_state(true);
    } else if (!strcmp(event, "relaunch")) {
        const char *p; size_t n;
        char params[1024] = "{}", reason_esc[160], line[1400];
        if (json_value(payload, "parameters", &p, &n) && n < sizeof params) { memcpy(params, p, n); params[n] = 0; }
        json_escape(reason, reason_esc, sizeof reason_esc);
        snprintf(line, sizeof line, "{\"kind\":\"relaunch\",\"reason\":\"%s\",\"parameters\":%s}", reason_esc, params);
        logline("SAM: relaunch (%s) %s", reason, params);
        to_app(line);
    } else if (!strcmp(event, "close")) {
        char reason_esc[160], line[300];
        json_escape(reason, reason_esc, sizeof reason_esc);
        snprintf(line, sizeof line, "{\"kind\":\"close\",\"reason\":\"%s\"}", reason_esc);
        logline("SAM: close (%s)", reason);
        to_app(line);
        if (!closing) { closing = true; g_timeout_add(kill_ms, kill_soft, NULL); }
    } else if (!sam_answered && !json_true(payload, "returnValue")) {
        // An answer with no event that says no: the call was refused (the app's role has no outbound to SAM, or SAM does not know it).
        sam_answered = true;
        logline("SAM refused registerApp: %.200s", payload);
        tell_sam_state(false);
    } else {
        logline("SAM: %.200s", payload);
    }
    return true;
}

static void register_with_sam(void) {
    LSError e;
    LSErrorInit(&e);
    if (!LSRegister(app_id, &sh, &e)) {
        logline("LSRegister %s: %s (the hub gives no bus name to this executable; the app runs without SAM's events)", app_id, e.message ? e.message : "?");
        LSErrorFree(&e);
        sh = NULL;
        tell_sam_state(false);
        return;
    }
    if (!LSGmainAttach(sh, loop, &e)) logline("LSGmainAttach: %s", e.message ? e.message : "?");
    LSErrorFree(&e);
    LSErrorInit(&e);
    char esc[400], payload[480];
    json_escape(app_id, esc, sizeof esc);
    snprintf(payload, sizeof payload, "{\"appId\":\"%s\",\"subscribe\":true}", esc);
    if (!LSCall(sh, SAM_REGISTER, payload, on_sam, NULL, &sam_token, &e)) {
        logline("LSCall registerApp: %s", e.message ? e.message : "?");
        tell_sam_state(false);
    } else {
        sam_call = true;
        logline("registered %s, registerApp sent: %s", app_id, payload);
    }
    LSErrorFree(&e);
}

static void unregister_from_sam(void) {
    if (!sh) return;
    LSError e;
    LSErrorInit(&e);
    if (sam_call) LSCallCancel(sh, sam_token, &e);
    LSErrorFree(&e);
    LSErrorInit(&e);
    LSUnregister(sh, &e);
    LSErrorFree(&e);
    sh = NULL;
}

// ---- the app ----
static void on_child(int pid, int status, gpointer data) {
    (void)data;
    if (pid != child_pid) return;
    child_pid = 0;
    if (WIFEXITED(status)) { child_status = WEXITSTATUS(status); logline("the app ended with %d", child_status); }
    else { child_status = 128 + WTERMSIG(status); logline("the app was killed by signal %d", WTERMSIG(status)); }
    unregister_from_sam();
    g_main_loop_quit(loop);
}

static void join(char *dst, size_t n, const char *a, const char *b) { snprintf(dst, n, "%s/%s", a, b); }

static void start_app(const char *home, bool hidden, const char *reason, const char *params) {
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sv) < 0) { logline("socketpair: %s", strerror(errno)); exit(70); }
    char launch[600], bundle[600], shim[600], app[600], assets[600];
    join(launch, sizeof launch, home, "gfx64/launch.sh");
    join(bundle, sizeof bundle, home, "gfx64/bundle");
    join(shim, sizeof shim, home, "gfx64/heapshim.so");
    join(app, sizeof app, home, "bin64/lumatv-card-app");
    join(assets, sizeof assets, home, "assets");
    const char *override = getenv("LUMATV_CARD_CHILD");
    pid_t pid = fork();
    if (pid < 0) { logline("fork: %s", strerror(errno)); exit(70); }
    if (pid == 0) {
        // The app and the shell that starts it: their own process group (a stop reaches all of it), gone with this program, the
        // channel as descriptor 3, the log as their output, nothing else open.
        setpgid(0, 0);
        prctl(PR_SET_PDEATHSIG, SIGTERM);
        if (logf) { int lf = fileno(logf); dup2(lf, 1); dup2(lf, 2); }
        if (dup2(sv[1], 3) < 0) _exit(126);
        for (int fd = 4; fd < 256; fd++) close(fd);
        char *args[32];
        int n = 0;
        if (override) {
            args[n++] = (char *)override;
        } else {
            args[n++] = "sh"; args[n++] = launch;
            args[n++] = "--bundle"; args[n++] = bundle;
            args[n++] = "--shim"; args[n++] = shim;
            args[n++] = "--";
            args[n++] = app;
        }
        args[n++] = "--app-id"; args[n++] = app_id;
        args[n++] = "--reason"; args[n++] = (char *)reason;
        args[n++] = "--params"; args[n++] = (char *)params;
        args[n++] = "--channel-fd"; args[n++] = "3";
        args[n++] = "--assets"; args[n++] = assets;
        if (hidden) args[n++] = "--hidden";
        args[n] = NULL;
        execv(override ? override : "/bin/sh", args);
        fprintf(stderr, "exec %s: %s\n", override ? override : "/bin/sh", strerror(errno));
        _exit(127);
    }
    setpgid(pid, pid);
    close(sv[1]);
    channel = sv[0];
    fcntl(channel, F_SETFL, fcntl(channel, F_GETFL) | O_NONBLOCK);
    child_pid = pid;
    g_child_watch_add(pid, on_child, NULL);
    g_io_add_watch(g_io_channel_unix_new(channel), G_IO_IN | G_IO_HUP | G_IO_ERR, on_app_says, NULL);
    logline("the app started: pid %d%s%s", (int)pid, hidden ? " (hidden: a preload)" : "", override ? " (test child)" : "");
}

// The app directory: LUMATV_CARD_HOME, else the parent of the directory this executable is in (<app>/bin/lumatv-card).
static void home_of_exe(char *out, size_t n) {
    const char *env = getenv("LUMATV_CARD_HOME");
    if (env && *env) { snprintf(out, n, "%s", env); return; }
    char exe[600];
    ssize_t len = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (len <= 0) { snprintf(out, n, "."); return; }
    exe[len] = 0;
    char *slash = strrchr(exe, '/');
    if (slash) *slash = 0;
    slash = strrchr(exe, '/');
    if (slash) *slash = 0;
    snprintf(out, n, "%s", exe);
}

int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    const char *log_path = getenv("LUMATV_CARD_LOG");
    logf = fopen(log_path && *log_path ? log_path : "/tmp/lumatv-card.log", "a");
    // SAM's launch: the one argument.
    const char *launch = argc > 1 && argv[1][0] == '{' ? argv[1] : "{}";
    char reason[64] = "undefined", params[1024] = "{}";
    json_string(launch, "reason", reason, sizeof reason);
    json_string(launch, "appId", app_id, sizeof app_id);
    const char *p; size_t n;
    bool snapshot = false;
    if (json_value(launch, "parameters", &p, &n) && n < sizeof params) { memcpy(params, p, n); params[n] = 0; snapshot = json_true(params, "snapshot"); }
    bool hidden = !strcmp(reason, "preload") || snapshot;
    const char *kill_env = getenv("LUMATV_CARD_KILL_MS");
    if (kill_env && atoi(kill_env) > 0) kill_ms = (unsigned)atoi(kill_env);
    setenv("XDG_RUNTIME_DIR", "/tmp/xdg", 0);
    setenv("WAYLAND_DISPLAY", "wayland-0", 0);
    char home[600];
    home_of_exe(home, sizeof home);
    logline("launch: app %s reason %s hidden %d parameters %s, home %s", app_id, reason, hidden, params, home);

    loop = g_main_loop_new(NULL, 0);
    start_app(home, hidden, reason, params);
    g_unix_signal_add(SIGTERM, on_sigterm, NULL);
    g_unix_signal_add(SIGINT, on_sigterm, NULL);
    g_unix_signal_add(SIGHUP, on_sigterm, NULL);
    register_with_sam();
    g_main_loop_run(loop);
    logline("done: status %d", child_status);
    return child_status < 0 ? 1 : child_status;
}
