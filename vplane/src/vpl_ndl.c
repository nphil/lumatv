#define _GNU_SOURCE
#include "vpl_ndl.h"

#include <dlfcn.h>
#include <errno.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <unistd.h>

#include "vpl.h"
#include "vpl_vpc.h"

/* NDL_DIRECTMEDIA_DATA_INFO_T of NDL_directmedia_types.h: the video info, then a 32 byte union of which this player uses none (audio
 * zeroed means "no audio", which the library accepts: jellyfin-webos-native docs/ndl.md). */
struct data_info {
    struct { int width, height, type, unknown1; } video;
    unsigned char audio[32];
};

typedef void (*load_cb)(int, long long, const char *);
typedef void (*released_cb)(const char *);

#define EVQ 64

struct vpl_ndl {
    void *lib;
    int (*dl_init)(void);
    void (*dl_fin)(void);
    int (*set_window)(const char *);
    int (*init)(const char *, released_cb);       /* v2 declares only the id; a second argument in r1 is ignored by a callee that does not read it */
    int (*load)(struct data_info *, load_cb);
    int (*set_state)(int);
    int (*set_area)(int, int, int, int);
    int (*play)(void *, unsigned, long long);
    int (*queued)(int *);
    int (*unload)(void);
    int (*quit)(void);
    const char *(*get_error)(void);
    int started, loaded;

    int lock;                       /* a spin lock: the callbacks run on the library's threads, and a plain libc built without libpthread would make mutexes no-ops */
    struct { int type; long long num; char text[160]; } q[EVQ];
    int head, count;
    int efd;
};

static vpl_ndl *g_ndl;     /* NDL_DirectMediaLoad's callback has no user pointer */

static void lock(vpl_ndl *n) {
    while (__atomic_exchange_n(&n->lock, 1, __ATOMIC_ACQUIRE)) sched_yield();
}
static void unlock(vpl_ndl *n) { __atomic_store_n(&n->lock, 0, __ATOMIC_RELEASE); }

static void on_released(const char *type) { vpl_say("ndl: resource released by policy: %s", type ? type : "?"); }

static void on_event(int type, long long num, const char *text) {
    vpl_ndl *n = g_ndl;
    if (!n) return;
    lock(n);
    if (n->count < EVQ) {
        int at = (n->head + n->count) % EVQ;
        n->q[at].type = type;
        n->q[at].num = num;
        snprintf(n->q[at].text, sizeof n->q[at].text, "%s", text ? text : "");
        n->count++;
    }
    unlock(n);
    uint64_t one = 1;
    if (write(n->efd, &one, sizeof one) < 0) { /* the queue is already non-empty; a full eventfd is a wake-up already */ }
}

static void *sym(void *lib, const char *name) { return dlsym(lib, name); }

int vpl_ndl_open_lib(vpl_ndl **out, const char *lib, char *err, size_t cap) {
    vpl_ndl *n = calloc(1, sizeof *n);
    if (!n) {
        snprintf(err, cap, "out of memory");
        return -1;
    }
    n->efd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    const char *names[3] = {lib, "libNDL_directmedia.so.1", "libNDL_directmedia.so"};
    for (int i = (lib && *lib) ? 0 : 1; i < 3 && !n->lib; i++) n->lib = dlopen(names[i], RTLD_NOW | RTLD_LOCAL);
    if (!n->lib) {
        snprintf(err, cap, "no NDL library (%s)", dlerror());
        goto fail;
    }
    n->dl_init = (int (*)(void))sym(n->lib, "NDL_DirectMedia_DL_Initialize");
    n->dl_fin = (void (*)(void))sym(n->lib, "NDL_DirectMedia_DL_Finalize");
    n->set_window = (int (*)(const char *))sym(n->lib, "NDL_DirectMediaSetWindowId");
    n->init = (int (*)(const char *, released_cb))sym(n->lib, "NDL_DirectMediaInit");
    n->load = (int (*)(struct data_info *, load_cb))sym(n->lib, "NDL_DirectMediaLoad");
    n->set_state = (int (*)(int))sym(n->lib, "NDL_DirectMediaSetAppState");
    n->set_area = (int (*)(int, int, int, int))sym(n->lib, "NDL_DirectVideoSetArea");
    n->play = (int (*)(void *, unsigned, long long))sym(n->lib, "NDL_DirectVideoPlay");
    n->queued = (int (*)(int *))sym(n->lib, "NDL_DirectVideoGetRenderBufferLength");
    n->unload = (int (*)(void))sym(n->lib, "NDL_DirectMediaUnload");
    n->quit = (int (*)(void))sym(n->lib, "NDL_DirectMediaQuit");
    n->get_error = (const char *(*)(void))sym(n->lib, "NDL_DirectMediaGetError");
    if (!n->dl_init || !n->set_window || !n->init || !n->load || !n->set_state || !n->play || !n->unload || !n->quit) {
        snprintf(err, cap, "the library lacks %s (API version 1?)", !n->dl_init ? "NDL_DirectMedia_DL_Initialize" : !n->set_window ? "NDL_DirectMediaSetWindowId" : !n->init ? "NDL_DirectMediaInit" : !n->load ? "NDL_DirectMediaLoad" : !n->set_state ? "NDL_DirectMediaSetAppState" : !n->play ? "NDL_DirectVideoPlay" : !n->unload ? "NDL_DirectMediaUnload" : "NDL_DirectMediaQuit");
        goto fail;
    }
    if (!n->dl_init()) {
        snprintf(err, cap, "NDL_DirectMedia_DL_Initialize failed (no implementation library)");
        goto fail;
    }
    *out = n;
    return 0;
fail:
    if (n->lib) dlclose(n->lib);
    if (n->efd >= 0) close(n->efd);
    free(n);
    return -1;
}

