/* A Wayland client for LG's compositor (and LumaTV's, which speaks the same tables), written against the wire so the player needs no
 * libwayland: it makes one window, exports one video element on it, and keeps a transparent ARGB buffer on it that the player may
 * draw into.
 *
 * What LG's side expects (docs/research/videoplane-saver-result.md has the sources): `wl_webos_shell.get_shell_surface` for the window,
 * `set_property("appId", ...)` because LG's compositor only composites windows it can attribute to an app (the id need not be an
 * installed app), the window type as `set_property("_WEBOS_WINDOW_TYPE", ...)`, `set_state(3)` for fullscreen (the card app's request on
 * this set), `wl_webos_foreign.export_element(surface, 0)` answered by `window_id_assigned("_Window_Id_N")`, which the media pipeline
 * then imports; where the video is on the screen is `wl_webos_exported.set_exported_window(source region, destination region)`.
 * The opcodes are from compositor/protocol/lg-tables.txt (read off the set's own libwayland-webos-client).
 */
#ifndef VPL_WL_H
#define VPL_WL_H

#include <stddef.h>
#include <stdint.h>

typedef struct vpl_wl vpl_wl;

typedef struct {
    const char *app_id;          /* the window's app id */
    const char *title;
    const char *window_type;     /* "_WEBOS_WINDOW_TYPE_SCREENSAVER", "..._CARD", "..._OVERLAY", or NULL for the compositor's default */
    int play_video;              /* set the window property playVideo (LG's flag for a screen saver that plays video) */
    int width, height;           /* the surface's size, the output's: 1920 x 1080 */
    int state;                   /* wl_webos_shell_surface state: 3 fullscreen */
    int export_first;            /* export the element before the first commit (1) or after it (0) */
    int timeout_ms;              /* each wait of the handshake */
} vpl_wl_opts;

/* Connects ($WAYLAND_SOCKET, or $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY), makes the window, commits a transparent buffer and waits for the
 * element's window id. 0 and a handle, or -1 and the reason in `err`. */
int vpl_wl_open(vpl_wl **out, const vpl_wl_opts *o, char *err, size_t cap);

/* Destroys the element and the window and closes the connection: the window is gone from the screen when this returns. */
void vpl_wl_close(vpl_wl *w);

int vpl_wl_fd(const vpl_wl *w);

/* Reads and handles what the compositor has sent. 0 = fine, 1 = the window must go (the compositor closed it, or the connection ended),
 * -1 = a protocol error (reason in `err`). */
int vpl_wl_dispatch(vpl_wl *w, char *err, size_t cap);

/* The element's id, "_Window_Id_N", what NDL is given before it initialises. */
const char *vpl_wl_window_id(const vpl_wl *w);

/* The window's buffer: width x height ARGB8888 premultiplied, transparent until drawn on. */
uint32_t *vpl_wl_pixels(vpl_wl *w);

/* Tells the compositor the box changed: damage, attach, commit. */
int vpl_wl_commit(vpl_wl *w, int x, int y, int bw, int bh);

/* Key presses, pointer buttons and pointer motion on any of the compositor's seats since the last call: the viewer wants the screen back. */
unsigned vpl_wl_activity(vpl_wl *w);

/* Where the video is: the source rectangle of the picture and the rectangle of the screen it fills. Commits. */
int vpl_wl_video(vpl_wl *w, int sx, int sy, int sw, int sh, int dx, int dy, int dw, int dh);

/* Sends what is queued. */
int vpl_wl_flush(vpl_wl *w);

/* How many requests and events have gone each way (for the status line and the tests). */
void vpl_wl_counts(const vpl_wl *w, unsigned *sent, unsigned *received);

#endif
