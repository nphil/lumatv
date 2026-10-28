/* A fake of the three devices the encoder path touches, behind the sysops table, so the sessions and the
 * probe run on the host: /dev/venc as ipc/local/venc.c behaves (KADP gate first, buffer map, instance
 * create/encode/destroy, mmap of the ES ring at offset = AUI size), /dev/hma as hma_drv.c (one
 * allocation per fd, physical addresses handed out from a bump allocator, mmap of the allocation), and
 * /dev/video60 as v4l2_vt.c and LG's library use it (plane prop, format, MMAP buffers, physical-address
 * query through two pointers, QBUF/DQBUF, STREAMON/OFF). The fake encoder "reads" the input frame through
 * the physical addresses it knows and writes an Annex B unit whose payload carries a checksum of it.
 *
 * Knobs (environment): FAKE_VENC_DENY_MAGIC=1 refuses the gate; FAKE_VT_PROTECTED=1 answers DQBUF with
 * EACCES (protected content); FAKE_VT_TIMEOUT=1 never has a frame; FAKE_VT_NO_BLENDED=1 refuses the blended
 * tap point with EOPNOTSUPP; FAKE_LOG=path appends one line per call. */
#define _GNU_SOURCE
#include <errno.h>
#include <linux/videodev2.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/types.h>

#include "../hma_abi.h"
#include "../synth.h"
#include "../sysops.h"
#include "../venc_abi.h"
#include "../vt_abi.h"

enum kind { K_NONE, K_VENC, K_HMA, K_VT };

#define MAX_FDS 32
#define ES_BASE 0x80100000u
#define ES_SIZE 0x270000u
#define AUI_BASE 0x80000000u
#define AUI_SIZE 0x10000u

struct region { uint32_t paddr; size_t size; uint8_t *mem; };
static struct region regions[64];
static int nregions;
static uint32_t bump = 0x90000000u;

static uint8_t *phys_to_mem(uint32_t paddr, size_t len)
{
    for (int i = 0; i < nregions; i++)
        if (paddr >= regions[i].paddr && paddr + len <= regions[i].paddr + regions[i].size)
            return regions[i].mem + (paddr - regions[i].paddr);
    return NULL;
}

static uint32_t add_region(size_t size, uint8_t *mem)
{
    uint32_t p = bump;
    bump += (uint32_t)((size + 0xfff) & ~0xfffu) + 0x1000;
    regions[nregions++] = (struct region){ p, size, mem };
    return p;
}

static void del_region(uint32_t paddr)
{
    for (int i = 0; i < nregions; i++)
        if (regions[i].paddr == paddr) {
            regions[i] = regions[--nregions];
            return;
        }
}

struct vtbuf { uint8_t *y, *c; uint32_t ylen, clen; uint32_t py, pc; int queued; uint32_t seq; };

struct fd_state {
    enum kind kind;
    int used;
    /* venc */
    int gate_ok, inst, frames, gop_count;
    uint32_t w, h, gop, kbps, profile;
    uint8_t *es;
    /* hma */
    uint32_t paddr; size_t size; uint8_t *mem; int cached;
    /* vt */
    struct v4l2_ext_capture_plane_prop prop;
    uint32_t fmt_w, fmt_h, divide, nbuf;
    struct vtbuf bufs[6];
    int streaming;
    uint32_t seq;
};

static struct fd_state fds[MAX_FDS];
static uint8_t *es_mem; /* the one ES ring (one encoder core) */
static int calls_log_init;
static FILE *calls_log;

static void logcall(const char *fmt, ...)
{
    if (!calls_log_init) {
        calls_log_init = 1;
        const char *p = getenv("FAKE_LOG");
        if (p)
            calls_log = fopen(p, "a");
    }
    if (!calls_log)
        return;
    va_list ap;
    va_start(ap, fmt);
    vfprintf(calls_log, fmt, ap);
    va_end(ap);
    fputc('\n', calls_log);
    fflush(calls_log);
}

