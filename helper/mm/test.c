// Host tests for lumatv-mm.c (built with -DLUMATV_MM_TEST, which includes this file in place of main): a fake
// libluna-service2 records every answer, and the callers recorded on the set (docs/research/arch/x4, 2026-11-20)
// are replayed against it.

struct LSHandle { int unused; };
struct LSMessage { const char *method, *payload, *sender; };
static struct LSHandle fake_handle;
static char replies[16][1024];
static int nreplies;
static char subs[16][64];
static int nsubs;
static char pushes[16][600];
static int npushes;
static char sent[16][600];
static int nsent;

bool LSErrorInit(LSError *e) { memset(e, 0, sizeof *e); return true; }
void LSErrorFree(LSError *e) { (void)e; }
bool LSRegister(const char *name, LSHandle **sh, LSError *e) { (void)name; (void)e; *sh = &fake_handle; return true; }
bool LSUnregister(LSHandle *sh, LSError *e) { (void)sh; (void)e; return true; }
bool LSRegisterCategory(LSHandle *sh, const char *c, LSMethod *m, LSSignal *s, void *p, LSError *e) { (void)sh; (void)c; (void)m; (void)s; (void)p; (void)e; return true; }
bool LSGmainAttach(LSHandle *sh, GMainLoop *l, LSError *e) { (void)sh; (void)l; (void)e; return true; }
bool LSCallOneReply(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *t, LSError *e) {
    (void)sh; (void)cb; (void)t; (void)e;
    snprintf(sent[nsent++ % 16], 600, "call %s %s", uri, payload);
    free(ctx);
    return true;
}
bool LSSignalSend(LSHandle *sh, const char *uri, const char *payload, LSError *e) {
    (void)sh; (void)e;
    snprintf(sent[nsent++ % 16], 600, "signal %s %s", uri, payload);
    return true;
}
const char *LSMessageGetPayload(LSMessage *m) { return m->payload; }
const char *LSMessageGetMethod(LSMessage *m) { return m->method; }
const char *LSMessageGetSender(LSMessage *m) { return m->sender; }
const char *LSMessageGetSenderServiceName(LSMessage *m) { return m->sender; }
bool LSMessageIsSubscription(LSMessage *m) { return json_bool(m->payload, "subscribe"); }
bool LSMessageReply(LSHandle *sh, LSMessage *m, const char *payload, LSError *e) { (void)sh; (void)m; (void)e; snprintf(replies[nreplies++ % 16], 1024, "%s", payload); return true; }
bool LSSubscriptionAdd(LSHandle *sh, const char *key, LSMessage *m, LSError *e) { (void)sh; (void)m; (void)e; snprintf(subs[nsubs++ % 16], 64, "%s", key); return true; }
bool LSSubscriptionReply(LSHandle *sh, const char *key, const char *payload, LSError *e) { (void)sh; (void)e; snprintf(pushes[npushes++ % 16], 600, "%s %s", key, payload); return true; }

static int failed;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #c); failed++; } } while (0)
static char tmp[200];

static const char *call(const char *method, const char *payload, const char *sender) {
    LSMessage m = { method, payload, sender };
    int before = nreplies;
    on_method(&fake_handle, &m, NULL);
    CHECK(nreplies == before + 1);
    return replies[(nreplies - 1) % 16];
}

static void put(const char *name, const char *text) {
    char p[300];
    snprintf(p, sizeof p, "%s/%s", tmp, name);
    FILE *f = fopen(p, "w");
    fputs(text, f);
    fclose(f);
}

static void reset_state(void) {
    char cmd[300];
    snprintf(cmd, sizeof cmd, "rm -rf %s/state && mkdir -p %s/state", tmp, tmp);
    CHECK(system(cmd) == 0);
    memset(&hs, 0, sizeof hs);
    bridge_fd = -1;
    bridge_broken = false;
    nreplies = nsubs = npushes = nsent = 0;
}

