/* the asserts here do work (zig cc defines NDEBUG from -O1 on unless a sanitizer is on) */
#undef NDEBUG
#include "../src/gf_sam.c"
#include <assert.h>

struct LSMessage { const char *payload; };
static const char *reply_payload;
static char called_uri[128], called_payload[2048];
static unsigned calls;
static int64_t now_ns;
int64_t gf_ns(void) { return now_ns; }
static unsigned cancelled;
static bool cancel(LSHandle *sh, LSMessageToken token, LSError *e) {
    (void)sh; (void)token; (void)e;
    cancelled++;
    return true;
}

static const char *payload(LSMessage *msg) { return msg->payload; }
static bool error_init(LSError *e) { memset(e, 0, sizeof *e); return true; }
static void error_free(LSError *e) { (void)e; }
static bool call(LSHandle *sh, const char *uri, const char *body, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *err) {
    (void)err;
    snprintf(called_uri, sizeof called_uri, "%s", uri);
    snprintf(called_payload, sizeof called_payload, "%s", body);
    *token = ++calls;
    LSMessage msg = {reply_payload};
    if (reply_payload) cb(sh, &msg, ctx);
    return true;
}

/* An installed list as SAM writes it: each app carries far more than the UI reads (a description, nested metadata with braces in strings). */
static char *sam_list(int n, int extras) {
    char pad[801];
    memset(pad, 'x', 800);
    pad[800] = 0;
    size_t cap = (size_t)n * 1400 + 4096;
    char *s = malloc(cap);
    assert(s);
    size_t o = (size_t)snprintf(s, cap, "{\"returnValue\":true,\"apps\":[");
    for (int i = 0; i < n; i++) {
        o += (size_t)snprintf(s + o, cap - o,
            "%s{\"id\":\"com.example.app%d\",\"title\":\"App %d \\\"Caf\\u00e9\\\" \\\\ }{\",\"icon\":\"/media/icons/%d.png\",\"type\":\"web\","
            "\"visible\":true,\"description\":\"%s\",\"nested\":{\"a\":[\"]\",\"}\"],\"b\":\"\\\"\"}}", i ? "," : "", i, i, i, pad);
    }
    if (extras) {
        char icon[601];
        memset(icon, 'i', 600);
        icon[600] = 0;
        o += (size_t)snprintf(s + o, cap - o, ",{\"id\":\"com.example.notitle\"},{\"title\":\"No id\"},{\"id\":\"com.example.bigicon\",\"title\":\"%s\",\"icon\":\"%s\"}", icon, icon);
    }
    snprintf(s + o, cap - o, "]}");
    return s;
}

static size_t count(const char *s, const char *needle) {
    size_t n = 0;
    for (; (s = strstr(s, needle)); s++) n++;
    return n;
}

static void *run_requests(void *s) {
    send_requests(s);
    return NULL;
}

/* One apps request answered with `payload`; the reply line is in `out` (GF_SAM_LINE bytes). As on the TV the reply is written on
 * another thread than the one that reads it, since a pipe holds less than a reply. */
static void ask_apps(struct gf_sam *s, const char *payload, char *out) {
    char line[256];
    pthread_t writer;
    reply_payload = payload;
    assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":30}", line, sizeof line) == 1);
    assert(pthread_create(&writer, NULL, run_requests, s) == 0);
    for (int spins = 0; !gf_sam_next(s, out, GF_SAM_LINE); spins++) {
        assert(spins < 5000);
        usleep(1000);
    }
    pthread_join(writer, NULL);
}

