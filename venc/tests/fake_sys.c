/* A fake of the devices the encoder and decoder paths touch, behind the sysops table, so the sessions and the
 * probe run on the host: /dev/venc as ipc/local/venc.c behaves (KADP gate first, buffer map, instance
 * create/encode/destroy, mmap of the ES ring at offset = AUI size), /dev/hma as hma_drv.c (one
 * allocation per fd, physical addresses handed out from a bump allocator, mmap of the allocation, dma-buf
 * export), /dev/video60 as v4l2_vt.c and LG's library use it (plane prop, format, MMAP buffers, physical-address
 * query through two pointers, QBUF/DQBUF, STREAMON/OFF), and /dev/video21 as v4l2_vdec.c does (stateful
 * decoder: MMAP bitstream buffers, DMABUF picture buffers, the source-change handshake, stop with a LAST buffer).
 * The fake encoder "reads" the input frame through the physical addresses it knows and writes an Annex B unit
 * whose payload carries a checksum of it. The fake decoder takes access units made by fake_vdec_au() and
 * writes the picture fake_vdec_pattern() gives for the unit's id.
 *
 * Real descriptors (stdin and stdout, or a pipe numbered 32 or more) can be passed to poll next to fake ones.
 *
 * Knobs (environment): FAKE_VENC_DENY_MAGIC=1 refuses the gate; FAKE_VENC_HANG=1 makes the encode call never return; FAKE_VT_PROTECTED=1 answers DQBUF with
 * EACCES (protected content); FAKE_VT_TIMEOUT=1 never has a frame; FAKE_VT_NO_BLENDED=1 refuses the blended
 * tap point with EOPNOTSUPP; FAKE_VDEC_BUSY=1 refuses STREAMON with ENOMEM (no decoder memory); FAKE_VDEC_STALL=1
 * takes access units and never makes a picture; FAKE_VDEC_REORDER=1 shows each pair of pictures in swapped order;
 * FAKE_VDEC_ERROR_EVERY=N flags every Nth picture as not decoded; FAKE_LOG=path appends one line per call. */
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
#include <unistd.h>

#include "../hma_abi.h"
#include "../synth.h"
#include "../sysops.h"
#include "../venc_abi.h"
#include "../vt_abi.h"
#include "../vdec_abi.h"

enum kind { K_NONE, K_VENC, K_HMA, K_VT, K_VDEC, K_DMABUF };

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

/* The decoder's state: pictures are made at QBUF of the bitstream buffer (an access unit is "FAKE", width and height as
 * u16, id as u32), wait for a queued picture buffer, and come out in the driver's order. */
#define VD_OUT 8
#define VD_CAP 12
#define VD_PEND 16

struct fake_pic { uint32_t id, w, h; uint64_t sec, usec; int err; };

struct fake_vdec {
    uint32_t codec, out_size;
    uint32_t cap_w, cap_h; /* the CAPTURE format: the size the driver writes next */
    uint32_t max_w, max_h; /* the size the buffers were made for */
    int subscribed, res_set;
    uint32_t nout, ncap;
    uint8_t *outmem[VD_OUT];
    int outstate[VD_OUT]; /* 0: with the caller, 2: taken in, waiting for DQBUF */
    struct {
        int state; /* 0: with the caller, 1: queued, 2: filled, waiting for DQBUF */
        uint8_t *mem;
        size_t size;
        struct fake_pic pic;
        int last;
        unsigned order;
    } cap[VD_CAP];
    int out_stream, cap_stream;
    struct fake_pic pend[VD_PEND];
    int npend;
    struct fake_pic held;
    int has_held, event_pending, need_cycle, draining, last_sent;
    unsigned order;
    uint64_t decoded;
};

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
    /* vdec */
    struct fake_vdec vd;
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
    if (!strcmp(path, VDEC_DEV))
        return alloc_fd(K_VDEC);
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
    logcall("close %s inst=%d", s->kind == K_VENC ? "venc" : s->kind == K_HMA ? "hma" : s->kind == K_VT ? "vt" : s->kind == K_VDEC ? "vdec" : "dmabuf", s->inst);
    if (s->kind == K_HMA && s->mem) {
        del_region(s->paddr);
        free(s->mem);
    }
    if (s->kind == K_VDEC)
        for (int i = 0; i < VD_OUT; i++)
            free(s->vd.outmem[i]);
    if (s->kind == K_VT)
        for (int i = 0; i < 6; i++) {
            if (s->bufs[i].y) { del_region(s->bufs[i].py); free(s->bufs[i].y); }
            if (s->bufs[i].c) { del_region(s->bufs[i].pc); free(s->bufs[i].c); }
        }
    memset(s, 0, sizeof(*s));
    return 0;
}

