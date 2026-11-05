/* LG's NDL DirectMedia (libNDL_directmedia.so.1, API 2, webOS 5 and later), loaded at run time: the video and audio ES feed, the HDR
 * metadata, the cloud-game frame-drop threshold and the render-queue readings the probe records.
 *
 * The call order the library demands (jellyfin-webos-native docs/ndl.md, webosbrew/samples; the stand-in gfn/feeder/tests/fake_ndl.c
 * refuses anything else): DL_Initialize, SetWindowId (or the library makes an SDL window and fails), Init(app id of the installed app
 * whose role names the media client services), Load(video and audio info together), SetAppState(FOREGROUND) (or every unit is dropped
 * silently), then Play per access unit. SetHDRInfo and SetFrameDropThreshold are applied after Load. The ES player has no clock: a unit is
 * shown when it arrives, the PTS is for A/V alignment and the queue bookkeeping. The same shape as vplane/src/vpl_ndl.h, which this
 * extends with audio, HDR and the game-mode knobs; it is its own copy so the saver and the stream can change independently.
 */
#define _GNU_SOURCE
#include "gf_pipe.h"

#include <dlfcn.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <unistd.h>

#include "gf_annexb.h"
#include "gf_log.h"

/* NDL_DIRECTMEDIA_DATA_INFO_T and the pieces of its union this feeder fills (NDL_directmedia_types.h of webosbrew/webos-userland). */
struct pcm_info { int type, unknown1; const char *format, *layout, *channelMode; int sampleRate; };
struct opus_info { int type, unknown1, channels, unknown2; double sampleRate; const char *streamHeader; };
struct data_info {
    struct { int width, height, type, unknown1; } video;
    union { int type; struct pcm_info pcm; struct opus_info opus; char padding[32]; } audio;
};
struct hdr_info { unsigned v[15]; };

typedef void (*load_cb)(int, long long, const char *);
typedef void (*released_cb)(const char *);

#define EVQ 128

struct gf_ndl {
    void *lib;
    char lib_name[64];
    int (*dl_init)(void);
    void (*dl_fin)(void);
    int (*set_window)(const char *);
    int (*init)(const char *, released_cb);
    int (*load)(struct data_info *, load_cb);
    int (*set_state)(int);
    int (*set_area)(int, int, int, int);
    int (*play)(void *, unsigned, long long);
    int (*play_audio)(void *, unsigned, long long);
    int (*queued)(int *);
    int (*audio_avail)(int *);
    int (*flush)(void);
    int (*set_hdr)(struct hdr_info);
    int (*set_drop)(int);
    int (*unload)(void);
    int (*quit)(void);
    const char *(*get_error)(void);
    int started, loaded;
    long long pts_unit;             /* what a feed's nanoseconds are divided by: the load's `pts_unit` */

    int lock;                       /* spin lock: callbacks run on the library's threads */
    struct { int type; long long num; char text[200]; } q[EVQ];
    int head, count;
    int efd;
};

#define N(p) ((struct gf_ndl *)(p))
#define P(n) ((gf_pipe *)(n))

static struct gf_ndl *g_ndl;        /* the load callback carries no user pointer */

static void lock(struct gf_ndl *n) { while (__atomic_exchange_n(&n->lock, 1, __ATOMIC_ACQUIRE)) sched_yield(); }
static void unlock(struct gf_ndl *n) { __atomic_store_n(&n->lock, 0, __ATOMIC_RELEASE); }

static void on_released(const char *type) { gf_say("ndl: resource released by policy: %s", type ? type : "?"); }

static void on_event(int type, long long num, const char *text) {
    struct gf_ndl *n = g_ndl;
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
    if (write(n->efd, &one, sizeof one) < 0) { /* already readable */ }
}

static const char *ndl_error(const gf_pipe *p) {
    const char *e = N(p)->get_error ? N(p)->get_error() : NULL;
    return e && *e ? e : "no message";
}

