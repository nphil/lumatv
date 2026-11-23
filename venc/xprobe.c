/* The decoder probes (lumatv-venc vdec, lumatv-venc chain): access units from an .au file (tools/es2au.py) go
 * through the hardware decoder (vdec.c); with `chain` every picture goes on to the hardware encoder (venc.c)
 * by physical address, the transcode path with the engine's bookkeeping left out. Each run prints per-picture
 * lines for the first pictures and one JSON line at the end.
 *
 *   lumatv-venc vdec  -i FILE.au [--codec mpeg2|h264] [--step setup|streamon|run] [-n MAX_AUS] [-r AU_PER_S]
 *                     [--dump PREFIX [--dump-n K]] [--ncap N] [--max WxH] [--check]
 *   lumatv-venc chain -i FILE.au [same] -o OUT.h264 [--keep-every N] [--fps F] [-b KBPS] [-g GOP] [--cbr] [-P ...]
 *
 * --step setup stops after the formats and buffers (nothing asked of the video MCU), streamon after the
 * first STREAMON (the decoder channel is taken and given back), run (default) decodes.
 * -r paces the access units in (0: as fast as the decoder takes them). --keep-every N encodes every Nth
 * picture only (60 -> 30 frames per second with 2). --dump writes the first K pictures as raw NV12 (w x h
 * luma then w x h/2 chroma). */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "annexb.h"
#include "sysops.h"
#include "vdec.h"
#include "venc.h"
#include "watchdog.h"
#include "xprobe.h"

#define STALL_S 5.0 /* no picture for this long while access units are in flight: the run ends as failed */

static volatile sig_atomic_t stop_now;
static void on_signal(int sig) { (void)sig; stop_now = 1; }

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static double cpu_s(void)
{
    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    return (double)ru.ru_utime.tv_sec + ru.ru_utime.tv_usec / 1e6 + (double)ru.ru_stime.tv_sec + ru.ru_stime.tv_usec / 1e6;
}

struct au {
    const uint8_t *data;
    uint32_t size, flags;
};

struct file_aus {
    void *map;
    size_t maplen;
    struct au *aus;
    size_t n, largest;
};

