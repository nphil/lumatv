/* The CPU decoder: libjpeg.so.62, the library the set ships (libjpeg-turbo), opened at run time. It is lumatv-jpeg's own fallback
 * when the hardware block is not there or does not take a file, and the baseline the TV slot times the block against.
 *
 * It decodes to the planes the JPEG holds (libjpeg's raw data output, no colour conversion, no chroma upsampling), at 1/1, 1/2,
 * 1/4 or 1/8 per axis (libjpeg's scaled IDCT), so the same converters turn them into NV12 and RGBA as the hardware's buffer.
 */
#ifndef LUMATV_JPEG_CPU_H
#define LUMATV_JPEG_CPU_H

#include <stddef.h>
#include <stdint.h>

#include "convert.h"

enum {
    CPU_OK = 0,
    CPU_NO_LIBRARY = 1, /* libjpeg.so.62 did not load, or lacks a function */
    CPU_UNSUPPORTED = 2, /* a file this build does not take: not 8-bit YCbCr or gray, chroma sampled other than 1 or 2 per axis */
    CPU_FAILED = 3,      /* libjpeg refused the file (corrupt, truncated beyond repair) or the structure size is not the library's */
    CPU_NOMEM = 4,
};

struct cpu_planes {
    uint8_t *buf;         /* malloc'd; one block holding every plane */
    struct yuv_image im;  /* points into buf; width and height are the decoded picture's */
};

/* Decodes `jpeg` at 1 / (1 << shift) per axis. On CPU_OK `out` holds the planes and `free(out->buf)` releases them. */
int cpu_decode(const uint8_t *jpeg, size_t n, unsigned shift, struct cpu_planes *out);

#endif
