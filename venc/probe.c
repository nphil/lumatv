/* lumatv-venc: the C4 hardware-encoder probe and the first streaming path.
 *
 * Small steps, each a separate run that ends by itself (the slot script runs them one by one and stops at
 * the first failure):
 *   lumatv-venc caps                     read-only: the device nodes, /proc/hma/status, nothing opened but files
 *   lumatv-venc vtcaps                   the video tap's capability record (opens /dev/video60 once, no capture)
 *   lumatv-venc gate                     open /dev/venc, pass the KADP gate, close
 *   lumatv-venc create [-w -h]           + ENC_CREATE, GET_BUFFER_INFO printed and checked against the pool table, DESTROY
 *   lumatv-venc map    [-w -h]           + mmap of the ES ring, its first bytes read, unmapped
 *   lumatv-venc hma    [-w -h]           one NV12 frame from an HMA pool: alloc, write, read back, free; nothing encoded
 *   lumatv-venc synth  [opts] -o out.h264 a moving test picture from CPU memory through /dev/venc (the encoder alone)
 *   lumatv-venc tapopen [opts]           the tap opened at the point and size asked, buffers mapped and their addresses
 *                                        printed, no streaming, closed
 *   lumatv-venc vt     [opts] -o out.h264 the screen: video tap -> encoder, no copy (the chosen route)
 *   lumatv-venc serve  [opts] -p PORT    the screen as an H.264 byte stream to one TCP client at a time
 *   lumatv-venc feed   [opts]            NV12 frames on stdin, H.264 access units on stdout: the compositor's screen recorder (feed_proto.h)
 *
 * /dev/video24 (the V4L2 M2M node) is never used: its mmap maps the kernel page at vmalloc_to_page(<physical
 * address>), so a frame written through it lands in kernel memory; the first slot wedged the kernel that way
 * (docs/research/venc-result.md).
 *
 * Options: -w WIDTH -h HEIGHT (default 1280x720) -r FPS (30) -b KBPS (4000; 0 = fixed QP -q) -g GOP (30)
 *          -n FRAMES (300) -l blended|osd|display|scaler-output (blended) -d DIVIDE (1) -P profile high|main|baseline
 *          --pool NAME (photofb) --csv FILE --cbr --nbuf N (4) --active-file PATH (serve)
 * Every run prints one JSON line with the numbers at the end. Nothing is written outside the paths given.
 * Every call that can block is under a watchdog (watchdog.h): exit 75 names the call that hung.
 */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#include "annexb.h"
#include "feed_proto.h"
#include "hma.h"
#include "pools.h"
#include "synth.h"
#include "sysops.h"
#include "venc.h"
#include "vtcap.h"
#include "watchdog.h"

#define SLOW_LIMIT 3 /* encodes slower than VENC_SLOW_MS before a run stops */

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

struct opts {
    const char *mode, *out, *csv, *pool, *active_file, *location;
    uint32_t w, h, fps, kbps, qp, gop, frames, divide, nbuf, port;
    int cbr, profile;
};

static void usage(void)
{
    fputs("usage: lumatv-venc caps|vtcaps|gate|create|map|hma|synth|tapopen|vt|serve|feed [-w W] [-h H] [-r FPS] [-b KBPS] [-q QP]\n"
          "       [-g GOP] [-n FRAMES] [-l blended|osd|display|scaler-output] [-d DIVIDE] [-P high|main|baseline]\n"
          "       [--pool NAME] [--cbr] [--nbuf N] [--csv FILE] [--active-file PATH] [-p PORT] [-o OUT.h264 (not feed)]\n",
          stderr);
}

