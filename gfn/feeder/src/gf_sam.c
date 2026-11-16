#define _GNU_SOURCE
#include "gf_sam.h"

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <ctype.h>
#include <inttypes.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "gf_log.h"

#define SAM_REGISTER "luna://com.webos.applicationManager/registerApp"
#define LS2_LIB "libluna-service2.so.3"
#define GLIB_LIB "libglib-2.0.so.0"
#define SAM_LAUNCH "luna://com.webos.applicationManager/launch"
#define SAM_APPS "luna://com.webos.applicationManager/listApps"
#define SAM_FOREGROUND "luna://com.webos.applicationManager/getForegroundAppInfo"

struct request {
    struct gf_sam *sam;
    int state, apps;
    uint64_t seq;
    char app_id[128], params[1024];
    unsigned long token;
    int64_t deadline_ms;
};

/* ---- luna-service2 and glib: the calls made (the public API; the card launcher makes the same ones) ----------------------------- */

typedef struct LSHandle LSHandle;
typedef struct LSMessage LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError {
    int error_code;
    char *message;
    const char *file;
    int line;
    const char *func;
    void *padding;
    unsigned long magic;
} LSError;
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef struct GMainLoop GMainLoop;
typedef int gboolean;
typedef void *gpointer;
typedef gboolean (*GSourceFunc)(gpointer);

struct gf_sam {
    struct {
        bool (*LSErrorInit)(LSError *);
        void (*LSErrorFree)(LSError *);
        bool (*LSRegister)(const char *, LSHandle **, LSError *);
        bool (*LSUnregister)(LSHandle *, LSError *);
        bool (*LSGmainAttach)(LSHandle *, GMainLoop *, LSError *);
        bool (*LSCall)(LSHandle *, const char *, const char *, LSFilterFunc, void *, LSMessageToken *, LSError *);
        bool (*LSCallCancel)(LSHandle *, LSMessageToken, LSError *);
        const char *(*LSMessageGetPayload)(LSMessage *);
    } ls;
    struct {
        GMainLoop *(*g_main_loop_new)(void *, gboolean);
        void (*g_main_loop_run)(GMainLoop *);
        void (*g_main_loop_quit)(GMainLoop *);
        unsigned (*g_timeout_add)(unsigned, GSourceFunc, gpointer);
    } glib;
    char app_id[128];
    pthread_t thread;
    int started;
    atomic_int stop;
    int rd, wr;                         /* the pipe the thread's lines go through */
    char acc[GF_SAM_LINE];
    size_t acc_len;
    GMainLoop *loop;
    LSHandle *sh;
    LSMessageToken token;
    int called, answered;
    LSMessageToken foreground_token;
    int foreground_called;
    pthread_mutex_t requests_lock;
    struct request requests[8];
};

/* ---- a minimal JSON scanner (lumatv-card.c's: top-level keys, raw values) ----------------------------------------------------------- */

/* The end of the value at `p`: a string, an object or array with what is inside it, or a bare word up to the next `,` or `}`. */
static const char *json_skip(const char *p) {
    int depth = 0;
    if (*p == '"') {
        p++;
        while (*p && *p != '"') {
            if (*p == '\\' && p[1]) p++;
            p++;
        }
        if (*p == '"') p++;
    } else if (*p == '{' || *p == '[') {
        do {
            if (*p == '"') {
                p++;
                while (*p && *p != '"') {
                    if (*p == '\\' && p[1]) p++;
                    p++;
                }
            } else if (*p == '{' || *p == '[') depth++;
            else if (*p == '}' || *p == ']') depth--;
            if (*p) p++;
        } while (*p && depth > 0);
    } else {
        while (*p && *p != ',' && *p != '}') p++;
    }
    return p;
}

static bool json_value(const char *obj, const char *key, const char **start, size_t *len) {
    const char *p = obj;
    while (*p && *p != '{') p++;
    if (*p != '{') return false;
    p++;
    size_t klen = strlen(key);
    while (*p) {
        while (*p == ' ' || *p == ',' || *p == '\n' || *p == '\r' || *p == '\t') p++;
        if (*p == '}') return false;
        if (*p != '"') return false;
        const char *k = ++p;
        while (*p && *p != '"') {
            if (*p == '\\' && p[1]) p++;
            p++;
        }
        if (!*p) return false;
        size_t kl = (size_t)(p - k);
        p++;
        while (*p == ' ' || *p == ':' || *p == '\t' || *p == '\r' || *p == '\n') p++;
        const char *v = p;
        p = json_skip(p);
        if (kl == klen && !strncmp(k, key, klen)) {
            *start = v;
            *len = (size_t)(p - v);
            return true;
        }
    }
    return false;
}

