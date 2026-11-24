#include "annexb.h"

#include <string.h>

static size_t find_start(const uint8_t *buf, size_t len, size_t from)
{
    for (size_t i = from; i + 3 <= len; i++)
        if (buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1)
            return i;
    return len;
}

int annexb_next(const uint8_t *buf, size_t len, size_t *pos, struct annexb_nal *nal)
{
    size_t sc = find_start(buf, len, *pos);
    if (sc >= len || sc + 3 >= len)
        return 0;
    size_t begin = sc + 3;
    size_t next = find_start(buf, len, begin);
    size_t end = next;
    /* a 4-byte start code's leading zero, and any zero padding, belongs to no NAL */
    while (end > begin && buf[end - 1] == 0)
        end--;
    nal->p = buf + begin;
    nal->len = end - begin;
    nal->type = nal->len ? (buf[begin] & 0x1f) : 0;
    *pos = next;
    return 1;
}

void annexb_summarize(const uint8_t *buf, size_t len, struct annexb_summary *s)
{
    memset(s, 0, sizeof(*s));
    s->profile_idc = s->level_idc = -1;
    size_t pos = 0;
    struct annexb_nal n;
    while (annexb_next(buf, len, &pos, &n)) {
        s->nals++;
        switch (n.type) {
        case 7:
            s->sps++;
            if (s->profile_idc < 0 && n.len >= 4) {
                s->profile_idc = n.p[1];
                s->constraint_set = n.p[2];
                s->level_idc = n.p[3];
            }
            break;
        case 8: s->pps++; break;
        case 5: s->idr++; break;
        case 1: s->slices++; break;
        case 6: s->sei++; break;
        case 9: s->aud++; break;
        default: s->other++; break;
        }
    }
}

int annexb_is_keyframe_unit(const uint8_t *buf, size_t len)
{
    size_t pos = 0;
    struct annexb_nal n;
    int sps = 0, pps = 0;
    while (annexb_next(buf, len, &pos, &n)) {
        if (n.type == 7)
            sps = 1;
        else if (n.type == 8)
            pps = 1;
        else if (n.type == 5)
            return sps && pps;
        else if (n.type == 1)
            return 0;
    }
    return 0;
}

/* ---- the pixel aspect ratio in the SPS ---- */

#define SPS_MAX 128 /* bytes of RBSP; the encoder's is about 15 */

struct bits {
    const uint8_t *p;
    size_t nbits, pos;
    int bad; /* read past the end, or a code that is not one */
};

static unsigned get_bit(struct bits *b)
{
    if (b->pos >= b->nbits) {
        b->bad = 1;
        return 0;
    }
    unsigned v = (b->p[b->pos >> 3] >> (7 - (b->pos & 7))) & 1;
    b->pos++;
    return v;
}

static unsigned get_bits(struct bits *b, unsigned n)
{
    unsigned v = 0;
    while (n--)
        v = (v << 1) | get_bit(b);
    return v;
}

/* Exp-Golomb ue(v); the signed se(v) has the same shape, so skipping one is reading this */
static unsigned get_ue(struct bits *b)
{
    unsigned zeros = 0;
    while (!get_bit(b)) {
        if (b->bad || ++zeros > 31) {
            b->bad = 1;
            return 0;
        }
    }
    return (1u << zeros) - 1 + get_bits(b, zeros);
}

struct bitw {
    uint8_t *p; /* zeroed */
    size_t cap, pos; /* bytes, bits */
    int bad;
};

static void put_bits(struct bitw *w, unsigned v, unsigned n)
{
    while (n--) {
        if ((w->pos >> 3) >= w->cap) {
            w->bad = 1;
            return;
        }
        if ((v >> n) & 1)
            w->p[w->pos >> 3] |= (uint8_t)(0x80 >> (w->pos & 7));
        w->pos++;
    }
}

/* aspect_ratio_idc of the ratios H.264 names (Table E-1, 1 to 16); any other goes as 255 with both numbers */
static unsigned aspect_idc(unsigned w, unsigned h)
{
    static const unsigned named[][2] = { { 1, 1 }, { 12, 11 }, { 10, 11 }, { 16, 11 }, { 40, 33 }, { 24, 11 }, { 20, 11 }, { 32, 11 },
                                         { 80, 33 }, { 18, 11 }, { 15, 11 }, { 64, 33 }, { 160, 99 }, { 4, 3 }, { 3, 2 }, { 2, 1 } };
    for (unsigned i = 0; i < sizeof(named) / sizeof(named[0]); i++)
        if (named[i][0] * h == named[i][1] * w)
            return i + 1;
    return 255;
}

