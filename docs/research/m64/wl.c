// Probe: a 64-bit Wayland client of LG's compositor, drawing with a 64-bit Mali blob, timing frame pacing.
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <math.h>
#include <wayland-client.h>
#include "webos-shell-client.h"

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext;
typedef int EGLint;
#define L(name, ret, ...) static ret (*name)(__VA_ARGS__);
L(eglGetPlatformDisplay, EGLDisplay, unsigned, void *, const intptr_t *)
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
L(glGetString, const unsigned char *, unsigned)
L(glClearColor, void, float, float, float, float)
L(glClear, void, unsigned)
L(glEnable, void, unsigned)
L(glDisable, void, unsigned)
L(glScissor, void, int, int, int, int)
L(glViewport, void, int, int, int, int)
L(wl_egl_window_create, void *, struct wl_surface *, int, int)

static struct wl_compositor *compositor;
static struct wl_shell *shell;
static struct wl_webos_shell *wshell;

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
  printf("global %s v%u\n", iface, ver);
  if (!strcmp(iface, "wl_compositor")) compositor = wl_registry_bind(r, name, &wl_compositor_interface, 3);
  else if (!strcmp(iface, "wl_shell")) shell = wl_registry_bind(r, name, &wl_shell_interface, 1);
  else if (!strcmp(iface, "wl_webos_shell")) wshell = wl_registry_bind(r, name, &wl_webos_shell_interface, 1);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) {}
static const struct wl_registry_listener reg = {global, global_remove};

static void ping(void *d, struct wl_shell_surface *s, uint32_t serial) { wl_shell_surface_pong(s, serial); }
static void configure(void *d, struct wl_shell_surface *s, uint32_t e, int32_t w, int32_t h) {}
static void popup_done(void *d, struct wl_shell_surface *s) {}
static const struct wl_shell_surface_listener ssl = {ping, configure, popup_done};

static void w_state(void *d, struct wl_webos_shell_surface *s, uint32_t st) { printf("webos state_changed %u\n", st); }
static void w_pos(void *d, struct wl_webos_shell_surface *s, int32_t x, int32_t y) { printf("webos position %d,%d\n", x, y); }
static void w_close(void *d, struct wl_webos_shell_surface *s) { printf("webos close\n"); }
static void w_exposed(void *d, struct wl_webos_shell_surface *s, struct wl_array *a) { printf("webos exposed (%zu bytes)\n", a->size); }
static void w_about(void *d, struct wl_webos_shell_surface *s, uint32_t st) { printf("webos state_about_to_change %u\n", st); }
static void w_addon(void *d, struct wl_webos_shell_surface *s, uint32_t st) {}
static const struct wl_webos_shell_surface_listener wsl = {w_state, w_pos, w_close, w_exposed, w_about, w_addon};

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

