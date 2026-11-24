/* Host tests of the transcode child (xcode.c) and its CPU scaler (scale.c) against the fake kernel (tests/fake_sys.c):
 * the scaler on its own, then xcode_run over pipes in a forked process per case, so the exit status and any device the
 * job left open show. Run by test.sh. */
#define _GNU_SOURCE
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../annexb.h"
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
    CHECK(offsetof(struct xc_cfg, out_size) == 56 && offsetof(struct xc_cfg, sar_w) == 60 && offsetof(struct xc_cfg, sar_h) == 62, "the pixel aspect ratio is the last word");
}

/* ---- the pixel aspect ratio written into the SPS ---- */

/* What annexb_set_sar must write is built here the slow way, as strings of '0' and '1', so that it does not share the code's bit handling. */
static void bs_put(char *s, unsigned v, unsigned n)
{
    size_t l = strlen(s);
    while (n--)
        s[l++] = ((v >> n) & 1) ? '1' : '0';
    s[l] = 0;
}

/* ue(v): n zeros, then v + 1 in n + 1 bits */
static void bs_ue(char *s, unsigned v)
{
    unsigned x = v + 1, n = 0;
    while (x >> (n + 1))
        n++;
    bs_put(s, 0, n);
    bs_put(s, x, n + 1);
}

/* a NAL: its header byte, then the bits with a stop bit and zero padding, 00 00 0x escaped as 00 00 03 0x */
static size_t bs_nal(const char *bits, uint8_t header, uint8_t *out)
{
    char s[700];
    snprintf(s, sizeof(s), "%s1", bits);
    while (strlen(s) % 8)
        strcat(s, "0");
    size_t n = 0;
    unsigned zeros = 0;
    out[n++] = header;
    for (size_t i = 0; s[i]; i += 8) {
        uint8_t b = 0;
        for (int k = 0; k < 8; k++)
            b = (uint8_t)(b << 1 | (s[i + k] == '1'));
        if (zeros >= 2 && b <= 3) {
            out[n++] = 3;
            zeros = 0;
        }
        out[n++] = b;
        zeros = b ? 0 : zeros + 1;
    }
    return n;
}

static int has_escape(const uint8_t *p, size_t n)
{
    for (size_t i = 0; i + 2 < n; i++)
        if (!p[i] && !p[i + 1] && p[i + 2] == 3)
            return 1;
    return 0;
}

static size_t hex(const char *s, uint8_t *out)
{
    size_t n = 0;
    for (; s[0] && s[1]; s += 2)
        out[n++] = (uint8_t)strtoul((char[3]) { s[0], s[1], 0 }, NULL, 16);
    return n;
}

struct sps_spec {
    unsigned sps_id, log2_fn, poc_type, refs, mbs_w, mbs_h, fields, crop, scaling;
};

/* an SPS as far as its VUI flag, which is not part of the string: the encoder's own is High, 4:2:0, picture order count type 2, progressive; the others
 * are what the reader has to step over without being surprised */
static void sps_bits(char *s, const struct sps_spec *c)
{
    s[0] = 0;
    bs_put(s, 100, 8);
    bs_put(s, 0, 8);
    bs_put(s, 41, 8);
    bs_ue(s, c->sps_id);
    bs_ue(s, 1);
    bs_ue(s, 0);
    bs_ue(s, 0);
    bs_put(s, 0, 1);
    bs_put(s, c->scaling, 1);
    bs_ue(s, c->log2_fn);
    bs_ue(s, c->poc_type);
    if (c->poc_type == 0) {
        bs_ue(s, 2);
    } else if (c->poc_type == 1) {
        bs_put(s, 0, 1);
        bs_ue(s, 3);
        bs_ue(s, 0);
        bs_ue(s, 2);
        bs_ue(s, 7);
        bs_ue(s, 1);
    }
    bs_ue(s, c->refs);
    bs_put(s, 0, 1);
    bs_ue(s, c->mbs_w - 1);
    bs_ue(s, c->mbs_h - 1);
    bs_put(s, c->fields ? 0 : 1, 1); /* frame_mbs_only_flag; with fields, mb_adaptive_frame_field_flag follows */
    if (c->fields)
        bs_put(s, 1, 1);
    bs_put(s, 1, 1);
    if (c->crop) {
        bs_put(s, 1, 1);
        bs_ue(s, c->crop);
        bs_ue(s, c->crop);
        bs_ue(s, 0);
        bs_ue(s, 0);
    } else {
        bs_put(s, 0, 1);
    }
}

