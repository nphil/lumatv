/* LG libvtcapture.so.1 histogram API, as recovered from the set's own binaries.
 *
 * Sources (webOS 10.2.1, libvtcapture.so.1.0.0, 32-bit ARM Thumb-2, built from LG's
 * lib32-screen-capture-webos 1.0.0-56): disassembly of vtCapture_{init,setPreset,get,finalize}Histogram, of the
 * VtHistogram C++ class behind them, and of LG's own caller (com.lighting.huesync.service/hue_sync_service); the
 * data layout and preset geometry are written down in LG's published kernel header v4l2-ext-vt.h
 * (kdriver/include/webosext/v4l2-ext-renderer-header/linux/). The two agree byte for byte.
 */
#ifndef LUMATV_VTHIST_H
#define LUMATV_VTHIST_H

#include <stdbool.h>
#include <stdint.h>

#define VTH_BINS 256       /* one bin per 8-bit code value, per channel */
#define VTH_MAX_ZONES 16   /* presets 0-2 have 16 areas, preset 3 (games) has 14 */
#define VTH_PRESETS 4

/* struct v4l2_ext_histogram_data, 0x604 bytes. The kernel fills it through V4L2_CID_EXT_HISTOGRAM_DATA
 * (V4L2_CID_USER_EXT_CAPTURE_BASE + 22 = 0x982a16) on /dev/video60 (vt-capture-dev). */
struct vth_zone {
    uint32_t area_id;          /* 0..N-1; the library fills it, the caller need not */
    uint16_t r[VTH_BINS];
    uint16_t g[VTH_BINS];
    uint16_t b[VTH_BINS];
};
_Static_assert(sizeof(struct vth_zone) == 0x604, "v4l2_ext_histogram_data is 0x604 bytes");

/* vtCapture_initHistogram(cb): cb is a plain function pointer wrapped into a std::function<bool()>; the library
 * calls it from its own thread when LG's resource manager takes the capture back (policy action). Hue Sync's
 * callback logs and returns false. Returns an opaque handle (a VtHistogram*), or NULL when:
 *   - the calling process is not allowed: /proc/<pid>/cmdline must contain
 *     "/com.lighting.huesync.service/hue_sync_service", or the file /var/luna/preferences/vt_devmode must exist
 *     ("dev mode on" is logged as a warning and the check passes);
 *   - the resource manager refuses (acquire fails; LG's other VT users hold the scaler): code 11 internally;
 *   - /dev/video60 cannot be opened or the START control fails.
 * The reason is only in the PmLog context "capture" (/var/log/messages), the C API returns just NULL. */
typedef bool (*vth_policy_cb)(void);
typedef void *(*vth_init_fn)(vth_policy_cb cb);

/* vtCapture_setPresetHistogram(h, preset): 0..3 (>3 returns 3). Must be called once before the first
 * getHistogram: the zone count of the preset (16, 16, 16, 14) is stored by this call and is 0 until then.
 * Returns 0, 2 (NULL handle, or histogram mode not started), 3 (bad preset). The control is
 * V4L2_CID_EXT_HISTOGRAM_PRESET (0x982a15). */
typedef int (*vth_preset_fn)(void *h, int preset);

/* vtCapture_getHistogram(h, out): out must hold N struct vth_zone (N from the preset; 16 is always enough).
 * The library zeroes a private buffer, numbers the areas, issues VIDIOC_G_EXT_CTRLS with size N * 0x604 and
 * memcpy()s the result into out. Return codes, from the errno of the ioctl:
 *   0 ok; 2 not started; 3 out == NULL; 7 EOPNOTSUPP; 11 EAGAIN; 13 ENODATA; 17 EACCES; 1 any other error. */
typedef int (*vth_get_fn)(void *h, struct vth_zone *out);

/* vtCapture_finalizeHistogram(h): stops the histogram (V4L2_CID_EXT_HISTOGRAM_STOP, 0x982a17), closes the fd,
 * releases the resource-manager client and deletes the handle. Returns 0 on success (2 is mapped to 0). */
typedef int (*vth_fin_fn)(void *h);

#endif
