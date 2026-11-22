#include "hevc_source.h"

#include <libavcodec/avcodec.h>
#include <libavutil/buffer.h>
#include <libavutil/frame.h>
#include <libavutil/pixfmt.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gf_annexb.h"

enum { MAX_AU = 128, MAX_SEEN = 256, WIDTH = 128, HEIGHT = 64, TAG_CELL = 8 };

struct hevc_surface {
    AVFrame *frame;
    uint64_t id;
    int64_t pts;
    size_t display;
    uint64_t sum;
    int linesize0;
    const uint8_t *plane[3];
    int live, freed, retired, planes_freed;
};

typedef struct {
    hevc_source *src;
    AVBufferRef *orig;                  /* the pool's reference, given back when the last user of the wrapper lets go */
    hevc_surface *owner;
} tracked;

typedef struct {
    AVCodecContext *ctx;
    AVPacket *pkt;
    size_t fed;
    int draining;
} decoder;

struct hevc_source {
    uint8_t *data;
    size_t size;
    gf_unit units[MAX_AU];
    size_t nau;
    int au_display[MAX_AU];
    uint16_t exp_tag[MAX_AU];           /* by display index */
    uint64_t ref_sum[MAX_AU];           /* by display index */
    int64_t pts[MAX_AU];                /* by display index */
    decoder dec;
    size_t out_count;
    size_t lookahead;
    hevc_surface surf[MAX_AU];
    size_t nsurf;
    uintptr_t seen[MAX_SEEN];
    size_t nseen;
    hevc_oracle o;
    char err[192];
};

static void seterr(hevc_source *s, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(s->err, sizeof s->err, fmt, ap);
    va_end(ap);
}

static uint8_t *slurp(const char *path, size_t *n) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *buf = len > 0 ? malloc((size_t)len) : NULL;
    if (buf && fread(buf, 1, (size_t)len, f) != (size_t)len) {
        free(buf);
        buf = NULL;
    }
    fclose(f);
    *n = buf ? (size_t)len : 0;
    return buf;
}

static uint64_t luma_hash(const uint8_t *plane, int linesize) {
    uint64_t h = 14695981039346656037ull;
    for (int y = 0; y < HEIGHT; y++) {
        const uint8_t *row = plane + (size_t)y * (size_t)linesize;
        for (int i = 0; i < WIDTH * 2; i++) h = (h ^ row[i]) * 1099511628211ull;
    }
    return h;
}

/* The 16 bit tag: bit c is set when the middle of cell c (8x8 luma samples, row-major from the top left) is bright. */
static int luma_tag(const uint8_t *plane, int linesize) {
    int tag = 0;
    for (int c = 0; c < 16; c++) {
        unsigned sum = 0;
        for (int y = (c / 8) * TAG_CELL + 2; y < (c / 8) * TAG_CELL + 6; y++)
            for (int x = (c % 8) * TAG_CELL + 2; x < (c % 8) * TAG_CELL + 6; x++) {
                const uint8_t *p = plane + (size_t)y * (size_t)linesize + 2 * (size_t)x;
                sum += (unsigned)(p[0] | (p[1] << 8));
            }
        if (sum > 16u * 502u) tag |= 1 << c;
    }
    return tag;
}

/* ---- the oracle's allocator hook ---------------------------------------------------------------------------------------------- */

static void tracked_free(void *opaque, uint8_t *data) {
    (void)data;
    tracked *t = opaque;
    hevc_source *s = t->src;
    s->o.freed++;
    if (t->owner) {
        t->owner->planes_freed++;
        if (t->owner->live) s->o.freed_while_live++;
    }
    av_buffer_unref(&t->orig);
    free(t);
}

static int tracked_get_buffer2(AVCodecContext *c, AVFrame *f, int flags) {
    hevc_source *s = c->opaque;
    int r = avcodec_default_get_buffer2(c, f, flags);
    if (r < 0) return r;
    s->o.handouts++;
    size_t i = 0;
    while (i < s->nseen && s->seen[i] != (uintptr_t)f->data[0]) i++;
    if (i == s->nseen && s->nseen < MAX_SEEN) {
        s->seen[s->nseen++] = (uintptr_t)f->data[0];
        s->o.distinct++;
    }
    for (int p = 0; p < AV_NUM_DATA_POINTERS && f->buf[p]; p++) {
        for (size_t k = 0; k < s->nsurf; k++)
            for (int q = 0; s->surf[k].live && q < 3; q++)
                if (s->surf[k].plane[q] == f->data[p]) s->o.recycled_live++;
        tracked *t = calloc(1, sizeof *t);
        AVBufferRef *w = t ? av_buffer_create(f->buf[p]->data, f->buf[p]->size, tracked_free, t, 0) : NULL;
        if (!w) {
            free(t);
            av_frame_unref(f);
            return AVERROR(ENOMEM);
        }
        t->src = s;
        t->orig = f->buf[p];
        f->buf[p] = w;
        s->o.allocated++;
    }
    return 0;
}

