#define _GNU_SOURCE
#include "xcode.h"

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/uio.h>
#include <time.h>
#include <unistd.h>

#include "annexb.h"
#include "hma.h"
#include "scale.h"
#include "vdec.h"
#include "venc.h"
#include "watchdog.h"
#include "xcode_proto.h"

#define POLL_MS 50
#define STATS_S 1.0
#define DEFAULT_OUT_SIZE (1536u * 1024u)
#define RING 256 /* more than the pictures a decoder holds back */
#define INITIAL_IN (256u * 1024u)
#define READ_CHUNK (64u * 1024u)
/* The backlog the child lets build up unread input to: a whole record always fits, and a decoder that is slower than the engine
 * sends is felt by the engine through the pipe filling up rather than by memory growing here. */
#define IN_MAX (XC_HEADER_LEN + XC_MAX_PAYLOAD + READ_CHUNK)
#define HMA_POOL "photofb" /* LG's own encoder plugin allocates from it; the picture buffers and the scaled picture */
/* What a step returns when the engine has gone (a write failed): the job ends cleanly, nothing to report to anyone. */
#define GONE (-1)

struct xcode_limits xcode_limits = { .first_stall_s = 5.0, .stall_s = 3.0, .drain_s = 3.0 };

static volatile sig_atomic_t g_stop;
static void on_term(int sig)
{
    (void)sig;
    g_stop = 1;
}

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

struct xc {
    const struct sysops *sys;
    int in, out;
    struct xc_cfg cfg;

    struct vdec d;
    struct venc v;
    int enc_up;
    struct scaler *sc;
    struct hma_buf sbuf; /* the scaled picture the encoder reads */
    uint32_t pw, ph;     /* the decoded size, fixed by the first picture */
    uint32_t ow, oh;     /* what the encoder is given */
    unsigned level;      /* the H.264 level the stream's SPS is made to say */

    uint8_t *in_buf;
    size_t in_cap, in_len, in_off;
    int eof, pending, drained, last_seen;
    double t_drain;

    uint8_t *key_buf; /* a copy of an IDR unit, whose SPS gets its level corrected; the encoder's ring is not ours to write */
    size_t key_cap;

    uint64_t seq, pic_no;
    uint64_t pts[RING];
    double t_feed[RING];
    int want_idr;

    uint64_t fed, encoded, dropped;
    double first_feed, last_pic, last_stats, dec_ms, enc_ms;
};

static void logf_(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void logf_(const char *fmt, ...)
{
    char line[300];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof(line), fmt, ap);
    va_end(ap);
    fprintf(stderr, "xcode: %s\n", line);
}

/* Writes every iovec byte; EINTR ends it when the job was told to stop. */
static int write_all(int fd, struct iovec *iov, int n)
{
    while (n > 0 && iov[0].iov_len == 0) {
        iov++;
        n--;
    }
    while (n > 0) {
        ssize_t w = writev(fd, iov, n);
        if (w < 0) {
            if (errno == EINTR && !g_stop)
                continue;
            return -1;
        }
        size_t left = (size_t)w;
        while (n > 0 && left >= iov[0].iov_len) {
            left -= iov[0].iov_len;
            iov++;
            n--;
        }
        if (n > 0) {
            iov[0].iov_base = (uint8_t *)iov[0].iov_base + left;
            iov[0].iov_len -= left;
        }
        while (n > 0 && iov[0].iov_len == 0) {
            iov++;
            n--;
        }
    }
    return 0;
}

static int emit(struct xc *x, const char *magic, uint32_t flags, uint64_t pts, uint32_t aux, const void *p1, size_t n1, const void *p2, size_t n2)
{
    struct xc_head h;
    memcpy(h.magic, magic, 4);
    h.flags = flags;
    h.pts = pts;
    h.size = (uint32_t)(n1 + n2);
    h.aux = aux;
    struct iovec iov[3] = { { &h, sizeof(h) }, { (void *)p1, n1 }, { (void *)p2, n2 } };
    return write_all(x->out, iov, 3);
}

