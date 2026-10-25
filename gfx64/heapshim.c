// LD_PRELOAD shim for the 64-bit Rockchip Mali blob (-wayland-gbm) on the C4. Kernel 5.4 has no /dev/dma_heap, and the
// blob allocates its window buffers from /dev/dma_heap/system-uncached. The shim answers opens of /dev/dma_heap/system*
// with a placeholder fd and each DMA_HEAP_IOCTL_ALLOC on it through heapcore.h (DRM dumb buffer, then dma_buf_te, then
// dma_buf_unified, the only allocator inside SAM's jail).
// /dev/dma_heap/protected and every other heap are reported missing. Everything else passes straight through.
//
// Hot paths: the process's GPU ioctls and closes all go through here. An ioctl costs one compare against the heap
// request number; a close costs one bit test. No locks, no allocation; the only state is atomics. Allocation (cold:
// once per buffer, 30-48 ms when CMA pages have to migrate) is never done per frame.
//
// Environment: LUMATV_HEAP=auto|dumb|te|unified (default auto), LUMATV_HEAP_LOG=1 logs every allocation to stderr; an
// allocation from an exporter (dma_buf_te, dma_buf_unified) and a failed allocation are always logged.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "heapcore.h"

#define HEAP_DIR "/dev/dma_heap/"
#define HEAP_DIR_LEN (sizeof(HEAP_DIR) - 1)
#define MAX_TRACKED_FD 4096

static _Atomic(void *) r_open, r_open64, r_openat, r_openat64, r_access, r_ioctl, r_close, r_dup, r_dup2, r_dup3;
static struct heap_state state;
static int log_all;
static pthread_once_t once = PTHREAD_ONCE_INIT;
static _Atomic uint64_t heap_fds[MAX_TRACKED_FD / 64];

// The real libc entry points, resolved on first use so a call that arrives before this library's constructor has
// run (another library's constructor) still works.
static void *real(_Atomic(void *) *slot, const char *sym) {
  void *f = atomic_load_explicit(slot, memory_order_relaxed);
  if (!f) { f = dlsym(RTLD_NEXT, sym); atomic_store_explicit(slot, f, memory_order_relaxed); }
  return f;
}
#define REAL(slot, sym, type) ((type)real(&slot, sym))
typedef int (*open_fn)(const char *, int, ...);
typedef int (*openat_fn)(int, const char *, int, ...);
typedef int (*access_fn)(const char *, int);
typedef int (*ioctl_fn)(int, unsigned long, ...);
typedef int (*close_fn)(int);
typedef int (*dup_fn)(int);
typedef int (*dup2_fn)(int, int);
typedef int (*dup3_fn)(int, int, int);

static void init(void) {
  enum heap_mode mode = HEAP_AUTO;
  const char *m = getenv("LUMATV_HEAP");
  if (m && !strcmp(m, "dumb")) mode = HEAP_DUMB_ONLY;
  else if (m && !strcmp(m, "te")) mode = HEAP_TE_ONLY;
  else if (m && !strcmp(m, "unified")) mode = HEAP_UNIFIED_ONLY;
  log_all = getenv("LUMATV_HEAP_LOG") != NULL;
  heap_state_init(&state, mode);
}
__attribute__((constructor)) static void ctor(void) { pthread_once(&once, init); }

static void logf_err(const char *fmt, ...) {
  char buf[256];
  va_list ap; va_start(ap, fmt);
  int n = vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  if (n > 0) { ssize_t w = write(2, buf, (size_t)(n < (int)sizeof buf ? n : (int)sizeof buf - 1)); (void)w; }
}

static inline int tracked(int fd) {
  return (unsigned)fd < MAX_TRACKED_FD &&
         ((atomic_load_explicit(&heap_fds[fd >> 6], memory_order_acquire) >> (fd & 63)) & 1);
}
static inline void track(int fd) { atomic_fetch_or(&heap_fds[fd >> 6], (uint64_t)1 << (fd & 63)); }
static inline void untrack(int fd) {
  if ((unsigned)fd < MAX_TRACKED_FD) atomic_fetch_and(&heap_fds[fd >> 6], ~((uint64_t)1 << (fd & 63)));
}

static inline int is_heap_path(const char *p) { return p && !strncmp(p, HEAP_DIR, HEAP_DIR_LEN); }
static inline int is_served_heap(const char *p) { return !strncmp(p + HEAP_DIR_LEN, "system", 6); }

static int open_heap(const char *p) {
  pthread_once(&once, init);
  if (!is_served_heap(p)) { errno = ENOENT; return -1; }
  // The blob only keeps this fd as the handle for its allocation ioctls; the memory comes from the devices in
  // heapcore.h, so a descriptor on /dev/null is enough.
  int fd = REAL(r_open, "open", open_fn)("/dev/null", O_RDWR | O_CLOEXEC);
  if (fd < 0) return -1;
  if ((unsigned)fd >= MAX_TRACKED_FD) { REAL(r_close, "close", close_fn)(fd); errno = EMFILE; return -1; }
  track(fd);
  if (log_all) logf_err("heapshim: open %s -> placeholder fd %d\n", p, fd);
  return fd;
}

