#define _GNU_SOURCE
#include "gf_sam.h"

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
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
    char acc[4096];                     /* what the reader has of the next line */
    size_t acc_len;
    GMainLoop *loop;
    LSHandle *sh;
    LSMessageToken token;
    int called, answered;
};

/* ---- a minimal JSON scanner (lumatv-card.c's: top-level keys, raw values) ----------------------------------------------------------- */

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
        while (*p && *p != '"') {
            if (*p == '\\') p++;
            p++;
        }
        size_t kl = (size_t)(p - k);
        p++;
        while (*p == ' ' || *p == ':') p++;
        const char *v = p;
        int depth = 0;
        if (*p == '"') {
            p++;
            while (*p && *p != '"') {
                if (*p == '\\') p++;
                p++;
            }
            if (*p == '"') p++;
        } else if (*p == '{' || *p == '[') {
            do {
                if (*p == '"') {
                    p++;
                    while (*p && *p != '"') {
                        if (*p == '\\') p++;
                        p++;
                    }
                } else if (*p == '{' || *p == '[') depth++;
                else if (*p == '}' || *p == ']') depth--;
                p++;
            } while (*p && depth > 0);
        } else {
            while (*p && *p != ',' && *p != '}') p++;
        }
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
    if (!json_value(obj, key, &v, &n) || n < 2 || v[0] != '"') return false;
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

/* One line to the feeder. A line is far under PIPE_BUF, so the write is whole or not made; a reader that is not reading loses it. */
static void emit(struct gf_sam *s, const char *line) {
    char buf[2048];
    size_t n = strlen(line);
    if (n + 2 > sizeof buf) {
        gf_say("sam: a message of %zu bytes is too long and is dropped", n);
        return;
    }
    memcpy(buf, line, n);
    buf[n++] = '\n';
    if (write(s->wr, buf, n) != (ssize_t)n) gf_say("sam: a line was not written (%s)", strerror(errno));
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

static gboolean on_tick(gpointer data) {
    struct gf_sam *s = data;
    if (atomic_load(&s->stop)) s->glib.g_main_loop_quit(s->loop);
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
    s->glib.g_timeout_add(100, on_tick, s);
    if (!atomic_load(&s->stop)) s->glib.g_main_loop_run(s->loop);
    s->ls.LSErrorInit(&e);
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
    if (s->started) pthread_join(s->thread, NULL);
    close(s->rd);
    close(s->wr);
    free(s);
}
