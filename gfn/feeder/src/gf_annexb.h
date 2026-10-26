/* Access units out of an elementary stream: Annex B (H.264, H.265) and the AV1 low-overhead OBU stream (what `-f obu` writes).
 *
 * NDL wants one picture per NDL_DirectVideoPlay (jellyfin-webos-native docs/ndl.md, "Split into access units"). A new picture starts at an
 * access unit delimiter, at a parameter set or prefix SEI that follows a slice, or at a VCL NAL whose first bit after the header says it is
 * the first slice of its picture (H.264 first_mb_in_slice = 0 is the Exp-Golomb bit 1; H.265 first_slice_segment_in_pic_flag is that bit).
 * AV1: a temporal unit starts at a temporal delimiter OBU (type 2).
 */
#ifndef GF_ANNEXB_H
#define GF_ANNEXB_H

#include <stddef.h>
#include <stdint.h>

enum { GF_CODEC_H264 = 1, GF_CODEC_H265 = 2, GF_CODEC_AV1 = 4 };

typedef struct {
    size_t offset, size;    /* the unit's bytes within the stream */
    int key;                /* 1 when the unit carries an IDR (H.264 5; H.265 16..21) or an AV1 key frame */
    int has_sps;            /* a parameter set is inside (what a decoder needs to start) */
} gf_unit;

/* Splits `data` into units; writes at most `cap` entries and returns how many there are in total (so a second call with a bigger table
 * finishes the job). `codec` is a GF_CODEC_*. A stream that does not begin at a unit boundary has its leading bytes in unit 0. */
size_t gf_split(const uint8_t *data, size_t n, int codec, gf_unit *out, size_t cap);

/* True when a NAL unit header byte(s) at `p` is a VCL unit that begins a picture. `p` points at the NAL header. */
int gf_first_slice(const uint8_t *p, size_t left, int codec);

/* NAL type of the header at `p`. */
int gf_nal_type(const uint8_t *p, int codec);

#endif