static int load_aus(const char *path, size_t limit, struct file_aus *f)
{
    memset(f, 0, sizeof(*f));
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    struct stat st;
    if (fd < 0 || fstat(fd, &st) < 0) {
        fprintf(stderr, "%s: %s\n", path, strerror(errno));
        if (fd >= 0)
            close(fd);
        return -1;
    }
    f->maplen = (size_t)st.st_size;
    f->map = mmap(NULL, f->maplen, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (f->map == MAP_FAILED) {
        fprintf(stderr, "%s: mmap: %s\n", path, strerror(errno));
        return -1;
    }
    size_t cap = 1024;
    f->aus = malloc(cap * sizeof(*f->aus));
    const uint8_t *p = f->map, *end = p + f->maplen;
    while (p + 24 <= end && (!limit || f->n < limit)) {
        uint32_t flags, size;
        if (memcmp(p, "XAUN", 4)) {
            fprintf(stderr, "%s: bad record at byte %zu\n", path, (size_t)(p - (const uint8_t *)f->map));
            return -1;
        }
        memcpy(&flags, p + 4, 4);
        memcpy(&size, p + 16, 4);
        if (p + 24 + size > end)
            break;
        if (f->n == cap) {
            cap *= 2;
            f->aus = realloc(f->aus, cap * sizeof(*f->aus));
        }
        f->aus[f->n++] = (struct au){ .data = p + 24, .size = size, .flags = flags };
        if (size > f->largest)
            f->largest = size;
        p += 24 + size;
    }
    if (!f->n) {
        fprintf(stderr, "%s: no access units\n", path);
        return -1;
    }
    return 0;
}

struct xopts {
    const char *mode, *in, *out, *dump, *step;
    uint32_t codec, max_w, max_h, ncap, nout, dump_n, limit, keep_every;
    double rate, fps;
    uint32_t kbps, gop;
    int cbr, check, bench, profile;
};

static int parse_wxh(const char *s, uint32_t *w, uint32_t *h)
{
    return sscanf(s, "%ux%u", w, h) == 2;
}

static int xparse(int argc, char **argv, struct xopts *o)
{
    memset(o, 0, sizeof(*o));
    o->mode = argv[1];
    o->codec = V4L2_PIX_FMT_MPEG2;
    o->max_w = 1920;
    o->max_h = 1088;
    o->ncap = 6;
    o->nout = 4;
    o->dump_n = 4;
    o->keep_every = 1;
    o->step = "run";
    o->fps = 30;
    o->kbps = 3500;
    o->gop = 60;
    o->profile = LX_VENC_H264_PROFILE_HIGH;
    for (int i = 2; i < argc; i++) {
        const char *a = argv[i];
        const char *v = i + 1 < argc ? argv[i + 1] : NULL;
#define NEED do { if (!v) return -1; i++; } while (0)
        if (!strcmp(a, "-i")) { NEED; o->in = v; }
        else if (!strcmp(a, "-o")) { NEED; o->out = v; }
        else if (!strcmp(a, "--codec")) {
            NEED;
            if (!strcmp(v, "mpeg2")) o->codec = V4L2_PIX_FMT_MPEG2;
            else if (!strcmp(v, "h264")) o->codec = V4L2_PIX_FMT_H264;
            else return -1;
        }
        else if (!strcmp(a, "--step")) { NEED; o->step = v; }
        else if (!strcmp(a, "-n")) { NEED; o->limit = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-r")) { NEED; o->rate = atof(v); }
        else if (!strcmp(a, "--dump")) { NEED; o->dump = v; }
        else if (!strcmp(a, "--dump-n")) { NEED; o->dump_n = (uint32_t)atoi(v); }
        else if (!strcmp(a, "--ncap")) { NEED; o->ncap = (uint32_t)atoi(v); }
        else if (!strcmp(a, "--nout")) { NEED; o->nout = (uint32_t)atoi(v); }
        else if (!strcmp(a, "--max")) { NEED; if (!parse_wxh(v, &o->max_w, &o->max_h)) return -1; }
        else if (!strcmp(a, "--keep-every")) { NEED; o->keep_every = (uint32_t)atoi(v); }
        else if (!strcmp(a, "--fps")) { NEED; o->fps = atof(v); }
        else if (!strcmp(a, "-b")) { NEED; o->kbps = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-g")) { NEED; o->gop = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-P")) {
            NEED;
            if (!strcmp(v, "high")) o->profile = LX_VENC_H264_PROFILE_HIGH;
            else if (!strcmp(v, "main")) o->profile = LX_VENC_H264_PROFILE_MAIN;
            else if (!strcmp(v, "baseline")) o->profile = LX_VENC_H264_PROFILE_BASELINE;
            else return -1;
        }
        else if (!strcmp(a, "--cbr")) o->cbr = 1;
        else if (!strcmp(a, "--check")) o->check = 1;
        else if (!strcmp(a, "--bench")) o->bench = 1;
        else return -1;
#undef NEED
    }
    if (!o->in || !o->keep_every || (strcmp(o->step, "setup") && strcmp(o->step, "streamon") && strcmp(o->step, "run")))
        return -1;
    return 0;
}

static uint32_t crc32_update(uint32_t crc, const uint8_t *p, size_t n)
{
    static uint32_t table[256];
    if (!table[1])
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t c = i;
            for (int k = 0; k < 8; k++)
                c = c & 1 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            table[i] = c;
        }
    crc = ~crc;
    while (n--)
        crc = table[(crc ^ *p++) & 0xff] ^ (crc >> 8);
    return ~crc;
}

static void dump_picture(const struct xopts *o, const struct vdec_pic *pic, uint32_t n)
{
    char path[512];
    snprintf(path, sizeof(path), "%s-%03u.nv12", o->dump, n);
    FILE *f = fopen(path, "wb");
    if (!f) {
        fprintf(stderr, "%s: %s\n", path, strerror(errno));
        return;
    }
    fwrite(pic->y, 1, (size_t)pic->width * pic->height, f);
    fwrite(pic->c, 1, (size_t)pic->width * pic->height / 2, f);
    fclose(f);
}

/* How fast the CPU reads a decoded picture through its dma-buf mapping (what a CPU scaler would pay). */
static void bench_read(const struct vdec_pic *pic)
{
    size_t n = (size_t)pic->width * pic->height;
    uint8_t *tmp = malloc(n);
    if (!tmp)
        return;
    double t = now_s();
    memcpy(tmp, pic->y, n);
    __asm__ volatile("" : : "r"(tmp) : "memory");
    double copy = now_s() - t;
    t = now_s();
    uint64_t sum = 0;
    const volatile uint64_t *q = (const volatile uint64_t *)pic->y;
    for (size_t i = 0; i < n / 8; i++)
        sum += q[i];
    double wide = now_s() - t;
    t = now_s();
    const volatile uint8_t *b = pic->y;
    for (size_t i = 0; i < n / 8; i++)
        sum += b[i];
    double narrow = now_s() - t;
    t = now_s();
    memcpy(tmp, pic->y, n);
    __asm__ volatile("" : : "r"(tmp) : "memory");
    double copy2 = now_s() - t;
    printf("bench: %zu bytes of luma: memcpy %.1f MB/s (again %.1f), 8-byte loads %.1f MB/s, byte loads %.2f MB/s (sum %llu)\n", n, n / copy / 1e6,
           n / copy2 / 1e6, n / wide / 1e6, (n / 8) / narrow / 1e6, (unsigned long long)sum);
    free(tmp);
}