static int hs_open_dev(void *ctx, const char *path) { (void)ctx; return REAL(r_open, "open", open_fn)(path, O_RDWR | O_CLOEXEC); }
static int hs_ioctl(void *ctx, int fd, unsigned long req, void *arg) { (void)ctx; return REAL(r_ioctl, "ioctl", ioctl_fn)(fd, req, arg); }
static int hs_close(int fd) { return REAL(r_close, "close", close_fn)(fd); }
static const struct heap_ops ops = {hs_open_dev, hs_ioctl, NULL};

static int heap_ioctl_alloc(struct dma_heap_allocation_data *a) {
  pthread_once(&once, init);
  struct timespec t0, t1;
  clock_gettime(CLOCK_MONOTONIC, &t0);
  uint64_t len = a->len;
  enum heap_source src;
  int r = heap_alloc(&state, &ops, a, hs_close, &src);
  int err = errno;
  clock_gettime(CLOCK_MONOTONIC, &t1);
  double ms = (double)(t1.tv_sec - t0.tv_sec) * 1e3 + (double)(t1.tv_nsec - t0.tv_nsec) / 1e6;
  if (r < 0)
    logf_err("heapshim: alloc %llu bytes FAILED (errno %d) after %.1f ms\n", (unsigned long long)len, err, ms);
  else if (src == HEAP_TE)
    logf_err("heapshim: alloc %llu bytes -> fd %u via dma_buf_te (CMA short) in %.1f ms\n", (unsigned long long)len, a->fd, ms);
  else if (src == HEAP_UNIFIED)
    logf_err("heapshim: alloc %llu bytes -> fd %u via dma_buf_unified in %.1f ms\n", (unsigned long long)len, a->fd, ms);
  else if (log_all)
    logf_err("heapshim: alloc %llu bytes -> fd %u via drm dumb in %.1f ms (dumb %u, te %u)\n", (unsigned long long)len, a->fd, ms,
             atomic_load(&state.n_dumb), atomic_load(&state.n_te));
  errno = err;
  return r;
}

static mode_t mode_arg(int fl, va_list ap) { return (fl & (O_CREAT | O_TMPFILE)) ? (mode_t)va_arg(ap, int) : 0; }

int open(const char *p, int fl, ...) {
  va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap);
  return is_heap_path(p) ? open_heap(p) : REAL(r_open, "open", open_fn)(p, fl, m);
}
int open64(const char *p, int fl, ...) {
  va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap);
  return is_heap_path(p) ? open_heap(p) : REAL(r_open64, "open64", open_fn)(p, fl, m);
}
int openat(int d, const char *p, int fl, ...) {
  va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap);
  return is_heap_path(p) ? open_heap(p) : REAL(r_openat, "openat", openat_fn)(d, p, fl, m);
}
int openat64(int d, const char *p, int fl, ...) {
  va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap);
  return is_heap_path(p) ? open_heap(p) : REAL(r_openat64, "openat64", openat_fn)(d, p, fl, m);
}
int access(const char *p, int mode) {
  if (is_heap_path(p)) {
    if (is_served_heap(p)) return 0;
    errno = ENOENT;
    return -1;
  }
  return REAL(r_access, "access", access_fn)(p, mode);
}

int close(int fd) {
  untrack(fd);  // before the real close: afterwards another thread may already have been handed the same number
  return REAL(r_close, "close", close_fn)(fd);
}
int dup(int fd) {
  int n = REAL(r_dup, "dup", dup_fn)(fd);
  if (n >= 0 && tracked(fd) && (unsigned)n < MAX_TRACKED_FD) track(n);
  return n;
}
int dup2(int fd, int to) {
  int n = REAL(r_dup2, "dup2", dup2_fn)(fd, to);
  if (n >= 0 && fd != to) { if (tracked(fd) && (unsigned)n < MAX_TRACKED_FD) track(n); else untrack(n); }
  return n;
}
int dup3(int fd, int to, int flags) {
  int n = REAL(r_dup3, "dup3", dup3_fn)(fd, to, flags);
  if (n >= 0) { if (tracked(fd) && (unsigned)n < MAX_TRACKED_FD) track(n); else untrack(n); }
  return n;
}

int ioctl(int fd, unsigned long req, ...) {
  va_list ap; va_start(ap, req); void *arg = va_arg(ap, void *); va_end(ap);
  if (req != DMA_HEAP_IOCTL_ALLOC || !tracked(fd)) return REAL(r_ioctl, "ioctl", ioctl_fn)(fd, req, arg);
  return heap_ioctl_alloc((struct dma_heap_allocation_data *)arg);
}