static bool json_string(const char *obj, const char *key, char *out, size_t outsz) {
    const char *v;
    size_t n;
    if (!json_value(obj, key, &v, &n) || n < 2 || v[0] != '"' || v[n - 1] != '"' || n - 2 >= outsz) return false;
    size_t o = 0;
    for (size_t i = 1; i + 1 < n && o + 1 < outsz; i++) {
        char c = v[i];
        if (c == '\\' && i + 2 < n) {
            i++;
            c = v[i];
            if (c == 'n') c = '\n';
            else if (c == 't') c = '\t';
        }
        out[o++] = c;
    }
    out[o] = 0;
    return true;
}

static bool json_true(const char *obj, const char *key) {
    const char *v;
    size_t n;
    return json_value(obj, key, &v, &n) && n >= 4 && !strncmp(v, "true", 4);
}

/* A string field of an app as SAM wrote it, quotes and escapes included, so it is JSON already. False when it is absent or not a string. */
static bool raw_string(const char *app, const char *key, const char **v, size_t *n) {
    return json_value(app, key, v, n) && *n >= 2 && (*v)[0] == '"' && (*v)[*n - 1] == '"';
}

#define APPS_TOO_LARGE "the installed app list exceeds the lifecycle limit"

/* SAM's listApps array (`apps`, `count` bytes) as the UI reads it: each app's id, title and icon and nothing else of what SAM
 * knows about it, written to `out` as an array. The UI's own parse needs a string id and title, so an app without both would have
 * failed the whole list there; it is left out here instead. Field lengths are the UI's to judge (it drops an id it cannot use and
 * cuts a label); only the whole list is bounded. Returns NULL, or the reason the list is refused: not an array of objects, or what
 * the UI reads of it still does not fit. */
static const char *project_apps(const char *apps, size_t count, char *out, size_t cap) {
    const char *last = apps + count - 1, *p = apps + 1;
    if (count < 2 || *last != ']' || cap < 3) return "the installed app list is malformed";
    size_t o = 0;
    out[o++] = '[';
    for (;;) {
        while (p < last && (*p == ',' || isspace((unsigned char)*p))) p++;
        if (p >= last) break;
        const char *end = p;
        if (*p != '{' || (end = json_skip(p)) > last) return "the installed app list is malformed";
        const char *id, *title, *icon = "";
        size_t idn, titlen, iconn = 0;
        if (raw_string(p, "id", &id, &idn) && raw_string(p, "title", &title, &titlen)) {
            bool has_icon = raw_string(p, "icon", &icon, &iconn);
            int w = snprintf(out + o, cap - o, "%s{\"id\":%.*s,\"title\":%.*s%s%.*s}", o > 1 ? "," : "", (int)idn, id, (int)titlen, title,
                             has_icon ? ",\"icon\":" : "", (int)(has_icon ? iconn : 0), icon);
            if (w < 0 || (size_t)w + 2 > cap - o) return APPS_TOO_LARGE;
            o += (size_t)w;
        }
        p = end;
    }
    out[o++] = ']';
    out[o] = 0;
    return NULL;
}

/* A string as the body of a JSON string. */
static void json_escape(const char *s, char *out, size_t outsz) {
    size_t o = 0;
    for (; *s && o + 7 < outsz; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') {
            out[o++] = '\\';
            out[o++] = (char)c;
        } else if (c == '\n') {
            out[o++] = '\\';
            out[o++] = 'n';
        } else if (c < 0x20) o += (size_t)snprintf(out + o, outsz - o, "\\u%04x", c);
        else out[o++] = (char)c;
    }
    out[o] = 0;
}

int gf_sam_parse_launch(const char *json, struct gf_launch *out) {
    memset(out, 0, sizeof *out);
    if (!json || json[0] != '{') return -1;
    char event[40] = "";
    json_string(json, "event", event, sizeof event);
    if (strcmp(event, "launch") != 0 && !json_true(json, "@system_native_app")) return -1;
    json_string(json, "appId", out->app_id, sizeof out->app_id);
    json_string(json, "reason", out->reason, sizeof out->reason);
    const char *params;
    size_t n;
    int snapshot = 0;
    if (json_value(json, "parameters", &params, &n) && n > 1 && params[0] == '{') {
        char copy[1024];
        if (n < sizeof copy) {
            memcpy(copy, params, n);
            copy[n] = 0;
            snapshot = json_true(copy, "snapshot");
        }
    }
    out->preload = !strcmp(out->reason, "preload") || snapshot;
    return 0;
}

