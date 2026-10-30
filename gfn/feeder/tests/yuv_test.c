/* gf_yuv.c on the host: the colour of the primaries and the greys under each matrix and range, a size that is not the source's (the box halving and the
 * bilinear pass keep a flat picture flat and a gradient a gradient), odd sizes, the refusals, and what a 1280x720 picture into a 480x270 box costs here
 * (printed, not asserted: the set's number is the TV check's). */
#define _GNU_SOURCE
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "gf_yuv.h"

static int fails;
#define CHECK(c, ...) do { if (!(c)) { fails++; printf("FAIL %s:%d: ", __FILE__, __LINE__); printf(__VA_ARGS__); printf("\n"); } } while (0)

static int near_(int a, int b, int tol) { return abs(a - b) <= tol; }

typedef struct { uint8_t *y, *u, *v; int w, h, cw, ch; } pic;

static pic make(int w, int h, int y, int u, int v) {
    pic p = {0};
    p.w = w, p.h = h, p.cw = (w + 1) / 2, p.ch = (h + 1) / 2;
    p.y = malloc((size_t)w * h), p.u = malloc((size_t)p.cw * p.ch), p.v = malloc((size_t)p.cw * p.ch);
    memset(p.y, y, (size_t)w * h), memset(p.u, u, (size_t)p.cw * p.ch), memset(p.v, v, (size_t)p.cw * p.ch);
    return p;
}

static void freep(pic *p) { free(p->y), free(p->u), free(p->v); }

static gf_yuv view(const pic *p, int full, int bt709) {
    gf_yuv s = {{p->y, p->u, p->v}, {p->w, p->cw, p->cw}, p->w, p->h, full, bt709};
    return s;
}

static uint32_t convert1(int y, int u, int v, int full, int bt709) {
    pic p = make(4, 4, y, u, v);
    gf_yuv s = view(&p, full, bt709);
    uint32_t out[4];
    size_t wn = gf_yuv_work_size(4, 4, 1, 1);
    void *work = aligned_alloc(16, (wn + 15) & ~(size_t)15);
    int rc = gf_yuv_to_xrgb(&s, out, 1, 1, 1, work);
    CHECK(rc == 0, "convert returned %d", rc);
    free(work);
    freep(&p);
    return out[0];
}

static void rgb(uint32_t px, int *r, int *g, int *b) { *r = (px >> 16) & 255, *g = (px >> 8) & 255, *b = px & 255; }

static void colours(void) {
    int r, g, b;
    uint32_t px;
    px = convert1(16, 128, 128, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK((px >> 24) == 0xFF && r == 0 && g == 0 && b == 0, "limited black is %d %d %d (alpha %02x)", r, g, b, px >> 24);
    px = convert1(235, 128, 128, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 255, 1) && near_(g, 255, 1) && near_(b, 255, 1), "limited white is %d %d %d", r, g, b);
    px = convert1(126, 128, 128, 0, 0);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 128, 2) && near_(g, 128, 2) && near_(b, 128, 2), "limited mid grey is %d %d %d", r, g, b);
    px = convert1(0, 128, 128, 1, 1);
    rgb(px, &r, &g, &b);
    CHECK(r == 0 && g == 0 && b == 0, "full black is %d %d %d", r, g, b);
    px = convert1(255, 128, 128, 1, 0);
    rgb(px, &r, &g, &b);
    CHECK(r == 255 && g == 255 && b == 255, "full white is %d %d %d", r, g, b);
    /* BT.709 limited red (R 255, G 0, B 0) is Y 63, U 102, V 240; blue is Y 32, U 240, V 118; green Y 173, U 42, V 26 */
    px = convert1(63, 102, 240, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 255, 3) && near_(g, 0, 3) && near_(b, 0, 3), "709 red is %d %d %d", r, g, b);
    px = convert1(32, 240, 118, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 0, 3) && near_(g, 0, 3) && near_(b, 255, 3), "709 blue is %d %d %d", r, g, b);
    px = convert1(173, 42, 26, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 0, 3) && near_(g, 255, 3) && near_(b, 0, 4), "709 green is %d %d %d", r, g, b);
    /* BT.601 limited red: Y 82, U 90, V 240 */
    px = convert1(82, 90, 240, 0, 0);
    rgb(px, &r, &g, &b);
    CHECK(near_(r, 255, 3) && near_(g, 0, 3) && near_(b, 0, 3), "601 red is %d %d %d", r, g, b);
    /* the matrix matters: the same numbers read as 709 are not red */
    px = convert1(82, 90, 240, 0, 1);
    rgb(px, &r, &g, &b);
    CHECK(!(near_(r, 255, 3) && near_(g, 0, 3) && near_(b, 0, 3)), "601 red read as 709 should differ, got %d %d %d", r, g, b);
}

