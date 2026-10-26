/* The wire between the helper (helper/app/src/images.rs) and lumatv-jpeg, over the socket the helper hands the child as fd 0.
 * Little-endian 32-bit words, no padding. One request in flight at a time; the child answers in order.
 *
 *   request  (32 bytes)  magic "JPQ1" | id | op fmt shift flags (one byte each, op lowest) | max_pixels | jpeg_len | 3 x 0
 *                        followed by jpeg_len bytes of JPEG file
 *   response (64 bytes)  magic "JPR1" | id | status | width | height | stride | fmt | data_len | decode_us | convert_us |
 *                        total_us | hw_pixfmt | hw_width | hw_height | hw_bytes | flags
 *                        followed by data_len bytes of pixels
 *
 * op     1 decode, 2 ping (answered with status 0 and no data: the library is not touched)
 * fmt    1 NV12 (Y plane, then interleaved CbCr at half height; stride is the Y row, data_len = stride * (h + (h + 1) / 2)),
 *        2 RGBA (R, G, B, A = 255), 3 BGRA (B, G, R, A = 255); stride = 4 * width
 * shift  0..3: decode at 1, 1/2, 1/4, 1/8 per axis (the hardware's scaled decode; the output is ceil(dim / 2^shift))
 * flags  request: bit 0 = full-range conversion off (BT.601 studio range expanded), bit 1 = swap Cb and Cr, bits 2 and 3 below
 * hw_*   what the hardware returned, before the crop and the conversion (diagnostics; 0 when the decode did not get that far)
 *
 * The hardware block is tried first; where it is missing, refuses the file or fails on it, libjpeg.so.62 gives the same planes
 * (JP_RFLAG_CPU in the response, the hardware's status in its bits 8..15), unless the request says otherwise: flags bit 2 =
 * hardware only (no fallback, the status is the hardware's), bit 3 = CPU only (the block is not touched).
 * flags  response: bit 0 = decoded on the CPU, bits 8..15 = why the block was not used (a JP_E_* status; 0 when it was, or was not tried)
 */
#ifndef LUMATV_JPEGPROTO_H
#define LUMATV_JPEGPROTO_H

#include <stdint.h>

#define JP_REQ_MAGIC 0x3151504au /* "JPQ1" */
#define JP_RSP_MAGIC 0x3152504au /* "JPR1" */
#define JP_REQ_WORDS 8
#define JP_RSP_WORDS 16
#define JP_REQ_BYTES (JP_REQ_WORDS * 4)
#define JP_RSP_BYTES (JP_RSP_WORDS * 4)

enum { JP_OP_DECODE = 1, JP_OP_PING = 2 };
enum { JP_FMT_NV12 = 1, JP_FMT_RGBA = 2, JP_FMT_BGRA = 3 };
enum { JP_FLAG_LIMITED = 1, JP_FLAG_SWAP_UV = 2, JP_FLAG_HW_ONLY = 4, JP_FLAG_CPU = 8 };
enum { JP_RFLAG_CPU = 1 };

enum {
    JP_OK = 0,
    JP_E_REQUEST = 1,     /* malformed request: the child closes the connection after this */
    JP_E_UNSUPPORTED = 2, /* a JPEG neither decoder takes (kind, precision, sampling), or the hardware's alone with flags bit 2 */
    JP_E_NO_LIBRARY = 3,  /* libdile_photo.so.1 or one of its three functions is missing (and libjpeg.so.62, unless flags bit 2) */
    JP_E_OPEN = 4,        /* DILE_JPEG_Open failed */
    JP_E_DECODE = 5,      /* DILE_JPEG_Decode failed */
    JP_E_OUTPUT = 6,      /* the library returned an image this build cannot read (dimensions, format, size) */
    JP_E_TOO_BIG = 7,     /* the converted image would exceed max_pixels */
    JP_E_NOMEM = 8,
};

#endif
