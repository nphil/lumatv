/* LG's starfish media pipeline in this process: libplayerAPIs.so.1, the C++ class StarfishMediaAPIs (include/starfish-media-pipeline/
 * StarfishMediaAPIs.h of webosbrew/webos-userland; the sequence of webosbrew/samples media/smp). The library is dlopened and its methods are
 * called by their mangled names, so no C++ runtime of ours is involved: `this` is the first argument, a `bool` comes back in r0, an `int&` is
 * a pointer, and the one method that returns a std::string (Feed) takes the result's storage as a hidden first argument (AAPCS), 24 bytes of
 * the C++11 layout {pointer, length, 16-byte local buffer or capacity}, which is freed with the library's own operator delete when the
 * pointer left the local buffer.
 *
 * The load is a BUFFERSTREAM of raw elementary streams (RAW, provider Chrome, the pair of levels a low-latency client uses), the stream's own
 * size and rate as the ceiling the decoder reserves for, and no game mode: this is the pipeline an app's own video has, and outside a Multi
 * View its AV connector takes the sink the load asks for (`avSink`, MAIN by default; Home's pipeline excepted); the NDL load (gf_ndl.c) is a low-delay
 * game session and such a pipeline occupies the sub scaler for itself. Feeding starts as soon as Load returns: the pipeline prerolls on
 * the first buffers and only then says LOADCOMPLETED, and a Play sent before that is queued, so Play is sent after Load, after the first
 * buffer taken and once more when LOADCOMPLETED has been seen.
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

typedef void (*load_cb)(int type, long long num, const char *text, void *data);

#define EVQ 128
#define OBJ_SIZE 65536      /* the class is a few hundred bytes on the sets seen; the storage is zeroed and the constructor fills what it uses */

struct gf_smp {
    void *lib;
    char lib_name[64];
    void (*ctor)(void *self, const char *uid);
    void (*dtor)(void *self);
    int (*load)(void *self, const char *payload, load_cb cb, void *data);
    void (*feed)(void *result, void *self, const char *payload);
    int (*play)(void *self);
    int (*unload)(void *self);
    int (*notify_fg)(void *self);
    int (*notify_bg)(void *self);
    int (*queue_len)(void *self, int *len);
    int (*audio_buf)(void *self, int *total, int *used);
    int (*flush)(void *self);
    int (*set_hdr)(void *self, const char *json);
    const char *(*media_id)(void *self);
    void (*op_delete)(void *);
    void *obj;
    int started, loaded, played_after_load;
    char error[200];

    int lock;                       /* spin lock: callbacks run on the library's threads */
    struct { int type; long long num; char text[200]; } q[EVQ];
    int head, count;
    int efd;
};

#define S(p) ((struct gf_smp *)(p))
#define P(s) ((gf_pipe *)(s))

static void lock(struct gf_smp *s) { while (__atomic_exchange_n(&s->lock, 1, __ATOMIC_ACQUIRE)) sched_yield(); }
static void unlock(struct gf_smp *s) { __atomic_store_n(&s->lock, 0, __ATOMIC_RELEASE); }

static void on_event(int type, long long num, const char *text, void *data) {
    struct gf_smp *s = data;
    if (!s) return;
    lock(s);
    if (type == GF_EV_LOAD_COMPLETED) s->loaded = 1;
    if (s->count < EVQ) {
        int at = (s->head + s->count) % EVQ;
        s->q[at].type = type;
        s->q[at].num = num;
        snprintf(s->q[at].text, sizeof s->q[at].text, "%s", text ? text : "");
        s->count++;
    }
    unlock(s);
    uint64_t one = 1;
    if (write(s->efd, &one, sizeof one) < 0) { /* already readable */ }
}

static const char *smp_error(const gf_pipe *p) { return S(p)->error[0] ? S(p)->error : "no message"; }