/* ---- the thread ------------------------------------------------------------------------------------------------------------------ */

/* One producer on the bus thread; large app lists can span several pipe writes. */
static void emit(struct gf_sam *s, const char *line) {
    size_t left = strlen(line);
    if (left + 1 >= GF_SAM_LINE) return;
    while (left) {
        ssize_t n = write(s->wr, line, left);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return;
        line += n;
        left -= (size_t)n;
    }
    if (write(s->wr, "\n", 1) < 0) { /* the reader is gone */ }
}

static bool on_sam(LSHandle *h, LSMessage *msg, void *ctx) {
    (void)h;
    struct gf_sam *s = ctx;
    const char *payload = s->ls.LSMessageGetPayload(msg);
    if (!payload) payload = "{}";
    char event[40] = "", reason[80] = "";
    json_string(payload, "event", event, sizeof event);
    json_string(payload, "reason", reason, sizeof reason);
    char reason_esc[200], line[1400];
    json_escape(reason, reason_esc, sizeof reason_esc);
    if (!strcmp(event, "registered")) {
        s->answered = 1;
        gf_say("sam: registered");
        emit(s, "{\"kind\":\"sam\",\"registered\":true}");
    } else if (!strcmp(event, "relaunch")) {
        const char *p;
        size_t n;
        char params[1024] = "{}";
        if (json_value(payload, "parameters", &p, &n) && n < sizeof params) {
            memcpy(params, p, n);
            params[n] = 0;
        }
        snprintf(line, sizeof line, "{\"kind\":\"relaunch\",\"reason\":\"%s\",\"parameters\":%s}", reason_esc, params);
        gf_say("sam: relaunch (%s) %s", reason, params);
        emit(s, line);
    } else if (!strcmp(event, "close")) {
        snprintf(line, sizeof line, "{\"kind\":\"close\",\"reason\":\"%s\"}", reason_esc);
        gf_say("sam: close (%s)", reason);
        emit(s, line);
    } else if (!s->answered && !json_true(payload, "returnValue")) {
        /* an answer with no event that says no: the call was refused (the role has no outbound to SAM, or SAM does not know the app) */
        s->answered = 1;
        gf_say("sam: registerApp refused: %.200s", payload);
        emit(s, "{\"kind\":\"sam\",\"registered\":false}");
    } else {
        gf_say("sam: %.200s", payload);
    }
    return true;
}

static void request_error(const struct request *r, const char *message, char *out, size_t cap) {
    char escaped[512];
    json_escape(message, escaped, sizeof escaped);
    snprintf(out, cap, "{\"kind\":\"%s\",\"seq\":%" PRIu64 ",\"app_id\":\"%s\",\"success\":false,\"message\":\"%s\"%s}",
             r->apps ? "apps" : "launched", r->seq, r->app_id, escaped, r->apps ? ",\"apps\":[]" : "");
}

static bool on_request(LSHandle *h, LSMessage *msg, void *ctx) {
    (void)h;
    struct request *r = ctx;
    struct gf_sam *s = r->sam;
    const char *payload = s->ls.LSMessageGetPayload(msg);
    if (!payload) payload = "{}";
    const char *apps = "[]";
    size_t count = 2;
    int ok = json_true(payload, "returnValue");
    if (ok && r->apps && (!json_value(payload, "apps", &apps, &count) || *apps != '[')) ok = 0;
    /* A reply is one line and a line is bounded, so the list goes on as what the UI reads of it, not as SAM wrote it. */
    size_t size = ok && r->apps ? GF_SAM_LINE - 1 : 1024;
    char *line = malloc(size);
    if (line) {
        const char *refused = NULL;
        if (ok && r->apps) {
            size_t head = (size_t)snprintf(line, size, "{\"kind\":\"apps\",\"seq\":%" PRIu64 ",\"success\":true,\"apps\":", r->seq);
            refused = project_apps(apps, count, line + head, size - head - 2);      /* room for the closing brace */
            if (!refused) strcat(line + head, "}");
        }
        if (!ok || refused) {
            char message[200] = "application manager refused the request";
            json_string(payload, "errorText", message, sizeof message);
            if (refused) snprintf(message, sizeof message, "%s", refused);
            request_error(r, message, line, 1024);
        } else if (!r->apps) {
            snprintf(line, size, "{\"kind\":\"launched\",\"seq\":%" PRIu64 ",\"app_id\":\"%s\",\"success\":true,\"message\":\"\"}", r->seq, r->app_id);
        }
        emit(s, line);
        free(line);
    }
    pthread_mutex_lock(&s->requests_lock);
    r->state = 0;
    pthread_mutex_unlock(&s->requests_lock);
    return true;
}

