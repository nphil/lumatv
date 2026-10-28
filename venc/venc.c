#include "venc.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

void venc_kadp_magic(LX_VENC_KADPCALL_PARAM_T *m)
{
    memset(m, 0, sizeof(*m));
    memcpy(m->kadpmagic1, "SIC", 3);
    memcpy(m->kadpmagic2, "KADP", 4);
    memcpy(m->kadpmagic3, "VENC", 4);
}

int venc_open(struct venc *v, const struct sysops *sys, char *err, size_t errlen)
{
    memset(v, 0, sizeof(*v));
    v->sys = sys;
    v->fd = -1;
    v->id = -1;
    v->fd = sys->open(VENC_DEV, O_RDWR);
    if (v->fd < 0)
        return sys_fail(err, errlen, "open " VENC_DEV);

    LX_VENC_KADPCALL_PARAM_T magic;
    venc_kadp_magic(&magic);
    if (sys->ioctl(v->fd, LX_VENC_IOW_VALID_KADPCALL, &magic) < 0) {
        sys_fail(err, errlen, "LX_VENC_IOW_VALID_KADPCALL (the KADP gate)");
        venc_close(v);
        return -1;
    }
    if (sys->ioctl(v->fd, LX_VENC_IOR_GET_BUFFER_INFO, &v->buf) < 0) {
        sys_fail(err, errlen, "LX_VENC_IOR_GET_BUFFER_INFO");
        venc_close(v);
        return -1;
    }
    if (v->buf.ui32ESBufSize == 0 || v->buf.ui32ESBufPhysBase == 0 || (v->buf.ui32AUIBufSize & 0xfff)) {
        errno = EPROTO;
        snprintf(err, errlen, "buffer info unusable: es 0x%08x+0x%x aui 0x%08x+0x%x", v->buf.ui32ESBufPhysBase,
                 v->buf.ui32ESBufSize, v->buf.ui32AUIBufPhysBase, v->buf.ui32AUIBufSize);
        venc_close(v);
        return -1;
    }
    /* the ES ring is at mmap offset = AUI size (ipc/local/venc.c, venc_mmap); PROT_READ is enough, the
     * encoder writes it from the other side */
    void *m = sys->mmap(NULL, v->buf.ui32ESBufSize, PROT_READ, MAP_SHARED, v->fd, (off_t)v->buf.ui32AUIBufSize);
    if (m == MAP_FAILED) {
        sys_fail(err, errlen, "mmap ES buffer");
        venc_close(v);
        return -1;
    }
    v->es = m;
    return 0;
}

int venc_create(struct venc *v, const struct venc_cfg *cfg, char *err, size_t errlen)
{
    if (v->id >= 0) {
        errno = EBUSY;
        return sys_fail(err, errlen, "venc_create: instance exists");
    }
    if (cfg->width < VENC_MIN_WIDTH || cfg->width > VENC_MAX_WIDTH || cfg->height < VENC_MIN_HEIGHT ||
        cfg->height > VENC_MAX_HEIGHT || (cfg->width & 1) || (cfg->height & 1) || cfg->fps == 0 ||
        cfg->kbps > VENC_MAX_KBPS || cfg->profile > LX_VENC_H264_PROFILE_BASELINE) {
        errno = EINVAL;
        return sys_fail(err, errlen, "venc_create: configuration out of range");
    }
    LX_VENC_ENC_CREATE_T c;
    memset(&c, 0, sizeof(c));
    c.width = cfg->width;
    c.height = cfg->height;
    c.framerate = cfg->fps;
    c.gopLength = cfg->gop;
    c.bitrate = cfg->kbps;
    c.fixedQp = cfg->kbps ? 0 : cfg->fixed_qp;
    c.imageType = LX_VENC_IMAGE_YUV420_SEMIPLANAR;
    c.codecType = LX_VENC_ENCODE_TYPE_H264;
    c.profile = cfg->profile;
    if (v->sys->ioctl(v->fd, LX_VENC_IOWR_ENC_CREATE, &c) < 0)
        return sys_fail(err, errlen, "LX_VENC_IOWR_ENC_CREATE");
    v->id = c.id;
    v->cfg = *cfg;
    v->frames = 0;
    if (cfg->kbps) {
        LX_VENC_ENC_RC_T rc = { .id = v->id, .rcType = cfg->cbr ? LX_VENC_RC_TYPE_CBR : LX_VENC_RC_TYPE_VBR, .value = cfg->kbps };
        if (v->sys->ioctl(v->fd, LX_VENC_IOW_ENC_SET_RC, &rc) < 0)
            return sys_fail(err, errlen, "LX_VENC_IOW_ENC_SET_RC");
    }
    return 0;
}