static int smp_open_lib(gf_pipe **out, const char *lib, char *err, size_t cap) {
    struct gf_smp *s = calloc(1, sizeof *s);
    if (!s) {
        snprintf(err, cap, "out of memory");
        return -1;
    }
    s->efd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    const char *names[3] = {lib, "libplayerAPIs.so.1", "libplayerAPIs.so"};
    for (int i = (lib && *lib) ? 0 : 1; i < 3 && !s->lib; i++) {
        s->lib = dlopen(names[i], RTLD_NOW | RTLD_LOCAL);
        if (s->lib) {
            const char *base = strrchr(names[i], '/');
            snprintf(s->lib_name, sizeof s->lib_name, "%s", base ? base + 1 : names[i]);
        }
    }
    if (!s->lib) {
        snprintf(err, cap, "no starfish media library (%s)", dlerror());
        goto fail;
    }
    /* StarfishMediaAPIs(const char *uid), ~StarfishMediaAPIs(), Load(const char *, void (*)(int, int64_t, const char *, void *), void *),
     * Feed(const char *) -> std::string, Play(), Unload(), notifyForeground(), notifyBackground(), getVideoRenderQueueLength(int &),
     * getAudioBufferSize(int &, int &), flush(), setHdrInfo(const char *), getMediaID(); and the C++ runtime's operator delete */
    s->ctor = (void (*)(void *, const char *))dlsym(s->lib, "_ZN17StarfishMediaAPIsC1EPKc");
    s->dtor = (void (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIsD1Ev");
    s->load = (int (*)(void *, const char *, load_cb, void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs4LoadEPKcPFvixS1_PvES2_");
    s->feed = (void (*)(void *, void *, const char *))dlsym(s->lib, "_ZN17StarfishMediaAPIs4FeedB5cxx11EPKc");
    s->play = (int (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs4PlayEv");
    s->unload = (int (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs6UnloadEv");
    s->notify_fg = (int (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs16notifyForegroundEv");
    s->notify_bg = (int (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs16notifyBackgroundEv");
    s->queue_len = (int (*)(void *, int *))dlsym(s->lib, "_ZN17StarfishMediaAPIs25getVideoRenderQueueLengthERi");
    s->audio_buf = (int (*)(void *, int *, int *))dlsym(s->lib, "_ZN17StarfishMediaAPIs18getAudioBufferSizeERiS0_");
    s->flush = (int (*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs5flushEv");
    s->set_hdr = (int (*)(void *, const char *))dlsym(s->lib, "_ZN17StarfishMediaAPIs10setHdrInfoEPKc");
    s->media_id = (const char *(*)(void *))dlsym(s->lib, "_ZN17StarfishMediaAPIs10getMediaIDEv");
    s->op_delete = (void (*)(void *))dlsym(s->lib, "_ZdlPv");
    const char *missing = !s->ctor ? "StarfishMediaAPIs(const char *)" : !s->dtor ? "~StarfishMediaAPIs()" : !s->load ? "Load with a user pointer"
                        : !s->feed ? "Feed (C++11 string)" : !s->play ? "Play" : !s->unload ? "Unload" : !s->notify_fg ? "notifyForeground"
                        : !s->op_delete ? "operator delete" : NULL;
    if (missing) {
        snprintf(err, cap, "the library lacks %s (not the webOS 5+ build?)", missing);
        goto fail;
    }
    *out = P(s);
    return 0;
fail:
    if (s->lib) dlclose(s->lib);
    if (s->efd >= 0) close(s->efd);
    free(s);
    return -1;
}

static const char *smp_lib_name(const gf_pipe *p) { return S(p)->lib_name; }

static unsigned smp_features(const gf_pipe *p) {
    const struct gf_smp *s = S(p);
    return (s->flush ? GF_PIPE_HAS_FLUSH : 0) | (s->audio_buf ? GF_PIPE_HAS_AUDIO_AVAIL : 0);
}

/* The OpusHead the pipeline wants with an Opus load, base64 (RFC 7845 section 5.1: version 1, the channel count, 312 samples of pre-skip,
 * the input rate, no gain, mapping family 0; what YouTube's load carries for its 48 kHz stereo). */
static void opus_head(char *out, size_t cap, unsigned channels, unsigned rate) {
    unsigned char h[19] = {'O', 'p', 'u', 's', 'H', 'e', 'a', 'd', 1, (unsigned char)channels, 312 & 255, 312 >> 8,
                           (unsigned char)rate, (unsigned char)(rate >> 8), (unsigned char)(rate >> 16), (unsigned char)(rate >> 24), 0, 0, 0};
    static const char b64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t o = 0;
    for (size_t i = 0; i < sizeof h && o + 5 < cap; i += 3) {
        unsigned v = (unsigned)h[i] << 16 | (i + 1 < sizeof h ? (unsigned)h[i + 1] << 8 : 0) | (i + 2 < sizeof h ? h[i + 2] : 0);
        out[o++] = b64[v >> 18 & 63];
        out[o++] = b64[v >> 12 & 63];
        out[o++] = i + 1 < sizeof h ? b64[v >> 6 & 63] : '=';
        out[o++] = i + 2 < sizeof h ? b64[v & 63] : '=';
    }
    out[o] = 0;
}

static int json_safe(const char *s) {
    for (; *s; s++)
        if (*s == '"' || *s == '\\' || (unsigned char)*s < 32) return 0;
    return 1;
}

static int smp_start(gf_pipe *p, const char *app_id, const char *window_id, const gf_load *l, const gf_pipe_opts *o, char *err, size_t cap) {
    struct gf_smp *s = S(p);
    s->error[0] = 0;
    const char *codec = l->codec == GF_CODEC_H264 ? "H264" : l->codec == GF_CODEC_H265 ? "H265" : l->codec == GF_CODEC_AV1 ? "AV1" : NULL;
    if (!codec) {
        snprintf(err, cap, "codec %u is not one the pipeline takes", l->codec);
        return -1;
    }
    if (l->audio == GF_AUDIO_PCM_S16LE) {
        snprintf(err, cap, "the starfish pipeline takes Opus or no audio, not PCM");
        return -1;
    }
    const char *sink = o->sink && o->sink[0] ? o->sink : "";
    if (!json_safe(app_id) || !json_safe(window_id) || !json_safe(sink)) {
        snprintf(err, cap, "the app id, the window id or the sink is not a plain string");
        return -1;
    }
    /* LG's AV connector (libav-connector.so.1, MultiviewConnector::processForcePictureInPictureMode, run when its Multi View watcher first
     * reports) puts a media pipeline of Home's on the main window in pip mode whatever sink the load asked for: Home's own picture-in-picture.
     * The connect then takes MAIN_SCALER from the app whose video holds it (M2, docs/research/multiview-result.md). Any other app keeps the
     * sink it asks for, so the sub window is asked for under another app's id. */
    if (!strcmp(sink, "sub_video") && !strcmp(app_id, "com.webos.app.home")) {
        snprintf(err, cap, "the sub_video sink is never Home's: LG's AV connector puts Home's media pipeline on the main window in pip mode");
        return -1;
    }
    /* the sink the video goes to (`avSink.videoSink.type`: main_video, sub_video, graphic): the AV connector inside the library takes the name as is
     * and asks the resource manager for that scaler, SUB_SCALER for sub_video, then videooutputd connects the decoder to that sink (SUB0) */
    char avsink[80] = "";
    if (sink[0]) snprintf(avsink, sizeof avsink, ",\"avSink\":{\"videoSink\":{\"type\":\"%s\"}}", sink);
    char audio[300] = "";
    if (l->audio == GF_AUDIO_OPUS) {
        char head[40];
        opus_head(head, sizeof head, l->channels, l->sample_rate);
        snprintf(audio, sizeof audio, ",\"opusInfo\":{\"sampleRate\":%u,\"channels\":\"%u\",\"streamHeader\":\"%s\"}", l->sample_rate / 1000, l->channels, head);
    }
    char payload[1700];
    unsigned fps = l->fps ? l->fps : 30;
    snprintf(payload, sizeof payload,
             "{\"args\":[{\"mediaTransportType\":\"BUFFERSTREAM\",\"option\":{\"appId\":\"%s\",\"windowId\":\"%s\",\"queryPosition\":false%s,"
             "\"externalStreamingInfo\":{\"contents\":{\"codec\":{\"video\":\"%s\"%s},"
             "\"esInfo\":{\"pauseAtDecodeTime\":false,\"ptsToDecode\":0,\"seperatedPTS\":true,\"videoFpsValue\":%u,\"videoFpsScale\":1},"
             "\"format\":\"RAW\",\"provider\":\"Chrome\"%s},"
             "\"restartStreaming\":false,\"streamQualityInfo\":true,\"streamQualityInfoNonFlushable\":true,\"totalStreamSize\":256,"
             "\"bufferingCtrInfo\":{\"preBufferByte\":0,\"bufferMinLevel\":0,\"bufferMaxLevel\":0,\"qBufferLevelAudio\":0,\"qBufferLevelVideo\":0,"
             "\"srcBufferLevelAudio\":{\"minimum\":1024,\"maximum\":32768},\"srcBufferLevelVideo\":{\"minimum\":1024,\"maximum\":1048576}}},"
             "\"adaptiveStreaming\":{\"audioOnly\":false,\"maxWidth\":%u,\"maxHeight\":%u,\"maxFrameRate\":%u}}}]}",
             app_id, window_id, avsink, codec, l->audio == GF_AUDIO_OPUS ? ",\"audio\":\"OPUS\"" : "", fps, audio, l->width, l->height, fps);
    if (!s->obj) {
        s->obj = calloc(1, OBJ_SIZE);
        if (!s->obj) {
            snprintf(err, cap, "out of memory");
            return -1;
        }
        s->ctor(s->obj, NULL);
    }
    s->started = 1;
    s->loaded = s->played_after_load = 0;
    if (!(s->notify_fg(s->obj) & 1)) gf_say("smp: notifyForeground said no (the pipeline may refuse the load)");
    if (!(s->load(s->obj, payload, on_event, s) & 1)) {
        snprintf(s->error, sizeof s->error, "Load refused");
        snprintf(err, cap, "StarfishMediaAPIs::Load refused the load (codec %s, %ux%u at %u, audio %u, sink %s)", codec, l->width, l->height, fps, l->audio, sink[0] ? sink : "main");
        return GF_PIPE_LOAD_FAILED;
    }
    s->play(s->obj);
    if (l->hdr) gf_say("smp: the HDR metadata is not passed on this pipeline; the stream's own SEI is what the display gets");
    const char *id = s->media_id ? s->media_id(s->obj) : NULL;
    gf_say("smp: loaded %s %ux%u at %u for %s in %s, media id %s", codec, l->width, l->height, fps, app_id, window_id, id ? id : "?");
    return 0;
}

/* Feed's std::string: "Ok" taken, "BufferFull" or "Pending" try again, anything else an error. */
static int smp_feed(struct gf_smp *s, const void *data, unsigned size, long long pts_ns, int es) {
    if (!s->started) return -1;
    if (s->loaded && !s->played_after_load) {
        s->played_after_load = 1;
        s->play(s->obj);
    }
    char payload[160];
    snprintf(payload, sizeof payload, "{\"bufferAddr\":\"%p\",\"bufferSize\":%u,\"pts\":%lld,\"esData\":%d}", data, size, pts_ns, es);
    struct { char *ptr; size_t len; union { char local[16]; size_t cap; } u; } reply __attribute__((aligned(8)));
    memset(&reply, 0, sizeof reply);
    s->feed(&reply, s->obj, payload);
    const char *text = reply.ptr ? reply.ptr : "";
    int rc = strstr(text, "Ok") ? 0 : (strstr(text, "BufferFull") || strstr(text, "Pending")) ? 1 : -1;
    if (rc < 0) snprintf(s->error, sizeof s->error, "Feed: %s", text[0] ? text : "no reply");
    if (reply.ptr && reply.ptr != reply.u.local) s->op_delete(reply.ptr);
    return rc;
}

static int smp_feed_video(gf_pipe *p, const void *unit, unsigned size, long long pts_ns) { return smp_feed(S(p), unit, size, pts_ns, 1); }

static int smp_feed_audio(gf_pipe *p, const void *data, unsigned size, long long pts_ns) { return smp_feed(S(p), data, size, pts_ns, 2); }

static int smp_queued(gf_pipe *p) {
    struct gf_smp *s = S(p);
    int len = -1;
    if (!s->started || !s->queue_len || !(s->queue_len(s->obj, &len) & 1)) return -1;
    return len;
}

static int smp_audio_avail(gf_pipe *p) {
    struct gf_smp *s = S(p);
    int total = 0, used = 0;
    if (!s->started || !s->audio_buf || !(s->audio_buf(s->obj, &total, &used) & 1)) return -1;
    return total > used ? total - used : 0;
}

static int smp_flush(gf_pipe *p) {
    struct gf_smp *s = S(p);
    if (!s->flush) return -2;
    return s->started && (s->flush(s->obj) & 1) ? 0 : -1;
}

static int smp_set_hdr(gf_pipe *p, const gf_hdr *h) {
    (void)p;
    (void)h;
    return -2;
}

static int smp_set_drop_threshold(gf_pipe *p, int threshold) {
    (void)p;
    (void)threshold;
    return -2;
}

static int smp_set_state(gf_pipe *p, int foreground) {
    struct gf_smp *s = S(p);
    if (!s->obj) return -1;
    int ok = foreground ? s->notify_fg(s->obj) : s->notify_bg ? s->notify_bg(s->obj) : 0;
    return (ok & 1) ? 0 : -1;
}

static int smp_area(gf_pipe *p, int x, int y, int w, int h) {
    (void)p;
    (void)x;
    (void)y;
    (void)w;
    (void)h;
    return -2;
}

static void smp_stop(gf_pipe *p) {
    struct gf_smp *s = S(p);
    if (s->started) s->unload(s->obj);
    s->started = s->loaded = 0;
    if (s->obj) {
        s->dtor(s->obj);
        free(s->obj);
        s->obj = NULL;
    }
}

static void smp_free(gf_pipe *p) {
    struct gf_smp *s = S(p);
    if (!s) return;
    smp_stop(p);
    /* no dlclose: LG's libraries start threads and register with the bus; they are not written to be unloaded */
    if (s->efd >= 0) close(s->efd);
    free(s);
}

static int smp_event_fd(const gf_pipe *p) { return S(p)->efd; }

static void smp_event_clear(gf_pipe *p) {
    uint64_t v;
    if (read(S(p)->efd, &v, sizeof v) < 0) { /* nothing pending */ }
}

static int smp_next_event(gf_pipe *p, int *type, long long *num, char *text, size_t cap) {
    struct gf_smp *s = S(p);
    lock(s);
    int got = s->count > 0;
    if (got) {
        *type = s->q[s->head].type;
        *num = s->q[s->head].num;
        snprintf(text, cap, "%s", s->q[s->head].text);
        s->head = (s->head + 1) % EVQ;
        s->count--;
    }
    unlock(s);
    return got;
}

const gf_pipe_ops gf_smp_ops = {
    .name = "smp",
    .open_lib = smp_open_lib,
    .lib_name = smp_lib_name,
    .features = smp_features,
    .start = smp_start,
    .feed_video = smp_feed_video,
    .feed_audio = smp_feed_audio,
    .queued = smp_queued,
    .audio_avail = smp_audio_avail,
    .flush = smp_flush,
    .set_hdr = smp_set_hdr,
    .set_drop_threshold = smp_set_drop_threshold,
    .set_state = smp_set_state,
    .area = smp_area,
    .stop = smp_stop,
    .free = smp_free,
    .event_fd = smp_event_fd,
    .event_clear = smp_event_clear,
    .next_event = smp_next_event,
    .error = smp_error,
};