/* an access unit as the encoder emits one: SPS, zero padding, PPS, zero padding, the IDR slice */
static size_t sps_unit(uint8_t *au, const uint8_t *sps, size_t n)
{
    static const uint8_t pps[] = { 0, 0, 0, 1, 0x68, 0xee, 0x38, 0x80 };
    static const uint8_t idr[] = { 0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0xa0, 0x10, 0x55, 0x80 };
    size_t k = 0;
    memcpy(au + k, "\0\0\0\1", 4);
    k += 4;
    memcpy(au + k, sps, n);
    k += n;
    memset(au + k, 0, 6);
    k += 6;
    memcpy(au + k, pps, sizeof(pps));
    k += sizeof(pps);
    memset(au + k, 0, 8);
    k += 8;
    memcpy(au + k, idr, sizeof(idr));
    return k + sizeof(idr);
}

/* the unit comes back exactly as it went in */
static void refused(const char *what, const uint8_t *sps, size_t n, unsigned w, unsigned h, size_t room)
{
    uint8_t au[512], before[512];
    size_t len = sps_unit(au, sps, n);
    memcpy(before, au, len);
    size_t got = annexb_set_sar(au, len, len + room, w, h);
    CHECK(got == 0 && !memcmp(au, before, len), "%s: refused and left alone (returned %zu)", what, got);
}