static void test_levels(void) {
    // Falling: medium below 250 MB, low below 200, critical below 45.7.
    CHECK(level_step(0, 74000) == 0);
    CHECK(level_step(0, 63999) == 1);
    CHECK(level_step(0, 51199) == 2);
    CHECK(level_step(0, 11000) == 3);
    // Rising: low is left above 220 MB to medium, medium above 290.8 MB.
    CHECK(level_step(2, 56000) == 2);
    CHECK(level_step(2, 56321) == 1);
    CHECK(level_step(1, 74453) == 1);
    CHECK(level_step(1, 74454) == 0);
    CHECK(level_step(3, 80000) == 0);
    // Reboot is never entered.
    CHECK(level_step(3, 0) == 3);
}

// The x4 callers: SAM's launches, WAM's poll, preloadmanager, the browser.
static void test_callers(void) {
    reset_state();
    put("usable", "218002\n");
    const char *launches[] = {
        "{\"defaultWindowType\":\"overlay\",\"appId\":\"com.palm.app.settings\",\"requiredMemory\":0,\"containerBasedApp\":false,\"appType\":\"web\",\"relaunch\":false,\"windowGroup\":false}",
        "{\"defaultWindowType\":\"card\",\"appId\":\"com.webos.app.browser\",\"requiredMemory\":160,\"containerBasedApp\":false,\"appType\":\"native_builtin\",\"relaunch\":false,\"windowGroup\":false}",
        "{\"defaultWindowType\":\"card\",\"appId\":\"youtube.leanback.v4\",\"requiredMemory\":0,\"containerBasedApp\":false,\"appType\":\"native_mvpd\",\"relaunch\":false,\"windowGroup\":false,\"preload\":\"semi-full\"}",
        "{\"defaultWindowType\":\"card\",\"appId\":\"com.webos.app.livetv\",\"requiredMemory\":0,\"containerBasedApp\":false,\"appType\":\"native\",\"relaunch\":false,\"windowGroup\":true,\"windowGroupOwner\":true}",
    };
    // No helper: answered at once all the same.
    for (size_t i = 0; i < 4; i++) CHECK(!strcmp(call("setRequiredMemory", launches[i], "com.webos.applicationManager"), "{\"returnValue\":true}"));
    // A helper: answered, then told.
    int sv[2];
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0);
    bridge_fd = sv[0];
    CHECK(!strcmp(call("setRequiredMemory", launches[1], "com.webos.applicationManager"), "{\"returnValue\":true}"));
    char buf[2048] = "";
    ssize_t n = read(sv[1], buf, sizeof buf - 1);
    CHECK(n > 0);
    buf[n > 0 ? n : 0] = 0;
    CHECK(strstr(buf, "{\"kind\":\"launch\",\"sender\":\"com.webos.applicationManager\",\"payload\":{") == buf);
    CHECK(strstr(buf, "\"requiredMemory\":160") != NULL);
    // A helper that stops reading: the socket fills, the answer still comes, and the link is marked broken.
    int small = 4096;
    setsockopt(sv[0], SOL_SOCKET, SO_SNDBUF, &small, sizeof small);
    for (int i = 0; i < 2000 && !bridge_broken; i++) CHECK(!strcmp(call("setRequiredMemory", launches[0], "com.webos.applicationManager"), "{\"returnValue\":true}"));
    CHECK(bridge_broken);
    close(sv[0]); close(sv[1]);
    bridge_fd = -1;
    bridge_broken = false;

    // WAM every 5 s, no helper state: the kernel's numbers (218002 pages = 851 MB, normal).
    CHECK(!strcmp(call("getCurrentMemState", "{}", "com.palm.webappmanager"), "{\"returnValue\":true,\"currentLevel\":\"normal\",\"usable\":851,\"prevLevel\":\"normal\"}"));
    // The helper's level wins while it is fresh.
    char state[] = "{\"kind\":\"state\",\"level\":\"medium\",\"prevLevel\":\"normal\",\"limit\":false,\"swapUsed\":12,\"units\":[{\"appId\":\"netflix\",\"pid\":19386}],\"policy\":{\"launchThreshold\":120}}";
    handle_line(state);
    CHECK(!strcmp(call("getCurrentMemState", "{}", "com.palm.webappmanager"), "{\"returnValue\":true,\"currentLevel\":\"medium\",\"usable\":851,\"prevLevel\":\"normal\"}"));
    hs.at -= STATE_FRESH_S + 1;
    CHECK(strstr(call("getCurrentMemState", "{}", "com.palm.webappmanager"), "\"currentLevel\":\"normal\"") != NULL);
    hs.at = mono_now();

    // preloadmanager: allowed, then blocked while an EFS app is in front.
    CHECK(!strcmp(call("getCustomThreshold", "{\"appId\":\"com.webos.app.livetv\",\"threshold\":\"100\",\"subscribe\":true}", "com.webos.service.preloadmanager"),
                  "{\"subscribed\":true,\"returnValue\":true,\"threshold\":\"100\",\"current\":\"851\"}"));
    CHECK(!strcmp(subs[nsubs - 1], "getCustomThreshold"));
    char limited[] = "{\"kind\":\"state\",\"level\":\"normal\",\"prevLevel\":\"normal\",\"limit\":true}";
    handle_line(limited);
    CHECK(!strcmp(call("getCustomThreshold", "{\"appId\":\"com.webos.app.hdmi2\",\"threshold\":\"40\",\"subscribe\":true}", "com.webos.service.preloadmanager"),
                  "{\"errorCode\":9,\"returnValue\":false,\"errorText\":\"get custom threshold blocked\"}"));

    // The browser and emmcd: both off.
    CHECK(!strcmp(call("getEFS", "{\"subscribe\":true}", "com.webos.app.browser"),
                  "{\"subscribed\":true,\"returnValue\":true,\"EfsInfo\":{\"activate\":false,\"type\":\"dynamic\",\"target\":false},\"HibernationInfo\":{\"activate\":false}}"));

    // getCloseAppId: refused unsubscribed (LG's errorCode 4), kept by app type, pushed before a close.
    CHECK(!strcmp(call("getCloseAppId", "{}", "com.webos.app.appcasting"), "{\"returnValue\":false,\"errorCode\":4,\"errorText\":\"Invalid Parameters Error\"}"));
    CHECK(!strcmp(call("getCloseAppId", "{\"appType\":\"web\",\"subscribe\":true}", "com.palm.webappmanager"), "{\"returnValue\":true,\"subscribed\":true}"));
    CHECK(!strcmp(subs[nsubs - 1], "close:web"));
    char closing[] = "{\"kind\":\"closing\",\"id\":\"com.webos.app.lgchannels\",\"appType\":\"web\"}";
    handle_line(closing);
    CHECK(npushes == 2);
    CHECK(!strcmp(pushes[0], "close:web {\"returnValue\":true,\"id\":\"com.webos.app.lgchannels\"}"));
    CHECK(!strcmp(pushes[1], "close: {\"returnValue\":true,\"id\":\"com.webos.app.lgchannels\"}"));

    // libSDL2 and diagnostics.
    handle_line(state);
    CHECK(!strcmp(call("getUnitList", "{\"procList\":200}", "com.webos.app.x"),
                  "{\"returnValue\":true,\"usable_memory\":851,\"swapUsed\":12,\"unitList\":[{\"appId\":\"netflix\",\"pid\":19386}]}"));
    CHECK(!strcmp(call("getPolicy", "{}", "x"), "{\"returnValue\":true,\"policy\":{\"launchThreshold\":120}}"));
    CHECK(!strcmp(call("startMemNotifier", "{}", "luna-send"), "{\"returnValue\":true}"));
    CHECK(strstr(call("sendLowMemPopupTest", "{}", "luna-send"), "not supported") != NULL);

    // The helper's signals and calls go out under this name.
    char sig[] = "{\"kind\":\"signal\",\"name\":\"thresholdChanged\",\"payload\":{\"previous\":\"normal\",\"current\":\"medium\"}}";
    char bad[] = "{\"kind\":\"signal\",\"name\":\"other\",\"payload\":{}}";
    char out[] = "{\"kind\":\"outbound\",\"uri\":\"luna://com.webos.service.preloadmanager/setPreloadStatus\",\"payload\":{\"status\":\"limit\"}}";
    nsent = 0;
    handle_line(sig);
    handle_line(bad);
    handle_line(out);
    CHECK(nsent == 2);
    CHECK(!strcmp(sent[0], "signal luna://com.webos.memorymanager/com/webos/memory/thresholdChanged {\"previous\":\"normal\",\"current\":\"medium\"}"));
    CHECK(!strcmp(sent[1], "call luna://com.webos.service.preloadmanager/setPreloadStatus {\"status\":\"limit\"}"));
}

