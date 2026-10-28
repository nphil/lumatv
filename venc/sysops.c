#include "sysops.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

static int real_open(const char *path, int flags) { return open(path, flags | O_CLOEXEC); }
static int real_ioctl(int fd, unsigned long request, void *arg) { return ioctl(fd, request, arg); }

const struct sysops sysops_real = {
    .open = real_open,
    .close = close,
    .ioctl = real_ioctl,
    .mmap = mmap,
    .munmap = munmap,
    .poll = poll,
};

int sys_fail(char *err, size_t errlen, const char *what)
{
    int e = errno;
    if (err && errlen)
        snprintf(err, errlen, "%s: %s (errno %d)", what, strerror(e), e);
    errno = e;
    return -1;
}
