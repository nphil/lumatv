/* A small H.264 Annex B reader: walks NAL units (start codes 00 00 01 / 00 00 00 01, leading and
 * trailing zero bytes tolerated, which the encoder's zero-padded parameter-set block relies on), counts
 * them by type and reads profile/level out of the SPS. Used by the probe to check what came back and by
 * the tests. */
#ifndef LUMATV_ANNEXB_H
#define LUMATV_ANNEXB_H

#include <stddef.h>
#include <stdint.h>

struct annexb_nal {
    const uint8_t *p; /* first byte after the start code (the NAL header) */
    size_t len;       /* bytes to the next start code, trailing zeros stripped */
    int type;         /* nal_unit_type, 1..23 */
};

/* Finds the next NAL at or after *pos; returns 1 and advances *pos past it, 0 at the end. */
int annexb_next(const uint8_t *buf, size_t len, size_t *pos, struct annexb_nal *nal);

struct annexb_summary {
    unsigned nals, sps, pps, idr, slices, sei, aud, other;
    int profile_idc, level_idc; /* from the first SPS, -1 if none */
    int constraint_set; /* constraint_set0..5 flags byte */
};

void annexb_summarize(const uint8_t *buf, size_t len, struct annexb_summary *s);

/* True when the access unit starts with the parameter sets a decoder needs (SPS and PPS before the first
 * slice). */
int annexb_is_keyframe_unit(const uint8_t *buf, size_t len);

#endif
