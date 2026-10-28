/* Host tests of the encoder path against the fake kernel (tests/fake_sys.c): the call order the real
 * driver insists on, the frame bookkeeping, the Annex B reader, the test picture, the video tap session
 * and its refusals. Run by test.sh. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../annexb.h"
#include "../hma.h"
#include "../synth.h"
#include "../venc.h"
#include "../vtcap.h"

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
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) == 0, "open: %s", err);
    CHECK(v.es != NULL && v.buf.ui32ESBufSize == 0x270000, "es mapped %p size 0x%x", (void *)v.es, v.buf.ui32ESBufSize);
    struct venc_frame f;
    CHECK(venc_encode(&v, 0x90000000, 0x90000000, &f, err, sizeof(err)) < 0 && errno == EINVAL, "encode before create refused");
    struct venc_cfg cfg = { .width = 320, .height = 240, .fps = 30, .gop = 4, .kbps = 1000, .profile = LX_VENC_H264_PROFILE_MAIN };
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) == 0, "create: %s", err);
    CHECK(v.id == 7, "instance id %d", v.id);
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) < 0, "second create refused");

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
    CHECK(venc_encode(&v, 0x12345678, 0x12345678, &f, err, sizeof(err)) < 0, "unknown physical address refused: %s", err);
    hma_free(&in);
    venc_close(&v);
    CHECK(v.fd == -1 && v.id == -1 && v.es == NULL, "closed");
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
    CHECK(c.streaming && c.stride == 960, "streaming, stride %u", c.stride);
    for (uint32_t i = 0; i < 4; i++)
        CHECK(c.paddr_y[i] && c.paddr_c[i] && c.plane[i][0] && c.plane[i][1], "buffer %u addresses", i);
    struct vtcap_frame f;
    for (int i = 0; i < 9; i++) {
        int r = vtcap_dequeue(&c, &f, 100, err, sizeof(err));
        CHECK(r == 0, "dequeue %d -> %d %s", i, r, err);
        CHECK(f.y_len == 960 * 544 && f.c_len == 960 * 272, "plane sizes %u %u", f.y_len, f.c_len);
        CHECK(synth_read_counter(f.y, 960, 544) == (uint32_t)i, "frame %d carries counter %u", i, synth_read_counter(f.y, 960, 544));
        CHECK(vtcap_requeue(&c, f.index, err, sizeof(err)) == 0, "requeue");
    }
    /* the encoder can read the tap's buffers by physical address */
    struct venc v;
    CHECK(venc_open(&v, &sysops_real, err, sizeof(err)) == 0, "venc open");
    struct venc_cfg cfg = { .width = 960, .height = 544, .fps = 30, .gop = 30, .kbps = 3000 };
    CHECK(venc_create(&v, &cfg, err, sizeof(err)) == 0, "venc create: %s", err);
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

int main(void)
{
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
