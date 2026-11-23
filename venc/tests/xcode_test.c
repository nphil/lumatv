/* Host tests of the transcode child (xcode.c) and its CPU scaler (scale.c) against the fake kernel (tests/fake_sys.c):
 * the scaler on its own, then xcode_run over pipes in a forked process per case, so the exit status and any device the
 * job left open show. Run by test.sh. */
#define _GNU_SOURCE
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../scale.h"
#include "../sysops.h"
#include "../venc_abi.h"
#include "../xcode.h"
#include "../xcode_proto.h"

size_t fake_vdec_au(uint8_t *out, uint32_t w, uint32_t h, uint32_t id);
void fake_vdec_pattern(uint8_t *y, uint8_t *c, uint32_t w, uint32_t h, uint32_t id);
uint32_t fake_nv12_hash(const uint8_t *y, const uint8_t *c, uint32_t w, uint32_t h);
int fake_open_fds(void);

static int failures;
#define CHECK(cond, ...) do { if (!(cond)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* ---- the scaler ---- */

/* the area-weighted mean of the source over the destination pixel's footprint, in floating point */
static double area_mean(const uint8_t *p, uint32_t stride, unsigned step, uint32_t sw, uint32_t sh, uint32_t dw, uint32_t dh, uint32_t dx, uint32_t dy)
{
    double x0 = (double)dx * sw / dw, x1 = (double)(dx + 1) * sw / dw;
    double y0 = (double)dy * sh / dh, y1 = (double)(dy + 1) * sh / dh;
    double sum = 0;
    for (uint32_t j = (uint32_t)y0; j < sh && j < y1; j++) {
        double wy = fmin(j + 1.0, y1) - fmax((double)j, y0);
        for (uint32_t i = (uint32_t)x0; i < sw && i < x1; i++) {
            double wx = fmin(i + 1.0, x1) - fmax((double)i, x0);
            sum += wx * wy * p[(size_t)j * stride + (size_t)i * step];
        }
    }
    return sum / ((x1 - x0) * (y1 - y0));
}

static void scale_case(uint32_t sw, uint32_t sh, uint32_t dw, uint32_t dh, uint32_t pad)
{
    uint32_t ss = sw + pad;
    uint8_t *src = calloc(1, (size_t)ss * sh * 3 / 2 + 64);
    uint8_t *dst = calloc(1, (size_t)dw * dh * 3 / 2);
    uint8_t *y = malloc((size_t)sw * sh), *c = malloc((size_t)sw * sh / 2);
    fake_vdec_pattern(y, c, sw, sh, 5);
    for (uint32_t r = 0; r < sh; r++)
        memcpy(src + (size_t)r * ss, y + (size_t)r * sw, sw);
    uint8_t *sc = src + (size_t)ss * sh;
    for (uint32_t r = 0; r < sh / 2; r++)
        memcpy(sc + (size_t)r * ss, c + (size_t)r * sw, sw);
    struct scaler *s = scale_new(sw, sh, dw, dh);
    CHECK(s != NULL, "scaler %ux%u -> %ux%u", sw, sh, dw, dh);
    if (s) {
        scale_run(s, src, ss, sc, ss, dst, dw, dst + (size_t)dw * dh, dw);
        int worst = 0;
        for (uint32_t r = 0; r < dh; r++)
            for (uint32_t x = 0; x < dw; x++) {
                int d = abs((int)dst[(size_t)r * dw + x] - (int)lround(area_mean(y, sw, 1, sw, sh, dw, dh, x, r)));
                worst = d > worst ? d : worst;
            }
        for (uint32_t r = 0; r < dh / 2; r++)
            for (uint32_t x = 0; x < dw / 2; x++)
                for (unsigned ch = 0; ch < 2; ch++) {
                    int d = abs((int)dst[(size_t)dw * dh + (size_t)r * dw + x * 2 + ch] - (int)lround(area_mean(c + ch, sw, 2, sw / 2, sh / 2, dw / 2, dh / 2, x, r)));
                    worst = d > worst ? d : worst;
                }
        CHECK(worst <= 1, "%ux%u -> %ux%u (stride +%u): worst difference from the exact area mean is %d", sw, sh, dw, dh, pad, worst);
        scale_free(s);
    }
    free(src);
    free(dst);
    free(y);
    free(c);
}