static int event(struct xc *x, uint32_t kind, uint32_t a, uint32_t b, uint32_t c, uint64_t ex, uint64_t ey, const char *text)
{
    struct xc_event e = { kind, a, b, c, ex, ey };
    return emit(x, XC_EVENT, 0, 0, 0, &e, sizeof(e), text, text ? strlen(text) : 0);
}

/* Reports a failure to the engine and to the log; returns the exit status for the caller to pass up. */
static int fail(struct xc *x, int code, const char *what, const char *err)
{
    char msg[256];
    snprintf(msg, sizeof(msg), "%s%s%s", what, err && *err ? ": " : "", err ? err : "");
    logf_("%s", msg);
    event(x, XC_EVT_ERROR, (uint32_t)code, 0, 0, 0, 0, msg);
    return code;
}

/* The lowest level whose limits hold the stream: the encoder always writes 5.1 into the SPS, and a phone's decoder is
 * entitled to refuse what it declares above its own (iOS stops at 4.2, many Androids at 4.0). */
static unsigned h264_level(uint32_t w, uint32_t h, uint32_t fps)
{
    static const struct {
        unsigned level;
        unsigned long frame_mbs, mbs_per_s;
    } limits[] = { { 30, 1620, 40500 }, { 31, 3600, 108000 }, { 32, 5120, 216000 }, { 40, 8192, 245760 }, { 42, 8704, 522240 } };
    unsigned long mbs = (unsigned long)((w + 15) / 16) * ((h + 15) / 16);
    for (size_t i = 0; i < sizeof(limits) / sizeof(limits[0]); i++)
        if (mbs <= limits[i].frame_mbs && mbs * fps <= limits[i].mbs_per_s)
            return limits[i].level;
    return 51;
}

/* level_idc is the fourth byte of an SPS: NAL header, profile_idc, constraint flags, level_idc. */
static void patch_level(uint8_t *au, size_t len, unsigned level)
{
    size_t pos = 0;
    struct annexb_nal nal;
    while (annexb_next(au, len, &pos, &nal)) {
        if (nal.type == 7 && nal.len >= 4) {
            ((uint8_t *)nal.p)[3] = (uint8_t)level;
            return;
        }
        if (nal.type != 6 && nal.type != 9 && nal.type != 8)
            return;
    }
}

/* Room for `want` bytes counted from the front of the unread input. */
static int reserve_in(struct xc *x, size_t want)
{
    if (x->in_off + want <= x->in_cap)
        return 0;
    if (x->in_off > 0) {
        memmove(x->in_buf, x->in_buf + x->in_off, x->in_len - x->in_off);
        x->in_len -= x->in_off;
        x->in_off = 0;
        if (want <= x->in_cap)
            return 0;
    }
    size_t cap = x->in_cap ? x->in_cap : INITIAL_IN;
    while (cap < want)
        cap *= 2;
    uint8_t *p = realloc(x->in_buf, cap);
    if (!p)
        return -1;
    x->in_buf = p;
    x->in_cap = cap;
    return 0;
}

/* Reads what the engine has sent without waiting. */
static int read_input(struct xc *x)
{
    if (reserve_in(x, x->in_len - x->in_off + READ_CHUNK) < 0)
        return fail(x, XC_EXIT_SETUP, "no memory for the input", NULL);
    ssize_t r = read(x->in, x->in_buf + x->in_len, x->in_cap - x->in_len);
    if (r > 0)
        x->in_len += (size_t)r;
    else if (r == 0 || (errno != EAGAIN && errno != EINTR))
        x->eof = 1;
    return 0;
}

/* One whole record at the front of the input: 1 with *h and *payload set, 0 when more bytes are needed, or a negative
 * exit status. */
