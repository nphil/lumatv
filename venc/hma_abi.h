/* LG HMA (hardware memory allocator) user ABI: /dev/hma on the C4 (char major 1819, mode 0666).
 *
 * Source: LG's published O22 tree, kdriver/core/hma/hma_drv.h and hma_drv.c, and libhma.h (the wrapper
 * LG's own encoder plugin links). One fd holds at most one allocation: HMA_IOCTL_ALLOC takes a pool name,
 * size and alignment and returns the physical address; mmap(fd, size) maps it (non-cached or
 * write-combined unless HMA_IOCTL_CACHED was set first); HMA_IOCTL_GET_DMABUF exports it as a dma-buf fd
 * (a GPU can import that); close() frees it. Physical addresses are 32-bit in this ABI ("user layer
 * assumes 32-bit physical addresses", hma_drv.h).
 *
 * Pools on this set (platform/o22_pioneer/platform_init.c): "vpb" 300 MB, "vpb8k" 148 MB, "cpb" 66 MB,
 * "venc0" 10 MB, "venc1" 12 MB (secure), "debug0", "debug1", "surface" 79 MB, "npu" 15 MB, "audio_buffer0";
 * aliases "photofb" and "cpb2" -> "cpb" and (4K model) "venc0" -> "cpb". /proc/hma/status lists them with
 * their free space. LG's encoder plugin allocates its input frames from "photofb".
 */
#ifndef LUMATV_HMA_ABI_H
#define LUMATV_HMA_ABI_H

#include <stdint.h>
#include <sys/ioctl.h>

#define HMA_DEV "/dev/hma"
#define HMA_POOL_LEN 16
#define HMA_IOCTL_TYPE 'h'

struct hma_ioctl_alloc {
    char pool[HMA_POOL_LEN]; /* pool or alias name; empty = the default pool */
    int32_t size;
    int32_t align;
    uint32_t paddr; /* [OUT] physical address */
};

struct hma_ioctl_cache_ctl {
    uint32_t paddr;
    uint32_t offset;
    uint32_t size;
    int32_t operation; /* HMA_CACHE_INV after a device wrote, HMA_CACHE_CLEAN before a device reads */
};
#define HMA_CACHE_INV 0
#define HMA_CACHE_CLEAN 1

struct hma_ioctl_pool_info {
    char pool[HMA_POOL_LEN];
    uint64_t addr;
    int32_t size;
};

#define HMA_IOCTL_CACHE_CTL _IOW(HMA_IOCTL_TYPE, 2, struct hma_ioctl_cache_ctl)
#define HMA_IOCTL_CACHED _IOW(HMA_IOCTL_TYPE, 4, int32_t)
#define HMA_IOCTL_ALLOC _IOWR(HMA_IOCTL_TYPE, 5, struct hma_ioctl_alloc)
#define HMA_IOCTL_GET_POOL_INFO _IOWR(HMA_IOCTL_TYPE, 6, struct hma_ioctl_pool_info)
#define HMA_IOCTL_GET_DMABUF _IOR(HMA_IOCTL_TYPE, 9, int32_t)

_Static_assert(sizeof(struct hma_ioctl_alloc) == 28, "hma_ioctl_alloc is 28 bytes");
_Static_assert(sizeof(struct hma_ioctl_cache_ctl) == 16, "hma_ioctl_cache_ctl is 16 bytes");
_Static_assert(sizeof(struct hma_ioctl_pool_info) == 32, "hma_ioctl_pool_info is 32 bytes");

#endif