static void test_scale(void)
{
    CHECK(!scale_new(1921, 1080, 1280, 720), "odd source width refused");
    CHECK(!scale_new(1280, 720, 1920, 1080), "enlarging refused");
    CHECK(!scale_new(1920, 1080, 100, 60), "more than 16 to 1 refused");
    CHECK(!scale_new(4096, 2176, 1920, 1080), "a source wider than 3840 refused");
    scale_case(32, 16, 32, 16, 0);   /* the identity is a copy */
    scale_case(64, 48, 32, 24, 0);   /* exactly 2:1 */
    scale_case(64, 48, 32, 24, 16);  /* a source stride wider than the picture */
    scale_case(192, 108, 128, 72, 0); /* 3:2, the 1080 to 720 case */
    scale_case(96, 64, 64, 36, 0);   /* different ratios across and down, with a fraction of a row */
    scale_case(720, 480, 640, 360, 0);

    uint32_t sw = 1920, sh = 1080, dw = 1280, dh = 720;
    uint8_t *src = malloc((size_t)sw * sh * 3 / 2), *dst = malloc((size_t)dw * dh * 3 / 2);
    memset(src, 77, (size_t)sw * sh);
    memset(src + (size_t)sw * sh, 200, (size_t)sw * sh / 2);
    struct scaler *s = scale_new(sw, sh, dw, dh);
    scale_run(s, src, sw, src + (size_t)sw * sh, sw, dst, dw, dst + (size_t)dw * dh, dw);
    int flat = 1;
    for (size_t i = 0; i < (size_t)dw * dh; i++)
        flat &= dst[i] == 77;
    for (size_t i = 0; i < (size_t)dw * dh / 2; i++)
        flat &= dst[(size_t)dw * dh + i] == 200;
    CHECK(flat, "a flat picture stays flat at 1920x1080 -> 1280x720");
    scale_free(s);
    free(src);
    free(dst);
}

/* ---- the wire format: the bytes the engine side (Rust) asserts too ---- */

static void test_wire(void)
{
    struct xc_head h;
    memcpy(h.magic, XC_AU, 4);
    h.flags = XC_FLAG_KEY;
    h.pts = 0x0102030405060708ull;
    h.size = 3;
    h.aux = 7;
    static const uint8_t golden[24] = { 'X', 'A', 'U', 'N', 1, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 3, 0, 0, 0, 7, 0, 0, 0 };
    CHECK(sizeof(h) == XC_HEADER_LEN && !memcmp(&h, golden, sizeof(golden)), "record header bytes");
    CHECK(sizeof(struct xc_cfg) == 64 && sizeof(struct xc_ready) == 24 && sizeof(struct xc_event) == 32, "record sizes");
}

/* ---- the child, over pipes ---- */

#define HIGH_FD 64 /* real descriptors stay clear of the fake kernel's table */
#define MAX_RECS 64

struct rec {
    struct xc_head h;
    uint8_t *data;
};
struct child {
    pid_t pid;
    int to, from;
};
struct run {
    struct rec rec[MAX_RECS];
    int n, status;
};

static void child_start(struct child *c)
{
    int in[2], out[2];
    if (pipe(in) || pipe(out)) {
        perror("pipe");
        exit(2);
    }
    fcntl(in[1], F_SETPIPE_SZ, 1 << 20);
    fcntl(out[1], F_SETPIPE_SZ, 1 << 20);
    fflush(NULL);
    c->pid = fork();
    if (c->pid < 0) {
        perror("fork");
        exit(2);
    }
    if (c->pid == 0) {
        int r = fcntl(in[0], F_DUPFD, HIGH_FD), w = fcntl(out[1], F_DUPFD, HIGH_FD + 1);
        close(in[0]);
        close(in[1]);
        close(out[0]);
        close(out[1]);
        fcntl(r, F_SETFL, fcntl(r, F_GETFL) | O_NONBLOCK);
        if (!getenv("XCODE_TEST_VERBOSE") && !freopen("/dev/null", "w", stderr))
            _exit(99);
        xcode_signals();
        int rc = xcode_run(&sysops_real, r, w);
        _exit(rc | (fake_open_fds() ? 0x80 : 0)); /* 0x80: a device was left open */
    }
    close(in[0]);
    close(out[1]);
    c->to = in[1];
    c->from = out[0];
}

