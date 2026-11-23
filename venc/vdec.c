#include "vdec.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include "watchdog.h"

static double now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e3 + (double)ts.tv_nsec / 1e6;
}

static int vioc(struct vdec *d, unsigned long req, void *arg, const char *what, unsigned budget_ms)
{
    wd_arm(what, budget_ms);
    int r = d->sys->ioctl(d->fd, req, arg);
    wd_disarm();
    return r;
}

static const char *fourcc_str(uint32_t f, char out[5])
{
    for (int i = 0; i < 4; i++) {
        unsigned char c = (unsigned char)(f >> (8 * i));
        out[i] = c >= 32 && c < 127 ? (char)c : '?';
    }
    out[4] = 0;
    return out;
}

int vdec_open(struct vdec *d, const struct sysops *sys, char *err, size_t errlen)
{
    memset(d, 0, sizeof(*d));
    d->sys = sys;
    d->fd = -1;
    for (int i = 0; i < VDEC_MAX_CAP; i++)
        d->cap[i].dmabuf = -1;
    wd_arm("open " VDEC_DEV, WD_V4L2_MS);
    d->fd = sys->open(VDEC_DEV, O_RDWR | O_NONBLOCK);
    wd_disarm();
    if (d->fd < 0)
        return sys_fail(err, errlen, "open " VDEC_DEV);
    struct v4l2_capability cap;
    memset(&cap, 0, sizeof(cap));
    if (vioc(d, VIDIOC_QUERYCAP, &cap, "VIDIOC_QUERYCAP", WD_V4L2_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_QUERYCAP");
        vdec_close(d);
        return -1;
    }
    uint32_t caps = (cap.capabilities & V4L2_CAP_DEVICE_CAPS) ? cap.device_caps : cap.capabilities;
    int named = strstr((const char *)cap.card, "Decoder") || strstr((const char *)cap.driver, "vdec");
    if (!named || !(caps & V4L2_CAP_VIDEO_M2M_MPLANE) || !(caps & V4L2_CAP_STREAMING)) {
        errno = ENODEV;
        snprintf(err, errlen, VDEC_DEV " is not the video decoder (driver '%.16s' card '%.32s' caps 0x%x)", cap.driver, cap.card, caps);
        vdec_close(d);
        return -1;
    }
    return 0;
}

/* ---- the read-only report ---- */

static void report_formats(struct vdec *d, FILE *out, enum v4l2_buf_type type, const char *name)
{
    for (uint32_t i = 0; i < 32; i++) {
        struct v4l2_fmtdesc f;
        memset(&f, 0, sizeof(f));
        f.index = i;
        f.type = type;
        if (vioc(d, VIDIOC_ENUM_FMT, &f, "VIDIOC_ENUM_FMT", WD_V4L2_MS) < 0) {
            if (errno != EINVAL)
                fprintf(out, "vdec: ENUM_FMT %s %u: %s\n", name, i, strerror(errno));
            break;
        }
        char cc[5];
        fprintf(out, "vdec: %s format %u '%s' flags 0x%x '%s'\n", name, i, fourcc_str(f.pixelformat, cc), f.flags, f.description);
        struct v4l2_frmsizeenum s;
        memset(&s, 0, sizeof(s));
        s.pixel_format = f.pixelformat;
        if (vioc(d, VIDIOC_ENUM_FRAMESIZES, &s, "VIDIOC_ENUM_FRAMESIZES", WD_V4L2_MS) == 0) {
            if (s.type == V4L2_FRMSIZE_TYPE_DISCRETE)
                fprintf(out, "vdec:   sizes discrete %ux%u\n", s.discrete.width, s.discrete.height);
            else
                fprintf(out, "vdec:   sizes %s %ux%u..%ux%u step %ux%u\n", s.type == V4L2_FRMSIZE_TYPE_CONTINUOUS ? "continuous" : "stepwise",
                        s.stepwise.min_width, s.stepwise.min_height, s.stepwise.max_width, s.stepwise.max_height,
                        s.stepwise.step_width, s.stepwise.step_height);
        }
    }
}

static void print_fmt(FILE *out, const char *tag, const struct v4l2_pix_format_mplane *p)
{
    char cc[5];
    fprintf(out, "vdec: %s '%s' %ux%u planes %u field %u colorspace %u", tag, fourcc_str(p->pixelformat, cc), p->width, p->height, p->num_planes, p->field, p->colorspace);
    for (unsigned i = 0; i < p->num_planes && i < VIDEO_MAX_PLANES; i++)
        fprintf(out, " [bytesperline %u sizeimage %u]", p->plane_fmt[i].bytesperline, p->plane_fmt[i].sizeimage);
    fputc('\n', out);
}

static void report_fmt(struct vdec *d, FILE *out, unsigned long req, const char *req_name, enum v4l2_buf_type type, const char *tag,
                       uint32_t fourcc, uint32_t w, uint32_t h, uint32_t sizeimage)
{
    struct v4l2_format f;
    memset(&f, 0, sizeof(f));
    f.type = type;
    if (fourcc) {
        f.fmt.pix_mp.pixelformat = fourcc;
        f.fmt.pix_mp.width = w;
        f.fmt.pix_mp.height = h;
        f.fmt.pix_mp.num_planes = 1;
        f.fmt.pix_mp.plane_fmt[0].sizeimage = sizeimage;
    }
    if (vioc(d, req, &f, req_name, WD_V4L2_MS) < 0) {
        fprintf(out, "vdec: %s %s: %s\n", req_name, tag, strerror(errno));
        return;
    }
    print_fmt(out, tag, &f.fmt.pix_mp);
}

int vdec_report(struct vdec *d, FILE *out, char *err, size_t errlen)
{
    struct v4l2_capability cap;
    memset(&cap, 0, sizeof(cap));
    if (vioc(d, VIDIOC_QUERYCAP, &cap, "VIDIOC_QUERYCAP", WD_V4L2_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_QUERYCAP");
    fprintf(out, "vdec: driver '%.16s' card '%.32s' bus '%.32s' version 0x%x capabilities 0x%x device_caps 0x%x\n", cap.driver, cap.card,
            cap.bus_info, cap.version, cap.capabilities, cap.device_caps);
    report_formats(d, out, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "OUTPUT");
    report_formats(d, out, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "CAPTURE");
    report_fmt(d, out, VIDIOC_G_FMT, "G_FMT", V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "default OUTPUT", 0, 0, 0, 0);
    report_fmt(d, out, VIDIOC_G_FMT, "G_FMT", V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "default CAPTURE", 0, 0, 0, 0);
    report_fmt(d, out, VIDIOC_TRY_FMT, "TRY_FMT", V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "MPEG2 1920x1088", V4L2_PIX_FMT_MPEG2, 1920, 1088, 1u << 20);
    report_fmt(d, out, VIDIOC_TRY_FMT, "TRY_FMT", V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "H264 1280x720", V4L2_PIX_FMT_H264, 1280, 720, 1u << 20);
    report_fmt(d, out, VIDIOC_TRY_FMT, "TRY_FMT", V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "NV12 1920x1088", V4L2_PIX_FMT_NV12, 1920, 1088, 0);
    report_fmt(d, out, VIDIOC_TRY_FMT, "TRY_FMT", V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "NV12 1280x720", V4L2_PIX_FMT_NV12, 1280, 720, 0);
    report_fmt(d, out, VIDIOC_TRY_FMT, "TRY_FMT", V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "NV12 720x480", V4L2_PIX_FMT_NV12, 720, 480, 0);

    /* every control, plain and compound */
    struct v4l2_query_ext_ctrl q;
    memset(&q, 0, sizeof(q));
    q.id = V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;
    for (int n = 0; n < 64; n++) {
        if (vioc(d, VIDIOC_QUERY_EXT_CTRL, &q, "VIDIOC_QUERY_EXT_CTRL", WD_V4L2_MS) < 0) {
            if (errno != EINVAL)
                fprintf(out, "vdec: QUERY_EXT_CTRL: %s\n", strerror(errno));
            break;
        }
        fprintf(out, "vdec: control 0x%08x type %u '%s' min %lld max %lld step %llu default %lld flags 0x%x elem_size %u elems %u\n", q.id,
                q.type, q.name, (long long)q.minimum, (long long)q.maximum, (unsigned long long)q.step, (long long)q.default_value,
                q.flags, q.elem_size, q.elems);
        q.id |= V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;
    }
    struct v4l2_control c = { .id = V4L2_CID_MIN_BUFFERS_FOR_CAPTURE };
    if (vioc(d, VIDIOC_G_CTRL, &c, "VIDIOC_G_CTRL", WD_V4L2_MS) == 0)
        fprintf(out, "vdec: MIN_BUFFERS_FOR_CAPTURE = %d\n", c.value);
    else
        fprintf(out, "vdec: MIN_BUFFERS_FOR_CAPTURE: %s\n", strerror(errno));
    return 0;
}

/* ---- buffers ---- */

static int qbuf_cap(struct vdec *d, uint32_t i, char *err, size_t errlen)
{
    struct v4l2_plane pl;
    struct v4l2_buffer b;
    memset(&pl, 0, sizeof(pl));
    memset(&b, 0, sizeof(b));
    b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    b.memory = V4L2_MEMORY_DMABUF;
    b.index = i;
    b.length = 1;
    b.m.planes = &pl;
    pl.m.fd = d->cap[i].dmabuf;
    pl.length = d->capsize;
    if (vioc(d, VIDIOC_QBUF, &b, "VIDIOC_QBUF (CAPTURE)", WD_V4L2_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_QBUF (CAPTURE)");
    d->cap[i].queued = 1;
    return 0;
}

/* The extension control's number is firmware-specific (vdec_abi.h): find it by name. */
static int find_res_info(struct vdec *d, uint32_t *id)
{
    struct v4l2_query_ext_ctrl q;
    memset(&q, 0, sizeof(q));
    q.id = V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;
    for (int n = 0; n < 64; n++) {
        if (vioc(d, VIDIOC_QUERY_EXT_CTRL, &q, "VIDIOC_QUERY_EXT_CTRL", WD_V4L2_MS) < 0)
            return -1;
        if (!strcmp((const char *)q.name, VDEC_RES_INFO_NAME) && q.elem_size == sizeof(struct vdec_res_info)) {
            *id = q.id;
            return 0;
        }
        q.id |= V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;
    }
    return -1;
}

static int set_res_info(struct vdec *d, uint32_t w, uint32_t h)
{
    uint32_t id;
    if (find_res_info(d, &id) < 0)
        return -1;
    struct vdec_res_info ri;
    memset(&ri, 0, sizeof(ri));
    ri.max_width = w;
    ri.max_height = h;
    ri.max_frame_rate.numerator = 60;
    ri.max_frame_rate.denominator = 1;
    struct v4l2_ext_control ec;
    memset(&ec, 0, sizeof(ec));
    ec.id = id;
    ec.size = sizeof(ri);
    ec.ptr = &ri;
    struct v4l2_ext_controls cs;
    memset(&cs, 0, sizeof(cs));
    cs.count = 1;
    cs.controls = &ec;
    return vioc(d, VIDIOC_S_EXT_CTRLS, &cs, "VIDIOC_S_EXT_CTRLS (res info)", WD_V4L2_MS);
}

static int streamon(struct vdec *d, enum v4l2_buf_type type, const char *what, unsigned ms)
{
    return vioc(d, VIDIOC_STREAMON, &type, what, ms);
}

static int streamoff(struct vdec *d, enum v4l2_buf_type type, const char *what)
{
    return vioc(d, VIDIOC_STREAMOFF, &type, what, WD_V4L2_STREAMOFF_MS);
}

int vdec_setup(struct vdec *d, const struct vdec_cfg *cfg, char *err, size_t errlen)
{
    if (d->fd < 0 || d->streaming || d->nout) {
        errno = d->fd < 0 ? EBADF : EBUSY;
        return sys_fail(err, errlen, "vdec_setup");
    }
    if ((cfg->codec != V4L2_PIX_FMT_MPEG2 && cfg->codec != V4L2_PIX_FMT_H264) || cfg->max_w < 16 || cfg->max_w > 1920 ||
        cfg->max_h < 16 || cfg->max_h > 1088 || cfg->nout < 2 || cfg->nout > VDEC_MAX_OUT || cfg->ncap < 3 || cfg->ncap > VDEC_MAX_CAP ||
        cfg->out_size < (64u << 10) || cfg->out_size > (8u << 20)) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vdec_setup: configuration out of range");
    }
    d->cfg = *cfg;
    char what[96];

    /* the OUTPUT format first: with the source-change event not yet subscribed the driver sizes the CAPTURE
     * format from it (v4l2_vdec.c, __ioc_s_fmt_vid_out_mplane), which is what makes the picture buffers the
     * largest size the stream will carry */
    struct v4l2_format f;
    memset(&f, 0, sizeof(f));
    f.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    f.fmt.pix_mp.pixelformat = cfg->codec;
    f.fmt.pix_mp.width = cfg->max_w;
    f.fmt.pix_mp.height = cfg->max_h;
    f.fmt.pix_mp.num_planes = 1;
    f.fmt.pix_mp.plane_fmt[0].sizeimage = cfg->out_size;
    if (vioc(d, VIDIOC_S_FMT, &f, "VIDIOC_S_FMT (OUTPUT)", WD_V4L2_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_S_FMT (OUTPUT)");
        goto fail;
    }
    if (f.fmt.pix_mp.pixelformat != cfg->codec) {
        errno = ENOTSUP;
        snprintf(err, errlen, "the decoder does not take this codec (S_FMT answered another format)");
        goto fail;
    }

    memset(&f, 0, sizeof(f));
    f.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    f.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12;
    f.fmt.pix_mp.width = cfg->max_w;
    f.fmt.pix_mp.height = cfg->max_h;
    if (vioc(d, VIDIOC_S_FMT, &f, "VIDIOC_S_FMT (CAPTURE)", WD_V4L2_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_S_FMT (CAPTURE)");
        goto fail;
    }
    if (f.fmt.pix_mp.pixelformat != V4L2_PIX_FMT_NV12 || f.fmt.pix_mp.num_planes != 1 ||
        f.fmt.pix_mp.plane_fmt[0].sizeimage < cfg->max_w * cfg->max_h / 2 * 3 || f.fmt.pix_mp.plane_fmt[0].sizeimage > (16u << 20)) {
        errno = EPROTO;
        snprintf(err, errlen, "the CAPTURE format is not one NV12 plane of a sane size (planes %u sizeimage %u)", f.fmt.pix_mp.num_planes,
                 f.fmt.pix_mp.plane_fmt[0].sizeimage);
        goto fail;
    }
    d->capsize = f.fmt.pix_mp.plane_fmt[0].sizeimage;
    d->w = f.fmt.pix_mp.width;
    d->h = f.fmt.pix_mp.height;

    /* a failure here is not fatal: the decoder then sizes its picture store from the stream */
    d->res_info_ok = set_res_info(d, cfg->max_w, cfg->max_h) == 0;

    struct v4l2_event_subscription sub;
    memset(&sub, 0, sizeof(sub));
    sub.type = V4L2_EVENT_SOURCE_CHANGE;
    if (vioc(d, VIDIOC_SUBSCRIBE_EVENT, &sub, "VIDIOC_SUBSCRIBE_EVENT", WD_V4L2_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_SUBSCRIBE_EVENT (source change)");
        goto fail;
    }

    /* bitstream buffers: the driver's own memory, mapped */
    struct v4l2_requestbuffers rb;
    memset(&rb, 0, sizeof(rb));
    rb.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    rb.memory = V4L2_MEMORY_MMAP;
    rb.count = cfg->nout;
    if (vioc(d, VIDIOC_REQBUFS, &rb, "VIDIOC_REQBUFS (OUTPUT)", WD_V4L2_REQBUFS_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_REQBUFS (OUTPUT)");
        goto fail;
    }
    if (rb.count < 2 || rb.count > VDEC_MAX_OUT) {
        errno = EPROTO;
        snprintf(err, errlen, "REQBUFS (OUTPUT) gave %u buffers", rb.count);
        goto fail;
    }
    d->nout = rb.count;
    for (uint32_t i = 0; i < d->nout; i++) {
        struct v4l2_plane pl;
        struct v4l2_buffer b;
        memset(&pl, 0, sizeof(pl));
        memset(&b, 0, sizeof(b));
        b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.index = i;
        b.length = 1;
        b.m.planes = &pl;
        snprintf(what, sizeof(what), "VIDIOC_QUERYBUF (OUTPUT %u)", i);
        if (vioc(d, VIDIOC_QUERYBUF, &b, what, WD_V4L2_MS) < 0) {
            sys_fail(err, errlen, "VIDIOC_QUERYBUF (OUTPUT)");
            goto fail;
        }
        if (pl.length < cfg->out_size) {
            errno = EPROTO;
            snprintf(err, errlen, "OUTPUT buffer %u is %u bytes, asked for %u", i, pl.length, cfg->out_size);
            goto fail;
        }
        wd_arm("mmap of an OUTPUT buffer", WD_MMAP_MS);
        void *m = d->sys->mmap(NULL, pl.length, PROT_READ | PROT_WRITE, MAP_SHARED, d->fd, (off_t)pl.m.mem_offset);
        wd_disarm();
        if (m == MAP_FAILED) {
            sys_fail(err, errlen, "mmap of an OUTPUT buffer");
            goto fail;
        }
        d->out[i].map = m;
        d->out[i].len = pl.length;
        d->out[i].queued = 0;
    }

    /* picture buffers: ours, from an HMA pool (the physical address is what the encoder is given), handed to the
     * driver as dma-bufs */
    d->ncap = cfg->ncap;
    const char *pool = cfg->pool ? cfg->pool : "photofb";
    for (uint32_t i = 0; i < d->ncap; i++) {
        if (hma_alloc(&d->cap[i].mem, d->sys, pool, d->capsize, 4096, 0, err, errlen) < 0) {
            d->ncap = i;
            goto fail;
        }
        int fd = hma_export_dmabuf(&d->cap[i].mem, err, errlen);
        if (fd < 0) {
            d->ncap = i + 1;
            goto fail;
        }
        d->cap[i].dmabuf = fd;
    }
    memset(&rb, 0, sizeof(rb));
    rb.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    rb.memory = V4L2_MEMORY_DMABUF;
    rb.count = d->ncap;
    if (vioc(d, VIDIOC_REQBUFS, &rb, "VIDIOC_REQBUFS (CAPTURE)", WD_V4L2_REQBUFS_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_REQBUFS (CAPTURE)");
        goto fail;
    }
    if (rb.count < d->ncap) {
        errno = EPROTO;
        snprintf(err, errlen, "REQBUFS (CAPTURE) gave %u of %u buffers", rb.count, d->ncap);
        goto fail;
    }
    for (uint32_t i = 0; i < d->ncap; i++)
        if (qbuf_cap(d, i, err, errlen) < 0)
            goto fail;

    return 0;
fail: {
    int e = errno;
    char keep[256];
    snprintf(keep, sizeof(keep), "%s", err ? err : "");
    vdec_close(d);
    if (err)
        snprintf(err, errlen, "%s", keep);
    errno = e;
    return -1;
}
}

int vdec_streamon(struct vdec *d, char *err, size_t errlen)
{
    if (d->fd < 0 || d->streaming || !d->nout) {
        errno = d->streaming ? EBUSY : EINVAL;
        return sys_fail(err, errlen, "vdec_streamon");
    }
    /* the decoder channel and its code-picture buffer are made on the first STREAMON */
    if (streamon(d, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "VIDIOC_STREAMON (OUTPUT)", WD_V4L2_STREAMON_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_STREAMON (OUTPUT)");
        goto fail;
    }
    d->streaming = 1;
    if (streamon(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMON (CAPTURE)", WD_V4L2_STREAMON_MS) < 0) {
        sys_fail(err, errlen, "VIDIOC_STREAMON (CAPTURE)");
        goto fail;
    }
    return 0;
fail: {
    int e = errno;
    char keep[256];
    snprintf(keep, sizeof(keep), "%s", err ? err : "");
    vdec_close(d);
    if (err)
        snprintf(err, errlen, "%s", keep);
    errno = e;
    return -1;
}
}

int vdec_start(struct vdec *d, const struct vdec_cfg *cfg, char *err, size_t errlen)
{
    return vdec_setup(d, cfg, err, errlen) < 0 ? -1 : vdec_streamon(d, err, errlen);
}

/* ---- running ---- */

int vdec_feed(struct vdec *d, const uint8_t *au, size_t len, uint64_t seq, int timeout_ms, char *err, size_t errlen)
{
    if (!d->streaming) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vdec_feed: not streaming");
    }
    if (len == 0 || len > d->out[0].len) {
        errno = EMSGSIZE;
        return sys_fail(err, errlen, "vdec_feed: access unit empty or larger than a bitstream buffer");
    }
    double deadline = now_ms() + timeout_ms;
    int idx = -1;
    for (;;) {
        for (uint32_t j = 0; j < d->nout; j++)
            if (!d->out[j].queued) {
                idx = (int)j;
                break;
            }
        if (idx >= 0)
            break;
        struct v4l2_plane pl;
        struct v4l2_buffer b;
        memset(&pl, 0, sizeof(pl));
        memset(&b, 0, sizeof(b));
        b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.length = 1;
        b.m.planes = &pl;
        if (vioc(d, VIDIOC_DQBUF, &b, "VIDIOC_DQBUF (OUTPUT)", WD_V4L2_MS) == 0) {
            if (b.index < d->nout)
                d->out[b.index].queued = 0;
            if (b.flags & V4L2_BUF_FLAG_ERROR)
                d->out_errors++;
            continue;
        }
        if (errno != EAGAIN)
            return sys_fail(err, errlen, "VIDIOC_DQBUF (OUTPUT)");
        double left = deadline - now_ms();
        if (left <= 0)
            return 1;
        struct pollfd p = { .fd = d->fd, .events = POLLOUT };
        wd_arm("poll for a free bitstream buffer", (unsigned)left + WD_POLL_EXTRA_MS);
        int pr = d->sys->poll(&p, 1, (int)left + 1);
        wd_disarm();
        if (pr < 0 && errno != EINTR)
            return sys_fail(err, errlen, "poll (OUTPUT)");
        if (pr < 0)
            return 1;
    }
    memcpy(d->out[idx].map, au, len);
    struct v4l2_plane pl;
    struct v4l2_buffer b;
    memset(&pl, 0, sizeof(pl));
    memset(&b, 0, sizeof(b));
    b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    b.memory = V4L2_MEMORY_MMAP;
    b.index = (uint32_t)idx;
    b.length = 1;
    b.m.planes = &pl;
    pl.bytesused = (uint32_t)len;
    pl.length = (uint32_t)d->out[idx].len;
    /* the sequence number rides as nanoseconds (x1000), so it survives the timeval round trip exactly */
    b.timestamp.tv_sec = (time_t)(seq / 1000000u);
    b.timestamp.tv_usec = (suseconds_t)(seq % 1000000u);
    if (vioc(d, VIDIOC_QBUF, &b, "VIDIOC_QBUF (OUTPUT)", WD_V4L2_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_QBUF (OUTPUT)");
    d->out[idx].queued = 1;
    d->fed++;
    return 0;
}

/* The picture size changed (or is announced for the first time): the driver holds the picture until the CAPTURE
 * queue is cycled. The buffers were made for the largest size, so they are only re-queued; one that does not
 * fit is a stream this configuration cannot carry. */
static int rearm(struct vdec *d, char *err, size_t errlen)
{
    struct v4l2_format f;
    memset(&f, 0, sizeof(f));
    f.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    if (vioc(d, VIDIOC_G_FMT, &f, "VIDIOC_G_FMT (CAPTURE)", WD_V4L2_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_G_FMT (CAPTURE)");
    const struct v4l2_pix_format_mplane *p = &f.fmt.pix_mp;
    if (p->pixelformat != V4L2_PIX_FMT_NV12 || p->width < 16 || p->height < 16 || p->width > d->cfg.max_w || p->height > d->cfg.max_h ||
        (uint64_t)p->width * p->height / 2 * 3 > d->capsize) {
        errno = EFBIG;
        snprintf(err, errlen, "the stream's picture size %ux%u does not fit the buffers (%ux%u max)", p->width, p->height, d->cfg.max_w, d->cfg.max_h);
        return -1;
    }
    if (streamoff(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMOFF (CAPTURE)") < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMOFF (CAPTURE)");
    for (uint32_t i = 0; i < d->ncap; i++)
        d->cap[i].queued = 0;
    for (uint32_t i = 0; i < d->ncap; i++)
        if (!d->cap[i].held && qbuf_cap(d, i, err, errlen) < 0)
            return -1;
    if (streamon(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMON (CAPTURE)", WD_V4L2_STREAMON_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMON (CAPTURE)");
    d->w = p->width;
    d->h = p->height;
    d->source_changes++;
    return 0;
}

/* The number of source changes handled, or -1 with err. */
static int reap_events(struct vdec *d, char *err, size_t errlen)
{
    int changes = 0;
    for (int n = 0; n < 16; n++) {
        struct v4l2_event ev;
        memset(&ev, 0, sizeof(ev));
        if (vioc(d, VIDIOC_DQEVENT, &ev, "VIDIOC_DQEVENT", WD_V4L2_MS) < 0) {
            if (errno == ENOENT || errno == EAGAIN)
                return changes;
            return sys_fail(err, errlen, "VIDIOC_DQEVENT");
        }
        if (ev.type == V4L2_EVENT_SOURCE_CHANGE && (ev.u.src_change.changes & V4L2_EVENT_SRC_CH_RESOLUTION)) {
            if (rearm(d, err, errlen) < 0)
                return -1;
            changes++;
        }
    }
    return changes;
}

int vdec_get_picture(struct vdec *d, struct vdec_pic *pic, int timeout_ms, char *err, size_t errlen)
{
    if (!d->streaming) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vdec_get_picture: not streaming");
    }
    double deadline = now_ms() + timeout_ms;
    for (;;) {
        struct v4l2_plane pl;
        struct v4l2_buffer b;
        memset(&pl, 0, sizeof(pl));
        memset(&b, 0, sizeof(b));
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_DMABUF;
        b.length = 1;
        b.m.planes = &pl;
        if (vioc(d, VIDIOC_DQBUF, &b, "VIDIOC_DQBUF (CAPTURE)", WD_V4L2_MS) == 0) {
            if (b.index >= d->ncap) {
                errno = EPROTO;
                return sys_fail(err, errlen, "VIDIOC_DQBUF (CAPTURE) returned an unknown buffer");
            }
            uint32_t i = b.index;
            d->cap[i].queued = 0;
            d->cap[i].held = 1;
            memset(pic, 0, sizeof(*pic));
            pic->index = (int)i;
            pic->width = d->w;
            pic->height = d->h;
            pic->stride = d->w;
            pic->paddr_y = d->cap[i].mem.paddr;
            pic->paddr_c = d->cap[i].mem.paddr + d->w * d->h;
            pic->y = d->cap[i].mem.vaddr;
            pic->c = d->cap[i].mem.vaddr + (size_t)d->w * d->h;
            pic->seq = (uint64_t)b.timestamp.tv_sec * 1000000u + (uint64_t)b.timestamp.tv_usec;
            pic->last = !!(b.flags & V4L2_BUF_FLAG_LAST);
            pic->error = !!(b.flags & V4L2_BUF_FLAG_ERROR);
            if (!pic->last && !pic->error)
                d->pictures++;
            else if (pic->error)
                d->cap_errors++;
            return 1;
        }
        if (errno != EAGAIN)
            return sys_fail(err, errlen, "VIDIOC_DQBUF (CAPTURE)");
        /* only with no finished picture left: those are of the old size, and the re-arm discards them */
        int changes = reap_events(d, err, errlen);
        if (changes < 0)
            return -1;
        if (changes > 0)
            continue;
        double left = deadline - now_ms();
        if (left <= 0)
            return 0;
        struct pollfd p = { .fd = d->fd, .events = POLLIN | POLLPRI };
        wd_arm("poll for a picture", (unsigned)left + WD_POLL_EXTRA_MS);
        int pr = d->sys->poll(&p, 1, (int)left + 1);
        wd_disarm();
        if (pr < 0 && errno != EINTR)
            return sys_fail(err, errlen, "poll (CAPTURE)");
        if (pr < 0)
            return 0;
    }
}

int vdec_put_picture(struct vdec *d, int index, char *err, size_t errlen)
{
    if (index < 0 || (uint32_t)index >= d->ncap || !d->cap[index].held) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vdec_put_picture: not a picture held by the caller");
    }
    d->cap[index].held = 0;
    return qbuf_cap(d, (uint32_t)index, err, errlen);
}

int vdec_drain(struct vdec *d, char *err, size_t errlen)
{
    struct v4l2_decoder_cmd c;
    memset(&c, 0, sizeof(c));
    c.cmd = V4L2_DEC_CMD_STOP;
    if (vioc(d, VIDIOC_DECODER_CMD, &c, "VIDIOC_DECODER_CMD (stop)", WD_V4L2_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_DECODER_CMD (stop)");
    return 0;
}

int vdec_flush(struct vdec *d, char *err, size_t errlen)
{
    if (!d->streaming) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vdec_flush: not streaming");
    }
    if (streamoff(d, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "VIDIOC_STREAMOFF (OUTPUT)") < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMOFF (OUTPUT)");
    if (streamoff(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMOFF (CAPTURE)") < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMOFF (CAPTURE)");
    for (uint32_t i = 0; i < d->nout; i++)
        d->out[i].queued = 0;
    for (uint32_t i = 0; i < d->ncap; i++)
        d->cap[i].queued = 0;
    for (uint32_t i = 0; i < d->ncap; i++)
        if (!d->cap[i].held && qbuf_cap(d, i, err, errlen) < 0)
            return -1;
    if (streamon(d, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "VIDIOC_STREAMON (OUTPUT)", WD_V4L2_STREAMON_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMON (OUTPUT)");
    if (streamon(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMON (CAPTURE)", WD_V4L2_STREAMON_MS) < 0)
        return sys_fail(err, errlen, "VIDIOC_STREAMON (CAPTURE)");
    return 0;
}

void vdec_close(struct vdec *d)
{
    if (!d->sys)
        return;
    if (d->fd >= 0) {
        if (d->streaming) {
            streamoff(d, V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, "VIDIOC_STREAMOFF (OUTPUT)");
            streamoff(d, V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, "VIDIOC_STREAMOFF (CAPTURE)");
            d->streaming = 0;
        }
        for (uint32_t i = 0; i < VDEC_MAX_OUT; i++)
            if (d->out[i].map) {
                d->sys->munmap(d->out[i].map, d->out[i].len);
                d->out[i].map = NULL;
            }
        /* the kernel drops its attachments to the dma-bufs when the fd closes; closing ours first is harmless */
        for (uint32_t i = 0; i < VDEC_MAX_CAP; i++)
            if (d->cap[i].dmabuf >= 0) {
                d->sys->close(d->cap[i].dmabuf);
                d->cap[i].dmabuf = -1;
            }
        wd_arm("close " VDEC_DEV, WD_V4L2_CLOSE_MS);
        d->sys->close(d->fd);
        wd_disarm();
        d->fd = -1;
    }
    /* a buffer the decoder might still write is freed only after its node is closed */
    for (uint32_t i = 0; i < VDEC_MAX_CAP; i++)
        hma_free(&d->cap[i].mem);
}
