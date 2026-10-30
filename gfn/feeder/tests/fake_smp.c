/* A stand-in for LG's libplayerAPIs.so.1 (the C++ class StarfishMediaAPIs, by its mangled names: gfn/feeder/src/gf_smp.c) for the host tests.
 * It logs every call to FAKE_SMP_LOG, keeps the load's payload, answers Feed with a C++11 std::string ("Ok", or "BufferFull" for the first
 * FAKE_SMP_FULL feeds), and delivers the events a real pipeline would (LOADCOMPLETED after the first picture, PLAYING after Play, UNLOADCOMPLETED)
 * from a thread of its own, the way the library's callbacks come. The object the feeder constructs is its own zeroed storage; this uses the first
 * bytes of it. FAKE_SMP_FAIL=load refuses the load.
 */
#define _GNU_SOURCE
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef void (*load_cb)(int type, long long num, const char *text, void *data);

struct smp {
    int loaded, playing, fed, full_left;
    load_cb cb;
    void *data;
    char media_id[32];
    char payload[4096];
};

static FILE *logf;

static void note(const char *fmt, ...) {
    if (!logf) {
        const char *p = getenv("FAKE_SMP_LOG");
        logf = fopen(p && *p ? p : "/dev/null", "a");
        if (!logf) return;
    }
    va_list ap;
    va_start(ap, fmt);
    vfprintf(logf, fmt, ap);
    va_end(ap);
    fputc('\n', logf);
    fflush(logf);
}

static void *events(void *arg) {
    struct smp *s = arg;
    usleep(20000);
    if (s->cb) s->cb(22, 0, "{\"state\":\"loaded\"}", s->data);
    usleep(5000);
    if (s->cb && s->playing) s->cb(26, 0, "", s->data);
    return NULL;
}

void _ZN17StarfishMediaAPIsC1EPKc(void *self, const char *uid) {
    struct smp *s = self;
    memset(s, 0, sizeof *s);
    snprintf(s->media_id, sizeof s->media_id, "%s", uid && *uid ? uid : "_fake0001");
    const char *full = getenv("FAKE_SMP_FULL");
    s->full_left = full ? atoi(full) : 0;
    note("StarfishMediaAPIs(%s)", uid ? uid : "null");
}

void _ZN17StarfishMediaAPIsD1Ev(void *self) {
    (void)self;
    note("~StarfishMediaAPIs");
}

int _ZN17StarfishMediaAPIs4LoadEPKcPFvixS1_PvES2_(void *self, const char *payload, load_cb cb, void *data) {
    struct smp *s = self;
    note("Load %s", payload);
    const char *fail = getenv("FAKE_SMP_FAIL");
    if (fail && !strcmp(fail, "load")) return 0;
    snprintf(s->payload, sizeof s->payload, "%s", payload);
    s->cb = cb;
    s->data = data;
    s->loaded = 1;
    return 1;
}

/* std::string by value: the hidden first argument is its storage, laid out as libstdc++'s C++11 string (pointer, length, 16-byte local buffer) */
struct cxx_string { char *ptr; size_t len; union { char local[16]; size_t cap; } u; };

void _ZN17StarfishMediaAPIs4FeedB5cxx11EPKc(struct cxx_string *out, void *self, const char *payload) {
    struct smp *s = self;
    const char *reply = "Ok";
    if (!s->loaded) reply = "NotLoaded";
    else if (s->full_left > 0) {
        s->full_left--;
        reply = "BufferFull";
    } else {
        if (!s->fed++) {
            pthread_t t;
            pthread_create(&t, NULL, events, s);
            pthread_detach(t);
        }
    }
    note("Feed %s -> %s", payload, reply);
    out->ptr = out->u.local;
    out->len = strlen(reply);
    memcpy(out->u.local, reply, out->len + 1);
}

int _ZN17StarfishMediaAPIs4PlayEv(void *self) {
    struct smp *s = self;
    note("Play");
    s->playing = 1;
    return 1;
}

int _ZN17StarfishMediaAPIs6UnloadEv(void *self) {
    struct smp *s = self;
    note("Unload");
    if (s->loaded && s->cb) s->cb(23, 0, "", s->data);
    s->loaded = s->playing = 0;
    return 1;
}

int _ZN17StarfishMediaAPIs16notifyForegroundEv(void *self) {
    (void)self;
    note("notifyForeground");
    return 1;
}

int _ZN17StarfishMediaAPIs16notifyBackgroundEv(void *self) {
    (void)self;
    note("notifyBackground");
    return 1;
}

int _ZN17StarfishMediaAPIs25getVideoRenderQueueLengthERi(void *self, int *len) {
    struct smp *s = self;
    *len = s->fed > 0 ? 1 : 0;
    return 1;
}

int _ZN17StarfishMediaAPIs18getAudioBufferSizeERiS0_(void *self, int *total, int *used) {
    (void)self;
    *total = 65536;
    *used = 0;
    return 1;
}

int _ZN17StarfishMediaAPIs5flushEv(void *self) {
    (void)self;
    note("flush");
    return 1;
}

int _ZN17StarfishMediaAPIs10setHdrInfoEPKc(void *self, const char *json) {
    (void)self;
    note("setHdrInfo %s", json);
    return 1;
}

const char *_ZN17StarfishMediaAPIs10getMediaIDEv(void *self) { return ((struct smp *)self)->media_id; }

void _ZdlPv(void *p) { free(p); }
