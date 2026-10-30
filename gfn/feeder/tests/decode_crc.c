/* A checksum of every picture FFmpeg's decoders give for an Annex B clip, linked to the libraries directly. Run for the host and, under qemu-arm-static, for the
 * ARM build (static, musl): the same clip must give the same lines, which says that the NEON assembly the cross compiler made decodes as the C does
 * (tests/arm_decode_check.sh). Not part of the feeder. */
#include <libavcodec/avcodec.h>
#include <libavutil/cpu.h>
#include <libavutil/frame.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gf_annexb.h"

static uint32_t fnv(uint32_t h, const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 16777619u;
    return h;
}

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    int h265 = !strcmp(argv[2], "h265");
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *data = malloc((size_t)n);
    if (fread(data, 1, (size_t)n, f) != (size_t)n) return 2;
    fclose(f);
    static gf_unit units[16384];
    size_t count = gf_split(data, (size_t)n, h265 ? GF_CODEC_H265 : GF_CODEC_H264, units, 16384);
    const AVCodec *c = avcodec_find_decoder_by_name(h265 ? "hevc" : "h264");
    AVCodecContext *ctx = avcodec_alloc_context3(c);
    ctx->thread_count = 1;
    ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
    if (avcodec_open2(ctx, c, NULL) < 0) return 1;
    AVPacket *pkt = av_packet_alloc();
    AVFrame *fr = av_frame_alloc();
    fprintf(stderr, "cpu flags %#x, neon %s\n", av_get_cpu_flags(), (av_get_cpu_flags() & AV_CPU_FLAG_NEON) ? "on" : "off");
    int pictures = 0;
    for (size_t i = 0; i < count; i++) {
        pkt->data = data + units[i].offset;
        pkt->size = (int)units[i].size;
        if (avcodec_send_packet(ctx, pkt) < 0) continue;
        while (avcodec_receive_frame(ctx, fr) == 0) {
            uint32_t h = 2166136261u;
            for (int y = 0; y < fr->height; y++) h = fnv(h, fr->data[0] + (size_t)y * fr->linesize[0], (size_t)fr->width);
            for (int p = 1; p < 3; p++)
                for (int y = 0; y < (fr->height + 1) / 2; y++) h = fnv(h, fr->data[p] + (size_t)y * fr->linesize[p], (size_t)(fr->width + 1) / 2);
            printf("%d %dx%d %08x\n", pictures++, fr->width, fr->height, h);
            av_frame_unref(fr);
        }
    }
    return pictures > 0 ? 0 : 1;
}
