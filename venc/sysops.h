/* The system calls the encoder, buffer and capture sessions make, behind one table so the host tests can
 * stand a fake kernel in for /dev/venc, /dev/hma and /dev/video60 (tests/fake_sys.c). */
#ifndef LUMATV_SYSOPS_H
#define LUMATV_SYSOPS_H

#include <poll.h>
#include <stddef.h>
#include <sys/types.h>

struct sysops {
    int (*open)(const char *path, int flags);
    int (*close)(int fd);
    int (*ioctl)(int fd, unsigned long request, void *arg);
    void *(*mmap)(void *addr, size_t length, int prot, int flags, int fd, off_t offset);
    int (*munmap)(void *addr, size_t length);
    int (*poll)(struct pollfd *fds, nfds_t nfds, int timeout);
};

extern const struct sysops sysops_real;

/* Formats "what: strerror(errno)" into err (errlen bytes) and returns -1; err may be NULL. */
int sys_fail(char *err, size_t errlen, const char *what);

#endif