static int ndl_open_lib(gf_pipe **out, const char *lib, char *err, size_t cap) {
    struct gf_ndl *n = calloc(1, sizeof *n);
    if (!n) {
        snprintf(err, cap, "out of memory");
        return -1;
    }
    n->efd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    const char *names[3] = {lib, "libNDL_directmedia.so.1", "libNDL_directmedia.so"};
    for (int i = (lib && *lib) ? 0 : 1; i < 3 && !n->lib; i++) {
        n->lib = dlopen(names[i], RTLD_NOW | RTLD_LOCAL);
        if (n->lib) {
            const char *base = strrchr(names[i], '/');
            snprintf(n->lib_name, sizeof n->lib_name, "%s", base ? base + 1 : names[i]);
        }
    }
    if (!n->lib) {
        snprintf(err, cap, "no NDL library (%s)", dlerror());
        goto fail;
    }
    n->dl_init = (int (*)(void))dlsym(n->lib, "NDL_DirectMedia_DL_Initialize");
    n->dl_fin = (void (*)(void))dlsym(n->lib, "NDL_DirectMedia_DL_Finalize");
    n->set_window = (int (*)(const char *))dlsym(n->lib, "NDL_DirectMediaSetWindowId");
    n->init = (int (*)(const char *, released_cb))dlsym(n->lib, "NDL_DirectMediaInit");
    n->load = (int (*)(struct data_info *, load_cb))dlsym(n->lib, "NDL_DirectMediaLoad");
    n->set_state = (int (*)(int))dlsym(n->lib, "NDL_DirectMediaSetAppState");
    n->set_area = (int (*)(int, int, int, int))dlsym(n->lib, "NDL_DirectVideoSetArea");
    n->play = (int (*)(void *, unsigned, long long))dlsym(n->lib, "NDL_DirectVideoPlay");
    n->play_audio = (int (*)(void *, unsigned, long long))dlsym(n->lib, "NDL_DirectAudioPlay");
    n->queued = (int (*)(int *))dlsym(n->lib, "NDL_DirectVideoGetRenderBufferLength");
    n->audio_avail = (int (*)(int *))dlsym(n->lib, "NDL_DirectAudioGetAvailableBufferSize");
    n->flush = (int (*)(void))dlsym(n->lib, "NDL_DirectVideoFlushRenderBuffer");
    n->set_hdr = (int (*)(struct hdr_info))dlsym(n->lib, "NDL_DirectVideoSetHDRInfo");
    n->set_drop = (int (*)(int))dlsym(n->lib, "NDL_DirectVideoSetFrameDropThreshold");
    n->unload = (int (*)(void))dlsym(n->lib, "NDL_DirectMediaUnload");
    n->quit = (int (*)(void))dlsym(n->lib, "NDL_DirectMediaQuit");
    n->get_error = (const char *(*)(void))dlsym(n->lib, "NDL_DirectMediaGetError");
    const char *missing = !n->dl_init ? "NDL_DirectMedia_DL_Initialize" : !n->set_window ? "NDL_DirectMediaSetWindowId" : !n->init ? "NDL_DirectMediaInit"
                        : !n->load ? "NDL_DirectMediaLoad" : !n->set_state ? "NDL_DirectMediaSetAppState" : !n->play ? "NDL_DirectVideoPlay"
                        : !n->play_audio ? "NDL_DirectAudioPlay" : !n->unload ? "NDL_DirectMediaUnload" : !n->quit ? "NDL_DirectMediaQuit" : NULL;
    if (missing) {
        snprintf(err, cap, "the library lacks %s (API version 1?)", missing);
        goto fail;
    }
    if (!n->dl_init()) {
        snprintf(err, cap, "NDL_DirectMedia_DL_Initialize failed (no implementation library)");
        goto fail;
    }
    n->pts_unit = 1;
    *out = P(n);
    return 0;
fail:
    if (n->lib) dlclose(n->lib);
    if (n->efd >= 0) close(n->efd);
    free(n);
    return -1;
}

static const char *ndl_lib_name(const gf_pipe *p) { return N(p)->lib_name; }