static void test_supervision(void) {
    reset_state();
    started_at = 100;
    ever_connected = false;
    CHECK(tick_verdict(110) == NULL);
    CHECK(tick_verdict(121) != NULL && strstr(tick_verdict(121), "did not connect"));
    int sv[2];
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0);
    bridge_attach(sv[0]);
    last_ping = 200;
    CHECK(tick_verdict(205) == NULL && ping_seq == 1);
    pong_seq = 1;
    CHECK(tick_verdict(210) == NULL && ping_seq == 2);
    CHECK(tick_verdict(215) == NULL && ping_seq == 3);
    // Two pings unanswered.
    CHECK(tick_verdict(220) != NULL && strstr(tick_verdict(220), "two heartbeats"));
    close(sv[1]);
    bridge_closed();
    CHECK(tick_verdict(gone_at + 5) == NULL);
    CHECK(tick_verdict(gone_at + 11) != NULL && strstr(tick_verdict(gone_at + 11), "gone for 10 s"));
    // A helper that said goodbye gets 60 s.
    handle_line("{\"kind\":\"bye\"}");
    CHECK(tick_verdict(gone_at + 11) == NULL);
    CHECK(tick_verdict(gone_at + 61) != NULL && strstr(tick_verdict(gone_at + 61), "within 60 s"));
    helper_bye = false;
    switched_off = 1;
    CHECK(!strcmp(tick_verdict(0), "switched off"));
    switched_off = 0;
}

