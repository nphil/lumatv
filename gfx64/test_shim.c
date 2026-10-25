// Host test of the interposer's bookkeeping: run under LD_PRELOAD=heapshim.so (native build) by check.sh. No GPU
// devices exist on the host, so allocations fail with ENOMEM; what is checked is which calls the shim claims and
// which it lets through.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include "heapcore.h"

static int fails;
#define CHECK(c) do { if (!(c)) { fails++; printf("FAIL line %d: %s (errno %d)\n", __LINE__, #c, errno); } } while (0)

static int heap_alloc_call(int fd) {
  struct dma_heap_allocation_data a = {8986624, 0, O_RDWR | O_CLOEXEC, 0};
  errno = 0;
  return ioctl(fd, DMA_HEAP_IOCTL_ALLOC, &a);
}

int main(void) {
  char link[64], target[64];

  // Heap nodes: system* served, everything else absent.
  CHECK(access("/dev/dma_heap/system-uncached", R_OK | W_OK) == 0);
  CHECK(access("/dev/dma_heap/system", F_OK) == 0);
  errno = 0; CHECK(access("/dev/dma_heap/protected", F_OK) == -1 && errno == ENOENT);
  errno = 0; CHECK(open("/dev/dma_heap/protected", O_RDWR) == -1 && errno == ENOENT);
  errno = 0; CHECK(open64("/dev/dma_heap/cma", O_RDWR) == -1 && errno == ENOENT);
  CHECK(access("/dev/null", F_OK) == 0);   // other paths untouched

  int fd = open64("/dev/dma_heap/system-uncached", O_RDWR | O_CLOEXEC);
  CHECK(fd >= 0);
  snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
  ssize_t n = readlink(link, target, sizeof target - 1);
  CHECK(n > 0 && (target[n] = 0, !strcmp(target, "/dev/null")));
  CHECK(fcntl(fd, F_GETFD) & FD_CLOEXEC);

  // openat/open on the heap path behave the same.
  int fd2 = openat(AT_FDCWD, "/dev/dma_heap/system", O_RDWR);
  CHECK(fd2 >= 0);
  close(fd2);

  // The heap request on a heap fd is claimed (no devices on the host, so it fails with ENOMEM, not ENOTTY).
  CHECK(heap_alloc_call(fd) == -1 && errno == ENOMEM);
  // A dup of the heap fd stays a heap fd; dup2/dup3 likewise.
  int d = dup(fd);
  CHECK(d >= 0 && heap_alloc_call(d) == -1 && errno == ENOMEM);
  int d2 = dup2(fd, 900);
  CHECK(d2 == 900 && heap_alloc_call(900) == -1 && errno == ENOMEM);
  int d3 = dup3(fd, 901, O_CLOEXEC);
  CHECK(d3 == 901 && heap_alloc_call(901) == -1 && errno == ENOMEM);

  // Any other fd passes straight through, including the same ioctl number: /dev/null answers ENOTTY.
  int plain = open("/dev/null", O_RDWR);
  CHECK(plain >= 0 && heap_alloc_call(plain) == -1 && errno == ENOTTY);
  // Overwriting a heap fd with a plain one drops the claim.
  CHECK(dup2(plain, 900) == 900 && heap_alloc_call(900) == -1 && errno == ENOTTY);

  // Closing releases the claim: the number comes back as an ordinary fd.
  close(d); close(901); close(fd);
  int again = open("/dev/null", O_RDWR);
  CHECK(again >= 0 && heap_alloc_call(again) == -1 && errno == ENOTTY);

  // Other ioctls on a heap fd and on others pass through.
  int hfd = open("/dev/dma_heap/system-uncached", O_RDWR);
  int one = 0;
  errno = 0; CHECK(ioctl(hfd, FIONREAD, &one) == -1 && errno == ENOTTY);   // /dev/null: not a tty
  close(hfd);

  printf(fails ? "test_shim: %d FAILED\n" : "test_shim: all passed\n", fails);
  return fails != 0;
}
