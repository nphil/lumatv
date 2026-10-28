#include "synth.h"

#include <string.h>

/* The eight SMPTE bars in BT.601 limited-range YCbCr. */
static const uint8_t bars[8][3] = {
    { 235, 128, 128 }, /* white */
    { 210, 16, 146 },  /* yellow */
    { 170, 166, 16 },  /* cyan */
    { 145, 54, 34 },   /* green */
    { 106, 202, 222 }, /* magenta */
    { 81, 90, 240 },   /* red */
    { 41, 240, 110 },  /* blue */
    { 16, 128, 128 },  /* black */
};

#define COUNTER_BITS 16
#define COUNTER_ROWS 24

void synth_nv12(uint8_t *y, uint8_t *uv, uint32_t width, uint32_t height, uint32_t frame)
{
    uint32_t bar_w = width / 8 ? width / 8 : 1;
    uint32_t shift = frame % width;
    uint32_t body = height > COUNTER_ROWS ? height - COUNTER_ROWS : height;
    for (uint32_t row = 0; row < body; row++) {
        uint8_t *yr = y + (size_t)row * width;
        for (uint32_t col = 0; col < width; col++) {
            uint32_t b = (((col + shift) / bar_w) & 7);
            yr[col] = bars[b][0];
        }
        /* the diagonal: one bright pixel per row, moving with the frame */
        uint32_t d = (row + frame) % width;
        yr[d] = 255;
        if (d + 1 < width)
            yr[d + 1] = 255;
    }
    for (uint32_t row = 0; row < body / 2; row++) {
        uint8_t *uvr = uv + (size_t)row * width;
        for (uint32_t col = 0; col < width / 2; col++) {
            uint32_t b = (((2 * col + shift) / bar_w) & 7);
            uvr[2 * col] = bars[b][1];
            uvr[2 * col + 1] = bars[b][2];
        }
    }
    /* the counter: 16 cells across the bottom rows, bright for a 1 bit, dark for 0, MSB left */
    uint32_t cell = width / COUNTER_BITS;
    for (uint32_t row = body; row < height; row++) {
        uint8_t *yr = y + (size_t)row * width;
        for (uint32_t col = 0; col < width; col++) {
            uint32_t bit = COUNTER_BITS - 1 - (col / cell < COUNTER_BITS ? col / cell : COUNTER_BITS - 1);
            yr[col] = (frame >> bit) & 1 ? 235 : 16;
        }
    }
    for (uint32_t row = body / 2; row < height / 2; row++)
        memset(uv + (size_t)row * width, 128, width);
}

uint32_t synth_read_counter(const uint8_t *y, uint32_t width, uint32_t height)
{
    if (height <= COUNTER_ROWS)
        return 0;
    uint32_t cell = width / COUNTER_BITS;
    const uint8_t *yr = y + (size_t)(height - COUNTER_ROWS / 2) * width;
    uint32_t v = 0;
    for (uint32_t i = 0; i < COUNTER_BITS; i++) {
        uint32_t col = i * cell + cell / 2;
        v = (v << 1) | (yr[col] > 128);
    }
    return v;
}
