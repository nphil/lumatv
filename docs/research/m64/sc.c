// Probe: 64-bit client of LG's compositor with its own swapchain and no interposer.
// The windowless Mali blob renders into dma-bufs this program allocates once (DRM dumb = contiguous CMA, or Arm's
// dma_buf_te = ordinary pages), imported with EGL_EXT_image_dma_buf_import and handed over with zwp_linux_dmabuf_v1.
// Usage: sc <blob.so> <dumb|te> [seconds]
#include <dlfcn.h>
#include <fcntl.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-client.h"
#include "webos-shell-client.h"

#define W 1920
#define H 1080
#define NBUF 3
#define FOURCC_ARGB8888 0x34325241u

struct drm_mode_create_dumb { uint32_t height, width, bpp, flags, handle, pitch; uint64_t size; };
struct drm_prime_handle { uint32_t handle, flags; int32_t fd; };
struct drm_gem_close { uint32_t handle, pad; };
#define DRM_IOCTL_MODE_CREATE_DUMB _IOWR('d', 0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_PRIME_HANDLE_TO_FD _IOWR('d', 0x2d, struct drm_prime_handle)
#define DRM_IOCTL_GEM_CLOSE _IOW('d', 0x09, struct drm_gem_close)
struct te_alloc { uint64_t pages; };
#define TE_ALLOC _IOR('E', 0x01, struct te_alloc)

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext, *EGLImage;
typedef int EGLint;
typedef intptr_t EGLAttrib;
#define L(name, ret, ...) static ret (*name)(__VA_ARGS__);
L(eglGetDisplay, EGLDisplay, void *)
L(eglInitialize, unsigned, EGLDisplay, EGLint *, EGLint *)
L(eglQueryString, const char *, EGLDisplay, EGLint)
L(eglChooseConfig, unsigned, EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *)
L(eglCreatePbufferSurface, EGLSurface, EGLDisplay, EGLConfig, const EGLint *)
L(eglCreateContext, EGLContext, EGLDisplay, EGLConfig, EGLContext, const EGLint *)
L(eglMakeCurrent, unsigned, EGLDisplay, EGLSurface, EGLSurface, EGLContext)
L(eglGetError, EGLint, void)
L(eglBindAPI, unsigned, unsigned)
L(eglGetProcAddress, void *, const char *)
L(glGetString, const unsigned char *, unsigned)
L(glClearColor, void, float, float, float, float)
L(glClear, void, unsigned)
L(glEnable, void, unsigned)
L(glDisable, void, unsigned)
L(glScissor, void, int, int, int, int)
L(glViewport, void, int, int, int, int)
L(glFinish, void, void)
L(glGenTextures, void, int, unsigned *)
L(glBindTexture, void, unsigned, unsigned)
L(glGenFramebuffers, void, int, unsigned *)
L(glBindFramebuffer, void, unsigned, unsigned)
L(glFramebufferTexture2D, void, unsigned, unsigned, unsigned, unsigned, int)
L(glCheckFramebufferStatus, unsigned, unsigned)
static EGLImage (*eglCreateImageKHR)(EGLDisplay, EGLContext, unsigned, void *, const EGLint *);
static void (*glEGLImageTargetTexture2DOES)(unsigned, void *);

static struct wl_compositor *compositor;
static struct wl_shell *shell;
static struct wl_webos_shell *wshell;
static struct zwp_linux_dmabuf_v1 *dmabuf;
static int argb_ok;

