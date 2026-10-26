/* LG libdile_photo.so.1 JPEG API (DILE_JPEG_*), as recovered from the set's own binaries.
 *
 * Sources (webOS 10.2.1 build 5202, 32-bit ARM Thumb-2): disassembly of libdile_photo.so.1.0.1 (md5 9127769b47e7009e5cc870b16c301f5b:
 * DILE_JPEG_Open 0xe84, DILE_JPEG_Close 0xea8, DILE_JPEG_Encode 0x10bc, DILE_JPEG_Decode 0x1390, and the conversion helpers
 * _get_scaling_shift_integer 0x1038, _get_hal_jpeg_scaling_option 0x1064, _get_hal_jpeg_pixel_format 0x1090,
 * _get_pixel_format_from_hal_jpeg 0x1364) and of libhal_photo.so.2.0.1 (md5 68bbd1f2b88ac95f73999ec97710bf7f: HAL_JPEG_Open
 * 0x1568, HAL_JPEG_Decode 0x1830, HAL_JPEG_Encode 0x1c4c), the layer below it that calls libkadaptor's KADP_MJPEG_* and the
 * kernel's /dev/lg/mjpeg0 (ioctls in LG's published kdriver/include/mjpeg_kapi.h). No LG header for DILE_JPEG exists in the
 * published GPL tree; the names of the C types below are ours, the layout and the values are the library's.
 * docs/research/hwjpeg-result.md has the evidence line by line.
 *
 *   int DILE_JPEG_Open(void);                       0 ok, 1 failed
 *   int DILE_JPEG_Close(void);                      0 ok, 1 failed
 *   int DILE_JPEG_Decode(const struct dile_jpeg_buffer *src, struct dile_jpeg_buffer *dst, int scaling);
 *   int DILE_JPEG_Encode(const struct dile_jpeg_buffer *src, struct dile_jpeg_buffer *dst);
 *
 * Decode and encode return 0 ok, 1 failed (the allocation or the HAL call), 2 a NULL argument or a NULL src data pointer.
 */
#ifndef LUMATV_DILEJPEG_H
#define LUMATV_DILEJPEG_H

#include <stdint.h>

/* One buffer of the call. Only the fields below are read or written by the library; the others are zero. On the set (ARM32) the
 * struct is 0x28 bytes and the offsets are the comments'. A 64-bit host build (the fake library, the host tests) is consistent
 * with itself only. */
struct dile_jpeg_buffer {
    void *data;            /* 0x00 decode: src = the JPEG file; dst = malloc'd by the library, free() it. encode: the reverse. */
    uint32_t r04;
    uint32_t r08;          /* 0x08 encode only: read, "!= 0" selects a second-plane path (HAL: chroma offset) */
    uint32_t size;         /* 0x0c bytes: decode src = the JPEG file's length; decode dst = set by the library to
                              (width >> shift) * (height >> shift) * 3, the size of the buffer it allocated */
    uint32_t r10, r14, r18;
    uint32_t width;        /* 0x1c decode src: only sizes dst's allocation ((width >> shift) * (height >> shift) * 3 bytes), so the
                              caller passes dimensions rounded up. decode dst: set by the call, the decoded width */
    uint32_t height;       /* 0x20 as width */
    uint32_t pixel_format; /* 0x24 decode dst: set by the call, one of the DILE_JPEG_PF_* below. encode src: the format given */
};
#if UINTPTR_MAX == 0xffffffffu
_Static_assert(sizeof(struct dile_jpeg_buffer) == 0x28, "the library's buffer is 0x28 bytes");
#endif

/* The `scaling` argument of DILE_JPEG_Decode. The library subtracts one for the shift it applies to the dims it allocates dst
 * for, and hands the same value on: HAL_JPEG_Decode maps 2, 3, 4 to the hardware's horizontal and vertical scale codes 1, 2, 3
 * (LX_MJPEG_SCALE_1_2, _1_4, _1_8 of mjpeg_kapi.h). A value outside 1..4 is logged and decoded as 1/1. */
enum {
    DILE_JPEG_SCALE_1_1 = 1,
    DILE_JPEG_SCALE_1_2 = 2,
    DILE_JPEG_SCALE_1_4 = 3,
    DILE_JPEG_SCALE_1_8 = 4,
};

/* Pixel formats the decode puts in dst.pixel_format. The library accepts 1..0x12 (0x12 of them) in both directions; the decode
 * produces the nine below, chosen from the hardware's chroma format (LX_MJPEG_FORMAT_*: 0 4:2:0, 1 4:2:2, 2 4:2:4 ("224": chroma
 * halved vertically only), 3 4:4:4, 4 4:0:0) and a flag the kernel adapter returns with the image (bUVInterleaved of
 * LX_MJPEG_IMAGE_BUF_T): HAL_JPEG_Decode 0x1ab4-0x1b9e. "Planar" and "semi-planar" are read from that flag; the names of the
 * codes are ours. The encode accepts 8, 13 and 14 as source formats (DILE_JPEG_Encode -> HAL_JPEG_Encode 0x1d9c-0x1db4). */
enum {
    DILE_JPEG_PF_YUV444_PLANAR = 7,
    DILE_JPEG_PF_YUV444_SEMI = 8,
    DILE_JPEG_PF_YUV422H_PLANAR = 9,
    DILE_JPEG_PF_YUV422H_SEMI = 10,
    DILE_JPEG_PF_YUV420_PLANAR = 12,
    DILE_JPEG_PF_YUV420_SEMI = 13, /* NV12 */
    DILE_JPEG_PF_GRAY = 14,
    DILE_JPEG_PF_YUV224_PLANAR = 15,
    DILE_JPEG_PF_YUV224_SEMI = 16,
};

typedef int (*dile_jpeg_open_fn)(void);
typedef int (*dile_jpeg_close_fn)(void);
typedef int (*dile_jpeg_decode_fn)(const struct dile_jpeg_buffer *src, struct dile_jpeg_buffer *dst, int scaling);

#endif
