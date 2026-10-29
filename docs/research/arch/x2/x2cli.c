// X2 test client: a 32-bit Wayland client that draws with LG's own libmali (the library every LG app uses) into
// a window on the X2 private socket. Logs the registry, EGL/GL strings and the per-frame result. Frame k is cleared
// to R=k*25 G=0x80 B=0xff-k*25 A=0xff so the server can check what it received.
// Usage: x2cli <libmali.so> [--frames 6] [--size WxH] [--no-egl] [--interval 0|1] [--pattern] [--opaque]
// --pattern draws the direct scan-out check's picture (docs/research/scanout-research.md, TV check list b) instead of the colour clear: bars, a white
// frame, a marker in each corner and the frame number twice, so a frame shown torn or half drawn reads as two numbers. --opaque asks for a window
// configuration without alpha (an XB24 buffer, opaque by its format), which the plane takes without a declared opaque region.
// Run with WAYLAND_DISPLAY=wayland-x2 XDG_RUNTIME_DIR=/tmp/x2 WAYLAND_DEBUG=1 to see every request libmali makes.
#define _GNU_SOURCE
#define X2_CLIENT
#include "x2wl.h"
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#ifndef MFD_CLOEXEC
int memfd_create(const char *name, unsigned flags);
#endif
#include <time.h>
#include <unistd.h>

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext;
typedef int EGLint;
#define L(name, ret, ...) static ret (*name)(__VA_ARGS__);
L(eglGetDisplay, EGLDisplay, void *)
L(eglGetPlatformDisplayEXT, EGLDisplay, unsigned, void *, const EGLint *)
L(eglInitialize, unsigned, EGLDisplay, EGLint *, EGLint *)
L(eglQueryString, const char *, EGLDisplay, EGLint)
L(eglChooseConfig, unsigned, EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *)
L(eglCreateWindowSurface, EGLSurface, EGLDisplay, EGLConfig, void *, const EGLint *)
L(eglCreateContext, EGLContext, EGLDisplay, EGLConfig, EGLContext, const EGLint *)
L(eglMakeCurrent, unsigned, EGLDisplay, EGLSurface, EGLSurface, EGLContext)
L(eglSwapBuffers, unsigned, EGLDisplay, EGLSurface)
L(eglSwapInterval, unsigned, EGLDisplay, EGLint)
L(eglGetError, EGLint, void)
L(eglBindAPI, unsigned, unsigned)
L(eglGetProcAddress, void *, const char *)
L(eglTerminate, unsigned, EGLDisplay)
L(glGetString, const unsigned char *, unsigned)
L(glClearColor, void, float, float, float, float)
L(glClear, void, unsigned)
L(glScissor, void, int, int, int, int)
L(glEnable, void, unsigned)
L(glViewport, void, int, int, int, int)
L(glFinish, void, void)
L(wl_egl_window_create, void *, struct wl_surface *, int, int)
L(wl_egl_window_destroy, void, void *)

static struct wl_compositor *compositor;
static struct wl_shell *shell;
static struct wl_shm *shm;
static int nglobals;

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    printf("[cli] global %2u %-36s v%u\n", name, iface, ver); nglobals++;
    if (!strcmp(iface, "wl_compositor")) compositor = wl_registry_bind(r, name, &wl_compositor_interface, 3);
    else if (!strcmp(iface, "wl_shell")) shell = wl_registry_bind(r, name, &wl_shell_interface, 1);
    else if (!strcmp(iface, "wl_shm")) shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) {}
static void (*registry_listener[])(void) = { (void (*)(void))global, (void (*)(void))global_remove };

// One filled rectangle through the scissor box (GL counts rows from the bottom): the pattern needs no shader, only clears.
static void rect(int x, int y, int w, int h, float r, float g, float b) {
    glScissor(x, y, w, h);
    glClearColor(r, g, b, 1.0f);
    glClear(0x4000);
}