int main(int argc, char **argv) {
  const char *lib = argv[1];
  const char *wtype = argc > 2 ? argv[2] : "_WEBOS_WINDOW_TYPE_OVERLAY";
  int seconds = argc > 3 ? atoi(argv[3]) : 6;
  void *h = dlopen(lib, RTLD_NOW | RTLD_GLOBAL);
  if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
#define S(name) *(void **)&name = dlsym(h, #name); if (!name) { printf("missing %s\n", #name); return 1; }
  S(eglGetPlatformDisplay) S(eglInitialize) S(eglQueryString) S(eglChooseConfig) S(eglCreateWindowSurface)
  S(eglCreateContext) S(eglMakeCurrent) S(eglSwapBuffers) S(eglSwapInterval) S(eglGetError) S(eglBindAPI)
  S(glGetString) S(glClearColor) S(glClear) S(glEnable) S(glDisable) S(glScissor) S(glViewport)
  *(void **)&wl_egl_window_create = dlsym(h, "wl_egl_window_create");
  if (!wl_egl_window_create) {
    void *we = dlopen("libwayland-egl.so.1", RTLD_NOW | RTLD_GLOBAL);
    if (we) *(void **)&wl_egl_window_create = dlsym(we, "wl_egl_window_create");
  }
  if (!wl_egl_window_create) { printf("missing wl_egl_window_create\n"); return 1; }

  struct wl_display *dpy = wl_display_connect(NULL);
  if (!dpy) { printf("wl_display_connect failed\n"); return 2; }
  struct wl_registry *r = wl_display_get_registry(dpy);
  wl_registry_add_listener(r, &reg, NULL);
  wl_display_roundtrip(dpy);
  if (!compositor || !shell || !wshell) { printf("missing globals\n"); return 3; }

  const int W = 1920, H = 1080;
  struct wl_surface *surf = wl_compositor_create_surface(compositor);
  struct wl_shell_surface *ss = wl_shell_get_shell_surface(shell, surf);
  wl_shell_surface_add_listener(ss, &ssl, NULL);
  struct wl_webos_shell_surface *ws = wl_webos_shell_get_shell_surface(wshell, surf);
  wl_webos_shell_surface_add_listener(ws, &wsl, NULL);
  wl_webos_shell_surface_set_property(ws, "appId", "org.nphil.lumaglass");
  wl_webos_shell_surface_set_property(ws, "_WEBOS_WINDOW_TYPE", wtype);
  wl_webos_shell_surface_set_property(ws, "title", "LumaTV 64-bit probe");
  wl_shell_surface_set_toplevel(ss);
  wl_webos_shell_surface_set_state(ws, WL_WEBOS_SHELL_SURFACE_STATE_FULLSCREEN);

  EGLDisplay d = eglGetPlatformDisplay(0x31D8 /*WAYLAND_KHR*/, dpy, NULL);
  EGLint maj, min;
  if (!d || !eglInitialize(d, &maj, &min)) { printf("eglInitialize: 0x%x\n", eglGetError()); return 4; }
  printf("EGL %s\n", eglQueryString(d, 0x3054));
  EGLint ca[] = {0x3033, 0x0004 /*WINDOW*/, 0x3040, 0x0004, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038};
  EGLConfig cfg; EGLint n = 0;
  if (!eglChooseConfig(d, ca, &cfg, 1, &n) || !n) { printf("no config 0x%x\n", eglGetError()); return 5; }
  void *ew = wl_egl_window_create(surf, W, H);
  EGLSurface es = eglCreateWindowSurface(d, cfg, ew, NULL);
  eglBindAPI(0x30A0);
  EGLint xa[] = {0x3098, 2, 0x3038};
  EGLContext c = eglCreateContext(d, cfg, 0, xa);
  if (!es || !c || !eglMakeCurrent(d, es, es, c)) { printf("surface/context 0x%x\n", eglGetError()); return 6; }
  eglSwapInterval(d, 1);
  printf("GL %s / %s\n", glGetString(0x1F01), glGetString(0x1F02));
  fflush(stdout);

  double t0 = now(), last = t0, worst = 0; int frames = 0, over20 = 0;
  while (now() - t0 < seconds) {
    double t = now() - t0;
    glViewport(0, 0, W, H);
    glDisable(0x0C11);
    glClearColor(0.10f, 0.07f, 0.22f, 1.0f); glClear(0x4000);
    glEnable(0x0C11); // scissor: a bar sweeping across, so motion and judder are visible
    int x = (int)((sin(t * 1.5) * 0.5 + 0.5) * (W - 240));
    glScissor(x, 300, 240, 480); glClearColor(0.62f, 0.55f, 1.0f, 1.0f); glClear(0x4000);
    glScissor(120, 120, (int)((W - 240) * fmod(t / seconds, 1.0)) + 1, 24); glClearColor(1, 1, 1, 1); glClear(0x4000);
    if (!eglSwapBuffers(d, es)) { printf("swap failed 0x%x\n", eglGetError()); break; }
    wl_display_dispatch_pending(dpy);
    double nw = now(), gap = nw - last; last = nw;
    if (frames > 0) { if (gap > worst) worst = gap; if (gap > 0.020) over20++; }
    frames++;
  }
  double total = now() - t0;
  printf("frames=%d in %.2f s = %.1f fps, worst gap %.1f ms, gaps over 20 ms: %d\n", frames, total, frames / total, worst * 1000, over20);
  return 0;
}
