/* A real HEVC decoder as the presenter's picture source, with an oracle on its buffers.
 *
 * The decoder is libavcodec from the feeder's pinned host FFmpeg (`sh gfn/feeder/third_party/build-ffmpeg.sh`), fed the tagged stream
 * fixtures/tagged-bframes.hevc one access unit at a time in decode order, so pictures come out reordered. A picture handed to the presenter is
 * the decoder's own AVFrame in its native format (yuv420p10le): the presenter never touches the pixels, and nothing here copies them.
 *
 * Every plane buffer the decoder hands out is wrapped in a buffer whose reference count the decoder, the DPB and the surface all share. The
 * wrapper's last unref is observed, so the oracle knows exactly when memory goes back to the decoder's pool, and it checks that
 *   - memory handed out again never overlaps a live surface (`recycled_live`),
 *   - no plane buffer of a live surface reaches count 0 (`freed_while_live`),
 *   - a surface's luma still equals a reference decode whenever the backend reads it (`integrity_failures`),
 *   - releasing a surface drops exactly one reference per plane (`release_ref_errors`),
 *   - after the decoder is closed and every surface retired no buffer is outstanding.
 * A surface is live from the moment it is made until its retirement, whatever its owner has done with it.
 */
#ifndef HEVC_SOURCE_H
#define HEVC_SOURCE_H

#include <stddef.h>
#include <stdint.h>

typedef struct hevc_source hevc_source;
typedef struct hevc_surface hevc_surface;

typedef struct {
    uint64_t pictures;              /* surfaces made */
    uint64_t retired;               /* surfaces whose life ended */
    uint64_t handouts;              /* plane-0 buffers the decoder asked for */
    uint64_t distinct;              /* different plane-0 addresses among them: handouts > distinct means the pool reuses memory */
    uint64_t allocated, freed;      /* wrapped plane buffers created and freed */
    uint64_t recycled_live;
    uint64_t freed_while_live;
    uint64_t integrity_failures;
    uint64_t release_ref_errors;
    uint64_t double_retire;
    uint64_t read_after_free;       /* the backend was shown a surface whose owner had already dropped it */
    unsigned live;                  /* surfaces made and not yet retired */
    int64_t outstanding;            /* allocated - freed at close, after the decoder is gone */
} hevc_oracle;

/* `pts[d]` is the timestamp of display index d; indexes past `npts` continue one tick apart. Returns NULL with a message in `err`. */
hevc_source *hevc_source_open(const char *stream_path, const char *order_path, const int64_t *pts, size_t npts, char *err, size_t errcap);

/* The next picture in display order: 0 and a surface; 1 when the stream is finished; -1 on an error (hevc_source_error). The picture's tag, timestamp and
 * luma are checked against the order file and a reference decode before it is returned. */
int hevc_source_next(hevc_source *s, hevc_surface **out);

const char *hevc_source_error(const hevc_source *s);
size_t hevc_source_pictures(const hevc_source *s);                  /* pictures in the stream */
size_t hevc_source_max_lookahead(const hevc_source *s);             /* most access units fed ahead of the pictures output: the reorder depth */
int hevc_source_au_display(const hevc_source *s, size_t au);        /* display index of the au-th access unit in decode order */
const hevc_oracle *hevc_source_oracle(const hevc_source *s);

uint64_t hevc_surface_id(const hevc_surface *f);                    /* the tag decoded from its pixels */
int64_t hevc_surface_pts(const hevc_surface *f);
size_t hevc_surface_display(const hevc_surface *f);
int hevc_surface_refs(const hevc_surface *f);                       /* reference count of its luma buffer; 0 once the owner dropped the frame */
int hevc_surface_planes_freed(const hevc_surface *f);               /* of its three plane buffers, how many have gone back to the pool */

/* The backend reads the surface: its luma is compared with the reference decode. 0 when intact. */
int hevc_surface_read(hevc_source *s, hevc_surface *f);

/* The end of a surface's life: its owner drops the frame (if it still holds it) and the cookie is spent. Called once, from the release path or for a
 * picture the presenter refused. */
void hevc_surface_retire(hevc_source *s, hevc_surface *f);

/* A faulty owner: drops the frame's references while the backend is still entitled to read it. Only for the oracle's negative control. */
void hevc_surface_drop_refs_early(hevc_source *s, hevc_surface *f);

/* Frees the decoder first, then reads the final counts into `final` (when given), then frees anything still held. Returns the buffers outstanding. */
int64_t hevc_source_close(hevc_source *s, hevc_oracle *final);

#endif