static int next_record(struct xc *x, struct xc_head *h, const uint8_t **payload)
{
    if (x->in_len - x->in_off < XC_HEADER_LEN)
        return 0;
    memcpy(h, x->in_buf + x->in_off, sizeof(*h));
    if (h->size > XC_MAX_PAYLOAD)
        return -fail(x, XC_EXIT_CONFIG, "a record is larger than the protocol allows", NULL);
    if (x->in_len - x->in_off < XC_HEADER_LEN + (size_t)h->size) {
        if (reserve_in(x, XC_HEADER_LEN + (size_t)h->size) < 0)
            return -fail(x, XC_EXIT_SETUP, "no memory for an access unit", NULL);
        return 0;
    }
    *payload = x->in_buf + x->in_off + XC_HEADER_LEN;
    return 1;
}

static void consume(struct xc *x, const struct xc_head *h)
{
    x->in_off += XC_HEADER_LEN + h->size;
    if (x->in_off == x->in_len)
        x->in_off = x->in_len = 0;
}

/* 0: the first record was a valid XCFG and *cfg is set, else an exit status. Waits for it. */
static int read_config(struct xc *x)
{
    double until = now_s() + 5.0;
    for (;;) {
        struct xc_head h;
        const uint8_t *payload;
        int r = next_record(x, &h, &payload);
        if (r < 0)
            return -r;
        if (r > 0) {
            if (memcmp(h.magic, XC_CONFIG, 4) || h.size != sizeof(struct xc_cfg))
                return fail(x, XC_EXIT_CONFIG, "the first record is not a configuration", NULL);
            memcpy(&x->cfg, payload, sizeof(x->cfg));
            consume(x, &h);
            return 0;
        }
        if (x->eof || g_stop)
            return fail(x, XC_EXIT_CONFIG, "the input ended before a configuration", NULL);
        double left = until - now_s();
        if (left <= 0)
            return fail(x, XC_EXIT_CONFIG, "no configuration within 5 s", NULL);
        struct pollfd p = { x->in, POLLIN, 0 };
        x->sys->poll(&p, 1, (int)(left * 1000) + 1);
        int rc = read_input(x);
        if (rc)
            return rc;
    }
}

static int check_config(struct xc *x)
{
    struct xc_cfg *c = &x->cfg;
    if (c->version != XC_VERSION)
        return fail(x, XC_EXIT_CONFIG, "unknown protocol version", NULL);
    if (c->codec != XC_CODEC_MPEG2 && c->codec != XC_CODEC_H264)
        return fail(x, XC_EXIT_CONFIG, "unknown codec", NULL);
    if (!c->src_w)
        c->src_w = 1920;
    if (!c->src_h)
        c->src_h = 1088;
    c->src_h = (c->src_h + 15) & ~15u;
    if (c->src_w < 16 || c->src_w > 1920 || c->src_h > 1088)
        return fail(x, XC_EXIT_CONFIG, "source size out of range", NULL);
    if ((!c->out_w) != (!c->out_h) || (c->out_w & 1) || (c->out_h & 1) || c->out_w > 1920 || c->out_h > 1080)
        return fail(x, XC_EXIT_CONFIG, "output size out of range", NULL);
    if (!c->keep_every)
        c->keep_every = 1;
    if (!c->fps)
        c->fps = 30;
    if (c->kbps < 64 || c->kbps > 20000)
        return fail(x, XC_EXIT_CONFIG, "bit rate out of range", NULL);
    if (!c->ncap)
        c->ncap = 6;
    if (!c->nout)
        c->nout = 4;
    if (!c->out_size)
        c->out_size = DEFAULT_OUT_SIZE;
    return 0;
}

