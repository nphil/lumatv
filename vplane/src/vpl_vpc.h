/* The clip container the player reads (tools/vplane_clips.py writes it; its docstring has the byte layout) and the glyph atlas of the
 * corner clock. A clip is mapped, checked once, and then every access unit is a pointer and a length: nothing is parsed while it plays.
 */
#ifndef VPL_VPC_H
#define VPL_VPC_H

#include <stddef.h>
#include <stdint.h>

#define VPC_H264 1
#define VPC_HEVC 2

typedef struct {
    void *map;           /* the whole file */
    size_t map_len;
    int codec;           /* VPC_H264 or VPC_HEVC: NDL's NDL_VIDEO_TYPE */
    uint32_t width, height, fps_num, fps_den;
    uint32_t count;      /* access units */
    uint32_t loop_from;  /* the key unit playback returns to */
    const uint8_t *payload;
    uint32_t *offset;    /* of every unit, from `payload` (malloc'd) */
    const uint8_t *table;/* size and flags of every unit, as the file has them */
} vpc_t;

/* 0 and a filled `c`, or -1 and a reason in `err`. Checked: magic, version, codec, the table fits the file, the units fill the
 * file to its last byte, at most 1M units, the first unit and the loop target are key units. */
int vpc_open(vpc_t *c, const char *path, char *err, size_t cap);
void vpc_close(vpc_t *c);
const uint8_t *vpc_unit(const vpc_t *c, uint32_t i, uint32_t *size, int *key);

/* The corner clock's digits. */
typedef struct {
    uint32_t cp;
    int32_t advance, left, top;
    uint32_t w, h;
    const uint8_t *alpha;  /* w x h */
} vpg_glyph;

typedef struct {
    void *map;
    size_t map_len;
    uint32_t right, baseline, height;   /* screen pixels: the clock's right edge, its baseline, its line */
    uint32_t count;
    vpg_glyph *glyph;                    /* malloc'd */
} vpg_t;

int vpg_open(vpg_t *g, const char *path, char *err, size_t cap);
void vpg_close(vpg_t *g);
const vpg_glyph *vpg_find(const vpg_t *g, uint32_t cp);

/* A print's caption: coverage of its words as bitmap, placed on the screen where the QML puts the text; `split` columns of it are the artist's
 * name (drawn in the first ink), the rest is the other ink's. */
typedef struct {
    void *map;
    size_t map_len;
    uint32_t w, h, split;
    int32_t x, y;               /* the bitmap's top left, in screen pixels */
    const uint8_t *alpha;       /* w x h */
} vpt_t;

int vpt_open(vpt_t *t, const char *path, char *err, size_t cap);
void vpt_close(vpt_t *t);

#endif
