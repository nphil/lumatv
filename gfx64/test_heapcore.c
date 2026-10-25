// Host test of the allocation policy in heapcore.h against a mock device layer.
//   zig cc -O1 -pthread -o test_heapcore test_heapcore.c && ./test_heapcore
#define _GNU_SOURCE
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "heapcore.h"

// ---- mock devices ----
struct mock {
  _Atomic int drm_open_ok, te_open_ok, uni_open_ok;   // device present
  _Atomic int dumb_ok, prime_ok, te_ok, uni_ok;       // operation succeeds
  _Atomic int drm_opens, te_opens, uni_opens, closes;
  _Atomic int dumb_creates, primes, gem_closes, te_allocs, uni_allocs;
  _Atomic int next_fd, next_handle;
  _Atomic uint32_t last_height, last_width, last_bpp, last_prime_flags;
  _Atomic uint64_t last_te_pages, last_uni_pages;
};
#define DRM_FD_BASE 100
#define TE_FD_BASE 200
#define UNI_FD_BASE 300
static struct mock M;
static int fails;
#define CHECK(c) do { if (!(c)) { fails++; printf("FAIL line %d: %s\n", __LINE__, #c); } } while (0)

static void reset(void) {
  memset(&M, 0, sizeof M);
  M.drm_open_ok = M.te_open_ok = M.uni_open_ok = M.dumb_ok = M.prime_ok = M.te_ok = M.uni_ok = 1;
  M.next_fd = 1000; M.next_handle = 1;
}
static int m_open(void *ctx, const char *path) {
  (void)ctx;
  if (!strcmp(path, HEAP_DRM_PATH)) { if (!M.drm_open_ok) { errno = ENOENT; return -1; } M.drm_opens++; return DRM_FD_BASE; }
  if (!strcmp(path, HEAP_TE_PATH)) { if (!M.te_open_ok) { errno = ENOENT; return -1; } M.te_opens++; return TE_FD_BASE; }
  if (!strcmp(path, HEAP_UNIFIED_PATH)) { if (!M.uni_open_ok) { errno = ENOENT; return -1; } M.uni_opens++; return UNI_FD_BASE; }
  errno = ENOENT; return -1;
}
static int m_ioctl(void *ctx, int fd, unsigned long req, void *arg) {
  (void)ctx;
  if (fd == DRM_FD_BASE && req == DRM_IOCTL_MODE_CREATE_DUMB) {
    struct drm_mode_create_dumb *c = arg;
    M.dumb_creates++; M.last_height = c->height; M.last_width = c->width; M.last_bpp = c->bpp;
    if (!M.dumb_ok) { errno = ENOMEM; return -1; }
    c->handle = (uint32_t)M.next_handle++; c->pitch = c->width * c->bpp / 8; c->size = (uint64_t)c->pitch * c->height;
    return 0;
  }
  if (fd == DRM_FD_BASE && req == DRM_IOCTL_PRIME_HANDLE_TO_FD) {
    struct drm_prime_handle *p = arg;
    M.primes++; M.last_prime_flags = p->flags;
    if (!M.prime_ok) { errno = EINVAL; return -1; }
    p->fd = M.next_fd++; return 0;
  }
  if (fd == DRM_FD_BASE && req == DRM_IOCTL_GEM_CLOSE) { M.gem_closes++; errno = 0; return 0; }
  if (fd == TE_FD_BASE && req == TE_ALLOC) {
    struct te_alloc *t = arg;
    M.te_allocs++; M.last_te_pages = t->pages;
    if (!M.te_ok) { errno = ENOMEM; return -1; }
    return M.next_fd++;
  }
  if (fd == UNI_FD_BASE && req == TE_ALLOC) {
    // LG's older exporter reads {u32 size in pages; u32 map_cached}: the page count must sit in the low word and
    // map_cached (the high word) must be 0 (write-combined), and the request number must be the one in
    // include/linux/dma-buf-unified.h: _IOR('E', 0x01, 8 bytes).
    struct te_alloc *t = arg;
    M.uni_allocs++; M.last_uni_pages = t->pages;
    if (!M.uni_ok) { errno = ENOMEM; return -1; }
    return M.next_fd++;
  }
  errno = ENOTTY; return -1;
}
static int m_close(int fd) { (void)fd; M.closes++; return 0; }
static const struct heap_ops ops = {m_open, m_ioctl, NULL};

static int alloc(struct heap_state *s, uint64_t len, uint32_t fd_flags, enum heap_source *src) {
  struct dma_heap_allocation_data a = {len, 0xdeadbeef, fd_flags, 0};
  int r = heap_alloc(s, &ops, &a, m_close, src);
  if (r == 0) return (int)a.fd;
  return -1;
}

