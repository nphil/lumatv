/* A software decoder for one stream of H.264 or H.265 access units: FFmpeg's decoders, loaded at run time from the feeder's own `lib/` directory
 * (libavutil-lumatv.so.N and libavcodec-lumatv.so.N, built by third_party/build-ffmpeg.sh: the decoders and nothing else, LGPL, shared so that they
 * can be replaced). This is the camera's picture when no hardware video window is free (the compositor grants a texture instead, hw-planes and
 * multiview-result.md): the feeder decodes here, scales and converts with gf_yuv.c and paints its window's buffer, which the compositor draws in the
 * claim's rectangle. One thread, low delay, no frame reordering wait: a picture comes out when its access unit goes in.
 *
 * The libraries are never linked: a feeder started without them still plays through NDL, and a load for a texture says why it cannot. */
#ifndef GF_SWDEC_H
#define GF_SWDEC_H

#include <stddef.h>
#include <stdint.h>

#include "gf_yuv.h"

typedef struct gf_swdec gf_swdec;

/* Opens a decoder for `codec` (gf_annexb.h GF_CODEC_H264 or GF_CODEC_H265) of a stream that says it is width x height. `libdir` holds the two libraries
 * (the feeder's own `lib/`); a NULL or empty one is the loader's own search (the tests put the host build on LD_LIBRARY_PATH). 0, or -1 with the
 * reason in `err`. */
int gf_swdec_open(gf_swdec **out, const char *libdir, unsigned codec, unsigned width, unsigned height, char *err, size_t cap);

/* Decodes one access unit (Annex B, one picture). 1: a picture came out (gf_swdec_picture has it), 0: none yet, -1: the unit did not decode (the
 * decoder drops what it cannot use and waits for the next key picture; `err` says what FFmpeg said). */
int gf_swdec_decode(gf_swdec *d, const void *unit, size_t size, int64_t pts, char *err, size_t cap);

/* The last picture gf_swdec_decode returned 1 for, valid until the next call. 0, or -1 when its pixel format is not 8-bit 4:2:0 (the texture path
 * takes what cameras send; `err` names the format). */
int gf_swdec_picture(const gf_swdec *d, gf_yuv *out, char *err, size_t cap);

/* The stream's size as the decoder knows it now (0 before the first picture). */
void gf_swdec_size(const gf_swdec *d, int *width, int *height);

/* Drops what the decoder holds (after a stall or a keyframe request). */
void gf_swdec_flush(gf_swdec *d);

void gf_swdec_close(gf_swdec *d);

#endif
