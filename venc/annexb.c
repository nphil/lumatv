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
