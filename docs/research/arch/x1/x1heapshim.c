// Heap shim for the 64-bit Mali blob on the C4 (kernel 5.4 has no /dev/dma_heap). Answers opens of
// /dev/dma_heap/system* and their DMA_HEAP_IOCTL_ALLOC with, in order: a DRM dumb buffer on /dev/dri/card0
// (contiguous CMA, the fast path measured in PLAN.md: 60.2-60.8 fps) exported as a dma-buf, and when that fails
// (CMA short) Arm's dma_buf_te exporter (ordinary pages, 55.9-56.6 fps). X1_HEAP=te forces the fallback,
// X1_HEAP=dumb forbids it. Every allocation is logged to stderr with the path taken, so a run's buffer source is
// known. Everything else passes straight through.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

struct dma_heap_allocation_data { uint64_t len; uint32_t fd; uint32_t fd_flags; uint64_t heap_flags; };
#define DMA_HEAP_IOCTL_ALLOC _IOWR('H', 0x0, struct dma_heap_allocation_data)
struct te_alloc { uint64_t pages; };
#define TE_ALLOC _IOR('E', 0x01, struct te_alloc)
struct drm_mode_create_dumb { uint32_t height, width, bpp, flags, handle, pitch; uint64_t size; };
struct drm_prime_handle { uint32_t handle, flags; int32_t fd; };
struct drm_gem_close { uint32_t handle, pad; };
#define DRM_IOCTL_MODE_CREATE_DUMB _IOWR('d', 0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_PRIME_HANDLE_TO_FD _IOWR('d', 0x2d, struct drm_prime_handle)
#define DRM_IOCTL_GEM_CLOSE _IOW('d', 0x09, struct drm_gem_close)

static int (*real_open)(const char *, int, ...);
static int (*real_open64)(const char *, int, ...);
static int (*real_openat)(int, const char *, int, ...);
static int (*real_openat64)(int, const char *, int, ...);
static int (*real_access)(const char *, int);
static int (*real_ioctl)(int, unsigned long, ...);
static int (*real_close)(int);
static volatile int heap_fds[16];
static int mode_dumb = 1, mode_te = 1, drm_fd = -1, te_fd = -1, n_dumb, n_te;

__attribute__((constructor)) static void init(void) {
    real_open = dlsym(RTLD_NEXT, "open"); real_open64 = dlsym(RTLD_NEXT, "open64");
    real_openat = dlsym(RTLD_NEXT, "openat"); real_openat64 = dlsym(RTLD_NEXT, "openat64");
    real_access = dlsym(RTLD_NEXT, "access"); real_ioctl = dlsym(RTLD_NEXT, "ioctl"); real_close = dlsym(RTLD_NEXT, "close");
    const char *m = getenv("X1_HEAP");
    if (m && !strcmp(m, "te")) mode_dumb = 0;
    if (m && !strcmp(m, "dumb")) mode_te = 0;
    for (int i = 0; i < 16; i++) heap_fds[i] = -1;
}

static int is_heap(const char *p) { return p && !strncmp(p, "/dev/dma_heap/", 14); }
static int is_heap_fd(int fd) { for (int i = 0; i < 16; i++) if (heap_fds[i] == fd) return 1; return 0; }
static int open_heap(const char *p) {
    if (strncmp(p + 14, "system", 6)) { errno = ENOENT; return -1; }
    // The heap fd itself is a placeholder the blob keeps open; a dup of /dev/null is enough, allocation is done
    // on the real devices below.
    int fd = real_open("/dev/null", O_RDWR | O_CLOEXEC);
    for (int i = 0; fd >= 0 && i < 16; i++) if (__sync_bool_compare_and_swap(&heap_fds[i], -1, fd)) break;
    fprintf(stderr, "x1heapshim: open %s -> placeholder fd %d\n", p, fd);
    return fd;
}