size_t annexb_set_sar(uint8_t *au, size_t len, size_t cap, unsigned sar_w, unsigned sar_h)
{
    if (!sar_w || !sar_h || sar_w > 0xffff || sar_h > 0xffff)
        return 0;
    unsigned gx = sar_w, gy = sar_h;
    while (gy) {
        unsigned t = gx % gy;
        gx = gy;
        gy = t;
    }
    sar_w /= gx;
    sar_h /= gx;

    size_t pos = 0;
    struct annexb_nal nal;
    int found = 0;
    while (annexb_next(au, len, &pos, &nal)) {
        if (nal.type == 7) {
            found = 1;
            break;
        }
        if (nal.type != 6 && nal.type != 8 && nal.type != 9)
            break;
    }
    if (!found || nal.len < 5)
        return 0;

    /* the RBSP: the NAL header gone, the emulation prevention bytes (00 00 03) taken out */
    uint8_t rbsp[SPS_MAX];
    size_t n = 0;
    unsigned zeros = 0;
    for (size_t i = 1; i < nal.len; i++) {
        uint8_t c = nal.p[i];
        if (zeros >= 2 && c == 3) {
            zeros = 0;
            continue;
        }
        if (n == sizeof(rbsp))
            return 0;
        rbsp[n++] = c;
        zeros = c ? 0 : zeros + 1;
    }

    /* 7.3.2.1.1, as far as the VUI flag; an SPS with scaling lists or a VUI is not the encoder's own, and left alone */
    struct bits b = { rbsp, n * 8, 0, 0 };
    unsigned profile = get_bits(&b, 8);
    get_bits(&b, 16); /* constraint flags, level_idc */
    get_ue(&b);       /* seq_parameter_set_id */
    switch (profile) {
    case 100: case 110: case 122: case 244: case 44: case 83: case 86: case 118: case 128: case 138: case 139: case 134: case 135:
        if (get_ue(&b) == 3) /* chroma_format_idc */
            get_bit(&b);     /* separate_colour_plane_flag */
        get_ue(&b);          /* bit_depth_luma_minus8 */
        get_ue(&b);          /* bit_depth_chroma_minus8 */
        get_bit(&b);         /* qpprime_y_zero_transform_bypass_flag */
        if (get_bit(&b))     /* seq_scaling_matrix_present_flag */
            return 0;
        break;
    default:
        break;
    }
    get_ue(&b); /* log2_max_frame_num_minus4 */
    unsigned poc = get_ue(&b);
    if (poc == 0) {
        get_ue(&b); /* log2_max_pic_order_cnt_lsb_minus4 */
    } else if (poc == 1) {
        get_bit(&b); /* delta_pic_order_always_zero_flag */
        get_ue(&b);  /* offset_for_non_ref_pic */
        get_ue(&b);  /* offset_for_top_to_bottom_field */
        unsigned cycle = get_ue(&b);
        if (cycle > 255)
            return 0;
        while (cycle--)
            get_ue(&b); /* offset_for_ref_frame */
    }
    get_ue(&b);  /* max_num_ref_frames */
    get_bit(&b); /* gaps_in_frame_num_value_allowed_flag */
    get_ue(&b);  /* pic_width_in_mbs_minus1 */
    get_ue(&b);  /* pic_height_in_map_units_minus1 */
    if (!get_bit(&b)) /* frame_mbs_only_flag */
        get_bit(&b);  /* mb_adaptive_frame_field_flag */
    get_bit(&b);      /* direct_8x8_inference_flag */
    if (get_bit(&b))  /* frame_cropping_flag */
        for (int i = 0; i < 4; i++)
            get_ue(&b);
    size_t vui_at = b.pos;
    /* The set's encoder writes a VUI of its own (timing_info and bitstream_restriction, no aspect ratio): its fields after the aspect flag are carried
     * over bit for bit up to the stop bit. Without a VUI, the flags that follow the ratio are all written as absent. */
    size_t keep_from = 0, keep_to = 0;
    if (get_bit(&b)) { /* vui_parameters_present_flag */
        if (get_bit(&b)) /* aspect_ratio_info_present_flag: the ratio is the encoder's, not ours to replace */
            return 0;
        keep_from = b.pos;
        keep_to = n * 8;
        while (keep_to > keep_from && !((rbsp[(keep_to - 1) >> 3] >> (7 - ((keep_to - 1) & 7))) & 1))
            keep_to--;
        if (keep_to <= keep_from + 8 || b.bad) /* a VUI is at least its eight flags */
            return 0;
        keep_to--; /* the stop bit */
    } else {
        /* nothing but the stop bit and its padding may follow, or this was not read as it was written */
        if (!get_bit(&b))
            return 0;
        while (b.pos < b.nbits)
            if (get_bit(&b))
                return 0;
        if (b.bad)
            return 0;
    }

    uint8_t out[SPS_MAX + 16] = { 0 };
    struct bitw w = { out, sizeof(out), 0, 0 };
    for (size_t i = 0; i < vui_at; i++)
        put_bits(&w, (rbsp[i >> 3] >> (7 - (i & 7))) & 1, 1);
    put_bits(&w, 1, 1); /* vui_parameters_present_flag */
    put_bits(&w, 1, 1); /* aspect_ratio_info_present_flag */
    unsigned idc = aspect_idc(sar_w, sar_h);
    put_bits(&w, idc, 8);
    if (idc == 255) {
        put_bits(&w, sar_w, 16);
        put_bits(&w, sar_h, 16);
    }
    if (keep_to > keep_from)
        for (size_t i = keep_from; i < keep_to; i++)
            put_bits(&w, (rbsp[i >> 3] >> (7 - (i & 7))) & 1, 1);
    else
        /* overscan_info, video_signal_type, chroma_loc_info, timing_info, nal_hrd, vcl_hrd, pic_struct and bitstream_restriction: none */
        put_bits(&w, 0, 8);
    put_bits(&w, 1, 1); /* rbsp_stop_one_bit */
    while (w.pos & 7)
        put_bits(&w, 0, 1);
    if (w.bad)
        return 0;

    uint8_t nal_out[1 + 2 * sizeof(out)];
    size_t m = 0;
    nal_out[m++] = nal.p[0];
    zeros = 0;
    for (size_t i = 0; i < w.pos >> 3; i++) {
        if (zeros >= 2 && out[i] <= 3) {
            nal_out[m++] = 3;
            zeros = 0;
        }
        nal_out[m++] = out[i];
        zeros = out[i] ? 0 : zeros + 1;
    }

    size_t sps_at = (size_t)(nal.p - au), tail = sps_at + nal.len;
    size_t grown = sps_at + m + (len - tail);
    if (grown > cap)
        return 0;
    memmove(au + sps_at + m, au + tail, len - tail);
    memcpy(au + sps_at, nal_out, m);
    return grown;
}