static int alloc_fd(enum kind k)
{
    for (int i = 3; i < MAX_FDS; i++)
        if (!fds[i].used) {
            memset(&fds[i], 0, sizeof(fds[i]));
            fds[i].used = 1;
            fds[i].kind = k;
            fds[i].inst = -1;
            return i;
        }
    errno = EMFILE;
    return -1;
}

static int fake_open(const char *path, int flags)
{
    (void)flags;
    logcall("open %s", path);
    if (!strcmp(path, VENC_DEV)) {
        if (!es_mem)
            es_mem = calloc(1, ES_SIZE);
        return alloc_fd(K_VENC);
    }
    if (!strcmp(path, HMA_DEV))
        return alloc_fd(K_HMA);
    if (!strcmp(path, VT_DEV))
        return alloc_fd(K_VT);
    errno = ENOENT;
    return -1;
}

static int fake_close(int fd)
{
    if (fd < 0 || fd >= MAX_FDS || !fds[fd].used) {
        errno = EBADF;
        return -1;
    }
    struct fd_state *s = &fds[fd];
    logcall("close %s inst=%d", s->kind == K_VENC ? "venc" : s->kind == K_HMA ? "hma" : "vt", s->inst);
    if (s->kind == K_HMA && s->mem) {
        del_region(s->paddr);
        free(s->mem);
    }
    if (s->kind == K_VT)
        for (int i = 0; i < 6; i++) {
            if (s->bufs[i].y) { del_region(s->bufs[i].py); free(s->bufs[i].y); }
            if (s->bufs[i].c) { del_region(s->bufs[i].pc); free(s->bufs[i].c); }
        }
    memset(s, 0, sizeof(*s));
    return 0;
}

/* a fake Annex B unit: I frames carry the 64-byte SPS/PPS block the real encoder emits, zero padded */
static uint32_t fake_encode(struct fd_state *s, const LX_VENC_ENC_ENCODE_T *e, uint8_t *out, size_t cap, int *intra_out)
{
    const uint8_t *y = phys_to_mem(e->bus_luma, (size_t)s->w * s->h);
    const uint8_t *c = phys_to_mem(e->bus_chroma_u, (size_t)s->w * s->h / 2);
    if (!y || !c) {
        errno = EFAULT;
        return 0;
    }
    uint32_t sum = 0;
    for (size_t i = 0; i < (size_t)s->w * s->h; i += 97)
        sum = sum * 31 + y[i];
    for (size_t i = 0; i < (size_t)s->w * s->h / 2; i += 89)
        sum = sum * 31 + c[i];
    size_t n = 0;
    int intra = s->frames == 0 || (s->gop && s->gop_count >= (int)s->gop);
    if (intra) {
        static const uint8_t sps[] = { 0, 0, 0, 1, 0x67, 100, 0, 41, 0xac, 0x2b, 0x40, 0x50, 0x1e, 0xd0, 0x80, 0 };
        static const uint8_t pps[] = { 0, 0, 0, 1, 0x68, 0xee, 0x38, 0x80 };
        memset(out, 0, VENC_SPS_PPS_BLOCK);
        memcpy(out, sps, sizeof(sps));
        out[5] = (uint8_t)(s->profile == LX_VENC_H264_PROFILE_BASELINE ? 66 : s->profile == LX_VENC_H264_PROFILE_MAIN ? 77 : 100);
        memcpy(out + sizeof(sps), pps, sizeof(pps));
        n = VENC_SPS_PPS_BLOCK;
        s->gop_count = 0;
    }
    uint8_t hdr[5] = { 0, 0, 0, 1, (uint8_t)(intra ? 0x65 : 0x41) };
    memcpy(out + n, hdr, 5);
    n += 5;
    size_t payload = intra ? 4000 : 900;
    if (n + payload + 8 > cap)
        payload = cap - n - 8;
    for (size_t i = 0; i < payload; i++)
        out[n + i] = (uint8_t)(0x10 + ((sum >> (i % 24)) & 0x3f)); /* never 00 00 0x */
    n += payload;
    out[n++] = 0x88; /* RBSP stop bit */
    s->frames++;
    s->gop_count++;
    *intra_out = intra;
    return (uint32_t)n;
}

