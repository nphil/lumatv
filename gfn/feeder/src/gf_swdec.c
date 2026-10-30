#define _GNU_SOURCE
#include "gf_swdec.h"

#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavcodec/version_major.h>
#include <libavutil/error.h>
#include <libavutil/frame.h>
#include <libavutil/pixfmt.h>

#include "gf_annexb.h"
#include "gf_proto.h"

struct fns {
    const AVCodec *(*find_by_name)(const char *);
    AVCodecContext *(*alloc_ctx)(const AVCodec *);
    int (*open)(AVCodecContext *, const AVCodec *, AVDictionary **);
    void (*free_ctx)(AVCodecContext **);
    int (*send)(AVCodecContext *, const AVPacket *);
    int (*recv)(AVCodecContext *, AVFrame *);
    void (*flush)(AVCodecContext *);
    AVPacket *(*pkt_alloc)(void);
    void (*pkt_free)(AVPacket **);
    AVFrame *(*frame_alloc)(void);
    void (*frame_free)(AVFrame **);
    void (*frame_unref)(AVFrame *);
    int (*strerror)(int, char *, size_t);
    unsigned (*version)(void);
};

struct gf_swdec {
    void *util, *codec;
    struct fns f;
    AVCodecContext *ctx;
    AVPacket *pkt;
    AVFrame *frame, *spare;     /* the picture that is kept, and the one the next is received into */
    int have;
};

/* The first file of `dir` that begins with `prefix` (libavutil-lumatv.so.), as a full path. */
static int find_lib(const char *dir, const char *prefix, char *out, size_t cap) {
    DIR *d = opendir(dir);
    if (!d) return -1;
    int found = -1;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (strncmp(e->d_name, prefix, strlen(prefix)) != 0) continue;
        /* the soname (libavcodec-lumatv.so.63), not the versioned file's links: the shortest name that matches wins */
        if (found < 0 || strlen(e->d_name) < strlen(out + strlen(dir) + 1)) {
            snprintf(out, cap, "%s/%s", dir, e->d_name);
            found = 0;
        }
    }
    closedir(d);
    return found;
}

static void *load(const char *dir, const char *prefix, const char *plain, int global, char *err, size_t cap) {
    char path[600];
    const char *name = plain;
    if (dir && *dir) {
        if (find_lib(dir, prefix, path, sizeof path) != 0) {
            snprintf(err, cap, "the texture decoder is not installed: no %s* in %s", prefix, dir);
            return NULL;
        }
        name = path;
    }
    void *h = dlopen(name, RTLD_NOW | (global ? RTLD_GLOBAL : RTLD_LOCAL));
    if (!h) snprintf(err, cap, "%s", dlerror());
    return h;
}

#define SYM(h, field, name)                                                              \
    do {                                                                                 \
        *(void **)(&d->f.field) = dlsym(h, name);                                        \
        if (!d->f.field) {                                                               \
            snprintf(err, cap, "the decoder library lacks %s", name);                    \
            goto fail;                                                                   \
        }                                                                                \
    } while (0)

int gf_swdec_open(gf_swdec **out, const char *libdir, unsigned codec, unsigned width, unsigned height, char *err, size_t cap) {
    const char *name = codec == GF_CODEC_H264 ? "h264" : codec == GF_CODEC_H265 ? "hevc" : NULL;
    if (!name) {
        snprintf(err, cap, "the texture path decodes H.264 and H.265, not codec %u", codec);
        return -1;
    }
    gf_swdec *d = calloc(1, sizeof *d);
    if (!d) {
        snprintf(err, cap, "out of memory");
        return -1;
    }
    d->util = load(libdir, "libavutil-lumatv.so.", "libavutil-lumatv.so", 1, err, cap);
    if (!d->util) goto fail;
    d->codec = load(libdir, "libavcodec-lumatv.so.", "libavcodec-lumatv.so", 0, err, cap);
    if (!d->codec) goto fail;
    SYM(d->codec, find_by_name, "avcodec_find_decoder_by_name");
    SYM(d->codec, alloc_ctx, "avcodec_alloc_context3");
    SYM(d->codec, open, "avcodec_open2");
    SYM(d->codec, free_ctx, "avcodec_free_context");
    SYM(d->codec, send, "avcodec_send_packet");
    SYM(d->codec, recv, "avcodec_receive_frame");
    SYM(d->codec, flush, "avcodec_flush_buffers");
    SYM(d->codec, pkt_alloc, "av_packet_alloc");
    SYM(d->codec, pkt_free, "av_packet_free");
    SYM(d->codec, version, "avcodec_version");
    SYM(d->util, frame_alloc, "av_frame_alloc");
    SYM(d->util, frame_free, "av_frame_free");
    SYM(d->util, frame_unref, "av_frame_unref");
    SYM(d->util, strerror, "av_strerror");
    if ((d->f.version() >> 16) != LIBAVCODEC_VERSION_MAJOR) {
        snprintf(err, cap, "libavcodec is major %u, the feeder was built for %u", d->f.version() >> 16, (unsigned)LIBAVCODEC_VERSION_MAJOR);
        goto fail;
    }
    const AVCodec *c = d->f.find_by_name(name);
    if (!c) {
        snprintf(err, cap, "this libavcodec has no %s decoder", name);
        goto fail;
    }
    d->ctx = d->f.alloc_ctx(c);
    d->pkt = d->f.pkt_alloc();
    d->frame = d->f.frame_alloc();
    d->spare = d->f.frame_alloc();
    if (!d->ctx || !d->pkt || !d->frame || !d->spare) {
        snprintf(err, cap, "out of memory");
        goto fail;
    }
    /* one thread (the compositor and the UI have the other cores), no waiting for reordering, the cheap paths, and no loop filter on pictures nothing
     * refers to: a camera's preview is not a film */
    d->ctx->thread_count = 1;
    d->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
    d->ctx->flags2 |= AV_CODEC_FLAG2_FAST;
    d->ctx->skip_loop_filter = AVDISCARD_NONREF;
    d->ctx->width = (int)width;
    d->ctx->height = (int)height;
    int rc = d->f.open(d->ctx, c, NULL);
    if (rc < 0) {
        char why[128] = "";
        d->f.strerror(rc, why, sizeof why);
        snprintf(err, cap, "avcodec_open2 (%s): %s", name, why);
        goto fail;
    }
    *out = d;
    return 0;
fail:
    gf_swdec_close(d);
    return -1;
}