static void dm_format(void *d, struct zwp_linux_dmabuf_v1 *z, uint32_t f) { if (f == FOURCC_ARGB8888) argb_ok = 1; }
static void dm_modifier(void *d, struct zwp_linux_dmabuf_v1 *z, uint32_t f, uint32_t hi, uint32_t lo) {
  if (f == FOURCC_ARGB8888) { argb_ok = 1; printf("ARGB8888 modifier 0x%08x%08x\n", hi, lo); }
}
static const struct zwp_linux_dmabuf_v1_listener dml = {dm_format, dm_modifier};

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
  if (!strcmp(iface, "wl_compositor")) compositor = wl_registry_bind(r, name, &wl_compositor_interface, 3);
  else if (!strcmp(iface, "wl_shell")) shell = wl_registry_bind(r, name, &wl_shell_interface, 1);
  else if (!strcmp(iface, "wl_webos_shell")) wshell = wl_registry_bind(r, name, &wl_webos_shell_interface, 1);
  else if (!strcmp(iface, "zwp_linux_dmabuf_v1")) {
    dmabuf = wl_registry_bind(r, name, &zwp_linux_dmabuf_v1_interface, ver < 3 ? ver : 3);
    zwp_linux_dmabuf_v1_add_listener(dmabuf, &dml, NULL);
  }
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) {}
static const struct wl_registry_listener reg = {global, global_remove};
static void ping(void *d, struct wl_shell_surface *s, uint32_t serial) { wl_shell_surface_pong(s, serial); }
static void configure(void *d, struct wl_shell_surface *s, uint32_t e, int32_t w, int32_t h) {}
static void popup_done(void *d, struct wl_shell_surface *s) {}
static const struct wl_shell_surface_listener ssl = {ping, configure, popup_done};

static struct { int fd; uint32_t stride; unsigned tex, fbo; struct wl_buffer *wb; int busy; } buf[NBUF];
static void released(void *d, struct wl_buffer *b) { buf[(intptr_t)d].busy = 0; }
static const struct wl_buffer_listener bl = {released};
static int frame_done;
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }
static void frame_cb(void *d, struct wl_callback *cb, uint32_t t) { frame_done = 1; wl_callback_destroy(cb); }
static const struct wl_callback_listener fl = {frame_cb};

static long meminfo(const char *key) {
  FILE *f = fopen("/proc/meminfo", "r"); char line[256]; long v = -1;
  while (fgets(line, sizeof line, f)) if (!strncmp(line, key, strlen(key))) { sscanf(line + strlen(key), " %ld", &v); break; }
  fclose(f); return v;
}

static int alloc_dumb(int drm, uint32_t *stride) {
  struct drm_mode_create_dumb c = {H, W, 32, 0, 0, 0, 0};
  if (ioctl(drm, DRM_IOCTL_MODE_CREATE_DUMB, &c) < 0) return -1;
  struct drm_prime_handle p = {c.handle, O_RDWR | O_CLOEXEC, -1};
  int r = ioctl(drm, DRM_IOCTL_PRIME_HANDLE_TO_FD, &p);
  struct drm_gem_close g = {c.handle, 0};
  ioctl(drm, DRM_IOCTL_GEM_CLOSE, &g);
  *stride = c.pitch;
  return r < 0 ? -1 : p.fd;
}
static int alloc_te(int te, uint32_t *stride) {
  struct te_alloc a = {(W * H * 4 + 4095) / 4096};
  *stride = W * 4;
  return ioctl(te, TE_ALLOC, &a);
}

