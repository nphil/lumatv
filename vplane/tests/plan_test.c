/* Unit tests of the player's pure parts: the saver's arithmetic (vpl_plan.c), the request reader (vpl_json.c) and the window's drawing
 * (vpl_ui.c). The expected numbers are what the QML's own formulas give (panels/Screensaver.qml), computed in Python with JavaScript's
 * rounding. Exit status 0 when every check passed.
 */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../src/vpl_json.h"
#include "../src/vpl_plan.h"
#include "../src/vpl_ui.h"
#include "../src/vpl_vpc.h"

static int failures;
#define CHECK(cond, ...) do { if (!(cond)) { failures++; printf("FAIL %s:%d: ", __FILE__, __LINE__); printf(__VA_ARGS__); printf("\n"); } } while (0)

static void drift(void) {
    struct { double t; int dx, dy; } want[] = {{0, 0, 0}, {4, 1, 1}, {135, 30, 16}, {270, 0, -19}, {405, -30, 8}, {540, 0, 10}, {1000.5, -24, -15}, {123456.789, -21, -13}};
    for (size_t i = 0; i < sizeof want / sizeof *want; i++) {
        int dx, dy;
        vpl_drift(want[i].t, 30, 20, 540, 380, &dx, &dy);
        CHECK(dx == want[i].dx && dy == want[i].dy, "drift(%g) = %d %d, want %d %d", want[i].t, dx, dy, want[i].dx, want[i].dy);
    }
    int dx, dy;
    vpl_drift(100, 0, 0, 540, 380, &dx, &dy);
    CHECK(dx == 0 && dy == 0, "no reach, no drift");
}

static void level_and_veil(void) {
    CHECK(fabs(vpl_level(0, 1, 0.6, 60) - 1.0) < 1e-9, "level at 0");
    CHECK(fabs(vpl_level(30, 1, 0.6, 60) - 0.8) < 1e-9, "level at 30 min");
    CHECK(fabs(vpl_level(60, 1, 0.6, 60) - 0.6) < 1e-9, "level at 60 min");
    CHECK(fabs(vpl_level(600, 1, 0.6, 60) - 0.6) < 1e-9, "level stays at the floor");
    CHECK(fabs(vpl_level(45, 0, 0.6, 60) - 1.0) < 1e-9, "no dim, no change");
    CHECK(fabs(vpl_level(-5, 1, 0.6, 60) - 1.0) < 1e-9, "a clock that went back");
    CHECK(fabs(vpl_ease(0.25) - 0.125) < 1e-9 && fabs(vpl_ease(0.75) - 0.875) < 1e-9 && vpl_ease(0.5) == 0.5, "InOutQuad");
    CHECK(vpl_veil(1, 0, 0, 700) == 0, "no veil at full level");
    CHECK(vpl_veil(0.8, 0, 0, 700) == 51, "a veil of 0.2 is 51");
    CHECK(vpl_veil(1, 1, 350, 700) == 128, "half way out is half black");
    CHECK(vpl_veil(0.7, 1, 175, 700) == 99, "out, a quarter in, over a dimmed picture");
    CHECK(vpl_veil(0.7, 2, 0, 700) == 255, "held black");
    CHECK(vpl_veil(0.7, 3, 525, 700) == 99, "in, three quarters through");
    CHECK(vpl_veil(1, 3, 700, 700) == 0, "in, finished");
}