/* ---- decoding ----------------------------------------------------------------------------------------------------------------- */

static int open_decoder(hevc_source *s, decoder *d, int tracked_alloc) {
    const AVCodec *codec = avcodec_find_decoder_by_name("hevc");
    if (!codec) return -1;
    d->ctx = avcodec_alloc_context3(codec);
    d->pkt = av_packet_alloc();
    d->fed = 0;
    d->draining = 0;
    if (!d->ctx || !d->pkt) return -1;
    d->ctx->thread_count = 1;
    if (tracked_alloc) {
        d->ctx->opaque = s;
        d->ctx->get_buffer2 = tracked_get_buffer2;
    }
    return avcodec_open2(d->ctx, codec, NULL) < 0 ? -1 : 0;
}

static void close_decoder(decoder *d) {
    avcodec_free_context(&d->ctx);
    av_packet_free(&d->pkt);
}

/* One decoded picture in display order: 0, 1 at the end of the stream, -1 on an error. Access units go in one at a time in decode order, with the
 * timestamp of their display index and no decode timestamp, as an RTP depacketiser leaves them. */
static int decode_next(hevc_source *s, decoder *d, AVFrame *out) {
    for (;;) {
        int r = avcodec_receive_frame(d->ctx, out);
        if (r == 0) return 0;
        if (r == AVERROR_EOF) return 1;
        if (r != AVERROR(EAGAIN)) return -1;
        if (d->fed < s->nau) {
            d->pkt->data = s->data + s->units[d->fed].offset;
            d->pkt->size = (int)s->units[d->fed].size;
            d->pkt->pts = s->pts[s->au_display[d->fed]];
            d->pkt->dts = AV_NOPTS_VALUE;
            d->fed++;
            if (avcodec_send_packet(d->ctx, d->pkt) < 0) return -1;
        } else if (!d->draining) {
            d->draining = 1;
            if (avcodec_send_packet(d->ctx, NULL) < 0) return -1;
        } else {
            return -1;
        }
    }
}

static int check_picture(hevc_source *s, const AVFrame *f, size_t k, int *tag, uint64_t *sum) {
    if (k >= s->nau) return seterr(s, "more pictures than access units"), -1;
    if (f->format != AV_PIX_FMT_YUV420P10LE || f->width != WIDTH || f->height != HEIGHT)
        return seterr(s, "picture %zu: unexpected format %d %dx%d", k, f->format, f->width, f->height), -1;
    if (f->pts != s->pts[k]) return seterr(s, "picture %zu: pts %lld, want %lld", k, (long long)f->pts, (long long)s->pts[k]), -1;
    *tag = luma_tag(f->data[0], f->linesize[0]);
    *sum = luma_hash(f->data[0], f->linesize[0]);
    if (*tag != s->exp_tag[k]) return seterr(s, "picture %zu: tag %#x, want %#x", k, *tag, s->exp_tag[k]), -1;
    return 0;
}

hevc_source *hevc_source_open(const char *stream_path, const char *order_path, const int64_t *pts, size_t npts, char *err, size_t errcap) {
    hevc_source *s = calloc(1, sizeof *s);
    if (!s) return NULL;
    int ok = 0;
    do {
        s->data = slurp(stream_path, &s->size);
        if (!s->data) { seterr(s, "cannot read %s", stream_path); break; }
        s->nau = gf_split(s->data, s->size, GF_CODEC_H265, s->units, MAX_AU);
        if (s->nau == 0 || s->nau > MAX_AU) { seterr(s, "%zu access units", s->nau); break; }
        FILE *f = fopen(order_path, "r");
        if (!f) { seterr(s, "cannot read %s", order_path); break; }
        size_t lines = 0;
        int display, tag;
        int bad = 0;
        while (fscanf(f, "%d %d", &display, &tag) == 2) {
            if (lines >= s->nau || display < 0 || (size_t)display >= s->nau) { bad = 1; break; }
            s->au_display[lines++] = display;
            s->exp_tag[display] = (uint16_t)tag;
        }
        fclose(f);
        if (bad || lines != s->nau) { seterr(s, "%s lists %zu access units, the stream has %zu", order_path, lines, s->nau); break; }
        for (size_t d = 0; d < s->nau; d++) s->pts[d] = d < npts ? pts[d] : pts[npts - 1] + (int64_t)(d - npts + 1);

        /* The reference decode: the luma every later decode of a picture must reproduce. */
        decoder ref = {0};
        if (open_decoder(s, &ref, 0) != 0) { seterr(s, "cannot open the hevc decoder"); close_decoder(&ref); break; }
        AVFrame *fr = av_frame_alloc();
        size_t k = 0;
        int r;
        while ((r = decode_next(s, &ref, fr)) == 0) {
            int t;
            if (check_picture(s, fr, k, &t, &s->ref_sum[k]) != 0) { r = -1; break; }
            av_frame_unref(fr);
            k++;
        }
        av_frame_free(&fr);
        close_decoder(&ref);
        if (r < 0) { if (!s->err[0]) seterr(s, "the reference decode failed"); break; }
        if (k != s->nau) { seterr(s, "the reference decode gave %zu of %zu pictures", k, s->nau); break; }

        if (open_decoder(s, &s->dec, 1) != 0) { seterr(s, "cannot open the hevc decoder"); break; }
        ok = 1;
    } while (0);
    if (!ok) {
        if (err && errcap) snprintf(err, errcap, "%s", s->err);
        close_decoder(&s->dec);
        free(s->data);
        free(s);
        return NULL;
    }
    return s;
}