static void test_decide(void) {
    reset_state();
    CHECK(decide() != NULL && strstr(decide(), "switched off"));
    put("state/enabled", "");
    CHECK(decide() == NULL);
    put("uptime", "1000.0 900.0\n");
    // Two of this boot in the window, one older, one of another boot: runs.
    put("state/failures", "boot-a 100 old\nboot-b 950 other boot\nboot-a 500 x\nboot-a 990 y\n");
    CHECK(recent_failures() == 2 && decide() == NULL);
    add_failure("z");
    CHECK(recent_failures() == 3 && decide() != NULL && strstr(decide(), "three failures"));
    // Still refused once the window has passed, until the boot changes.
    put("uptime", "5000.0 900.0\n");
    CHECK(recent_failures() == 0 && decide() != NULL);
    put("state/gave-up", "boot-old\n");
    CHECK(decide() == NULL);
    // The file keeps eight lines at most.
    for (int i = 0; i < 10; i++) add_failure("more");
    FILE *f = fopen(in_state("failures"), "r");
    int lines = 0;
    char line[512];
    while (fgets(line, sizeof line, f)) lines++;
    fclose(f);
    CHECK(lines == 8);
}

static void test_unclean(void) {
    reset_state();
    put("uptime", "1000.0 900.0\n");
    put("state/running", "boot-other\n");
    consume_unclean();
    CHECK(recent_failures() == 0 && !file_exists(in_state("running")));
    put("state/running", "boot-a\n");
    consume_unclean();
    CHECK(recent_failures() == 1);
    CHECK(file_exists(in_state("crash/pending")));
}

