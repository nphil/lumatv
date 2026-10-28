/* lumatv-venc: the C4 hardware-encoder probe and the first streaming path.
 *
 *   lumatv-venc caps                     what the encoder nodes say about themselves (read-only ioctls on /dev/video24)
 *   lumatv-venc vtcaps                   the video tap's capability record (opens /dev/video60 once, no capture)
 *   lumatv-venc synth [opts] -o out.h264 a moving test picture from CPU memory through /dev/venc (the encoder alone)
 *   lumatv-venc m2m   [opts] -o out.h264 the same through the V4L2 M2M node /dev/video24 (the fallback route)
 *   lumatv-venc vt    [opts] -o out.h264 the screen: video tap -> encoder, no copy (the chosen route)
 *   lumatv-venc serve [opts] -p PORT     the screen as an H.264 byte stream to one TCP client at a time
 *
 * Options: -w WIDTH -h HEIGHT (default 1280x720) -r FPS (30) -b KBPS (4000; 0 = fixed QP -q) -g GOP (30)
 *          -n FRAMES (300) -l blended|osd|display|scaler-output (blended) -d DIVIDE (1) -P profile high|main|baseline
 *          --pool NAME (photofb) --csv FILE --cbr --nbuf N (4) --active-file PATH (serve)
 * Every run prints one JSON line with the numbers at the end. Nothing is written outside the paths given.
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
#include "hma.h"
#include "synth.h"
#include "sysops.h"
#include "venc.h"
#include "vtcap.h"

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
    fputs("usage: lumatv-venc caps|vtcaps|synth|m2m|vt|serve [-w W] [-h H] [-r FPS] [-b KBPS] [-q QP] [-g GOP] [-n FRAMES]\n"
          "       [-l blended|osd|display|scaler-output] [-d DIVIDE] [-P high|main|baseline] [--pool NAME] [--cbr]\n"
          "       [--nbuf N] [--csv FILE] [--active-file PATH] [-p PORT] -o OUT.h264\n",
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
    puts("/proc/hma/status (pools and free space):");
    cat_head("/proc/hma/status", 40);
    puts("/dev/video24 (sic-encoder, V4L2):");
    int fd = open("/dev/video24", O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        printf("  open: %s\n", strerror(errno));
        return 1;
    }
    struct v4l2_capability cap;
    memset(&cap, 0, sizeof(cap));
    if (ioctl(fd, VIDIOC_QUERYCAP, &cap) == 0)
        printf("  driver %s card '%s' bus %s caps 0x%08x device_caps 0x%08x\n", cap.driver, cap.card, cap.bus_info,
               cap.capabilities, cap.device_caps);
    else
        printf("  QUERYCAP: %s\n", strerror(errno));
    for (int type = 0; type < 2; type++) {
        struct v4l2_fmtdesc fd_;
        for (uint32_t i = 0;; i++) {
            memset(&fd_, 0, sizeof(fd_));
            fd_.index = i;
            fd_.type = type ? V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE : V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
            if (ioctl(fd, VIDIOC_ENUM_FMT, &fd_) < 0)
                break;
            char fcc[5] = { (char)(fd_.pixelformat & 0xff), (char)(fd_.pixelformat >> 8), (char)(fd_.pixelformat >> 16), (char)(fd_.pixelformat >> 24), 0 };
            printf("  %s format %u: %s\n", type ? "cap (ES) " : "out (YUV)", i, fcc);
            if (type) {
                struct v4l2_frmsizeenum fs;
                memset(&fs, 0, sizeof(fs));
                fs.pixel_format = fd_.pixelformat;
                if (ioctl(fd, VIDIOC_ENUM_FRAMESIZES, &fs) == 0 && fs.type == V4L2_FRMSIZE_TYPE_STEPWISE)
                    printf("    sizes %ux%u .. %ux%u step %ux%u\n", fs.stepwise.min_width, fs.stepwise.min_height,
                           fs.stepwise.max_width, fs.stepwise.max_height, fs.stepwise.step_width, fs.stepwise.step_height);
            }
        }
    }
    static const struct { uint32_t id; const char *name; } ctrls[] = {
        { V4L2_CID_MPEG_VIDEO_BITRATE, "bitrate" }, { V4L2_CID_MPEG_VIDEO_GOP_SIZE, "gop" },
        { V4L2_CID_MPEG_VIDEO_H264_PROFILE, "h264_profile" }, { V4L2_CID_MPEG_VIDEO_H264_LEVEL, "h264_level" },
        { V4L2_CID_MPEG_VIDEO_FORCE_KEY_FRAME, "force_key_frame" },
    };
    for (unsigned i = 0; i < sizeof(ctrls) / sizeof(ctrls[0]); i++) {
        struct v4l2_queryctrl q;
        memset(&q, 0, sizeof(q));
        q.id = ctrls[i].id;
        if (ioctl(fd, VIDIOC_QUERYCTRL, &q) == 0)
            printf("  control %-16s min %d max %d step %d default %d\n", ctrls[i].name, q.minimum, q.maximum, q.step, q.default_value);
        else
            printf("  control %-16s %s\n", ctrls[i].name, strerror(errno));
    }
    close(fd);
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
    printf("venc: es 0x%08x+0x%x aui 0x%08x+0x%x in 0x%08x+0x%x\n", v.buf.ui32ESBufPhysBase, v.buf.ui32ESBufSize,
           v.buf.ui32AUIBufPhysBase, v.buf.ui32AUIBufSize, v.buf.ui32InBufPhysBase, v.buf.ui32InBufSize);
    size_t frame_bytes = (size_t)o->w * o->h * 3 / 2;
    struct hma_buf in[2];
    for (int i = 0; i < 2; i++)
        if (hma_alloc(&in[i], &sysops_real, o->pool, frame_bytes, 4096, 0, err, sizeof(err)) < 0) {
            fprintf(stderr, "hma: %s\n", err);
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

/* ---- m2m: CPU frames through /dev/video24 (standard V4L2, the fallback) ---- */