const char *vpl_ndl_error(const vpl_ndl *n) {
    const char *e = n->get_error ? n->get_error() : NULL;
    return e && *e ? e : "no message";
}

int vpl_ndl_start(vpl_ndl *n, const char *app_id, const char *window_id, int codec, int width, int height, char *err, size_t cap) {
    if (n->set_window(window_id) != 0) {
        snprintf(err, cap, "NDL_DirectMediaSetWindowId(%s): %s", window_id, vpl_ndl_error(n));
        return -1;
    }
    if (n->init(app_id, on_released) != 0) {
        snprintf(err, cap, "NDL_DirectMediaInit(%s): %s", app_id, vpl_ndl_error(n));
        return -1;
    }
    n->started = 1;
    g_ndl = n;
    struct data_info info;
    memset(&info, 0, sizeof info);
    info.video.width = width;
    info.video.height = height;
    info.video.type = codec == VPC_H264 ? 1 : 2;      /* NDL_VIDEO_TYPE_H264 = 1, _H265 = 2 */
    if (n->load(&info, on_event) != 0) {
        snprintf(err, cap, "NDL_DirectMediaLoad: %s", vpl_ndl_error(n));
        return -1;
    }
    n->loaded = 1;
    if (n->set_state(0) != 0) {                        /* NDL_DIRECTMEDIA_APP_STATE_FOREGROUND */
        snprintf(err, cap, "NDL_DirectMediaSetAppState(foreground): %s", vpl_ndl_error(n));
        return -1;
    }
    return 0;
}

int vpl_ndl_feed(vpl_ndl *n, const void *unit, unsigned size, int64_t pts_us) {
    int rc = n->play((void *)unit, size, (long long)pts_us);
    return rc == 0 ? 0 : 1;
}

int vpl_ndl_queued(vpl_ndl *n) {
    int len = -1;
    if (!n->queued || n->queued(&len) != 0) return -1;
    return len;
}

int vpl_ndl_area(vpl_ndl *n, int x, int y, int w, int h) {
    if (!n->set_area) return -2;
    return n->set_area(x, y, w, h);
}

void vpl_ndl_stop(vpl_ndl *n) {
    if (n->loaded) n->unload();
    n->loaded = 0;
    if (n->started) n->quit();
    n->started = 0;
    g_ndl = NULL;
}

void vpl_ndl_free(vpl_ndl *n) {
    if (!n) return;
    if (n->started) vpl_ndl_stop(n);
    if (n->dl_fin) n->dl_fin();
    /* no dlclose: LG's libraries start threads and register with the bus; unloading them is not something they are written for */
    if (n->efd >= 0) close(n->efd);
    free(n);
}

int vpl_ndl_event_fd(const vpl_ndl *n) { return n->efd; }

void vpl_ndl_event_clear(vpl_ndl *n) {
    uint64_t v;
    if (read(n->efd, &v, sizeof v) < 0) { /* nothing was pending */ }
}

int vpl_ndl_next_event(vpl_ndl *n, int *type, long long *num, char *text, size_t cap) {
    lock(n);
    int got = n->count > 0;
    if (got) {
        *type = n->q[n->head].type;
        *num = n->q[n->head].num;
        snprintf(text, cap, "%s", n->q[n->head].text);
        n->head = (n->head + 1) % EVQ;
        n->count--;
    }
    unlock(n);
    return got;
}