static int setup(struct xc *x)
{
    char err[160];
    struct vdec_cfg vc = {
        .codec = x->cfg.codec == XC_CODEC_H264 ? V4L2_PIX_FMT_H264 : V4L2_PIX_FMT_MPEG2,
        .max_w = x->cfg.src_w,
        .max_h = x->cfg.src_h,
        .nout = x->cfg.nout,
        .ncap = x->cfg.ncap,
        .out_size = x->cfg.out_size,
        .pool = HMA_POOL,
    };
    if (vdec_open(&x->d, x->sys, err, sizeof(err)) < 0)
        return fail(x, XC_EXIT_SETUP, "decoder", err);
    if (vdec_start(&x->d, &vc, err, sizeof(err)) < 0)
        return fail(x, XC_EXIT_SETUP, "decoder", err);
    struct xc_ready rd = { XC_VERSION, x->cfg.src_w, x->cfg.src_h, x->cfg.out_size, x->cfg.ncap, (uint32_t)((size_t)x->d.capsize * x->d.ncap) };
    if (emit(x, XC_READY, 0, 0, 0, &rd, sizeof(rd), NULL, 0) < 0)
        return GONE;
    return 0;
}

/* The picture size is known: the scaler, its buffer and the encoder. */
static int start_encoder(struct xc *x, uint32_t w, uint32_t h)
{
    char err[160];
    x->pw = w;
    x->ph = h;
    x->ow = x->cfg.out_w && x->cfg.out_w < w ? x->cfg.out_w : w;
    x->oh = x->cfg.out_h && x->cfg.out_h < h ? x->cfg.out_h : h;
    if (x->ow != w || x->oh != h) {
        x->sc = scale_new(w, h, x->ow, x->oh);
        if (!x->sc)
            return fail(x, XC_EXIT_SETUP, "a picture this size cannot be scaled to the output size", NULL);
        if (hma_alloc(&x->sbuf, x->sys, HMA_POOL, (size_t)x->ow * x->oh * 3 / 2, 4096, 0, err, sizeof(err)) < 0)
            return fail(x, XC_EXIT_SETUP, "scaled picture buffer", err);
    }
    struct venc_cfg ec = { .width = x->ow, .height = x->oh, .fps = x->cfg.fps, .gop = x->cfg.gop, .kbps = x->cfg.kbps, .fixed_qp = 26,
                           .profile = x->cfg.profile, .cbr = (x->cfg.flags & XC_CFG_CBR) != 0 };
    if (venc_open(&x->v, x->sys, err, sizeof(err)) < 0 || venc_create(&x->v, &ec, err, sizeof(err)) < 0)
        return fail(x, XC_EXIT_ENCODER, "encoder", err);
    x->enc_up = 1;
    x->level = h264_level(x->ow, x->oh, x->cfg.fps);
    return 0;
}

static int encode_picture(struct xc *x, const struct vdec_pic *pic)
{
    char err[160];
    uint32_t py = pic->paddr_y, pc = pic->paddr_c;
    if (x->sc) {
        size_t luma = (size_t)x->ow * x->oh;
        scale_run(x->sc, pic->y, pic->stride, pic->c, pic->stride, x->sbuf.vaddr, x->ow, x->sbuf.vaddr + luma, x->ow);
        py = x->sbuf.paddr;
        pc = x->sbuf.paddr + (uint32_t)luma;
    }
    if (x->want_idr) {
        x->want_idr = 0;
        if (venc_force_idr(&x->v, err, sizeof(err)) < 0)
            return fail(x, XC_EXIT_ENCODER, "encoder", err);
    }
    struct venc_frame f;
    double t0 = now_s();
    if (venc_encode(&x->v, py, pc, &f, err, sizeof(err)) < 0)
        return fail(x, XC_EXIT_ENCODER, "encoder", err);
    double t1 = now_s();
    x->enc_ms = (t1 - t0) * 1e3;
    x->encoded++;
    uint64_t pts = x->pts[pic->seq % RING];
    uint32_t tenths = (uint32_t)(x->enc_ms * 10.0 + 0.5);
    if (!f.size)
        return 0;
    int rc;
    if (f.keyframe) {
        if (x->key_cap < f.size) {
            uint8_t *p = realloc(x->key_buf, f.size);
            if (!p)
                return fail(x, XC_EXIT_SETUP, "no memory for a keyframe", NULL);
            x->key_buf = p;
            x->key_cap = f.size;
        }
        memcpy(x->key_buf, f.data, f.size);
        patch_level(x->key_buf, f.size, x->level);
        rc = emit(x, XC_OUT, XC_FLAG_KEY, pts, tenths, x->key_buf, f.size, NULL, 0);
    } else {
        rc = emit(x, XC_OUT, 0, pts, tenths, f.data, f.size, NULL, 0);
    }
    return rc < 0 ? GONE : 0;
}