int main(void) {
    struct gf_sam *s = calloc(1, sizeof *s);
    assert(s);
    int fds[2];
    assert(pipe2(fds, O_CLOEXEC | O_NONBLOCK) == 0);
    s->rd = fds[0]; s->wr = fds[1];
    fcntl(s->wr, F_SETFL, fcntl(s->wr, F_GETFL) & ~O_NONBLOCK);     /* the writing end blocks, as in gf_sam_start */
    pthread_mutex_init(&s->requests_lock, NULL);
    s->ls.LSMessageGetPayload = payload;
    s->ls.LSCall = call;
    s->ls.LSErrorInit = error_init;
    s->ls.LSErrorFree = error_free;
    s->ls.LSCallCancel = cancel;
    char line[4096];
    reply_payload = "{\"returnValue\":true}";
    assert(gf_sam_request(s, "{\"kind\":\"launch\",\"seq\":18446744073709551615,\"app_id\":\"com.example.game\",\"parameters\":{\"level\":3}}", line, sizeof line) == 1);
    send_requests(s);
    assert(!strcmp(called_uri, SAM_LAUNCH));
    assert(!strcmp(called_payload, "{\"id\":\"com.example.game\",\"params\":{\"level\":3}}"));
    assert(gf_sam_next(s, line, sizeof line));
    assert(strstr(line, "\"kind\":\"launched\"") && strstr(line, "\"success\":true") && strstr(line, "18446744073709551615"));
    reply_payload = "{\"returnValue\":false,\"errorText\":\"missing game\"}";
    assert(gf_sam_request(s, "{\"kind\":\"launch\",\"seq\":2,\"app_id\":\"com.example.missing\"}", line, sizeof line) == 1);
    send_requests(s);
    assert(gf_sam_next(s, line, sizeof line));
    assert(strstr(line, "\"success\":false") && strstr(line, "missing game"));
    reply_payload = "{\"returnValue\":true,\"apps\":[{\"id\":\"com.example.game\",\"title\":\"Game\",\"icon\":\"icon.png\"}]}";
    assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":3}", line, sizeof line) == 1);
    send_requests(s);
    assert(!strcmp(called_uri, SAM_APPS));
    assert(gf_sam_next(s, line, sizeof line));
    assert(strstr(line, "\"kind\":\"apps\"") && strstr(line, "icon.png") && strstr(line, "\"success\":true"));
    /* A real installed list is far over the line limit as SAM writes it; the UI is sent what it reads of each app. */
    {
        char *big = malloc(GF_SAM_LINE), *list = sam_list(300, 1);
        assert(big && strlen(list) > GF_SAM_LINE);
        ask_apps(s, list, big);
        assert(strstr(big, "\"success\":true") && strlen(big) < GF_SAM_LINE);
        assert(count(big, "\"id\":\"com.example.app") == 300 && !strstr(big, "description") && !strstr(big, "nested"));
        assert(strstr(big, "\"title\":\"App 7 \\\"Caf\\u00e9\\\" \\\\ }{\",\"icon\":\"/media/icons/7.png\"}"));
        char i600[601];                 /* a long title and a long icon are the UI's to cut, not dropped here */
        memset(i600, 'i', 600);
        i600[600] = 0;
        assert(strstr(big, "com.example.bigicon") && count(big, i600) == 2);
        assert(!strstr(big, "notitle") && !strstr(big, "No id"));
        free(list);
        list = sam_list(2500, 0);       /* what the UI reads of this many apps still does not fit a line: refused, not cut */
        ask_apps(s, list, big);
        assert(strstr(big, "\"success\":false") && strstr(big, "exceeds") && strstr(big, "\"apps\":[]"));
        free(list);
        ask_apps(s, "{\"returnValue\":true,\"apps\":[{\"id\":\"a\",\"title\":\"A\"},{\"id\":\"b\"", big);
        assert(strstr(big, "\"success\":false") && strstr(big, "\"apps\":[]"));
        ask_apps(s, "{\"returnValue\":true,\"apps\":[1]}", big);
        assert(strstr(big, "\"success\":false"));
        ask_apps(s, "{\"returnValue\":true,\"apps\":[]}", big);
        assert(!strcmp(big, "{\"kind\":\"apps\",\"seq\":30,\"success\":true,\"apps\":[]}"));
        free(big);
    }
    LSMessage front = {"{\"returnValue\":true,\"appId\":\"org.nphil.lumagame\"}"};
    on_foreground(NULL, &front, s);
    assert(gf_sam_next(s, line, sizeof line));
    assert(!strcmp(line, "{\"kind\":\"foreground\",\"app_id\":\"org.nphil.lumagame\"}"));
    assert(gf_sam_request(s, "{\"kind\":\"launch\",\"seq\":4,\"app_id\":\"bad\\\"id\"}", line, sizeof line) == -1);
    assert(strstr(line, "invalid app id"));
    assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":-1}", line, sizeof line) == -1);
    assert(gf_sam_request(NULL, "{\"kind\":\"apps\",\"seq\":5}", line, sizeof line) == -1 && strstr(line, "unavailable"));
    assert(calls == 3 + 5);
    for (int i = 0; i < 8; i++) assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":6}", line, sizeof line) == 1);
    assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":7}", line, sizeof line) == -1 && strstr(line, "queue is full"));
    assert(gf_sam_request(s, "{\"kind\":\"ready\"}", line, sizeof line) == 0);
    reply_payload = NULL;
    send_requests(s);
    now_ns = 10000000000LL;
    send_requests(s);
    assert(cancelled == 8);
    unsigned timeouts = 0;
    while (gf_sam_next(s, line, sizeof line)) {
        assert(strstr(line, "did not reply within 10 seconds"));
        timeouts++;
    }
    assert(timeouts == 8);
    gf_sam_stop(s);
    puts("sam_test: lifecycle rule passed (launch, apps, foreground, errors)");
    return 0;
}
