/* LG video-tap capture ABI: /dev/video60 "vt-capture-dev" on the C4.
 *
 * Sources: LG's published header v4l2-ext-vt.h (kdriver/include/webosext/v4l2-ext-renderer-header/linux/,
 * webOS 25) for the structs and the +N control offsets, and the set's own libvtcapture.so.1.0.0 (webOS
 * 10.2.1) for the control base, which is V4L2_CID_USER_BASE + 0x2a00 on this firmware, not the +0x2100 the
 * newer header says: VtDriver::setCapturePlanePropertes issues 0x982a03, getCapturePlaneInfo 0x982a01,
 * getCaptureVideoWindowInfo 0x982a02, setCaptureFrameRate 0x982a08, getCaptureFrameRate 0x982a07, and the
 * histogram calls 0x982a14..17 (docs/research/ambient-result.md). The kernel side is
 * kdriver/core/de/v4l2_vt.c.
 *
 * What the device does: a V4L2 multi-planar capture device (vb2, MMAP/USERPTR/DMABUF, EXPBUF) that copies
 * one of five points of the display pipeline into NV12 buffers at a size of the caller's choice (the
 * scaler does the resize), at the display rate or a divided rate. Only one capture runs at a time
 * (the library-level histogram and a frame capture cannot coexist; a second opener is refused the
 * capture). When the video on screen is protected (secure video path) the driver forces the OSD-only
 * point at open, and DQBUF answers EACCES for a protected frame, so protected video never reaches a
 * capture buffer (v4l2_vt.c, v4l2_vt_fh_open and v4l2_vt_vidioc_dqbuf).
 */
#ifndef LUMATV_VT_ABI_H
#define LUMATV_VT_ABI_H

#include <stdint.h>

#define VT_DEV "/dev/video60"
#define VT_CID_BASE 0x00982a00u /* V4L2_CID_USER_BASE + 0x2a00, as the set's library issues it */

#define V4L2_CID_EXT_CAPTURE_CAPABILITY_INFO (VT_CID_BASE + 0)
#define V4L2_CID_EXT_CAPTURE_PLANE_INFO (VT_CID_BASE + 1)
#define V4L2_CID_EXT_CAPTURE_VIDEO_WIN_INFO (VT_CID_BASE + 2)
#define V4L2_CID_EXT_CAPTURE_PLANE_PROP (VT_CID_BASE + 3)
#define V4L2_CID_EXT_CAPTURE_FREEZE_MODE (VT_CID_BASE + 4)
#define V4L2_CID_EXT_CAPTURE_DONE_USER_PROCESSING (VT_CID_BASE + 5)
#define V4L2_CID_EXT_CAPTURE_PHYSICAL_MEMORY_INFO (VT_CID_BASE + 6)
#define V4L2_CID_EXT_CAPTURE_OUTPUT_FRAMERATE (VT_CID_BASE + 7)
#define V4L2_CID_EXT_CAPTURE_DIVIDE_FRAMERATE (VT_CID_BASE + 8)
#define V4L2_CID_EXT_HISTOGRAM_START (VT_CID_BASE + 20)
#define V4L2_CID_EXT_HISTOGRAM_STOP (VT_CID_BASE + 23)

struct v4l2_ext_video_rect {
    uint16_t x, y, w, h;
};

/* Where in the pipeline the copy is taken (v4l2-ext-vt.h, V4L2_CID_EXT_CAPTURE_PLANE_PROP):
 *   SCALER_INPUT   the video source before de-interlace (LG SoC only; "VTV")
 *   SCALER_OUTPUT  the video after the scaler, before picture processing
 *   DISPLAY_OUTPUT the video after picture processing, before the OSD is blended
 *   BLENDED_OUTPUT the screen: video with the OSD blended over it
 *   OSD_OUTPUT     the OSD (graphics) plane alone
 * In the kernel these set the display feedback point (FB_WITHOUT_OSD, FB_DISP, FB_WITH_OSD, FB_OSD_ONLY),
 * the same tap LG's own Miracast transmitter feeds the encoder from (v4l2_vt.c, vt_set_plane_property:
 * "use display feedback for miracast-tx"). */
