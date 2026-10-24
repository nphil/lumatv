// X2 test compositor: a private Wayland socket that accepts LG's 32-bit Mali client buffers. Nothing on screen,
// no DRM master, LG's own socket untouched. Two modes:
//   plain (default): own wl_drm / mali_buffer_sharing / zwp_linux_dmabuf_v1 globals; received dma-bufs are
//                    mmap'd and sampled, so the protocol and the buffer kind are observed directly.
//   --egl <libmali>: EGL on GBM over /dev/dri/renderD128 with the given driver, eglBindWaylandDisplayWL supplies
//                    the buffer globals, and attached buffers are imported with EGL_WAYLAND_BUFFER_WL and read back
//                    through a texture: the import path a replacement compositor would use.
// Usage: x2srv [--rundir /tmp/x2] [--socket wayland-x2] [--no-mali] [--no-drm] [--no-dmabuf] [--afbc]
//              [--alloc-device /dev/dma_buf_te] [--drm-device /dev/dri/renderD128] [--seconds 20] [--egl /usr/lib/libmali.so.0]
#define _GNU_SOURCE
#define X2_SERVER
#include "x2wl.h"
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

struct dma_buf_sync { uint64_t flags; };
#define DMA_BUF_IOCTL_SYNC _IOW('b', 0, struct dma_buf_sync)

static struct wl_display *display;
static int opt_mali = 1, opt_drm = 1, opt_dmabuf = 1, opt_afbc = 0, opt_seconds = 20;
static const char *opt_alloc_device = "/dev/dma_buf_te", *opt_drm_device = "/dev/dri/renderD128", *opt_egl = NULL;
static int commits, frames_sent, buffers_created, pixel_ok, pixel_bad, pixel_unread;

static double now_s(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }
static double t0;
static void logf_(const char *fmt, ...) { va_list ap; va_start(ap, fmt); printf("[srv %7.3f] ", now_s() - t0); vprintf(fmt, ap); printf("\n"); va_end(ap); fflush(stdout); }

enum { KIND_SHM = 1, KIND_DRM_PRIME, KIND_MALI, KIND_DMABUF };
struct buf {
    int kind, fd, w, h, stride; uint32_t fmt; uint32_t extra[6]; uint64_t modifier; off_t size; void *map; struct wl_resource *res;
};
static const char *kind_name(int k) { return k == KIND_DRM_PRIME ? "wl_drm.create_prime_buffer" : k == KIND_MALI ? "mali_buffer_sharing.create_buffer" : k == KIND_DMABUF ? "zwp_linux_dmabuf_v1" : "wl_shm"; }
static void fourcc(uint32_t f, char *s) { for (int i = 0; i < 4; i++) { char c = (f >> (8 * i)) & 255; s[i] = (c >= 32 && c < 127) ? c : '?'; } s[4] = 0; }

static void describe_fd(int fd, struct buf *b) {
    char link[128] = "?", path[64]; snprintf(path, sizeof path, "/proc/self/fd/%d", fd);
    ssize_t n = readlink(path, link, sizeof link - 1); if (n > 0) link[n] = 0;
    struct stat st; off_t sz = -1;
    if (!fstat(fd, &st)) sz = st.st_size;
    off_t end = lseek(fd, 0, SEEK_END); lseek(fd, 0, SEEK_SET);
    b->size = end > 0 ? end : sz;
    logf_("  fd %d: %s, fstat size %lld, lseek end %lld, mode 0%o", fd, link, (long long)sz, (long long)end, st.st_mode & 0170000);
}