// Frame k of the direct scan-out check's picture: eight colour bars across the middle third, a 4 px white frame on the edges, a 96 px marker in every
// corner (top left red, top right green, bottom left blue, bottom right white), a bar that moves 16 px a frame along the middle, and k in twelve bits
// (64x48 cells, white one) as one row at the top and the same at the bottom: a torn frame shows two different numbers.
static void pattern(int W, int H, int k) {
    static const float bars[8][3] = {{1, 1, 1}, {1, 1, 0}, {0, 1, 1}, {0, 1, 0}, {1, 0, 1}, {1, 0, 0}, {0, 0, 1}, {0, 0, 0}};
    glEnable(0x0C11 /* GL_SCISSOR_TEST */);
    rect(0, 0, W, H, 0.12f, 0.12f, 0.12f);
    for (int i = 0; i < 8; i++) rect(i * W / 8, H / 3, W / 8, H / 3, bars[i][0], bars[i][1], bars[i][2]);
    rect(0, 0, W, 4, 1, 1, 1);
    rect(0, H - 4, W, 4, 1, 1, 1);
    rect(0, 0, 4, H, 1, 1, 1);
    rect(W - 4, 0, 4, H, 1, 1, 1);
    rect(8, H - 8 - 96, 96, 96, 1, 0, 0);
    rect(W - 8 - 96, H - 8 - 96, 96, 96, 0, 1, 0);
    rect(8, 8, 96, 96, 0, 0, 1);
    rect(W - 8 - 96, 8, 96, 96, 1, 1, 1);
    for (int bit = 0; bit < 12; bit++) {
        float v = ((k >> bit) & 1) ? 1.0f : 0.0f;
        rect(160 + bit * 72, H - 8 - 48, 64, 48, v, v, v);
        rect(160 + bit * 72, 8, 64, 48, v, v, v);
    }
    rect((k * 16) % (W - 64), H / 2 - 24, 64, 48, 1, 0.5f, 0);
}

static void ping(void *d, struct wl_shell_surface *s, uint32_t serial) { wl_shell_surface_pong(s, serial); }
static void configure(void *d, struct wl_shell_surface *s, uint32_t e, int32_t w, int32_t h) { printf("[cli] configure %dx%d\n", w, h); }
static void popup_done(void *d, struct wl_shell_surface *s) {}
static void (*ss_listener[])(void) = { (void (*)(void))ping, (void (*)(void))configure, (void (*)(void))popup_done };

static int frame_done;
static void cb_done(void *d, struct wl_callback *cb, uint32_t t) { frame_done++; wl_proxy_destroy(P(cb)); }
static void (*cb_listener[])(void) = { (void (*)(void))cb_done };

static double now_s(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

int main(int argc, char **argv) {
    const char *lib = argc > 1 ? argv[1] : "/usr/lib/libmali.so.0";
    int frames = 6, W = 256, H = 256, use_egl = 1, interval = 1, use_pattern = 0, opaque = 0;
    for (int i = 2; i < argc; i++) {
        if (!strcmp(argv[i], "--frames")) frames = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--size")) sscanf(argv[++i], "%dx%d", &W, &H);
        else if (!strcmp(argv[i], "--no-egl")) use_egl = 0;
        else if (!strcmp(argv[i], "--interval")) interval = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--pattern")) use_pattern = 1;
        else if (!strcmp(argv[i], "--opaque")) opaque = 1;
    }
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) { printf("[cli] wl_display_connect failed: %s\n", strerror(errno)); return 2; }
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_proxy_add_listener(P(reg), registry_listener, NULL);
    wl_display_roundtrip(dpy);
    printf("[cli] %d globals; compositor %p shell %p shm %p\n", nglobals, (void *)compositor, (void *)shell, (void *)shm);
    if (!compositor || !shell) { printf("[cli] missing globals\n"); return 3; }
    struct wl_surface *surf = wl_compositor_create_surface(compositor);
    struct wl_shell_surface *ss = wl_shell_get_shell_surface(shell, surf);
    wl_proxy_add_listener(P(ss), ss_listener, NULL);
    wl_shell_surface_set_title(ss, "x2 probe");
    wl_shell_surface_set_toplevel(ss);

    if (!use_egl) {
        // Plumbing check without the GPU: one wl_shm buffer, one commit, one frame callback.
        if (!shm) { printf("[cli] no wl_shm\n"); return 3; }
        int fd = memfd_create("x2", 0); if (fd < 0) { char t[] = "/tmp/x2/shm-XXXXXX"; fd = mkstemp(t); unlink(t); }
        int stride = W * 4, size = stride * H;
        ftruncate(fd, size);
        uint32_t *px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        for (int i = 0; i < W * H; i++) px[i] = 0xff8000ffu;
        struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, size);
        struct wl_buffer *b = wl_shm_pool_create_buffer(pool, 0, W, H, stride, 0 /* ARGB8888 */);
        struct wl_callback *cb = wl_surface_frame(surf); wl_proxy_add_listener(P(cb), cb_listener, NULL);
        wl_surface_attach(surf, b, 0, 0); wl_surface_damage(surf, 0, 0, W, H); wl_surface_commit(surf);
        while (!frame_done && wl_display_dispatch(dpy) != -1) {}
        printf("[cli] shm frame acknowledged: %d\n", frame_done);
        wl_buffer_destroy(b);
        wl_display_roundtrip(dpy);
        wl_display_disconnect(dpy);
        return 0;
    }

    void *h = dlopen(lib, RTLD_NOW | RTLD_GLOBAL);
    if (!h) { printf("[cli] dlopen(%s): %s\n", lib, dlerror()); return 1; }
