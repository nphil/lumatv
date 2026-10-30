/* The video pipeline the feeder drives: one of two of LG's libraries, loaded at run time and spoken to through one table of calls.
 *
 *   ndl  libNDL_directmedia.so.1 (gf_ndl.c): the cloud-game session. Its load is a low-delay game-mode one, and such a pipeline takes the
 *        display's sub scaler for itself (`subScalerOccupied`), so it can only ever be the one video on the screen.
 *   smp  libplayerAPIs.so.1 (gf_smp.c): the pipeline an app's own video has (the C++ class StarfishMediaAPIs, called by its mangled
 *        names). No game mode, so the sub scaler stays free, and its AV connector takes the sink the Multi View state gives it, MAIN or
 *        SUB0: the pipeline that plays beside an app's video (docs/research/multiview-result.md).
 *
 * The events both deliver are PF_EVENT_T of StarfishMediaAPIs.h (NDL passes them through); the ones acted on or counted are named here.
 * Events arrive on the libraries' threads and are queued; the descriptor is readable when the queue is not empty: clear it, then drain.
 */
#ifndef GF_PIPE_H
#define GF_PIPE_H

#include <stddef.h>

#include "gf_proto.h"

enum { GF_EV_STR_VIDEO_INFO = 4, GF_EV_STR_RESOURCE_INFO = 17, GF_EV_INT_ERROR = 18, GF_EV_LOAD_COMPLETED = 22, GF_EV_UNLOAD_COMPLETED = 23,
       GF_EV_PLAYING = 26, GF_EV_INT_BUFFER_LOW = 44, GF_EV_STR_BUFFER_FULL = 45, GF_EV_STR_BUFFER_LOW = 46 };

/* Which optional calls a library has (HDR, drop threshold, flush, audio buffer size, area): a bit set, for the hello and the probe. */
enum { GF_PIPE_HAS_HDR = 1, GF_PIPE_HAS_DROP_THRESHOLD = 2, GF_PIPE_HAS_FLUSH = 4, GF_PIPE_HAS_AUDIO_AVAIL = 8, GF_PIPE_HAS_SET_AREA = 16 };

/* `start` says this when the library refused the load itself (the one failure a different audio type can cure); -1 is any other step. */
#define GF_PIPE_LOAD_FAILED (-3)

typedef struct gf_pipe gf_pipe;     /* a driver's own object */

/* What a load asks of the display beyond the stream (`--sink`, GF_SINK): `sink` is "" for the main window every pipeline gets, "sub_video" for the sub
 * window beside an app's video. Only the starfish pipeline can ask (its load option `avSink.videoSink.type`); NDL refuses such a load. */
typedef struct gf_pipe_opts {
    const char *sink;
} gf_pipe_opts;

typedef struct gf_pipe_ops {
    const char *name;                                                   /* "ndl", "smp" */
    /* dlopen `lib` (NULL or "": the driver's default name), resolve every call, initialise. */
    int (*open_lib)(gf_pipe **out, const char *lib, char *err, size_t cap);
    const char *(*lib_name)(const gf_pipe *p);
    unsigned (*features)(const gf_pipe *p);
    /* Window, app id, load, foreground, then the HDR info and the drop threshold when asked. `l` names the stream; its `pts_unit` is what the
     * PTS of every feed is divided by to reach the library's unit; `o` what the load asks of the display. 0 started, GF_PIPE_LOAD_FAILED, -1. */
    int (*start)(gf_pipe *p, const char *app_id, const char *window_id, const gf_load *l, const gf_pipe_opts *o, char *err, size_t cap);
    /* One access unit: 0 taken, 1 the pipeline's buffer is full (offer it again), -1 refused with an error. */
    int (*feed_video)(gf_pipe *p, const void *unit, unsigned size, long long pts_ns);
    int (*feed_audio)(gf_pipe *p, const void *data, unsigned size, long long pts_ns);
    int (*queued)(gf_pipe *p);                                          /* render buffer length, or -1 */
    int (*audio_avail)(gf_pipe *p);                                     /* audio buffer bytes available, or -1 */
    int (*flush)(gf_pipe *p);                                           /* -2: the library has no such call */
    int (*set_hdr)(gf_pipe *p, const gf_hdr *h);                        /* -2: no such call */
    int (*set_drop_threshold)(gf_pipe *p, int threshold);               /* -2: no such call */
    int (*set_state)(gf_pipe *p, int foreground);
    int (*area)(gf_pipe *p, int x, int y, int w, int h);                /* -2: no such call */
    void (*stop)(gf_pipe *p);                                           /* unload and quit; the library stays loaded */
    void (*free)(gf_pipe *p);
    int (*event_fd)(const gf_pipe *p);
    void (*event_clear)(gf_pipe *p);
    int (*next_event)(gf_pipe *p, int *type, long long *num, char *text, size_t cap);
    const char *(*error)(const gf_pipe *p);
} gf_pipe_ops;

extern const gf_pipe_ops gf_ndl_ops;
extern const gf_pipe_ops gf_smp_ops;

/* The driver named "ndl" or "smp" ("" and NULL: ndl); NULL for any other word. */
const gf_pipe_ops *gf_pipe_named(const char *name);

#endif