/* GONE when the engine has gone, else 0 or an exit status. */
static int on_picture(struct xc *x, const struct vdec_pic *pic)
{
    double t = now_s();
    x->last_pic = t;
    x->dec_ms = (t - x->t_feed[pic->seq % RING]) * 1e3;
    if (!x->pw) {
        int rc = start_encoder(x, pic->width, pic->height);
        if (rc)
            return rc;
        event(x, XC_EVT_FIRST, pic->width, pic->height, (uint32_t)((t - x->first_feed) * 1e3), 0, 0, NULL);
    } else if (pic->width != x->pw || pic->height != x->ph) {
        return fail(x, XC_EXIT_SHAPE, "the stream changed picture size", NULL);
    }
    uint64_t n = x->pic_no++;
    if (pic->error) {
        x->dropped++;
        return 0;
    }
    if (n % x->cfg.keep_every)
        return 0;
    return encode_picture(x, pic);
}

static int pictures(struct xc *x)
{
    char err[160];
    for (;;) {
        struct vdec_pic pic;
        int r = vdec_get_picture(&x->d, &pic, 0, err, sizeof(err));
        if (r < 0)
            return fail(x, XC_EXIT_DECODER, "decoder", err);
        if (r == 0)
            return 0;
        int rc = 0;
        if (pic.last)
            x->last_seen = 1;
        else
            rc = on_picture(x, &pic);
        if (vdec_put_picture(&x->d, pic.index, err, sizeof(err)) < 0 && !rc)
            rc = fail(x, XC_EXIT_DECODER, "decoder", err);
        if (rc)
            return rc;
        if (pic.last)
            return 0;
    }
}

/* Hands the engine's records to the decoder, in order, until one of them has to wait for a free bitstream buffer. */
static int pump(struct xc *x)
{
    char err[160];
    x->pending = 0;
    for (;;) {
        struct xc_head h;
        const uint8_t *payload;
        int r = next_record(x, &h, &payload);
        if (r < 0)
            return -r;
        if (r == 0)
            return 0;
        if (!memcmp(h.magic, XC_AU, 4)) {
            if (h.size) {
                if (h.size > x->d.cfg.out_size)
                    return fail(x, XC_EXIT_DECODER, "an access unit is larger than a bitstream buffer", NULL);
                int fr = vdec_feed(&x->d, payload, h.size, x->seq, 0, err, sizeof(err));
                if (fr < 0)
                    return fail(x, XC_EXIT_DECODER, "decoder", err);
                if (fr == 1) {
                    x->pending = 1;
                    return 0;
                }
                double t = now_s();
                x->pts[x->seq % RING] = h.pts;
                x->t_feed[x->seq % RING] = t;
                if (!x->first_feed)
                    x->first_feed = t;
                x->seq++;
                x->fed++;
            }
        } else if (!memcmp(h.magic, XC_IDR, 4)) {
            x->want_idr = 1;
        } else if (!memcmp(h.magic, XC_BITRATE, 4)) {
            if (h.aux >= 64 && h.aux <= 20000) {
                x->cfg.kbps = h.aux;
                if (x->enc_up && venc_set_bitrate(&x->v, h.aux, err, sizeof(err)) < 0)
                    return fail(x, XC_EXIT_ENCODER, "encoder", err);
            }
        }
        consume(x, &h);
    }
}

