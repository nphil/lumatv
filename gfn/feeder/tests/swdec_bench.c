/* What a camera's picture costs as a texture: gf_swdec.c decodes every access unit of an Annex B clip and gf_yuv.c scales and converts each picture to
 * the box size, timed in CPU time (a loaded host does not inflate it). Not a test: the numbers are for docs/research/multiview-result.md, and the same
 * program is built for the set, where the feeder's own `tex` stats line says the same thing a second at a time.
 *
 *   swdec_bench LIBDIR CLIP h264|h265 OUT_WxH
 *
 * The clip is an Annex B stream; its units are cut as the feeder cuts a file (gf_annexb.c). */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "gf_annexb.h"
#include "gf_swdec.h"
#include "gf_yuv.h"

static double cpu_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

int main(int argc, char **argv) {
    if (argc < 5) {
        fprintf(stderr, "usage: swdec_bench LIBDIR CLIP h264|h265 OUT_WxH\n");
        return 2;
    }
    unsigned codec = !strcmp(argv[3], "h265") ? GF_CODEC_H265 : GF_CODEC_H264;
    int dw = 0, dh = 0;
    if (sscanf(argv[4], "%dx%d", &dw, &dh) != 2) return 2;
    FILE *f = fopen(argv[2], "rb");
    if (!f) {
        perror(argv[2]);
        return 2;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *data = malloc((size_t)n);
    if (fread(data, 1, (size_t)n, f) != (size_t)n) return 2;
    fclose(f);
    static gf_unit table[16384];
    size_t units_n = gf_split(data, (size_t)n, (int)codec, table, sizeof table / sizeof *table);
    int units = (int)(units_n < 16384 ? units_n : 16384);
    gf_swdec *d;
    char err[300];
    if (gf_swdec_open(&d, argv[1], codec, 0, 0, err, sizeof err) != 0) {
        fprintf(stderr, "open: %s\n", err);
        return 1;
    }
    uint32_t *out = malloc((size_t)dw * dh * 4);
    void *work = NULL;
    size_t work_cap = 0;
    double dec = 0, conv = 0, dec_max = 0, conv_max = 0;
    int pictures = 0, sw = 0, sh = 0;
    for (int i = 0; i < units; i++) {
        double t0 = cpu_ms();
        int rc = gf_swdec_decode(d, data + table[i].offset, table[i].size, (int64_t)i * 33333, err, sizeof err);
        double t1 = cpu_ms();
        dec += t1 - t0;
        if (t1 - t0 > dec_max) dec_max = t1 - t0;
        if (rc < 0) {
            fprintf(stderr, "unit %d: %s\n", i, err);
            continue;
        }
        if (rc == 0) continue;
        gf_yuv pic;
        if (gf_swdec_picture(d, &pic, err, sizeof err) != 0) {
            fprintf(stderr, "picture: %s\n", err);
            return 1;
        }
        size_t need = gf_yuv_work_size(pic.width, pic.height, dw, dh);
        if (need > work_cap) {
            free(work);
            work = aligned_alloc(16, (need + 15) & ~(size_t)15);
            work_cap = need;
        }
        double t2 = cpu_ms();
        gf_yuv_to_xrgb(&pic, out, dw, dh, dw, work);
        double t3 = cpu_ms();
        conv += t3 - t2;
        if (t3 - t2 > conv_max) conv_max = t3 - t2;
        sw = pic.width;
        sh = pic.height;
        pictures++;
    }
    printf("{\"clip\":\"%s\",\"codec\":\"%s\",\"src\":\"%dx%d\",\"out\":\"%dx%d\",\"units\":%d,\"pictures\":%d,\"decode_ms_avg\":%.2f,\"decode_ms_max\":%.2f,"
           "\"convert_ms_avg\":%.2f,\"convert_ms_max\":%.2f,\"cpu_ms_per_picture\":%.2f}\n",
           argv[2], argv[3], sw, sh, dw, dh, units, pictures, pictures ? dec / pictures : 0, dec_max, pictures ? conv / pictures : 0, conv_max,
           pictures ? (dec + conv) / pictures : 0);
    gf_swdec_close(d);
    return pictures > 0 ? 0 : 1;
}