static bool on_foreground(LSHandle *h, LSMessage *msg, void *ctx) {
    (void)h;
    struct gf_sam *s = ctx;
    const char *payload = s->ls.LSMessageGetPayload(msg);
    char id[128] = "", escaped[300], line[400];
    if (!payload || !json_true(payload, "returnValue") || !json_string(payload, "appId", id, sizeof id)) return true;
    json_escape(id, escaped, sizeof escaped);
    snprintf(line, sizeof line, "{\"kind\":\"foreground\",\"app_id\":\"%s\"}", escaped);
    emit(s, line);
    return true;
}

int gf_sam_request(gf_sam *s, const char *line, char *reply, size_t cap) {
    char kind[32] = "";
    reply[0] = 0;
    if (!json_string(line, "kind", kind, sizeof kind) || (strcmp(kind, "launch") && strcmp(kind, "apps"))) return 0;
    struct request r = {.sam = s, .apps = !strcmp(kind, "apps")};
    const char *v;
    size_t n;
    char seq[24] = "";
    if (json_value(line, "seq", &v, &n) && n && n < sizeof seq) {
        memcpy(seq, v, n);
        char *end;
        errno = 0;
        r.seq = strtoull(seq, &end, 10);
        while (isspace((unsigned char)*end)) end++;
        if (errno || *end || !isdigit((unsigned char)seq[0])) {
            r.seq = 0;
            request_error(&r, "invalid request sequence", reply, cap);
            return -1;
        }
    } else {
        request_error(&r, "missing request sequence", reply, cap);
        return -1;
    }
    if (!r.apps) {
        json_string(line, "app_id", r.app_id, sizeof r.app_id);
        int valid = r.app_id[0] != 0;
        for (const char *p = r.app_id; *p; p++)
            if (!isalnum((unsigned char)*p) && *p != '.' && *p != '-' && *p != '_') valid = 0;
        if (!valid) {
            r.app_id[0] = 0;
            request_error(&r, "invalid app id", reply, cap);
            return -1;
        }
        snprintf(r.params, sizeof r.params, "{}");
        if (json_value(line, "parameters", &v, &n)) {
            if (n >= sizeof r.params || n < 2 || v[0] != '{' || v[n - 1] != '}') {
                request_error(&r, "launch parameters must be an object under 1024 bytes", reply, cap);
                return -1;
            }
            memcpy(r.params, v, n);
            r.params[n] = 0;
        }
    }
    if (!s || atomic_load(&s->stop)) {
        request_error(&r, "application manager is unavailable", reply, cap);
        return -1;
    }
    pthread_mutex_lock(&s->requests_lock);
    for (size_t i = 0; i < sizeof s->requests / sizeof *s->requests; i++) {
        if (s->requests[i].state) continue;
        r.state = 1;
        s->requests[i] = r;
        pthread_mutex_unlock(&s->requests_lock);
        return 1;
    }
    pthread_mutex_unlock(&s->requests_lock);
    request_error(&r, "application manager request queue is full", reply, cap);
    return -1;
}

