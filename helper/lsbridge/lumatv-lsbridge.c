// lumatv-lsbridge: one luna-service2 client that holds every luna subscription the helper follows, in place of one
// `luna-send -n 1000000` process each (0.4 MB of private memory apiece on the set).
//
// The helper starts it as a child and talks to it over stdin and stdout, one line each way. No JSON is parsed here.
//   helper -> bridge   S <id> <uri> <payload>   call <uri> with <payload> as a subscription; <id> is the helper's number for it
//                      C <id>                    end that call
//                      Q                         exit
//   bridge -> helper   H 1                       registered on the bus and ready (the version of this protocol)
//                      R <id> <payload>          one reply, the text luna-send prints
//                      E <id> <text>             the call cannot be made: nothing more follows for <id>
//                      X <text>                  it cannot run: it exits after this line
// A reply is one line: the newlines in it are spaces (JSON has none inside a string). Stdin ending ends the bridge, so one cannot
// outlive the helper, however the helper ends: the bus drops the calls with the connection. A call the service stops answering is
// not ended here either, as with luna-send: the service's last reply says so (`returnValue:false`) and the helper ends the call.
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

// ---- luna-service2 (public API, webosose/luna-service2 lunaservice.h) ----
typedef struct LSHandle LSHandle;
typedef struct LSMessage LSMessage;
typedef unsigned long LSMessageToken;
typedef struct LSError { int error_code; char *message; const char *file; int line; const char *func; void *padding; unsigned long magic; } LSError;
typedef bool (*LSFilterFunc)(LSHandle *sh, LSMessage *reply, void *ctx);
typedef void (*LSDisconnectHandler)(LSHandle *sh, void *ctx);
typedef struct GMainLoop GMainLoop;
bool LSErrorInit(LSError *e);
void LSErrorFree(LSError *e);
bool LSRegister(const char *name, LSHandle **sh, LSError *e);
bool LSUnregister(LSHandle *sh, LSError *e);
bool LSGmainAttach(LSHandle *sh, GMainLoop *loop, LSError *e);
bool LSCall(LSHandle *sh, const char *uri, const char *payload, LSFilterFunc cb, void *ctx, LSMessageToken *token, LSError *e);
bool LSCallCancel(LSHandle *sh, LSMessageToken token, LSError *e);
const char *LSMessageGetPayload(LSMessage *m);
bool LSHandleSetDisconnectHandler(LSHandle *sh, LSDisconnectHandler handler, void *ctx, LSError *e);

// ---- glib (only what the loop needs) ----
typedef struct GIOChannel GIOChannel;
typedef int gboolean;
typedef void *gpointer;
typedef gboolean (*GIOFunc)(GIOChannel *, int condition, gpointer);
enum { G_IO_IN = 1, G_IO_ERR = 8, G_IO_HUP = 16 };
GMainLoop *g_main_loop_new(void *context, gboolean running);
void g_main_loop_run(GMainLoop *loop);
void g_main_loop_quit(GMainLoop *loop);
GIOChannel *g_io_channel_unix_new(int fd);
unsigned g_io_add_watch(GIOChannel *ch, int condition, GIOFunc fn, gpointer data);

#define MAX_SUBS 64
// The longest line the helper may send: a uri and a payload are a few hundred bytes.
#define MAX_LINE 16384

struct sub { unsigned long id; LSMessageToken token; }; // id 0: the slot is free
static struct sub subs[MAX_SUBS];
static LSHandle *bus;
static GMainLoop *loop;
static int out_fd = 1;
static int exit_code;
static char inbuf[MAX_LINE + 1];
static size_t inlen;

static void put(const char *s, size_t n) {
    while (n) {
        ssize_t w = write(out_fd, s, n);
        if (w < 0) {
            if (errno == EINTR) continue;
            _exit(0); // the helper has gone: nothing is left to tell
        }
        s += w;
        n -= (size_t)w;
    }
}

static void emit(char kind, unsigned long id, const char *text) {
    size_t len = strlen(text);
    char *line = malloc(len + 32);
    if (!line) _exit(3);
    int n = id ? snprintf(line, 32, "%c %lu ", kind, id) : snprintf(line, 32, "%c ", kind);
    for (size_t i = 0; i < len; i++) line[n + i] = (text[i] == '\n' || text[i] == '\r') ? ' ' : text[i];
    line[n + len] = '\n';
    put(line, (size_t)n + len + 1);
    free(line);
}

static void quit(int code) {
    exit_code = code;
    if (loop) g_main_loop_quit(loop);
}

static bool on_reply(LSHandle *sh, LSMessage *msg, void *ctx) {
    (void)sh;
    struct sub *s = ctx;
    if (!s->id) return true; // ended meanwhile
    const char *payload = LSMessageGetPayload(msg);
    emit('R', s->id, payload && *payload ? payload : "{}");
    return true;
}

