/* A deterministic NV12 test picture for the encoder probe: colour bars that slide one column a frame, a
 * frame counter rendered as a bar of bits along the bottom, and a diagonal line, so a decoded clip shows
 * motion, frame order and chroma at a glance and the encoder has real work to do. */
#ifndef LUMATV_SYNTH_H
#define LUMATV_SYNTH_H

#include <stdint.h>

/* y: width*height bytes, uv: width*height/2 bytes (interleaved Cb Cr), stride == width. */
void synth_nv12(uint8_t *y, uint8_t *uv, uint32_t width, uint32_t height, uint32_t frame);

/* The counter the bottom bar of a frame encodes (for a test that reads it back from the luma plane). */
uint32_t synth_read_counter(const uint8_t *y, uint32_t width, uint32_t height);

#endif