static void send_rec(struct child *c, const char *magic, uint32_t flags, uint64_t pts, uint32_t aux, const void *p, size_t n)
{
    struct xc_head h;
    memcpy(h.magic, magic, 4);
    h.flags = flags;
    h.pts = pts;
    h.size = (uint32_t)n;
    h.aux = aux;
    if (write(c->to, &h, sizeof(h)) < 0 || (n && write(c->to, p, n) < 0))
        return; /* the child has gone: the exit status says why */
}

static void send_au(struct child *c, uint32_t w, uint32_t h, uint32_t id, uint64_t pts)
{
    uint8_t au[12];
    fake_vdec_au(au, w, h, id);
    send_rec(c, XC_AU, 0, pts, 0, au, sizeof(au));
}

static struct xc_cfg cfg_for(uint32_t w, uint32_t h)
{
    struct xc_cfg cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.version = XC_VERSION;
    cfg.codec = XC_CODEC_MPEG2;
    cfg.src_w = w;
    cfg.src_h = h;
    cfg.fps = 30;
    cfg.kbps = 3500;
    return cfg;
}

static int read_full(int fd, void *buf, size_t n)
{
    size_t got = 0;
    while (got < n) {
        ssize_t k = read(fd, (char *)buf + got, n - got);
        if (k <= 0)
            return 0;
        got += (size_t)k;
    }
    return 1;
}

/* 1: a record was read, 0: the child closed its end, -1: nothing within the time */
static int read_rec(struct child *c, struct rec *r, int timeout_ms)
{
    struct pollfd p = { c->from, POLLIN, 0 };
    if (poll(&p, 1, timeout_ms) <= 0)
        return -1;
    if (!read_full(c->from, &r->h, sizeof(r->h)))
        return 0;
    r->data = malloc((size_t)r->h.size + 1);
    return !r->h.size || read_full(c->from, r->data, r->h.size);
}

static void collect(struct child *c, struct run *run, int timeout_ms)
{
    while (run->n < MAX_RECS && read_rec(c, &run->rec[run->n], timeout_ms) == 1)
        run->n++;
}

/* closes the input, reads what is left, reaps the child: its exit status */
static int finish(struct child *c, struct run *run)
{
    close(c->to);
    collect(c, run, 3000);
    close(c->from);
    int st = 0;
    waitpid(c->pid, &st, 0);
    run->status = WIFEXITED(st) ? WEXITSTATUS(st) : 1000 + WTERMSIG(st);
    return run->status;
}

static void run_free(struct run *r)
{
    for (int i = 0; i < r->n; i++)
        free(r->rec[i].data);
}

static int units(const struct run *r, const struct rec **out, int max)
{
    int n = 0;
    for (int i = 0; i < r->n && n < max; i++)
        if (!memcmp(r->rec[i].h.magic, XC_OUT, 4))
            out[n++] = &r->rec[i];
    return n;
}

static const struct xc_event *event_of(const struct run *r, uint32_t kind)
{
    for (int i = 0; i < r->n; i++)
        if (!memcmp(r->rec[i].h.magic, XC_EVENT, 4) && r->rec[i].h.size >= sizeof(struct xc_event) && ((struct xc_event *)r->rec[i].data)->kind == kind)
            return (struct xc_event *)r->rec[i].data;
    return NULL;
}

static int count_of(const struct run *r, const char *magic)
{
    int n = 0;
    for (int i = 0; i < r->n; i++)
        n += !memcmp(r->rec[i].h.magic, magic, 4);
    return n;
}

/* what the fake encoder was handed for access unit `id`: the decoded picture, scaled when the output is smaller */
static uint32_t picture_sum(uint32_t w, uint32_t h, uint32_t id, uint32_t ow, uint32_t oh)
{
    uint8_t *y = malloc((size_t)w * h * 3 / 2), *c = y + (size_t)w * h;
    fake_vdec_pattern(y, c, w, h, id);
    uint32_t sum;
    if (ow == w && oh == h) {
        sum = fake_nv12_hash(y, c, w, h);
    } else {
        struct scaler *s = scale_new(w, h, ow, oh);
        uint8_t *d = malloc((size_t)ow * oh * 3 / 2);
        scale_run(s, y, w, c, w, d, ow, d + (size_t)ow * oh, ow);
        sum = fake_nv12_hash(d, d + (size_t)ow * oh, ow, oh);
        scale_free(s);
        free(d);
    }
    free(y);
    return sum;
}

