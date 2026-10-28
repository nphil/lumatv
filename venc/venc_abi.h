/* LG video-encoder ioctl ABI for /dev/venc on the C4 (LG1213 "O22", webOS 10.2.1).
 *
 * Sources: LG's published O22 tree, kdriver/core/vdec/venc/include/venc_kapi.h (the ioctl numbers and the
 * structs), kdriver/core/vdec/ipc/local/venc.c (the /dev/venc character device: the KADP magic gate, the
 * IPC forwarding to the video MCU, the mmap layout), and the DWARF of the set's own user of this ABI,
 * /usr/lib/gstreamer-1.0/libgstlxvideoenc.so (type sizes 44/52/48/12, the call order). Field names are
 * kept so the two can be read side by side. Every struct here is the same size and layout for a 32-bit
 * and a 64-bit caller: the only pointer is inside a union that carries an explicit 64-bit member.
 *
 * The encoder is a Hantro H1 (hx280enc) driven by the video MCU's firmware; Linux talks to it over IPC.
 * /dev/venc (char major 1820, root:composit 0660) is the IPC front door LG's libkadaptor uses;
 * /dev/video24 "sic-encoder" is a V4L2 M2M wrapper over the same IPC; /dev/lg/venc0 (major 162) is the
 * old in-kernel driver, which has no hardware table for this chip on the ARM side.
 */
#ifndef LUMATV_VENC_ABI_H
#define LUMATV_VENC_ABI_H

#include <stdint.h>
#include <sys/ioctl.h>

#define VENC_DEV "/dev/venc"
#define VENC_IOC_MAGIC 'n'

typedef enum {
    LX_VENC_ENCODE_TYPE_H264 = 0,
    LX_VENC_ENCODE_TYPE_VP8 = 1,
} LX_VENC_ENCODE_TYPE_T;

/* Input picture layout for the memory-to-memory ("ENC") mode. */
typedef enum {
    LX_VENC_IMAGE_YUV420_PLANAR = 0,       /* Y, then U, then V */
    LX_VENC_IMAGE_YUV420_SEMIPLANAR = 1,   /* Y, then interleaved UV: NV12 (what the video tap delivers) */
    LX_VENC_IMAGE_YUV420_SEMIPLANAR_VU = 2, /* NV21 */
    LX_VENC_IMAGE_YUV422_INTERLEAVED_YUYV = 3,
    LX_VENC_IMAGE_YUV422_INTERLEAVED_UYVY = 4,
} LX_VENC_IMAGE_TYPE_T;

typedef enum {
    LX_VENC_RC_TYPE_VBR = 0,
    LX_VENC_RC_TYPE_CBR = 1,
    LX_VENC_RC_TYPE_FIXED_QP = 2,
} LX_VENC_RC_TYPE_T;

typedef enum {
    LX_VENC_INTRA_FRAME = 0,     /* I (IDR) frame */
    LX_VENC_PREDICTED_FRAME = 1, /* P frame; the H1 has no B frames */
} LX_VENC_CODING_TYPE;

/* H.264 profile codes of LX_VENC_ENC_CREATE_T.profile (venc/ipc/local/v4l2_venc.h, enum venc_h264_profile). */
#define LX_VENC_H264_PROFILE_HIGH 0
#define LX_VENC_H264_PROFILE_MAIN 1
#define LX_VENC_H264_PROFILE_BASELINE 2

/* The first ioctl on a fresh /dev/venc fd must be this one, with the three strings below; every other
 * ioctl answers EFAULT until it has been accepted (ipc/local/venc.c, venc_ioctl: "invalid ioctl path"). */
typedef struct {
    uint8_t kadpmagic1[5]; /* "SIC" */
    uint8_t kadpmagic2[5]; /* "KADP" */
    uint8_t kadpmagic3[5]; /* "VENC" */
} LX_VENC_KADPCALL_PARAM_T;

/* Buffers the MCU reserved for this instance; physical addresses. The ES buffer (ESBuf) is the output ring
 * the encoder writes compressed frames into; the AUI buffer carries access-unit records in record mode.
 * mmap on /dev/venc after this ioctl: offset 0 maps the AUI buffer, offset ui32AUIBufSize maps the ES buffer
 * (ipc/local/venc.c, venc_mmap). */
typedef struct {
    uint32_t ui32PhysicalAddress; /* thumbnail */
    uint32_t ui32Size;
    uint32_t ui32ESBufPhysBase;
    uint32_t ui32ESBufSize;
    uint32_t ui32ESBufOffset;
    uint32_t ui32AUIBufPhysBase;
    uint32_t ui32AUIBufSize;
    uint32_t ui32AUIBufOffset;
    uint32_t ui32InBufPhysBase;
    uint32_t ui32InBufSize;
    uint32_t ui32InBufOffset;
} LX_VENC_BUFFER_T;

typedef struct {
    int32_t id;          /* [OUT] instance id, passed to every later call */
    uint32_t width;      /* 240..1920, multiple of 16 is safest (H1 macroblocks) */
    uint32_t height;     /* 120..1080 */
    uint32_t framerate;  /* frames per second, integer */
    uint32_t gopLength;  /* frames between I frames; 0 = one I frame only */
    uint32_t bitrate;    /* kbit/s; 0 = fixed QP */
    uint32_t fixedQp;    /* 0 = rate control */
    LX_VENC_IMAGE_TYPE_T imageType;
    LX_VENC_ENCODE_TYPE_T codecType;
    uint32_t interlaced;    /* BOOLEAN */
    uint32_t topFieldFirst; /* BOOLEAN */
    uint32_t profile;       /* LX_VENC_H264_PROFILE_* */
    uint32_t level;         /* 0: the MCU's default (5.1 in ENC mode) */
} LX_VENC_ENC_CREATE_T;

