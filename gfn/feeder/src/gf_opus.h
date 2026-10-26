/* libopus's decoder (build/opus, pinned 1.5.2, fixed-point) behind three calls: what the feeder uses when NDL refuses the Opus load and
 * takes PCM instead. Mono and stereo only, which is what GeForce NOW streams (the engine loads two channels at 48 kHz).
 */
#ifndef GF_OPUS_H
#define GF_OPUS_H

#include <stddef.h>
#include <stdint.h>

typedef struct gf_opus gf_opus;

/* A decoder for `rate` Hz (8, 12, 16, 24 or 48 k) and 1 or 2 channels; NULL with `err` filled when libopus says no. */
gf_opus *gf_opus_open(unsigned rate, unsigned channels, char *err, size_t cap);

/* One packet to interleaved S16 samples: the frames per channel (at most `max_frames`, 5760 holds any packet), or a negative libopus error. */
int gf_opus_decode(gf_opus *d, const uint8_t *packet, size_t n, int16_t *pcm, int max_frames);

void gf_opus_close(gf_opus *d);

#define GF_OPUS_MAX_FRAMES 5760     /* 120 ms at 48 kHz */

#endif
