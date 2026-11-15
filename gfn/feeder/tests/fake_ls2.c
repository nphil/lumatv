#define LSCall original_LSCall
#include "../../../rustui/app/stub/fakels2.c"
#undef LSCall

bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e) {
    if (strstr(uri, "registerApp")) return original_LSCall(sh, uri, payload, cb, ctx, token, e);
    flog("call %s %s", uri, payload);
    *token = ++next_token;
    const char *json = NULL;
    if (strstr(uri, "/launch")) json = getenv("FAKE_LAUNCH_REPLY");
    else if (strstr(uri, "/listApps")) json = getenv("FAKE_APPS_REPLY");
    else if (strstr(uri, "/getForegroundAppInfo")) json = getenv("FAKE_FOREGROUND_REPLY");
    if (json) {
        struct delivery *d = calloc(1, sizeof *d);
        d->sh = sh;
        d->cb = cb;
        d->ctx = ctx;
        d->json = strdup(json);
        g_timeout_add(1, deliver, d);
    }
    return true;
}
