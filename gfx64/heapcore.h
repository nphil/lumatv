// Allocation policy of the heap shim, separate from the libc interposition so a host test can drive it with a
// mock ioctl layer. Header-only; every function is static.
//
// Policy: a DRM dumb buffer on /dev/dri/card0 first (contiguous CMA, the fast path: 60.2-60.8 fps on screen),
// Arm's /dev/dma_buf_te (ordinary pages, 55.9-56.6 fps) when CMA is short. A dumb allocation that has to migrate
// movable pages out of CMA takes 30-48 ms, so the policy is only ever run when a buffer is created, never per frame.
#pragma once
#include <errno.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <stdint.h>
#include <sys/ioctl.h>

#define HEAP_PAGE 4096u

struct dma_heap_allocation_data { uint64_t len; uint32_t fd; uint32_t fd_flags; uint64_t heap_flags; };
struct te_alloc { uint64_t pages; };
struct drm_mode_create_dumb { uint32_t height, width, bpp, flags, handle, pitch; uint64_t size; };
struct drm_prime_handle { uint32_t handle, flags; int32_t fd; };
struct drm_gem_close { uint32_t handle, pad; };

#define DMA_HEAP_IOCTL_ALLOC _IOWR('H', 0x0, struct dma_heap_allocation_data)
#define TE_ALLOC _IOR('E', 0x01, struct te_alloc)
#define DRM_IOCTL_MODE_CREATE_DUMB _IOWR('d', 0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_PRIME_HANDLE_TO_FD _IOWR('d', 0x2d, struct drm_prime_handle)
#define DRM_IOCTL_GEM_CLOSE _IOW('d', 0x09, struct drm_gem_close)

#define HEAP_DRM_PATH "/dev/dri/card0"
#define HEAP_TE_PATH "/dev/dma_buf_te"

enum heap_mode { HEAP_AUTO, HEAP_DUMB_ONLY, HEAP_TE_ONLY };
enum heap_source { HEAP_NONE, HEAP_DUMB, HEAP_TE };

// The only way the policy touches the system. In the shim these are the real libc calls (never the interposed
// ones); in the test they are mocks.
struct heap_ops {
  int (*open_dev)(void *ctx, const char *path);                    // fd, or -1 with errno
  int (*ioctl)(void *ctx, int fd, unsigned long req, void *arg);   // 0 / fd, or -1 with errno
  void *ctx;
};

struct heap_state {
  enum heap_mode mode;
  _Atomic int drm_fd, te_fd;               // -1 until first use; opened once, kept for the process lifetime
  _Atomic uint32_t n_dumb, n_te, n_fail;
};

static inline void heap_state_init(struct heap_state *s, enum heap_mode mode) {
  s->mode = mode;
  atomic_init(&s->drm_fd, -1);
  atomic_init(&s->te_fd, -1);
  atomic_init(&s->n_dumb, 0);
  atomic_init(&s->n_te, 0);
  atomic_init(&s->n_fail, 0);
}

// Lazily opened device fd shared by all threads. Two threads may both open; the loser closes its copy through
// `close_fd` (the shim's real close).
static inline int heap_dev_fd(_Atomic int *slot, const struct heap_ops *ops, const char *path, int (*close_fd)(int)) {
  int fd = atomic_load_explicit(slot, memory_order_acquire);
  if (fd >= 0) return fd;
  int n = ops->open_dev(ops->ctx, path);
  if (n < 0) return -1;
  int expect = -1;
  if (atomic_compare_exchange_strong(slot, &expect, n)) return n;
  if (close_fd) close_fd(n);
  return expect;
}

// A dumb buffer is sized by geometry; the blob only wants a dma-buf of at least `len` bytes, so ask for one
// 4096-byte row (1024 x 32 bit) per page: pitch*height is then exactly pages*4096.
static inline int heap_alloc_dumb(struct heap_state *s, const struct heap_ops *ops, uint32_t pages, int export_flags,
                                  int (*close_fd)(int)) {
  int drm = heap_dev_fd(&s->drm_fd, ops, HEAP_DRM_PATH, close_fd);
  if (drm < 0) return -1;
  struct drm_mode_create_dumb c = {pages, 1024, 32, 0, 0, 0, 0};
  if (ops->ioctl(ops->ctx, drm, DRM_IOCTL_MODE_CREATE_DUMB, &c) < 0) return -1;
  struct drm_prime_handle ph = {c.handle, (uint32_t)export_flags, -1};
  int r = ops->ioctl(ops->ctx, drm, DRM_IOCTL_PRIME_HANDLE_TO_FD, &ph);
  int saved = errno;
  // The exported dma-buf keeps the memory alive; the GEM handle must go either way or it leaks.
  struct drm_gem_close g = {c.handle, 0};
  ops->ioctl(ops->ctx, drm, DRM_IOCTL_GEM_CLOSE, &g);
  errno = saved;
  return r < 0 ? -1 : ph.fd;
}

static inline int heap_alloc_te(struct heap_state *s, const struct heap_ops *ops, uint32_t pages, int (*close_fd)(int)) {
  int te = heap_dev_fd(&s->te_fd, ops, HEAP_TE_PATH, close_fd);
  if (te < 0) return -1;
  struct te_alloc t = {pages};
  return ops->ioctl(ops->ctx, te, TE_ALLOC, &t);
}

// Serves one DMA_HEAP_IOCTL_ALLOC. Returns 0 with a->fd set, or -1 with errno ENOMEM (EINVAL for a bad length),
// which is what the kernel's own heap returns when it is out of memory. `*src` says which source answered.
static inline int heap_alloc(struct heap_state *s, const struct heap_ops *ops, struct dma_heap_allocation_data *a,
                             int (*close_fd)(int), enum heap_source *src) {
  *src = HEAP_NONE;
  uint64_t pages64 = (a->len + HEAP_PAGE - 1) / HEAP_PAGE;
  if (a->len == 0 || pages64 > UINT32_MAX) { errno = EINVAL; return -1; }
  uint32_t pages = (uint32_t)pages64;
  // Export with the caller's flags: the kernel heap hands out O_RDWR (+ O_CLOEXEC) dma-bufs.
  int flags = (int)(a->fd_flags & (O_ACCMODE | O_CLOEXEC));
  if ((flags & O_ACCMODE) == 0) flags |= O_RDWR;
  int fd = -1;
  if (s->mode != HEAP_TE_ONLY) {
    fd = heap_alloc_dumb(s, ops, pages, flags, close_fd);
    if (fd >= 0) { *src = HEAP_DUMB; atomic_fetch_add(&s->n_dumb, 1); }
  }
  if (fd < 0 && s->mode != HEAP_DUMB_ONLY) {
    fd = heap_alloc_te(s, ops, pages, close_fd);
    if (fd >= 0) { *src = HEAP_TE; atomic_fetch_add(&s->n_te, 1); }
  }
  if (fd < 0) { atomic_fetch_add(&s->n_fail, 1); errno = ENOMEM; return -1; }
  a->fd = (uint32_t)fd;
  return 0;
}
