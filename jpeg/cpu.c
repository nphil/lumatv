#define _GNU_SOURCE
#include "cpu.h"

#include <dlfcn.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "libjpeg62/jpeglib.h"

struct api {
    void *lib;
    struct jpeg_error_mgr *(*std_error)(struct jpeg_error_mgr *);
    void (*create)(j_decompress_ptr, int, size_t);
    int (*read_header)(j_decompress_ptr, boolean);
    boolean (*start)(j_decompress_ptr);
    JDIMENSION (*read_raw)(j_decompress_ptr, JSAMPIMAGE, JDIMENSION);
    void (*destroy)(j_decompress_ptr);
};

static struct api g_api;

static int load(void) {
    if (g_api.lib) return CPU_OK;
    void *lib = dlopen("libjpeg.so.62", RTLD_NOW | RTLD_LOCAL);
    if (!lib) return CPU_NO_LIBRARY;
    struct api a = {.lib = lib};
    *(void **)&a.std_error = dlsym(lib, "jpeg_std_error");
    *(void **)&a.create = dlsym(lib, "jpeg_CreateDecompress");
    *(void **)&a.read_header = dlsym(lib, "jpeg_read_header");
    *(void **)&a.start = dlsym(lib, "jpeg_start_decompress");
    *(void **)&a.read_raw = dlsym(lib, "jpeg_read_raw_data");
    *(void **)&a.destroy = dlsym(lib, "jpeg_destroy_decompress");
    if (!a.std_error || !a.create || !a.read_header || !a.start || !a.read_raw || !a.destroy) {
        dlclose(lib);
        return CPU_NO_LIBRARY;
    }
    g_api = a;
    return CPU_OK;
}

struct errmgr {
    struct jpeg_error_mgr pub;
    jmp_buf jb;
};

static void on_error(j_common_ptr c) {
    longjmp(((struct errmgr *)c->err)->jb, 1);
}

static void on_message(j_common_ptr c) {
    (void)c;
}

/* An in-memory source. A file cut short ends in a made-up EOI, which libjpeg reads as the end of the data (the rows it has not
 * got are grey), the way every decoder shows a truncated download. */
static void src_init(j_decompress_ptr c) {
    (void)c;
}

static boolean src_fill(j_decompress_ptr c) {
    static const JOCTET eoi[2] = {0xff, JPEG_EOI};
    c->src->next_input_byte = eoi;
    c->src->bytes_in_buffer = 2;
    return TRUE;
}

static void src_skip(j_decompress_ptr c, long n) {
    if (n <= 0) return;
    while (n > (long)c->src->bytes_in_buffer) {
        n -= (long)c->src->bytes_in_buffer;
        src_fill(c);
    }
    c->src->next_input_byte += n;
    c->src->bytes_in_buffer -= (size_t)n;
}

static void src_term(j_decompress_ptr c) {
    (void)c;
}

