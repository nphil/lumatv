/* One physically contiguous buffer from LG's HMA pools (/dev/hma), mapped for the CPU. The physical address
 * is what the encoder and the video tap speak; the dma-buf export is how a GPU would reach the same bytes. */
#ifndef LUMATV_HMA_H
#define LUMATV_HMA_H

#include <stddef.h>
#include <stdint.h>

#include "hma_abi.h"
#include "sysops.h"

struct hma_buf {
    const struct sysops *sys;
    int fd;         /* the /dev/hma fd that owns the allocation; -1 when empty */
    uint32_t paddr; /* physical address */
    uint8_t *vaddr; /* CPU mapping, size bytes */
    size_t size;
    int cached; /* 1: cached mapping (needs hma_cache before/after device access), 0: write-combined */
};

/* Allocates size bytes (align bytes aligned, 4096 is right for the encoder and the GPU) from pool
 * ("photofb" is what LG's encoder plugin uses) and maps it. cached=0 is the safe choice for frames the
 * CPU writes once and a device reads. Returns 0, or -1 with err filled. */
int hma_alloc(struct hma_buf *b, const struct sysops *sys, const char *pool, size_t size, size_t align,
              int cached, char *err, size_t errlen);

/* Cache maintenance for a cached mapping: HMA_CACHE_CLEAN before a device reads, HMA_CACHE_INV after a
 * device wrote. A no-op for a write-combined mapping. */
int hma_cache(struct hma_buf *b, int operation, size_t offset, size_t length);

/* Exports the buffer as a dma-buf fd (caller closes it). */
int hma_export_dmabuf(struct hma_buf *b, char *err, size_t errlen);

/* Unmaps and frees (closing the fd frees the allocation in the kernel). Safe on an empty struct. */
void hma_free(struct hma_buf *b);

#endif