static void send_requests(struct gf_sam *s) {
    for (size_t i = 0; i < sizeof s->requests / sizeof *s->requests; i++) {
        struct request *r = &s->requests[i];
        pthread_mutex_lock(&s->requests_lock);
        int send = r->state == 1;
        int expired = r->state == 2 && gf_ms() >= r->deadline_ms;
        if (send) {
            r->state = 2;
            r->deadline_ms = gf_ms() + 10000;
        }
        pthread_mutex_unlock(&s->requests_lock);
        if (expired) {
            LSError e;
            s->ls.LSErrorInit(&e);
            s->ls.LSCallCancel(s->sh, r->token, &e);
            s->ls.LSErrorFree(&e);
            char line[1024];
            request_error(r, "application manager did not reply within 10 seconds", line, sizeof line);
            emit(s, line);
            pthread_mutex_lock(&s->requests_lock);
            r->state = 0;
            pthread_mutex_unlock(&s->requests_lock);
        }
        if (!send) continue;
        char payload[1400];
        if (r->apps) snprintf(payload, sizeof payload, "{}");
        else snprintf(payload, sizeof payload, "{\"id\":\"%s\",\"params\":%s}", r->app_id, r->params);
        LSError e;
        s->ls.LSErrorInit(&e);
        if (!s->ls.LSCall(s->sh, r->apps ? SAM_APPS : SAM_LAUNCH, payload, on_request, r, &r->token, &e)) {
            char line[1024];
            request_error(r, e.message ? e.message : "application manager call failed", line, sizeof line);
            emit(s, line);
            pthread_mutex_lock(&s->requests_lock);
            r->state = 0;
            pthread_mutex_unlock(&s->requests_lock);
        }
        s->ls.LSErrorFree(&e);
    }
}

static gboolean on_tick(gpointer data) {
    struct gf_sam *s = data;
    if (atomic_load(&s->stop)) s->glib.g_main_loop_quit(s->loop);
    else send_requests(s);
    return 1;
}

static void *sam_thread(void *arg) {
    struct gf_sam *s = arg;
    sigset_t all;
    sigfillset(&all);
    pthread_sigmask(SIG_BLOCK, &all, NULL);     /* the feeder's own thread takes the signals */
    LSError e;
    s->ls.LSErrorInit(&e);
    if (!s->ls.LSRegister(s->app_id, &s->sh, &e)) {
        gf_say("sam: LSRegister %s: %s (the hub gives no bus name to this executable; the app runs without SAM's events)", s->app_id, e.message ? e.message : "?");
        s->ls.LSErrorFree(&e);
        s->sh = NULL;
        emit(s, "{\"kind\":\"sam\",\"registered\":false}");
        atomic_store(&s->stop, 1);
        return NULL;
    }
    s->loop = s->glib.g_main_loop_new(NULL, 0);
    if (!s->ls.LSGmainAttach(s->sh, s->loop, &e)) gf_say("sam: LSGmainAttach: %s", e.message ? e.message : "?");
    s->ls.LSErrorFree(&e);
    s->ls.LSErrorInit(&e);
    char esc[300], payload[400];
    json_escape(s->app_id, esc, sizeof esc);
    snprintf(payload, sizeof payload, "{\"appId\":\"%s\",\"subscribe\":true}", esc);
    if (!s->ls.LSCall(s->sh, SAM_REGISTER, payload, on_sam, s, &s->token, &e)) {
        gf_say("sam: LSCall registerApp: %s", e.message ? e.message : "?");
        emit(s, "{\"kind\":\"sam\",\"registered\":false}");
    } else {
        s->called = 1;
        gf_say("sam: registered %s, registerApp sent: %s", s->app_id, payload);
    }
    s->ls.LSErrorFree(&e);
    s->ls.LSErrorInit(&e);
    s->foreground_called = s->ls.LSCall(s->sh, SAM_FOREGROUND, "{\"subscribe\":true}", on_foreground, s, &s->foreground_token, &e);
    s->ls.LSErrorFree(&e);
    s->glib.g_timeout_add(100, on_tick, s);
    if (!atomic_load(&s->stop)) s->glib.g_main_loop_run(s->loop);
    s->ls.LSErrorInit(&e);
    if (s->foreground_called) s->ls.LSCallCancel(s->sh, s->foreground_token, &e);
    for (size_t i = 0; i < sizeof s->requests / sizeof *s->requests; i++)
        if (s->requests[i].state == 2) s->ls.LSCallCancel(s->sh, s->requests[i].token, &e);
    if (s->called) s->ls.LSCallCancel(s->sh, s->token, &e);
    s->ls.LSErrorFree(&e);
    s->ls.LSErrorInit(&e);
    s->ls.LSUnregister(s->sh, &e);
    s->ls.LSErrorFree(&e);
    s->sh = NULL;
    return NULL;
}

/* ---- the feeder's side ------------------------------------------------------------------------------------------------------------ */