static int stats(struct xc *x, double t)
{
    if (t - x->last_stats < STATS_S)
        return 0;
    x->last_stats = t;
    return event(x, XC_EVT_STATS, (uint32_t)x->dropped, (uint32_t)(x->dec_ms + 0.5), (uint32_t)(x->enc_ms * 10.0 + 0.5), x->fed, x->encoded, NULL) < 0 ? -1 : 0;
}

static void cleanup(struct xc *x)
{
    venc_close(&x->v);
    scale_free(x->sc);
    hma_free(&x->sbuf);
    vdec_close(&x->d);
    free(x->in_buf);
    free(x->key_buf);
}

static int run(struct xc *x)
{
    char err[160];
    int rc = read_config(x);
    if (!rc)
        rc = check_config(x);
    if (!rc)
        rc = setup(x);
    x->last_stats = now_s();
    while (rc == 0 && !g_stop) {
        int want_in = !x->eof && x->in_len - x->in_off < IN_MAX;
        struct pollfd p[2] = { { x->in, want_in ? POLLIN : 0, 0 }, { x->d.fd, POLLIN | POLLPRI | (x->pending ? POLLOUT : 0), 0 } };
        int pr = x->sys->poll(p, 2, POLL_MS);
        if (pr < 0 && errno != EINTR) {
            rc = fail(x, XC_EXIT_DECODER, "poll", strerror(errno));
            break;
        }
        if (g_stop)
            break;
        if (want_in && (p[0].revents & (POLLIN | POLLHUP | POLLERR))) {
            rc = read_input(x);
            if (rc)
                break;
        }
        rc = pump(x);
        if (rc)
            break;
        rc = pictures(x);
        if (rc)
            break;
        double t = now_s();
        if (stats(x, t) < 0)
            break;
        if (x->first_feed && !x->pw && t - x->first_feed > xcode_limits.first_stall_s) {
            rc = fail(x, XC_EXIT_STALLED, "no picture from the first access units", NULL);
            break;
        }
        if (x->pw && x->fed > x->pic_no + 8 && t - x->last_pic > xcode_limits.stall_s) {
            rc = fail(x, XC_EXIT_STALLED, "no picture for the stall limit", NULL);
            break;
        }
        if (x->eof && !x->pending) {
            if (!x->drained) {
                if (vdec_drain(&x->d, err, sizeof(err)) < 0) {
                    logf_("drain: %s", err);
                    break;
                }
                x->drained = 1;
                x->t_drain = t;
            }
            if (x->last_seen || t - x->t_drain > xcode_limits.drain_s)
                break;
        }
    }
    return rc == GONE ? XC_EXIT_OK : rc;
}

int xcode_run(const struct sysops *sys, int in_fd, int out_fd)
{
    struct xc *x = calloc(1, sizeof(*x));
    if (!x)
        return XC_EXIT_SETUP;
    x->sys = sys;
    x->in = in_fd;
    x->out = out_fd;
    x->sbuf.fd = -1;
    x->v.fd = -1;
    int rc = run(x);
    cleanup(x);
    free(x);
    return rc;
}

void xcode_signals(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_term; /* no SA_RESTART: a blocked write or poll must return so the job can end */
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);
}

int xcode_main(void)
{
    struct rlimit nocore = { 0, 0 };
    setrlimit(RLIMIT_CORE, &nocore);
    xcode_signals();
    prctl(PR_SET_PDEATHSIG, SIGTERM);
    wd_install();
    int fl = fcntl(0, F_GETFL);
    if (fl >= 0)
        fcntl(0, F_SETFL, fl | O_NONBLOCK);
    int rc = xcode_run(&sysops_real, 0, 1);
    logf_("exit %d", rc);
    return rc;
}