/* the fake encoder renders its checksum into the unit's payload */
static int unit_is(const struct rec *r, uint32_t sum)
{
    size_t off = (r->h.flags & XC_FLAG_KEY) ? VENC_SPS_PPS_BLOCK + 5 : 5;
    if (r->h.size < off + 24)
        return 0;
    for (int i = 0; i < 24; i++)
        if (r->data[off + i] != (uint8_t)(0x10 + ((sum >> i) & 0x3f)))
            return 0;
    return 1;
}

static void test_basic(void)
{
    struct child c;
    child_start(&c);
    struct xc_cfg cfg = cfg_for(640, 368);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 12; i++) {
        send_au(&c, 640, 368, 100 + i, 1000 + i * 3003ull);
        if (i == 4)
            send_rec(&c, "ZZZZ", 0, 0, 0, "x", 1); /* a record this version does not know is skipped */
    }
    struct run r = { .n = 0 };
    int rc = finish(&c, &r);
    CHECK(rc == XC_EXIT_OK, "exit %d", rc);
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    CHECK(n == 12, "%d units", n);
    CHECK(count_of(&r, XC_READY) == 1 && !memcmp(r.rec[0].h.magic, XC_READY, 4), "the ready record comes first");
    if (r.n && r.rec[0].h.size == sizeof(struct xc_ready)) {
        struct xc_ready *rd = (struct xc_ready *)r.rec[0].data;
        CHECK(rd->version == XC_VERSION && rd->max_w == 640 && rd->max_h == 368 && rd->ncap == 6 && rd->hma_bytes == 6u * 640 * 368 / 2 * 3, "ready: %ux%u ncap %u bytes %u", rd->max_w, rd->max_h, rd->ncap, rd->hma_bytes);
    }
    const struct xc_event *first = event_of(&r, XC_EVT_FIRST);
    CHECK(first && first->a == 640 && first->b == 368, "first-picture event");
    for (int k = 0; k < n && k < 12; k++) {
        CHECK(u[k]->h.pts == 1000 + (uint64_t)k * 3003, "unit %d carries pts %llu", k, (unsigned long long)u[k]->h.pts);
        CHECK(((u[k]->h.flags & XC_FLAG_KEY) != 0) == (k == 0), "unit %d key flag %u", k, u[k]->h.flags);
        CHECK(unit_is(u[k], picture_sum(640, 368, 100 + (uint32_t)k, 640, 368)), "unit %d was made from picture %d", k, 100 + k);
    }
    if (n) {
        CHECK(u[0]->h.size > 8 && u[0]->data[4] == 0x67 && u[0]->data[7] == 30, "the SPS says level %d, not the encoder's 4.1", u[0]->data[7]);
    }
    run_free(&r);
}

static void test_keep_every(void)
{
    struct child c;
    child_start(&c);
    struct xc_cfg cfg = cfg_for(640, 368);
    cfg.keep_every = 2;
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 12; i++)
        send_au(&c, 640, 368, i, 10 + i);
    struct run r = { .n = 0 };
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    CHECK(n == 6, "%d units from 12 pictures kept 1 in 2", n);
    for (int k = 0; k < n && k < 6; k++) {
        CHECK(u[k]->h.pts == 10 + 2 * (uint64_t)k, "unit %d pts %llu", k, (unsigned long long)u[k]->h.pts);
        CHECK(unit_is(u[k], picture_sum(640, 368, 2 * (uint32_t)k, 640, 368)), "unit %d picture", k);
    }
    run_free(&r);
}

static void test_scaled(void)
{
    struct child c;
    child_start(&c);
    struct xc_cfg cfg = cfg_for(0, 0); /* the largest picture the station sends: 1920x1088 */
    cfg.out_w = 1280;
    cfg.out_h = 720;
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 3; i++)
        send_au(&c, 1920, 1080, 7 + i, 500 + i); /* 1080 is shorter than the buffers' 1088: the driver reports a source change */
    struct run r = { .n = 0 };
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    CHECK(n == 3, "%d units", n);
    const struct xc_event *first = event_of(&r, XC_EVT_FIRST);
    CHECK(first && first->a == 1920 && first->b == 1080, "decoded size reported");
    for (int k = 0; k < n && k < 3; k++)
        CHECK(unit_is(u[k], picture_sum(1920, 1080, 7 + (uint32_t)k, 1280, 720)), "unit %d is the picture scaled to 1280x720", k);
    if (n)
        CHECK(u[0]->data[7] == 31, "1280x720 at 30 fps says level %d", u[0]->data[7]);
    run_free(&r);
}