static int alloc_dumb(uint64_t len) {
    if (drm_fd < 0) drm_fd = real_open("/dev/dri/card0", O_RDWR | O_CLOEXEC);
    if (drm_fd < 0) return -1;
    // The blob asks for a byte length; a dumb buffer is sized by geometry, so allocate it as one row of 32-bit pixels
    // per 4 KB page (the blob only cares about the dma-buf's size and that it is contiguous).
    uint32_t pages = (uint32_t)((len + 4095) / 4096);
    struct drm_mode_create_dumb c = { pages, 1024, 32, 0, 0, 0, 0 };
    if (real_ioctl(drm_fd, DRM_IOCTL_MODE_CREATE_DUMB, &c) < 0) return -1;
    struct drm_prime_handle ph = { c.handle, O_RDWR | O_CLOEXEC, -1 };
    int r = real_ioctl(drm_fd, DRM_IOCTL_PRIME_HANDLE_TO_FD, &ph);
    struct drm_gem_close g = { c.handle, 0 };
    real_ioctl(drm_fd, DRM_IOCTL_GEM_CLOSE, &g);
    return r < 0 ? -1 : ph.fd;
}
static int alloc_te(uint64_t len) {
    if (te_fd < 0) te_fd = real_open("/dev/dma_buf_te", O_RDWR | O_CLOEXEC);
    if (te_fd < 0) return -1;
    struct te_alloc t = { (len + 4095) / 4096 };
    return real_ioctl(te_fd, TE_ALLOC, &t);
}

static mode_t mode_arg(int fl, va_list ap) { return (fl & (O_CREAT | O_TMPFILE)) ? (mode_t)va_arg(ap, int) : 0; }
int open(const char *p, int fl, ...) { va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap); return is_heap(p) ? open_heap(p) : real_open(p, fl, m); }
int open64(const char *p, int fl, ...) { va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap); return is_heap(p) ? open_heap(p) : real_open64(p, fl, m); }
int openat(int d, const char *p, int fl, ...) { va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap); return is_heap(p) ? open_heap(p) : real_openat(d, p, fl, m); }
int openat64(int d, const char *p, int fl, ...) { va_list ap; va_start(ap, fl); mode_t m = mode_arg(fl, ap); va_end(ap); return is_heap(p) ? open_heap(p) : real_openat64(d, p, fl, m); }
int access(const char *p, int mode) { if (is_heap(p)) { if (!strncmp(p + 14, "system", 6)) return 0; errno = ENOENT; return -1; } return real_access(p, mode); }
int close(int fd) { for (int i = 0; i < 16; i++) if (heap_fds[i] == fd) __sync_bool_compare_and_swap(&heap_fds[i], fd, -1); return real_close(fd); }

int ioctl(int fd, unsigned long req, ...) {
    va_list ap; va_start(ap, req); void *arg = va_arg(ap, void *); va_end(ap);
    if (req != DMA_HEAP_IOCTL_ALLOC || !is_heap_fd(fd)) return real_ioctl(fd, req, arg);
    struct dma_heap_allocation_data *a = arg;
    int b = -1; const char *how = "none";
    if (mode_dumb) { b = alloc_dumb(a->len); if (b >= 0) { how = "drm dumb (CMA)"; n_dumb++; } }
    if (b < 0 && mode_te) { b = alloc_te(a->len); if (b >= 0) { how = "dma_buf_te (pages)"; n_te++; } }
    struct timespec ts; clock_gettime(CLOCK_MONOTONIC, &ts);
    fprintf(stderr, "x1heapshim: alloc %llu bytes -> fd %d via %s (dumb so far %d, te %d) at monotonic %.3f\n", (unsigned long long)a->len, b, how, n_dumb, n_te, ts.tv_sec + ts.tv_nsec / 1e9);
    if (b < 0) { errno = ENOMEM; return -1; }
    a->fd = (uint32_t)b;
    return 0;
}