int cpu_decode(const uint8_t *jpeg, size_t n, unsigned shift, struct cpu_planes *out) {
    memset(out, 0, sizeof *out);
    if (shift > 3 || !n) return CPU_FAILED;
    int rc = load();
    if (rc) return rc;

    struct jpeg_decompress_struct c;
    struct errmgr err;
    struct jpeg_source_mgr src;
    uint8_t *volatile buf = NULL;
    volatile int started = 0;
    memset(&c, 0, sizeof c);
    c.err = g_api.std_error(&err.pub);
    err.pub.error_exit = on_error;
    err.pub.output_message = on_message;
    if (setjmp(err.jb)) {
        if (started) g_api.destroy(&c);
        free(buf);
        return CPU_FAILED;
    }
    g_api.create(&c, JPEG_LIB_VERSION, sizeof c);
    started = 1;
    src.next_input_byte = jpeg;
    src.bytes_in_buffer = n;
    src.init_source = src_init;
    src.fill_input_buffer = src_fill;
    src.skip_input_data = src_skip;
    src.term_source = src_term;
    c.src = &src;
    /* jpeg_resync_to_restart is the library's own, found by name so that nothing is linked from it. */
    *(void **)&src.resync_to_restart = dlsym(g_api.lib, "jpeg_resync_to_restart");
    if (!src.resync_to_restart) {
        g_api.destroy(&c);
        return CPU_NO_LIBRARY;
    }

    g_api.read_header(&c, TRUE);
    if (c.data_precision != 8 || !((c.num_components == 1 && c.jpeg_color_space == JCS_GRAYSCALE) ||
                                   (c.num_components == 3 && c.jpeg_color_space == JCS_YCbCr))) {
        g_api.destroy(&c);
        return CPU_UNSUPPORTED;
    }
    int gray = c.num_components == 1;
    c.raw_data_out = TRUE;
    c.out_color_space = gray ? JCS_GRAYSCALE : JCS_YCbCr;
    c.scale_num = 1;
    c.scale_denom = 1u << shift;
    if (!g_api.start(&c)) {
        g_api.destroy(&c);
        return CPU_FAILED;
    }

    /* Each plane: the components' own sizes after the IDCT scaling (libjpeg gives a subsampled component a larger IDCT when it
     * can, so a scaled 4:2:0 picture comes out with chroma at the picture's own size). */
    int hs[3] = {0, 0, 0}, vs[3] = {0, 0, 0};
    size_t stride[3] = {0, 0, 0}, rows[3] = {0, 0, 0}, off[3] = {0, 0, 0}, total = 0;
    int nc = c.num_components;
    for (int i = 0; i < nc; i++) {
        jpeg_component_info *p = &c.comp_info[i];
        int hn = p->h_samp_factor * p->DCT_scaled_size, hd = c.max_h_samp_factor * c.min_DCT_scaled_size;
        int vn = p->v_samp_factor * p->DCT_scaled_size, vd = c.max_v_samp_factor * c.min_DCT_scaled_size;
        hs[i] = hn == hd ? 0 : 2 * hn == hd ? 1 : -1;
        vs[i] = vn == vd ? 0 : 2 * vn == vd ? 1 : -1;
        if (hs[i] < 0 || vs[i] < 0 || (i == 0 && (hs[0] || vs[0])) || (i == 2 && (hs[2] != hs[1] || vs[2] != vs[1]))) {
            g_api.destroy(&c);
            return CPU_UNSUPPORTED;
        }
        stride[i] = (size_t)p->width_in_blocks * (size_t)p->DCT_scaled_size;
        rows[i] = (size_t)c.total_iMCU_rows * (size_t)p->v_samp_factor * (size_t)p->DCT_scaled_size;
        off[i] = total;
        total += stride[i] * rows[i];
        if (total > (size_t)0x7fffffff || total < off[i]) {
            g_api.destroy(&c);
            return CPU_NOMEM;
        }
    }
    buf = malloc(total ? total : 1);
    if (!buf) {
        g_api.destroy(&c);
        return CPU_NOMEM;
    }
    memset(buf, 128, total);

    JSAMPROW lines[3][32];
    JSAMPARRAY image[3] = {lines[0], lines[1], lines[2]};
    size_t done[3] = {0, 0, 0};
    const JDIMENSION per_call = (JDIMENSION)(c.max_v_samp_factor * c.min_DCT_scaled_size);
    while (c.output_scanline < c.output_height) {
        for (int i = 0; i < nc; i++) {
            int per = c.comp_info[i].v_samp_factor * c.comp_info[i].DCT_scaled_size;
            for (int r = 0; r < per; r++) lines[i][r] = buf + off[i] + (done[i] + (size_t)r) * stride[i];
            done[i] += (size_t)per;
        }
        if (g_api.read_raw(&c, image, per_call) == 0) break;
    }

    out->buf = buf;
    out->im.y = buf + off[0];
    out->im.ystride = stride[0];
    out->im.width = (int)c.output_width;
    out->im.height = (int)c.output_height;
    if (!gray) {
        out->im.cb = buf + off[1];
        out->im.cr = buf + off[2];
        out->im.cstride = stride[1];
        out->im.cstep = 1;
        out->im.hshift = hs[1];
        out->im.vshift = vs[1];
    }
    g_api.destroy(&c);
    return CPU_OK;
}