struct run {
    double sum_dec, max_dec, sum_total, max_total, sum_enc, max_enc, last_pic, sum_gap, max_gap;
    uint64_t enc_frames, enc_bytes, enc_keys;
    uint32_t w, h;
    unsigned sizes_seen;
};

int xprobe_main(int argc, char **argv)
{
    struct xopts o;
    if (xparse(argc, argv, &o) < 0) {
        fputs("usage: lumatv-venc vdec|chain -i FILE.au [--codec mpeg2|h264] [--step setup|streamon|run] [-n MAX_AUS] [-r AU_PER_S]\n"
              "       [--dump PREFIX] [--dump-n K] [--ncap N] [--nout N] [--max WxH] [--check] [--bench]\n"
              "       chain: -o OUT.h264 [--keep-every N] [--fps F] [-b KBPS] [-g GOP] [--cbr] [-P high|main|baseline]\n",
              stderr);
        return 64;
    }
    int chain = !strcmp(o.mode, "chain");
    if (chain && !o.out && !strcmp(o.step, "run")) {
        fputs("chain needs -o OUT.h264\n", stderr);
        return 64;
    }
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);
    setvbuf(stdout, NULL, _IOLBF, 0);
    wd_install();

    struct file_aus fa;
    if (load_aus(o.in, o.limit, &fa) < 0)
        return 2;
    double *t_feed = calloc(fa.n, sizeof(double));
    char err[256] = "";
    struct vdec d;
    if (vdec_open(&d, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "vdec: %s\n", err);
        return 2;
    }
    uint32_t out_size = (uint32_t)((fa.largest * 5 / 4 + 65535) & ~65535u);
    if (out_size < (256u << 10))
        out_size = 256u << 10;
    struct vdec_cfg cfg = { .codec = o.codec, .max_w = o.max_w, .max_h = o.max_h, .nout = o.nout, .ncap = o.ncap, .out_size = out_size, .pool = "photofb" };
    double t_start = now_s(), cpu0 = cpu_s();
    if (vdec_setup(&d, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "vdec: %s\n", err);
        return 3;
    }
    printf("vdec: %u bitstream buffers of %zu bytes, %u picture buffers of %u bytes (first at 0x%08x), res-info hint %s, %zu access units (largest %zu bytes)\n",
           d.nout, d.out[0].len, d.ncap, d.capsize, d.cap[0].mem.paddr, d.res_info_ok ? "accepted" : "not accepted", fa.n, fa.largest);
    if (!strcmp(o.step, "setup")) {
        vdec_close(&d);
        printf("{\"step\":\"vdec-setup\",\"ok\":true}\n");
        return 0;
    }
    if (vdec_streamon(&d, err, sizeof(err)) < 0) {
        fprintf(stderr, "vdec: %s\n", err);
        return 3;
    }
    printf("vdec: streaming (%.0f ms for setup and STREAMON)\n", (now_s() - t_start) * 1e3);
    if (!strcmp(o.step, "streamon")) {
        usleep(300 * 1000);
        vdec_close(&d);
        printf("{\"step\":\"vdec-streamon\",\"ok\":true}\n");
        return 0;
    }

    struct venc v;
    memset(&v, 0, sizeof(v));
    int enc_up = 0, out_fd = -1, rc = 0;
    if (chain) {
        out_fd = open(o.out, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
        if (out_fd < 0) {
            fprintf(stderr, "%s: %s\n", o.out, strerror(errno));
            vdec_close(&d);
            return 2;
        }
    }

    struct run r;
    memset(&r, 0, sizeof(r));
    size_t next = 0;
    double t0 = now_s(), due = t0, eos_t = 0, first_feed = 0, first_pic = 0;
    int eos = 0, stalled = 0;
    uint32_t dumped = 0, crc_n = 0;
    uint64_t pic_no = 0;
    uint32_t crc_all = 0;
    struct vdec_pic pic;
    while (!stop_now) {
        double t = now_s();
        if (next < fa.n && t >= due) {
            int fr = vdec_feed(&d, fa.aus[next].data, fa.aus[next].size, next, 0, err, sizeof(err));
            if (fr < 0) {
                fprintf(stderr, "vdec: %s\n", err);
                rc = 4;
                break;
            }
            if (fr == 0) {
                t_feed[next] = t;
                if (!first_feed)
                    first_feed = t;
                next++;
                due = o.rate > 0 ? due + 1.0 / o.rate : t;
            }
        }
        if (next >= fa.n && !eos) {
            if (vdec_drain(&d, err, sizeof(err)) < 0)
                fprintf(stderr, "vdec: drain: %s\n", err);
            eos = 1;
            eos_t = now_s();
        }
        double wait_s = 0.005;
        if (next < fa.n && o.rate > 0 && due > t)
            wait_s = due - t > 0.02 ? 0.02 : due - t;
        int gr = vdec_get_picture(&d, &pic, (int)(wait_s * 1000), err, sizeof(err));
        if (gr < 0) {
            fprintf(stderr, "vdec: %s\n", err);
            rc = 4;
            break;
        }
        t = now_s();
        if (gr == 0) {
            double idle_from = r.last_pic > 0 ? r.last_pic : first_feed;
            if (eos && t - eos_t > 3.0) {
                printf("vdec: no end-of-stream marker %.1f s after the drain; stopping\n", t - eos_t);
                break;
            }
            if (first_feed && t - idle_from > STALL_S && (next > d.pictures)) {
                printf("vdec: STALL: no picture for %.1f s with %zu access units fed, %llu pictures out\n", t - idle_from, next,
                       (unsigned long long)d.pictures);
                stalled = 1;
                rc = 5;
                break;
            }
            continue;
        }
        if (pic.last) {
            vdec_put_picture(&d, pic.index, err, sizeof(err));
            printf("vdec: end-of-stream marker after %llu pictures\n", (unsigned long long)d.pictures);
            break;
        }
        if (!first_pic) {
            first_pic = t;
            printf("vdec: first picture %.0f ms after the first access unit (seq %llu, %ux%u)\n", (t - first_feed) * 1e3, (unsigned long long)pic.seq,
                   pic.width, pic.height);
        }
        if (r.last_pic > 0) {
            double gap = t - r.last_pic;
            r.sum_gap += gap;
            if (gap > r.max_gap)
                r.max_gap = gap;
        }
        r.last_pic = t;
        double dec_s = pic.seq < fa.n ? t - t_feed[pic.seq] : 0;
        r.sum_dec += dec_s;
        if (dec_s > r.max_dec)
            r.max_dec = dec_s;
        if (pic.width != r.w || pic.height != r.h) {
            r.w = pic.width;
            r.h = pic.height;
            r.sizes_seen++;
            printf("vdec: picture size %ux%u (picture %llu, source changes so far %u)\n", r.w, r.h, (unsigned long long)pic_no, d.source_changes);
        }
        if (pic_no < 12 || pic_no % 60 == 0)
            printf("vdec: picture %llu seq %llu %ux%u%s decode %.1f ms (fed %zu)\n", (unsigned long long)pic_no, (unsigned long long)pic.seq, pic.width,
                   pic.height, pic.error ? " ERROR" : "", dec_s * 1e3, next);
        if (o.bench && pic_no == 8)
            bench_read(&pic);
        if (o.dump && dumped < o.dump_n) {
            dump_picture(&o, &pic, dumped);
            dumped++;
        }
        if (o.check) {
            uint32_t c = crc32_update(0, pic.y, (size_t)pic.width * pic.height);
            c = crc32_update(c, pic.c, (size_t)pic.width * pic.height / 2);
            crc_all = crc32_update(crc_all, (const uint8_t *)&c, 4);
            if (crc_n++ < 8)
                printf("vdec: picture %llu crc32 %08x\n", (unsigned long long)pic_no, c);
        }
        if (chain && pic_no % o.keep_every == 0 && !pic.error) {
            if (enc_up && (v.cfg.width != pic.width || v.cfg.height != pic.height)) {
                venc_close(&v);
                enc_up = 0;
            }
            if (!enc_up) {
                struct venc_cfg ec = { .width = pic.width, .height = pic.height, .fps = (uint32_t)(o.fps + 0.5), .gop = o.gop, .kbps = o.kbps,
                                       .fixed_qp = 26, .profile = (uint32_t)o.profile, .cbr = o.cbr };
                if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0 || venc_create(&v, &ec, err, sizeof(err)) < 0) {
                    fprintf(stderr, "venc: %s\n", err);
                    rc = 6;
                    vdec_put_picture(&d, pic.index, err, sizeof(err));
                    break;
                }
                enc_up = 1;
                printf("venc: created %ux%u %u fps %u kbps gop %u\n", ec.width, ec.height, ec.fps, ec.kbps, ec.gop);
            }
            struct venc_frame f;
            double te = now_s();
            if (venc_encode(&v, pic.paddr_y, pic.paddr_c, &f, err, sizeof(err)) < 0) {
                fprintf(stderr, "venc: %s\n", err);
                rc = 6;
                vdec_put_picture(&d, pic.index, err, sizeof(err));
                break;
            }
            double done = now_s();
            double enc_s = done - te, total_s = pic.seq < fa.n ? done - t_feed[pic.seq] : 0;
            r.sum_enc += enc_s;
            if (enc_s > r.max_enc)
                r.max_enc = enc_s;
            r.sum_total += total_s;
            if (total_s > r.max_total)
                r.max_total = total_s;
            r.enc_frames++;
            r.enc_bytes += f.size;
            r.enc_keys += f.keyframe;
            if (r.enc_frames <= 4)
                printf("venc: frame %llu %u bytes %s encode %.1f ms, access unit in -> H.264 out %.1f ms\n", (unsigned long long)r.enc_frames, f.size,
                       f.keyframe ? "I" : "P", enc_s * 1e3, total_s * 1e3);
            if (write(out_fd, f.data, f.size) != (ssize_t)f.size) {
                fprintf(stderr, "%s: write: %s\n", o.out, strerror(errno));
                rc = 2;
                vdec_put_picture(&d, pic.index, err, sizeof(err));
                break;
            }
        }
        pic_no++;
        if (vdec_put_picture(&d, pic.index, err, sizeof(err)) < 0) {
            fprintf(stderr, "vdec: %s\n", err);
            rc = 4;
            break;
        }
    }
    double wall = now_s() - t0, cpu = cpu_s() - cpu0;
    unsigned changes = d.source_changes;
    unsigned out_errors = d.out_errors, cap_errors = d.cap_errors;
    unsigned long long fed = d.fed;
    if (enc_up)
        venc_close(&v);
    if (out_fd >= 0)
        close(out_fd);
    vdec_close(&d);
    double n = pic_no ? (double)pic_no : 1;
    printf("{\"step\":\"%s\",\"ok\":%s,\"aus\":%zu,\"fed\":%llu,\"pictures\":%llu,\"size\":\"%ux%u\",\"sizes_seen\":%u,\"source_changes\":%u,"
           "\"first_picture_ms\":%.0f,\"wall_s\":%.3f,\"pic_per_s\":%.2f,\"cpu_pct\":%.1f,\"decode_ms_avg\":%.1f,\"decode_ms_max\":%.1f,"
           "\"gap_ms_avg\":%.1f,\"gap_ms_max\":%.1f,\"out_errors\":%u,\"cap_errors\":%u,\"stalled\":%d,\"crc_all\":\"%08x\"",
           chain ? "chain" : "vdec", rc ? "false" : "true", fa.n, fed, (unsigned long long)pic_no, r.w, r.h, r.sizes_seen, changes,
           first_pic ? (first_pic - first_feed) * 1e3 : -1.0, wall, wall > 0 ? pic_no / wall : 0.0, wall > 0 ? 100.0 * cpu / wall : 0.0,
           1e3 * r.sum_dec / n, 1e3 * r.max_dec, pic_no > 1 ? 1e3 * r.sum_gap / (double)(pic_no - 1) : 0.0, 1e3 * r.max_gap, out_errors,
           cap_errors, stalled, crc_all);
    if (chain) {
        double m = r.enc_frames ? (double)r.enc_frames : 1;
        printf(",\"encoded\":%llu,\"keyframes\":%llu,\"bytes\":%llu,\"kbps_got\":%.0f,\"encode_ms_avg\":%.2f,\"encode_ms_max\":%.2f,"
               "\"in_to_out_ms_avg\":%.1f,\"in_to_out_ms_max\":%.1f",
               (unsigned long long)r.enc_frames, (unsigned long long)r.enc_keys, (unsigned long long)r.enc_bytes,
               wall > 0 ? r.enc_bytes * 8.0 / wall / 1000.0 : 0.0, 1e3 * r.sum_enc / m, 1e3 * r.max_enc, 1e3 * r.sum_total / m, 1e3 * r.max_total);
    }
    puts("}");
    free(t_feed);
    free(fa.aus);
    munmap(fa.map, fa.maplen);
    return rc;
}
