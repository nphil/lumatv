/* Host tests of the encoder path against the fake kernel (tests/fake_sys.c): the call order the real
 * driver insists on, the frame bookkeeping, the Annex B reader, the test picture, the video tap session
 * and its refusals. Run by test.sh. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../annexb.h"
#include "../hma.h"
#include "../pools.h"
#include "../synth.h"
#include "../venc.h"
#include "../vtcap.h"
#include "../watchdog.h"

#include <sys/wait.h>
#include <unistd.h>

int fake_open_fds(void);

static int failures;
#define CHECK(cond, ...) do { if (!(cond)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

static void test_annexb(void)
{
    static const uint8_t s[] = { 0, 0, 0, 1, 0x67, 100, 0, 41, 0xaa, 0, 0, 0, 0, 0, 0, 1, 0x68, 0xee, 0, 0, 1, 0x65, 0x88, 0, 0 };
    struct annexb_summary sum;
    annexb_summarize(s, sizeof(s), &sum);
    CHECK(sum.nals == 3, "nals %u", sum.nals);
    CHECK(sum.sps == 1 && sum.pps == 1 && sum.idr == 1, "sps %u pps %u idr %u", sum.sps, sum.pps, sum.idr);
    CHECK(sum.profile_idc == 100 && sum.level_idc == 41, "profile %d level %d", sum.profile_idc, sum.level_idc);
    CHECK(annexb_is_keyframe_unit(s, sizeof(s)), "keyframe unit");
    size_t pos = 0;
    struct annexb_nal n;
    CHECK(annexb_next(s, sizeof(s), &pos, &n) && n.type == 7 && n.len == 5, "first nal type %d len %zu", n.type, n.len);
    CHECK(annexb_next(s, sizeof(s), &pos, &n) && n.type == 8 && n.len == 2, "second nal type %d len %zu", n.type, n.len);
    CHECK(annexb_next(s, sizeof(s), &pos, &n) && n.type == 5 && n.len == 2, "third nal type %d len %zu", n.type, n.len);
    CHECK(!annexb_next(s, sizeof(s), &pos, &n), "end");
    static const uint8_t p[] = { 0, 0, 1, 0x41, 0x9a };
    CHECK(!annexb_is_keyframe_unit(p, sizeof(p)), "p slice is not a keyframe");
    static const uint8_t junk[] = { 1, 2, 3, 0, 0 };
    pos = 0;
    CHECK(!annexb_next(junk, sizeof(junk), &pos, &n), "no start code");
    annexb_summarize(NULL, 0, &sum);
    CHECK(sum.nals == 0 && sum.profile_idc == -1, "empty");
}

static void test_synth(void)
{
    uint32_t w = 320, h = 240;
    uint8_t *y = malloc(w * h), *uv = malloc(w * h / 2);
    for (uint32_t f = 0; f < 70000; f += 2345) {
        synth_nv12(y, uv, w, h, f);
        CHECK(synth_read_counter(y, w, h) == (f & 0xffff), "counter %u read as %u", f, synth_read_counter(y, w, h));
    }
    synth_nv12(y, uv, w, h, 0);
    uint8_t *y1 = malloc(w * h);
    memcpy(y1, y, w * h);
    synth_nv12(y, uv, w, h, 1);
    CHECK(memcmp(y, y1, w * h) != 0, "frames differ");
    int ok = 1;
    for (size_t i = 0; i < w * h; i++)
        if (y[i] < 16) ok = 0;
    CHECK(ok, "luma stays in range");
    free(y); free(uv); free(y1);
}

static void test_venc_sequence(void)
{
    char err[256] = "";
    struct venc v;
    unsigned long arms0 = wd_count();
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) == 0, "open: %s", err);
    CHECK(v.es == NULL && v.buf.ui32ESBufSize == 0 && !v.created, "nothing mapped before create");
    struct venc_frame f;
    CHECK(venc_encode(&v, 0x90000000, 0x90000000, &f, err, sizeof(err)) < 0 && errno == EINVAL, "encode before create refused");
    struct venc_cfg cfg = { .width = 320, .height = 240, .fps = 30, .gop = 4, .kbps = 1000, .profile = LX_VENC_H264_PROFILE_MAIN };
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) == 0, "create: %s", err);
    CHECK(v.created && v.id == 0, "instance created, id %d", v.id);
    CHECK(v.es != NULL && v.buf.ui32ESBufSize == 0x270000 && v.buf.ui32ESBufPhysBase == 0x80100000, "es mapped after create: %p size 0x%x base 0x%x",
          (void *)v.es, v.buf.ui32ESBufSize, v.buf.ui32ESBufPhysBase);
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) < 0 && errno == EBUSY, "second create refused");
    CHECK(wd_count() > arms0, "the watchdog was armed around the calls");

    struct hma_buf in;
    CHECK(hma_alloc(&in, &sysops_real, "photofb", 320 * 240 * 3 / 2, 4096, 0, err, sizeof(err)) == 0, "hma: %s", err);
    CHECK(in.paddr && in.vaddr, "hma paddr 0x%x vaddr %p", in.paddr, (void *)in.vaddr);
    struct hma_buf bad;
    CHECK(hma_alloc(&bad, &sysops_real, "nosuchpool", 4096, 4096, 0, err, sizeof(err)) < 0, "bad pool refused");
    CHECK(bad.fd == -1, "bad alloc closed its fd");

    unsigned keys = 0;
    for (uint32_t i = 0; i < 10; i++) {
        synth_nv12(in.vaddr, in.vaddr + 320 * 240, 320, 240, i);
        CHECK(venc_encode(&v, in.paddr, in.paddr + 320 * 240, &f, err, sizeof(err)) == 0, "encode %u: %s", i, err);
        CHECK(f.data == v.es && f.size > 0 && f.size <= v.buf.ui32ESBufSize, "frame %u in the ring", i);
        CHECK(f.hw_ms == 7, "hw_ms %u", f.hw_ms);
        struct annexb_summary sum;
        annexb_summarize(f.data, f.size, &sum);
        if (f.keyframe) {
            keys++;
            CHECK(sum.sps == 1 && sum.pps == 1 && sum.idr == 1 && sum.slices == 0, "I frame %u: sps %u pps %u idr %u", i, sum.sps, sum.pps, sum.idr);
            CHECK(sum.profile_idc == 77, "main profile in sps: %d", sum.profile_idc);
            CHECK(annexb_is_keyframe_unit(f.data, f.size), "keyframe unit %u", i);
        } else {
            CHECK(sum.sps == 0 && sum.idr == 0 && sum.slices == 1, "P frame %u: sps %u idr %u slices %u", i, sum.sps, sum.idr, sum.slices);
        }
    }
    CHECK(keys == 3, "gop 4 over 10 frames gives 3 I frames, got %u", keys);
    CHECK(v.frames == 10, "frames counted %llu", (unsigned long long)v.frames);
    CHECK(venc_force_idr(&v, err, sizeof(err)) == 0, "idr: %s", err);
    CHECK(venc_encode(&v, in.paddr, in.paddr + 320 * 240, &f, err, sizeof(err)) == 0 && f.keyframe, "forced idr");
    CHECK(venc_set_bitrate(&v, 2500, err, sizeof(err)) == 0 && v.cfg.kbps == 2500, "bitrate change");
    CHECK(venc_set_bitrate(&v, 0, err, sizeof(err)) < 0, "bitrate 0 refused");
    CHECK(venc_encode(&v, 0x12345670, 0x12345670, &f, err, sizeof(err)) < 0, "unknown physical address refused: %s", err);
    CHECK(venc_encode(&v, 0x12345678, 0x12345678, &f, err, sizeof(err)) < 0 && errno == EINVAL, "unaligned address refused locally");
    CHECK(venc_encode(&v, 0, in.paddr, &f, err, sizeof(err)) < 0 && errno == EINVAL, "zero address refused locally");
    hma_free(&in);
    venc_close(&v);
    CHECK(v.fd == -1 && v.id == -1 && v.es == NULL && !v.created, "closed");
    venc_close(&v);
    CHECK(fake_open_fds() == 0, "fds left open: %d", fake_open_fds());
}

static void test_venc_gate(void)
{
    char err[256] = "";
    struct venc v;
    setenv("FAKE_VENC_DENY_MAGIC", "1", 1);
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) < 0 && strstr(err, "KADP"), "gate refusal reported: %s", err);
    CHECK(fake_open_fds() == 0, "fd closed after refusal");
    unsetenv("FAKE_VENC_DENY_MAGIC");
    struct venc_cfg bad = { .width = 4096, .height = 2160, .fps = 30 };
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) == 0, "open");
    CHECK(venc_create(&v, &bad, err, sizeof(err)) < 0 && errno == EINVAL, "4K refused locally");
    venc_close(&v);
    /* a hung encode call is ended by the watchdog with exit 75, in a child so the test survives */
    pid_t pid = fork();
    if (pid == 0) {
        setenv("FAKE_VENC_HANG", "1", 1);
        struct venc h;
        struct venc_cfg c2 = { .width = 320, .height = 240, .fps = 30, .gop = 4, .kbps = 1000 };
        struct hma_buf hb;
        if (venc_open(&h, &sysops_real, NULL, 0) || venc_create(&h, &c2, NULL, 0) || hma_alloc(&hb, &sysops_real, "photofb", 320 * 240 * 3 / 2, 4096, 0, NULL, 0))
            _exit(3);
        struct venc_frame hf;
        venc_encode(&h, hb.paddr, hb.paddr + 320 * 240, &hf, NULL, 0);
        _exit(4); /* not reached: the watchdog ends the process */
    }
    int status = 0;
    CHECK(pid > 0 && waitpid(pid, &status, 0) == pid, "fork/wait");
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 75, "hung encode ended by the watchdog (status 0x%x)", status);
    LX_VENC_KADPCALL_PARAM_T m;
    venc_kadp_magic(&m);
    CHECK(!memcmp(m.kadpmagic1, "SIC\0\0", 5) && !memcmp(m.kadpmagic2, "KADP\0", 5) && !memcmp(m.kadpmagic3, "VENC\0", 5), "magic bytes");
}

