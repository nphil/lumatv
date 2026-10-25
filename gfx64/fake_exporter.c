// Test double for SAM's jail, preloaded AFTER heapshim.so so the shim's "real" open and ioctl land here: /dev has
// dma_buf_unified and no dri/card0 or dma_buf_te. An allocation request on the unified device returns a memfd of the
// requested pages, as the kernel exporter returns a dma-buf fd. The request is decoded the way LG's older exporter
// does (include/linux/dma-buf-unified.h: {u32 size in pages; u32 map_cached}), which also is the layout of the Arm
// exporter's u64 on a little-endian machine; map_cached other than 0 is refused so a shim that sent junk in the
// high word fails the test. FAKE_EXPORTER_FULL=1 makes every allocation fail with ENOMEM.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include "heapcore.h"

struct unified_req { uint32_t size; uint32_t map_cached; };
_Static_assert(sizeof(struct unified_req) == sizeof(struct te_alloc), "same size");
_Static_assert(TE_ALLOC == _IOR('E', 0x01, struct unified_req), "same request number as dma_buf_unified's alloc");

static int unified_fd = -1;

int open(const char *p, int fl, ...) {
  va_list ap; va_start(ap, fl); mode_t m = (fl & (O_CREAT | O_TMPFILE)) ? (mode_t)va_arg(ap, int) : 0; va_end(ap);
  if (!strcmp(p, HEAP_DRM_PATH) || !strcmp(p, HEAP_TE_PATH)) { errno = ENOENT; return -1; }
  static int (*real_open)(const char *, int, ...);
  if (!real_open) real_open = dlsym(RTLD_NEXT, "open");
  if (!strcmp(p, HEAP_UNIFIED_PATH)) {
    int fd = real_open("/dev/null", O_RDWR | O_CLOEXEC);
    unified_fd = fd;
    return fd;
  }
  return real_open(p, fl, m);
}

int ioctl(int fd, unsigned long req, ...) {
  va_list ap; va_start(ap, req); void *arg = va_arg(ap, void *); va_end(ap);
  if (fd == unified_fd && fd >= 0) {
    if (req != TE_ALLOC) { errno = ENOTTY; return -1; }
    struct unified_req r;
    memcpy(&r, arg, sizeof r);
    if (r.size == 0 || r.map_cached != 0) { errno = EFAULT; return -1; }
    if (getenv("FAKE_EXPORTER_FULL")) { errno = ENOMEM; return -1; }
    int m = memfd_create("dma_buf_unified", MFD_CLOEXEC);
    if (m < 0) return -1;
    if (ftruncate(m, (off_t)r.size * 4096) < 0) { close(m); return -1; }
    return m;
  }
  static int (*real_ioctl)(int, unsigned long, ...);
  if (!real_ioctl) real_ioctl = dlsym(RTLD_NEXT, "ioctl");
  return real_ioctl(fd, req, arg);
}
