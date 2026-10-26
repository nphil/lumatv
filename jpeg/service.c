/* lumatv-jpeg: the 32-bit JPEG decoder of LumaTV, a small service over LG's hardware JPEG block.
 *
 * LG's userspace wrapper for the block (libdile_photo.so.1, DILE_JPEG_*, dilejpeg.h) is 32-bit only, like libvtcapture, so the
 * 64-bit helper starts this program when an image is wanted and talks to it over the socket it hands over as fd 0 (jpegproto.h).
 * It takes the bytes of a JPEG file and answers the pixels: NV12 or RGBA, at 1, 1/2, 1/4 or 1/8 per axis (the hardware's scaled
 * decode), after the conversion from whatever YUV layout the hardware wrote. Where the block is missing, refuses a file or fails
 * on it, libjpeg.so.62 (the CPU's decoder, cpu.c) gives the same planes and the answer says so. Nothing else leaves the
 * process: no network, no file (the helper reads the file and sends the bytes), no bus call.
 *
 *   lumatv-jpeg [-f FD] [-c CLOSE_MS] [-x EXIT_MS] [-A ALIGN] [-B VALIGN]     serve (default)
 *   lumatv-jpeg -n                                                            resolve the library's functions, print, exit
 *   lumatv-jpeg -b FILE [-N COUNT] [-z SHIFT] [-F nv12|rgba|bgra] [-o DIR] [-C|-H] [-L] [-S]
 *                                                                             time COUNT decodes of FILE (the TV slot's tool)
 *
 *   -f FD        the connected socket (default 0)
 *   -C, -H       bench: CPU only (libjpeg, the baseline), or hardware only (no fallback)
 *   -c CLOSE_MS  the hardware is given back (DILE_JPEG_Close) after this long without a request (default 3000)
 *   -x EXIT_MS   the process ends after this long without a request (default 30000); the helper starts it again when needed
 *   -A, -B       the hardware buffer's row alignment and plane-height alignment, in pixels and rows (default 1, 1). The planes of
 *                the buffer are laid out Y, Cb, Cr (or Y, CbCr) with Y rows of align(width, A) bytes; the TV slot's comparison
 *                (jpeg/compare.py) says which values the set's buffers match.
 *
 * The library is not loaded until the first decode, and is closed again when the requests stop: the device is held only while
 * images are being decoded. The block is idle on the set otherwise (docs/research/hardware-census.md, rank 3).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "convert.h"
#include "cpu.h"
#include "dilejpeg.h"
#include "jpegproto.h"

#define MAX_JPEG (64u * 1024 * 1024)
#define MIN_DIM 16u    /* LX_MJPEG_MIN_HSIZE / VSIZE (mjpeg_kapi.h) and HAL_JPEG_Encode's own check */
#define MAX_DIM 32768u /* LX_MJPEG_MAX_HSIZE / VSIZE */
#define HARD_MAX_PIXELS (64u * 1024 * 1024) /* of output, whatever a request allows: an 8K picture is 33 million */

static dile_jpeg_open_fn p_open;
static dile_jpeg_close_fn p_close;
static dile_jpeg_decode_fn p_decode;
static void *g_lib;
static int g_open;
static volatile sig_atomic_t g_stop;
static unsigned g_align = 1, g_valign = 1;