int venc_encode(struct venc *v, uint32_t paddr_y, uint32_t paddr_c, struct venc_frame *out, char *err,
                size_t errlen)
{
    if (v->id < 0) {
        errno = EINVAL;
        return sys_fail(err, errlen, "venc_encode: no instance");
    }
    LX_VENC_ENC_ENCODE_T e;
    memset(&e, 0, sizeof(e));
    e.id = v->id;
    e.bus_luma = paddr_y;
    e.bus_chroma_u = paddr_c;
    e.bus_chroma_v = 0;
    e.sizer = 0;
    e.compat_p_out_buf = 0; /* the MCU addresses the output by bus_output; LG's plugin passes its mapping, unused */
    e.bus_output = v->buf.ui32ESBufPhysBase;
    e.out_buf_size = v->buf.ui32ESBufSize;
    e.timeIncrement = 1;
    int r = v->sys->ioctl(v->fd, LX_VENC_IOWR_ENC_ENCODE, &e);
    /* the MCU answers HXENC_OK (0) or HXENC_FRAME_READY (1); anything else is an error */
    if (r < 0 || r > 1)
        return sys_fail(err, errlen, "LX_VENC_IOWR_ENC_ENCODE");
    if (e.stream_size == 0 || e.stream_size > v->buf.ui32ESBufSize) {
        errno = EPROTO;
        snprintf(err, errlen, "encode returned stream_size %u (ring %u)", e.stream_size, v->buf.ui32ESBufSize);
        return -1;
    }
    out->data = v->es;
    out->size = e.stream_size;
    out->keyframe = e.codingType == LX_VENC_INTRA_FRAME;
    out->hw_ms = e.duration;
    v->frames++;
    return 0;
}

int venc_set_bitrate(struct venc *v, uint32_t kbps, char *err, size_t errlen)
{
    LX_VENC_ENC_PARAM_T p = { .adjustParam = ENCODE_PARAM_BITRATE, .bitrate = kbps };
    if (v->id < 0 || kbps == 0 || kbps > VENC_MAX_KBPS) {
        errno = EINVAL;
        return sys_fail(err, errlen, "venc_set_bitrate");
    }
    if (v->sys->ioctl(v->fd, LX_VENC_IOWR_ENC_PARAM, &p) < 0)
        return sys_fail(err, errlen, "LX_VENC_IOWR_ENC_PARAM bitrate");
    v->cfg.kbps = kbps;
    return 0;
}

int venc_force_idr(struct venc *v, char *err, size_t errlen)
{
    LX_VENC_ENC_PARAM_T p = { .adjustParam = ENCODE_PARAM_SET_IDR, .setIDRFrame = 1 };
    if (v->id < 0) {
        errno = EINVAL;
        return sys_fail(err, errlen, "venc_force_idr");
    }
    if (v->sys->ioctl(v->fd, LX_VENC_IOWR_ENC_PARAM, &p) < 0)
        return sys_fail(err, errlen, "LX_VENC_IOWR_ENC_PARAM idr");
    return 0;
}

void venc_close(struct venc *v)
{
    if (!v->sys)
        return;
    if (v->id >= 0) {
        LX_VENC_ENC_DESTROY_T d = { .id = v->id };
        v->sys->ioctl(v->fd, LX_VENC_IOW_ENC_DESTROY, &d);
        v->id = -1;
    }
    if (v->es) {
        v->sys->munmap(v->es, v->buf.ui32ESBufSize);
        v->es = NULL;
    }
    if (v->fd >= 0) {
        v->sys->close(v->fd);
        v->fd = -1;
    }
}