typedef struct {
    int32_t id;
} LX_VENC_ENC_DESTROY_T;

/* One frame in, one access unit out. The MCU writes the Annex B bytes to bus_output and reports stream_size.
 * An I frame starts with the 64-byte SPS/PPS header block the MCU made at create time, zero padded
 * (hxenc/h1encode.c, ENC_H264Encode). duration is the hardware clock time of the encode in ms ("for debug"). */
typedef struct {
    int32_t id;
    uint32_t bus_luma;     /* physical address of the Y plane */
    uint32_t bus_chroma_u; /* physical address of the UV plane (NV12) or U plane */
    uint32_t bus_chroma_v; /* V plane for planar input; unused for NV12 */
    union {
        void *p_out_buf; /* a user mapping of bus_output; the MCU does not read it */
        uint32_t compat_p_out_buf;
        uint64_t sizer; /* v4l2_venc.c puts the total input size here; harmless */
    };
    uint32_t bus_output;   /* physical address the stream goes to */
    uint32_t out_buf_size; /* bytes available there */
    uint32_t timeIncrement;
    LX_VENC_CODING_TYPE codingType; /* [OUT] I or P */
    uint32_t stream_size;           /* [OUT] bytes written */
    uint32_t duration;              /* [OUT] encode time, ms */
} LX_VENC_ENC_ENCODE_T;

typedef struct {
    int32_t id;
    LX_VENC_RC_TYPE_T rcType;
    uint32_t value; /* kbit/s for VBR/CBR, QP for FIXED_QP */
} LX_VENC_ENC_RC_T;

/* Live parameter changes, applied before the next frame. */
#define ENCODE_PARAM_FRAMERATE (1u << 0)
#define ENCODE_PARAM_GOP_LENGTH (1u << 1)
#define ENCODE_PARAM_BITRATE (1u << 2)
#define ENCODE_PARAM_SET_IDR (1u << 3)

typedef struct {
    uint32_t adjustParam; /* ENCODE_PARAM_* bits */
    uint32_t framerate;
    uint32_t gopLength;
    uint32_t bitrate; /* kbit/s */
    uint32_t setIDRFrame;
} LX_VENC_ENC_PARAM_T;

#define LX_VENC_IOR_GET_BUFFER_INFO _IOR(VENC_IOC_MAGIC, 1, LX_VENC_BUFFER_T)
#define LX_VENC_IOWR_ENC_CREATE _IOWR(VENC_IOC_MAGIC, 21, LX_VENC_ENC_CREATE_T)
#define LX_VENC_IOW_ENC_DESTROY _IOW(VENC_IOC_MAGIC, 22, LX_VENC_ENC_DESTROY_T)
#define LX_VENC_IOWR_ENC_ENCODE _IOWR(VENC_IOC_MAGIC, 23, LX_VENC_ENC_ENCODE_T)
#define LX_VENC_IOW_ENC_SET_RC _IOW(VENC_IOC_MAGIC, 25, LX_VENC_ENC_RC_T)
#define LX_VENC_IO_SET_IDR _IO(VENC_IOC_MAGIC, 26)
#define LX_VENC_IOWR_ENC_PARAM _IOWR(VENC_IOC_MAGIC, 28, LX_VENC_ENC_PARAM_T)
#define LX_VENC_IOW_VALID_KADPCALL _IOW(VENC_IOC_MAGIC, 30, LX_VENC_KADPCALL_PARAM_T)

_Static_assert(sizeof(LX_VENC_KADPCALL_PARAM_T) == 15, "kadp magic is 15 bytes");
_Static_assert(sizeof(LX_VENC_BUFFER_T) == 44, "LX_VENC_BUFFER_T is 44 bytes");
_Static_assert(sizeof(LX_VENC_ENC_CREATE_T) == 52, "LX_VENC_ENC_CREATE_T is 52 bytes");
_Static_assert(sizeof(LX_VENC_ENC_ENCODE_T) == 48, "LX_VENC_ENC_ENCODE_T is 48 bytes");
_Static_assert(sizeof(LX_VENC_ENC_RC_T) == 12, "LX_VENC_ENC_RC_T is 12 bytes");
_Static_assert(sizeof(LX_VENC_ENC_PARAM_T) == 20, "LX_VENC_ENC_PARAM_T is 20 bytes");

/* Limits (v4l2_venc.h and hxenc/h1encode.c): one chip, one H1 core, 240x120 to 1920x1080, H.264 or VP8,
 * bitrate control 1 kbit/s to 20 Mbit/s, GOP up to 300, I and P only. In the display-feedback record mode
 * the MCU encodes 1080p at 30 fps at most (FHD_FRAMERATE_MAX, every other vsync skipped); memory-to-memory
 * mode runs as fast as frames are pushed. */
#define VENC_MAX_WIDTH 1920u
#define VENC_MAX_HEIGHT 1080u
#define VENC_MIN_WIDTH 240u
#define VENC_MIN_HEIGHT 120u
#define VENC_MAX_KBPS 20000u
#define VENC_SPS_PPS_BLOCK 64u /* the zero-padded parameter-set block at the start of every I frame */

#endif