static void sample_buffer(struct buf *b, const char *when) {
    if (!b) { logf_("  (no buffer attached)"); return; }
    if (b->kind == KIND_SHM) { logf_("  shm buffer: not sampled"); return; }
    if (!b->map && b->size > 0) {
        b->map = mmap(NULL, b->size, PROT_READ, MAP_SHARED, b->fd, 0);
        if (b->map == MAP_FAILED) { b->map = NULL; logf_("  mmap(fd %d, %lld) failed: %s", b->fd, (long long)b->size, strerror(errno)); }
    }
    if (!b->map) { pixel_unread++; return; }
    struct dma_buf_sync s = { 1 | 2 }; // START | READ
    int sr = ioctl(b->fd, DMA_BUF_IOCTL_SYNC, &s);
    const uint8_t *p = b->map;
    int bpp = 4;
    uint32_t p00 = *(const uint32_t *)(p);
    uint32_t pc = *(const uint32_t *)(p + (b->h / 2) * (size_t)b->stride + (b->w / 2) * bpp);
    uint32_t pl = *(const uint32_t *)(p + (b->h - 1) * (size_t)b->stride + (b->w - 1) * bpp);
    int distinct = 1; uint32_t prev = p00;
    for (int x = 1; x < b->w; x++) { uint32_t v = *(const uint32_t *)(p + x * bpp); if (v != prev) { distinct++; prev = v; } }
    s.flags = 4 | 2; ioctl(b->fd, DMA_BUF_IOCTL_SYNC, &s); // END | READ
    logf_("  %s: pixel(0,0)=%08x center=%08x last=%08x, %d runs in row 0, sync ioctl %s", when, p00, pc, pl, distinct, sr ? strerror(errno) : "ok");
    // The client clears frame k to R=k*25, G=0x80, B=0xff-k*25, A=0xff; a solid first row with G==0x80 and A==0xff
    // in either byte order counts as a correct read.
    uint8_t b0 = p00 & 255, b1 = (p00 >> 8) & 255, b2 = (p00 >> 16) & 255, b3 = p00 >> 24;
    if (distinct <= 3 && b1 == 0x80 && b3 == 0xff && (uint32_t)b0 + b2 == 0xff) pixel_ok++; else pixel_bad++;
}