static void test_dumb_first(void) {
  reset(); struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  int fd = alloc(&s, 8986624, O_RDWR | O_CLOEXEC, &src);   // the 1080p window buffer size seen on the set
  CHECK(fd >= 1000 && src == HEAP_DUMB);
  CHECK(M.last_width == 1024 && M.last_bpp == 32 && M.last_height == 2194);  // 2194 pages * 4096 >= 8986624
  CHECK((uint64_t)M.last_height * 4096 >= 8986624 && (uint64_t)(M.last_height - 1) * 4096 < 8986624);
  CHECK(M.gem_closes == 1 && M.te_allocs == 0 && s.n_dumb == 1);
  CHECK(M.last_prime_flags == (uint32_t)(O_RDWR | O_CLOEXEC));
}
static void test_fallback_cma_short(void) {
  reset(); M.dumb_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  int fd = alloc(&s, 4096 * 10, O_RDWR | O_CLOEXEC, &src);
  CHECK(fd >= 1000 && src == HEAP_TE && M.last_te_pages == 10 && s.n_te == 1 && s.n_dumb == 0);
  // CMA frees up again (video stopped): the next buffer is contiguous again; the policy keeps no "exhausted" flag.
  M.dumb_ok = 1;
  fd = alloc(&s, 4096 * 10, O_RDWR, &src);
  CHECK(fd >= 1000 && src == HEAP_DUMB && s.n_dumb == 1 && s.n_te == 1);
}
static void test_prime_failure_releases_handle(void) {
  reset(); M.prime_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  int fd = alloc(&s, 100000, O_RDWR, &src);
  CHECK(src == HEAP_TE && fd >= 1000 && M.gem_closes == 1);
}
static void test_no_drm_device(void) {
  reset(); M.drm_open_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  CHECK(alloc(&s, 5000, O_RDWR, &src) >= 0 && src == HEAP_TE && M.last_te_pages == 2);
}
static void test_both_fail(void) {
  reset(); M.dumb_ok = M.te_ok = M.uni_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  errno = 0; CHECK(alloc(&s, 5000, O_RDWR, &src) < 0 && errno == ENOMEM && src == HEAP_NONE && s.n_fail == 1);
  reset(); M.drm_open_ok = M.te_open_ok = M.uni_open_ok = 0; heap_state_init(&s, HEAP_AUTO);
  errno = 0; CHECK(alloc(&s, 5000, O_RDWR, &src) < 0 && errno == ENOMEM);
}
// SAM's jail: /dev has dma_buf_unified and no dri/ or dma_buf_te. The unified exporter answers in auto mode.
static void test_jail_unified_only(void) {
  reset(); M.drm_open_ok = M.te_open_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  int fd = alloc(&s, 8986624, O_RDWR | O_CLOEXEC, &src);
  CHECK(fd >= 1000 && src == HEAP_UNIFIED && s.n_unified == 1 && s.n_dumb == 0 && s.n_te == 0 && s.n_fail == 0);
  CHECK(M.last_uni_pages == 2194 && (M.last_uni_pages >> 32) == 0);      // low word: pages; high word: map_cached 0
  CHECK(M.dumb_creates == 0 && M.te_allocs == 0 && M.uni_opens == 1);
  for (int i = 0; i < 3; i++) CHECK(alloc(&s, 4096 * 4, O_RDWR, &src) >= 1000 && src == HEAP_UNIFIED);
  CHECK(M.uni_opens == 1 && M.uni_allocs == 4 && s.n_unified == 4);       // opened once, kept
  CHECK(atomic_load(&s.unified_fd) == UNI_FD_BASE);
}
// Devices that exist but have no memory fall through to the next: the unified exporter is the last resort, tried
// after dma_buf_te, and nothing is asked of it while an earlier source answers.
static void test_unified_after_te(void) {
  reset(); M.dumb_ok = 0; struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  CHECK(alloc(&s, 4096, O_RDWR, &src) >= 0 && src == HEAP_TE && M.uni_opens == 0 && M.uni_allocs == 0);
  M.te_ok = 0;
  CHECK(alloc(&s, 4096 * 3, O_RDWR, &src) >= 0 && src == HEAP_UNIFIED && M.last_uni_pages == 3);
  M.uni_ok = 0; errno = 0;
  CHECK(alloc(&s, 4096, O_RDWR, &src) < 0 && errno == ENOMEM && src == HEAP_NONE && s.n_fail == 1);
}
static void test_modes(void) {
  reset(); struct heap_state s; enum heap_source src;
  heap_state_init(&s, HEAP_TE_ONLY);
  CHECK(alloc(&s, 4096, O_RDWR, &src) >= 0 && src == HEAP_TE && M.dumb_creates == 0 && M.drm_opens == 0);
  reset(); M.dumb_ok = 0; heap_state_init(&s, HEAP_DUMB_ONLY);
  errno = 0; CHECK(alloc(&s, 4096, O_RDWR, &src) < 0 && errno == ENOMEM && M.te_allocs == 0 && M.te_opens == 0);
  CHECK(M.uni_allocs == 0 && M.uni_opens == 0);
  reset(); heap_state_init(&s, HEAP_UNIFIED_ONLY);
  CHECK(alloc(&s, 4096, O_RDWR, &src) >= 0 && src == HEAP_UNIFIED && M.drm_opens == 0 && M.te_opens == 0);
  reset(); M.uni_ok = 0; heap_state_init(&s, HEAP_TE_ONLY);
  CHECK(alloc(&s, 4096, O_RDWR, &src) >= 0 && src == HEAP_TE && M.uni_opens == 0);   // te-only never reaches unified
  M.te_ok = 0; errno = 0;
  CHECK(alloc(&s, 4096, O_RDWR, &src) < 0 && errno == ENOMEM && M.uni_opens == 0);
}
static void test_bad_length(void) {
  reset(); struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  errno = 0; CHECK(alloc(&s, 0, O_RDWR, &src) < 0 && errno == EINVAL);
  errno = 0; CHECK(alloc(&s, (uint64_t)UINT32_MAX * 4096 + 1, O_RDWR, &src) < 0 && errno == EINVAL);
  CHECK(M.dumb_creates == 0 && M.te_allocs == 0 && M.uni_allocs == 0);
  CHECK(alloc(&s, 1, O_RDWR, &src) >= 0 && M.last_height == 1);   // one byte still takes a page
}
static void test_flags(void) {
  reset(); struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  alloc(&s, 4096, 0, &src);                       // no access mode given: defaults to O_RDWR
  CHECK(M.last_prime_flags == (uint32_t)O_RDWR);
  alloc(&s, 4096, O_RDWR | O_CLOEXEC | 0x40000000, &src);   // unrelated bits are not forwarded
  CHECK(M.last_prime_flags == (uint32_t)(O_RDWR | O_CLOEXEC));
}
static void test_devices_opened_once(void) {
  reset(); struct heap_state s; heap_state_init(&s, HEAP_AUTO); enum heap_source src;
  for (int i = 0; i < 5; i++) alloc(&s, 4096, O_RDWR, &src);
  CHECK(M.drm_opens == 1 && M.te_opens == 0 && M.dumb_creates == 5);
  M.dumb_ok = 0;
  for (int i = 0; i < 5; i++) alloc(&s, 4096, O_RDWR, &src);
  CHECK(M.drm_opens == 1 && M.te_opens == 1 && M.te_allocs == 5 && s.n_dumb == 5 && s.n_te == 5);
}