#define NEED(handle, field, name)                                                                                        \
    do {                                                                                                                 \
        *(void **)(&field) = dlsym(handle, name);                                                                        \
        if (!(field)) {                                                                                                  \
            snprintf(err, cap, "%s has no %s", #handle, name);                                                           \
            goto fail;                                                                                                   \
        }                                                                                                                \
    } while (0)

gf_sam *gf_sam_start(const char *app_id, char *err, size_t cap) {
    struct gf_sam *s = calloc(1, sizeof *s);
    void *ls2 = NULL, *glib = NULL;
    int fds[2] = {-1, -1};
    if (!s) {
        snprintf(err, cap, "out of memory");
        return NULL;
    }
    pthread_mutex_init(&s->requests_lock, NULL);
    snprintf(s->app_id, sizeof s->app_id, "%s", app_id);
    ls2 = dlopen(LS2_LIB, RTLD_NOW | RTLD_GLOBAL);
    if (!ls2) {
        snprintf(err, cap, "%s", dlerror());
        goto fail;
    }
    glib = dlopen(GLIB_LIB, RTLD_NOW | RTLD_GLOBAL);
    if (!glib) {
        snprintf(err, cap, "%s", dlerror());
        goto fail;
    }
    NEED(ls2, s->ls.LSErrorInit, "LSErrorInit");
    NEED(ls2, s->ls.LSErrorFree, "LSErrorFree");
    NEED(ls2, s->ls.LSRegister, "LSRegister");
    NEED(ls2, s->ls.LSUnregister, "LSUnregister");
    NEED(ls2, s->ls.LSGmainAttach, "LSGmainAttach");
    NEED(ls2, s->ls.LSCall, "LSCall");
    NEED(ls2, s->ls.LSCallCancel, "LSCallCancel");
    NEED(ls2, s->ls.LSMessageGetPayload, "LSMessageGetPayload");
    NEED(glib, s->glib.g_main_loop_new, "g_main_loop_new");
    NEED(glib, s->glib.g_main_loop_run, "g_main_loop_run");
    NEED(glib, s->glib.g_main_loop_quit, "g_main_loop_quit");
    NEED(glib, s->glib.g_timeout_add, "g_timeout_add");
    if (pipe2(fds, O_CLOEXEC | O_NONBLOCK) != 0) {
        snprintf(err, cap, "pipe: %s", strerror(errno));
        goto fail;
    }
    s->rd = fds[0];
    s->wr = fds[1];
    /* the writing end blocks (a line is whole or waits); only the reading end is polled */
    fcntl(s->wr, F_SETFL, fcntl(s->wr, F_GETFL) & ~O_NONBLOCK);
    if (pthread_create(&s->thread, NULL, sam_thread, s) != 0) {
        snprintf(err, cap, "pthread_create: %s", strerror(errno));
        goto fail;
    }
    s->started = 1;
    return s;
fail:
    if (fds[0] >= 0) close(fds[0]);
    if (fds[1] >= 0) close(fds[1]);
    if (ls2) dlclose(ls2);
    if (glib) dlclose(glib);
    pthread_mutex_destroy(&s->requests_lock);
    free(s);
    return NULL;
}

int gf_sam_fd(const gf_sam *s) { return s ? s->rd : -1; }

int gf_sam_next(gf_sam *s, char *line, size_t cap) {
    if (!s) return 0;
    for (;;) {
        char *nl = memchr(s->acc, '\n', s->acc_len);
        if (nl) {
            size_t n = (size_t)(nl - s->acc);
            size_t take = n < cap - 1 ? n : cap - 1;
            memcpy(line, s->acc, take);
            line[take] = 0;
            memmove(s->acc, nl + 1, s->acc_len - n - 1);
            s->acc_len -= n + 1;
            return 1;
        }
        if (s->acc_len == sizeof s->acc) s->acc_len = 0;        /* a line that never ends is not one */
        ssize_t r = read(s->rd, s->acc + s->acc_len, sizeof s->acc - s->acc_len);
        if (r <= 0) return 0;
        s->acc_len += (size_t)r;
    }
}

void gf_sam_stop(gf_sam *s) {
    if (!s) return;
    atomic_store(&s->stop, 1);
    close(s->rd);                  /* release a large reply blocked in the pipe before joining */
    if (s->started) pthread_join(s->thread, NULL);
    close(s->wr);
    pthread_mutex_destroy(&s->requests_lock);
    free(s);
}