static int parse(int argc, char **argv, struct opts *o)
{
    memset(o, 0, sizeof(*o));
    o->w = 1280; o->h = 720; o->fps = 30; o->kbps = 4000; o->qp = 26; o->gop = 30; o->frames = 300;
    o->divide = 1; o->nbuf = 4; o->pool = "photofb"; o->location = "blended"; o->port = 8554;
    o->active_file = "/tmp/lumatv/venc-active";
    if (argc < 2) return -1;
    o->mode = argv[1];
    for (int i = 2; i < argc; i++) {
        const char *a = argv[i];
        const char *v = i + 1 < argc ? argv[i + 1] : NULL;
#define NEED if (!v) return -1; i++
        if (!strcmp(a, "-w")) { NEED; o->w = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-h")) { NEED; o->h = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-r")) { NEED; o->fps = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-b")) { NEED; o->kbps = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-q")) { NEED; o->qp = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-g")) { NEED; o->gop = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-n")) { NEED; o->frames = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-l")) { NEED; o->location = v; }
        else if (!strcmp(a, "-d")) { NEED; o->divide = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-p")) { NEED; o->port = (uint32_t)atoi(v); }
        else if (!strcmp(a, "-o")) { NEED; o->out = v; }
        else if (!strcmp(a, "-P")) {
            NEED;
            if (!strcmp(v, "high")) o->profile = LX_VENC_H264_PROFILE_HIGH;
            else if (!strcmp(v, "main")) o->profile = LX_VENC_H264_PROFILE_MAIN;
            else if (!strcmp(v, "baseline")) o->profile = LX_VENC_H264_PROFILE_BASELINE;
            else return -1;
        }
        else if (!strcmp(a, "--pool")) { NEED; o->pool = v; }
        else if (!strcmp(a, "--csv")) { NEED; o->csv = v; }
        else if (!strcmp(a, "--nbuf")) { NEED; o->nbuf = (uint32_t)atoi(v); }
        else if (!strcmp(a, "--active-file")) { NEED; o->active_file = v; }
        else if (!strcmp(a, "--cbr")) o->cbr = 1;
        else return -1;
#undef NEED
    }
    return 0;
}

/* per-run statistics */
struct stats {
    uint64_t frames, bytes, keyframes, refused, timeouts;
    double ioctl_sum, ioctl_max, hw_sum, hw_max, wait_sum, wait_max;
    double t0, cpu0;
    uint8_t *first_key;
    size_t first_key_len;
    FILE *csv;
};

static void stats_begin(struct stats *s, const char *csv)
{
    memset(s, 0, sizeof(*s));
    s->t0 = now_s();
    s->cpu0 = cpu_s();
    if (csv) {
        s->csv = fopen(csv, "w");
        if (s->csv)
            fputs("frame,key,bytes,ioctl_us,hw_ms,wait_us\n", s->csv);
    }
}

static void stats_frame(struct stats *s, const struct venc_frame *f, double ioctl_s, double wait_s)
{
    s->frames++;
    s->bytes += f->size;
    if (f->keyframe) {
        s->keyframes++;
        if (!s->first_key) {
            s->first_key = malloc(f->size);
            if (s->first_key) {
                memcpy(s->first_key, f->data, f->size);
                s->first_key_len = f->size;
            }
        }
    }
    s->ioctl_sum += ioctl_s;
    if (ioctl_s > s->ioctl_max) s->ioctl_max = ioctl_s;
    s->hw_sum += f->hw_ms;
    if (f->hw_ms > s->hw_max) s->hw_max = f->hw_ms;
    s->wait_sum += wait_s;
    if (wait_s > s->wait_max) s->wait_max = wait_s;
    if (s->csv)
        fprintf(s->csv, "%llu,%d,%u,%.0f,%u,%.0f\n", (unsigned long long)s->frames, f->keyframe, f->size, ioctl_s * 1e6, f->hw_ms, wait_s * 1e6);
}

static void stats_end(struct stats *s, const struct opts *o, const char *route, const char *extra)
{
    double wall = now_s() - s->t0, cpu = cpu_s() - s->cpu0;
    struct annexb_summary sum;
    memset(&sum, 0, sizeof(sum));
    sum.profile_idc = sum.level_idc = -1;
    if (s->first_key)
        annexb_summarize(s->first_key, s->first_key_len, &sum);
    double n = s->frames ? (double)s->frames : 1;
    printf("{\"route\":\"%s\",\"w\":%u,\"h\":%u,\"fps_asked\":%u,\"kbps_asked\":%u,\"gop\":%u,"
           "\"frames\":%llu,\"keyframes\":%llu,\"refused\":%llu,\"timeouts\":%llu,\"bytes\":%llu,"
           "\"wall_s\":%.3f,\"fps_got\":%.2f,\"kbps_got\":%.0f,\"cpu_pct\":%.1f,"
           "\"ioctl_ms_avg\":%.2f,\"ioctl_ms_max\":%.2f,\"hw_ms_avg\":%.2f,\"hw_ms_max\":%.1f,"
           "\"wait_ms_avg\":%.2f,\"wait_ms_max\":%.2f,"
           "\"first_key\":{\"bytes\":%zu,\"sps\":%u,\"pps\":%u,\"idr\":%u,\"profile_idc\":%d,\"level_idc\":%d}%s%s}\n",
           route, o->w, o->h, o->fps, o->kbps, o->gop, (unsigned long long)s->frames, (unsigned long long)s->keyframes,
           (unsigned long long)s->refused, (unsigned long long)s->timeouts, (unsigned long long)s->bytes, wall,
           wall > 0 ? s->frames / wall : 0.0, wall > 0 ? s->bytes * 8.0 / wall / 1000.0 : 0.0,
           wall > 0 ? 100.0 * cpu / wall : 0.0, 1e3 * s->ioctl_sum / n, 1e3 * s->ioctl_max, s->hw_sum / n, s->hw_max,
           1e3 * s->wait_sum / n, 1e3 * s->wait_max, s->first_key_len, sum.sps, sum.pps, sum.idr, sum.profile_idc,
           sum.level_idc, extra && *extra ? "," : "", extra ? extra : "");
    fflush(stdout);
    if (s->csv)
        fclose(s->csv);
    free(s->first_key);
}

