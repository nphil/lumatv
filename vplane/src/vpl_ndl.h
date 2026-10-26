/* LG's NDL DirectMedia (libNDL_directmedia.so.1, API version 2, webOS 5 and later) loaded at run time, and the order its calls need.
 *
 * Why NDL and not the starfish pipeline or raw V4L2 (docs/research/opennow-result.md, "Which media API to use"): it takes H.264 and
 * H.265 access units and puts the decoded pictures on the display engine's video window; the decoder, the secure memory and the plane
 * are LG's. It is verified on hardware by webosbrew/samples (OLED77C5, webOS 10.3.1) and jellyfin-webos-native (docs/ndl.md). What it
 * demands, all of it from those two:
 *   - NDL_DirectMedia_DL_Initialize() before anything, or every entry point traps;
 *   - the window id from wl_webos_foreign (NDL_DirectMediaSetWindowId) BEFORE NDL_DirectMediaInit, or the library tries to make an SDL
 *     window of its own;
 *   - NDL_DirectMediaSetAppState(FOREGROUND) or every unit is accepted and dropped without an error;
 *   - whole access units, one NDL_DirectVideoPlay each, with a PTS in microseconds;
 *   - the clock is the caller's: the ES player presents a unit when it arrives.
 * It also needs the executable to have an LS2 role (payload/vplane/role.json), or LSRegisterPubPriv aborts the process.
 * Nothing is linked: a TV without the library, or an old API, gives a clear reason and the saver stays on the GPU.
 */
#ifndef VPL_NDL_H
#define VPL_NDL_H

#include <stddef.h>
#include <stdint.h>

typedef struct vpl_ndl vpl_ndl;

/* The pipeline's events (PF_EVENT_T of StarfishMediaAPIs.h; the ones this player acts on). */
enum { NDL_EV_STR_VIDEO_INFO = 4, NDL_EV_INT_ERROR = 18, NDL_EV_LOAD_COMPLETED = 22, NDL_EV_UNLOAD_COMPLETED = 23, NDL_EV_PLAYING = 26,
       NDL_EV_BUFFER_LOW = 44, NDL_EV_BUFFER_FULL = 45, NDL_EV_STR_BUFFER_LOW = 46 };

/* dlopen `lib` (NULL: libNDL_directmedia.so.1, then the unversioned name), resolve the calls, call DL_Initialize. */
int vpl_ndl_open_lib(vpl_ndl **out, const char *lib, char *err, size_t cap);

/* SetWindowId, Init, Load, SetAppState(FOREGROUND), in that order. `codec` is VPC_H264 or VPC_HEVC. */
int vpl_ndl_start(vpl_ndl *n, const char *app_id, const char *window_id, int codec, int width, int height, char *err, size_t cap);

/* 0 taken, 1 the pipeline's buffer is full (try again shortly), -1 refused. */
int vpl_ndl_feed(vpl_ndl *n, const void *unit, unsigned size, int64_t pts_us);

/* Frames waiting in the render queue, or -1. */
int vpl_ndl_queued(vpl_ndl *n);

/* NDL_DirectVideoSetArea; returns the library's code, or -2 when this library has no such call. */
int vpl_ndl_area(vpl_ndl *n, int x, int y, int w, int h);

/* Unload and quit; the library stays loaded (LG's libraries are not written to be unloaded). */
void vpl_ndl_stop(vpl_ndl *n);
void vpl_ndl_free(vpl_ndl *n);

/* Events come from the library's own threads; they are queued and `vpl_ndl_event_fd` becomes readable. The reader clears the descriptor FIRST
 * and then drains the queue, so an event that arrives while it drains wakes it again. */
int vpl_ndl_event_fd(const vpl_ndl *n);
void vpl_ndl_event_clear(vpl_ndl *n);
int vpl_ndl_next_event(vpl_ndl *n, int *type, long long *num, char *text, size_t cap);

/* NDL_DirectMediaGetError(). */
const char *vpl_ndl_error(const vpl_ndl *n);

#endif
