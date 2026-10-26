/* The feeder's Opus decoder (gf_opus.c over build/opus) as a command: packets in (u16 length, bytes; the --audio file format) from the file
 * named by argv[1], S16 stereo PCM out on stdout. tests/test_gfn_feeder.py builds it for ARM (static musl, so qemu-arm runs it with no
 * sysroot) to show that the ARM build of the decoder gives the host's samples. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "gf_opus.h"
int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "rb");
    char err[200];
    gf_opus *d = gf_opus_open(48000, 2, err, sizeof err);
    if (!d) { fprintf(stderr, "%s\n", err); return 1; }
    static int16_t pcm[GF_OPUS_MAX_FRAMES * 2];
    uint8_t buf[2000];
    uint8_t lb[2];
    while (fread(lb, 1, 2, f) == 2) {
        unsigned n = lb[0] | (lb[1] << 8);
        if (fread(buf, 1, n, f) != n) return 2;
        int fr = gf_opus_decode(d, buf, n, pcm, GF_OPUS_MAX_FRAMES);
        if (fr <= 0) return 3;
        fwrite(pcm, 4, (size_t)fr, stdout);
    }
    return 0;
}