static gf_yuv scaled_run(const pic *p, int dw, int dh, uint32_t **out, int full, int bt709) {
    gf_yuv s = view(p, full, bt709);
    size_t wn = gf_yuv_work_size(p->w, p->h, dw, dh);
    void *work = aligned_alloc(16, (wn + 15) & ~(size_t)15);
    *out = malloc((size_t)dw * dh * 4);
    int rc = gf_yuv_to_xrgb(&s, *out, dw, dh, dw, work);
    CHECK(rc == 0, "scaled convert returned %d", rc);
    free(work);
    return s;
}

static void scaling(void) {
    /* a flat picture stays flat at every size, up, down and by a factor that is not a power of two */
    int sizes[][2] = {{1280, 720}, {640, 360}, {480, 270}, {100, 57}, {2000, 1500}, {64, 64}, {1, 1}};
    for (unsigned i = 0; i < sizeof sizes / sizeof *sizes; i++) {
        pic p = make(641, 363, 126, 128, 128);                      /* odd sizes on purpose */
        uint32_t *o;
        scaled_run(&p, sizes[i][0], sizes[i][1], &o, 0, 0);
        int bad = 0;
        for (int k = 0; k < sizes[i][0] * sizes[i][1]; k++) {
            int r, g, b;
            rgb(o[k], &r, &g, &b);
            if (!near_(r, 128, 2) || !near_(g, 128, 2) || !near_(b, 128, 2)) bad++;
        }
        CHECK(bad == 0, "a flat 641x363 into %dx%d has %d wrong pixels", sizes[i][0], sizes[i][1], bad);
        free(o);
        freep(&p);
    }
    /* a horizontal luma ramp stays monotone and spans the range after a halving and a bilinear pass */
    pic p = make(1280, 720, 16, 128, 128);
    for (int y = 0; y < 720; y++)
        for (int x = 0; x < 1280; x++) p.y[y * 1280 + x] = (uint8_t)(16 + (x * 219) / 1279);
    uint32_t *o;
    scaled_run(&p, 480, 270, &o, 0, 1);
    int prev = -1, back = 0, r, g, b;
    for (int x = 0; x < 480; x++) {
        rgb(o[135 * 480 + x], &r, &g, &b);
        if (g < prev - 1) back++;
        prev = g;
    }
    CHECK(back == 0, "the ramp goes backwards %d times", back);
    rgb(o[135 * 480], &r, &g, &b);
    CHECK(g < 12, "the ramp starts at %d", g);
    rgb(o[135 * 480 + 479], &r, &g, &b);
    CHECK(g > 243, "the ramp ends at %d", g);
    free(o);
    /* a 1-pixel checkerboard of 16 and 235 halves to the mean, not to either (the box filter) */
    for (int y = 0; y < 720; y++)
        for (int x = 0; x < 1280; x++) p.y[y * 1280 + x] = ((x ^ y) & 1) ? 235 : 16;
    scaled_run(&p, 640, 360, &o, 0, 1);
    rgb(o[100 * 640 + 100], &r, &g, &b);
    CHECK(near_(g, 128, 4), "a checkerboard halves to %d, not the mean", g);
    free(o);
    freep(&p);
    /* the corners are the source's corners: left top red, right bottom blue (chroma kept apart from luma) */
    pic q = make(4, 4, 126, 128, 128);
    for (int y = 0; y < 2; y++)
        for (int x = 0; x < 2; x++) q.u[y * 2 + x] = (x == 0 && y == 0) ? 90 : 128, q.v[y * 2 + x] = (x == 0 && y == 0) ? 240 : 128;
    scaled_run(&q, 4, 4, &o, 0, 0);
    rgb(o[0], &r, &g, &b);
    CHECK(r > g + 40 && r > b + 40, "the first pixel is not red: %d %d %d", r, g, b);
    rgb(o[15], &r, &g, &b);
    CHECK(near_(r, g, 6) && near_(g, b, 6), "the last pixel is not grey: %d %d %d", r, g, b);
    free(o);
    freep(&q);
}

/* The converter's output for pseudo-random pictures is fixed, to the bit: these are the checksums of the straightforward version (a row of each plane blended, then
 * sampled and converted pixel by pixel), so that a faster one is proved to give the same picture. */
