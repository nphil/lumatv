// X1: a trivial full-screen client of LG's compositor, drawing with the 64-bit Mali blob, presenting at 60 Hz for
// N seconds. It measures whether LG's compositor alone can keep a client's frames on the 60 Hz deadline: one clear
// and one small moving rectangle per frame, nothing else. The per-frame timing is logged (swap-to-swap gap) and
// the ftrace harness counts presented frames from outside.
//   X1_SECONDS=60 X1_WINDOW_TYPE=_WEBOS_WINDOW_TYPE_OVERLAY x1client <libmali-...-wayland-gbm.so>
// Window: wl_shell toplevel + wl_webos_shell_surface (state fullscreen, appId org.nphil.lumatv.x1).
// Buffers: the driver's own window path; LD_PRELOAD=x1heapshim.so answers its /dev/dma_heap opens.
#define _GNU_SOURCE
#define X2_CLIENT
#include "../x2/x2wl.h"
#include <dlfcn.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

// ---- wl_webos_shell protocol tables (webos-wayland-extensions/protocol/webos-shell.xml, v2) ----
extern const struct wl_interface wl_webos_shell_interface, wl_webos_shell_surface_interface;
static const struct wl_interface *t_none[4] = {0};
static const struct wl_interface *t_getss[2] = { &wl_webos_shell_surface_interface, &wl_surface_interface };
static const struct wl_message webos_shell_requests[] = {
    { "get_system_pip", "", t_none },
    { "get_shell_surface", "no", t_getss },
};
const struct wl_interface wl_webos_shell_interface = { "wl_webos_shell", 2, 2, webos_shell_requests, 0, NULL };
static const struct wl_message webos_ss_requests[] = {
    { "set_location_hint", "u", t_none }, { "set_state", "u", t_none }, { "set_property", "ss", t_none },
    { "set_key_mask", "u", t_none }, { "set_addon", "2s", t_none }, { "reset_addon", "2", t_none },
};
static const struct wl_message webos_ss_events[] = {
    { "state_changed", "u", t_none }, { "position_changed", "ii", t_none }, { "close", "", t_none },
    { "exposed", "a", t_none }, { "state_about_to_change", "u", t_none }, { "addon_status_changed", "2u", t_none },
};
const struct wl_interface wl_webos_shell_surface_interface = { "wl_webos_shell_surface", 2, 6, webos_ss_requests, 6, webos_ss_events };
struct wl_webos_shell; struct wl_webos_shell_surface;
static struct wl_webos_shell_surface *webos_get_shell_surface(struct wl_webos_shell *s, struct wl_surface *su) {
    return (struct wl_webos_shell_surface *)wl_proxy_marshal_flags(P(s), 1, &wl_webos_shell_surface_interface, wl_proxy_get_version(P(s)), 0, NULL, su);
}
static void webos_ss_set_state(struct wl_webos_shell_surface *s, uint32_t st) { wl_proxy_marshal_flags(P(s), 1, NULL, wl_proxy_get_version(P(s)), 0, st); }
static void webos_ss_set_property(struct wl_webos_shell_surface *s, const char *n, const char *v) { wl_proxy_marshal_flags(P(s), 2, NULL, wl_proxy_get_version(P(s)), 0, n, v); }

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext;
typedef int EGLint;
#define L(name, ret, ...) static ret (*name)(__VA_ARGS__);
L(eglGetPlatformDisplay, EGLDisplay, unsigned, void *, const intptr_t *)
L(eglGetPlatformDisplayEXT, EGLDisplay, unsigned, void *, const EGLint *)
L(eglGetDisplay, EGLDisplay, void *)
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
L(eglTerminate, unsigned, EGLDisplay)
L(eglGetProcAddress, void *, const char *)
L(glGetString, const unsigned char *, unsigned)
L(glClearColor, void, float, float, float, float)
L(glClear, void, unsigned)
L(glEnable, void, unsigned)
L(glDisable, void, unsigned)
L(glScissor, void, int, int, int, int)
L(glViewport, void, int, int, int, int)
L(wl_egl_window_create, void *, struct wl_surface *, int, int)
L(wl_egl_window_destroy, void, void *)

static struct wl_compositor *compositor;
static struct wl_shell *shell;
static struct wl_webos_shell *wshell;
static int closed;

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    if (!strcmp(iface, "wl_compositor")) compositor = wl_registry_bind(r, name, &wl_compositor_interface, 3);
    else if (!strcmp(iface, "wl_shell")) shell = wl_registry_bind(r, name, &wl_shell_interface, 1);
    else if (!strcmp(iface, "wl_webos_shell")) wshell = wl_registry_bind(r, name, &wl_webos_shell_interface, ver < 2 ? ver : 2);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) {}