static void test_vtcap(void)
{
    char err[256] = "";
    struct vtcap c;
    CHECK(vtcap_open(&c, &sysops_real, V4L2_EXT_CAPTURE_BLENDED_OUTPUT, 1000, 540, 4, 1, err, sizeof(err)) < 0, "width not a multiple of 16 refused");
    CHECK(vtcap_open(&c, &sysops_real, V4L2_EXT_CAPTURE_SCALER_INPUT, 960, 540, 4, 1, err, sizeof(err)) < 0 && strstr(err, "PLANE_PROP"), "unsupported point: %s", err);
    CHECK(fake_open_fds() == 0, "fd closed after refusal");
    CHECK(vtcap_open(&c, &sysops_real, V4L2_EXT_CAPTURE_BLENDED_OUTPUT, 960, 544, 4, 2, err, sizeof(err)) == 0, "open: %s", err);
    CHECK(!c.streaming && c.stride == 960 && c.nqueued == 4, "open leaves streaming off, stride %u, %u queued", c.stride, c.nqueued);
    for (uint32_t i = 0; i < 4; i++)
        CHECK(c.paddr_y[i] && c.paddr_c[i] && c.plane[i][0] && c.plane[i][1] && c.queued[i], "buffer %u addresses and queued", i);
    struct vtcap_frame f;
    CHECK(vtcap_dequeue(&c, &f, 10, err, sizeof(err)) < 0 && strstr(err, "not streaming"), "dequeue before start refused: %s", err);
    CHECK(vtcap_requeue(&c, 0, err, sizeof(err)) < 0 && errno == EINVAL, "requeue of a queued buffer refused");
    CHECK(vtcap_start(&c, err, sizeof(err)) == 0 && c.streaming, "start: %s", err);
    for (int i = 0; i < 9; i++) {
        int r = vtcap_dequeue(&c, &f, 100, err, sizeof(err));
        CHECK(r == 0, "dequeue %d -> %d %s", i, r, err);
        CHECK(f.y_len == 960 * 544 && f.c_len == 960 * 272, "plane sizes %u %u", f.y_len, f.c_len);
        CHECK(synth_read_counter(f.y, 960, 544) == (uint32_t)i, "frame %d carries counter %u", i, synth_read_counter(f.y, 960, 544));
        CHECK(c.nqueued == 3 && !c.queued[f.index], "one buffer held by the session");
        CHECK(vtcap_requeue(&c, f.index, err, sizeof(err)) == 0 && c.nqueued == 4, "requeue");
    }
    /* with every buffer dequeued and none given back, the next dequeue is refused before it reaches the driver */
    {
        struct vtcap_frame held[4];
        for (int i = 0; i < 4; i++)
            CHECK(vtcap_dequeue(&c, &held[i], 100, err, sizeof(err)) == 0, "hold %d", i);
        CHECK(c.nqueued == 0, "all held");
        CHECK(vtcap_dequeue(&c, &f, 10, err, sizeof(err)) < 0 && strstr(err, "no buffer is queued"), "dequeue with nothing queued refused: %s", err);
        for (int i = 0; i < 4; i++)
            CHECK(vtcap_requeue(&c, held[i].index, err, sizeof(err)) == 0, "give back %d", i);
    }
    /* the encoder can read the tap's buffers by physical address */
    struct venc v;
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) == 0, "venc open");
    struct venc_cfg cfg = { .width = 960, .height = 544, .fps = 30, .gop = 30, .kbps = 3000 };
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) == 0 && v.es, "venc create: %s", err);
    CHECK(vtcap_dequeue(&c, &f, 100, err, sizeof(err)) == 0, "dequeue for encode");
    struct venc_frame out;
    CHECK(venc_encode(&v, f.paddr_y, f.paddr_c, &out, err, sizeof(err)) == 0 && out.keyframe, "encode tap frame: %s", err);
    vtcap_requeue(&c, f.index, err, sizeof(err));
    venc_close(&v);
    setenv("FAKE_VT_PROTECTED", "1", 1);
    CHECK(vtcap_dequeue(&c, &f, 100, err, sizeof(err)) == 2, "protected content reported as refused");
    unsetenv("FAKE_VT_PROTECTED");
    setenv("FAKE_VT_TIMEOUT", "1", 1);
    CHECK(vtcap_dequeue(&c, &f, 10, err, sizeof(err)) == 1, "timeout reported");
    unsetenv("FAKE_VT_TIMEOUT");
    vtcap_close(&c);
    CHECK(c.fd == -1 && !c.streaming, "closed");
    vtcap_close(&c);
    CHECK(fake_open_fds() == 0, "fds left open: %d", fake_open_fds());
    struct v4l2_ext_capture_capability_info cap;
    CHECK(vtcap_capability(&sysops_real, &cap, err, sizeof(err)) == 0 && cap.max_res.w == 1920 && cap.num_plane == 2, "capability");
    enum v4l2_ext_capture_location l;
    CHECK(vtcap_location_parse("osd", &l) == 0 && l == V4L2_EXT_CAPTURE_OSD_OUTPUT, "parse osd");
    CHECK(vtcap_location_parse("tv", &l) < 0, "parse unknown");
    CHECK(!strcmp(vtcap_location_name(V4L2_EXT_CAPTURE_BLENDED_OUTPUT), "blended"), "name");
}

static void test_pools(void)
{
    struct pools ps;
    const char *text = "0x0000000076200000\t0x0c00000\tVENC_S_hma\n0x0000000077200000\t0x4200000\tcpb\n0x0000000080e28000\t0x1800000\tde-vt-m0\njunk line\n";
    CHECK(pools_parse(&ps, text) == 3, "parsed %u pools", ps.n);
    CHECK(!strcmp(ps.p[1].name, "cpb") && ps.p[1].base == 0x77200000 && ps.p[1].size == 0x4200000, "cpb row");
    CHECK(pools_find(&ps, 0x77200000, 0x270000) == &ps.p[1], "inside cpb");
    CHECK(pools_find(&ps, 0x80e28000 + 0x1800000 - 0x1000, 0x2000) == NULL, "crossing the end of de-vt-m0");
    CHECK(pools_find(&ps, 0x12345000, 0x1000) == NULL, "nowhere");
    CHECK(pools_read(&ps, "/nonexistent/hma/status") == 0, "absent file");
}

int main(void)
{
    test_pools();
    test_annexb();
    test_synth();
    test_venc_sequence();
    test_venc_gate();
    test_vtcap();
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    puts("venc_test: ok");
    return 0;
}
