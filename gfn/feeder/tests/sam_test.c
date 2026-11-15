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

int main(void) {
    struct gf_sam *s = calloc(1, sizeof *s);
    assert(s);
    int fds[2];
    assert(pipe2(fds, O_CLOEXEC | O_NONBLOCK) == 0);
    s->rd = fds[0]; s->wr = fds[1];
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
    LSMessage front = {"{\"returnValue\":true,\"appId\":\"org.nphil.lumagame\"}"};
    on_foreground(NULL, &front, s);
    assert(gf_sam_next(s, line, sizeof line));
    assert(!strcmp(line, "{\"kind\":\"foreground\",\"app_id\":\"org.nphil.lumagame\"}"));
    assert(gf_sam_request(s, "{\"kind\":\"launch\",\"seq\":4,\"app_id\":\"bad\\\"id\"}", line, sizeof line) == -1);
    assert(strstr(line, "invalid app id"));
    assert(gf_sam_request(s, "{\"kind\":\"apps\",\"seq\":-1}", line, sizeof line) == -1);
    assert(gf_sam_request(NULL, "{\"kind\":\"apps\",\"seq\":5}", line, sizeof line) == -1 && strstr(line, "unavailable"));
    assert(calls == 3);
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