// ---- threads: first use of both devices races; every allocation must still succeed and the counts must add up ----
static struct heap_state shared;
static void *worker(void *arg) {
  int dumb_only_fail = (int)(intptr_t)arg;
  for (int i = 0; i < 200; i++) {
    enum heap_source src;
    if (dumb_only_fail && i % 2) M.dumb_ok = 0; else M.dumb_ok = 1;   // CMA availability flaps under the workers
    int fd = alloc(&shared, 4096 * (1 + i % 7), O_RDWR | O_CLOEXEC, &src);
    if (fd < 0) { fails++; }
  }
  return NULL;
}
static void test_threads(void) {
  reset(); heap_state_init(&shared, HEAP_AUTO);
  pthread_t t[8];
  for (int i = 0; i < 8; i++) pthread_create(&t[i], NULL, worker, (void *)(intptr_t)(i & 1));
  for (int i = 0; i < 8; i++) pthread_join(t[i], NULL);
  CHECK(shared.n_dumb + shared.n_te == 8 * 200 && shared.n_fail == 0);
  CHECK(M.drm_opens >= 1 && M.te_opens >= 1 && (M.drm_opens - 1) + (M.te_opens - 1) == M.closes);  // each losing open closed
  CHECK(atomic_load(&shared.drm_fd) == DRM_FD_BASE && atomic_load(&shared.te_fd) == TE_FD_BASE);
  CHECK(M.gem_closes == M.next_handle - 1);  // one GEM close per handle the mock handed out
}

int main(void) {
  test_dumb_first(); test_fallback_cma_short(); test_prime_failure_releases_handle(); test_no_drm_device();
  test_both_fail(); test_jail_unified_only(); test_unified_after_te(); test_modes(); test_bad_length(); test_flags(); test_devices_opened_once(); test_threads();
  printf(fails ? "test_heapcore: %d FAILED\n" : "test_heapcore: all passed\n", fails);
  return fails != 0;
}
