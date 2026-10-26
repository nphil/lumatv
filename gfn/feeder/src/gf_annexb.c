#include "gf_annexb.h"

#include <string.h>

int gf_nal_type(const uint8_t *p, int codec) { return codec == GF_CODEC_H265 ? (p[0] >> 1) & 0x3F : p[0] & 0x1F; }

int gf_first_slice(const uint8_t *p, size_t left, int codec) {
    int t = gf_nal_type(p, codec);
    if (codec == GF_CODEC_H265) {
        if (t >= 32 || left < 3) return 0;          /* not VCL */
        return (p[2] & 0x80) != 0;
    }
    if (t < 1 || t > 5 || left < 2) return 0;
    return (p[1] & 0x80) != 0;                      /* first_mb_in_slice ue(v) = 0 */
}

static int is_key_type(int t, int codec) { return codec == GF_CODEC_H265 ? (t >= 16 && t <= 21) : t == 5; }
static int is_vcl(int t, int codec) { return codec == GF_CODEC_H265 ? t < 32 : (t >= 1 && t <= 5); }
static int is_sps(int t, int codec) { return codec == GF_CODEC_H265 ? (t == 32 || t == 33 || t == 34) : (t == 7 || t == 8); }
/* Non-VCL units that may only begin an access unit (or sit before its first slice). */
static int opens_au(int t, int codec) {
    if (codec == GF_CODEC_H265) return t == 35 || t == 32 || t == 33 || t == 34 || t == 39 || (t >= 41 && t <= 44) || (t >= 48 && t <= 55);
    return t == 9 || t == 7 || t == 8 || t == 6 || (t >= 14 && t <= 18);
}

static size_t find_start(const uint8_t *d, size_t n, size_t from) {
    for (size_t i = from; i + 2 < n; i++)
        if (d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1) return i;
    return n;
}

static size_t push(gf_unit *out, size_t cap, size_t count, size_t off, size_t end, int key, int sps) {
    if (count < cap) {
        out[count].offset = off;
        out[count].size = end - off;
        out[count].key = key;
        out[count].has_sps = sps;
    }
    return count + 1;
}

static size_t split_nal(const uint8_t *d, size_t n, int codec, gf_unit *out, size_t cap) {
    size_t count = 0, au_start = 0;
    int au_vcl = 0, au_key = 0, au_sps = 0, have_au = 0;
    size_t sc = find_start(d, n, 0);
    while (sc < n) {
        size_t hdr = sc + 3;
        size_t code_at = (sc > 0 && d[sc - 1] == 0) ? sc - 1 : sc;     /* a 4 byte start code belongs to its unit */
        size_t next = find_start(d, n, hdr);
        if (hdr >= n) break;
        int t = gf_nal_type(d + hdr, codec);
        int starts = 0;
        if (have_au) {
            if (opens_au(t, codec)) starts = au_vcl;                                      /* a header after a slice: the next picture */
            else if (is_vcl(t, codec)) starts = au_vcl && gf_first_slice(d + hdr, n - hdr, codec);
        }
        if (starts) {
            count = push(out, cap, count, au_start, code_at, au_key, au_sps);
            au_start = code_at;
            au_vcl = au_key = au_sps = 0;
        }
        if (!have_au) {
            au_start = code_at;
            have_au = 1;
        }
        if (is_vcl(t, codec)) au_vcl = 1;
        if (is_key_type(t, codec)) au_key = 1;
        if (is_sps(t, codec)) au_sps = 1;
        sc = next;
    }
    if (have_au) count = push(out, cap, count, au_start, n, au_key, au_sps);
    return count;
}

/* ---- AV1 ------------------------------------------------------------------------------------------------------------------ */

static int leb128(const uint8_t *p, size_t left, uint64_t *v, size_t *len) {
    *v = 0;
    for (size_t i = 0; i < 8 && i < left; i++) {
        *v |= (uint64_t)(p[i] & 0x7F) << (7 * i);
        if (!(p[i] & 0x80)) {
            *len = i + 1;
            return 1;
        }
    }
    return 0;
}

static size_t split_obu(const uint8_t *d, size_t n, gf_unit *out, size_t cap) {
    size_t count = 0, pos = 0, tu_start = 0;
    int have = 0, key = 0, sps = 0;
    while (pos < n) {
        int type = (d[pos] >> 3) & 0xF, ext = (d[pos] >> 2) & 1, has_size = (d[pos] >> 1) & 1;
        size_t hlen = 1 + (size_t)ext;
        uint64_t size = 0;
        size_t slen = 0;
        if (has_size) {
            if (!leb128(d + pos + hlen, n - pos - hlen, &size, &slen)) break;
        } else size = n - pos - hlen;
        if (type == 2 && have) {                    /* temporal delimiter: the next temporal unit */
            count = push(out, cap, count, tu_start, pos, key, sps);
            tu_start = pos;
            key = sps = 0;
        }
        if (!have) {
            tu_start = pos;
            have = 1;
        }
        if (type == 1) sps = 1;
        if ((type == 6 || type == 3) && size > 0) {
            const uint8_t *fh = d + pos + hlen + slen;
            if (!(fh[0] & 0x80) && ((fh[0] >> 5) & 3) == 0) key = 1;      /* show_existing_frame 0, frame_type KEY_FRAME */
        }
        pos += hlen + slen + (size_t)size;
    }
    if (have) count = push(out, cap, count, tu_start, n, key, sps);
    return count;
}

size_t gf_split(const uint8_t *data, size_t n, int codec, gf_unit *out, size_t cap) {
    if (codec == GF_CODEC_AV1) return split_obu(data, n, out, cap);
    return split_nal(data, n, codec, out, cap);
}