static int venc_ioctl(struct fd_state *s, unsigned long req, void *arg)
{
    if (req == LX_VENC_IOW_VALID_KADPCALL) {
        LX_VENC_KADPCALL_PARAM_T *m = arg;
        if (getenv("FAKE_VENC_DENY_MAGIC") || strcmp((char *)m->kadpmagic1, "SIC") || strcmp((char *)m->kadpmagic2, "KADP") ||
            strcmp((char *)m->kadpmagic3, "VENC")) {
            errno = EINVAL;
            return -1;
        }
        s->gate_ok = 1;
        logcall("venc gate ok");
        return 0;
    }
    if (!s->gate_ok) {
        logcall("venc ioctl 0x%lx before gate", req);
        errno = EFAULT;
        return -1;
    }
    if (req == LX_VENC_IOR_GET_BUFFER_INFO) {
        LX_VENC_BUFFER_T *b = arg;
        memset(b, 0, sizeof(*b));
        b->ui32ESBufPhysBase = ES_BASE;
        b->ui32ESBufSize = ES_SIZE;
        b->ui32AUIBufPhysBase = AUI_BASE;
        b->ui32AUIBufSize = AUI_SIZE;
        logcall("venc bufinfo");
        return 0;
    }
    if (req == LX_VENC_IOWR_ENC_CREATE) {
        LX_VENC_ENC_CREATE_T *c = arg;
        if (s->inst >= 0 || c->width < 96 || c->width > 1920 || c->height > 1080 || c->codecType != LX_VENC_ENCODE_TYPE_H264 ||
            c->imageType != LX_VENC_IMAGE_YUV420_SEMIPLANAR) {
            errno = EINVAL;
            return -1;
        }
        s->inst = 7;
        c->id = 7;
        s->w = c->width; s->h = c->height; s->gop = c->gopLength; s->kbps = c->bitrate; s->profile = c->profile;
        s->frames = 0;
        logcall("venc create %ux%u fps %u gop %u kbps %u profile %u", c->width, c->height, c->framerate, c->gopLength, c->bitrate, c->profile);
        return 0;
    }
    if (req == LX_VENC_IOW_ENC_SET_RC) {
        LX_VENC_ENC_RC_T *rc = arg;
        if (rc->id != s->inst) { errno = EINVAL; return -1; }
        logcall("venc rc type %d value %u", rc->rcType, rc->value);
        return 0;
    }
    if (req == LX_VENC_IOWR_ENC_PARAM) {
        LX_VENC_ENC_PARAM_T *p = arg;
        if (p->adjustParam & ENCODE_PARAM_SET_IDR)
            s->gop_count = (int)s->gop; /* next frame is intra */
        if (p->adjustParam & ENCODE_PARAM_BITRATE)
            s->kbps = p->bitrate;
        logcall("venc param 0x%x bitrate %u idr %u", p->adjustParam, p->bitrate, p->setIDRFrame);
        return 0;
    }
    if (req == LX_VENC_IOWR_ENC_ENCODE) {
        LX_VENC_ENC_ENCODE_T *e = arg;
        if (e->id != s->inst || e->bus_output != ES_BASE || e->out_buf_size > ES_SIZE) { errno = EINVAL; return -1; }
        int intra = 0;
        uint32_t n = fake_encode(s, e, es_mem, e->out_buf_size, &intra);
        if (!n)
            return -1;
        e->stream_size = n;
        e->codingType = intra ? LX_VENC_INTRA_FRAME : LX_VENC_PREDICTED_FRAME;
        e->duration = 7;
        logcall("venc encode y 0x%08x c 0x%08x -> %u bytes %s", e->bus_luma, e->bus_chroma_u, n, e->codingType == LX_VENC_INTRA_FRAME ? "I" : "P");
        return (s->frames & 1) ? 1 : 0; /* HXENC_OK or HXENC_FRAME_READY */
    }
    if (req == LX_VENC_IOW_ENC_DESTROY) {
        LX_VENC_ENC_DESTROY_T *d = arg;
        if (d->id != s->inst) { errno = EINVAL; return -1; }
        logcall("venc destroy %d", d->id);
        s->inst = -1;
        return 0;
    }
    errno = ENOTTY;
    return -1;
}

