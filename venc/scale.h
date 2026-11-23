/* NV12 to smaller NV12, area averaged, on the CPU: what makes a 1080-line station's picture into 720 lines for the
 * `mobile` profile, since the decoder and the encoder cannot scale. The source is a decoded picture in HMA memory
 * (uncached: about 1 GB/s with wide loads, 50 MB/s byte by byte), so every source row is copied once, with wide
 * loads, into a cached line before anything reads it; the destination is written a row at a time, sequentially.
 *
 * Each output sample is the exact average of the source area it covers (a box filter with fractional edges), in
 * integers: a 2:1 reduction is the mean of four samples, 3:2 weighs them 1, 1/2, 1/2 and 1/4 in the middle. The
 * chroma plane is scaled per channel (Cb and Cr stay interleaved), so a 4:2:0 picture stays 4:2:0.
 *
 *   s = scale_new(1920, 1080, 1280, 720); scale_run(s, ...) per picture; scale_free(s). */
#ifndef LUMATV_SCALE_H
#define LUMATV_SCALE_H

#include <stddef.h>
#include <stdint.h>

struct scaler;

/* A scaler from sw x sh to dw x dh. All four are even, the destination is no larger than the source and no smaller
 * than a sixteenth of it in either direction. NULL for sizes that do not fit that, or without memory. */
struct scaler *scale_new(unsigned sw, unsigned sh, unsigned dw, unsigned dh);

/* One picture: luma plane (sw x sh bytes, rows sy_stride apart) and interleaved chroma plane (sw/2 pairs by sh/2 rows,
 * rows sc_stride apart), to the same two planes at dw x dh. Not reentrant on one scaler. */
void scale_run(struct scaler *s, const uint8_t *sy, unsigned sy_stride, const uint8_t *sc, unsigned sc_stride, uint8_t *dy,
               unsigned dy_stride, uint8_t *dc, unsigned dc_stride);

void scale_free(struct scaler *s);

#endif
