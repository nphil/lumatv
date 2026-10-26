#include "vpl_ui.h"

#include <string.h>

int vui_empty(vui_rect r) { return r.x1 <= r.x0 || r.y1 <= r.y0; }

vui_rect vui_union(vui_rect a, vui_rect b) {
    if (vui_empty(a)) return b;
    if (vui_empty(b)) return a;
    vui_rect u = {a.x0 < b.x0 ? a.x0 : b.x0, a.y0 < b.y0 ? a.y0 : b.y0, a.x1 > b.x1 ? a.x1 : b.x1, a.y1 > b.y1 ? a.y1 : b.y1};
    return u;
}

vui_rect vui_clip(vui_rect r, const vui_canvas *c) {
    if (r.x0 < 0) r.x0 = 0;
    if (r.y0 < 0) r.y0 = 0;
    if (r.x1 > c->w) r.x1 = c->w;
    if (r.y1 > c->h) r.y1 = c->h;
    return r;
}

void vui_fill(vui_canvas *c, vui_rect r, uint8_t veil) {
    r = vui_clip(r, c);
    if (vui_empty(r)) return;
    uint32_t v = (uint32_t)veil << 24;
    for (int y = r.y0; y < r.y1; y++) {
        uint32_t *row = c->px + (size_t)y * (size_t)c->stride;
        if (v == 0) memset(row + r.x0, 0, sizeof(uint32_t) * (size_t)(r.x1 - r.x0));
        else
            for (int x = r.x0; x < r.x1; x++) row[x] = v;
    }
}

vui_rect vui_text_rect(const vpg_t *g, const char *text, int right, int baseline) {
    vui_rect none = {0, 0, 0, 0};
    int total = 0, top = 0, bottom = 0;
    for (const char *p = text; *p; p++) {
        const vpg_glyph *gl = vpg_find(g, (unsigned char)*p);
        if (!gl) return none;
        total += gl->advance;
        if (-gl->top < top) top = -gl->top;
        if ((int)gl->h - gl->top > bottom) bottom = (int)gl->h - gl->top;
    }
    /* a pixel of room each side: a glyph's antialiased edge can reach past its advance */
    vui_rect r = {right - total - 2, baseline + top - 1, right + 2, baseline + bottom + 1};
    return r;
}

static uint32_t mul255(uint32_t a, uint32_t b) {
    uint32_t t = a * b + 128;
    return (t + (t >> 8)) >> 8;
}

void vui_draw_text(vui_canvas *c, const vpg_t *g, const char *text, int right, int baseline, uint32_t rgb, int alpha, uint8_t veil) {
    vui_rect box = vui_text_rect(g, text, right, baseline);
    if (vui_empty(box)) return;
    vui_fill(c, box, veil);
    int total = 0;
    for (const char *p = text; *p; p++) {
        const vpg_glyph *gl = vpg_find(g, (unsigned char)*p);
        if (gl) total += gl->advance;
    }
    int pen = right - total;
    uint32_t cr = (rgb >> 16) & 0xFF, cg = (rgb >> 8) & 0xFF, cb = rgb & 0xFF;
    for (const char *p = text; *p; p++) {
        const vpg_glyph *gl = vpg_find(g, (unsigned char)*p);
        if (!gl) continue;
        for (uint32_t gy = 0; gy < gl->h; gy++) {
            int y = baseline - gl->top + (int)gy;
            if (y < 0 || y >= c->h) continue;
            uint32_t *row = c->px + (size_t)y * (size_t)c->stride;
            for (uint32_t gx = 0; gx < gl->w; gx++) {
                int x = pen + gl->left + (int)gx;
                if (x < 0 || x >= c->w) continue;
                uint32_t cover = gl->alpha[gy * gl->w + gx];
                if (!cover) continue;
                uint32_t a = mul255(cover, (uint32_t)alpha);
                uint32_t d = row[x];
                uint32_t inv = 255 - a;
                uint32_t oa = a + mul255(d >> 24, inv);
                uint32_t or_ = mul255(cr, a) + mul255((d >> 16) & 0xFF, inv);
                uint32_t og = mul255(cg, a) + mul255((d >> 8) & 0xFF, inv);
                uint32_t ob = mul255(cb, a) + mul255(d & 0xFF, inv);
                row[x] = oa << 24 | or_ << 16 | og << 8 | ob;
            }
        }
        pen += gl->advance;
    }
}

vui_rect vui_caption_rect(const vpt_t *t, int dx, int dy) {
    vui_rect r = {t->x + dx, t->y + dy, t->x + dx + (int)t->w, t->y + dy + (int)t->h};
    return r;
}

void vui_draw_caption(vui_canvas *c, const vpt_t *t, int dx, int dy, uint32_t ink, int ink_alpha, uint32_t ink2, int ink2_alpha, int opacity, uint8_t veil) {
    vui_rect box = vui_caption_rect(t, dx, dy);
    vui_fill(c, box, veil);
    if (opacity <= 0) return;
    for (uint32_t gy = 0; gy < t->h; gy++) {
        int y = t->y + dy + (int)gy;
        if (y < 0 || y >= c->h) continue;
        uint32_t *row = c->px + (size_t)y * (size_t)c->stride;
        for (uint32_t gx = 0; gx < t->w; gx++) {
            int x = t->x + dx + (int)gx;
            if (x < 0 || x >= c->w) continue;
            uint32_t cover = t->alpha[gy * t->w + gx];
            if (!cover) continue;
            int first = gx < t->split;
            uint32_t rgb = first ? ink : ink2;
            uint32_t a = mul255(mul255(cover, (uint32_t)(first ? ink_alpha : ink2_alpha)), (uint32_t)opacity);
            uint32_t d = row[x];
            uint32_t inv = 255 - a;
            uint32_t oa = a + mul255(d >> 24, inv);
            uint32_t or_ = mul255((rgb >> 16) & 0xFF, a) + mul255((d >> 16) & 0xFF, inv);
            uint32_t og = mul255((rgb >> 8) & 0xFF, a) + mul255((d >> 8) & 0xFF, inv);
            uint32_t ob = mul255(rgb & 0xFF, a) + mul255(d & 0xFF, inv);
            row[x] = oa << 24 | or_ << 16 | og << 8 | ob;
        }
    }
}
