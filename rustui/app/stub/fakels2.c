// A stand-in for LG's libluna-service2.so.3, for the host test of lumatv-card.c (tests/test_card_launcher.py). It has the calls the
// launcher makes, runs on the real glib's main loop, writes every call to FAKE_LS_LOG, and plays SAM from a script:
//   FAKE_SAM_EVENTS  a file of lines "<delay ms> <json>": each json is delivered to the registerApp call's callback that many ms after the call
//   FAKE_LS_REFUSE   set: LSRegister fails as the hub does for an executable it has no role for
// Nothing here is LG's behaviour: it only lets the launcher's own logic run where there is no hub.
#define _GNU_SOURCE
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct LSHandle { int unused; } LSHandle;
typedef struct LSMessage { const char *payload; } LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError { int error_code; char *message; const char *file; int line; const char *func; void *padding; unsigned long magic; } LSError;
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef struct GMainLoop GMainLoop;
typedef int gboolean;
typedef void *gpointer;
unsigned g_timeout_add(unsigned ms, gboolean (*fn)(gpointer), gpointer data);

static unsigned long next_token = 100;

static void flog(const char *fmt, ...) {
    const char *path = getenv("FAKE_LS_LOG");
    if (!path) return;
    FILE *f = fopen(path, "a");
    if (!f) return;
    va_list ap;
    va_start(ap, fmt);
    vfprintf(f, fmt, ap);
    va_end(ap);
    fputc('\n', f);
    fclose(f);
}

static void fail(LSError *e, const char *why) {
    if (e) { e->message = strdup(why); e->error_code = -1; }
}

bool LSErrorInit(LSError *e) { memset(e, 0, sizeof *e); return true; }
void LSErrorFree(LSError *e) { free(e->message); memset(e, 0, sizeof *e); }

bool LSRegister(const char *name, LSHandle **sh, LSError *e) {
    flog("register %s", name);
    if (getenv("FAKE_LS_REFUSE")) { fail(e, "Denied: no role file for executable"); return false; }
    *sh = calloc(1, sizeof **sh);
    return true;
}

bool LSUnregister(LSHandle *sh, LSError *e) { (void)e; flog("unregister"); free(sh); return true; }
bool LSGmainAttach(LSHandle *sh, GMainLoop *loop, LSError *e) { (void)sh; (void)loop; (void)e; flog("attach"); return true; }

struct delivery { LSHandle *sh; LSFilterFunc cb; void *ctx; char *json; };

static gboolean deliver(gpointer data) {
    struct delivery *d = data;
    LSMessage m = { d->json };
    flog("deliver %s", d->json);
    d->cb(d->sh, &m, d->ctx);
    free(d->json);
    free(d);
    return 0;
}

bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e) {
    (void)e;
    flog("call %s %s", uri, payload);
    *token = ++next_token;
    const char *script = getenv("FAKE_SAM_EVENTS");
    if (script && strstr(uri, "registerApp")) {
        FILE *f = fopen(script, "r");
        char line[2048];
        while (f && fgets(line, sizeof line, f)) {
            unsigned delay = 0;
            int used = 0;
            if (sscanf(line, "%u %n", &delay, &used) < 1) continue;
            line[strcspn(line, "\n")] = 0;
            struct delivery *d = calloc(1, sizeof *d);
            d->sh = sh; d->cb = cb; d->ctx = ctx; d->json = strdup(line + used);
            g_timeout_add(delay, deliver, d);
        }
        if (f) fclose(f);
    }
    return true;
}

bool LSCallCancel(LSHandle *sh, LSMessageToken token, LSError *e) { (void)sh; (void)e; flog("cancel %lu", token); return true; }
const char *LSMessageGetPayload(LSMessage *m) { return m->payload; }