static void golden(void) {
    static const struct { int sw, sh, dw, dh, full, bt709; uint32_t hash; } cfg[] = {
        {1280, 720, 480, 270, 0, 1, 0xb22e38fau}, {641, 363, 480, 270, 0, 0, 0x89491b74u}, {640, 360, 480, 270, 1, 1, 0x3bdd012eu},
        {1920, 1080, 960, 540, 0, 1, 0xe4980c20u}, {333, 211, 700, 400, 0, 1, 0xcdd0bcd8u}, {64, 48, 1, 1, 0, 0, 0x0be8ca3bu},
    };
    uint32_t rng = 12345;
#define RND() (rng = rng * 1664525u + 1013904223u, rng >> 8)
    for (unsigned c = 0; c < sizeof cfg / sizeof *cfg; c++) {
        int sw = cfg[c].sw, sh = cfg[c].sh, cw = (sw + 1) / 2, ch = (sh + 1) / 2;
        uint8_t *y = malloc((size_t)sw * sh), *u = malloc((size_t)cw * ch), *v = malloc((size_t)cw * ch);
        for (int i = 0; i < sw * sh; i++) y[i] = (uint8_t)(RND() % 220 + 16);
        for (int i = 0; i < cw * ch; i++) u[i] = (uint8_t)(RND() % 200 + 28), v[i] = (uint8_t)(RND() % 200 + 28);
        gf_yuv s = {{y, u, v}, {sw, cw, cw}, sw, sh, cfg[c].full, cfg[c].bt709};
        size_t wn = gf_yuv_work_size(sw, sh, cfg[c].dw, cfg[c].dh);
        void *work = aligned_alloc(16, (wn + 15) & ~(size_t)15);
        uint32_t *o = malloc((size_t)cfg[c].dw * cfg[c].dh * 4);
        CHECK(gf_yuv_to_xrgb(&s, o, cfg[c].dw, cfg[c].dh, cfg[c].dw, work) == 0, "golden %u converts", c);
        uint32_t h = 2166136261u;
        for (int i = 0; i < cfg[c].dw * cfg[c].dh; i++) h = (h ^ o[i]) * 16777619u;
        CHECK(h == cfg[c].hash, "golden %u (%dx%d to %dx%d): %08x, not %08x", c, sw, sh, cfg[c].dw, cfg[c].dh, h, cfg[c].hash);
        free(o), free(work), free(y), free(u), free(v);
    }
}

static void refusals(void) {
    pic p = make(8, 8, 100, 128, 128);
    gf_yuv s = view(&p, 0, 1);
    uint32_t out[16];
    char work[4096] __attribute__((aligned(16)));
    CHECK(gf_yuv_to_xrgb(&s, out, 0, 4, 4, work) == -1, "a zero width is refused");
    CHECK(gf_yuv_to_xrgb(&s, out, 4, 4, 3, work) == -1, "a stride below the width is refused");
    CHECK(gf_yuv_to_xrgb(&s, out, 9000, 4, 9000, work) == -1, "9000 wide is refused");
    CHECK(gf_yuv_to_xrgb(NULL, out, 4, 4, 4, work) == -1, "no source is refused");
    CHECK(gf_yuv_work_size(0, 4, 4, 4) == 0, "an empty source needs nothing");
    gf_yuv e = s;
    e.width = 0;
    CHECK(gf_yuv_to_xrgb(&e, out, 4, 4, 4, work) == -1, "an empty picture is refused");
    freep(&p);
}

static void speed(void) {
    struct { int sw, sh, dw, dh; } runs[] = {{1280, 720, 480, 270}, {640, 360, 480, 270}, {1280, 720, 960, 540}, {1920, 1080, 960, 540}};
    for (unsigned i = 0; i < sizeof runs / sizeof *runs; i++) {
        pic p = make(runs[i].sw, runs[i].sh, 126, 128, 128);
        for (int k = 0; k < runs[i].sw * runs[i].sh; k += 7) p.y[k] = (uint8_t)(k * 31);
        uint32_t *o;
        gf_yuv s = view(&p, 0, 1);
        size_t wn = gf_yuv_work_size(runs[i].sw, runs[i].sh, runs[i].dw, runs[i].dh);
        void *work = aligned_alloc(16, (wn + 15) & ~(size_t)15);
        o = malloc((size_t)runs[i].dw * runs[i].dh * 4);
        struct timespec a, b;
        clock_gettime(CLOCK_MONOTONIC, &a);
        const int n = 40;
        for (int k = 0; k < n; k++) gf_yuv_to_xrgb(&s, o, runs[i].dw, runs[i].dh, runs[i].dw, work);
        clock_gettime(CLOCK_MONOTONIC, &b);
        double ms = ((b.tv_sec - a.tv_sec) * 1e3 + (b.tv_nsec - a.tv_nsec) / 1e6) / n;
        printf("speed: %dx%d -> %dx%d: %.2f ms a picture on this host (work %zu KB)\n", runs[i].sw, runs[i].sh, runs[i].dw, runs[i].dh, ms, wn / 1024);
        free(o);
        free(work);
        freep(&p);
    }
}

int main(void) {
    colours();
    scaling();
    golden();
    refusals();
    speed();
    if (fails) {
        printf("%d checks failed\n", fails);
        return 1;
    }
    printf("yuv_test: ok\n");
    return 0;
}
