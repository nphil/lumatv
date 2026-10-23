// Bench: cost of the same 1080p frame (three clears + glFinish) into a driver-allocated texture versus imported
// dma-bufs (Arm dma_buf_te, DRM dumb), on the 64-bit Mali blob. Offscreen; works with the set in standby.
#include <dlfcn.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

#define W 1920
#define H 1080
struct drm_mode_create_dumb { uint32_t height, width, bpp, flags, handle, pitch; uint64_t size; };
struct drm_prime_handle { uint32_t handle, flags; int32_t fd; };
#define DRM_IOCTL_MODE_CREATE_DUMB _IOWR('d', 0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_PRIME_HANDLE_TO_FD _IOWR('d', 0x2d, struct drm_prime_handle)
struct te_alloc { uint64_t pages; };
#define TE_ALLOC _IOR('E', 0x01, struct te_alloc)

typedef void *P;
static P (*eglGetDisplay)(P);
static unsigned (*eglInitialize)(P, int *, int *);
static unsigned (*eglChooseConfig)(P, const int *, P *, int, int *);
static P (*eglCreatePbufferSurface)(P, P, const int *);
static P (*eglCreateContext)(P, P, P, const int *);
static unsigned (*eglMakeCurrent)(P, P, P, P);
static unsigned (*eglBindAPI)(unsigned);
static P (*eglGetProcAddress)(const char *);
static int (*eglGetError)(void);
static P (*eglCreateImageKHR)(P, P, unsigned, P, const int *);
static void (*glEGLImageTargetTexture2DOES)(unsigned, P);
static void (*glClearColor)(float, float, float, float), (*glClear)(unsigned), (*glEnable)(unsigned),
    (*glDisable)(unsigned), (*glScissor)(int, int, int, int), (*glViewport)(int, int, int, int), (*glFinish)(void),
    (*glGenTextures)(int, unsigned *), (*glBindTexture)(unsigned, unsigned), (*glGenFramebuffers)(int, unsigned *),
    (*glBindFramebuffer)(unsigned, unsigned), (*glFramebufferTexture2D)(unsigned, unsigned, unsigned, unsigned, int),
    (*glTexImage2D)(unsigned, int, int, int, int, int, unsigned, unsigned, const void *), (*glFlush)(void);
static unsigned (*glCheckFramebufferStatus)(unsigned);

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void frame(int i) {
  glViewport(0, 0, W, H);
  glDisable(0x0C11);
  glClearColor(0.10f, 0.07f, 0.22f, 1.0f); glClear(0x4000);
  glEnable(0x0C11);
  glScissor((i * 13) % (W - 240), 300, 240, 480); glClearColor(0.62f, 0.55f, 1.0f, 1.0f); glClear(0x4000);
  glScissor(120, 936, 400 + i, 24); glClearColor(1, 1, 1, 1); glClear(0x4000);
}

static void bench(const char *name, unsigned fbo) {
  glBindFramebuffer(0x8D40, fbo);
  unsigned st = glCheckFramebufferStatus(0x8D40);
  if (st != 0x8CD5) { printf("%-22s fbo incomplete 0x%x\n", name, st); return; }
  for (int i = 0; i < 5; i++) { frame(i); glFinish(); }
  double sum = 0, worst = 0;
  for (int i = 0; i < 60; i++) {
    double t = now(); frame(i); glFinish(); t = now() - t; sum += t; if (t > worst) worst = t;
  }
  printf("%-22s frame avg %.2f ms, worst %.2f ms\n", name, sum * 1000 / 60, worst * 1000);
}

static unsigned import_fbo(P d, int fd, int stride) {
  int ia[] = {0x3057, W, 0x3056, H, 0x3271, 0x34325241, 0x3272, fd, 0x3273, 0, 0x3274, stride, 0x3038};
  P img = eglCreateImageKHR(d, 0, 0x3270, 0, ia);
  if (!img) { printf("import failed 0x%x\n", eglGetError()); return 0; }
  unsigned tex, fbo;
  glGenTextures(1, &tex); glBindTexture(0x0DE1, tex); glEGLImageTargetTexture2DOES(0x0DE1, img);
  glGenFramebuffers(1, &fbo); glBindFramebuffer(0x8D40, fbo);
  glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, tex, 0);
  return fbo;
}

int main(int argc, char **argv) {
  void *h = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
  if (!h) { printf("dlopen %s\n", dlerror()); return 1; }
#define S(n) *(void **)&n = dlsym(h, #n); if (!n) { printf("missing %s\n", #n); return 1; }
  S(eglGetDisplay) S(eglInitialize) S(eglChooseConfig) S(eglCreatePbufferSurface) S(eglCreateContext) S(eglMakeCurrent)
  S(eglBindAPI) S(eglGetProcAddress) S(eglGetError) S(glClearColor) S(glClear) S(glEnable) S(glDisable) S(glScissor)
  S(glViewport) S(glFinish) S(glGenTextures) S(glBindTexture) S(glGenFramebuffers) S(glBindFramebuffer)
  S(glFramebufferTexture2D) S(glTexImage2D) S(glCheckFramebufferStatus) S(glFlush)
  *(void **)&eglCreateImageKHR = eglGetProcAddress("eglCreateImageKHR");
  *(void **)&glEGLImageTargetTexture2DOES = eglGetProcAddress("glEGLImageTargetTexture2DOES");
  P d = eglGetDisplay(0); int ma, mi; eglInitialize(d, &ma, &mi);
  int ca[] = {0x3033, 1, 0x3040, 4, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038}; P cfg; int n;
  eglChooseConfig(d, ca, &cfg, 1, &n);
  int pa[] = {0x3057, W, 0x3056, H, 0x3038}; P ps = eglCreatePbufferSurface(d, cfg, pa);
  eglBindAPI(0x30A0); int xa[] = {0x3098, 2, 0x3038}; P c = eglCreateContext(d, cfg, 0, xa);
  if (!eglMakeCurrent(d, ps, ps, c)) { printf("makecurrent 0x%x\n", eglGetError()); return 2; }

  bench("pbuffer (driver)", 0);
  unsigned tex, fbo;
  glGenTextures(1, &tex); glBindTexture(0x0DE1, tex); glTexImage2D(0x0DE1, 0, 0x1908, W, H, 0, 0x1908, 0x1401, 0);
  glGenFramebuffers(1, &fbo); glBindFramebuffer(0x8D40, fbo); glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, tex, 0);
  bench("texture (driver)", fbo);

  int te = open("/dev/dma_buf_te", O_RDWR | O_CLOEXEC);
  struct te_alloc a = {(W * H * 4 + 4095) / 4096};
  int tfd = ioctl(te, TE_ALLOC, &a);
  if (tfd >= 0) { unsigned f = import_fbo(d, tfd, W * 4); if (f) bench("imported dma_buf_te", f); }
  int drm = open("/dev/dri/card0", O_RDWR | O_CLOEXEC);
  struct drm_mode_create_dumb cd = {H, W, 32, 0, 0, 0, 0};
  if (ioctl(drm, DRM_IOCTL_MODE_CREATE_DUMB, &cd) == 0) {
    struct drm_prime_handle ph = {cd.handle, O_RDWR | O_CLOEXEC, -1};
    if (ioctl(drm, DRM_IOCTL_PRIME_HANDLE_TO_FD, &ph) == 0) { unsigned f = import_fbo(d, ph.fd, cd.pitch); if (f) bench("imported DRM dumb", f); }
  } else printf("dumb allocation failed\n");
  return 0;
}