static void (*registry_listener[])(void) = { (void (*)(void))global, (void (*)(void))global_remove };
static void ping(void *d, struct wl_shell_surface *s, uint32_t serial) { wl_shell_surface_pong(s, serial); }
static void configure(void *d, struct wl_shell_surface *s, uint32_t e, int32_t w, int32_t h) {}
static void popup_done(void *d, struct wl_shell_surface *s) {}
static void (*ss_listener[])(void) = { (void (*)(void))ping, (void (*)(void))configure, (void (*)(void))popup_done };
static void w_state(void *d, struct wl_webos_shell_surface *s, uint32_t st) { printf("[x1] webos state_changed %u\n", st); fflush(stdout); }
static void w_pos(void *d, struct wl_webos_shell_surface *s, int32_t x, int32_t y) {}
static void w_close(void *d, struct wl_webos_shell_surface *s) { printf("[x1] webos close\n"); closed = 1; }
static void w_exposed(void *d, struct wl_webos_shell_surface *s, struct wl_array *a) {}
static void w_about(void *d, struct wl_webos_shell_surface *s, uint32_t st) {}
static void w_addon(void *d, struct wl_webos_shell_surface *s, uint32_t st) {}
static void (*wss_listener[])(void) = { (void (*)(void))w_state, (void (*)(void))w_pos, (void (*)(void))w_close, (void (*)(void))w_exposed, (void (*)(void))w_about, (void (*)(void))w_addon };

