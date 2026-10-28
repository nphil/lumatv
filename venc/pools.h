/* The HMA pool table from /proc/hma/status ("0x<base>\t0x<size>\t<name>" per line), so a physical address
 * the MCU or the tap hands back can be checked against memory that exists before anything maps or
 * encodes it. The first slot measured this table on the set (docs/research/venc-result.md). */
#ifndef LUMATV_POOLS_H
#define LUMATV_POOLS_H

#include <stddef.h>
#include <stdint.h>

#define POOLS_MAX 64

struct pool {
    uint64_t base;
    uint64_t size;
    char name[24];
};

struct pools {
    struct pool p[POOLS_MAX];
    unsigned n;
};

/* Parses text in the /proc/hma/status format. Returns the number of pools read. */
unsigned pools_parse(struct pools *ps, const char *text);

/* Reads /proc/hma/status (or path). Returns the number of pools, 0 when the file is absent. */
unsigned pools_read(struct pools *ps, const char *path);

/* The pool that holds [paddr, paddr+len), or NULL. */
const struct pool *pools_find(const struct pools *ps, uint64_t paddr, uint64_t len);

#endif