#define S(name) *(void **)&name = dlsym(h, #name); if (!name) { printf("[cli] missing %s\n", #name); return 1; }
    S(eglGetDisplay) S(eglInitialize) S(eglQueryString) S(eglChooseConfig) S(eglCreateWindowSurface) S(eglCreateContext)
    S(eglMakeCurrent) S(eglSwapBuffers) S(eglSwapInterval) S(eglGetError) S(eglBindAPI) S(eglGetProcAddress) S(eglTerminate)
    S(glGetString) S(glClearColor) S(glClear) S(glViewport) S(glFinish) S(glScissor) S(glEnable)
    *(void **)&eglGetPlatformDisplayEXT = dlsym(h, "eglGetPlatformDisplayEXT");
    void *we = dlopen("libwayland-egl.so.1", RTLD_NOW | RTLD_GLOBAL);
    if (we) { *(void **)&wl_egl_window_create = dlsym(we, "wl_egl_window_create"); *(void **)&wl_egl_window_destroy = dlsym(we, "wl_egl_window_destroy"); }
    if (!wl_egl_window_create) { printf("[cli] missing wl_egl_window_create\n"); return 1; }

    printf("[cli] EGL client extensions: %s\n", eglQueryString(NULL, 0x3055) ? eglQueryString(NULL, 0x3055) : "(none)");
    printf("[cli] eglBindWaylandDisplayWL via eglGetProcAddress: %p\n", eglGetProcAddress("eglBindWaylandDisplayWL"));
    EGLDisplay d = eglGetPlatformDisplayEXT ? eglGetPlatformDisplayEXT(0x31D8 /* EGL_PLATFORM_WAYLAND_EXT */, dpy, NULL) : NULL;
    if (!d) d = eglGetDisplay(dpy);
    EGLint maj = 0, min = 0;
    if (!d || !eglInitialize(d, &maj, &min)) { printf("[cli] eglInitialize: 0x%x\n", eglGetError()); return 4; }
    printf("[cli] EGL %d.%d vendor \"%s\" version \"%s\"\n", maj, min, eglQueryString(d, 0x3053), eglQueryString(d, 0x3054));
    printf("[cli] EGL display extensions: %s\n", eglQueryString(d, 0x3055));
    fflush(stdout);
    EGLint ca[] = { 0x3033, 0x0004 /* WINDOW */, 0x3040, 0x0004 /* ES2 */, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, opaque ? 0 : 8, 0x3038 };
    EGLConfig cfg; EGLint n = 0;
    if (!eglChooseConfig(d, ca, &cfg, 1, &n) || !n) { printf("[cli] no config 0x%x\n", eglGetError()); return 5; }
    void *ew = wl_egl_window_create(surf, W, H);
    EGLSurface es = eglCreateWindowSurface(d, cfg, ew, NULL);
    if (!es) { printf("[cli] eglCreateWindowSurface failed: 0x%x\n", eglGetError()); return 6; }
    eglBindAPI(0x30A0);
    EGLint xa[] = { 0x3098, 2, 0x3038 };
    EGLContext c = eglCreateContext(d, cfg, NULL, xa);
    if (!c || !eglMakeCurrent(d, es, es, c)) { printf("[cli] context/makecurrent 0x%x\n", eglGetError()); return 6; }
    eglSwapInterval(d, interval);
    printf("[cli] GL \"%s\" / \"%s\"\n", glGetString(0x1F01), glGetString(0x1F02));
    fflush(stdout);
    double t0 = now_s();
    for (int k = 0; k < frames; k++) {
        glViewport(0, 0, W, H);
        if (use_pattern) {
            pattern(W, H, k);
        } else {
            glClearColor((k * 25) / 255.0f, 0x80 / 255.0f, (0xff - k * 25) / 255.0f, 1.0f);
            glClear(0x4000);
        }
        double ts = now_s();
        unsigned ok = eglSwapBuffers(d, es);
        printf("[cli] frame %d: eglSwapBuffers %s (0x%x) took %.1f ms\n", k, ok ? "ok" : "FAILED", ok ? 0 : eglGetError(), (now_s() - ts) * 1000);
        fflush(stdout);
        if (!ok) break;
        wl_display_dispatch_pending(dpy);
        wl_display_flush(dpy);
    }
    printf("[cli] %d frames in %.1f ms\n", frames, (now_s() - t0) * 1000);
    eglMakeCurrent(d, NULL, NULL, NULL);
    eglTerminate(d);
    wl_egl_window_destroy(ew);
    wl_display_roundtrip(dpy);
    wl_display_disconnect(dpy);
    printf("[cli] done\n");
    return 0;
}