static int hma_ioctl(struct fd_state *s, unsigned long req, void *arg)
{
    if (req == HMA_IOCTL_ALLOC) {
        struct hma_ioctl_alloc *a = arg;
        if (s->mem || a->size <= 0) { errno = s->mem ? EBUSY : EINVAL; return -1; }
        if (strcmp(a->pool, "photofb") && strcmp(a->pool, "cpb") && strcmp(a->pool, "debug0") && strcmp(a->pool, "venc0")) {
            errno = ENOMEM;
            return -1;
        }
        s->mem = calloc(1, (size_t)a->size);
        s->size = (size_t)a->size;
        s->paddr = add_region(s->size, s->mem);
        a->paddr = s->paddr;
        logcall("hma alloc %s %d -> 0x%08x", a->pool, a->size, a->paddr);
        return 0;
    }
    if (req == HMA_IOCTL_CACHED) { s->cached = *(int32_t *)arg; return 0; }
    if (req == HMA_IOCTL_CACHE_CTL) { logcall("hma cache op"); return 0; }
    if (req == HMA_IOCTL_GET_DMABUF) { *(int32_t *)arg = 99; return 99; }
    errno = ENOTTY;
    return -1;
}

static int vt_ext(struct fd_state *s, int set, struct v4l2_ext_controls *cs)
{
    if (cs->count != 1 || !cs->controls) { errno = EINVAL; return -1; }
    struct v4l2_ext_control *c = cs->controls;
    switch (c->id) {
    case V4L2_CID_EXT_CAPTURE_CAPABILITY_INFO: {
        if (set || c->size != sizeof(struct v4l2_ext_capture_capability_info)) { errno = EINVAL; return -1; }
        struct v4l2_ext_capture_capability_info *i = c->ptr;
        memset(i, 0, sizeof(*i));
        i->flags = V4L2_EXT_CAPTURE_CAP_SCALE_DOWN | V4L2_EXT_CAPTURE_CAP_DIVIDE_FRAMERATE;
        i->scale_down_limit_w = 192; i->scale_down_limit_h = 108;
        i->num_video_frame_buffer = 5;
        i->max_res.w = 1920; i->max_res.h = 1080;
        i->num_plane = 2; i->pixel_format = 1;
        return 0;
    }
    case V4L2_CID_EXT_CAPTURE_PLANE_PROP: {
        if (c->size != sizeof(struct v4l2_ext_capture_plane_prop)) { errno = EINVAL; return -1; }
        struct v4l2_ext_capture_plane_prop *p = c->ptr;
        if (!set) { *p = s->prop; return 0; }
        if (p->l == V4L2_EXT_CAPTURE_SCALER_INPUT || (p->l == V4L2_EXT_CAPTURE_BLENDED_OUTPUT && getenv("FAKE_VT_NO_BLENDED"))) {
            errno = EOPNOTSUPP;
            return -1;
        }
        if (p->plane.w & 15) { errno = EOPNOTSUPP; return -1; }
        s->prop = *p;
        logcall("vt plane_prop l=%u %ux%u bufs %d", p->l, p->plane.w, p->plane.h, p->buf_count);
        return 0;
    }
    case V4L2_CID_EXT_CAPTURE_PLANE_INFO: {
        struct v4l2_ext_capture_plane_info *i = c->ptr;
        memset(i, 0, sizeof(*i));
        i->stride = s->prop.plane.w;
        i->plane_region = s->prop.plane;
        i->active_region = s->prop.plane;
        return 0;
    }
    case V4L2_CID_EXT_CAPTURE_PHYSICAL_MEMORY_INFO: {
        struct v4l2_ext_capture_physical_memory_info *pm = c->ptr;
        if (pm->buf_index < 0 || pm->buf_index >= (int)s->nbuf || !s->bufs[pm->buf_index].y) { errno = EINVAL; return -1; }
        /* the driver writes through the pointers it reads from the 32-bit members; on a 64-bit host the
         * session put them below 4 GiB, so the full pointer and the compat member agree */
        if (sizeof(void *) == 8 && ((uintptr_t)pm->y >> 32 || (uintptr_t)pm->c >> 32)) { errno = EFAULT; return -1; }
        *pm->y = s->bufs[pm->buf_index].py;
        *pm->c = s->bufs[pm->buf_index].pc;
        return 0;
    }
    default:
        errno = EINVAL;
        return -1;
    }
}