/* the encoder's checksum of a frame, which ends up in the unit's payload (the tests compute the same to see what it was given) */
uint32_t fake_nv12_hash(const uint8_t *y, const uint8_t *c, uint32_t w, uint32_t h)
{
    uint32_t sum = 0;
    for (size_t i = 0; i < (size_t)w * h; i += 97)
        sum = sum * 31 + y[i];
    for (size_t i = 0; i < (size_t)w * h / 2; i += 89)
        sum = sum * 31 + c[i];
    return sum;
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
    uint32_t sum = fake_nv12_hash(y, c, s->w, s->h);
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
        /* the MCU fills this from the instance's memory, which ENC_Create allocates: all zeros before it
         * (h1encode.c, H1EncodeGetBufferInfo / H1EncodeMemoryInit; the first slot measured the zeros) */
        LX_VENC_BUFFER_T *b = arg;
        memset(b, 0, sizeof(*b));
        if (s->inst >= 0) {
            b->ui32ESBufPhysBase = ES_BASE;
            b->ui32ESBufSize = ES_SIZE;
            b->ui32AUIBufPhysBase = AUI_BASE;
            b->ui32AUIBufSize = AUI_SIZE;
        }
        logcall("venc bufinfo inst=%d", s->inst);
        return 0;
    }
    if (req == LX_VENC_IOWR_ENC_CREATE) {
        LX_VENC_ENC_CREATE_T *c = arg;
        if (s->inst >= 0 || c->width < 96 || c->width > 1920 || c->height > 1080 || c->codecType != LX_VENC_ENCODE_TYPE_H264 ||
            c->imageType != LX_VENC_IMAGE_YUV420_SEMIPLANAR) {
            errno = EINVAL;
            return -1;
        }
        s->inst = 0; /* pstCreate->id = handle->id, never set: 0 on the real MCU */
        c->id = 0;
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
        if (getenv("FAKE_VENC_HANG")) {
            logcall("venc encode hangs");
            for (;;)
                pause();
        }
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
    if (req == HMA_IOCTL_GET_DMABUF) {
        if (!s->mem) { errno = EINVAL; return -1; }
        int d = alloc_fd(K_DMABUF); /* an fd of its own that names the same memory */
        if (d < 0)
            return -1;
        fds[d].paddr = s->paddr;
        fds[d].size = s->size;
        fds[d].mem = s->mem;
        *(int32_t *)arg = d;
        return d;
    }
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

/* ---- the decoder ---- */

/* an access unit for the fake decoder, and the picture it turns into */
size_t fake_vdec_au(uint8_t *out, uint32_t w, uint32_t h, uint32_t id)
{
    memcpy(out, "FAKE", 4);
    out[4] = (uint8_t)w;
    out[5] = (uint8_t)(w >> 8);
    out[6] = (uint8_t)h;
    out[7] = (uint8_t)(h >> 8);
    for (int i = 0; i < 4; i++)
        out[8 + i] = (uint8_t)(id >> (8 * i));
    return 12;
}

void fake_vdec_pattern(uint8_t *y, uint8_t *c, uint32_t w, uint32_t h, uint32_t id)
{
    for (uint32_t r = 0; r < h; r++)
        for (uint32_t x = 0; x < w; x++)
            y[(size_t)r * w + x] = (uint8_t)(id * 7 + r * 3 + x);
    for (uint32_t r = 0; r < h / 2; r++)
        for (uint32_t x = 0; x < w / 2; x++) {
            c[((size_t)r * (w / 2) + x) * 2] = (uint8_t)(id * 5 + r * 2 + x);
            c[((size_t)r * (w / 2) + x) * 2 + 1] = (uint8_t)(255 - (id * 3 + r + x * 2));
        }
}

static void vd_push(struct fake_vdec *v, const struct fake_pic *p)
{
    if (v->npend == VD_PEND)
        memmove(&v->pend[0], &v->pend[1], (VD_PEND - 1) * sizeof(v->pend[0])), v->npend--;
    v->pend[v->npend++] = *p;
}

/* Gives every queued picture buffer a picture, or the LAST marker once a stop has drained everything. */
static void vd_deliver(struct fake_vdec *v)
{
    if (!v->cap_stream)
        return;
    for (;;) {
        /* the event comes after every picture of the old size has been delivered, when the first of the new size is next */
        if (v->npend && !v->need_cycle && v->subscribed && (v->pend[0].w != v->cap_w || v->pend[0].h != v->cap_h)) {
            v->cap_w = v->pend[0].w;
            v->cap_h = v->pend[0].h;
            v->event_pending = 1;
            v->need_cycle = 1; /* the picture waits until the caller has cycled the CAPTURE queue */
        }
        int ci = -1;
        for (uint32_t i = 0; i < v->ncap; i++)
            if (v->cap[i].state == 1) {
                ci = (int)i;
                break;
            }
        if (ci < 0)
            return;
        if (v->npend && !v->need_cycle) {
            struct fake_pic p = v->pend[0];
            memmove(&v->pend[0], &v->pend[1], (size_t)(v->npend - 1) * sizeof(v->pend[0]));
            v->npend--;
            size_t luma = (size_t)p.w * p.h;
            if (luma * 3 / 2 > v->cap[ci].size) {
                logcall("vdec picture %ux%u does not fit a %zu byte buffer", p.w, p.h, v->cap[ci].size); /* the real driver would overrun it */
                p.err = 1;
            } else {
                fake_vdec_pattern(v->cap[ci].mem, v->cap[ci].mem + luma, p.w, p.h, p.id);
            }
            v->cap[ci].pic = p;
            v->cap[ci].last = 0;
        } else if (v->draining && !v->npend && !v->last_sent) {
            v->cap[ci].last = 1;
            v->last_sent = 1;
        } else {
            return;
        }
        v->cap[ci].state = 2;
        v->cap[ci].order = v->order++;
    }
}

static void vd_decode(struct fake_vdec *v, uint32_t used, const uint8_t *au, uint64_t sec, uint64_t usec)
{
    if (used < 12 || memcmp(au, "FAKE", 4) || getenv("FAKE_VDEC_STALL"))
        return;
    struct fake_pic p = { .w = (uint32_t)au[4] | (uint32_t)au[5] << 8, .h = (uint32_t)au[6] | (uint32_t)au[7] << 8,
                          .id = (uint32_t)au[8] | (uint32_t)au[9] << 8 | (uint32_t)au[10] << 16 | (uint32_t)au[11] << 24, .sec = sec, .usec = usec };
    if (p.w > v->max_w || p.h > v->max_h)
        logcall("vdec picture %ux%u is larger than the %ux%u the buffers are for", p.w, p.h, v->max_w, v->max_h);
    v->decoded++;
    const char *every = getenv("FAKE_VDEC_ERROR_EVERY");
    p.err = every && atoi(every) > 0 && v->decoded % (uint64_t)atoi(every) == 0;
    if (getenv("FAKE_VDEC_REORDER")) {
        if (!v->has_held) {
            v->held = p;
            v->has_held = 1;
            return;
        }
        vd_push(v, &p);
        vd_push(v, &v->held);
        v->has_held = 0;
        return;
    }
    vd_push(v, &p);
}

static short vd_ready(const struct fake_vdec *v, short events)
{
    short r = 0;
    for (uint32_t i = 0; i < v->ncap; i++)
        if (v->cap[i].state == 2 && (events & (POLLIN | POLLRDNORM)))
            r |= POLLIN;
    for (uint32_t i = 0; i < v->nout; i++)
        if (v->outstate[i] == 2 && (events & POLLOUT))
            r |= POLLOUT;
    if (v->event_pending && (events & POLLPRI))
        r |= POLLPRI;
    return r;
}

static int vdec_ioctl(struct fd_state *s, unsigned long req, void *arg)
{
    struct fake_vdec *v = &s->vd;
    switch (req) {
    case VIDIOC_QUERYCAP: {
        struct v4l2_capability *c = arg;
        memset(c, 0, sizeof(*c));
        strcpy((char *)c->driver, "vdec-decoder");
        strcpy((char *)c->card, "LG Decoder");
        c->capabilities = V4L2_CAP_VIDEO_M2M_MPLANE | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;
        c->device_caps = V4L2_CAP_VIDEO_M2M_MPLANE | V4L2_CAP_STREAMING;
        return 0;
    }
    case VIDIOC_S_FMT:
    case VIDIOC_G_FMT: {
        struct v4l2_format *f = arg;
        struct v4l2_pix_format_mplane *p = &f->fmt.pix_mp;
        if (f->type == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE) {
            if (req == VIDIOC_S_FMT) {
                if (p->pixelformat != V4L2_PIX_FMT_MPEG2 && p->pixelformat != V4L2_PIX_FMT_H264) { errno = EINVAL; return -1; }
                v->codec = p->pixelformat;
                v->out_size = p->plane_fmt[0].sizeimage;
                v->cap_w = v->max_w = p->width; /* the CAPTURE format follows the OUTPUT one until the stream says otherwise */
                v->cap_h = v->max_h = p->height;
            }
            p->pixelformat = v->codec;
            p->width = v->max_w;
            p->height = v->max_h;
            p->num_planes = 1;
            p->plane_fmt[0].sizeimage = v->out_size;
            return 0;
        }
        if (f->type == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) {
            if (req == VIDIOC_S_FMT) {
                v->cap_w = v->max_w = p->width;
                v->cap_h = v->max_h = p->height;
            }
            p->pixelformat = V4L2_PIX_FMT_NV12;
            p->width = v->cap_w;
            p->height = v->cap_h;
            p->num_planes = 1;
            p->plane_fmt[0].sizeimage = v->cap_w * v->cap_h / 2 * 3;
            return 0;
        }
        errno = EINVAL;
        return -1;
    }
    case VIDIOC_QUERY_EXT_CTRL: {
        struct v4l2_query_ext_ctrl *q = arg;
        if ((q->id & V4L2_CTRL_ID_MASK) >= VDEC_CID_EXT_RES_INFO) { errno = EINVAL; return -1; }
        memset(q, 0, sizeof(*q));
        q->id = VDEC_CID_EXT_RES_INFO;
        q->type = 0x1001;
        strcpy((char *)q->name, VDEC_RES_INFO_NAME);
        q->elem_size = sizeof(struct vdec_res_info);
        q->elems = 1;
        return 0;
    }
    case VIDIOC_S_EXT_CTRLS: {
        struct v4l2_ext_controls *cs = arg;
        if (cs->count != 1 || cs->controls[0].id != VDEC_CID_EXT_RES_INFO || cs->controls[0].size != sizeof(struct vdec_res_info)) { errno = EINVAL; return -1; }
        v->res_set = 1;
        return 0;
    }
    case VIDIOC_SUBSCRIBE_EVENT: {
        struct v4l2_event_subscription *sub = arg;
        if (sub->type != V4L2_EVENT_SOURCE_CHANGE) { errno = EINVAL; return -1; }
        v->subscribed = 1;
        return 0;
    }
    case VIDIOC_DQEVENT: {
        struct v4l2_event *ev = arg;
        if (!v->event_pending) { errno = ENOENT; return -1; }
        memset(ev, 0, sizeof(*ev));
        ev->type = V4L2_EVENT_SOURCE_CHANGE;
        ev->u.src_change.changes = V4L2_EVENT_SRC_CH_RESOLUTION;
        v->event_pending = 0;
        return 0;
    }
    case VIDIOC_REQBUFS: {
        struct v4l2_requestbuffers *r = arg;
        if (r->type == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE && r->memory == V4L2_MEMORY_MMAP) {
            for (uint32_t i = 0; i < VD_OUT; i++) {
                free(v->outmem[i]);
                v->outmem[i] = NULL;
                v->outstate[i] = 0;
            }
            v->nout = r->count > VD_OUT ? VD_OUT : r->count;
            for (uint32_t i = 0; i < v->nout; i++)
                v->outmem[i] = calloc(1, v->out_size);
            r->count = v->nout;
            return 0;
        }
        if (r->type == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE && r->memory == V4L2_MEMORY_DMABUF) {
            v->ncap = r->count > VD_CAP ? VD_CAP : r->count;
            memset(v->cap, 0, sizeof(v->cap));
            r->count = v->ncap;
            return 0;
        }
        errno = EINVAL;
        return -1;
    }
    case VIDIOC_QUERYBUF: {
        struct v4l2_buffer *b = arg;
        if (b->type != V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE || b->index >= v->nout || b->length < 1) { errno = EINVAL; return -1; }
        b->m.planes[0].length = v->out_size;
        b->m.planes[0].m.mem_offset = 0x10000 * (b->index + 1);
        return 0;
    }
    case VIDIOC_QBUF: {
        struct v4l2_buffer *b = arg;
        if (b->type == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE) {
            if (!v->out_stream || b->index >= v->nout || v->outstate[b->index] || b->memory != V4L2_MEMORY_MMAP) { errno = EINVAL; return -1; }
            vd_decode(v, b->m.planes[0].bytesused, v->outmem[b->index], (uint64_t)b->timestamp.tv_sec, (uint64_t)b->timestamp.tv_usec);
            v->outstate[b->index] = 2;
            vd_deliver(v);
            return 0;
        }
        if (b->type == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) {
            int dfd = b->m.planes[0].m.fd;
            if (b->index >= v->ncap || v->cap[b->index].state || b->memory != V4L2_MEMORY_DMABUF) { errno = EINVAL; return -1; }
            if (dfd < 0 || dfd >= MAX_FDS || !fds[dfd].used || fds[dfd].kind != K_DMABUF) { errno = EBADF; return -1; }
            v->cap[b->index].mem = fds[dfd].mem;
            v->cap[b->index].size = fds[dfd].size;
            v->cap[b->index].state = 1;
            vd_deliver(v);
            return 0;
        }
        errno = EINVAL;
        return -1;
    }
    case VIDIOC_DQBUF: {
        struct v4l2_buffer *b = arg;
        if (b->type == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE) {
            for (uint32_t i = 0; i < v->nout; i++)
                if (v->outstate[i] == 2) {
                    v->outstate[i] = 0;
                    b->index = i;
                    b->flags = 0;
                    return 0;
                }
            errno = EAGAIN;
            return -1;
        }
        if (b->type == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) {
            int best = -1;
            for (uint32_t i = 0; i < v->ncap; i++)
                if (v->cap[i].state == 2 && (best < 0 || v->cap[i].order < v->cap[best].order))
                    best = (int)i;
            if (best < 0) { errno = EAGAIN; return -1; }
            v->cap[best].state = 0;
            b->index = (uint32_t)best;
            b->timestamp.tv_sec = (time_t)v->cap[best].pic.sec;
            b->timestamp.tv_usec = (suseconds_t)v->cap[best].pic.usec;
            b->flags = v->cap[best].last ? V4L2_BUF_FLAG_LAST : v->cap[best].pic.err ? V4L2_BUF_FLAG_ERROR : 0;
            b->m.planes[0].bytesused = v->cap[best].last ? 0 : v->cap[best].pic.w * v->cap[best].pic.h / 2 * 3;
            return 0;
        }
        errno = EINVAL;
        return -1;
    }
    case VIDIOC_STREAMON: {
        enum v4l2_buf_type t = *(enum v4l2_buf_type *)arg;
        if (getenv("FAKE_VDEC_BUSY")) { errno = ENOMEM; return -1; }
        if (t == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE)
            v->out_stream = 1;
        else if (t == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) {
            v->cap_stream = 1;
            v->need_cycle = 0;
            vd_deliver(v);
        } else {
            errno = EINVAL;
            return -1;
        }
        logcall("vdec streamon %d", (int)t);
        return 0;
    }
    case VIDIOC_STREAMOFF: {
        enum v4l2_buf_type t = *(enum v4l2_buf_type *)arg;
        if (t == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE) {
            v->out_stream = 0;
            memset(v->outstate, 0, sizeof(v->outstate));
            v->npend = 0;
            v->has_held = v->draining = v->last_sent = 0;
        } else if (t == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE) {
            v->cap_stream = 0;
            for (uint32_t i = 0; i < v->ncap; i++)
                v->cap[i].state = 0;
        } else {
            errno = EINVAL;
            return -1;
        }
        logcall("vdec streamoff %d", (int)t);
        return 0;
    }
    case VIDIOC_DECODER_CMD: {
        struct v4l2_decoder_cmd *c = arg;
        if (c->cmd != V4L2_DEC_CMD_STOP) { errno = EINVAL; return -1; }
        if (v->has_held) {
            vd_push(v, &v->held);
            v->has_held = 0;
        }
        v->draining = 1;
        vd_deliver(v);
        logcall("vdec stop");
        return 0;
    }
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
    case K_VDEC: return vdec_ioctl(s, req, arg);
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
        if (s->inst < 0 || offset != (off_t)AUI_SIZE || length != ES_SIZE) { errno = EINVAL; return MAP_FAILED; }
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
    if (s->kind == K_VDEC) {
        uint32_t idx = (uint32_t)offset / 0x10000;
        if (idx == 0 || idx > s->vd.nout || length > s->vd.out_size) { errno = EINVAL; return MAP_FAILED; }
        return s->vd.outmem[idx - 1];
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
        if (fds[i].kind == K_VDEC)
            for (uint32_t b = 0; b < VD_OUT; b++)
                if (fds[i].vd.outmem[b] == addr) return 0;
    }
    if (addr == es_mem) return 0;
    return munmap(addr, length);
}

static int fake_is_fake(int fd)
{
    return fd >= 3 && fd < MAX_FDS;
}

/* Fake descriptors answer from their state; the others are real ones (the pipes the test drives the program with) and go to
 * poll(2), without waiting when a fake one is already ready. */
static int fake_poll(struct pollfd *pfds, nfds_t n, int timeout)
{
    int ready = 0;
    struct pollfd real[8];
    nfds_t map[8], nreal = 0;
    for (nfds_t i = 0; i < n; i++) {
        pfds[i].revents = 0;
        int fd = pfds[i].fd;
        if (!fake_is_fake(fd)) {
            if (nreal < 8) {
                real[nreal] = pfds[i];
                map[nreal++] = i;
            }
            continue;
        }
        if (!fds[fd].used)
            continue;
        if (fds[fd].kind == K_VDEC) {
            pfds[i].revents = vd_ready(&fds[fd].vd, pfds[i].events);
            ready += pfds[i].revents != 0;
        } else if (fds[fd].kind == K_VT) {
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
    if (nreal) {
        if (poll(real, nreal, ready ? 0 : timeout) > 0)
            for (nfds_t j = 0; j < nreal; j++) {
                pfds[map[j]].revents = real[j].revents;
                ready += real[j].revents != 0;
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