static void start(unsigned long id, const char *uri, const char *payload) {
    struct sub *free_slot = NULL;
    for (int i = 0; i < MAX_SUBS; i++) {
        if (subs[i].id == id) { emit('E', id, "that id is in use"); return; }
        if (!subs[i].id && !free_slot) free_slot = &subs[i];
    }
    if (!free_slot) { emit('E', id, "too many subscriptions"); return; }
    free_slot->id = id;
    LSError e;
    LSErrorInit(&e);
    if (!LSCall(bus, uri, payload, on_reply, free_slot, &free_slot->token, &e)) {
        emit('E', id, e.message ? e.message : "the call failed");
        free_slot->id = 0;
    }
    LSErrorFree(&e);
}

static void cancel(unsigned long id) {
    for (int i = 0; i < MAX_SUBS; i++) {
        if (subs[i].id != id) continue;
        LSError e;
        LSErrorInit(&e);
        LSCallCancel(bus, subs[i].token, &e);
        LSErrorFree(&e);
        subs[i].id = 0;
    }
}

// One line of the helper's, without its newline. The line is cut up in place.
static void handle_line(char *line) {
    if (line[0] == 'Q' && !line[1]) { quit(0); return; }
    if ((line[0] != 'S' && line[0] != 'C') || line[1] != ' ') return;
    char *end;
    unsigned long id = strtoul(line + 2, &end, 10);
    if (end == line + 2 || !id) return;
    if (line[0] == 'C') { cancel(id); return; }
    if (*end != ' ') { emit('E', id, "no uri"); return; }
    char *uri = end + 1;
    char *space = strchr(uri, ' ');
    const char *payload = "{}";
    if (space) { *space = 0; payload = space + 1; }
    if (!*uri) { emit('E', id, "no uri"); return; }
    start(id, uri, payload);
}

// Takes bytes read from stdin; false when a line is longer than the helper ever sends.
static bool consume(const char *data, size_t n) {
    while (n) {
        size_t room = MAX_LINE - inlen;
        size_t take = n < room ? n : room;
        memcpy(inbuf + inlen, data, take);
        inlen += take;
        data += take;
        n -= take;
        char *start_of = inbuf;
        char *nl;
        while ((nl = memchr(start_of, '\n', (size_t)(inbuf + inlen - start_of)))) {
            *nl = 0;
            if (nl > start_of && nl[-1] == '\r') nl[-1] = 0;
            handle_line(start_of);
            start_of = nl + 1;
        }
        inlen = (size_t)(inbuf + inlen - start_of);
        memmove(inbuf, start_of, inlen);
        if (inlen == MAX_LINE) return false;
    }
    return true;
}

static gboolean on_stdin(GIOChannel *ch, int condition, gpointer data) {
    (void)ch; (void)condition; (void)data;
    for (;;) {
        char chunk[4096];
        ssize_t n = read(0, chunk, sizeof chunk);
        if (n > 0) {
            if (!consume(chunk, (size_t)n)) { emit('X', 0, "a line is too long"); quit(3); return 0; }
            continue;
        }
        if (n < 0 && errno == EINTR) continue;
        if (n < 0 && errno == EAGAIN) return 1;
        quit(0); // the helper ended (or stdin failed): the bus drops the calls with the connection
        return 0;
    }
}

static void on_disconnect(LSHandle *sh, void *ctx) {
    (void)sh; (void)ctx;
    emit('X', 0, "disconnected from the bus");
    quit(4);
}

#ifndef LUMATV_LSBRIDGE_TEST
int main(int argc, char **argv) {
    // `--name NAME` registers under NAME instead of anonymously: for finding what the hub's role and permission files allow.
    const char *name = argc == 3 && !strcmp(argv[1], "--name") ? argv[2] : NULL;
    struct rlimit nocore = { 0, 0 };
    setrlimit(RLIMIT_CORE, &nocore); // a crash must not leave a dump on the set
    signal(SIGPIPE, SIG_IGN);
    LSError e;
    LSErrorInit(&e);
    if (!LSRegister(name, &bus, &e)) {
        char why[256];
        snprintf(why, sizeof why, "cannot register: %s", e.message ? e.message : "?");
        emit('X', 0, why);
        return 2;
    }
    loop = g_main_loop_new(NULL, 0);
    if (!LSGmainAttach(bus, loop, &e)) {
        char why[256];
        snprintf(why, sizeof why, "cannot attach to the loop: %s", e.message ? e.message : "?");
        emit('X', 0, why);
        return 2;
    }
    LSHandleSetDisconnectHandler(bus, on_disconnect, NULL, &e);
    LSErrorFree(&e);
    int flags = fcntl(0, F_GETFL);
    if (flags >= 0) fcntl(0, F_SETFL, flags | O_NONBLOCK);
    g_io_add_watch(g_io_channel_unix_new(0), G_IO_IN | G_IO_HUP | G_IO_ERR, on_stdin, NULL);
    emit('H', 0, "1");
    g_main_loop_run(loop);
    return exit_code;
}
#else
#include "test.c"
#endif