static void say(const char *fmt, ...) {
    char line[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    fprintf(stderr, "lumatv-jpeg: %s\n", line);
}

static uint64_t mono_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

static uint64_t cpu_us(void) {
    struct timespec ts;
    return clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &ts) == 0 ? (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u : 0;
}

/* ---- the library ------------------------------------------------------------------------------------------------------- */

static int g_lib_failed;

static int load_library(void) {
    if (p_decode) return JP_OK;
    if (g_lib_failed) return JP_E_NO_LIBRARY; /* said once; the library does not appear while the program runs */
    /* libdile_photo calls PmLog (_PmLogMsgKV) without linking it: on the set it fails to load unless the logging library is already
     * in the process, global (2026-10-25). Its absence is not an error here: the library's own load says what is missing. */
    void *pmlog = dlopen("libPmLogLib.so.3", RTLD_NOW | RTLD_GLOBAL);
    (void)pmlog;
    g_lib = dlopen("libdile_photo.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!g_lib) {
        say("libdile_photo.so.1 did not load: %s", dlerror());
        g_lib_failed = 1;
        return JP_E_NO_LIBRARY;
    }
    p_open = (dile_jpeg_open_fn)dlsym(g_lib, "DILE_JPEG_Open");
    p_close = (dile_jpeg_close_fn)dlsym(g_lib, "DILE_JPEG_Close");
    p_decode = (dile_jpeg_decode_fn)dlsym(g_lib, "DILE_JPEG_Decode");
    if (!p_open || !p_close || !p_decode) {
        say("libdile_photo.so.1 lacks DILE_JPEG_Open/Close/Decode");
        p_decode = NULL;
        dlclose(g_lib);
        g_lib = NULL;
        g_lib_failed = 1;
        return JP_E_NO_LIBRARY;
    }
    return JP_OK;
}

static int open_device(void) {
    if (g_open) return JP_OK;
    int rc = load_library();
    if (rc) return rc;
    rc = p_open();
    if (rc) {
        say("DILE_JPEG_Open returned %d", rc);
        return JP_E_OPEN;
    }
    g_open = 1;
    return JP_OK;
}

static void close_device(void) {
    if (!g_open) return;
    int rc = p_close();
    if (rc) say("DILE_JPEG_Close returned %d", rc);
    g_open = 0;
}

/* ---- what the hardware writes ------------------------------------------------------------------------------------------ */

struct hwgeo {
    int hshift, vshift, semi, gray;
};

/* The pixel_format -> layout table of dilejpeg.h. */
static int hw_geo(uint32_t pf, struct hwgeo *g) {
    memset(g, 0, sizeof *g);
    switch (pf) {
    case DILE_JPEG_PF_YUV444_PLANAR: break;
    case DILE_JPEG_PF_YUV444_SEMI: g->semi = 1; break;
    case DILE_JPEG_PF_YUV422H_PLANAR: g->hshift = 1; break;
    case DILE_JPEG_PF_YUV422H_SEMI: g->hshift = 1; g->semi = 1; break;
    case DILE_JPEG_PF_YUV420_PLANAR: g->hshift = g->vshift = 1; break;
    case DILE_JPEG_PF_YUV420_SEMI: g->hshift = g->vshift = 1; g->semi = 1; break;
    case DILE_JPEG_PF_GRAY: g->gray = 1; break;
    case DILE_JPEG_PF_YUV224_PLANAR: g->vshift = 1; break;
    case DILE_JPEG_PF_YUV224_SEMI: g->vshift = 1; g->semi = 1; break;
    default: return -1;
    }
    return 0;
}

static size_t align_up(size_t v, size_t a) {
    return a > 1 ? (v + a - 1) / a * a : v;
}

/* The buffer's planes for `w` x `h` as reported, and the bytes they take. */
static size_t hw_planes(const struct hwgeo *g, uint32_t w, uint32_t h, const uint8_t *base, int swap_uv, struct yuv_image *im) {
    size_t ystride = align_up(w, g_align), yrows = align_up(h, g_valign);
    size_t ybytes = ystride * yrows;
    memset(im, 0, sizeof *im);
    im->y = base;
    im->ystride = ystride;
    im->hshift = g->hshift;
    im->vshift = g->vshift;
    if (g->gray) return ybytes;
    size_t cw = (ystride + ((size_t)1 << g->hshift) - 1) >> g->hshift;
    size_t crows = (yrows + ((size_t)1 << g->vshift) - 1) >> g->vshift;
    const uint8_t *cb, *cr;
    size_t total;
    if (g->semi) {
        im->cstride = cw * 2;
        im->cstep = 2;
        cb = base + ybytes;
        cr = cb + 1;
        total = ybytes + im->cstride * crows;
    } else {
        im->cstride = cw;
        im->cstep = 1;
        cb = base + ybytes;
        cr = cb + cw * crows;
        total = ybytes + 2 * cw * crows;
    }
    im->cb = swap_uv ? cr : cb;
    im->cr = swap_uv ? cb : cr;
    return total;
}

/* ---- the JPEG file's own header ---------------------------------------------------------------------------------------- */

struct jinfo {
    uint32_t width, height;
    int hw; /* 0, or the status the hardware path would answer for this file (JP_E_UNSUPPORTED) */
};

/* Reads the frame header: the dimensions, and whether the block takes the file (it decodes baseline, extended-sequential and
 * progressive Huffman JPEG, 8 bit, gray or 3 components at 4:4:4, 4:2:2, 4:2:0 or 4:2:4, 16 to 32768 a side: mjpeg_kapi.h
 * LX_MJPEG_FORMAT_*, LX_MJPEG_MIN/MAX_HSIZE, SOF_MARKER_PDCT_HD). Returns 0, or JP_E_REQUEST for a file that is not a JPEG. */
static int jpeg_probe(const uint8_t *p, size_t n, struct jinfo *ji) {
    memset(ji, 0, sizeof *ji);
    if (n < 4 || p[0] != 0xff || p[1] != 0xd8) return JP_E_REQUEST;
    size_t i = 2;
    while (i + 4 <= n) {
        if (p[i] != 0xff) return JP_E_REQUEST;
        while (i < n && p[i] == 0xff) i++;
        if (i >= n) break;
        unsigned m = p[i++];
        if (m == 0x01 || (m >= 0xd0 && m <= 0xd8)) continue; /* TEM, RSTn, SOI: no length */
        if (m == 0xd9 || m == 0xda) break;                   /* EOI, SOS: no frame header before the data */
        if (i + 2 > n) break;
        size_t len = ((size_t)p[i] << 8) | p[i + 1];
        if (len < 2 || i + len > n) return JP_E_REQUEST;
        if (m >= 0xc0 && m <= 0xcf && m != 0xc4 && m != 0xc8 && m != 0xcc) {
            const uint8_t *s = p + i + 2;
            if (len < 8) return JP_E_REQUEST;
            ji->height = ((uint32_t)s[1] << 8) | s[2];
            ji->width = ((uint32_t)s[3] << 8) | s[4];
            if (!ji->width || !ji->height) return JP_E_REQUEST;
            unsigned nf = s[5];
            ji->hw = JP_E_UNSUPPORTED;
            if ((m != 0xc0 && m != 0xc1 && m != 0xc2) || s[0] != 8) return 0; /* lossless, hierarchical, arithmetic, not 8 bit */
            if (ji->width < MIN_DIM || ji->height < MIN_DIM || ji->width > MAX_DIM || ji->height > MAX_DIM) return 0;
            if (nf == 1) {
                ji->hw = 0;
                return 0;
            }
            if (nf != 3 || len < 8 + 3u * nf) return 0;
            unsigned h0 = s[7] >> 4, v0 = s[7] & 15;
            for (int c = 1; c < 3; c++)
                if (s[6 + 3 * c + 1] != 0x11) return 0;
            if ((h0 == 1 || h0 == 2) && (v0 == 1 || v0 == 2)) ji->hw = 0;
            return 0;
        }
        i += len;
    }
    return JP_E_REQUEST;
}

/* ---- one decode -------------------------------------------------------------------------------------------------------- */

struct result {
    uint8_t *data; /* malloc'd; the caller frees */
    uint32_t width, height, stride, bytes;
    uint32_t decode_us, convert_us;
    uint32_t hw_pixfmt, hw_width, hw_height, hw_bytes;
    uint32_t flags; /* JP_RFLAG_* and the hardware's status in bits 8..15 */
};

static uint32_t ceil_shift(uint32_t v, unsigned s) {
    return (v + (1u << s) - 1) >> s;
}

/* The hardware path: the library's buffer and the planes in it. `dump`: where to write the buffer before it is converted (the TV
 * slot's comparison), or NULL. On JP_OK `*buf` is the library's buffer (free() it) and `im` points into it. */
static int hw_decode(const uint8_t *jpeg, size_t n, const struct jinfo *ji, unsigned shift, unsigned flags, uint32_t ow, uint32_t oh,
                     const char *dump, struct result *r, uint8_t **buf, struct yuv_image *im) {
    int rc;
    if (ji->hw) return ji->hw;
    if ((rc = open_device())) return rc;

    /* DILE reads the dimensions only to size the buffer it allocates: (w >> shift) * (h >> shift) * 3. The hardware rounds the
     * scaled size up and may pad it to its macroblock, so ask for more than the picture needs rather than trust the library's
     * arithmetic with the exact size. */
    struct dile_jpeg_buffer src, dst;
    memset(&src, 0, sizeof src);
    memset(&dst, 0, sizeof dst);
    src.data = (void *)jpeg;
    src.size = (uint32_t)n;
    src.width = (uint32_t)align_up(ji->width, 64) + 64;
    src.height = (uint32_t)align_up(ji->height, 64) + 64;

    uint64_t t0 = mono_us();
    int lrc = p_decode(&src, &dst, (int)shift + DILE_JPEG_SCALE_1_1);
    uint64_t t1 = mono_us();
    r->decode_us = (uint32_t)(t1 - t0);
    if (lrc) {
        say("DILE_JPEG_Decode returned %d (%ux%u, shift %u)", lrc, ji->width, ji->height, shift);
        if (dst.data) free(dst.data);
        return JP_E_DECODE;
    }
    r->hw_pixfmt = dst.pixel_format;
    r->hw_width = dst.width;
    r->hw_height = dst.height;
    r->hw_bytes = dst.size;
    if (dump) {
        FILE *f = fopen(dump, "wb");
        if (f) {
            fwrite(dst.data, 1, dst.size, f);
            fclose(f);
        }
    }

    struct hwgeo g;
    rc = JP_OK;
    if (!dst.data || hw_geo(dst.pixel_format, &g) || dst.width < 1 || dst.height < 1 || dst.width > ow + 63 || dst.height > oh + 63 ||
        dst.width + 1 < ow || dst.height + 1 < oh) {
        say("DILE_JPEG_Decode gave an image this build cannot read: %ux%u format %u (expected %ux%u)", dst.width, dst.height,
            dst.pixel_format, ow, oh);
        rc = JP_E_OUTPUT;
    } else {
        size_t need = hw_planes(&g, dst.width, dst.height, dst.data, (flags & JP_FLAG_SWAP_UV) != 0, im);
        if (need > dst.size) {
            say("the planes of a %ux%u format %u buffer take %zu bytes, the library allocated %u", dst.width, dst.height,
                dst.pixel_format, need, dst.size);
            rc = JP_E_OUTPUT;
        }
    }
    if (rc) {
        free(dst.data);
        return rc;
    }
    im->width = (int)(dst.width < ow ? dst.width : ow);
    im->height = (int)(dst.height < oh ? dst.height : oh);
    *buf = dst.data;
    return JP_OK;
}

/* The CPU path (libjpeg.so.62), the same planes. */
static int cpu_path(const uint8_t *jpeg, size_t n, unsigned shift, unsigned flags, uint32_t ow, uint32_t oh, struct result *r,
                    uint8_t **buf, struct yuv_image *im) {
    struct cpu_planes pl;
    uint64_t t0 = mono_us();
    int rc = cpu_decode(jpeg, n, shift, &pl);
    r->decode_us = (uint32_t)(mono_us() - t0);
    switch (rc) {
    case CPU_OK: break;
    case CPU_NO_LIBRARY: return JP_E_NO_LIBRARY;
    case CPU_UNSUPPORTED: return JP_E_UNSUPPORTED;
    case CPU_NOMEM: return JP_E_NOMEM;
    default: say("libjpeg refused the file"); return JP_E_DECODE;
    }
    *im = pl.im;
    if ((uint32_t)im->width < ow || (uint32_t)im->height < oh) {
        say("libjpeg gave %dx%d for a %ux%u picture", im->width, im->height, ow, oh);
        free(pl.buf);
        return JP_E_OUTPUT;
    }
    im->width = (int)ow;
    im->height = (int)oh;
    if (flags & JP_FLAG_SWAP_UV) {
        const uint8_t *t = im->cb;
        im->cb = im->cr;
        im->cr = t;
    }
    *buf = pl.buf;
    return JP_OK;
}

static int decode_one(const uint8_t *jpeg, size_t n, int fmt, unsigned shift, unsigned flags, uint32_t max_pixels, const char *dump,
                      struct result *r) {
    struct jinfo ji;
    memset(r, 0, sizeof *r);
    int rc = jpeg_probe(jpeg, n, &ji);
    if (rc) return rc;
    uint32_t ow = ceil_shift(ji.width, shift), oh = ceil_shift(ji.height, shift);
    if (!max_pixels || max_pixels > HARD_MAX_PIXELS) max_pixels = HARD_MAX_PIXELS;
    if ((uint64_t)ow * oh > max_pixels) return JP_E_TOO_BIG;

    uint8_t *buf = NULL;
    struct yuv_image im;
    memset(&im, 0, sizeof im);
    if (flags & JP_FLAG_CPU) {
        rc = cpu_path(jpeg, n, shift, flags, ow, oh, r, &buf, &im);
        if (rc) return rc;
        r->flags |= JP_RFLAG_CPU;
    } else {
        rc = hw_decode(jpeg, n, &ji, shift, flags, ow, oh, dump, r, &buf, &im);
        if (rc && !(flags & JP_FLAG_HW_ONLY)) {
            /* The block is not there, does not take this file, or failed on it: the same planes from libjpeg. */
            uint32_t hw_us = r->decode_us;
            int crc = cpu_path(jpeg, n, shift, flags, ow, oh, r, &buf, &im);
            if (crc) return rc; /* the hardware's reason is the more useful one */
            r->decode_us += hw_us;
            r->flags |= JP_RFLAG_CPU | ((uint32_t)rc << 8);
            rc = JP_OK;
        }
        if (rc) return rc;
    }

    if (fmt == JP_FMT_NV12) {
        r->stride = (uint32_t)((im.width + 1) & ~1);
        r->bytes = r->stride * ((uint32_t)im.height + ((uint32_t)im.height + 1) / 2);
    } else {
        r->stride = (uint32_t)im.width * 4;
        r->bytes = r->stride * (uint32_t)im.height;
    }
    r->width = (uint32_t)im.width;
    r->height = (uint32_t)im.height;
    r->data = malloc(r->bytes);
    if (!r->data) {
        free(buf);
        return JP_E_NOMEM;
    }
    uint64_t t2 = mono_us();
    if (fmt == JP_FMT_NV12)
        yuv_to_nv12(&im, r->data, r->data + (size_t)r->stride * (uint32_t)im.height, r->stride);
    else
        yuv_to_rgba(&im, (flags & JP_FLAG_LIMITED) ? &YUV_LIMITED : &YUV_FULL, fmt == JP_FMT_BGRA, r->data, r->stride);
    r->convert_us = (uint32_t)(mono_us() - t2);
    free(buf);
    return JP_OK;
}

/* ---- the socket -------------------------------------------------------------------------------------------------------- */

static int read_full(int fd, void *buf, size_t n) {
    uint8_t *p = buf;
    while (n) {
        ssize_t k = read(fd, p, n);
        if (k < 0 && errno == EINTR && !g_stop) continue;
        if (k <= 0) return -1;
        p += k;
        n -= (size_t)k;
    }
    return 0;
}

static int write_full(int fd, const void *buf, size_t n) {
    const uint8_t *p = buf;
    while (n) {
        ssize_t k = write(fd, p, n);
        if (k < 0 && errno == EINTR) continue;
        if (k <= 0) return -1;
        p += k;
        n -= (size_t)k;
    }
    return 0;
}

static void put32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

static uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

static int respond(int fd, uint32_t id, int status, const struct result *r, uint32_t total_us) {
    uint8_t h[JP_RSP_BYTES];
    memset(h, 0, sizeof h);
    uint32_t len = status == JP_OK && r->data ? r->bytes : 0;
    put32(h + 0, JP_RSP_MAGIC);
    put32(h + 4, id);
    put32(h + 8, (uint32_t)status);
    put32(h + 12, r->width);
    put32(h + 16, r->height);
    put32(h + 20, r->stride);
    put32(h + 24, 0);
    put32(h + 28, len);
    put32(h + 32, r->decode_us);
    put32(h + 36, r->convert_us);
    put32(h + 40, total_us);
    put32(h + 44, r->hw_pixfmt);
    put32(h + 48, r->hw_width);
    put32(h + 52, r->hw_height);
    put32(h + 56, r->hw_bytes);
    put32(h + 60, r->flags);
    return write_full(fd, h, sizeof h) || (len && write_full(fd, r->data, len)) ? -1 : 0;
}

/* One request. Returns 0 to go on, -1 when the connection is over. */
static int serve_one(int fd) {
    uint8_t h[JP_REQ_BYTES];
    if (read_full(fd, h, sizeof h)) return -1;
    struct result r;
    memset(&r, 0, sizeof r);
    uint32_t id = get32(h + 4);
    uint32_t w2 = get32(h + 8), max_pixels = get32(h + 12), len = get32(h + 16);
    unsigned op = w2 & 255, fmt = (w2 >> 8) & 255, shift = (w2 >> 16) & 255, flags = w2 >> 24;
    uint64_t t0 = mono_us();
    if (get32(h) != JP_REQ_MAGIC || len > MAX_JPEG) {
        respond(fd, id, JP_E_REQUEST, &r, 0);
        return -1;
    }
    if (op == JP_OP_PING && len == 0) return respond(fd, id, JP_OK, &r, 0);
    uint8_t *jpeg = len ? malloc(len) : NULL;
    if (op != JP_OP_DECODE || fmt < JP_FMT_NV12 || fmt > JP_FMT_BGRA || shift > 3 || !len || !jpeg) {
        int rc = !len || op != JP_OP_DECODE || fmt < JP_FMT_NV12 || fmt > JP_FMT_BGRA || shift > 3 ? JP_E_REQUEST : JP_E_NOMEM;
        free(jpeg);
        /* The body is still on the wire: it is read and dropped so the next request starts where it should. */
        uint8_t sink[4096];
        for (uint32_t left = len; left;) {
            size_t k = left < sizeof sink ? left : sizeof sink;
            if (read_full(fd, sink, k)) return -1;
            left -= (uint32_t)k;
        }
        return respond(fd, id, rc, &r, 0);
    }
    if (read_full(fd, jpeg, len)) {
        free(jpeg);
        return -1;
    }
    int rc = decode_one(jpeg, len, (int)fmt, shift, flags, max_pixels, NULL, &r);
    free(jpeg);
    int out = respond(fd, id, rc, &r, (uint32_t)(mono_us() - t0));
    free(r.data);
    return out;
}

static void on_signal(int sig) {
    (void)sig;
    g_stop = 1;
}

static int serve(int fd, int close_ms, int exit_ms) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_signal;
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);
    uint64_t last = mono_us();
    while (!g_stop) {
        struct pollfd p = {.fd = fd, .events = POLLIN};
        int k = poll(&p, 1, 250);
        uint64_t idle_ms = (mono_us() - last) / 1000u;
        if (k < 0 && errno != EINTR) break;
        if (k == 0) {
            if (g_open && idle_ms >= (uint64_t)close_ms) {
                close_device();
                say("idle, the hardware block is given back");
            }
            if (idle_ms >= (uint64_t)exit_ms) break;
            continue;
        }
        if (k < 0) continue;
        if (p.revents & POLLIN) {
            if (serve_one(fd)) break;
            last = mono_us();
        } else if (p.revents & (POLLHUP | POLLERR)) {
            break;
        }
    }
    close_device();
    return 0;
}