static void test_sar_rewrite(void)
{
    /* what the encoder's stream says of its pixel aspect ratio once this is done: aspect_ratio_idc (Table E-1), or 255 with the ratio in lowest terms */
    static const struct {
        unsigned w, h, idc, rw, rh;
    } sars[] = {
        { 1, 1, 1, 0, 0 },
        { 10, 11, 3, 0, 0 },
        { 40, 33, 5, 0, 0 },
        { 80, 66, 5, 0, 0 },
        { 4, 3, 14, 0, 0 },
        { 32, 27, 255, 32, 27 },
        { 64, 54, 255, 32, 27 },
        { 1, 32768, 255, 1, 32768 }, /* ends the SPS in 23 zero bits: with the stop bit last in its byte, 00 00 01 wants an escape byte */
        { 65535, 65534, 255, 65535, 65534 },
    };
    /* every combination of these is an SPS: 3 * 3 * 2 * 4 * 3 * 2 of them, 9 ratios each */
    static const unsigned ids[] = { 0, 1, 4 }, frames[] = { 0, 4, 12 }, refs[] = { 1, 3 }, crops[] = { 0, 8, 255, 4095 };
    unsigned flag_at = 0, escaped_in = 0, escaped_out = 0, bad = 0, cases = 0;
    char first[200] = "";
    for (unsigned v = 0; v < 3 * 3 * 2 * 4 * 3 * 2; v++) {
        unsigned r = v;
        struct sps_spec sp = { .sps_id = ids[r % 3] };
        r /= 3;
        sp.log2_fn = frames[r % 3];
        r /= 3;
        sp.refs = refs[r % 2];
        r /= 2;
        sp.crop = crops[r % 4];
        r /= 4;
        sp.poc_type = r % 3;
        r /= 3;
        sp.fields = r % 2;
        sp.mbs_w = v & 1 ? 120 : 44;
        sp.mbs_h = v & 1 ? 68 : 30;
        char bits[700], in_bits[700], want_bits[700];
        sps_bits(bits, &sp);
        flag_at |= 1u << (strlen(bits) % 8);
        snprintf(in_bits, sizeof(in_bits), "%s0", bits);
        uint8_t in_nal[256], want_nal[256], au[512], want[512];
        size_t in_n = bs_nal(in_bits, 0x67, in_nal);
        escaped_in |= (unsigned)has_escape(in_nal, in_n);
        for (unsigned k = 0; k < sizeof(sars) / sizeof(sars[0]); k++) {
            snprintf(want_bits, sizeof(want_bits), "%s11", bits);
            bs_put(want_bits, sars[k].idc, 8);
            if (sars[k].idc == 255) {
                bs_put(want_bits, sars[k].rw, 16);
                bs_put(want_bits, sars[k].rh, 16);
            }
            bs_put(want_bits, 0, 8);
            size_t want_n = bs_nal(want_bits, 0x67, want_nal);
            escaped_out |= (unsigned)has_escape(want_nal, want_n);
            size_t len = sps_unit(au, in_nal, in_n), want_len = sps_unit(want, want_nal, want_n);
            size_t got = annexb_set_sar(au, len, len + 64, sars[k].w, sars[k].h);
            cases++;
            if ((got != want_len || memcmp(au, want, want_len)) && !bad++)
                snprintf(first, sizeof(first), "variant %u with %u:%u: returned %zu, wanted %zu", v, sars[k].w, sars[k].h, got, want_len);
        }
    }
    CHECK(!bad, "%u of %u rewrites differ from the reference; the first: %s", bad, cases, first);
    CHECK(flag_at == 0xff, "the VUI flag fell at every bit of a byte: %#x", flag_at);
    CHECK(escaped_in && escaped_out, "an escape byte was in the SPS read (%u) and in the SPS written (%u)", escaped_in, escaped_out);

    /* an x264 SPS with its VUI cut off (its own stream, 704x480), the ratio written in, then read by ffprobe: 40:33 is 16:9, 10:11 is 4:3, and
     * 32:27 on this picture is 704:405 */
    static const struct {
        unsigned w, h;
        const char *sps;
    } golden[] = {
        { 40, 33, "6764001eacb40581ed828040" },
        { 10, 11, "6764001eacb40581ed818040" },
        { 4, 3, "6764001eacb40581ed870040" },
        { 32, 27, "6764001eacb40581edff8010000d8040" },
    };
    uint8_t plain[16], au[512], before[512], want[16];
    size_t plain_n = hex("6764001eacb40581ec80", plain);
    for (unsigned k = 0; k < sizeof(golden) / sizeof(golden[0]); k++) {
        size_t len = sps_unit(au, plain, plain_n), want_n = hex(golden[k].sps, want);
        memcpy(before, au, len);
        size_t got = annexb_set_sar(au, len, len + want_n - plain_n, golden[k].w, golden[k].h);
        CHECK(got == len + want_n - plain_n && !memcmp(au, before, 4) && !memcmp(au + 4, want, want_n) &&
                  !memcmp(au + 4 + want_n, before + 4 + plain_n, len - 4 - plain_n),
              "%u:%u: the SPS is %s, and everything after it moves up unchanged", golden[k].w, golden[k].h, golden[k].sps);
    }
    refused("no room for the two bytes it grows by", plain, plain_n, 40, 33, 1);

    /* what it leaves alone */
    uint8_t sps[160];
    size_t n = hex("6764001eacb40581ed808800001f480007530078b175", sps);
    refused("an SPS with a VUI already", sps, n, 40, 33, 64);
    char bits[700];
    struct sps_spec scaled = { .log2_fn = 4, .poc_type = 2, .refs = 1, .mbs_w = 44, .mbs_h = 30, .scaling = 1 };
    sps_bits(bits, &scaled);
    strcat(bits, "0");
    n = bs_nal(bits, 0x67, sps);
    refused("an SPS with scaling lists", sps, n, 40, 33, 64);
    struct sps_spec plain_spec = { .log2_fn = 4, .poc_type = 2, .refs = 1, .mbs_w = 44, .mbs_h = 30 };
    sps_bits(bits, &plain_spec);
    strcat(bits, "01");
    n = bs_nal(bits, 0x67, sps);
    refused("an SPS with more after its VUI flag", sps, n, 40, 33, 64);
    refused("a ratio of 0", plain, plain_n, 0, 1, 64);
    refused("a ratio of 1:0", plain, plain_n, 1, 0, 64);
    refused("a ratio over 16 bits", plain, plain_n, 65536, 1, 64);
    for (size_t cut = 1; cut < plain_n; cut++) {
        uint8_t part[32] = { 0, 0, 0, 1 }, keep[32];
        memcpy(part + 4, plain, cut);
        memcpy(keep, part, 4 + cut);
        CHECK(annexb_set_sar(part, 4 + cut, 4 + cut + 64, 40, 33) == 0 && !memcmp(part, keep, 4 + cut), "an SPS cut after %zu bytes is left alone", cut);
    }
    static const uint8_t no_sps[] = { 0, 0, 0, 1, 0x41, 0x9a, 0x00, 0x33, 0x80 };
    uint8_t p_unit[sizeof(no_sps)];
    memcpy(p_unit, no_sps, sizeof(no_sps));
    CHECK(annexb_set_sar(p_unit, sizeof(p_unit), 128, 40, 33) == 0 && !memcmp(p_unit, no_sps, sizeof(no_sps)), "a unit with no SPS is left alone");
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

/* the fake encoder renders its checksum into the unit's payload, behind the slice's NAL header; a key unit's SPS and PPS before it may have grown */
static int unit_is(const struct rec *r, uint32_t sum)
{
    size_t pos = 0;
    struct annexb_nal nal;
    while (annexb_next(r->data, r->h.size, &pos, &nal) && nal.type != 1 && nal.type != 5)
        ;
    if (nal.type != 1 && nal.type != 5)
        return 0;
    if (nal.len < 1 + 24)
        return 0;
    for (int i = 0; i < 24; i++)
        if (nal.p[1 + i] != (uint8_t)(0x10 + ((sum >> i) & 0x3f)))
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
        CHECK(u[0]->h.size == VENC_SPS_PPS_BLOCK + 5 + 4000 + 1, "without a pixel aspect ratio the unit is the encoder's own: %u bytes", u[0]->h.size);
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

static void test_pixel_aspect(void)
{
    /* the encoder's SPS (an x264 one without its VUI) with 40:33 added and the level said as 3.0 */
    static const uint8_t golden[12] = { 0x67, 0x64, 0x00, 0x1e, 0xac, 0xb4, 0x05, 0x81, 0xed, 0x82, 0x80, 0x40 };
    struct xc_cfg cfg = cfg_for(640, 368);
    cfg.sar_w = 40;
    cfg.sar_h = 33;
    struct child c;
    child_start(&c);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 3; i++)
        send_au(&c, 640, 368, i, i);
    struct run r = { .n = 0 };
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    const struct rec *u[MAX_RECS];
    int n = units(&r, u, MAX_RECS);
    CHECK(n == 3, "%d units", n);
    if (n == 3) {
        CHECK(u[0]->h.size == VENC_SPS_PPS_BLOCK + 5 + 4000 + 1 + 2 && !memcmp(u[0]->data + 4, golden, sizeof(golden)),
              "the IDR's SPS carries 40:33: %u bytes", u[0]->h.size);
        CHECK(unit_is(u[0], picture_sum(640, 368, 0, 640, 368)), "the slice behind the grown SPS is the encoder's");
        CHECK(u[1]->h.size == 5 + 900 + 1 && u[2]->h.size == 5 + 900 + 1, "the other units are as the encoder made them");
    }
    run_free(&r);

    /* an SPS that cannot take it (Main profile has no chroma and bit depth fields, so the fake's SPS does not read as one) goes out as it was */
    cfg.profile = LX_VENC_H264_PROFILE_MAIN;
    child_start(&c);
    send_rec(&c, XC_CONFIG, 0, 0, 0, &cfg, sizeof(cfg));
    for (uint32_t i = 0; i < 3; i++)
        send_au(&c, 640, 368, i, i);
    memset(&r, 0, sizeof(r));
    CHECK(finish(&c, &r) == XC_EXIT_OK, "exit");
    n = units(&r, u, MAX_RECS);
    CHECK(n == 3, "%d units", n);
    if (n == 3)
        CHECK(u[0]->h.size == VENC_SPS_PPS_BLOCK + 5 + 4000 + 1 && u[0]->data[5] == 77, "the unit went out unchanged: %u bytes", u[0]->h.size);
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
    bad = cfg;
    bad.sar_w = 40;
    memset(&r, 0, sizeof(r));
    CHECK((rc = run_failure(&bad, NULL, NULL, &r)) == XC_EXIT_CONFIG, "a pixel aspect ratio with no height: exit %d", rc);
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
    test_sar_rewrite();
    test_basic();
    test_keep_every();
    test_scaled();
    test_pixel_aspect();
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
