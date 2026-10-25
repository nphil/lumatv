// Host test of the whole interposer under SAM's jail conditions: run by check.sh under
// LD_PRELOAD="heapshim.so fake_exporter.so" (the fake sits behind the shim). The blob's open and allocation calls
// end in a memfd from the fake dma_buf_unified; /dev/dri/card0 and /dev/dma_buf_te do not exist.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <unistd.h>

#include "heapcore.h"

static int fails;
#define CHECK(c) do { if (!(c)) { fails++; printf("FAIL line %d: %s (errno %d)\n", __LINE__, #c, errno); } } while (0)

static int alloc_fd(int heap, uint64_t len, int *err) {
  struct dma_heap_allocation_data a = {len, 0, O_RDWR | O_CLOEXEC, 0};
  errno = 0;
  int r = ioctl(heap, DMA_HEAP_IOCTL_ALLOC, &a);
  *err = errno;
  return r < 0 ? -1 : (int)a.fd;
}

int main(int argc, char **argv) {
  int full = argc > 1 && !strcmp(argv[1], "full");
  int heap = open64("/dev/dma_heap/system-uncached", O_RDWR | O_CLOEXEC);
  CHECK(heap >= 0);
  int err;
  if (full) {
    CHECK(alloc_fd(heap, 8986624, &err) == -1 && err == ENOMEM);   // the exporter is out of memory: ENOMEM, as the kernel heap
    printf(fails ? "test_shim_jail(full): %d FAILED\n" : "test_shim_jail(full): all passed\n", fails);
    return fails != 0;
  }
  // The three 1080p window buffers of the blob (8986624 bytes, seen on the set), then a small and an odd size.
  uint64_t sizes[] = {8986624, 8986624, 8986624, 4096, 5000};
  for (unsigned i = 0; i < sizeof sizes / sizeof *sizes; i++) {
    int fd = alloc_fd(heap, sizes[i], &err);
    CHECK(fd >= 0);
    if (fd < 0) continue;
    char link[64], target[64];
    snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
    ssize_t n = readlink(link, target, sizeof target - 1);
    CHECK(n > 0 && (target[n] = 0, !strncmp(target, "/memfd:dma_buf_unified", 22)));
    struct stat st;
    CHECK(fstat(fd, &st) == 0 && (uint64_t)st.st_size >= sizes[i] && (uint64_t)st.st_size < sizes[i] + 4096);
    close(fd);
  }
  // The exporter's device descriptor is the shim's, not the blob's: the heap fd is still a /dev/null placeholder.
  char link[64], target[64];
  snprintf(link, sizeof link, "/proc/self/fd/%d", heap);
  ssize_t n = readlink(link, target, sizeof target - 1);
  CHECK(n > 0 && (target[n] = 0, !strcmp(target, "/dev/null")));
  printf(fails ? "test_shim_jail: %d FAILED\n" : "test_shim_jail: all passed\n", fails);
  return fails != 0;
}