static int set_ctrl(int fd, uint32_t id, int32_t val)
{
    struct v4l2_control c = { .id = id, .value = val };
    return ioctl(fd, VIDIOC_S_CTRL, &c);
}

static int do_m2m(const struct opts *o)
{
    int out = open_out(o->out);
    if (out < 0)
        return 1;
    int fd = open("/dev/video24", O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        fprintf(stderr, "open /dev/video24: %s\n", strerror(errno));
        return 2;
    }
    struct v4l2_format f;
    memset(&f, 0, sizeof(f));
    f.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    f.fmt.pix_mp.width = o->w;
    f.fmt.pix_mp.height = o->h;
    f.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12M;
    f.fmt.pix_mp.num_planes = 2;
    if (ioctl(fd, VIDIOC_S_FMT, &f) < 0) {
        fprintf(stderr, "S_FMT out: %s\n", strerror(errno));
        return 3;
    }
    uint32_t ysize = f.fmt.pix_mp.plane_fmt[0].sizeimage, csize = f.fmt.pix_mp.plane_fmt[1].sizeimage;
    memset(&f, 0, sizeof(f));
    f.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    f.fmt.pix_mp.width = o->w;
    f.fmt.pix_mp.height = o->h;
    f.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_H264;
    f.fmt.pix_mp.num_planes = 1;
    f.fmt.pix_mp.plane_fmt[0].sizeimage = o->w * o->h;
    if (ioctl(fd, VIDIOC_S_FMT, &f) < 0) {
        fprintf(stderr, "S_FMT cap: %s\n", strerror(errno));
        return 3;
    }
    uint32_t essize = f.fmt.pix_mp.plane_fmt[0].sizeimage;
    struct v4l2_streamparm parm;
    memset(&parm, 0, sizeof(parm));
    parm.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    parm.parm.output.timeperframe.numerator = 1;
    parm.parm.output.timeperframe.denominator = o->fps;
    if (ioctl(fd, VIDIOC_S_PARM, &parm) < 0)
        fprintf(stderr, "S_PARM: %s\n", strerror(errno));
    if (set_ctrl(fd, V4L2_CID_MPEG_VIDEO_BITRATE, (int32_t)(o->kbps * 1000)) < 0)
        fprintf(stderr, "S_CTRL bitrate: %s\n", strerror(errno));
    if (set_ctrl(fd, V4L2_CID_MPEG_VIDEO_GOP_SIZE, (int32_t)o->gop) < 0)
        fprintf(stderr, "S_CTRL gop: %s\n", strerror(errno));
    if (set_ctrl(fd, V4L2_CID_MPEG_VIDEO_H264_PROFILE, o->profile == LX_VENC_H264_PROFILE_BASELINE ? V4L2_MPEG_VIDEO_H264_PROFILE_BASELINE
                                                        : o->profile == LX_VENC_H264_PROFILE_MAIN ? V4L2_MPEG_VIDEO_H264_PROFILE_MAIN
                                                                                                   : V4L2_MPEG_VIDEO_H264_PROFILE_HIGH) < 0)
        fprintf(stderr, "S_CTRL profile: %s\n", strerror(errno));
    enum { NB = 2 };
    uint8_t *outp[NB][2], *capp[NB];
    uint32_t outlen[NB][2], caplen[NB];
    for (int type = 0; type < 2; type++) {
        struct v4l2_requestbuffers req;
        memset(&req, 0, sizeof(req));
        req.count = NB;
        req.type = type ? V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE : V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        req.memory = V4L2_MEMORY_MMAP;
        if (ioctl(fd, VIDIOC_REQBUFS, &req) < 0) {
            fprintf(stderr, "REQBUFS %s: %s\n", type ? "cap" : "out", strerror(errno));
            return 4;
        }
        for (int i = 0; i < NB; i++) {
            struct v4l2_buffer b;
            struct v4l2_plane pl[2];
            memset(&b, 0, sizeof(b));
            memset(pl, 0, sizeof(pl));
            b.type = req.type;
            b.memory = V4L2_MEMORY_MMAP;
            b.index = (uint32_t)i;
            b.length = type ? 1 : 2;
            b.m.planes = pl;
            if (ioctl(fd, VIDIOC_QUERYBUF, &b) < 0) {
                fprintf(stderr, "QUERYBUF: %s\n", strerror(errno));
                return 4;
            }
            for (uint32_t p = 0; p < b.length; p++) {
                void *m = mmap(NULL, pl[p].length, type ? PROT_READ : PROT_WRITE, MAP_SHARED, fd, (off_t)pl[p].m.mem_offset);
                if (m == MAP_FAILED) {
                    fprintf(stderr, "mmap %s[%d][%u] (offset 0x%x len %u): %s\n", type ? "cap" : "out", i, p, pl[p].m.mem_offset, pl[p].length, strerror(errno));
                    return 5;
                }
                if (type) { capp[i] = m; caplen[i] = pl[p].length; }
                else { outp[i][p] = m; outlen[i][p] = pl[p].length; }
            }
        }
    }
    printf("m2m: out planes %u+%u bytes, es buffers %u bytes\n", ysize, csize, essize);
    for (int t = 0; t < 2; t++) {
        int type = t ? V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE : V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        if (ioctl(fd, VIDIOC_STREAMON, &type) < 0) {
            fprintf(stderr, "STREAMON %d: %s\n", type, strerror(errno));
            return 6;
        }
    }
    struct stats st;
    stats_begin(&st, o->csv);
    int rc = 0;
    for (uint32_t i = 0; i < o->frames && !stop_now; i++) {
        int bi = (int)(i % NB);
        double tf = now_s();
        synth_nv12(outp[bi][0], outp[bi][1], o->w, o->h, i);
        double fill = now_s() - tf;
        struct v4l2_buffer b;
        struct v4l2_plane pl[2];
        memset(&b, 0, sizeof(b));
        memset(pl, 0, sizeof(pl));
        b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.index = (uint32_t)bi;
        b.length = 2;
        b.m.planes = pl;
        pl[0].bytesused = ysize;
        pl[1].bytesused = csize;
        gettimeofday(&b.timestamp, NULL);
        double t1 = now_s();
        if (ioctl(fd, VIDIOC_QBUF, &b) < 0) { fprintf(stderr, "QBUF out: %s\n", strerror(errno)); rc = 7; break; }
        memset(&b, 0, sizeof(b));
        memset(pl, 0, sizeof(pl));
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.index = (uint32_t)bi;
        b.length = 1;
        b.m.planes = pl;
        if (ioctl(fd, VIDIOC_QBUF, &b) < 0) { fprintf(stderr, "QBUF cap: %s\n", strerror(errno)); rc = 7; break; }
        struct pollfd pfd = { .fd = fd, .events = POLLIN };
        int pr = poll(&pfd, 1, 2000);
        if (pr <= 0) { fprintf(stderr, "frame %u: no result within 2 s\n", i); rc = 8; break; }
        memset(&b, 0, sizeof(b));
        memset(pl, 0, sizeof(pl));
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.length = 1;
        b.m.planes = pl;
        if (ioctl(fd, VIDIOC_DQBUF, &b) < 0) { fprintf(stderr, "DQBUF cap: %s\n", strerror(errno)); rc = 9; break; }
        double t2 = now_s();
        struct venc_frame vf = { .data = capp[b.index], .size = pl[0].bytesused, .keyframe = !!(b.flags & V4L2_BUF_FLAG_KEYFRAME), .hw_ms = 0 };
        stats_frame(&st, &vf, t2 - t1, fill);
        if (write_all(out, vf.data, vf.size) < 0) { rc = 10; break; }
        memset(&b, 0, sizeof(b));
        memset(pl, 0, sizeof(pl));
        b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.length = 2;
        b.m.planes = pl;
        if (ioctl(fd, VIDIOC_DQBUF, &b) < 0) { fprintf(stderr, "DQBUF out: %s\n", strerror(errno)); rc = 9; break; }
    }
    stats_end(&st, o, "m2m:/dev/video24", "\"wait_is\":\"fill_ms\"");
    for (int t = 0; t < 2; t++) {
        int type = t ? V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE : V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        ioctl(fd, VIDIOC_STREAMOFF, &type);
    }
    for (int i = 0; i < NB; i++) {
        munmap(outp[i][0], outlen[i][0]);
        munmap(outp[i][1], outlen[i][1]);
        munmap(capp[i], caplen[i]);
    }
    close(fd);
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
        printf(" 0x%08x/0x%08x", cap.paddr_y[i], cap.paddr_c[i]);
    putchar('\n');
    if (cap.stride && cap.stride != o->w)
        fprintf(stderr, "warning: tap stride %u differs from width %u; the encoder assumes stride == width\n", cap.stride, o->w);
    struct venc_cfg cfg;
    venc_cfg_from(o, &cfg);
    if (venc_create(&v, &cfg, err, sizeof(err)) < 0) {
        fprintf(stderr, "venc create: %s\n", err);
        vtcap_close(&cap);
        venc_close(&v);
        return 4;
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
    if (!strcmp(o.mode, "caps")) return do_caps();
    if (!strcmp(o.mode, "vtcaps")) return do_vtcaps();
    if (!strcmp(o.mode, "synth")) return do_synth(&o);
    if (!strcmp(o.mode, "m2m")) return do_m2m(&o);
    if (!strcmp(o.mode, "vt")) return do_vt(&o);
    if (!strcmp(o.mode, "serve")) return do_serve(&o);
    usage();
    return 64;
}