int gf_swdec_decode(gf_swdec *d, const void *unit, size_t size, int64_t pts, char *err, size_t cap) {
    d->pkt->data = (uint8_t *)unit;         /* not reference counted: the decoder copies it, with its padding */
    d->pkt->size = (int)size;
    d->pkt->pts = pts;
    d->pkt->dts = pts;
    int rc = d->f.send(d->ctx, d->pkt);
    if (rc < 0 && rc != AVERROR(EAGAIN)) {
        char why[128] = "";
        d->f.strerror(rc, why, sizeof why);
        snprintf(err, cap, "%s", why);
        return -1;
    }
    int got = 0;
    for (;;) {
        rc = d->f.recv(d->ctx, d->spare);
        if (rc < 0) break;                  /* EAGAIN: nothing more for now; EOF cannot happen without a flush packet */
        AVFrame *t = d->frame;
        d->f.frame_unref(t);
        d->frame = d->spare;
        d->spare = t;
        got = 1;
    }
    if (got) d->have = 1;
    if (rc != AVERROR(EAGAIN) && rc != AVERROR_EOF && !got) {
        char why[128] = "";
        d->f.strerror(rc, why, sizeof why);
        snprintf(err, cap, "%s", why);
        return -1;
    }
    return got;
}

int gf_swdec_picture(const gf_swdec *d, gf_yuv *out, char *err, size_t cap) {
    const AVFrame *f = d->frame;
    if (!d->have || f->width <= 0 || f->height <= 0) {
        snprintf(err, cap, "no picture");
        return -1;
    }
    int full = f->format == AV_PIX_FMT_YUVJ420P;
    if (f->format != AV_PIX_FMT_YUV420P && !full) {
        snprintf(err, cap, "the picture's pixel format %d is not 8-bit 4:2:0 (the texture path takes what cameras send)", f->format);
        return -1;
    }
    if (f->color_range == AVCOL_RANGE_JPEG) full = 1;
    int bt709 = f->colorspace == AVCOL_SPC_BT709 ||
                (f->colorspace != AVCOL_SPC_BT470BG && f->colorspace != AVCOL_SPC_SMPTE170M && f->colorspace != AVCOL_SPC_SMPTE240M && f->height >= 720);
    for (int i = 0; i < 3; i++) {
        out->plane[i] = f->data[i];
        out->stride[i] = f->linesize[i];
    }
    out->width = f->width;
    out->height = f->height;
    out->full_range = full;
    out->bt709 = bt709;
    return 0;
}

void gf_swdec_size(const gf_swdec *d, int *width, int *height) {
    *width = d->have ? d->frame->width : 0;
    *height = d->have ? d->frame->height : 0;
}

void gf_swdec_flush(gf_swdec *d) {
    if (d->ctx) d->f.flush(d->ctx);
    if (d->frame) d->f.frame_unref(d->frame);
    d->have = 0;
}

void gf_swdec_close(gf_swdec *d) {
    if (!d) return;
    if (d->ctx && d->f.free_ctx) d->f.free_ctx(&d->ctx);
    if (d->pkt && d->f.pkt_free) {
        d->pkt->data = NULL;                /* the unit was the caller's */
        d->pkt->size = 0;
        d->f.pkt_free(&d->pkt);
    }
    if (d->frame && d->f.frame_free) d->f.frame_free(&d->frame);
    if (d->spare && d->f.frame_free) d->f.frame_free(&d->spare);
    /* no dlclose: the libraries start no threads here, but a second open in the same process is then free and a leak of two mappings is not worth a
     * crash in a destructor that ran after main */
    free(d);
}
