// Host tests for lumatv-lsbridge.c (built with -DLUMATV_LSBRIDGE_TEST, which includes this file in place of main): a fake
// libluna-service2 records the calls, and the test answers them through the callbacks the bridge handed over.
#include <poll.h>

struct LSHandle { int unused; };
struct LSMessage { const char *payload; };
static struct LSHandle fake_handle;
static struct { char uri[200]; char payload[400]; LSFilterFunc cb; void *ctx; } calls[MAX_SUBS + 4];
static int ncalls;
static LSMessageToken cancelled[MAX_SUBS + 4];
static int ncancelled;
static bool refuse_calls;
static int quits;

bool LSErrorInit(LSError *e) { memset(e, 0, sizeof *e); return true; }
void LSErrorFree(LSError *e) { (void)e; }
bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e) {
    (void)sh;
    if (refuse_calls) { e->message = "refused"; return false; }
    snprintf(calls[ncalls].uri, sizeof calls[ncalls].uri, "%s", uri);
    snprintf(calls[ncalls].payload, sizeof calls[ncalls].payload, "%s", payload);
    calls[ncalls].cb = cb;
    calls[ncalls].ctx = ctx;
    *token = 100 + (LSMessageToken)ncalls;
    ncalls++;
    return true;
}
bool LSCallCancel(LSHandle *sh, LSMessageToken token, LSError *e) { (void)sh; (void)e; cancelled[ncancelled++] = token; return true; }
const char *LSMessageGetPayload(LSMessage *m) { return m->payload; }
void g_main_loop_quit(GMainLoop *l) { (void)l; quits++; }
static int watched[8];
static int nwatched;
static int watched_condition;
static GIOFunc watched_fn;
GIOChannel *g_io_channel_unix_new(int fd) { return (GIOChannel *)(long)(fd + 1); }
unsigned g_io_add_watch(GIOChannel *ch, int condition, GIOFunc fn, gpointer data) {
    (void)data;
    watched[nwatched++] = (int)(long)ch - 1;
    watched_condition = condition;
    watched_fn = fn;
    return 1;
}

static int failed;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #c); failed++; } } while (0)

static char out_path[] = "/tmp/lumatv-lsbridge-test-XXXXXX";

static void reset(void) {
    memset(subs, 0, sizeof subs);
    memset(calls, 0, sizeof calls);
    ncalls = ncancelled = quits = nwatched = 0;
    refuse_calls = false;
    inlen = 0;
    exit_code = 0;
    if (ftruncate(out_fd, 0) != 0) abort();
    lseek(out_fd, 0, SEEK_SET);
}

// What the bridge wrote since the last reset.
static const char *output(void) {
    static char text[4096];
    ssize_t n = pread(out_fd, text, sizeof text - 1, 0);
    text[n < 0 ? 0 : n] = 0;
    return text;
}

static void line(const char *text) { CHECK(consume(text, strlen(text))); }

static void answer(int call, const char *payload) {
    LSMessage m = { payload };
    CHECK(calls[call].cb(&fake_handle, &m, calls[call].ctx));
}

