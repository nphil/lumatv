/* The access-unit splitter against hand-made streams: AUD-delimited and bare HEVC, two slices per picture, H.264, AV1 OBUs. */
#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "gf_annexb.h"

static size_t put_nal_hevc(unsigned char *d, size_t at, int type, int first_bit, int four) {
    if (four) d[at++] = 0;
    d[at++] = 0, d[at++] = 0, d[at++] = 1;
    d[at++] = (unsigned char)(type << 1);
    d[at++] = 1;                                   /* nuh_temporal_id_plus1 */
    d[at++] = first_bit ? 0x80 : 0x00;             /* the first bit of the slice header (or payload bytes for non-VCL) */
    d[at++] = 0x5a;
    d[at++] = 0x33;
    return at;
}

static size_t put_nal_h264(unsigned char *d, size_t at, int type, int first_bit) {
    d[at++] = 0, d[at++] = 0, d[at++] = 0, d[at++] = 1;
    d[at++] = (unsigned char)(0x60 | type);
    d[at++] = first_bit ? 0x80 : 0x00;
    d[at++] = 0x11;
    return at;
}

static void hevc_with_aud(void) {
    unsigned char d[512];
    size_t at = 0;
    /* AU 0: AUD VPS SPS PPS SEI IDR(19) */
    at = put_nal_hevc(d, at, 35, 0, 1);
    at = put_nal_hevc(d, at, 32, 0, 1);
    at = put_nal_hevc(d, at, 33, 0, 1);
    at = put_nal_hevc(d, at, 34, 0, 1);
    at = put_nal_hevc(d, at, 39, 0, 1);
    at = put_nal_hevc(d, at, 19, 1, 1);
    /* AU 1: AUD SEI TRAIL_R(1) */
    at = put_nal_hevc(d, at, 35, 0, 1);
    at = put_nal_hevc(d, at, 39, 0, 1);
    at = put_nal_hevc(d, at, 1, 1, 1);
    /* AU 2: AUD TRAIL_R with two slices */
    at = put_nal_hevc(d, at, 35, 0, 1);
    at = put_nal_hevc(d, at, 1, 1, 1);
    at = put_nal_hevc(d, at, 1, 0, 0);
    gf_unit u[8];
    size_t n = gf_split(d, at, GF_CODEC_H265, u, 8);
    assert(n == 3);
    assert(u[0].offset == 0 && u[0].key == 1 && u[0].has_sps == 1);
    assert(u[1].key == 0 && u[1].has_sps == 0);
    assert(u[0].offset + u[0].size == u[1].offset && u[1].offset + u[1].size == u[2].offset && u[2].offset + u[2].size == at);
    assert(d[u[1].offset] == 0 && d[u[1].offset + 3] == 1 && ((d[u[1].offset + 4] >> 1) & 0x3F) == 35);
    assert(gf_split(d, at, GF_CODEC_H265, NULL, 0) == 3);
    printf("hevc_with_aud ok\n");
}

static void hevc_bare(void) {
    unsigned char d[512];
    size_t at = 0;
    at = put_nal_hevc(d, at, 32, 0, 1);
    at = put_nal_hevc(d, at, 33, 0, 1);
    at = put_nal_hevc(d, at, 34, 0, 1);
    at = put_nal_hevc(d, at, 19, 1, 1);
    at = put_nal_hevc(d, at, 19, 0, 1);       /* second slice of the IDR */
    at = put_nal_hevc(d, at, 1, 1, 1);        /* picture 2 */
    at = put_nal_hevc(d, at, 39, 0, 1);       /* prefix SEI: opens picture 3 */
    at = put_nal_hevc(d, at, 1, 1, 1);
    at = put_nal_hevc(d, at, 40, 0, 1);       /* suffix SEI stays with picture 3 */
    at = put_nal_hevc(d, at, 1, 1, 1);        /* picture 4 */
    gf_unit u[8];
    size_t n = gf_split(d, at, GF_CODEC_H265, u, 8);
    assert(n == 4);
    assert(u[0].key == 1 && u[0].has_sps == 1);
    assert(u[1].key == 0 && u[2].key == 0 && u[3].key == 0);
    assert(u[3].offset + u[3].size == at);
    printf("hevc_bare ok\n");
}

static void h264(void) {
    unsigned char d[512];
    size_t at = 0;
    at = put_nal_h264(d, at, 9, 0);
    at = put_nal_h264(d, at, 7, 0);
    at = put_nal_h264(d, at, 8, 0);
    at = put_nal_h264(d, at, 5, 1);
    at = put_nal_h264(d, at, 9, 0);
    at = put_nal_h264(d, at, 1, 1);
    at = put_nal_h264(d, at, 1, 0);
    at = put_nal_h264(d, at, 1, 1);           /* no AUD: the first-mb bit opens it */
    gf_unit u[8];
    size_t n = gf_split(d, at, GF_CODEC_H264, u, 8);
    assert(n == 3);
    assert(u[0].key == 1 && u[0].has_sps == 1 && u[1].key == 0 && u[2].key == 0);
    assert(u[2].offset + u[2].size == at);
    printf("h264 ok\n");
}

static size_t put_obu(unsigned char *d, size_t at, int type, const unsigned char *payload, size_t n) {
    d[at++] = (unsigned char)((type << 3) | 2);      /* has_size_field */
    d[at++] = (unsigned char)n;
    memcpy(d + at, payload, n);
    return at + n;
}

static void av1(void) {
    unsigned char d[512], none[1] = {0}, seq[4] = {0x00, 0x00, 0x00, 0x00}, keyfh[4] = {0x10, 0, 0, 0}, interfh[4] = {0x30, 0, 0, 0};
    size_t at = 0;
    at = put_obu(d, at, 2, none, 0);
    at = put_obu(d, at, 1, seq, 4);
    at = put_obu(d, at, 6, keyfh, 4);
    at = put_obu(d, at, 2, none, 0);
    at = put_obu(d, at, 6, interfh, 4);
    at = put_obu(d, at, 2, none, 0);
    at = put_obu(d, at, 6, interfh, 4);
    gf_unit u[8];
    size_t n = gf_split(d, at, GF_CODEC_AV1, u, 8);
    assert(n == 3);
    assert(u[0].key == 1 && u[0].has_sps == 1 && u[1].key == 0 && u[2].key == 0);
    assert(u[0].offset == 0 && u[2].offset + u[2].size == at);
    assert(((d[u[1].offset] >> 3) & 0xF) == 2);
    printf("av1 ok\n");
}

int main(void) {
    hevc_with_aud();
    hevc_bare();
    h264();
    av1();
    printf("split_test: all ok\n");
    return 0;
}
