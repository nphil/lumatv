#include "vpl_json.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static const char *skip_ws(const char *p, const char *end) {
    while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')) p++;
    return p;
}

/* p is on the opening quote; returns the position after the closing one, or NULL. */
static const char *skip_string(const char *p, const char *end) {
    p++;
    while (p < end) {
        if (*p == '\\') {
            p += 2;
            continue;
        }
        if (*p == '"') return p + 1;
        p++;
    }
    return NULL;
}

/* Skips one value of any kind; nested objects and arrays are walked, not parsed. */
static const char *skip_value(const char *p, const char *end) {
    p = skip_ws(p, end);
    if (p >= end) return NULL;
    if (*p == '"') return skip_string(p, end);
    if (*p == '{' || *p == '[') {
        int depth = 0;
        while (p < end) {
            if (*p == '"') {
                p = skip_string(p, end);
                if (!p) return NULL;
                continue;
            }
            if (*p == '{' || *p == '[') depth++;
            if (*p == '}' || *p == ']') {
                depth--;
                if (depth == 0) return p + 1;
            }
            p++;
        }
        return NULL;
    }
    while (p < end && *p != ',' && *p != '}' && *p != ']' && *p != ' ' && *p != '\t' && *p != '\n' && *p != '\r') p++;
    return p;
}

int vj_valid(const char *json, size_t len) {
    const char *end = json + len;
    const char *p = skip_ws(json, end);
    if (p >= end || *p != '{') return 0;
    p = skip_value(p, end);
    if (!p) return 0;
    return skip_ws(p, end) == end;
}

/* The raw text of the value of a top-level key: [*v, *v + *n). */
static int find(const char *json, size_t len, const char *key, const char **v, size_t *n) {
    const char *end = json + len;
    const char *p = skip_ws(json, end);
    size_t klen = strlen(key);
    if (p >= end || *p != '{') return 0;
    p++;
    for (;;) {
        p = skip_ws(p, end);
        if (p >= end || *p == '}') return 0;
        if (*p == ',') {
            p++;
            continue;
        }
        if (*p != '"') return 0;
        const char *k = p + 1;
        const char *after = skip_string(p, end);
        if (!after) return 0;
        size_t kl = (size_t)(after - 1 - k);
        p = skip_ws(after, end);
        if (p >= end || *p != ':') return 0;
        p = skip_ws(p + 1, end);
        const char *vend = skip_value(p, end);
        if (!vend) return 0;
        if (kl == klen && memcmp(k, key, klen) == 0) {
            *v = p;
            *n = (size_t)(vend - p);
            return 1;
        }
        p = vend;
    }
}

double vj_num(const char *json, size_t len, const char *key, double def) {
    const char *v;
    size_t n;
    if (!find(json, len, key, &v, &n) || n == 0 || n > 40) return def;
    if (*v == '"' || *v == '{' || *v == '[' || *v == 't' || *v == 'f' || *v == 'n') return def;
    char buf[48];
    memcpy(buf, v, n);
    buf[n] = 0;
    char *e;
    double d = strtod(buf, &e);
    if (e == buf || *e != 0 || !isfinite(d)) return def;
    return d;
}

int64_t vj_int(const char *json, size_t len, const char *key, int64_t def) {
    double d = vj_num(json, len, key, NAN);
    if (!isfinite(d) || d > 9.0e15 || d < -9.0e15) return def;
    return (int64_t)llround(d);
}

int vj_bool(const char *json, size_t len, const char *key, int def) {
    const char *v;
    size_t n;
    if (!find(json, len, key, &v, &n)) return def;
    if (n == 4 && memcmp(v, "true", 4) == 0) return 1;
    if (n == 5 && memcmp(v, "false", 5) == 0) return 0;
    return def;
}

static int put_utf8(char *out, size_t cap, size_t *o, unsigned cp) {
    char b[4];
    int k;
    if (cp < 0x80) {
        b[0] = (char)cp;
        k = 1;
    } else if (cp < 0x800) {
        b[0] = (char)(0xC0 | (cp >> 6));
        b[1] = (char)(0x80 | (cp & 0x3F));
        k = 2;
    } else {
        b[0] = (char)(0xE0 | (cp >> 12));
        b[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
        b[2] = (char)(0x80 | (cp & 0x3F));
        k = 3;
    }
    if (*o + (size_t)k >= cap) return 0;
    memcpy(out + *o, b, (size_t)k);
    *o += (size_t)k;
    return 1;
}

int vj_str(const char *json, size_t len, const char *key, char *out, size_t cap) {
    const char *v;
    size_t n;
    if (cap == 0 || !find(json, len, key, &v, &n) || n < 2 || *v != '"') return -1;
    size_t o = 0;
    for (const char *p = v + 1; p < v + n - 1; p++) {
        unsigned c = (unsigned char)*p;
        if (c == '\\' && p + 1 < v + n - 1) {
            p++;
            switch (*p) {
            case 'n': c = '\n'; break;
            case 't': c = '\t'; break;
            case 'r': c = '\r'; break;
            case 'b': c = '\b'; break;
            case 'f': c = '\f'; break;
            case 'u':
                if (p + 4 < v + n - 1) {
                    char h[5] = {p[1], p[2], p[3], p[4], 0};
                    c = (unsigned)strtoul(h, NULL, 16);
                    p += 4;
                    if (!put_utf8(out, cap, &o, c)) goto done;
                    continue;
                }
                break;
            default: c = (unsigned char)*p; break;
            }
        }
        if (o + 1 >= cap) break;
        out[o++] = (char)c;
    }
done:
    out[o] = 0;
    return (int)o;
}