int main(void) {
    int fd = mkstemp(out_path);
    if (fd < 0) return 2;
    unlink(out_path);
    out_fd = fd;
    bus = &fake_handle;
    loop = (GMainLoop *)&fake_handle;

    // A subscription is called as the helper wrote it, and each reply comes back under its id on one line.
    reset();
    line("S 7 luna://com.webos.service.tvpower/power/getPowerState {\"subscribe\":true}\n");
    CHECK(ncalls == 1);
    CHECK(!strcmp(calls[0].uri, "luna://com.webos.service.tvpower/power/getPowerState"));
    CHECK(!strcmp(calls[0].payload, "{\"subscribe\":true}"));
    CHECK(!strcmp(output(), ""));
    answer(0, "{\"state\":\"Active\",\"returnValue\":true}");
    answer(0, "{\"a\":\n1}");
    answer(0, "");
    CHECK(!strcmp(output(), "R 7 {\"state\":\"Active\",\"returnValue\":true}\nR 7 {\"a\": 1}\nR 7 {}\n"));

    // Two subscriptions do not mix, and ending one leaves the other.
    reset();
    line("S 1 luna://a/x {}\nS 2 luna://b/y {\"k\":\"v w\"}\n");
    CHECK(ncalls == 2);
    CHECK(!strcmp(calls[1].payload, "{\"k\":\"v w\"}"));
    line("C 1\n");
    CHECK(ncancelled == 1 && cancelled[0] == 100);
    answer(0, "{\"late\":true}"); // a reply that was already on its way when the call ended is dropped
    answer(1, "{\"two\":true}");
    CHECK(!strcmp(output(), "R 2 {\"two\":true}\n"));
    line("C 1\nC 99\n"); // ending an ended or an unknown call does nothing
    CHECK(ncancelled == 1);

    // A line may arrive in pieces and with CRLF; lines that are not the helper's are skipped.
    reset();
    line("S 3 luna://c/");
    line("d {}\r\nhello\nS x luna://e/f {}\nX 1 y\nS 4 luna://g/h {}\n");
    CHECK(ncalls == 2);
    CHECK(!strcmp(calls[0].uri, "luna://c/d"));
    CHECK(!strcmp(calls[0].payload, "{}"));
    CHECK(!strcmp(calls[1].uri, "luna://g/h"));
    CHECK(!strcmp(output(), ""));

    // A payload that was left off is an empty object; a call with no uri is refused under its id.
    reset();
    line("S 5 luna://a/b\n");
    CHECK(ncalls == 1 && !strcmp(calls[0].payload, "{}"));
    line("S 6\nS 8 \n");
    CHECK(ncalls == 1);
    CHECK(!strcmp(output(), "E 6 no uri\nE 8 no uri\n"));

    // A call the bus refuses is reported and frees its slot; a reused id and a full table are reported too.
    reset();
    refuse_calls = true;
    line("S 9 luna://a/b {}\n");
    CHECK(!strcmp(output(), "E 9 refused\n"));
    refuse_calls = false;
    line("S 9 luna://a/b {}\nS 9 luna://a/b {}\n");
    CHECK(ncalls == 1);
    CHECK(strstr(output(), "E 9 that id is in use\n") != NULL);
    reset();
    for (unsigned long id = 1; id <= MAX_SUBS; id++) {
        char text[64];
        snprintf(text, sizeof text, "S %lu luna://a/b {}\n", id);
        line(text);
    }
    CHECK(ncalls == MAX_SUBS);
    line("S 500 luna://a/b {}\n");
    CHECK(!strcmp(output(), "E 500 too many subscriptions\n"));
    line("C 3\nS 501 luna://a/b {}\n"); // a freed slot is taken again
    CHECK(ncalls == MAX_SUBS + 1);
    CHECK(!strcmp(output(), "E 500 too many subscriptions\n"));

    // A line longer than the helper ever sends ends the bridge.
    reset();
    static char big[MAX_LINE + 10];
    memset(big, 'a', sizeof big);
    CHECK(!consume(big, sizeof big));

    // Q and the end of stdin end it.
    reset();
    line("Q\n");
    CHECK(quits == 1 && exit_code == 0);
    reset();
    int p[2];
    CHECK(pipe(p) == 0);
    int saved = dup(0);
    dup2(p[0], 0);
    fcntl(0, F_SETFL, fcntl(0, F_GETFL) | O_NONBLOCK);
    CHECK(write(p[1], "S 1 luna://a/b {}\n", 18) == 18);
    CHECK(on_stdin(NULL, G_IO_IN, NULL) == 1); // read what is there, then wait for more
    CHECK(ncalls == 1 && quits == 0);
    close(p[1]);
    CHECK(on_stdin(NULL, G_IO_HUP, NULL) == 0); // the helper has gone
    CHECK(quits == 1 && exit_code == 0);
    dup2(saved, 0);

    reset();
    emit('X', 0, "gone");
    CHECK(!strcmp(output(), "X gone\n"));

    // The bus hanging up ends the bridge. Of what is open once the library has registered, only the new stream sockets are watched.
    reset();
    int old_stream[2], bus_end[2], datagram[2], plain[2];
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, old_stream) == 0);
    fdmask before;
    open_fds(before);
    CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, bus_end) == 0);
    CHECK(socketpair(AF_UNIX, SOCK_DGRAM, 0, datagram) == 0);
    CHECK(pipe(plain) == 0);
    CHECK(watch_bus_sockets(before) == 2);
    CHECK(nwatched == 2 && watched[0] == bus_end[0] && watched[1] == bus_end[1]);
    CHECK(watched_condition == (G_IO_HUP | G_IO_ERR | G_IO_NVAL));
    struct pollfd pfd = { bus_end[0], 0, 0 };
    CHECK(poll(&pfd, 1, 0) == 0); // the other end is open: nothing to report
    close(bus_end[1]);
    CHECK(poll(&pfd, 1, 0) == 1 && (pfd.revents & POLLHUP)); // a closed peer is a hang-up, which the watch waits for
    CHECK(watched_fn(NULL, G_IO_HUP, NULL) == 0);
    CHECK(!strcmp(output(), "X disconnected from the bus\n") && quits == 1 && exit_code == 4);

    if (failed) { fprintf(stderr, "%d check(s) failed\n", failed); return 1; }
    puts("lumatv-lsbridge host tests passed");
    return 0;
}
