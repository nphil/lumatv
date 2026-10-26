/* A stand-in for LG's libNDL_directmedia.so.1 (API version 2), for the player's host tests.
 *
 * It does what the real one demands and refuses what the real one punishes, and writes every call to $FAKE_NDL_LOG so a test can read
 * the order: the demands are the ones jellyfin-webos-native's docs/ndl.md and webosbrew/samples found on hardware (DL_Initialize first,
 * the window id before Init, the foreground state before pictures count). A call that breaks one is logged as VIOLATION.
 *
 * FAKE_NDL_FAIL=dlinit|init|load|state     that step fails
 * FAKE_NDL_NO_PLAYING=1                    the pipeline loads but never says it is playing
 * FAKE_NDL_ERROR_AFTER=N                   an error event after N units
 * FAKE_NDL_FULL_EVERY=N                    every Nth NDL_DirectVideoPlay says the buffer is full
 * FAKE_NDL_QUEUE=N                         the render queue's length
 * FAKE_NDL_LOAD_MS=N                       the load takes this long (default 20)
 * FAKE_NDL_VERBOSE=1                       every unit is logged (size, pts), not only the first few
 * FAKE_NDL_UNLOAD_MS=N                     Unload takes this long (a teardown that is slow: the window must not wait for it)
 */
#define _GNU_SOURCE
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef void (*load_cb)(int, long long, const char *);

struct data_info {
    struct { int width, height, type, unknown1; } video;
    unsigned char audio[32];
};

static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;
static FILE *logf;
static int dl_inited, window_set, inited, loaded, foreground;
static char window_id[64], err[160] = "";
static load_cb cb;
static int played, shown, dropped, full_n;
static long long last_pts = -1;
static int load_w, load_h, load_type;

static long long ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void say(const char *fmt, ...) {
    pthread_mutex_lock(&mu);
    if (!logf) {
        const char *p = getenv("FAKE_NDL_LOG");
        logf = p ? fopen(p, "a") : stderr;
        if (!logf) logf = stderr;
    }
    va_list ap;
    va_start(ap, fmt);
    fprintf(logf, "T %lld ", ms());
    vfprintf(logf, fmt, ap);
    fputc('\n', logf);
    fflush(logf);
    va_end(ap);
    pthread_mutex_unlock(&mu);
}

static int env_is(const char *k, const char *v) {
    const char *e = getenv(k);
    return e && !strcmp(e, v);
}
static int env_int(const char *k, int def) {
    const char *e = getenv(k);
    return e ? atoi(e) : def;
}

static int bad(const char *what) {
    say("VIOLATION %s", what);
    snprintf(err, sizeof err, "%s", what);
    return -1;
}

int NDL_DirectMedia_DL_Initialize(void) {
    say("DL_Initialize");
    if (env_is("FAKE_NDL_FAIL", "dlinit")) return 0;
    dl_inited = 1;
    return 1;
}
void NDL_DirectMedia_DL_Finalize(void) { say("DL_Finalize"); }

const char *NDL_DirectMediaGetError(void) { return err; }

int NDL_DirectMediaSetWindowId(const char *id) {
    if (!dl_inited) return bad("SetWindowId before DL_Initialize");
    say("SetWindowId %s", id ? id : "(null)");
    snprintf(window_id, sizeof window_id, "%s", id ? id : "");
    window_set = 1;
    return 0;
}

/* v2 declares NDL_DirectMediaInit(const char *app_id); the player passes a second argument, which this ignores as the real one may */
int NDL_DirectMediaInit(const char *app_id) {
    if (!dl_inited) return bad("Init before DL_Initialize");
    if (!window_set) return bad("Init before SetWindowId (the library would make an SDL window of its own)");
    say("Init %s xdg=%s", app_id ? app_id : "(null)", getenv("XDG_RUNTIME_DIR") ? getenv("XDG_RUNTIME_DIR") : "-");     /* the real library's AV connector needs it */
    if (env_is("FAKE_NDL_FAIL", "init")) {
        snprintf(err, sizeof err, "init refused");
        return -1;
    }
    inited = 1;
    return 0;
}

static void *events(void *arg) {
    (void)arg;
    usleep((useconds_t)env_int("FAKE_NDL_LOAD_MS", 20) * 1000);
    if (cb) cb(22, 1, "true");
    usleep(30000);
    if (!env_is("FAKE_NDL_NO_PLAYING", "1") && cb) cb(26, 1, "true");
    return NULL;
}

int NDL_DirectMediaLoad(struct data_info *info, load_cb callback) {
    if (!inited) return bad("Load before Init");
    load_w = info->video.width;
    load_h = info->video.height;
    load_type = info->video.type;
    say("Load %dx%d type %d", load_w, load_h, load_type);
    if (env_is("FAKE_NDL_FAIL", "load")) {
        snprintf(err, sizeof err, "load refused");
        return -1;
    }
    cb = callback;
    loaded = 1;
    pthread_t t;
    pthread_create(&t, NULL, events, NULL);
    pthread_detach(t);
    return 0;
}

int NDL_DirectMediaSetAppState(int state) {
    say("SetAppState %d", state);
    if (env_is("FAKE_NDL_FAIL", "state")) return -1;
    foreground = state == 0;
    return 0;
}

int NDL_DirectVideoSetArea(int l, int t, int w, int h) {
    say("SetArea %d %d %d %d", l, t, w, h);
    return 0;
}

int NDL_DirectVideoGetRenderBufferLength(int *len) {
    *len = env_int("FAKE_NDL_QUEUE", 1);
    return 0;
}

int NDL_DirectVideoPlay(void *buf, unsigned size, long long pts) {
    if (!loaded) return bad("Play before Load");
    int every = env_int("FAKE_NDL_FULL_EVERY", 0);
    if (every && ++full_n % every == 0) return -1;
    const unsigned char *b = buf;
    int annexb = size > 4 && b[0] == 0 && b[1] == 0 && (b[2] == 1 || (b[2] == 0 && b[3] == 1));
    if (!annexb) bad("a unit that does not start with an Annex B start code");
    if (pts <= last_pts) bad("a PTS that does not rise");
    last_pts = pts;
    played++;
    if (foreground) shown++;
    else {
        dropped++;
        if (dropped == 1) say("DROPPED units until SetAppState(FOREGROUND): the real library logs 'Video feed in the background state' and shows nothing");
    }
    /* the first few units and then one in sixty are logged: size, pts, the first bytes */
    if (played <= 3 || played % 60 == 0 || env_is("FAKE_NDL_VERBOSE", "1")) say("Play #%d size %u pts %lld head %02x%02x%02x%02x%02x%02x", played, size, pts, b[0], b[1], b[2], b[3], b[4], b[5]);
    int errAfter = env_int("FAKE_NDL_ERROR_AFTER", 0);
    if (errAfter && played == errAfter && cb) cb(18, 2331, "play error");
    return 0;
}

int NDL_DirectMediaUnload(void) {
    say("Unload");
    usleep((useconds_t)env_int("FAKE_NDL_UNLOAD_MS", 0) * 1000);
    loaded = 0;
    return 0;
}

int NDL_DirectMediaQuit(void) {
    say("Quit played=%d shown=%d dropped=%d", played, shown, dropped);
    inited = 0;
    window_set = 0;
    foreground = 0;
    last_pts = -1;
    played = shown = dropped = 0;
    return 0;
}
