/* What the window carries over its video: a veil (black at some alpha: the dim, and the fade between prints) and the corner clock.
 * The window's buffer is ARGB8888, premultiplied, as Wayland's shm wants it; a veil of alpha A is the word A << 24 and a pixel of the
 * clock is its colour over that.
 */
#ifndef VPL_UI_H
#define VPL_UI_H

#include <stdint.h>

#include "vpl_vpc.h"

typedef struct {
    uint32_t *px;
    int w, h;
    int stride;          /* in pixels */
} vui_canvas;

typedef struct { int x0, y0, x1, y1; } vui_rect;   /* x1 and y1 are outside */

int vui_empty(vui_rect r);
vui_rect vui_union(vui_rect a, vui_rect b);
vui_rect vui_clip(vui_rect r, const vui_canvas *c);

/* Every pixel of `r` becomes the veil. */
void vui_fill(vui_canvas *c, vui_rect r, uint8_t veil);

/* The box `text` covers when its right edge is at `right` and its baseline at `baseline`; empty if a character has no glyph. */
vui_rect vui_text_rect(const vpg_t *g, const char *text, int right, int baseline);

/* `text` in colour `rgb` (0xRRGGBB) at `alpha` over a veil: the box is reset to the veil first, then the glyphs are laid on it. */
void vui_draw_text(vui_canvas *c, const vpg_t *g, const char *text, int right, int baseline, uint32_t rgb, int alpha, uint8_t veil);

/* The box a caption covers when it is drawn `dx`, `dy` from its place. */
vui_rect vui_caption_rect(const vpt_t *t, int dx, int dy);

/* The caption over a veil: the box is reset to the veil, then the coverage is laid on in the artist's ink for the first `split` columns
 * and the other ink for the rest; `opacity` (0..255) scales both alphas (the caption fades with its print). */
void vui_draw_caption(vui_canvas *c, const vpt_t *t, int dx, int dy, uint32_t ink, int ink_alpha, uint32_t ink2, int ink2_alpha, int opacity, uint8_t veil);

#endif