static void test_order_and_errors(void)
{
    setenv("FAKE_VDEC_REORDER", "1", 1);
    struct child c;
    child_start(&c);
    unsetenv("FAKE_VDEC_REORDER");
    struct xc_cfg cfg = cfg_for(640, 368);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 5; i++)
        send_au(&c, 640, 368, i, 10 + i);
    struct run r = { .n = 0 };
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    static const uint32_t want[5] = { 1, 0, 3, 2, 4 }; /* pairs swapped, and the last picture the driver held back comes out at the end */
    CHECK(n == 5, "%d units", n);
    for (int k = 0; k < n && k < 5; k++) {
        CHECK(u[k]->h.pts == 10 + want[k], "unit %d pts %llu", k, (unsigned long long)u[k]->h.pts);
        CHECK(unit_is(u[k], picture_sum(640, 368, want[k], 640, 368)), "unit %d picture", k);
    }
    run_free(&r);

    setenv("FAKE_VDEC_ERROR_EVERY", "3", 1);
    child_start(&c);
    unsetenv("FAKE_VDEC_ERROR_EVERY");
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 9; i++)
        send_au(&c, 640, 368, i, 10 + i);
    struct run e = { .n = 0 };
    CHECK(finish(&c, &e) == XC_EXIT_OK, "exit");
    n = units(&e, u, MAX_RECS);
    static const uint32_t kept[6] = { 0, 1, 3, 4, 6, 7 }; /* the 3rd, 6th and 9th pictures were not decoded and are not encoded */
    CHECK(n == 6, "%d units", n);
    for (int k = 0; k < n && k < 6; k++)
        CHECK(u[k]->h.pts == 10 + kept[k], "unit %d pts %llu", k, (unsigned long long)u[k]->h.pts);
    run_free(&e);
}

static void test_requests(void)
{
    struct child c;
    child_start(&c);
    struct xc_cfg cfg = cfg_for(640, 368);
    cfg.gop = 1000;
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    struct run r = { .n = 0 };
    for (uint32_t i = 0; i < 3; i++)
        send_au(&c, 640, 368, i, i);
    while (count_of(&r, XC_OUT) < 3 && r.n < MAX_RECS && read_rec(&c, &r.rec[r.n], 2000) == 1)
        r.n++;
    send_rec(&c, XC_BITRATE, 0, 0, 2000, NULL, 0);
    send_rec(&c, XC_IDR, 0, 0, 0, NULL, 0);
    for (uint32_t i = 3; i < 5; i++)
        send_au(&c, 640, 368, i, i);
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    CHECK(n == 5, "%d units", n);
    static const int key[5] = { 1, 0, 0, 1, 0 };
    for (int k = 0; k < n && k < 5; k++)
        CHECK(((u[k]->h.flags & XC_FLAG_KEY) != 0) == key[k], "unit %d key flag %u (an XIDR makes the next unit an IDR)", k, u[k]->h.flags);
    run_free(&r);
}

static int run_failure(const struct xc_cfg *cfg, const char *envname, void (*feed)(struct child *), struct run *r)
{
    if (envname)
        setenv(envname, "1", 1);
    struct child c;
    child_start(&c);
    if (envname)
        unsetenv(envname);
    if (cfg)
        send_rec(&c, XC_CONFIG, 0, 0, 0, cfg, sizeof(*cfg));
    if (feed)
        feed(&c);
    return finish(&c, r);
}

static void feed_three(struct child *c)
{
    for (uint32_t i = 0; i < 3; i++)
        send_au(c, 640, 368, i, i);
}

static void feed_shape(struct child *c)
{
    feed_three(c);
    send_au(c, 320, 240, 9, 9);
}

static void feed_au_first(struct child *c)
{
    send_au(c, 640, 368, 0, 0);
}

static void feed_huge(struct child *c)
{
    static uint8_t big[70000];
    send_rec(c, XC_AU, 0, 0, 0, big, sizeof(big));
}