static void order(void) {
    vpl_order o;
    vpl_rng r;
    vpl_order_init(&o, 4);
    vpl_rng_seed(&r, 12345);
    int seq[4];
    for (int i = 0; i < 4; i++) {
        seq[i] = vpl_order_next(&o, 0, &r);
        o.current = seq[i];
    }
    CHECK(seq[0] == 0 && seq[1] == 1 && seq[2] == 2 && seq[3] == 3, "in sequence from the start");
    CHECK(vpl_order_next(&o, 0, &r) == 0, "and around");
    int seen[6][8] = {{0}}, prev = -1, repeats = 0;
    vpl_order_init(&o, 6);
    for (int bag = 0; bag < 8; bag++) {
        for (int i = 0; i < 6; i++) {
            int v = vpl_order_next(&o, 1, &r);
            CHECK(v >= 0 && v < 6, "an index in range: %d", v);
            if (v < 0 || v > 5) return;
            seen[v][bag]++;
            if (v == prev) repeats++;
            prev = v;
            o.current = v;
        }
        for (int v = 0; v < 6; v++) CHECK(seen[v][bag] == 1, "bag %d holds print %d once, not %d times", bag, v, seen[v][bag]);
    }
    CHECK(repeats == 0, "a print never follows itself (%d times)", repeats);
    vpl_order_init(&o, 1);
    CHECK(vpl_order_next(&o, 1, &r) == 0 && vpl_order_next(&o, 0, &r) == 0, "one print is the only choice");
    vpl_order_init(&o, 0);
    CHECK(vpl_order_next(&o, 1, &r) == -1, "no prints, no index");
    vpl_rng a, b;
    vpl_rng_seed(&a, 77);
    vpl_rng_seed(&b, 77);
    CHECK(vpl_rng_next(&a) == vpl_rng_next(&b) && vpl_rng_next(&a) == vpl_rng_next(&b), "the same seed gives the same order");
}

static void clock_text(void) {
    char t[16];
    vpl_clock_text(t, sizeof t, 5, 7, 1);
    CHECK(!strcmp(t, "5:07"), "twelve hour, no leading zero: %s", t);
    vpl_clock_text(t, sizeof t, 17, 5, 1);
    CHECK(!strcmp(t, "5:05"), "the afternoon folds to 12 hours: %s", t);
    vpl_clock_text(t, sizeof t, 5, 7, 0);
    CHECK(!strcmp(t, "05:07"), "HH:mm: %s", t);
    vpl_clock_text(t, sizeof t, 17, 5, 0);
    CHECK(!strcmp(t, "17:05"), "24 hours: %s", t);
    vpl_clock_text(t, sizeof t, 0, 0, 1);
    CHECK(!strcmp(t, "12:00"), "midnight is 12: %s", t);
    vpl_clock_text(t, sizeof t, 12, 30, 1);
    CHECK(!strcmp(t, "12:30"), "noon is 12: %s", t);
    vpl_clock_text(t, sizeof t, 23, 39, 1);
    CHECK(!strcmp(t, "11:39"), "%s", t);
}

static void json(void) {
    const char *j = " {\"seq\": 12, \"op\":\"start\", \"nested\":{\"seq\":99,\"a\":[1,2,{\"x\":\"}\"}]}, \"ratio\":0.6,\"on\":true,\"off\":false,\"e\":3e2,\"s\":\"a\\\"b\\\\c\\u00e9\\n\"} ";
    size_t n = strlen(j);
    CHECK(vj_valid(j, n), "valid");
    CHECK(vj_int(j, n, "seq", -1) == 12, "a top-level key is not confused with a nested one");
    CHECK(fabs(vj_num(j, n, "ratio", 0) - 0.6) < 1e-12, "number");
    CHECK(vj_int(j, n, "e", 0) == 300, "an exponent reads as an integer");
    CHECK(vj_bool(j, n, "on", 0) == 1 && vj_bool(j, n, "off", 1) == 0, "booleans");
    CHECK(vj_bool(j, n, "missing", 7) == 7 && vj_int(j, n, "missing", 8) == 8, "defaults for a missing key");
    CHECK(vj_int(j, n, "op", 5) == 5, "a string is not a number");
    char s[32];
    CHECK(vj_str(j, n, "op", s, sizeof s) == 5 && !strcmp(s, "start"), "string");
    int k = vj_str(j, n, "s", s, sizeof s);
    CHECK(k == 8 && !strcmp(s, "a\"b\\c\xc3\xa9\n"), "escapes decoded: %d '%s'", k, s);
    CHECK(vj_str(j, n, "seq", s, sizeof s) == -1, "a number is not a string");
    CHECK(vj_str(j, n, "op", s, 3) == 2 && !strcmp(s, "st"), "cut to the buffer");
    CHECK(!vj_valid("{\"a\":1", 6) && !vj_valid("[1]", 3) && !vj_valid("", 0) && !vj_valid("{} x", 4), "invalid texts");
    CHECK(vj_int("{\"a\": 1.5}", 10, "a", 0) == 2, "1.5 rounds to 2");
    CHECK(vj_int("{\"a\": \"7\"}", 10, "a", 0) == 0, "a string of digits is not a number");
    CHECK(vj_int("{\"seq\":1730000000123}", 21, "seq", 0) == 1730000000123LL, "a millisecond epoch fits");
}

