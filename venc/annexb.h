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

/* Writes a pixel aspect ratio into the first SPS of the access unit `au` (`len` bytes in a buffer of `cap`): the VUI's aspect_ratio_info
 * (H.264 E.1.1), which the encoder's own SPS does not carry. The set's encoder writes a VUI with timing_info and bitstream_restriction: the
 * ratio goes in front of its other fields, which are kept; an SPS with no VUI gets one with the ratio alone. The unit grows by a few bytes and
 * everything after the SPS moves up. Returns the unit's new length, or 0 with the unit untouched when there is no SPS it can rewrite (one with
 * scaling lists or a ratio already, or one that does not parse as the encoder writes them), the ratio is 0 or does not fit 16 bits, or `cap`
 * is too small. */
size_t annexb_set_sar(uint8_t *au, size_t len, size_t cap, unsigned sar_w, unsigned sar_h);

#endif