// ---- wl_buffer (ours) ----
static void buffer_destroy_req(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static const void *buffer_impl[] = { buffer_destroy_req };
static void buffer_res_destroy(struct wl_resource *r) {
    struct buf *b = wl_resource_get_user_data(r);
    if (!b) return;
    logf_("wl_buffer@%u destroyed (%s)", wl_resource_get_id(r), kind_name(b->kind));
    if (b->map) munmap(b->map, b->size);
    if (b->fd >= 0) close(b->fd);
    free(b);
}
static struct wl_resource *make_buffer(struct wl_client *client, uint32_t id, struct buf *b) {
    struct wl_resource *r = wl_resource_create(client, &wl_buffer_interface, 1, id);
    b->res = r;
    wl_resource_set_implementation(r, buffer_impl, b, buffer_res_destroy);
    buffers_created++;
    return r;
}

// ---- mali_buffer_sharing v5: create_buffer(n i i u u u u h), auth(u); event alloc_device(s) ----
static void mali_create_buffer(struct wl_client *c, struct wl_resource *r, uint32_t id, int32_t a1, int32_t a2, uint32_t a3, uint32_t a4, uint32_t a5, uint32_t a6, int32_t fd) {
    char f4[5], f5[5]; fourcc(a4, f4); fourcc(a3, f5);
    logf_("mali_buffer_sharing.create_buffer(new wl_buffer@%u, %d, %d, %u, %u(%s), %u(0x%x), %u(0x%x), fd %d)", id, a1, a2, a3, a4, f4, a5, a5, a6, a6, fd);
    struct buf *b = calloc(1, sizeof *b);
    b->kind = KIND_MALI; b->fd = fd; b->w = a1; b->h = a2; b->stride = a3; b->fmt = a4; b->extra[0] = a5; b->extra[1] = a6;
    describe_fd(fd, b);
    make_buffer(c, id, b);
}
static void mali_auth(struct wl_client *c, struct wl_resource *r, uint32_t magic) { logf_("mali_buffer_sharing.auth(%u)", magic); }
static const void *mali_impl[] = { mali_create_buffer, mali_auth };
static void mali_bind(struct wl_client *client, void *data, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(client, &mali_buffer_sharing_interface, version, id);
    wl_resource_set_implementation(r, mali_impl, NULL, NULL);
    logf_("client bound mali_buffer_sharing v%u; sending alloc_device(\"%s\")", version, opt_alloc_device);
    wl_resource_post_event(r, 0, opt_alloc_device);
}

// ---- wl_drm v2 ----
static void drm_authenticate(struct wl_client *c, struct wl_resource *r, uint32_t magic) { logf_("wl_drm.authenticate(%u) -> authenticated", magic); wl_resource_post_event(r, 2); }
static void drm_create_buffer(struct wl_client *c, struct wl_resource *r, uint32_t id, uint32_t name, int32_t w, int32_t h, uint32_t stride, uint32_t fmt) {
    logf_("wl_drm.create_buffer(new wl_buffer@%u, name %u, %dx%d, stride %u, fmt 0x%x): GEM names are not importable here", id, name, w, h, stride, fmt);
    wl_resource_post_error(r, 0, "no GEM name import");
}
static void drm_create_planar(struct wl_client *c, struct wl_resource *r, uint32_t id, uint32_t name, int32_t w, int32_t h, uint32_t fmt, int32_t o0, int32_t s0, int32_t o1, int32_t s1, int32_t o2, int32_t s2) {
    logf_("wl_drm.create_planar_buffer(new wl_buffer@%u, name %u, %dx%d, fmt 0x%x)", id, name, w, h, fmt);
    wl_resource_post_error(r, 0, "no GEM name import");
}
static void drm_create_prime(struct wl_client *c, struct wl_resource *r, uint32_t id, int32_t fd, int32_t w, int32_t h, uint32_t fmt, int32_t o0, int32_t s0, int32_t o1, int32_t s1, int32_t o2, int32_t s2) {
    char f4[5]; fourcc(fmt, f4);
    logf_("wl_drm.create_prime_buffer(new wl_buffer@%u, fd %d, %dx%d, fmt 0x%x(%s), off0 %d stride0 %d, off1 %d stride1 %d, off2 %d stride2 %d)", id, fd, w, h, fmt, f4, o0, s0, o1, s1, o2, s2);
    struct buf *b = calloc(1, sizeof *b);
    b->kind = KIND_DRM_PRIME; b->fd = fd; b->w = w; b->h = h; b->stride = s0; b->fmt = fmt;
    describe_fd(fd, b);
    make_buffer(c, id, b);
}
static const void *drm_impl[] = { drm_authenticate, drm_create_buffer, drm_create_planar, drm_create_prime };
static void drm_bind(struct wl_client *client, void *data, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(client, &wl_drm_interface, version, id);
    wl_resource_set_implementation(r, drm_impl, NULL, NULL);
    logf_("client bound wl_drm v%u; sending device(\"%s\"), formats, capabilities(prime)", version, opt_drm_device);
    wl_resource_post_event(r, 0, opt_drm_device);
    uint32_t fmts[] = { 0x34325241, 0x34325258, 0x34324241, 0x34324258 }; // AR24 XR24 AB24 XB24
    for (unsigned i = 0; i < 4; i++) wl_resource_post_event(r, 1, fmts[i]);
    if (version >= 2) wl_resource_post_event(r, 3, 1u);
}

// ---- zwp_linux_dmabuf_v1 v3 ----
struct params { struct buf pending; int planes; int used; };
static void params_destroy(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static void params_add(struct wl_client *c, struct wl_resource *r, int32_t fd, uint32_t plane, uint32_t off, uint32_t stride, uint32_t mhi, uint32_t mlo) {
    struct params *p = wl_resource_get_user_data(r);
    logf_("zwp_linux_buffer_params_v1.add(fd %d, plane %u, off %u, stride %u, modifier 0x%08x%08x)", fd, plane, off, stride, mhi, mlo);
    if (plane == 0) { p->pending.fd = fd; p->pending.stride = stride; p->pending.modifier = ((uint64_t)mhi << 32) | mlo; } else close(fd);
    p->planes++;
}
static void params_finish(struct wl_client *c, struct wl_resource *r, uint32_t id, int32_t w, int32_t h, uint32_t fmt, uint32_t flags, int immed) {
    struct params *p = wl_resource_get_user_data(r);
    char f4[5]; fourcc(fmt, f4);
    logf_("zwp_linux_buffer_params_v1.%s(%dx%d, fmt 0x%x(%s), flags %u) with %d plane(s)", immed ? "create_immed" : "create", w, h, fmt, f4, flags, p->planes);
    struct buf *b = calloc(1, sizeof *b); *b = p->pending; b->kind = KIND_DMABUF; b->w = w; b->h = h; b->fmt = fmt; b->map = NULL;
    describe_fd(b->fd, b);
    p->used = 1;
    if (immed) make_buffer(c, id, b);
    else { struct wl_resource *br = make_buffer(c, 0, b); wl_resource_post_event(r, 0, br); }
}
static void params_create(struct wl_client *c, struct wl_resource *r, int32_t w, int32_t h, uint32_t fmt, uint32_t flags) { params_finish(c, r, 0, w, h, fmt, flags, 0); }
static void params_create_immed(struct wl_client *c, struct wl_resource *r, uint32_t id, int32_t w, int32_t h, uint32_t fmt, uint32_t flags) { params_finish(c, r, id, w, h, fmt, flags, 1); }
static const void *params_impl[] = { params_destroy, params_add, params_create, params_create_immed };
static void params_res_destroy(struct wl_resource *r) { struct params *p = wl_resource_get_user_data(r); if (p && !p->used && p->pending.fd > 0) close(p->pending.fd); free(p); }
static void dmabuf_destroy(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static void dmabuf_create_params(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct wl_resource *pr = wl_resource_create(c, &zwp_linux_buffer_params_v1_interface, wl_resource_get_version(r), id);
    struct params *p = calloc(1, sizeof *p); p->pending.fd = -1;
    wl_resource_set_implementation(pr, params_impl, p, params_res_destroy);
}
static const void *dmabuf_impl[] = { dmabuf_destroy, dmabuf_create_params };
static void dmabuf_bind(struct wl_client *client, void *data, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(client, &zwp_linux_dmabuf_v1_interface, version, id);
    wl_resource_set_implementation(r, dmabuf_impl, NULL, NULL);
    logf_("client bound zwp_linux_dmabuf_v1 v%u; sending formats (linear%s)", version, opt_afbc ? " + AFBC 32x8 split sparse" : "");
    uint32_t fmts[] = { 0x34325241, 0x34325258, 0x34324241, 0x34324258 };
    for (unsigned i = 0; i < 4; i++) {
        if (version >= 3) {
            wl_resource_post_event(r, 1, fmts[i], 0u, 0u);
            if (opt_afbc) wl_resource_post_event(r, 1, fmts[i], 0x08000000u, 0x00000062u);
        } else wl_resource_post_event(r, 0, fmts[i]);
    }
}

// ---- EGL import path (--egl) ----
typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext, *EGLImage;
typedef int EGLint;
static void *egl_lib;
static EGLDisplay egl_dpy;
static EGLContext egl_ctx;
static unsigned (*p_eglBindWaylandDisplayWL)(EGLDisplay, struct wl_display *);
static unsigned (*p_eglQueryWaylandBufferWL)(EGLDisplay, struct wl_resource *, EGLint, EGLint *);
static EGLImage (*p_eglCreateImageKHR)(EGLDisplay, EGLContext, unsigned, void *, const EGLint *);
static unsigned (*p_eglDestroyImageKHR)(EGLDisplay, EGLImage);
static EGLint (*p_eglGetError)(void);
static const char *(*p_eglQueryString)(EGLDisplay, EGLint);
static void (*p_glEGLImageTargetTexture2DOES)(unsigned, void *);
static void (*p_glGenTextures)(int, unsigned *);
static void (*p_glBindTexture)(unsigned, unsigned);
static void (*p_glGenFramebuffers)(int, unsigned *);
static void (*p_glBindFramebuffer)(unsigned, unsigned);
static void (*p_glFramebufferTexture2D)(unsigned, unsigned, unsigned, unsigned, int);
static unsigned (*p_glCheckFramebufferStatus)(unsigned);
static void (*p_glReadPixels)(int, int, int, int, unsigned, unsigned, void *);
static unsigned (*p_glGetError)(void);
static void (*p_glFinish)(void);

static int egl_init(const char *lib) {
    egl_lib = dlopen(lib, RTLD_NOW | RTLD_GLOBAL);
    if (!egl_lib) { logf_("dlopen(%s): %s", lib, dlerror()); return -1; }
    void *(*gpa)(const char *) = dlsym(egl_lib, "eglGetProcAddress");
#define S(n) *(void **)&p_##n = dlsym(egl_lib, #n); if (!p_##n && gpa) *(void **)&p_##n = gpa(#n); if (!p_##n) { logf_("missing %s", #n); return -1; }
    S(eglGetError) S(eglQueryString) S(eglBindWaylandDisplayWL) S(eglQueryWaylandBufferWL) S(eglCreateImageKHR) S(eglDestroyImageKHR)
    S(glEGLImageTargetTexture2DOES) S(glGenTextures) S(glBindTexture) S(glGenFramebuffers) S(glBindFramebuffer) S(glFramebufferTexture2D)
    S(glCheckFramebufferStatus) S(glReadPixels) S(glGetError) S(glFinish)
    void *(*gbm_create_device)(int) = dlsym(egl_lib, "gbm_create_device");
    EGLDisplay (*getpd)(unsigned, void *, const EGLint *) = dlsym(egl_lib, "eglGetPlatformDisplayEXT");
    if (!getpd && gpa) *(void **)&getpd = gpa("eglGetPlatformDisplayEXT");
    if (!getpd) *(void **)&getpd = dlsym(egl_lib, "eglGetPlatformDisplay");
    unsigned (*init)(EGLDisplay, EGLint *, EGLint *) = dlsym(egl_lib, "eglInitialize");
    unsigned (*choose)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *) = dlsym(egl_lib, "eglChooseConfig");
    EGLContext (*cctx)(EGLDisplay, EGLConfig, EGLContext, const EGLint *) = dlsym(egl_lib, "eglCreateContext");
    EGLSurface (*cpb)(EGLDisplay, EGLConfig, const EGLint *) = dlsym(egl_lib, "eglCreatePbufferSurface");
    unsigned (*mc)(EGLDisplay, EGLSurface, EGLSurface, EGLContext) = dlsym(egl_lib, "eglMakeCurrent");
    unsigned (*bindapi)(unsigned) = dlsym(egl_lib, "eglBindAPI");
    if (!gbm_create_device || !getpd || !init || !choose || !cctx || !cpb || !mc || !bindapi) { logf_("missing EGL/GBM entry points"); return -1; }
    int fd = open(opt_drm_device, O_RDWR | O_CLOEXEC);
    if (fd < 0) { logf_("open %s: %s", opt_drm_device, strerror(errno)); return -1; }
    void *gbm = gbm_create_device(fd);
    logf_("gbm_create_device(%s) = %p", opt_drm_device, gbm);
    egl_dpy = getpd(0x31D7 /* EGL_PLATFORM_GBM_KHR */, gbm, NULL);
    EGLint maj = 0, min = 0;
    if (!egl_dpy || !init(egl_dpy, &maj, &min)) { logf_("eglInitialize on GBM failed: 0x%x", p_eglGetError()); return -1; }
    logf_("EGL %d.%d %s | %s", maj, min, p_eglQueryString(egl_dpy, 0x3053), p_eglQueryString(egl_dpy, 0x3054));
    logf_("EGL extensions: %s", p_eglQueryString(egl_dpy, 0x3055));
    bindapi(0x30A0);
    EGLint ca[] = { 0x3033, 0x0001 /* PBUFFER */, 0x3040, 0x0004 /* ES2 */, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038 };
    EGLConfig cfg = NULL; EGLint n = 0;
    EGLint xa[] = { 0x3098, 2, 0x3038 };
    if (choose(egl_dpy, ca, &cfg, 1, &n) && n) {
        egl_ctx = cctx(egl_dpy, cfg, NULL, xa);
        EGLint pa[] = { 0x3057, 4, 0x3056, 4, 0x3038 };
        EGLSurface pb = cpb(egl_dpy, cfg, pa);
        if (!egl_ctx || !pb || !mc(egl_dpy, pb, pb, egl_ctx)) { logf_("context/pbuffer/makecurrent: 0x%x", p_eglGetError()); return -1; }
        logf_("pbuffer context current");
    } else {
        // No pbuffer config (the 64-bit blob without a GBM device): EGL_KHR_no_config_context + EGL_KHR_surfaceless_context.
        logf_("no pbuffer config (0x%x); using a no-config surfaceless context", p_eglGetError());
        egl_ctx = cctx(egl_dpy, NULL, NULL, xa);
        if (!egl_ctx || !mc(egl_dpy, NULL, NULL, egl_ctx)) { logf_("surfaceless context/makecurrent: 0x%x", p_eglGetError()); return -1; }
        logf_("surfaceless context current");
    }
    if (!p_eglBindWaylandDisplayWL(egl_dpy, display)) { logf_("eglBindWaylandDisplayWL failed: 0x%x", p_eglGetError()); return -1; }
    logf_("eglBindWaylandDisplayWL ok: the driver now serves its own buffer globals on this socket");
    return 0;
}

static void egl_sample(struct wl_resource *buffer) {
    EGLint w = 0, h = 0, fmt = 0, inv = 0;
    unsigned q1 = p_eglQueryWaylandBufferWL(egl_dpy, buffer, 0x3057, &w), q2 = p_eglQueryWaylandBufferWL(egl_dpy, buffer, 0x3056, &h);
    unsigned q3 = p_eglQueryWaylandBufferWL(egl_dpy, buffer, 0x3080 /* EGL_TEXTURE_FORMAT */, &fmt);
    unsigned q4 = p_eglQueryWaylandBufferWL(egl_dpy, buffer, 0x31DB /* EGL_WAYLAND_Y_INVERTED_WL */, &inv);
    logf_("  eglQueryWaylandBufferWL: width %d (%u) height %d (%u) texture_format 0x%x (%u) y_inverted %d (%u)", w, q1, h, q2, fmt, q3, inv, q4);
    EGLImage img = p_eglCreateImageKHR(egl_dpy, NULL, 0x31D5 /* EGL_WAYLAND_BUFFER_WL */, buffer, NULL);
    if (!img) { logf_("  eglCreateImageKHR(EGL_WAYLAND_BUFFER_WL) failed: 0x%x", p_eglGetError()); pixel_unread++; return; }
    unsigned tex = 0, fbo = 0;
    p_glGenTextures(1, &tex); p_glBindTexture(0x0DE1, tex);
    p_glEGLImageTargetTexture2DOES(0x0DE1, img);
    p_glGenFramebuffers(1, &fbo); p_glBindFramebuffer(0x8D40, fbo);
    p_glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, tex, 0);
    unsigned st = p_glCheckFramebufferStatus(0x8D40);
    if (st != 0x8CD5) { logf_("  framebuffer incomplete 0x%x (gl error 0x%x)", st, p_glGetError()); pixel_unread++; p_eglDestroyImageKHR(egl_dpy, img); return; }
    uint8_t px[3][4];
    p_glReadPixels(0, 0, 1, 1, 0x1908, 0x1401, px[0]);
    p_glReadPixels(w / 2, h / 2, 1, 1, 0x1908, 0x1401, px[1]);
    p_glReadPixels(w - 1, h - 1, 1, 1, 0x1908, 0x1401, px[2]);
    p_glFinish();
    unsigned err = p_glGetError();
    logf_("  EGL import + glReadPixels RGBA: (0,0)=%02x%02x%02x%02x center=%02x%02x%02x%02x last=%02x%02x%02x%02x gl error 0x%x", px[0][0], px[0][1], px[0][2], px[0][3], px[1][0], px[1][1], px[1][2], px[1][3], px[2][0], px[2][1], px[2][2], px[2][3], err);
    if (!err && px[0][1] == 0x80 && px[0][3] == 0xff && (unsigned)px[0][0] + px[0][2] == 0xff && !memcmp(px[0], px[1], 4)) pixel_ok++; else pixel_bad++;
    p_eglDestroyImageKHR(egl_dpy, img);
}

// ---- wl_surface / wl_compositor / wl_shell ----
struct surface { struct wl_resource *attached; struct wl_resource *frame_cb; int commits; };
static void surf_destroy(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static void surf_attach(struct wl_client *c, struct wl_resource *r, struct wl_resource *buffer, int32_t x, int32_t y) {
    struct surface *s = wl_resource_get_user_data(r);
    s->attached = buffer;
    logf_("wl_surface@%u.attach(wl_buffer@%u, %d, %d)", wl_resource_get_id(r), buffer ? wl_resource_get_id(buffer) : 0, x, y);
}
static void surf_damage(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y, int32_t w, int32_t h) { logf_("wl_surface.damage(%d,%d %dx%d)", x, y, w, h); }
static void surf_frame(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct surface *s = wl_resource_get_user_data(r);
    s->frame_cb = wl_resource_create(c, &wl_callback_interface, 1, id);
    wl_resource_set_implementation(s->frame_cb, NULL, NULL, NULL);
}
static void surf_region(struct wl_client *c, struct wl_resource *r, struct wl_resource *reg) {}
static void surf_commit(struct wl_client *c, struct wl_resource *r) {
    struct surface *s = wl_resource_get_user_data(r);
    s->commits++; commits++;
    logf_("wl_surface@%u.commit #%d", wl_resource_get_id(r), s->commits);
    if (s->attached) {
        if (opt_egl) egl_sample(s->attached);
        else if (wl_shm_buffer_get(s->attached)) { logf_("  shm buffer attached: not sampled"); pixel_unread++; }
        else sample_buffer(wl_resource_get_user_data(s->attached), "mmap sample");
        wl_resource_post_event(s->attached, 0); // wl_buffer.release: we have read it
    }
    if (s->frame_cb) {
        struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
        wl_resource_post_event(s->frame_cb, 0, (uint32_t)(t.tv_sec * 1000 + t.tv_nsec / 1000000));
        wl_resource_destroy(s->frame_cb); s->frame_cb = NULL; frames_sent++;
    }
}
static void surf_transform(struct wl_client *c, struct wl_resource *r, int32_t t) {}
static void surf_scale(struct wl_client *c, struct wl_resource *r, int32_t sc) {}
static void surf_damage_buffer(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y, int32_t w, int32_t h) { logf_("wl_surface.damage_buffer(%d,%d %dx%d)", x, y, w, h); }
static void surf_offset(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y) {}
static const void *surface_impl[] = { surf_destroy, surf_attach, surf_damage, surf_frame, surf_region, surf_region, surf_commit, surf_transform, surf_scale, surf_damage_buffer, surf_offset };
static void surface_res_destroy(struct wl_resource *r) { free(wl_resource_get_user_data(r)); }

static void region_destroy(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static void region_rect(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y, int32_t w, int32_t h) {}
static const void *region_impl[] = { region_destroy, region_rect, region_rect };

static void comp_create_surface(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct wl_resource *sr = wl_resource_create(c, &wl_surface_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(sr, surface_impl, calloc(1, sizeof(struct surface)), surface_res_destroy);
    logf_("wl_compositor.create_surface -> wl_surface@%u", id);
}
static void comp_create_region(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct wl_resource *rr = wl_resource_create(c, &wl_region_interface, 1, id);
    wl_resource_set_implementation(rr, region_impl, NULL, NULL);
}
static const void *compositor_impl[] = { comp_create_surface, comp_create_region };
static void compositor_bind(struct wl_client *client, void *data, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(client, &wl_compositor_interface, version, id);
    wl_resource_set_implementation(r, compositor_impl, NULL, NULL);
}

static void ss_noop(struct wl_client *c, struct wl_resource *r, ...) {}
static void ss_set_toplevel(struct wl_client *c, struct wl_resource *r) { logf_("wl_shell_surface@%u.set_toplevel", wl_resource_get_id(r)); }
static void ss_set_title(struct wl_client *c, struct wl_resource *r, const char *t) { logf_("wl_shell_surface.set_title(\"%s\")", t); }
static const void *shell_surface_impl[] = { ss_noop, ss_noop, ss_noop, ss_set_toplevel, ss_noop, ss_noop, ss_noop, ss_noop, ss_set_title, ss_noop };
static void shell_get_shell_surface(struct wl_client *c, struct wl_resource *r, uint32_t id, struct wl_resource *surface) {
    struct wl_resource *sr = wl_resource_create(c, &wl_shell_surface_interface, 1, id);
    wl_resource_set_implementation(sr, shell_surface_impl, NULL, NULL);
    wl_resource_post_event(sr, 1, 0u, 256, 256); // configure
}
static const void *shell_impl[] = { shell_get_shell_surface };
static void shell_bind(struct wl_client *client, void *data, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(client, &wl_shell_interface, version, id);
    wl_resource_set_implementation(r, shell_impl, NULL, NULL);
}

// ---- lifecycle ----
static struct wl_listener client_destroy_l, client_created_l;
static void on_client_destroy(struct wl_listener *l, void *data) { logf_("client disconnected"); wl_display_terminate(display); }
static void on_client_created(struct wl_listener *l, void *data) {
    struct wl_client *c = data; int32_t pid = 0; uint32_t uid = 0, gid = 0;
    wl_client_get_credentials(c, &pid, &uid, &gid);
    logf_("client connected: pid %d uid %u", pid, uid);
    client_destroy_l.notify = on_client_destroy;
    wl_client_add_destroy_listener(c, &client_destroy_l);
}
static int on_timeout(void *data) { logf_("timeout after %d s", opt_seconds); wl_display_terminate(display); return 0; }

int main(int argc, char **argv) {
    const char *rundir = "/tmp/x2", *sock = "wayland-x2";
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--rundir")) rundir = argv[++i];
        else if (!strcmp(argv[i], "--socket")) sock = argv[++i];
        else if (!strcmp(argv[i], "--no-mali")) opt_mali = 0;
        else if (!strcmp(argv[i], "--no-drm")) opt_drm = 0;
        else if (!strcmp(argv[i], "--no-dmabuf")) opt_dmabuf = 0;
        else if (!strcmp(argv[i], "--afbc")) opt_afbc = 1;
        else if (!strcmp(argv[i], "--alloc-device")) opt_alloc_device = argv[++i];
        else if (!strcmp(argv[i], "--drm-device")) opt_drm_device = argv[++i];
        else if (!strcmp(argv[i], "--seconds")) opt_seconds = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--egl")) opt_egl = argv[++i];
        else { fprintf(stderr, "unknown option %s\n", argv[i]); return 2; }
    }
    t0 = now_s();
    mkdir(rundir, 0700);
    setenv("XDG_RUNTIME_DIR", rundir, 1);
    display = wl_display_create();
    if (!display) { logf_("wl_display_create failed"); return 1; }
    if (wl_display_add_socket(display, sock)) { logf_("wl_display_add_socket(%s/%s): %s", rundir, sock, strerror(errno)); return 1; }
    wl_display_init_shm(display);
    wl_global_create(display, &wl_compositor_interface, 3, NULL, compositor_bind);
    wl_global_create(display, &wl_shell_interface, 1, NULL, shell_bind);
    if (opt_egl) {
        if (egl_init(opt_egl)) return 1;
    } else {
        if (opt_mali) wl_global_create(display, &mali_buffer_sharing_interface, 5, NULL, mali_bind);
        if (opt_drm) wl_global_create(display, &wl_drm_interface, 2, NULL, drm_bind);
        if (opt_dmabuf) wl_global_create(display, &zwp_linux_dmabuf_v1_interface, 3, NULL, dmabuf_bind);
    }
    client_created_l.notify = on_client_created;
    wl_display_add_client_created_listener(display, &client_created_l);
    struct wl_event_source *timer = wl_event_loop_add_timer(wl_display_get_event_loop(display), on_timeout, NULL);
    wl_event_source_timer_update(timer, opt_seconds * 1000);
    logf_("listening on %s/%s (%s%s%s%s)", rundir, sock, opt_egl ? "buffer globals from the driver" : "own globals:",
          !opt_egl && opt_mali ? " mali_buffer_sharing" : "", !opt_egl && opt_drm ? " wl_drm" : "", !opt_egl && opt_dmabuf ? " linux-dmabuf" : "");
    wl_display_run(display);
    logf_("summary: %d buffers created, %d commits, %d frame callbacks, pixel checks ok %d bad %d unread %d", buffers_created, commits, frames_sent, pixel_ok, pixel_bad, pixel_unread);
    wl_display_destroy(display);
    char path[256]; snprintf(path, sizeof path, "%s/%s", rundir, sock); unlink(path); snprintf(path, sizeof path, "%s/%s.lock", rundir, sock); unlink(path);
    return 0;
}