static double now_s(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

int main(int argc, char **argv) {
    const char *lib = argc > 1 ? argv[1] : "libmali.so";
    int seconds = getenv("X1_SECONDS") ? atoi(getenv("X1_SECONDS")) : 60;
    const char *wtype = getenv("X1_WINDOW_TYPE") ? getenv("X1_WINDOW_TYPE") : "_WEBOS_WINDOW_TYPE_OVERLAY";
    int W = 1920, H = 1080;
    void *h = dlopen(lib, RTLD_NOW | RTLD_GLOBAL);
    if (!h) { printf("[x1] dlopen(%s): %s\n", lib, dlerror()); return 1; }
#define S(name) *(void **)&name = dlsym(h, #name); if (!name) { printf("[x1] missing %s\n", #name); return 1; }
    S(eglInitialize) S(eglQueryString) S(eglChooseConfig) S(eglCreateWindowSurface) S(eglCreateContext) S(eglMakeCurrent)
    S(eglSwapBuffers) S(eglSwapInterval) S(eglGetError) S(eglBindAPI) S(eglTerminate) S(eglGetProcAddress) S(eglGetDisplay)
    S(glGetString) S(glClearColor) S(glClear) S(glEnable) S(glDisable) S(glScissor) S(glViewport)
    *(void **)&eglGetPlatformDisplay = dlsym(h, "eglGetPlatformDisplay");
    *(void **)&eglGetPlatformDisplayEXT = dlsym(h, "eglGetPlatformDisplayEXT");
    if (!eglGetPlatformDisplayEXT) *(void **)&eglGetPlatformDisplayEXT = eglGetProcAddress("eglGetPlatformDisplayEXT");
    *(void **)&wl_egl_window_create = dlsym(h, "wl_egl_window_create");
    *(void **)&wl_egl_window_destroy = dlsym(h, "wl_egl_window_destroy");
    if (!wl_egl_window_create) { void *we = dlopen("libwayland-egl.so.1", RTLD_NOW | RTLD_GLOBAL); if (we) { *(void **)&wl_egl_window_create = dlsym(we, "wl_egl_window_create"); *(void **)&wl_egl_window_destroy = dlsym(we, "wl_egl_window_destroy"); } }
    if (!wl_egl_window_create) { printf("[x1] missing wl_egl_window_create\n"); return 1; }

    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) { printf("[x1] wl_display_connect failed\n"); return 2; }
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_proxy_add_listener(P(reg), registry_listener, NULL);
    wl_display_roundtrip(dpy);
    if (!compositor || !shell || !wshell) { printf("[x1] missing globals (compositor %p shell %p webos_shell %p)\n", (void *)compositor, (void *)shell, (void *)wshell); return 3; }
    struct wl_surface *surf = wl_compositor_create_surface(compositor);
    struct wl_shell_surface *ss = wl_shell_get_shell_surface(shell, surf);
    wl_proxy_add_listener(P(ss), ss_listener, NULL);
    struct wl_webos_shell_surface *ws = webos_get_shell_surface(wshell, surf);
    wl_proxy_add_listener(P(ws), wss_listener, NULL);
    webos_ss_set_property(ws, "appId", "org.nphil.lumatv.x1");
    webos_ss_set_property(ws, "_WEBOS_WINDOW_TYPE", wtype);
    webos_ss_set_property(ws, "title", "LumaTV X1 floor probe");
    wl_shell_surface_set_toplevel(ss);
    webos_ss_set_state(ws, 3 /* fullscreen */);

    EGLDisplay d = NULL;
    if (eglGetPlatformDisplay) d = eglGetPlatformDisplay(0x31D8 /* EGL_PLATFORM_WAYLAND_KHR */, dpy, NULL);
    if (!d && eglGetPlatformDisplayEXT) d = eglGetPlatformDisplayEXT(0x31D8, dpy, NULL);
    if (!d) d = eglGetDisplay(dpy);
    EGLint maj = 0, min = 0;
    if (!d || !eglInitialize(d, &maj, &min)) { printf("[x1] eglInitialize: 0x%x\n", eglGetError()); return 4; }
    printf("[x1] EGL %d.%d %s | %s\n", maj, min, eglQueryString(d, 0x3053), eglQueryString(d, 0x3054));
    EGLint ca[] = { 0x3033, 0x0004, 0x3040, 0x0004, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038 };
    EGLConfig cfg; EGLint n = 0;
    if (!eglChooseConfig(d, ca, &cfg, 1, &n) || !n) { printf("[x1] no config 0x%x\n", eglGetError()); return 5; }
    void *ew = wl_egl_window_create(surf, W, H);
    EGLSurface es = eglCreateWindowSurface(d, cfg, ew, NULL);
    eglBindAPI(0x30A0);
    EGLint xa[] = { 0x3098, 2, 0x3038 };
    EGLContext c = eglCreateContext(d, cfg, NULL, xa);
    if (!es || !c || !eglMakeCurrent(d, es, es, c)) { printf("[x1] surface/context 0x%x\n", eglGetError()); return 6; }
    eglSwapInterval(d, 1);
    printf("[x1] GL %s / %s\n", glGetString(0x1F01), glGetString(0x1F02));
    printf("[x1] window %dx%d type %s, %d s, starting at monotonic %.3f\n", W, H, wtype, seconds, now_s());
    fflush(stdout);

    // Bins of swap-to-swap gaps: <=17 ms (on time), 17-20, 20-33, >33; worst; a per-second line for the harness.
    double t0 = now_s(), last = t0, worst = 0, sec_mark = t0; int frames = 0, b17 = 0, b20 = 0, b33 = 0, b_over = 0, sec_frames = 0;
    while (now_s() - t0 < seconds && !closed) {
        double t = now_s() - t0;
        glViewport(0, 0, W, H);
        glDisable(0x0C11);
        glClearColor(0.08f, 0.08f, 0.10f, 1.0f); glClear(0x4000);
        glEnable(0x0C11);
        int x = (int)((sin(t * 1.5) * 0.5 + 0.5) * (W - 240));
        glScissor(x, 300, 240, 480); glClearColor(0.62f, 0.55f, 1.0f, 1.0f); glClear(0x4000);
        if (!eglSwapBuffers(d, es)) { printf("[x1] swap failed 0x%x\n", eglGetError()); break; }
        wl_display_dispatch_pending(dpy);
        double nw = now_s(), gap = nw - last; last = nw;
        if (frames > 0) { if (gap > worst) worst = gap; if (gap <= 0.0175) b17++; else if (gap <= 0.020) b20++; else if (gap <= 0.0335) b33++; else b_over++; }
        // The first eight swaps' returns and each gap over 20 ms on their own lines, on the compositor log's clock: swap N
        // returned at `nw`, N-1 that long before.
        if (frames < 8) { printf("[x1] swap %d returned at monotonic %.3f (its draw started %.1f ms before)\n", frames + 1, nw, (nw - t0 - t) * 1000); fflush(stdout); }
        if (frames > 0 && gap > 0.020) { printf("[x1] gap %.1f ms before swap %d returned at monotonic %.3f\n", gap * 1000, frames + 1, nw); fflush(stdout); }
        frames++; sec_frames++;
        if (nw - sec_mark >= 1.0) { printf("[x1] t=%3.0f s: %d swaps this second, worst gap so far %.1f ms\n", nw - t0, sec_frames, worst * 1000); fflush(stdout); sec_mark = nw; sec_frames = 0; }
    }
    double total = now_s() - t0;
    printf("[x1] RESULT frames=%d in %.2f s = %.2f fps; gaps: <=17.5 ms %d, 17.5-20 %d, 20-33.5 %d, >33.5 %d; worst %.1f ms; ended at monotonic %.3f\n", frames, total, frames / total, b17, b20, b33, b_over, worst * 1000, now_s());
    fflush(stdout);
    eglMakeCurrent(d, NULL, NULL, NULL);
    eglTerminate(d);
    wl_display_roundtrip(dpy);
    wl_display_disconnect(dpy);
    return 0;
}