int main(int argc, char **argv) {
  if (argc < 3) { fprintf(stderr, "usage: sc blob.so dumb|te [seconds]\n"); return 1; }
  int use_dumb = !strcmp(argv[2], "dumb");
  int seconds = argc > 3 ? atoi(argv[3]) : 8;
  void *h = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
  if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
#define S(name) *(void **)&name = dlsym(h, #name); if (!name) { printf("missing %s\n", #name); return 1; }
  S(eglGetDisplay) S(eglInitialize) S(eglQueryString) S(eglChooseConfig) S(eglCreatePbufferSurface) S(eglCreateContext)
  S(eglMakeCurrent) S(eglGetError) S(eglBindAPI) S(eglGetProcAddress) S(glGetString) S(glClearColor) S(glClear) S(glEnable)
  S(glDisable) S(glScissor) S(glViewport) S(glFinish) S(glGenTextures) S(glBindTexture) S(glGenFramebuffers)
  S(glBindFramebuffer) S(glFramebufferTexture2D) S(glCheckFramebufferStatus)
  *(void **)&eglCreateImageKHR = eglGetProcAddress("eglCreateImageKHR");
  *(void **)&glEGLImageTargetTexture2DOES = eglGetProcAddress("glEGLImageTargetTexture2DOES");
  if (!eglCreateImageKHR || !glEGLImageTargetTexture2DOES) { printf("no dma-buf import entry points\n"); return 1; }

  struct wl_display *dpy = wl_display_connect(NULL);
  if (!dpy) { printf("wl_display_connect failed\n"); return 2; }
  struct wl_registry *r = wl_display_get_registry(dpy);
  wl_registry_add_listener(r, &reg, NULL);
  wl_display_roundtrip(dpy);
  wl_display_roundtrip(dpy);
  if (!compositor || !shell || !wshell || !dmabuf) { printf("missing globals\n"); return 3; }
  printf("compositor takes ARGB8888 dma-bufs: %s\n", argb_ok ? "yes" : "not advertised");

  EGLDisplay d = eglGetDisplay(0);
  EGLint maj, min;
  if (!eglInitialize(d, &maj, &min)) { printf("eglInitialize 0x%x\n", eglGetError()); return 4; }
  EGLint ca[] = {0x3033, 0x0001, 0x3040, 0x0004, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038};
  EGLConfig cfg; EGLint n = 0;
  eglChooseConfig(d, ca, &cfg, 1, &n);
  EGLint pa[] = {0x3057, 1, 0x3056, 1, 0x3038};
  EGLSurface ps = eglCreatePbufferSurface(d, cfg, pa);
  eglBindAPI(0x30A0);
  EGLint xa[] = {0x3098, 2, 0x3038};
  EGLContext c = eglCreateContext(d, cfg, 0, xa);
  if (!eglMakeCurrent(d, ps, ps, c)) { printf("makecurrent 0x%x\n", eglGetError()); return 5; }
  printf("EGL %s / GL %s\n", eglQueryString(d, 0x3054), glGetString(0x1F02));

  long cma0 = meminfo("CmaFree:"), mem0 = meminfo("MemFree:");
  int dev = open(use_dumb ? "/dev/dri/card0" : "/dev/dma_buf_te", O_RDWR | O_CLOEXEC);
  if (dev < 0) { perror("open allocator"); return 6; }
  for (int i = 0; i < NBUF; i++) {
    buf[i].fd = use_dumb ? alloc_dumb(dev, &buf[i].stride) : alloc_te(dev, &buf[i].stride);
    if (buf[i].fd < 0) { printf("allocation %d failed (%s)\n", i, use_dumb ? "contiguous" : "dma_buf_te"); return 7; }
    EGLint ia[] = {0x3057, W, 0x3056, H, 0x3271, (EGLint)FOURCC_ARGB8888, 0x3272, buf[i].fd, 0x3273, 0, 0x3274,
                   (EGLint)buf[i].stride, 0x3038};
    EGLImage img = eglCreateImageKHR(d, 0, 0x3270, NULL, ia);
    if (!img) { printf("import %d failed 0x%x\n", i, eglGetError()); return 8; }
    glGenTextures(1, &buf[i].tex);
    glBindTexture(0x0DE1, buf[i].tex);
    glEGLImageTargetTexture2DOES(0x0DE1, img);
    glGenFramebuffers(1, &buf[i].fbo);
    glBindFramebuffer(0x8D40, buf[i].fbo);
    glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, buf[i].tex, 0);
    unsigned st = glCheckFramebufferStatus(0x8D40);
    if (st != 0x8CD5) { printf("fbo %d incomplete 0x%x\n", i, st); return 9; }
    struct zwp_linux_buffer_params_v1 *p = zwp_linux_dmabuf_v1_create_params(dmabuf);
    zwp_linux_buffer_params_v1_add(p, buf[i].fd, 0, 0, buf[i].stride, 0, 0);
    buf[i].wb = zwp_linux_buffer_params_v1_create_immed(p, W, H, FOURCC_ARGB8888, 0);
    zwp_linux_buffer_params_v1_destroy(p);
    wl_buffer_add_listener(buf[i].wb, &bl, (void *)(intptr_t)i);
  }
  printf("3 buffers (%s): CmaFree %ld -> %ld kB, MemFree %ld -> %ld kB\n", use_dumb ? "contiguous" : "dma_buf_te",
         cma0, meminfo("CmaFree:"), mem0, meminfo("MemFree:"));

  struct wl_surface *surf = wl_compositor_create_surface(compositor);
  struct wl_shell_surface *ss = wl_shell_get_shell_surface(shell, surf);
  wl_shell_surface_add_listener(ss, &ssl, NULL);
  struct wl_webos_shell_surface *ws = wl_webos_shell_get_shell_surface(wshell, surf);
  wl_webos_shell_surface_set_property(ws, "appId", "org.nphil.lumaglass");
  wl_webos_shell_surface_set_property(ws, "_WEBOS_WINDOW_TYPE", "_WEBOS_WINDOW_TYPE_OVERLAY");
  wl_shell_surface_set_toplevel(ss);
  wl_webos_shell_surface_set_state(ws, WL_WEBOS_SHELL_SURFACE_STATE_FULLSCREEN);
  wl_display_roundtrip(dpy);
  fflush(stdout);

  // Pipelined: the next frame is drawn while the compositor shows the current one, and committed the moment the
  // frame callback says the compositor is ready. GL's origin is the bottom-left of the buffer, so y is flipped.
  double t0 = now(), last = 0, worst = 0, draw_worst = 0, draw_sum = 0; int frames = 0, over20 = 0, waits = 0, drawn = 0;
  int pending = -1, first = 1;
  while (now() - t0 < seconds) {
    if (pending < 0) {
      int i = -1;
      for (int k = 0; k < NBUF; k++) if (!buf[k].busy) { i = k; break; }
      if (i >= 0) {
        double d0 = now(), t = d0 - t0 + 1.0 / 60;
        glBindFramebuffer(0x8D40, buf[i].fbo);
        glViewport(0, 0, W, H);
        glDisable(0x0C11);
        glClearColor(0.10f, 0.07f, 0.22f, 1.0f); glClear(0x4000);
        glEnable(0x0C11);
        int x = (int)((sin(t * 1.5) * 0.5 + 0.5) * (W - 240));
        glScissor(x, H - 300 - 480, 240, 480); glClearColor(0.62f, 0.55f, 1.0f, 1.0f); glClear(0x4000);
        glScissor(120, H - 120 - 24, (int)((W - 240) * fmod(t / seconds, 1.0)) + 1, 24); glClearColor(1, 1, 1, 1);
        glClear(0x4000);
        glFinish();
        double dt = now() - d0; draw_sum += dt; if (dt > draw_worst) draw_worst = dt; drawn++;
        pending = i;
      } else waits++;
    }
    if (pending >= 0 && (first || frame_done)) {
      if (frame_done) {
        double nw = now();
        if (frames > 0) { double gap = nw - last; if (gap > worst) worst = gap; if (gap > 0.020) over20++; }
        last = nw; frames++;
      }
      first = 0; frame_done = 0;
      buf[pending].busy = 1;
      struct wl_callback *cb = wl_surface_frame(surf);
      wl_callback_add_listener(cb, &fl, NULL);
      wl_surface_attach(surf, buf[pending].wb, 0, 0);
      wl_surface_damage(surf, 0, 0, W, H);
      wl_surface_commit(surf);
      wl_display_flush(dpy);
      pending = -1;
      continue;
    }
    if (wl_display_dispatch(dpy) < 0) { printf("display lost\n"); return 10; }
  }
  double total = now() - t0;
  printf("frames=%d in %.2f s = %.1f fps, worst gap %.1f ms, gaps over 20 ms: %d, draw avg %.2f ms worst %.2f ms, "
         "no free buffer %d times\n", frames, total, frames / total, worst * 1000, over20, draw_sum * 1000 / drawn,
         draw_worst * 1000, waits);
  return 0;
}