static void test_failures(void)
{
    struct xc_cfg cfg = cfg_for(640, 368);
    struct run r;
    int rc;

    memset(&r, 0, sizeof(r));
    rc = run_failure(&cfg, "FAKE_VDEC_BUSY", feed_three, &r);
    CHECK(rc == XC_EXIT_SETUP, "no decoder memory: exit %d", rc);
    const struct xc_event *e = event_of(&r, XC_EVT_ERROR);
    CHECK(e && e->a == XC_EXIT_SETUP && count_of(&r, XC_READY) == 0, "the failure is reported, and nothing was ready");
    run_free(&r);

    /* the job gives up by itself while its input is still open */
    memset(&r, 0, sizeof(r));
    xcode_limits.first_stall_s = 0.2;
    setenv("FAKE_VDEC_STALL", "1", 1);
    struct child sc;
    child_start(&sc);
    unsetenv("FAKE_VDEC_STALL");
    xcode_limits.first_stall_s = 5.0;
    send_rec(&sc, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    feed_three(&sc);
    collect(&sc, &r, 3000);
    rc = finish(&sc, &r);
    CHECK(rc == XC_EXIT_STALLED, "a decoder that makes no picture: exit %d", rc);
    CHECK(event_of(&r, XC_EVT_ERROR) != NULL, "the stall is reported");
    run_free(&r);

    memset(&r, 0, sizeof(r));
    rc = run_failure(&cfg, NULL, feed_shape, &r);
    CHECK(rc == XC_EXIT_SHAPE, "a picture size change: exit %d", rc);
    CHECK(count_of(&r, XC_OUT) == 3, "the pictures before the change were sent: %d", count_of(&r, XC_OUT));
    run_free(&r);

    struct xc_cfg small = cfg;
    small.out_size = 65536;
    memset(&r, 0, sizeof(r));
    rc = run_failure(&small, NULL, feed_huge, &r);
    CHECK(rc == XC_EXIT_DECODER, "an access unit larger than a bitstream buffer: exit %d", rc);
    run_free(&r);

    struct xc_cfg bad = cfg;
    bad.version = 2;
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(&bad, NULL, NULL, &r)) == XC_EXIT_CONFIG, "unknown version: exit %d", rc);
    run_free(&r);
    bad = cfg;
    bad.kbps = 10;
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(&bad, NULL, NULL, &r)) == XC_EXIT_CONFIG, "bit rate out of range: exit %d", rc);
    run_free(&r);
    bad = cfg;
    bad.out_w = 641;
    bad.out_h = 360;
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(&bad, NULL, NULL, &r)) == XC_EXIT_CONFIG, "odd output width: exit %d", rc);
    run_free(&r);
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(NULL, NULL, NULL, &r)) == XC_EXIT_CONFIG, "input ended before a configuration: exit %d", rc);
    run_free(&r);
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(NULL, NULL, feed_au_first, &r)) == XC_EXIT_CONFIG, "first record not a configuration: exit %d", rc);
    run_free(&r);
}

static void test_ending(void)
{
    struct child c;
    struct xc_cfg cfg = cfg_for(640, 368);
    struct run r = { .n = 0 };

    /* SIGTERM while idle: a clean end with the devices closed */
    child_start(&c);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    CHECK(read_rec(&c, &r.rec[r.n], 2000) == 1 && !memcmp(r.rec[0].h.magic, XC_READY, 4), "ready before the signal");
    r.n++;
    kill(c.pid, SIGTERM);
    CHECK(finish(&c, &r) == XC_EXIT_OK, "SIGTERM: exit %d", r.status);
    run_free(&r);

    /* the engine stops reading: the next write fails and the job ends cleanly */
    memset(&r, 0, sizeof(r));
    child_start(&c);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    CHECK(read_rec(&c, &r.rec[r.n], 2000) == 1, "ready");
    r.n++;
    close(c.from);
    c.from = open("/dev/null", O_RDONLY);
    for (uint32_t i = 0; i < 10; i++)
        send_au(&c, 640, 368, i, i);
    close(c.to);
    int st = 0;
    waitpid(c.pid, &st, 0);
    CHECK(WIFEXITED(st) && WEXITSTATUS(st) == XC_EXIT_OK, "a closed output: status %d", st);
    close(c.from);
    run_free(&r);
}

int main(void)
{
    signal(SIGPIPE, SIG_IGN);
    alarm(30); /* a hang is a failure, not a stuck build */
    test_scale();
    test_wire();
    test_basic();
    test_keep_every();
    test_scaled();
    test_order_and_errors();
    test_requests();
    test_failures();
    test_ending();
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    puts("xcode_test: ok");
    return 0;
}
