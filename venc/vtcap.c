#include "vtcap.h"

#include <errno.h>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

static const char *const location_names[] = { "scaler-input", "scaler-output", "display", "blended", "osd" };

const char *vtcap_location_name(enum v4l2_ext_capture_location l)
{
    return (unsigned)l < 5 ? location_names[l] : "?";
}

int vtcap_location_parse(const char *s, enum v4l2_ext_capture_location *l)
{
    for (unsigned i = 0; i < 5; i++)
        if (strcmp(s, location_names[i]) == 0) {
            *l = (enum v4l2_ext_capture_location)i;
            return 0;
        }
    return -1;
}

static int ext_ctrl(const struct sysops *sys, int fd, unsigned long req, uint32_t id, void *payload, uint32_t size)
{
    struct v4l2_ext_control ctl;
    struct v4l2_ext_controls ctls;
    memset(&ctl, 0, sizeof(ctl));
    memset(&ctls, 0, sizeof(ctls));
    ctl.id = id;
    ctl.size = size;
    ctl.ptr = payload;
    ctls.ctrl_class = V4L2_CTRL_CLASS_USER;
    ctls.count = 1;
    ctls.controls = &ctl;
    return sys->ioctl(fd, req, &ctls);
}

/* The physical-address query writes through two pointers the driver reads as 32-bit values. */
static uint32_t *low_page(const struct sysops *sys)
{
    if (sizeof(void *) == 4) {
        static uint32_t words[2];
        return words;
    }
    static const uintptr_t hints[] = { 0x7f000000u, 0x60000000u, 0x40000000u, 0x20000000u, 0x10000000u };
    for (unsigned i = 0; i < sizeof(hints) / sizeof(hints[0]); i++) {
        void *p = sys->mmap((void *)hints[i], 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (p == MAP_FAILED)
            continue;
        if ((uintptr_t)p < 0x100000000ull)
            return p;
        sys->munmap(p, 4096);
    }
    return NULL;
}

int vtcap_capability(const struct sysops *sys, struct v4l2_ext_capture_capability_info *cap, char *err, size_t errlen)
{
    int fd = sys->open(VT_DEV, O_RDWR);
    if (fd < 0)
        return sys_fail(err, errlen, "open " VT_DEV);
    memset(cap, 0, sizeof(*cap));
    int r = ext_ctrl(sys, fd, VIDIOC_G_EXT_CTRLS, V4L2_CID_EXT_CAPTURE_CAPABILITY_INFO, cap, sizeof(*cap));
    if (r < 0)
        sys_fail(err, errlen, "G_EXT_CTRLS CAPABILITY_INFO");
    sys->close(fd);
    return r < 0 ? -1 : 0;
}

int vtcap_open(struct vtcap *c, const struct sysops *sys, enum v4l2_ext_capture_location location, uint32_t width,
               uint32_t height, uint32_t nbuf, uint32_t divide, char *err, size_t errlen)
{
    memset(c, 0, sizeof(*c));
    c->sys = sys;
    c->fd = -1;
    if (width == 0 || height == 0 || (width & 15) || nbuf == 0 || nbuf > VTCAP_MAX_BUFS || divide == 0) {
        errno = EINVAL;
        return sys_fail(err, errlen, "vtcap_open: width must be a multiple of 16, 1..6 buffers");
    }
    c->width = width;
    c->height = height;
    c->nbuf = nbuf;
    c->location = location;
    c->lowmem = low_page(sys);
    if (!c->lowmem) {
        errno = ENOMEM;
        return sys_fail(err, errlen, "vtcap_open: no page below 4 GiB for the address query");
    }
    c->fd = sys->open(VT_DEV, O_RDWR);
    if (c->fd < 0)
        return sys_fail(err, errlen, "open " VT_DEV);

    /* 1. where to tap and how big (LG's library: setCapturePlanePropertes, then setFMT with the same size) */
    struct v4l2_ext_capture_plane_prop prop = {
        .l = location, .plane = { .x = 0, .y = 0, .w = (uint16_t)width, .h = (uint16_t)height }, .buf_count = (int32_t)nbuf,
    };
    if (ext_ctrl(sys, c->fd, VIDIOC_S_EXT_CTRLS, V4L2_CID_EXT_CAPTURE_PLANE_PROP, &prop, sizeof(prop)) < 0) {
        sys_fail(err, errlen, "S_EXT_CTRLS PLANE_PROP (the tap point/size; EOPNOTSUPP = this point is not offered)");
        goto fail;
    }
    struct v4l2_format fmt;
    memset(&fmt, 0, sizeof(fmt));
    fmt.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    fmt.fmt.pix_mp.width = width;
    fmt.fmt.pix_mp.height = height;
    fmt.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12;
    fmt.fmt.pix_mp.num_planes = 2;
    if (sys->ioctl(c->fd, VIDIOC_S_FMT, &fmt) < 0) {
        sys_fail(err, errlen, "VIDIOC_S_FMT");
        goto fail;
    }
    if (divide > 1) {
        /* a plain control, as LG's setCaptureFrameRate issues it (VIDIOC_S_CTRL); the driver ignores it
         * silently when more than one process has the device open */
        struct v4l2_control ctl = { .id = V4L2_CID_EXT_CAPTURE_DIVIDE_FRAMERATE, .value = (int32_t)divide };
        if (sys->ioctl(c->fd, VIDIOC_S_CTRL, &ctl) < 0) {
            sys_fail(err, errlen, "S_CTRL DIVIDE_FRAMERATE");
            goto fail;
        }
    }
    /* 2. buffers: REQBUFS, then the plane info (LG's library reads it after REQBUFS), QUERYBUF + mmap of
     * both planes, the physical address of each */
    struct v4l2_requestbuffers req;
    memset(&req, 0, sizeof(req));
    req.count = nbuf;
    req.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    req.memory = V4L2_MEMORY_MMAP;
    if (sys->ioctl(c->fd, VIDIOC_REQBUFS, &req) < 0) {
        sys_fail(err, errlen, "VIDIOC_REQBUFS");
        goto fail;
    }
    if (req.count < nbuf) {
        errno = ENOMEM;
        snprintf(err, errlen, "VIDIOC_REQBUFS gave %u of %u buffers", req.count, nbuf);
        goto fail;
    }
    struct v4l2_ext_capture_plane_info pinfo;
    memset(&pinfo, 0, sizeof(pinfo));
    if (ext_ctrl(sys, c->fd, VIDIOC_G_EXT_CTRLS, V4L2_CID_EXT_CAPTURE_PLANE_INFO, &pinfo, sizeof(pinfo)) == 0)
        c->stride = pinfo.stride;
    for (uint32_t i = 0; i < nbuf; i++) {
        struct v4l2_buffer b;
        struct v4l2_plane planes[2];
        memset(&b, 0, sizeof(b));
        memset(planes, 0, sizeof(planes));
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.index = i;
        b.length = 2;
        b.m.planes = planes;
        if (sys->ioctl(c->fd, VIDIOC_QUERYBUF, &b) < 0) {
            sys_fail(err, errlen, "VIDIOC_QUERYBUF");
            goto fail;
        }
        for (int p = 0; p < 2; p++) {
            void *m = sys->mmap(NULL, planes[p].length, PROT_READ | PROT_WRITE, MAP_SHARED, c->fd, (off_t)planes[p].m.mem_offset);
            if (m == MAP_FAILED) {
                sys_fail(err, errlen, "mmap capture plane");
                goto fail;
            }
            c->plane[i][p] = m;
            c->plane_len[i][p] = planes[p].length;
        }
        struct v4l2_ext_capture_physical_memory_info pm;
        memset(&pm, 0, sizeof(pm));
        pm.buf_index = (int32_t)i;
        pm.buf_location = V4L2_EXT_CAPTURE_OUTPUT_BUF;
        c->lowmem[0] = c->lowmem[1] = 0;
        pm.y = &c->lowmem[0];
        pm.c = &c->lowmem[1];
        if (ext_ctrl(sys, c->fd, VIDIOC_G_EXT_CTRLS, V4L2_CID_EXT_CAPTURE_PHYSICAL_MEMORY_INFO, &pm, sizeof(pm)) < 0) {
            sys_fail(err, errlen, "G_EXT_CTRLS PHYSICAL_MEMORY_INFO");
            goto fail;
        }
        c->paddr_y[i] = c->lowmem[0];
        c->paddr_c[i] = c->lowmem[1];
        if (c->paddr_y[i] == 0 || c->paddr_c[i] == 0) {
            errno = EPROTO;
            snprintf(err, errlen, "buffer %u has no physical address (y 0x%08x c 0x%08x)", i, c->paddr_y[i], c->paddr_c[i]);
            goto fail;
        }
        if (vtcap_requeue(c, (int)i, err, errlen) < 0)
            goto fail;
    }

    /* 3. go */
    int type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    if (sys->ioctl(c->fd, VIDIOC_STREAMON, &type) < 0) {
        sys_fail(err, errlen, "VIDIOC_STREAMON");
        goto fail;
    }
    c->streaming = 1;
    return 0;
fail:
    vtcap_close(c);
    return -1;
}

int vtcap_dequeue(struct vtcap *c, struct vtcap_frame *f, int timeout_ms, char *err, size_t errlen)
{
    struct pollfd pfd = { .fd = c->fd, .events = POLLIN };
    int pr = c->sys->poll(&pfd, 1, timeout_ms);
    if (pr < 0)
        return sys_fail(err, errlen, "poll " VT_DEV);
    if (pr == 0)
        return 1;
    struct v4l2_buffer b;
    struct v4l2_plane planes[2];
    memset(&b, 0, sizeof(b));
    memset(planes, 0, sizeof(planes));
    b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    b.memory = V4L2_MEMORY_MMAP;
    b.length = 2;
    b.m.planes = planes;
    if (c->sys->ioctl(c->fd, VIDIOC_DQBUF, &b) < 0) {
        if (errno == EACCES || errno == ENODATA || errno == EAGAIN)
            return 2;
        return sys_fail(err, errlen, "VIDIOC_DQBUF");
    }
    if (b.index >= c->nbuf) {
        errno = EPROTO;
        snprintf(err, errlen, "DQBUF index %u out of range", b.index);
        return -1;
    }
    f->index = (int)b.index;
    f->paddr_y = c->paddr_y[b.index];
    f->paddr_c = c->paddr_c[b.index];
    f->y = c->plane[b.index][0];
    f->c = c->plane[b.index][1];
    f->y_len = planes[0].bytesused ? planes[0].bytesused : c->plane_len[b.index][0];
    f->c_len = planes[1].bytesused ? planes[1].bytesused : c->plane_len[b.index][1];
    f->ts_us = (uint64_t)b.timestamp.tv_sec * 1000000ull + (uint64_t)b.timestamp.tv_usec;
    f->sequence = b.sequence;
    return 0;
}

int vtcap_requeue(struct vtcap *c, int index, char *err, size_t errlen)
{
    struct v4l2_buffer b;
    struct v4l2_plane planes[2];
    memset(&b, 0, sizeof(b));
    memset(planes, 0, sizeof(planes));
    b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    b.memory = V4L2_MEMORY_MMAP;
    b.index = (uint32_t)index;
    b.length = 2;
    b.m.planes = planes;
    if (c->sys->ioctl(c->fd, VIDIOC_QBUF, &b) < 0)
        return sys_fail(err, errlen, "VIDIOC_QBUF");
    return 0;
}

void vtcap_close(struct vtcap *c)
{
    if (!c->sys)
        return;
    if (c->fd >= 0) {
        if (c->streaming) {
            int type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
            c->sys->ioctl(c->fd, VIDIOC_STREAMOFF, &type);
            c->streaming = 0;
        }
        for (uint32_t i = 0; i < VTCAP_MAX_BUFS; i++)
            for (int p = 0; p < 2; p++)
                if (c->plane[i][p]) {
                    c->sys->munmap(c->plane[i][p], c->plane_len[i][p]);
                    c->plane[i][p] = NULL;
                }
        struct v4l2_requestbuffers req;
        memset(&req, 0, sizeof(req));
        req.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        req.memory = V4L2_MEMORY_MMAP;
        c->sys->ioctl(c->fd, VIDIOC_REQBUFS, &req);
        c->sys->close(c->fd);
        c->fd = -1;
    }
    if (c->lowmem && sizeof(void *) == 8) {
        c->sys->munmap(c->lowmem, 4096);
        c->lowmem = NULL;
    }
}