static unsigned ndl_features(const gf_pipe *p) {
    const struct gf_ndl *n = N(p);
    return (n->set_hdr ? GF_PIPE_HAS_HDR : 0) | (n->set_drop ? GF_PIPE_HAS_DROP_THRESHOLD : 0) | (n->flush ? GF_PIPE_HAS_FLUSH : 0)
         | (n->audio_avail ? GF_PIPE_HAS_AUDIO_AVAIL : 0) | (n->set_area ? GF_PIPE_HAS_SET_AREA : 0);
}

/* The sample-rate enum is not in frequency order (NDL_directmedia_types.h). */
static int pcm_rate(unsigned hz) {
    switch (hz) {
    case 48000: return 1;
    case 44100: return 2;
    case 32000: return 3;
    case 24000: return 4;
    case 16000: return 5;
    case 12000: return 6;
    case 8000: return 7;
    case 22050: return 8;
    default: return 0;
    }
}

static int ndl_set_hdr(gf_pipe *p, const gf_hdr *h) {
    struct gf_ndl *n = N(p);
    if (!n->set_hdr) return -2;
    struct hdr_info i;
    memcpy(i.v, h, sizeof i.v);
    return n->set_hdr(i);
}

static int ndl_set_drop_threshold(gf_pipe *p, int threshold) { return N(p)->set_drop ? N(p)->set_drop(threshold) : -2; }

static int ndl_start(gf_pipe *p, const char *app_id, const char *window_id, const gf_load *l, const gf_pipe_opts *o, char *err, size_t cap) {
    struct gf_ndl *n = N(p);
    if (o->sink && o->sink[0]) {
        snprintf(err, cap, "NDL cannot ask for the %s sink; the starfish pipeline can (--pipe smp)", o->sink);
        return -1;
    }
    if (n->set_window(window_id) != 0) {
        snprintf(err, cap, "NDL_DirectMediaSetWindowId(%s): %s", window_id, ndl_error(p));
        return -1;
    }
    if (n->init(app_id, on_released) != 0) {
        snprintf(err, cap, "NDL_DirectMediaInit(%s): %s", app_id, ndl_error(p));
        return -1;
    }
    n->started = 1;
    n->pts_unit = l->pts_unit > 0 ? l->pts_unit : 1;
    g_ndl = n;
    struct data_info info;
    memset(&info, 0, sizeof info);
    info.video.width = (int)l->width;
    info.video.height = (int)l->height;
    info.video.type = l->codec == GF_CODEC_H264 ? 1 : l->codec == GF_CODEC_H265 ? 2 : l->codec == GF_CODEC_AV1 ? 4 : 0;
    if (l->audio == GF_AUDIO_OPUS) {
        info.audio.opus.type = 3;
        info.audio.opus.channels = (int)l->channels;
        info.audio.opus.sampleRate = l->sample_rate / 1000.0;      /* kHz, as SS4S ndl_audio.c passes it */
    } else if (l->audio == GF_AUDIO_PCM_S16LE) {
        info.audio.pcm.type = 1;
        info.audio.pcm.format = "S16LE";
        info.audio.pcm.layout = "interleaved";
        info.audio.pcm.channelMode = l->channels == 1 ? "mono" : l->channels == 6 ? "6-channel" : "stereo";
        info.audio.pcm.sampleRate = pcm_rate(l->sample_rate);
        if (!info.audio.pcm.sampleRate) {
            snprintf(err, cap, "PCM at %u Hz is not a rate the library has (0 would be bypass and take the pipeline down)", l->sample_rate);
            return -1;
        }
    }
    if (n->load(&info, on_event) != 0) {
        snprintf(err, cap, "NDL_DirectMediaLoad: %s", ndl_error(p));
        return GF_PIPE_LOAD_FAILED;
    }
    n->loaded = 1;
    if (n->set_state(0) != 0) {
        snprintf(err, cap, "NDL_DirectMediaSetAppState(foreground): %s", ndl_error(p));
        return -1;
    }
    if (l->hdr) {
        int rc = ndl_set_hdr(p, &l->hdr_info);
        if (rc == -2) gf_say("ndl: this library has no NDL_DirectVideoSetHDRInfo; the stream's own SEI is what the display gets");
        else if (rc != 0) gf_say("ndl: NDL_DirectVideoSetHDRInfo returned %d: %s", rc, ndl_error(p));
    }
    if (l->drop_threshold >= 0) {
        int rc = ndl_set_drop_threshold(p, l->drop_threshold);
        if (rc == -2) gf_say("ndl: this library has no NDL_DirectVideoSetFrameDropThreshold");
        else if (rc != 0) gf_say("ndl: NDL_DirectVideoSetFrameDropThreshold(%d) returned %d: %s", l->drop_threshold, rc, ndl_error(p));
    }
    return 0;
}