static int write_all(int fd, const uint8_t *p, size_t n)
{
    while (n) {
        ssize_t w = write(fd, p, n);
        if (w < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        p += w;
        n -= (size_t)w;
    }
    return 0;
}

static int open_out(const char *path)
{
    if (!path) {
        fputs("-o OUT.h264 is required\n", stderr);
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (fd < 0)
        fprintf(stderr, "open %s: %s\n", path, strerror(errno));
    return fd;
}

static void venc_cfg_from(const struct opts *o, struct venc_cfg *c)
{
    memset(c, 0, sizeof(*c));
    c->width = o->w; c->height = o->h; c->fps = o->fps; c->gop = o->gop; c->kbps = o->kbps;
    c->fixed_qp = o->qp; c->profile = (uint32_t)o->profile; c->cbr = o->cbr;
}

/* ---- caps: read-only ---- */

static void stat_node(const char *path)
{
    struct stat st;
    if (stat(path, &st) < 0) {
        printf("  %-16s absent (%s)\n", path, strerror(errno));
        return;
    }
    printf("  %-16s %c%c%c%c%c%c%c%c%c uid %u gid %u major %u minor %u\n", path, S_ISCHR(st.st_mode) ? 'c' : '-',
           st.st_mode & S_IRUSR ? 'r' : '-', st.st_mode & S_IWUSR ? 'w' : '-', st.st_mode & S_IXUSR ? 'x' : '-',
           st.st_mode & S_IRGRP ? 'r' : '-', st.st_mode & S_IWGRP ? 'w' : '-', st.st_mode & S_IXGRP ? 'x' : '-',
           st.st_mode & S_IROTH ? 'r' : '-', st.st_mode & S_IWOTH ? 'w' : '-', (unsigned)st.st_uid, (unsigned)st.st_gid,
           (unsigned)major(st.st_rdev), (unsigned)minor(st.st_rdev));
}

static void cat_head(const char *path, int lines)
{
    FILE *f = fopen(path, "r");
    if (!f) {
        printf("  %s: %s\n", path, strerror(errno));
        return;
    }
    char line[256];
    while (lines-- > 0 && fgets(line, sizeof(line), f))
        printf("  | %s", line);
    fclose(f);
}

static int do_caps(void)
{
    puts("nodes:");
    stat_node("/dev/venc");
    stat_node("/dev/video24");
    stat_node("/dev/lg/venc0");
    stat_node("/dev/hma");
    stat_node("/dev/video60");
    puts("/proc/hma/status (pools):");
    cat_head("/proc/hma/status", 60);
    struct pools ps;
    unsigned n = pools_read(&ps, NULL);
    const struct pool *cpb = NULL, *vt0 = NULL, *vt1 = NULL;
    for (unsigned i = 0; i < n; i++) {
        if (!strcmp(ps.p[i].name, "cpb")) cpb = &ps.p[i];
        if (!strcmp(ps.p[i].name, "de-vt-m0")) vt0 = &ps.p[i];
        if (!strcmp(ps.p[i].name, "de-vt-m1")) vt1 = &ps.p[i];
    }
    printf("pools parsed: %u; cpb (the encoder's ES/AUI and photofb frames) %s; de-vt-m0 (tap buffers) %s; de-vt-m1 %s\n", n,
           cpb ? "present" : "ABSENT", vt0 ? "present" : "ABSENT", vt1 ? "present" : "ABSENT");
    return 0;
}

static int do_vtcaps(void)
{
    char err[256] = "";
    struct v4l2_ext_capture_capability_info cap;
    if (vtcap_capability(&sysops_real, &cap, err, sizeof(err)) < 0) {
        printf("video tap capability: %s\n", err);
        return 1;
    }
    printf("video tap: flags 0x%x (scale-up %d, scale-down %d, divide-framerate %d) up-limit %ux%u down-limit %ux%u "
           "buffers %u max %ux%u planes %u pixel_format %u\n",
           cap.flags, !!(cap.flags & V4L2_EXT_CAPTURE_CAP_SCALE_UP), !!(cap.flags & V4L2_EXT_CAPTURE_CAP_SCALE_DOWN),
           !!(cap.flags & V4L2_EXT_CAPTURE_CAP_DIVIDE_FRAMERATE), cap.scale_up_limit_w, cap.scale_up_limit_h,
           cap.scale_down_limit_w, cap.scale_down_limit_h, cap.num_video_frame_buffer, cap.max_res.w, cap.max_res.h,
           cap.num_plane, cap.pixel_format);
    return 0;
}

static void print_bufinfo(const struct venc *v)
{
    struct pools ps;
    unsigned n = pools_read(&ps, NULL);
    const struct pool *pe = n ? pools_find(&ps, v->buf.ui32ESBufPhysBase, v->buf.ui32ESBufSize) : NULL;
    const struct pool *pa = n ? pools_find(&ps, v->buf.ui32AUIBufPhysBase, v->buf.ui32AUIBufSize) : NULL;
    printf("venc: instance %d; es 0x%08x+0x%x (pool %s) aui 0x%08x+0x%x (pool %s) in 0x%08x+0x%x thumb 0x%08x+0x%x\n", v->id,
           v->buf.ui32ESBufPhysBase, v->buf.ui32ESBufSize, pe ? pe->name : (n ? "NONE" : "unknown"), v->buf.ui32AUIBufPhysBase,
           v->buf.ui32AUIBufSize, pa ? pa->name : (n ? "NONE" : "unknown"), v->buf.ui32InBufPhysBase, v->buf.ui32InBufSize,
           v->buf.ui32PhysicalAddress, v->buf.ui32Size);
}

/* ---- the small steps: gate, create, map, hma, tapopen ---- */

static int do_gate(void)
{
    char err[256] = "";
    struct venc v;
    if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc: %s\n", err);
        return 2;
    }
    puts("gate: /dev/venc opened and the KADP gate passed");
    venc_close(&v);
    puts("gate: closed");
    printf("{\"step\":\"gate\",\"ok\":true}\n");
    return 0;
}

/* create: the instance and its buffer map; map: the same plus the ES ring mapped and read */
static int do_create(const struct opts *o, int map)
{
    char err[256] = "";
    struct venc v;
    if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc: %s\n", err);
        return 2;
    }
    struct venc_cfg cfg;
    venc_cfg_from(o, &cfg);
    if (venc_create(&v, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc create: %s\n", err);
        venc_close(&v);
        return 4;
    }
    print_bufinfo(&v);
    if (map) {
        /* venc_create mapped the ring already; show that the mapping reads, and what it holds */
        const volatile uint8_t *p = v.es;
        unsigned nonzero = 0;
        for (size_t i = 0; i < 4096 && i < v.buf.ui32ESBufSize; i++)
            nonzero += p[i] != 0;
        printf("map: ES ring mapped at %p; first 16 bytes:", (const void *)v.es);
        for (size_t i = 0; i < 16; i++)
            printf(" %02x", p[i]);
        printf("; %u non-zero bytes in the first 4 KiB\n", nonzero);
    }
    venc_close(&v);
    puts("destroyed and closed");
    printf("{\"step\":\"%s\",\"ok\":true,\"id\":%d,\"es_size\":%u,\"aui_size\":%u}\n", map ? "map" : "create", v.id,
           v.buf.ui32ESBufSize, v.buf.ui32AUIBufSize);
    return 0;
}

static int do_hma(const struct opts *o)
{
    char err[256] = "";
    size_t frame_bytes = (size_t)o->w * o->h * 3 / 2;
    struct hma_buf b;
    if (hma_alloc(&b, &sysops_real, o->pool, frame_bytes, 4096, 0, err, sizeof(err)) < 0) {
        fprintf(stderr, "hma: %s\n", err);
        return 3;
    }
    struct pools ps;
    unsigned n = pools_read(&ps, NULL);
    const struct pool *pp = n ? pools_find(&ps, b.paddr, frame_bytes) : NULL;
    printf("hma: %zu bytes from %s at 0x%08x (pool %s), mapped at %p\n", frame_bytes, o->pool, b.paddr, pp ? pp->name : (n ? "NONE" : "unknown"),
           (void *)b.vaddr);
    if (n && !pp) {
        fprintf(stderr, "hma: the address lies in no listed pool; stopping\n");
        hma_free(&b);
        return 5;
    }
    synth_nv12(b.vaddr, b.vaddr + (size_t)o->w * o->h, o->w, o->h, 0x1234);
    uint32_t back = synth_read_counter(b.vaddr, o->w, o->h);
    printf("hma: pattern written and read back: counter 0x%04x (expected 0x1234)\n", back);
    hma_free(&b);
    if (back != 0x1234)
        return 6;
    printf("{\"step\":\"hma\",\"ok\":true,\"paddr\":%u,\"bytes\":%zu}\n", b.paddr, frame_bytes);
    return 0;
}

static int do_tapopen(const struct opts *o)
{
    char err[256] = "";
    enum v4l2_ext_capture_location loc;
    if (vtcap_location_parse(o->location, &loc) < 0) {
        fprintf(stderr, "unknown location %s\n", o->location);
        return 1;
    }
    struct vtcap cap;
    if (vtcap_open(&cap, &sysops_real, loc, o->w, o->h, o->nbuf, o->divide, err, sizeof(err)) < 0) {
        fprintf(stderr, "tap: %s\n", err);
        return 3;
    }
    printf("tap: %s %ux%u stride %u, %u buffers queued, no streaming:\n", o->location, o->w, o->h, cap.stride, cap.nqueued);
    for (uint32_t i = 0; i < o->nbuf; i++)
        printf("  buffer %u: y 0x%08x (%u bytes) c 0x%08x (%u bytes) pool %s\n", i, cap.paddr_y[i], cap.plane_len[i][0], cap.paddr_c[i],
               cap.plane_len[i][1], cap.pool[i]);
    vtcap_close(&cap);
    puts("tap: buffers released and closed");
    printf("{\"step\":\"tapopen\",\"ok\":true,\"location\":\"%s\",\"stride\":%u}\n", o->location, cap.stride);
    return 0;
}

/* ---- synth: CPU frames through /dev/venc ---- */

static int do_synth(const struct opts *o)
{
    char err[256] = "";
    int out = open_out(o->out);
    if (out < 0)
        return 1;
    struct venc v;
    if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc: %s\n", err);
        return 2;
    }
    size_t frame_bytes = (size_t)o->w * o->h * 3 / 2;
    struct hma_buf in[2];
    for (int i = 0; i < 2; i++)
        if (hma_alloc(&in[i], &sysops_real, o->pool, frame_bytes, 4096, 0, err, sizeof(err)) < 0) {
            fprintf(stderr, "hma: %s\n", err);
            if (i) hma_free(&in[0]);
            venc_close(&v);
            return 3;
        }
    printf("hma: pool %s, two frames of %zu bytes at 0x%08x and 0x%08x\n", o->pool, frame_bytes, in[0].paddr, in[1].paddr);
    struct venc_cfg cfg;
    venc_cfg_from(o, &cfg);
    if (venc_create(&v, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc create: %s\n", err);
        hma_free(&in[0]); hma_free(&in[1]); venc_close(&v);
        return 4;
    }
    print_bufinfo(&v);
    printf("venc: instance %d, %ux%u %u fps %u kbps gop %u\n", v.id, o->w, o->h, o->fps, o->kbps, o->gop);
    struct stats st;
    stats_begin(&st, o->csv);
    double period = 1.0 / o->fps, next = now_s();
    int rc = 0;
    for (uint32_t i = 0; i < o->frames && !stop_now; i++) {
        struct hma_buf *b = &in[i & 1];
        double tf = now_s();
        synth_nv12(b->vaddr, b->vaddr + (size_t)o->w * o->h, o->w, o->h, i);
        hma_cache(b, HMA_CACHE_CLEAN, 0, frame_bytes);
        double fill = now_s() - tf;
        struct venc_frame f;
        double t1 = now_s();
        if (venc_encode(&v, b->paddr, b->paddr + o->w * o->h, &f, err, sizeof(err)) < 0) {
            fprintf(stderr, "frame %u: %s\n", i, err);
            rc = 5;
            break;
        }
        double t2 = now_s();
        stats_frame(&st, &f, t2 - t1, fill);
        if (write_all(out, f.data, f.size) < 0) {
            rc = 6;
            break;
        }
        if (v.slow >= SLOW_LIMIT) {
            fprintf(stderr, "%u encodes took longer than %d ms; stopping\n", v.slow, VENC_SLOW_MS);
            rc = 7;
            break;
        }
        next += period;
        double sleep_s = next - now_s();
        if (sleep_s > 0)
            usleep((useconds_t)(sleep_s * 1e6));
    }
    stats_end(&st, o, "synth:/dev/venc", "\"wait_is\":\"fill_ms\"");
    hma_free(&in[0]);
    hma_free(&in[1]);
    venc_close(&v);
    close(out);
    return rc;
}

/* ---- vt and serve: the screen through the tap into the encoder ---- */

struct sink {
    int fd;          /* file or socket */
    int is_socket;
};

static int sink_write(struct sink *s, const uint8_t *p, size_t n)
{
    return write_all(s->fd, p, n);
}

static int mark_active(const char *path, const struct opts *o, const char *peer)
{
    if (!path)
        return 0;
    char dir[256];
    snprintf(dir, sizeof(dir), "%s", path);
    char *slash = strrchr(dir, '/');
    if (slash && slash != dir) {
        *slash = 0;
        mkdir(dir, 0755);
    }
    FILE *f = fopen(path, "w");
    if (!f)
        return -1;
    fprintf(f, "{\"what\":\"screen-encode\",\"location\":\"%s\",\"w\":%u,\"h\":%u,\"peer\":\"%s\",\"pid\":%d}\n", o->location, o->w, o->h, peer, (int)getpid());
    fclose(f);
    return 0;
}

static int run_capture(const struct opts *o, struct sink *sink, uint32_t max_frames, struct stats *st)
{
    char err[256] = "";
    enum v4l2_ext_capture_location loc;
    if (vtcap_location_parse(o->location, &loc) < 0) {
        fprintf(stderr, "unknown location %s\n", o->location);
        return 1;
    }
    struct venc v;
    if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc: %s\n", err);
        return 2;
    }
    struct vtcap cap;
    if (vtcap_open(&cap, &sysops_real, loc, o->w, o->h, o->nbuf, o->divide, err, sizeof(err)) < 0) {
        fprintf(stderr, "tap: %s\n", err);
        venc_close(&v);
        return 3;
    }
    printf("tap: %s %ux%u stride %u, %u buffers at", o->location, o->w, o->h, cap.stride, o->nbuf);
    for (uint32_t i = 0; i < o->nbuf; i++)
        printf(" 0x%08x/0x%08x(%s)", cap.paddr_y[i], cap.paddr_c[i], cap.pool[i]);
    putchar('\n');
    if (cap.stride && cap.stride != o->w) {
        fprintf(stderr, "tap stride %u differs from width %u; the encoder assumes stride == width; stopping\n", cap.stride, o->w);
        vtcap_close(&cap);
        venc_close(&v);
        return 3;
    }
    struct venc_cfg cfg;
    venc_cfg_from(o, &cfg);
    if (venc_create(&v, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc create: %s\n", err);
        vtcap_close(&cap);
        venc_close(&v);
        return 4;
    }
    print_bufinfo(&v);
    if (vtcap_start(&cap, err, sizeof(err)) < 0) {
        fprintf(stderr, "tap: %s\n", err);
        vtcap_close(&cap);
        venc_close(&v);
        return 3;
    }
    int rc = 0;
    uint32_t n = 0, idle = 0;
    while (!stop_now && (max_frames == 0 || n < max_frames)) {
        struct vtcap_frame fr;
        double t0 = now_s();
        int r = vtcap_dequeue(&cap, &fr, 1000, err, sizeof(err));
        double wait = now_s() - t0;
        if (r < 0) {
            fprintf(stderr, "tap: %s\n", err);
            rc = 5;
            break;
        }
        if (r == 1) {
            st->timeouts++;
            if (++idle >= 10) {
                fprintf(stderr, "tap: no frame for 10 s\n");
                rc = 6;
                break;
            }
            continue;
        }
        if (r == 2) {
            st->refused++;
            usleep(100000);
            continue;
        }
        idle = 0;
        struct venc_frame f;
        double t1 = now_s();
        int er = venc_encode(&v, fr.paddr_y, fr.paddr_c, &f, err, sizeof(err));
        double t2 = now_s();
        if (vtcap_requeue(&cap, fr.index, err, sizeof(err)) < 0) {
            fprintf(stderr, "tap: %s\n", err);
            rc = 7;
            break;
        }
        if (er < 0) {
            fprintf(stderr, "frame %u: %s\n", n, err);
            rc = 8;
            break;
        }
        n++;
        stats_frame(st, &f, t2 - t1, wait);
        if (v.slow >= SLOW_LIMIT) {
            fprintf(stderr, "%u encodes took longer than %d ms; stopping\n", v.slow, VENC_SLOW_MS);
            rc = 10;
            break;
        }
        if (sink_write(sink, f.data, f.size) < 0) {
            if (sink->is_socket) {
                fprintf(stderr, "client gone\n");
                rc = 0;
            } else {
                rc = 9;
            }
            break;
        }
    }
    vtcap_close(&cap);
    venc_close(&v);
    return rc;
}

static int do_vt(const struct opts *o)
{
    struct sink s = { .fd = open_out(o->out), .is_socket = 0 };
    if (s.fd < 0)
        return 1;
    struct stats st;
    stats_begin(&st, o->csv);
    int rc = run_capture(o, &s, o->frames, &st);
    char extra[64];
    snprintf(extra, sizeof(extra), "\"location\":\"%s\",\"wait_is\":\"tap_wait_ms\"", o->location);
    stats_end(&st, o, "vt:/dev/video60->/dev/venc", extra);
    close(s.fd);
    return rc;
}

static int do_serve(const struct opts *o)
{
    int ls = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (ls < 0) {
        perror("socket");
        return 1;
    }
    int one = 1;
    setsockopt(ls, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((uint16_t)o->port);
    if (bind(ls, (struct sockaddr *)&addr, sizeof(addr)) < 0 || listen(ls, 1) < 0) {
        perror("bind/listen");
        return 1;
    }
    printf("serve: H.264 Annex B of the %s tap at %ux%u on port %u; one client at a time; stop with SIGTERM\n",
           o->location, o->w, o->h, o->port);
    fflush(stdout);
    int rc = 0;
    while (!stop_now) {
        struct pollfd pfd = { .fd = ls, .events = POLLIN };
        if (poll(&pfd, 1, 500) <= 0)
            continue;
        struct sockaddr_in peer;
        socklen_t plen = sizeof(peer);
        int cfd = accept4(ls, (struct sockaddr *)&peer, &plen, SOCK_CLOEXEC);
        if (cfd < 0)
            continue;
        setsockopt(cfd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
        char peername[64];
        snprintf(peername, sizeof(peername), "%s:%u", inet_ntoa(peer.sin_addr), ntohs(peer.sin_port));
        printf("serve: client %s\n", peername);
        fflush(stdout);
        mark_active(o->active_file, o, peername);
        struct sink s = { .fd = cfd, .is_socket = 1 };
        struct stats st;
        stats_begin(&st, NULL);
        signal(SIGPIPE, SIG_IGN);
        rc = run_capture(o, &s, 0, &st);
        char extra[96];
        snprintf(extra, sizeof(extra), "\"location\":\"%s\",\"peer\":\"%s\",\"wait_is\":\"tap_wait_ms\"", o->location, peername);
        stats_end(&st, o, "serve:/dev/video60->/dev/venc->tcp", extra);
        close(cfd);
        if (o->active_file)
            unlink(o->active_file);
        if (rc && rc != 9)
            break;
    }
    close(ls);
    if (o->active_file)
        unlink(o->active_file);
    return rc;
}

/* ---- feed: the compositor's screen recorder (feed_proto.h): NV12 frames from stdin through /dev/venc, access units to stdout ---- */

static uint32_t get32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }
static uint64_t get64(const uint8_t *p) { return (uint64_t)get32(p) | (uint64_t)get32(p + 4) << 32; }
static void put32(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); p[2] = (uint8_t)(v >> 16); p[3] = (uint8_t)(v >> 24); }
static void put64(uint8_t *p, uint64_t v) { put32(p, (uint32_t)v); put32(p + 4, (uint32_t)(v >> 32)); }

/* Reads exactly n bytes: 1 when they are all there, 0 on a clean end of input before the first of them, -1 on an error, a short
 * record or a stop signal. */
static int read_fully(int fd, uint8_t *p, size_t n)
{
    size_t got = 0;
    while (got < n) {
        ssize_t r = read(fd, p + got, n - got);
        if (r < 0) {
            if (errno == EINTR && !stop_now)
                continue;
            return -1;
        }
        if (r == 0)
            return got ? -1 : 0;
        got += (size_t)r;
    }
    return 1;
}

/* The order is the one the MCU insists on (venc.h): open, the frame's HMA buffer, create (which allocates the ES ring), then frames. */
static int do_feed(const struct opts *o)
{
    char err[256] = "";
    /* stdout is the protocol's output; anything else this program prints goes to stderr (the recorder's log) */
    int out = dup(1);
    if (out < 0 || dup2(2, 1) < 0) {
        perror("dup");
        return 1;
    }
    /* SIGTERM must interrupt a read that waits for the next frame, and a recorder that went away must not kill the program before the
     * hardware is closed */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_signal;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);
    struct venc v;
    if (venc_open(&v, &sysops_real, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc: %s\n", err);
        return 2;
    }
    size_t frame_bytes = (size_t)o->w * o->h * 3 / 2;
    struct hma_buf in;
    if (hma_alloc(&in, &sysops_real, o->pool, frame_bytes, 4096, 0, err, sizeof(err)) < 0) {
        fprintf(stderr, "hma: %s\n", err);
        venc_close(&v);
        return 3;
    }
    struct venc_cfg cfg;
    venc_cfg_from(o, &cfg);
    if (venc_create(&v, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc create: %s\n", err);
        hma_free(&in);
        venc_close(&v);
        return 4;
    }
    print_bufinfo(&v);
    fprintf(stderr, "feed: %ux%u %u fps %u kbps gop %u, frame in pool %s at 0x%08x\n", o->w, o->h, o->fps, o->kbps, o->gop, o->pool, in.paddr);
    uint8_t head[FEED_HEADER_LEN];
    memset(head, 0, sizeof(head));
    memcpy(head, FEED_MAGIC_READY, 4);
    put32(head + 4, FEED_VERSION);
    put32(head + 8, o->w);
    put32(head + 12, o->h);
    int rc = write_all(out, head, sizeof(head)) < 0 ? 8 : 0;
    struct stats st;
    stats_begin(&st, o->csv);
    while (!rc && !stop_now) {
        int r = read_fully(0, head, sizeof(head));
        if (r == 0)
            break; /* the recorder closed its end: the recording is over */
        if (r < 0) {
            rc = stop_now ? 0 : 5;
            break;
        }
        if (memcmp(head, FEED_MAGIC_FRAME, 4) != 0 || get32(head + 16) != frame_bytes) {
            fprintf(stderr, "feed: a frame header that is not one (size %u, %zu expected)\n", get32(head + 16), frame_bytes);
            rc = 6;
            break;
        }
        uint32_t flags = get32(head + 4);
        uint64_t pts = get64(head + 8);
        double tw = now_s();
        if (read_fully(0, in.vaddr, frame_bytes) != 1) {
            rc = stop_now ? 0 : 5;
            break;
        }
        hma_cache(&in, HMA_CACHE_CLEAN, 0, frame_bytes);
        double wait = now_s() - tw;
        /* the first frame is an IDR whatever is asked; a later one only when the recorder asks */
        if ((flags & FEED_FLAG_KEY) && v.frames > 0 && venc_force_idr(&v, err, sizeof(err)) < 0) {
            fprintf(stderr, "idr: %s\n", err);
            rc = 9;
            break;
        }
        struct venc_frame f;
        double t1 = now_s();
        if (venc_encode(&v, in.paddr, in.paddr + (uint32_t)(o->w * o->h), &f, err, sizeof(err)) < 0) {
            fprintf(stderr, "frame: %s\n", err);
            rc = 7;
            break;
        }
        stats_frame(&st, &f, now_s() - t1, wait);
        uint8_t unit[FEED_HEADER_LEN];
        memset(unit, 0, sizeof(unit));
        memcpy(unit, FEED_MAGIC_UNIT, 4);
        put32(unit + 4, f.keyframe ? FEED_FLAG_KEY : 0);
        put64(unit + 8, pts);
        put32(unit + 16, f.size);
        put32(unit + 20, f.hw_ms);
        if (write_all(out, unit, sizeof(unit)) < 0 || write_all(out, f.data, f.size) < 0) {
            rc = 8;
            break;
        }
        /* the encoder alone shares the video MCU with the decoder: three slow encodes in the run end it, as in every other mode */
        if (v.slow >= SLOW_LIMIT) {
            fprintf(stderr, "%u encodes took longer than %d ms; stopping\n", v.slow, VENC_SLOW_MS);
            rc = 10;
            break;
        }
    }
    stats_end(&st, o, "feed:pipe->/dev/venc->pipe", "\"wait_is\":\"read_ms\"");
    hma_free(&in);
    venc_close(&v);
    close(out);
    return rc;
}

int main(int argc, char **argv)
{
    struct opts o;
    if (parse(argc, argv, &o) < 0) {
        usage();
        return 64;
    }
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);
    setvbuf(stdout, NULL, _IOLBF, 0);
    wd_install();
    if (!strcmp(o.mode, "caps")) return do_caps();
    if (!strcmp(o.mode, "vtcaps")) return do_vtcaps();
    if (!strcmp(o.mode, "gate")) return do_gate();
    if (!strcmp(o.mode, "create")) return do_create(&o, 0);
    if (!strcmp(o.mode, "map")) return do_create(&o, 1);
    if (!strcmp(o.mode, "hma")) return do_hma(&o);
    if (!strcmp(o.mode, "synth")) return do_synth(&o);
    if (!strcmp(o.mode, "tapopen")) return do_tapopen(&o);
    if (!strcmp(o.mode, "vt")) return do_vt(&o);
    if (!strcmp(o.mode, "serve")) return do_serve(&o);
    if (!strcmp(o.mode, "feed")) return do_feed(&o);
    usage();
    return 64;
}