int hevc_source_next(hevc_source *s, hevc_surface **out) {
    AVFrame *f = av_frame_alloc();
    if (!f) return seterr(s, "out of memory"), -1;
    int r = decode_next(s, &s->dec, f);
    if (r != 0) {
        av_frame_free(&f);
        if (r < 0) seterr(s, "decode failed at picture %zu", s->out_count);
        return r;
    }
    size_t k = s->out_count;
    int tag;
    uint64_t sum;
    if (check_picture(s, f, k, &tag, &sum) != 0 || sum != s->ref_sum[k] || s->nsurf == MAX_AU || !f->buf[0] || !f->buf[1] || !f->buf[2]) {
        if (!s->err[0]) seterr(s, "picture %zu: luma differs from the reference decode or no surface slot", k);
        av_frame_free(&f);
        return -1;
    }
    hevc_surface *sf = &s->surf[s->nsurf++];
    memset(sf, 0, sizeof *sf);
    sf->frame = f;
    sf->id = (uint64_t)tag;
    sf->pts = f->pts;
    sf->display = k;
    sf->sum = sum;
    sf->linesize0 = f->linesize[0];
    sf->live = 1;
    for (int p = 0; p < 3; p++) {
        sf->plane[p] = f->data[p];
        ((tracked *)av_buffer_get_opaque(f->buf[p]))->owner = sf;
    }
    s->o.pictures++;
    s->o.live++;
    size_t ahead = s->dec.fed - (k + 1);
    if (ahead > s->lookahead) s->lookahead = ahead;
    s->out_count++;
    *out = sf;
    return 0;
}

const char *hevc_source_error(const hevc_source *s) { return s->err; }
size_t hevc_source_pictures(const hevc_source *s) { return s->nau; }
size_t hevc_source_max_lookahead(const hevc_source *s) { return s->lookahead; }
int hevc_source_au_display(const hevc_source *s, size_t au) { return s->au_display[au]; }
const hevc_oracle *hevc_source_oracle(const hevc_source *s) { return &s->o; }

uint64_t hevc_surface_id(const hevc_surface *f) { return f->id; }
int64_t hevc_surface_pts(const hevc_surface *f) { return f->pts; }
size_t hevc_surface_display(const hevc_surface *f) { return f->display; }
int hevc_surface_refs(const hevc_surface *f) { return f->frame ? av_buffer_get_ref_count(f->frame->buf[0]) : 0; }
int hevc_surface_planes_freed(const hevc_surface *f) { return f->planes_freed; }

int hevc_surface_read(hevc_source *s, hevc_surface *f) {
    if (f->freed) s->o.read_after_free++;
    if (luma_hash(f->plane[0], f->linesize0) == f->sum) return 0;
    s->o.integrity_failures++;
    return -1;
}

/* Drops the frame's references and checks that each plane buffer lost exactly one. */
static void release_frame(hevc_source *s, hevc_surface *f) {
    AVBufferRef *watch[3];
    int before[3];
    for (int p = 0; p < 3; p++) {
        watch[p] = av_buffer_ref(f->frame->buf[p]);
        before[p] = watch[p] ? av_buffer_get_ref_count(watch[p]) : 0;
    }
    av_frame_free(&f->frame);
    for (int p = 0; p < 3; p++) {
        if (!watch[p] || av_buffer_get_ref_count(watch[p]) != before[p] - 1) s->o.release_ref_errors++;
        av_buffer_unref(&watch[p]);
    }
    f->freed = 1;
}

void hevc_surface_retire(hevc_source *s, hevc_surface *f) {
    if (f->retired) {
        s->o.double_retire++;
        return;
    }
    f->retired = 1;
    f->live = 0;
    s->o.live--;
    s->o.retired++;
    if (!f->freed) release_frame(s, f);
}

void hevc_surface_drop_refs_early(hevc_source *s, hevc_surface *f) {
    if (!f->freed) release_frame(s, f);
}

int64_t hevc_source_close(hevc_source *s, hevc_oracle *final) {
    close_decoder(&s->dec);
    s->o.outstanding = (int64_t)(s->o.allocated - s->o.freed);
    if (final) *final = s->o;
    int64_t outstanding = s->o.outstanding;
    for (size_t i = 0; i < s->nsurf; i++) av_frame_free(&s->surf[i].frame);
    free(s->data);
    free(s);
    return outstanding;
}