static void ui(void) {
    enum { W = 64, H = 32 };
    static uint32_t px[W * H];
    vui_canvas c = {px, W, H, W};
    /* a font of one glyph, '1': a 2 x 3 block of full coverage, advance 4, three above the baseline */
    static const uint8_t cover[6] = {255, 255, 255, 255, 128, 0};
    vpg_glyph gl = {'1', 4, 1, 3, 2, 3, cover};
    vpg_t g = {.right = 60, .baseline = 20, .height = 8, .count = 1, .glyph = &gl};
    vui_fill(&c, (vui_rect){0, 0, W, H}, 100);
    CHECK(px[0] == 100u << 24 && px[W * H - 1] == 100u << 24, "a veil of 100");
    vui_fill(&c, (vui_rect){0, 0, W, H}, 0);
    CHECK(px[5] == 0, "no veil is a transparent buffer");
    vui_rect r = vui_text_rect(&g, "11", 40, 20);
    CHECK(r.x0 == 40 - 8 - 2 && r.x1 == 42 && r.y0 == 20 - 3 - 1 && r.y1 == 20 + 0 + 1, "the box of '11': %d %d %d %d", r.x0, r.y0, r.x1, r.y1);
    CHECK(vui_empty(vui_text_rect(&g, "1x", 40, 20)), "a character with no glyph gives an empty box");
    vui_draw_text(&c, &g, "11", 40, 20, 0xFF8000, 255, 0);
    /* the second '1' starts at 40 - 4 = 36; its glyph left is 1, so x = 37, rows 17..19 */
    uint32_t p = px[17 * W + 37];
    CHECK(p == 0xFFFF8000u, "full coverage over no veil is the colour opaque: %08x", p);
    uint32_t half = px[19 * W + 37];
    CHECK((half >> 24) == 128 && ((half >> 16) & 0xFF) == 128, "half coverage is premultiplied: %08x", half);
    CHECK(px[19 * W + 38] == 0, "no coverage leaves the veil: %08x", px[19 * W + 38]);
    vui_fill(&c, (vui_rect){0, 0, W, H}, 200);
    vui_draw_text(&c, &g, "1", 40, 20, 0xFFFFFF, 255, 200);
    CHECK(px[17 * W + 37] == 0xFFFFFFFFu, "white over a veil is white: %08x", px[17 * W + 37]);
    CHECK(px[19 * W + 38] == 200u << 24, "outside the glyph the veil stays: %08x", px[19 * W + 38]);
    vui_rect clip = vui_clip((vui_rect){-5, -5, 500, 500}, &c);
    CHECK(clip.x0 == 0 && clip.y0 == 0 && clip.x1 == W && clip.y1 == H, "clipped to the canvas");
    vui_rect u = vui_union((vui_rect){0, 0, 0, 0}, (vui_rect){3, 4, 5, 6});
    CHECK(u.x0 == 3 && u.y1 == 6, "the union with nothing is the other");
}

int main(void) {
    drift();
    level_and_veil();
    order();
    clock_text();
    json();
    ui();
    if (failures) {
        printf("%d failed\n", failures);
        return 1;
    }
    printf("plan_test: all passed\n");
    return 0;
}
