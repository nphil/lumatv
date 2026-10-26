/* LG's NDL DirectMedia (libNDL_directmedia.so.1, API 2, webOS 5 and later), loaded at run time: the video and audio ES feed, the HDR
 * metadata, the cloud-game frame-drop threshold and the render-queue readings the probe records.
 *
 * The call order the library demands (jellyfin-webos-native docs/ndl.md, webosbrew/samples; the stand-in gfn/feeder/tests/fake_ndl.c
 * refuses anything else): DL_Initialize, SetWindowId (or the library makes an SDL window and fails), Init(app id of the installed app
 * whose role names the media client services), Load(video and audio info together), SetAppState(FOREGROUND) (or every unit is dropped
 * silently), then Play per access unit. SetHDRInfo and SetFrameDropThreshold are applied after Load. The ES player has no clock: a unit is
 * shown when it arrives, the PTS is for A/V alignment and the queue bookkeeping. The same shape as vplane/src/vpl_ndl.h, which this
 * extends with audio, HDR and the game-mode knobs; it is its own copy so the saver and the stream can change independently.
 */
#ifndef GF_NDL_H
#define GF_NDL_H

#include <stddef.h>
#include <stdint.h>

#include "gf_proto.h"

typedef struct gf_ndl gf_ndl;

/* PF_EVENT_T of StarfishMediaAPIs.h, the ones acted on or counted. */
enum { NDL_EV_STR_VIDEO_INFO = 4, NDL_EV_STR_RESOURCE_INFO = 17, NDL_EV_INT_ERROR = 18, NDL_EV_LOAD_COMPLETED = 22, NDL_EV_UNLOAD_COMPLETED = 23,
       NDL_EV_PLAYING = 26, NDL_EV_INT_BUFFER_LOW = 44, NDL_EV_STR_BUFFER_FULL = 45, NDL_EV_STR_BUFFER_LOW = 46 };

/* dlopen `lib` (NULL: libNDL_directmedia.so.1, then the unversioned name), resolve every call, run DL_Initialize. */
int gf_ndl_open_lib(gf_ndl **out, const char *lib, char *err, size_t cap);
const char *gf_ndl_lib_name(const gf_ndl *n);

/* Which optional calls this library has (HDR, drop threshold, flush, audio buffer size): a bit set, for the hello and the probe. */
unsigned gf_ndl_features(const gf_ndl *n);
enum { GF_NDL_HAS_HDR = 1, GF_NDL_HAS_DROP_THRESHOLD = 2, GF_NDL_HAS_FLUSH = 4, GF_NDL_HAS_AUDIO_AVAIL = 8, GF_NDL_HAS_SET_AREA = 16 };

/* SetWindowId, Init, Load, SetAppState(FOREGROUND), then the HDR info and the drop threshold when asked. `l` names the stream. 0 started,
 * GF_NDL_LOAD_FAILED when the library refused the load itself (the one failure a different audio type can cure), -1 any other step. */
#define GF_NDL_LOAD_FAILED (-3)
int gf_ndl_start(gf_ndl *n, const char *app_id, const char *window_id, const gf_load *l, char *err, size_t cap);

/* NDL_DirectVideoPlay: 0 taken, 1 the pipeline's buffer is full, -1 refused with an error. */
int gf_ndl_feed_video(gf_ndl *n, const void *unit, unsigned size, long long pts);
/* NDL_DirectAudioPlay: same answers. */
int gf_ndl_feed_audio(gf_ndl *n, const void *data, unsigned size, long long pts);

int gf_ndl_queued(gf_ndl *n);                 /* render buffer length, or -1 */
int gf_ndl_audio_avail(gf_ndl *n);            /* audio buffer bytes available, or -1 */
int gf_ndl_flush(gf_ndl *n);
int gf_ndl_set_hdr(gf_ndl *n, const gf_hdr *h);
int gf_ndl_set_drop_threshold(gf_ndl *n, int threshold);
int gf_ndl_set_state(gf_ndl *n, int foreground);
int gf_ndl_area(gf_ndl *n, int x, int y, int w, int h);

void gf_ndl_stop(gf_ndl *n);                  /* Unload and Quit; the library stays loaded */
void gf_ndl_free(gf_ndl *n);

/* Events arrive on the library's threads and are queued; the descriptor is readable when the queue is not empty. Clear it first, then drain. */
int gf_ndl_event_fd(const gf_ndl *n);
void gf_ndl_event_clear(gf_ndl *n);
int gf_ndl_next_event(gf_ndl *n, int *type, long long *num, char *text, size_t cap);

const char *gf_ndl_error(const gf_ndl *n);

#endif
