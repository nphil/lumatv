#include "hma.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

#include "hma_abi.h"
#include "watchdog.h"

int hma_alloc(struct hma_buf *b, const struct sysops *sys, const char *pool, size_t size, size_t align,
              int cached, char *err, size_t errlen)
{
    memset(b, 0, sizeof(*b));
    b->sys = sys;
    b->fd = -1;
    if (size == 0 || size > 0x7fffffffu || strlen(pool) >= HMA_POOL_LEN) {
        errno = EINVAL;
        return sys_fail(err, errlen, "hma: bad size or pool name");
    }
    wd_arm("open /dev/hma", WD_HMA_MS);
    b->fd = sys->open(HMA_DEV, O_RDWR);
    wd_disarm();
    if (b->fd < 0)
        return sys_fail(err, errlen, "open " HMA_DEV);
    if (cached) {
        int32_t one = 1;
        wd_arm("HMA_IOCTL_CACHED", WD_HMA_MS);
        int r = sys->ioctl(b->fd, HMA_IOCTL_CACHED, &one);
        wd_disarm();
        if (r < 0) {
            sys_fail(err, errlen, "HMA_IOCTL_CACHED");
            hma_free(b);
            return -1;
        }
        b->cached = 1;
    }
    struct hma_ioctl_alloc a;
    memset(&a, 0, sizeof(a));
    snprintf(a.pool, sizeof(a.pool), "%s", pool);
    a.size = (int32_t)size;
    a.align = (int32_t)align;
    wd_arm("HMA_IOCTL_ALLOC", WD_HMA_MS);
    int ar = sys->ioctl(b->fd, HMA_IOCTL_ALLOC, &a);
    wd_disarm();
    if (ar < 0 || a.paddr == 0) {
        if (a.paddr == 0 && errno == 0)
            errno = ENOMEM;
        sys_fail(err, errlen, "HMA_IOCTL_ALLOC");
        hma_free(b);
        return -1;
    }
    b->paddr = a.paddr;
    b->size = size;
    if (a.paddr & 0xfff) {
        errno = EPROTO;
        snprintf(err, errlen, "HMA_IOCTL_ALLOC returned an unaligned address 0x%08x; not mapping it", a.paddr);
        hma_free(b);
        return -1;
    }
    wd_arm("mmap /dev/hma", WD_MMAP_MS);
    void *m = sys->mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, b->fd, 0);
    wd_disarm();
    if (m == MAP_FAILED) {
        sys_fail(err, errlen, "mmap " HMA_DEV);
        hma_free(b);
        return -1;
    }
    b->vaddr = m;
    return 0;
}

int hma_cache(struct hma_buf *b, int operation, size_t offset, size_t length)
{
    if (!b->cached)
        return 0;
    struct hma_ioctl_cache_ctl c = {
        .paddr = b->paddr, .offset = (uint32_t)offset, .size = (uint32_t)length, .operation = operation,
    };
    wd_arm("HMA_IOCTL_CACHE_CTL", WD_HMA_MS);
    int r = b->sys->ioctl(b->fd, HMA_IOCTL_CACHE_CTL, &c);
    wd_disarm();
    return r;
}

int hma_export_dmabuf(struct hma_buf *b, char *err, size_t errlen)
{
    int32_t dmabuf = -1;
    wd_arm("HMA_IOCTL_GET_DMABUF", WD_HMA_MS);
    int r = b->sys->ioctl(b->fd, HMA_IOCTL_GET_DMABUF, &dmabuf);
    wd_disarm();
    if (r < 0)
        return sys_fail(err, errlen, "HMA_IOCTL_GET_DMABUF");
    /* the driver returns the fd both as the ioctl result and through the argument */
    return dmabuf >= 0 ? dmabuf : r;
}

void hma_free(struct hma_buf *b)
{
    if (!b->sys)
        return;
    if (b->vaddr)
        b->sys->munmap(b->vaddr, b->size);
    if (b->fd >= 0)
        b->sys->close(b->fd);
    b->vaddr = NULL;
    b->fd = -1;
    b->paddr = 0;
    b->size = 0;
}
