#define _GNU_SOURCE
#include "vpl_vpc.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define VPC_MAX_UNITS (1u << 20)

static uint32_t rd32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }

static void fail(char *err, size_t cap, const char *what, const char *path) {
    if (err && cap) snprintf(err, cap, "%s: %s", path, what);
}

static void *map_file(const char *path, size_t *len, char *err, size_t cap) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        if (err && cap) snprintf(err, cap, "%s: %s", path, strerror(errno));
        return NULL;
    }
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < 8) {
        close(fd);
        fail(err, cap, "empty or unreadable", path);
        return NULL;
    }
    void *m = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (m == MAP_FAILED) {
        if (err && cap) snprintf(err, cap, "%s: mmap: %s", path, strerror(errno));
        return NULL;
    }
    *len = (size_t)st.st_size;
    return m;
}

int vpc_open(vpc_t *c, const char *path, char *err, size_t cap) {
    memset(c, 0, sizeof *c);
    size_t len;
    uint8_t *m = map_file(path, &len, err, cap);
    if (!m) return -1;
    if (len < 40 || memcmp(m, "VPC1", 4) != 0) {
        munmap(m, len);
        fail(err, cap, "not a VPC1 file", path);
        return -1;
    }
    if (m[4] != 1 || (m[5] != VPC_H264 && m[5] != VPC_HEVC)) {
        munmap(m, len);
        fail(err, cap, "unknown version or codec", path);
        return -1;
    }
    uint32_t w = rd32(m + 8), h = rd32(m + 12), num = rd32(m + 16), den = rd32(m + 20);
    uint32_t count = rd32(m + 24), loop_from = rd32(m + 28), off = rd32(m + 32);
    if (w == 0 || h == 0 || w > 8192 || h > 8192 || num == 0 || den == 0 || count == 0 || count > VPC_MAX_UNITS || loop_from >= count ||
        off != 40 + 8u * count || off > len) {
        munmap(m, len);
        fail(err, cap, "bad header", path);
        return -1;
    }
    uint32_t *offs = malloc(sizeof(uint32_t) * count);
    if (!offs) {
        munmap(m, len);
        fail(err, cap, "out of memory", path);
        return -1;
    }
    uint64_t pos = 0;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t size = rd32(m + 40 + 8 * i);
        offs[i] = (uint32_t)pos;
        pos += size;
        if (size == 0 || pos > len - off) {
            free(offs);
            munmap(m, len);
            fail(err, cap, "a unit runs past the end of the file", path);
            return -1;
        }
    }
    if (pos != len - off || !(rd32(m + 44) & 1) || !(rd32(m + 40 + 8 * loop_from + 4) & 1)) {
        free(offs);
        munmap(m, len);
        fail(err, cap, pos != len - off ? "bytes after the last unit" : "the first unit or the loop target is not a key unit", path);
        return -1;
    }
    c->map = m;
    c->map_len = len;
    c->codec = m[5];
    c->width = w;
    c->height = h;
    c->fps_num = num;
    c->fps_den = den;
    c->count = count;
    c->loop_from = loop_from;
    c->payload = m + off;
    c->offset = offs;
    c->table = m + 40;
    return 0;
}

void vpc_close(vpc_t *c) {
    if (c->map) munmap(c->map, c->map_len);
    free(c->offset);
    memset(c, 0, sizeof *c);
}

const uint8_t *vpc_unit(const vpc_t *c, uint32_t i, uint32_t *size, int *key) {
    if (i >= c->count) return NULL;
    if (size) *size = rd32(c->table + 8 * i);
    if (key) *key = (int)(rd32(c->table + 8 * i + 4) & 1);
    return c->payload + c->offset[i];
}

int vpg_open(vpg_t *g, const char *path, char *err, size_t cap) {
    memset(g, 0, sizeof *g);
    size_t len;
    uint8_t *m = map_file(path, &len, err, cap);
    if (!m) return -1;
    if (len < 20 || memcmp(m, "VPG1", 4) != 0) {
        munmap(m, len);
        fail(err, cap, "not a VPG1 file", path);
        return -1;
    }
    uint32_t count = rd32(m + 4);
    if (count == 0 || count > 256 || len < 20 + 28u * count) {
        munmap(m, len);
        fail(err, cap, "bad glyph table", path);
        return -1;
    }
    vpg_glyph *gl = calloc(count, sizeof *gl);
    if (!gl) {
        munmap(m, len);
        fail(err, cap, "out of memory", path);
        return -1;
    }
    const uint8_t *plane = m + 20 + 28u * count;
    size_t plane_len = len - (20 + 28u * count);
    for (uint32_t i = 0; i < count; i++) {
        const uint8_t *e = m + 20 + 28u * i;
        gl[i].cp = rd32(e);
        gl[i].advance = (int32_t)rd32(e + 4);
        gl[i].left = (int32_t)rd32(e + 8);
        gl[i].top = (int32_t)rd32(e + 12);
        gl[i].w = rd32(e + 16);
        gl[i].h = rd32(e + 20);
        uint32_t o = rd32(e + 24);
        if (gl[i].w > 512 || gl[i].h > 512 || (uint64_t)o + (uint64_t)gl[i].w * gl[i].h > plane_len) {
            free(gl);
            munmap(m, len);
            fail(err, cap, "a glyph runs past the end of the file", path);
            return -1;
        }
        gl[i].alpha = plane + o;
    }
    g->map = m;
    g->map_len = len;
    g->right = rd32(m + 8);
    g->baseline = rd32(m + 12);
    g->height = rd32(m + 16);
    g->count = count;
    g->glyph = gl;
    return 0;
}

void vpg_close(vpg_t *g) {
    if (g->map) munmap(g->map, g->map_len);
    free(g->glyph);
    memset(g, 0, sizeof *g);
}

const vpg_glyph *vpg_find(const vpg_t *g, uint32_t cp) {
    for (uint32_t i = 0; i < g->count; i++)
        if (g->glyph[i].cp == cp) return &g->glyph[i];
    return NULL;
}