static int vt_ioctl(struct fd_state *s, unsigned long req, void *arg)
{
    switch (req) {
    case VIDIOC_S_EXT_CTRLS: return vt_ext(s, 1, arg);
    case VIDIOC_G_EXT_CTRLS: return vt_ext(s, 0, arg);
    case VIDIOC_S_CTRL: {
        struct v4l2_control *ctl = arg;
        if (ctl->id != V4L2_CID_EXT_CAPTURE_DIVIDE_FRAMERATE) { errno = EINVAL; return -1; }
        s->divide = (uint32_t)ctl->value;
        logcall("vt divide %u", s->divide);
        return 0;
    }
    case VIDIOC_S_FMT: {
        struct v4l2_format *f = arg;
        if (f->type != V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) { errno = EINVAL; return -1; }
        s->fmt_w = f->fmt.pix_mp.width; s->fmt_h = f->fmt.pix_mp.height;
        logcall("vt s_fmt %ux%u", s->fmt_w, s->fmt_h);
        return 0;
    }
    case VIDIOC_REQBUFS: {
        struct v4l2_requestbuffers *r = arg;
        if (r->type != V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE || r->memory != V4L2_MEMORY_MMAP) { errno = EINVAL; return -1; }
        for (uint32_t i = 0; i < 6; i++) {
            if (s->bufs[i].y) { del_region(s->bufs[i].py); free(s->bufs[i].y); }
            if (s->bufs[i].c) { del_region(s->bufs[i].pc); free(s->bufs[i].c); }
            memset(&s->bufs[i], 0, sizeof(s->bufs[i]));
        }
        if (r->count > 6) r->count = 6;
        s->nbuf = r->count;
        for (uint32_t i = 0; i < r->count; i++) {
            s->bufs[i].ylen = s->fmt_w * s->fmt_h;
            s->bufs[i].clen = s->fmt_w * s->fmt_h / 2;
            s->bufs[i].y = calloc(1, s->bufs[i].ylen);
            s->bufs[i].c = calloc(1, s->bufs[i].clen);
            s->bufs[i].py = add_region(s->bufs[i].ylen, s->bufs[i].y);
            s->bufs[i].pc = add_region(s->bufs[i].clen, s->bufs[i].c);
        }
        logcall("vt reqbufs %u", r->count);
        return 0;
    }
    case VIDIOC_QUERYBUF: {
        struct v4l2_buffer *b = arg;
        if (b->index >= s->nbuf || b->length < 2) { errno = EINVAL; return -1; }
        b->m.planes[0].length = s->bufs[b->index].ylen;
        b->m.planes[0].m.mem_offset = 0x10000 * (b->index * 2 + 1);
        b->m.planes[1].length = s->bufs[b->index].clen;
        b->m.planes[1].m.mem_offset = 0x10000 * (b->index * 2 + 2);
        return 0;
    }
    case VIDIOC_QBUF: {
        struct v4l2_buffer *b = arg;
        if (b->index >= s->nbuf) { errno = EINVAL; return -1; }
        s->bufs[b->index].queued = 1;
        return 0;
    }
    case VIDIOC_DQBUF: {
        struct v4l2_buffer *b = arg;
        if (!s->streaming) { errno = EINVAL; return -1; }
        if (getenv("FAKE_VT_PROTECTED")) { errno = EACCES; return -1; }
        for (uint32_t i = 0; i < s->nbuf; i++)
            if (s->bufs[i].queued) {
                s->bufs[i].queued = 0;
                synth_nv12(s->bufs[i].y, s->bufs[i].c, s->fmt_w, s->fmt_h, s->seq);
                b->index = i;
                b->sequence = s->seq++;
                b->timestamp.tv_sec = 1; b->timestamp.tv_usec = (suseconds_t)(s->seq * 33333) % 1000000;
                b->m.planes[0].bytesused = s->bufs[i].ylen;
                b->m.planes[1].bytesused = s->bufs[i].clen;
                return 0;
            }
        errno = EAGAIN;
        return -1;
    }
    case VIDIOC_STREAMON: s->streaming = 1; logcall("vt streamon"); return 0;
    case VIDIOC_STREAMOFF: s->streaming = 0; logcall("vt streamoff"); return 0;
    default:
        errno = ENOTTY;
        return -1;
    }
}