// The fallback in a child: the record is made, the marker goes and LG's binary (a script here) is exec'd with the same arguments.
static void test_fallback(void) {
    reset_state();
    put("uptime", "1000.0 900.0\n");
    put("state/running", "boot-a\n");
    char lg[300], marker[300], cmd[700];
    snprintf(lg, sizeof lg, "%s/memchute", tmp);
    snprintf(marker, sizeof marker, "%s/execd", tmp);
    snprintf(cmd, sizeof cmd, "printf '#!/bin/sh\\necho \"$@\" > %s\\n' > %s && chmod +x %s", marker, lg, lg);
    CHECK(system(cmd) == 0);
    remember_call("setRequiredMemory", "com.webos.applicationManager");
    pid_t pid = fork();
    if (pid == 0) {
        static char *args[] = { "memchute", "--from-systemd", NULL };
        saved_argv = args;
        lg_binary = lg;
        fallback("the helper missed two heartbeats", true);
    }
    int status;
    waitpid(pid, &status, 0);
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0);
    char got[100];
    CHECK(read_first_line(marker, got, sizeof got) && !strcmp(got, "--from-systemd"));
    CHECK(!file_exists(in_state("running")));
    CHECK(recent_failures() == 1);
    char pending[100], path[400], reason[400];
    CHECK(read_first_line(in_state("crash/pending"), pending, sizeof pending));
    snprintf(path, sizeof path, "%s/crash/%s/reason.txt", state_dir, pending);
    FILE *f = fopen(path, "r");
    CHECK(f != NULL);
    if (f) { size_t n = fread(reason, 1, sizeof reason - 1, f); reason[n] = 0; fclose(f); }
    CHECK(strstr(reason, "last error: the helper missed two heartbeats\n") != NULL);
    snprintf(path, sizeof path, "%s/crash/%s/calls.txt", state_dir, pending);
    CHECK(read_first_line(path, got, sizeof got) && strstr(got, "setRequiredMemory from com.webos.applicationManager"));
    // A deliberate hand-back counts nothing.
    unlink(marker);
    pid = fork();
    if (pid == 0) {
        static char *args[] = { "memchute", NULL };
        saved_argv = args;
        lg_binary = lg;
        fallback("switched off", false);
    }
    waitpid(pid, &status, 0);
    CHECK(file_exists(marker) && recent_failures() == 1);
}

static void test_records_rotate(void) {
    reset_state();
    put("uptime", "1000.0 900.0\n");
    for (int i = 0; i < CRASH_KEEP + 3; i++) crash_record("again");
    DIR *d = opendir(crash_dir());
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d))) if (e->d_name[0] != '.' && strcmp(e->d_name, "pending")) n++;
    closedir(d);
    CHECK(n == CRASH_KEEP);
}

int main(void) {
    snprintf(tmp, sizeof tmp, "/tmp/lumatv-mm-test-XXXXXX");
    if (!mkdtemp(tmp)) return 1;
    static char sd[300], up[300], ut[300], bi[300], ld[300];
    snprintf(sd, sizeof sd, "%s/state", tmp); state_dir = sd;
    snprintf(up, sizeof up, "%s/usable", tmp); usable_path = up;
    snprintf(ut, sizeof ut, "%s/uptime", tmp); uptime_path = ut;
    snprintf(bi, sizeof bi, "%s/boot", tmp); boot_id_path = bi;
    snprintf(ld, sizeof ld, "%s/logs", tmp); log_directory = ld;
    put("boot", "boot-a\n");
    put("uptime", "1000.0 900.0\n");
    bus = &fake_handle;
    test_levels();
    test_callers();
    test_supervision();
    test_decide();
    test_unclean();
    test_fallback();
    test_records_rotate();
    char cmd[300];
    snprintf(cmd, sizeof cmd, "rm -rf %s", tmp);
    if (system(cmd) != 0) failed++;
    if (failed) { fprintf(stderr, "%d check(s) failed\n", failed); return 1; }
    printf("lumatv-mm tests: ok\n");
    return 0;
}