/* ---- the TV slot's tool ------------------------------------------------------------------------------------------------ */

static long read_kv(const char *path, const char *key) {
    FILE *f = fopen(path, "r");
    char line[512];
    long v = -1;
    size_t kl = strlen(key);
    if (!f) return -1;
    while (fgets(line, sizeof line, f))
        if (!strncmp(line, key, kl)) {
            v = strtol(line + kl, NULL, 10);
            break;
        }
    fclose(f);
    return v;
}

/* The interrupt count of the JPEG decoder (the line named JpgDec in /proc/interrupts), all CPUs added up; -1 when there is none. */
static long jpg_irqs(void) {
    FILE *f = fopen("/proc/interrupts", "r");
    char line[1024];
    long sum = -1;
    if (!f) return -1;
    while (fgets(line, sizeof line, f)) {
        if (!strstr(line, "JpgDec")) continue;
        sum = 0;
        char *p = strchr(line, ':');
        while (p && *++p) {
            char *end;
            long v = strtol(p, &end, 10);
            if (end == p) break;
            sum += v;
            p = end;
        }
        break;
    }
    fclose(f);
    return sum;
}

static int cmp_u32(const void *a, const void *b) {
    uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
    return x < y ? -1 : x > y;
}

static int bench(const char *file, int count, unsigned shift, int fmt, const char *dir, unsigned flags) {
    FILE *f = fopen(file, "rb");
    if (!f) {
        say("%s: %s", file, strerror(errno));
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *jpeg = n > 0 && (unsigned long)n <= MAX_JPEG ? malloc((size_t)n) : NULL;
    if (!jpeg || fread(jpeg, 1, (size_t)n, f) != (size_t)n) {
        say("%s: cannot read it (or over %u bytes)", file, MAX_JPEG);
        fclose(f);
        return 1;
    }
    fclose(f);
    struct jinfo ji;
    int rc = jpeg_probe(jpeg, (size_t)n, &ji);
    if (rc) {
        say("%s: not a JPEG file (status %d)", file, rc);
        return 1;
    }
    printf("file=%s bytes=%ld width=%u height=%u block_takes_it=%d shift=%u count=%d convert=%s path=%s\n", file, n, ji.width, ji.height, ji.hw == 0,
           shift, count, convert_impl(), (flags & JP_FLAG_CPU) ? "cpu" : (flags & JP_FLAG_HW_ONLY) ? "hardware" : "hardware, then cpu");
    uint32_t *dec = calloc((size_t)count, 4), *conv = calloc((size_t)count, 4), *wall = calloc((size_t)count, 4);
    long irq0 = jpg_irqs(), cma0 = read_kv("/proc/meminfo", "CmaFree:"), cma_min = cma0;
    uint64_t cpu0 = cpu_us(), wall0 = mono_us();
    int ok = 0;
    for (int i = 0; i < count; i++) {
        struct result r;
        char dump[512];
        const char *dp = NULL;
        if (i == 0 && dir) {
            snprintf(dump, sizeof dump, "%s/hw-s%u.bin", dir, shift);
            dp = dump;
        }
        uint64_t t = mono_us();
        int st = decode_one(jpeg, (size_t)n, fmt, shift, flags, 0, dp, &r);
        wall[i] = (uint32_t)(mono_us() - t);
        long cma = read_kv("/proc/meminfo", "CmaFree:");
        if (cma >= 0 && (cma_min < 0 || cma < cma_min)) cma_min = cma;
        if (st) {
            printf("iter=%d status=%d\n", i, st);
            free(r.data);
            continue;
        }
        dec[ok] = r.decode_us;
        conv[ok] = r.convert_us;
        if (i == 0) {
            printf("out_width=%u out_height=%u out_stride=%u hw_width=%u hw_height=%u hw_pixfmt=%u hw_bytes=%u used=%s hw_status=%u\n", r.width,
                   r.height, r.stride, r.hw_width, r.hw_height, r.hw_pixfmt, r.hw_bytes, (r.flags & JP_RFLAG_CPU) ? "cpu" : "hardware",
                   (r.flags >> 8) & 255);
            if (dir) {
                char path[512];
                snprintf(path, sizeof path, "%s/out-s%u.%s", dir, shift, fmt == JP_FMT_NV12 ? "nv12" : "rgba");
                FILE *o = fopen(path, "wb");
                if (o) {
                    fwrite(r.data, 1, r.bytes, o);
                    fclose(o);
                }
            }
        }
        ok++;
        free(r.data);
    }
    uint64_t wall_us = mono_us() - wall0, cpu = cpu_us() - cpu0;
    long irq1 = jpg_irqs(), cma1 = read_kv("/proc/meminfo", "CmaFree:");
    if (ok) {
        qsort(dec, (size_t)ok, 4, cmp_u32);
        qsort(conv, (size_t)ok, 4, cmp_u32);
        uint64_t sd = 0, sc = 0;
        for (int i = 0; i < ok; i++) {
            sd += dec[i];
            sc += conv[i];
        }
        printf("decode_us min=%u median=%u mean=%llu max=%u\n", dec[0], dec[ok / 2], (unsigned long long)(sd / (uint64_t)ok), dec[ok - 1]);
        printf("convert_us min=%u median=%u mean=%llu max=%u\n", conv[0], conv[ok / 2], (unsigned long long)(sc / (uint64_t)ok), conv[ok - 1]);
    }
    printf("ok=%d of %d wall_ms=%llu cpu_ms=%llu cpu_per_image_ms=%llu\n", ok, count, (unsigned long long)(wall_us / 1000),
           (unsigned long long)(cpu / 1000), ok ? (unsigned long long)(cpu / 1000 / (uint64_t)ok) : 0ull);
    printf("jpgdec_irq_before=%ld jpgdec_irq_after=%ld cma_free_kb_before=%ld cma_free_kb_after=%ld cma_free_kb_min=%ld\n", irq0, irq1,
           cma0, cma1, cma_min);
    free(dec);
    free(conv);
    free(wall);
    free(jpeg);
    close_device();
    return ok == count ? 0 : 2;
}

int main(int argc, char **argv) {
    int fd = 0, close_ms = 3000, exit_ms = 30000, count = 20, fmt = JP_FMT_RGBA, dry = 0;
    unsigned shift = 0, flags = 0;
    const char *bench_file = NULL, *dir = NULL;
    int c;
    while ((c = getopt(argc, argv, "f:c:x:A:B:nb:N:z:F:o:LSCH")) != -1) {
        switch (c) {
        case 'f': fd = atoi(optarg); break;
        case 'c': close_ms = atoi(optarg); break;
        case 'x': exit_ms = atoi(optarg); break;
        case 'A': g_align = (unsigned)atoi(optarg); break;
        case 'B': g_valign = (unsigned)atoi(optarg); break;
        case 'n': dry = 1; break;
        case 'b': bench_file = optarg; break;
        case 'N': count = atoi(optarg); break;
        case 'z': shift = (unsigned)atoi(optarg); break;
        case 'L': flags |= JP_FLAG_LIMITED; break;
        case 'S': flags |= JP_FLAG_SWAP_UV; break;
        case 'C': flags |= JP_FLAG_CPU; break;
        case 'H': flags |= JP_FLAG_HW_ONLY; break;
        case 'F':
            fmt = !strcmp(optarg, "nv12") ? JP_FMT_NV12 : !strcmp(optarg, "bgra") ? JP_FMT_BGRA : JP_FMT_RGBA;
            break;
        case 'o': dir = optarg; break;
        default:
            fprintf(stderr, "usage: lumatv-jpeg [-f FD] [-c MS] [-x MS] [-A N] [-B N] | -n | -b FILE [-N COUNT] [-z 0..3] [-F nv12|rgba|bgra] [-o DIR] [-L] [-S] [-C|-H]\n");
            return 64;
        }
    }
    if (g_align < 1 || g_align > 4096 || g_valign < 1 || g_valign > 4096 || shift > 3 || count < 1 || close_ms < 0 || exit_ms < 0) {
        say("a value is out of range");
        return 64;
    }
    if (dry) {
        int rc = load_library();
        printf("library: open=%p close=%p decode=%p\n", (void *)p_open, (void *)p_close, (void *)p_decode);
        return rc ? 1 : 0;
    }
    if (bench_file) return bench(bench_file, count, shift, fmt, dir, flags);
    return serve(fd, close_ms, exit_ms);
}