static int fake_ioctl(int fd, unsigned long req, void *arg)
{
    if (fd < 0 || fd >= MAX_FDS || !fds[fd].used) { errno = EBADF; return -1; }
    struct fd_state *s = &fds[fd];
    switch (s->kind) {
    case K_VENC: return venc_ioctl(s, req, arg);
    case K_HMA: return hma_ioctl(s, req, arg);
    case K_VT: return vt_ioctl(s, req, arg);
    default: errno = EBADF; return -1;
    }
}

static void *fake_mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset)
{
    if (fd == -1)
        return mmap(addr, length, prot, flags, fd, offset); /* the low page */
    if (fd < 0 || fd >= MAX_FDS || !fds[fd].used) { errno = EBADF; return MAP_FAILED; }
    struct fd_state *s = &fds[fd];
    if (s->kind == K_VENC) {
        if (offset != (off_t)AUI_SIZE || length != ES_SIZE) { errno = EINVAL; return MAP_FAILED; }
        return es_mem;
    }
    if (s->kind == K_HMA) {
        if (!s->mem || length > s->size + 8192) { errno = EINVAL; return MAP_FAILED; }
        return s->mem;
    }
    if (s->kind == K_VT) {
        uint32_t idx = (uint32_t)offset / 0x10000;
        if (idx == 0 || (idx - 1) / 2 >= s->nbuf) { errno = EINVAL; return MAP_FAILED; }
        struct vtbuf *b = &s->bufs[(idx - 1) / 2];
        return (idx & 1) ? b->y : b->c;
    }
    errno = EINVAL;
    return MAP_FAILED;
}

static int fake_munmap(void *addr, size_t length)
{
    /* fake mappings are heap memory owned by the fd; only the anonymous low page is a real mapping */
    for (int i = 0; i < MAX_FDS; i++) {
        if (!fds[i].used) continue;
        if (fds[i].kind == K_HMA && fds[i].mem == addr) return 0;
        if (fds[i].kind == K_VT)
            for (uint32_t b = 0; b < 6; b++)
                if (fds[i].bufs[b].y == addr || fds[i].bufs[b].c == addr) return 0;
    }
    if (addr == es_mem) return 0;
    return munmap(addr, length);
}

static int fake_poll(struct pollfd *pfds, nfds_t n, int timeout)
{
    (void)timeout;
    int ready = 0;
    for (nfds_t i = 0; i < n; i++) {
        pfds[i].revents = 0;
        int fd = pfds[i].fd;
        if (fd >= 0 && fd < MAX_FDS && fds[fd].used && fds[fd].kind == K_VT) {
            if (getenv("FAKE_VT_TIMEOUT"))
                continue;
            for (uint32_t b = 0; b < fds[fd].nbuf; b++)
                if (fds[fd].bufs[b].queued || getenv("FAKE_VT_PROTECTED")) {
                    pfds[i].revents = POLLIN;
                    ready++;
                    break;
                }
        }
    }
    return ready;
}

const struct sysops sysops_real = {
    .open = fake_open, .close = fake_close, .ioctl = fake_ioctl, .mmap = fake_mmap, .munmap = fake_munmap, .poll = fake_poll,
};

int sys_fail(char *err, size_t errlen, const char *what)
{
    int e = errno;
    if (err && errlen)
        snprintf(err, errlen, "%s: %s (errno %d)", what, strerror(e), e);
    errno = e;
    return -1;
}

/* for the tests: how many fds are open, and the last instance state */
int fake_open_fds(void)
{
    int n = 0;
    for (int i = 0; i < MAX_FDS; i++)
        n += fds[i].used;
    return n;
}