enum v4l2_ext_capture_location {
    V4L2_EXT_CAPTURE_SCALER_INPUT = 0,
    V4L2_EXT_CAPTURE_SCALER_OUTPUT = 1,
    V4L2_EXT_CAPTURE_DISPLAY_OUTPUT = 2,
    V4L2_EXT_CAPTURE_BLENDED_OUTPUT = 3,
    V4L2_EXT_CAPTURE_OSD_OUTPUT = 4,
};

enum v4l2_ext_capture_buf_location {
    V4L2_EXT_CAPTURE_INPUT_BUF = 0,
    V4L2_EXT_CAPTURE_OUTPUT_BUF = 1,
};

/* S_EXT_CTRLS payload of PLANE_PROP; size 16. plane.w must be a multiple of 16. buf_count is the number
 * of capture buffers the driver keeps (LG's library uses 5; 0 means "no buffers, feed the encoder's
 * display input instead", the Miracast transmitter's setting). */
struct v4l2_ext_capture_plane_prop {
    uint32_t l; /* enum v4l2_ext_capture_location */
    struct v4l2_ext_video_rect plane;
    int32_t buf_count;
};

struct v4l2_ext_capture_capability_info {
    uint32_t flags;
    uint32_t scale_up_limit_w, scale_up_limit_h;
    uint32_t scale_down_limit_w, scale_down_limit_h;
    uint32_t num_video_frame_buffer;
    struct v4l2_ext_video_rect max_res;
    uint32_t num_plane;    /* 1 interleaved, 2 semi-planar, 3 planar */
    uint32_t pixel_format; /* 1 = YUV420 semi-planar (NV12) */
};
#define V4L2_EXT_CAPTURE_CAP_SCALE_UP 0x0004
#define V4L2_EXT_CAPTURE_CAP_SCALE_DOWN 0x0008
#define V4L2_EXT_CAPTURE_CAP_DIVIDE_FRAMERATE 0x0010

struct v4l2_ext_capture_plane_info {
    uint32_t stride;
    struct v4l2_ext_video_rect plane_region;
    struct v4l2_ext_video_rect active_region;
};

/* G_EXT_CTRLS payload of PHYSICAL_MEMORY_INFO. The driver writes the Y and C physical addresses THROUGH
 * the two pointers (copy_to_user to *y and *c), and it reads those pointers from the 32-bit compat
 * members, so a 64-bit caller must place the two uint32_t targets below 4 GiB (v4l2_vt.c, case
 * V4L2_CID_EXT_CAPTURE_PHYSICAL_MEMORY_INFO). A 32-bit caller is unaffected. */
struct v4l2_ext_capture_physical_memory_info {
    int32_t buf_index;
    union {
        uint32_t *y;
        uint32_t compat_y_data;
        uint64_t y_sizer;
    };
    union {
        uint32_t *c;
        uint32_t compat_c_data;
        uint64_t c_sizer;
    };
    uint32_t buf_location; /* enum v4l2_ext_capture_buf_location */
};

_Static_assert(sizeof(struct v4l2_ext_video_rect) == 8, "rect is 8 bytes");
_Static_assert(sizeof(struct v4l2_ext_capture_plane_prop) == 16, "plane_prop is 16 bytes");
_Static_assert(sizeof(struct v4l2_ext_capture_capability_info) == 40, "capability_info is 40 bytes");
_Static_assert(sizeof(struct v4l2_ext_capture_plane_info) == 20, "plane_info is 20 bytes");
_Static_assert(sizeof(struct v4l2_ext_capture_physical_memory_info) == 32, "physical_memory_info is 32 bytes");

#endif