static int ndl_feed_video(gf_pipe *p, const void *unit, unsigned size, long long pts_ns) {
    int rc = N(p)->play((void *)unit, size, pts_ns / N(p)->pts_unit);
    return rc == 0 ? 0 : 1;
}

static int ndl_feed_audio(gf_pipe *p, const void *data, unsigned size, long long pts_ns) {
    int rc = N(p)->play_audio((void *)data, size, pts_ns / N(p)->pts_unit);
    return rc == 0 ? 0 : 1;
}

static int ndl_queued(gf_pipe *p) {
    int len = -1;
    if (!N(p)->queued || N(p)->queued(&len) != 0) return -1;
    return len;
}

static int ndl_audio_avail(gf_pipe *p) {
    int v = -1;
    if (!N(p)->audio_avail || N(p)->audio_avail(&v) != 0) return -1;
    return v;
}

static int ndl_flush(gf_pipe *p) { return N(p)->flush ? N(p)->flush() : -2; }

static int ndl_set_state(gf_pipe *p, int foreground) { return N(p)->set_state(foreground ? 0 : 1); }

static int ndl_area(gf_pipe *p, int x, int y, int w, int h) { return N(p)->set_area ? N(p)->set_area(x, y, w, h) : -2; }

static void ndl_stop(gf_pipe *p) {
    struct gf_ndl *n = N(p);
    if (n->loaded) n->unload();
    n->loaded = 0;
    if (n->started) n->quit();
    n->started = 0;
    g_ndl = NULL;
}

static void ndl_free(gf_pipe *p) {
    struct gf_ndl *n = N(p);
    if (!n) return;
    if (n->started) ndl_stop(p);
    if (n->dl_fin) n->dl_fin();
    /* no dlclose: LG's libraries start threads and register with the bus; they are not written to be unloaded */
    if (n->efd >= 0) close(n->efd);
    free(n);
}

static int ndl_event_fd(const gf_pipe *p) { return N(p)->efd; }

static void ndl_event_clear(gf_pipe *p) {
    uint64_t v;
    if (read(N(p)->efd, &v, sizeof v) < 0) { /* nothing pending */ }
}

static int ndl_next_event(gf_pipe *p, int *type, long long *num, char *text, size_t cap) {
    struct gf_ndl *n = N(p);
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

const gf_pipe_ops gf_ndl_ops = {
    .name = "ndl",
    .open_lib = ndl_open_lib,
    .lib_name = ndl_lib_name,
    .features = ndl_features,
    .start = ndl_start,
    .feed_video = ndl_feed_video,
    .feed_audio = ndl_feed_audio,
    .queued = ndl_queued,
    .audio_avail = ndl_audio_avail,
    .flush = ndl_flush,
    .set_hdr = ndl_set_hdr,
    .set_drop_threshold = ndl_set_drop_threshold,
    .set_state = ndl_set_state,
    .area = ndl_area,
    .stop = ndl_stop,
    .free = ndl_free,
    .event_fd = ndl_event_fd,
    .event_clear = ndl_event_clear,
    .next_event = ndl_next_event,
    .error = ndl_error,
};

const gf_pipe_ops *gf_pipe_named(const char *name) {
    if (!name || !*name || !strcmp(name, "ndl")) return &gf_ndl_ops;
    if (!strcmp(name, "es")) return &gf_es_ops;
    if (!strcmp(name, "smp")) return &gf_smp_ops;
    return NULL;
}
