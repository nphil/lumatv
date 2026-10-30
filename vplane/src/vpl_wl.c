#define _GNU_SOURCE
#include "vpl_wl.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/un.h>
#include <unistd.h>

#include "vpl.h"

/* Interfaces and opcodes. wl_* are the core protocol's; wl_webos_* are LG's, from compositor/protocol/lg-tables.txt. */
enum { DISPLAY = 1 };
enum { DISPLAY_SYNC = 0, DISPLAY_GET_REGISTRY = 1 };
enum { REGISTRY_BIND = 0 };
enum { COMPOSITOR_CREATE_SURFACE = 0, COMPOSITOR_CREATE_REGION = 1 };
enum { SURFACE_DESTROY = 0, SURFACE_ATTACH = 1, SURFACE_DAMAGE = 2, SURFACE_COMMIT = 6 };
enum { REGION_DESTROY = 0, REGION_ADD = 1 };
enum { SHM_CREATE_POOL = 0 };
enum { POOL_CREATE_BUFFER = 0, POOL_DESTROY = 1 };
enum { BUFFER_DESTROY = 0 };
enum { SHELL_GET_SHELL_SURFACE = 1 };
enum { SHELL_SURFACE_SET_STATE = 1, SHELL_SURFACE_SET_PROPERTY = 2 };
enum { FOREIGN_EXPORT_ELEMENT = 1 };
enum { EXPORTED_DESTROY = 0, EXPORTED_SET_EXPORTED_WINDOW = 1 };
enum { WL_SHM_FORMAT_ARGB8888 = 0, WL_SHM_FORMAT_XRGB8888 = 1 };
enum { SEAT_GET_POINTER = 0, SEAT_GET_KEYBOARD = 1 };
#define MAX_SEATS 8

struct vpl_wl {
    int fd;
    uint32_t next;
    uint8_t out[8192];
    size_t out_len;
    uint8_t in[65536];
    size_t in_len;
    unsigned sent, received;

    uint32_t registry, compositor, shm, shell, foreign;
    uint32_t surface, shell_surface, exported, pool, sync_cb;
    struct { uint32_t name, version; } g_compositor, g_shm, g_shell, g_foreign;

    /* every wl_seat the compositor offers (LG's has one per input device), with the keyboard and pointer taken from it: a key or a pointer
     * press on any of them is the viewer asking for the screen back */
    struct { uint32_t id, keyboard, pointer; } seats[MAX_SEATS];
    int nseats;
    unsigned activity;

    char window_id[64];
    int have_id, sync_done, closed, failed;
    char err[240];

    /* Two buffers of one pool, side by side in one mapping. The compositor reads a committed buffer when it likes, until it sends that
     * buffer's release, so a buffer is painted only while it is not `busy` (committed and not yet released). `back` is the one the next
     * commit attaches. */
    int w, h;
    uint32_t format;               /* the shm format of the two buffers */
    int shm_fd;
    uint32_t buffer[2];
    uint32_t *px[2];
    int busy[2], back;
    size_t px_len;                 /* one buffer's bytes */
    void *map;
};

static void set_err(vpl_wl *w, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void set_err(vpl_wl *w, const char *fmt, ...) {
    if (w->failed) return;
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(w->err, sizeof w->err, fmt, ap);
    va_end(ap);
    w->failed = 1;
}

/* ---- the wire --------------------------------------------------------------------------------------------------------- */

static uint32_t rd32(const uint8_t *p) { uint32_t v; memcpy(&v, p, 4); return v; }
static void wr32(uint8_t *p, uint32_t v) { memcpy(p, &v, 4); }

static int flush_out(vpl_wl *w) {
    size_t off = 0;
    while (off < w->out_len) {
        ssize_t n = send(w->fd, w->out + off, w->out_len - off, MSG_NOSIGNAL);
        if (n > 0) {
            off += (size_t)n;
            continue;
        }
        if (n < 0 && errno == EINTR) continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            struct pollfd p = {w->fd, POLLOUT, 0};
            if (poll(&p, 1, 2000) <= 0) {
                set_err(w, "the compositor does not read (send stalled 2 s)");
                w->closed = 1;
                return -1;
            }
            continue;
        }
        set_err(w, "send: %s", strerror(errno));
        w->closed = 1;
        return -1;
    }
    w->out_len = 0;
    return 0;
}

static size_t msg_begin(vpl_wl *w, uint32_t obj, uint16_t op) {
    if (w->out_len + 1024 > sizeof w->out) flush_out(w);
    size_t at = w->out_len;
    wr32(w->out + at, obj);
    wr32(w->out + at + 4, op);
    w->out_len += 8;
    return at;
}

static void put_u32(vpl_wl *w, uint32_t v) {
    wr32(w->out + w->out_len, v);
    w->out_len += 4;
}

static void put_str(vpl_wl *w, const char *s) {
    size_t n = strlen(s) + 1;
    if (n > 600) n = 600;
    put_u32(w, (uint32_t)n);
    memcpy(w->out + w->out_len, s, n - 1);
    memset(w->out + w->out_len + n - 1, 0, ((n + 3) & ~(size_t)3) - (n - 1));
    w->out_len += (n + 3) & ~(size_t)3;
}

static void msg_end(vpl_wl *w, size_t at) {
    uint32_t size = (uint32_t)(w->out_len - at);
    wr32(w->out + at + 4, rd32(w->out + at + 4) | size << 16);
    w->sent++;
}

/* ---- events ----------------------------------------------------------------------------------------------------------- */

static int get_str(const uint8_t *p, size_t n, size_t *pos, char *out, size_t cap) {
    if (*pos + 4 > n) return -1;
    uint32_t len = rd32(p + *pos);
    *pos += 4;
    size_t padded = ((size_t)len + 3) & ~(size_t)3;
    if (len == 0 || *pos + padded > n) return -1;
    size_t k = len - 1 < cap - 1 ? len - 1 : cap - 1;
    memcpy(out, p + *pos, k);
    out[k] = 0;
    *pos += padded;
    return 0;
}

static void bind_global(vpl_wl *w, uint32_t name, const char *iface, uint32_t ver, uint32_t id);

static void bind_seat(vpl_wl *w, uint32_t name, uint32_t version) {
    if (w->nseats >= MAX_SEATS) return;
    uint32_t id = w->next++;
    bind_global(w, name, "wl_seat", version < 4 ? version : 4, id);
    w->seats[w->nseats].id = id;
    w->nseats++;
}

static void on_seat_event(vpl_wl *w, uint32_t id, uint32_t op, const uint8_t *a, size_t n) {
    for (int i = 0; i < w->nseats; i++) {
        if (w->seats[i].id == id && op == 0 && n >= 4) {           /* capabilities: 1 pointer, 2 keyboard */
            uint32_t caps = rd32(a);
            if ((caps & 2) && !w->seats[i].keyboard) {
                w->seats[i].keyboard = w->next++;
                size_t at = msg_begin(w, id, SEAT_GET_KEYBOARD);
                put_u32(w, w->seats[i].keyboard);
                msg_end(w, at);
            }
            if ((caps & 1) && !w->seats[i].pointer) {
                w->seats[i].pointer = w->next++;
                size_t at = msg_begin(w, id, SEAT_GET_POINTER);
                put_u32(w, w->seats[i].pointer);
                msg_end(w, at);
            }
            return;
        }
        if (w->seats[i].keyboard == id && op == 3 && n >= 16 && rd32(a + 12) == 1) {   /* key(serial, time, key, state): pressed */
            w->activity++;
            return;
        }
        if (w->seats[i].pointer == id) {
            if ((op == 2 && n >= 12) || (op == 3 && n >= 16 && rd32(a + 12) == 1)) w->activity++;   /* motion, button pressed */
            return;
        }
    }
}

static void on_event(vpl_wl *w, uint32_t id, uint32_t op, const uint8_t *a, size_t n) {
    w->received++;
    if (id == DISPLAY) {
        if (op == 0 && n >= 12) {
            uint32_t obj = rd32(a), code = rd32(a + 4);
            char msg[160] = "";
            size_t pos = 8;
            get_str(a, n, &pos, msg, sizeof msg);
            set_err(w, "wayland error on object %u, code %u: %s", obj, code, msg);
            w->closed = 1;
        }
        return;
    }
    if (id == w->registry && op == 0) {
        size_t pos = 4;
        char iface[64];
        if (n < 12 || get_str(a, n, &pos, iface, sizeof iface) != 0 || pos + 4 > n) return;
        uint32_t name = rd32(a), ver = rd32(a + pos);
        if (!strcmp(iface, "wl_compositor")) w->g_compositor.name = name, w->g_compositor.version = ver;
        else if (!strcmp(iface, "wl_shm")) w->g_shm.name = name, w->g_shm.version = ver;
        else if (!strcmp(iface, "wl_webos_shell")) w->g_shell.name = name, w->g_shell.version = ver;
        else if (!strcmp(iface, "wl_webos_foreign")) w->g_foreign.name = name, w->g_foreign.version = ver;
        else if (!strcmp(iface, "wl_seat")) bind_seat(w, name, ver);
        return;
    }
    for (int i = 0; i < 2; i++) {
        if (w->buffer[i] && id == w->buffer[i]) {
            if (op == 0) w->busy[i] = 0;                       /* wl_buffer.release */
            return;
        }
    }
    if (w->sync_cb && id == w->sync_cb && op == 0) {
        w->sync_done = 1;
        return;
    }
    if (w->shell_surface && id == w->shell_surface && op == 2) {   /* close */
        w->closed = 1;
        return;
    }
    if (w->exported && id == w->exported && op == 0) {            /* window_id_assigned(string, uint) */
        size_t pos = 0;
        if (get_str(a, n, &pos, w->window_id, sizeof w->window_id) == 0) w->have_id = 1;
        return;
    }
    on_seat_event(w, id, op, a, n);
}

static int parse_in(vpl_wl *w) {
    size_t off = 0;
    while (w->in_len - off >= 8) {
        uint32_t id = rd32(w->in + off), so = rd32(w->in + off + 4);
        size_t size = so >> 16;
        uint32_t op = so & 0xFFFF;
        if (size < 8 || size > sizeof w->in) {
            set_err(w, "a message of %zu bytes from the compositor", size);
            return -1;
        }
        if (w->in_len - off < size) break;
        on_event(w, id, op, w->in + off + 8, size - 8);
        off += size;
    }
    if (off) {
        memmove(w->in, w->in + off, w->in_len - off);
        w->in_len -= off;
    }
    return 0;
}

/* Reads whatever is there (waiting up to timeout_ms for the first byte). 0 fine, 1 the connection ended. */
static int pump(vpl_wl *w, int timeout_ms) {
    struct pollfd p = {w->fd, POLLIN, 0};
    int r = poll(&p, 1, timeout_ms);
    if (r < 0 && errno != EINTR) {
        set_err(w, "poll: %s", strerror(errno));
        w->closed = 1;
        return 1;
    }
    if (r <= 0) return 0;
    for (;;) {
        if (w->in_len == sizeof w->in) {
            set_err(w, "the compositor's messages fill the input buffer");
            return 1;
        }
        ssize_t n = recv(w->fd, w->in + w->in_len, sizeof w->in - w->in_len, 0);
        if (n > 0) {
            w->in_len += (size_t)n;
            if (parse_in(w) != 0) {
                w->closed = 1;
                return 1;
            }
            if (w->out_len && flush_out(w) != 0) return 1;           /* what the events asked for (a seat's keyboard) */
            continue;
        }
        if (n == 0) {
            if (!w->failed) set_err(w, "the compositor closed the connection");
            w->closed = 1;
            return 1;
        }
        if (errno == EINTR) continue;
        if (errno == EAGAIN || errno == EWOULDBLOCK) return 0;
        set_err(w, "recv: %s", strerror(errno));
        w->closed = 1;
        return 1;
    }
}

/* A round trip: everything sent so far has been handled by the compositor when this returns 0. */
static int roundtrip(vpl_wl *w, int timeout_ms) {
    w->sync_cb = w->next++;
    w->sync_done = 0;
    size_t at = msg_begin(w, DISPLAY, DISPLAY_SYNC);
    put_u32(w, w->sync_cb);
    msg_end(w, at);
    if (flush_out(w) != 0) return -1;
    int64_t end = vpl_ms() + timeout_ms;
    while (!w->sync_done && !w->closed) {
        int64_t left = end - vpl_ms();
        if (left <= 0) {
            set_err(w, "the compositor did not answer within %d ms", timeout_ms);
            return -1;
        }
        pump(w, (int)left);
    }
    return w->closed && !w->sync_done ? -1 : 0;
}

/* ---- the window ------------------------------------------------------------------------------------------------------- */

static int connect_display(vpl_wl *w) {
    const char *s = getenv("WAYLAND_SOCKET");
    if (s && *s) {
        w->fd = atoi(s);
        unsetenv("WAYLAND_SOCKET");
        fcntl(w->fd, F_SETFD, FD_CLOEXEC);
    } else {
        const char *dir = getenv("XDG_RUNTIME_DIR");
        const char *name = getenv("WAYLAND_DISPLAY");
        if (!dir || !*dir) dir = "/tmp/xdg";
        if (!name || !*name) name = "wayland-0";
        struct sockaddr_un a = {.sun_family = AF_UNIX};
        int n = name[0] == '/' ? snprintf(a.sun_path, sizeof a.sun_path, "%s", name) : snprintf(a.sun_path, sizeof a.sun_path, "%s/%s", dir, name);
        if (n < 0 || (size_t)n >= sizeof a.sun_path) {
            set_err(w, "the socket path is too long");
            return -1;
        }
        w->fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (w->fd < 0 || connect(w->fd, (struct sockaddr *)&a, sizeof a) != 0) {
            set_err(w, "cannot connect to %s: %s", a.sun_path, strerror(errno));
            if (w->fd >= 0) close(w->fd);
            w->fd = -1;
            return -1;
        }
    }
    fcntl(w->fd, F_SETFL, fcntl(w->fd, F_GETFL) | O_NONBLOCK);
    return 0;
}

static void bind_global(vpl_wl *w, uint32_t name, const char *iface, uint32_t ver, uint32_t id) {
    size_t at = msg_begin(w, w->registry, REGISTRY_BIND);
    put_u32(w, name);
    put_str(w, iface);
    put_u32(w, ver);
    put_u32(w, id);
    msg_end(w, at);
}

static void set_prop(vpl_wl *w, const char *k, const char *v) {
    size_t at = msg_begin(w, w->shell_surface, SHELL_SURFACE_SET_PROPERTY);
    put_str(w, k);
    put_str(w, v);
    msg_end(w, at);
}

static int make_buffer(vpl_wl *w) {
    size_t stride = (size_t)w->w * 4;
    w->px_len = stride * (size_t)w->h;
    size_t pool_len = w->px_len * 2;
    w->shm_fd = (int)syscall(SYS_memfd_create, "lumatv-vplane", 1u /* MFD_CLOEXEC */);
    if (w->shm_fd < 0 || ftruncate(w->shm_fd, (off_t)pool_len) != 0) {
        set_err(w, "no shared memory for the window's buffer: %s", strerror(errno));
        return -1;
    }
    void *m = mmap(NULL, pool_len, PROT_READ | PROT_WRITE, MAP_SHARED, w->shm_fd, 0);
    if (m == MAP_FAILED) {
        set_err(w, "mmap of the window's buffer: %s", strerror(errno));
        return -1;
    }
    w->map = m;
    w->px[0] = m;
    w->px[1] = (uint32_t *)((uint8_t *)m + w->px_len);
    /* wl_shm.create_pool carries the descriptor out of band, so what is queued goes first and this message goes alone */
    if (flush_out(w) != 0) return -1;
    w->pool = w->next++;
    size_t at = msg_begin(w, w->shm, SHM_CREATE_POOL);
    put_u32(w, w->pool);
    put_u32(w, (uint32_t)pool_len);
    msg_end(w, at);
    union { struct cmsghdr h; char buf[CMSG_SPACE(sizeof(int))]; } u;
    memset(&u, 0, sizeof u);
    struct iovec iov = {w->out, w->out_len};
    struct msghdr mh = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = u.buf, .msg_controllen = sizeof u.buf};
    struct cmsghdr *c = CMSG_FIRSTHDR(&mh);
    c->cmsg_level = SOL_SOCKET;
    c->cmsg_type = SCM_RIGHTS;
    c->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(c), &w->shm_fd, sizeof(int));
    ssize_t n;
    do n = sendmsg(w->fd, &mh, MSG_NOSIGNAL);
    while (n < 0 && errno == EINTR);
    if (n != (ssize_t)w->out_len) {
        set_err(w, "sendmsg of the buffer's descriptor: %s", n < 0 ? strerror(errno) : "short write");
        return -1;
    }
    w->out_len = 0;
    for (int i = 0; i < 2; i++) {
        w->buffer[i] = w->next++;
        at = msg_begin(w, w->pool, POOL_CREATE_BUFFER);
        put_u32(w, w->buffer[i]);
        put_u32(w, (uint32_t)(w->px_len * (size_t)i));
        put_u32(w, (uint32_t)w->w);
        put_u32(w, (uint32_t)w->h);
        put_u32(w, (uint32_t)stride);
        put_u32(w, w->format);
        msg_end(w, at);
    }
    return 0;
}

int vpl_wl_commit(vpl_wl *w, int x, int y, int bw, int bh) {
    if (w->closed) return -1;
    if (w->busy[w->back]) return -1;          /* the caller painted a buffer it had no right to */
    if (x < 0) x = 0;
    if (y < 0) y = 0;
    if (x + bw > w->w) bw = w->w - x;
    if (y + bh > w->h) bh = w->h - y;
    size_t at = msg_begin(w, w->surface, SURFACE_ATTACH);
    put_u32(w, w->buffer[w->back]);
    put_u32(w, 0);
    put_u32(w, 0);
    msg_end(w, at);
    w->busy[w->back] = 1;
    w->back ^= 1;
    if (bw > 0 && bh > 0) {
        at = msg_begin(w, w->surface, SURFACE_DAMAGE);
        put_u32(w, (uint32_t)x);
        put_u32(w, (uint32_t)y);
        put_u32(w, (uint32_t)bw);
        put_u32(w, (uint32_t)bh);
        msg_end(w, at);
    }
    at = msg_begin(w, w->surface, SURFACE_COMMIT);
    msg_end(w, at);
    return flush_out(w);
}

static uint32_t make_region(vpl_wl *w, int x, int y, int rw, int rh) {
    uint32_t id = w->next++;
    size_t at = msg_begin(w, w->compositor, COMPOSITOR_CREATE_REGION);
    put_u32(w, id);
    msg_end(w, at);
    at = msg_begin(w, id, REGION_ADD);
    put_u32(w, (uint32_t)x);
    put_u32(w, (uint32_t)y);
    put_u32(w, (uint32_t)rw);
    put_u32(w, (uint32_t)rh);
    msg_end(w, at);
    return id;
}

int vpl_wl_video(vpl_wl *w, int sx, int sy, int sw, int sh, int dx, int dy, int dw, int dh) {
    if (w->closed) return -1;
    uint32_t src = make_region(w, sx, sy, sw, sh), dst = make_region(w, dx, dy, dw, dh);
    size_t at = msg_begin(w, w->exported, EXPORTED_SET_EXPORTED_WINDOW);
    put_u32(w, src);
    put_u32(w, dst);
    msg_end(w, at);
    at = msg_begin(w, src, REGION_DESTROY);
    msg_end(w, at);
    at = msg_begin(w, dst, REGION_DESTROY);
    msg_end(w, at);
    at = msg_begin(w, w->surface, SURFACE_COMMIT);
    msg_end(w, at);
    return flush_out(w);
}

static int export_element(vpl_wl *w, int timeout_ms) {
    w->exported = w->next++;
    size_t at = msg_begin(w, w->foreign, FOREIGN_EXPORT_ELEMENT);
    put_u32(w, w->exported);
    put_u32(w, w->surface);
    put_u32(w, 0);                  /* type 0: a video element (jellyfin-webos-native docs/ndl.md; the event echoes it) */
    msg_end(w, at);
    if (flush_out(w) != 0) return -1;
    int64_t end = vpl_ms() + timeout_ms;
    while (!w->have_id && !w->closed) {
        int64_t left = end - vpl_ms();
        if (left <= 0) {
            set_err(w, "no window id within %d ms of export_element", timeout_ms);
            return -1;
        }
        pump(w, (int)left);
    }
    return w->have_id ? 0 : -1;
}

static void teardown(vpl_wl *w) {
    if (w->fd >= 0) {
        if (!w->closed && w->surface) {
            if (w->exported) {
                size_t at = msg_begin(w, w->exported, EXPORTED_DESTROY);
                msg_end(w, at);
            }
            size_t at = msg_begin(w, w->surface, SURFACE_DESTROY);
            msg_end(w, at);
            flush_out(w);
        }
        close(w->fd);
    }
    if (w->map) munmap(w->map, w->px_len * 2);
    if (w->shm_fd > 0) close(w->shm_fd);
}

int vpl_wl_open(vpl_wl **out, const vpl_wl_opts *o, char *err, size_t cap) {
    vpl_wl *w = calloc(1, sizeof *w);
    if (!w) {
        snprintf(err, cap, "out of memory");
        return -1;
    }
    w->fd = -1;
    w->shm_fd = -1;
    w->next = 2;
    w->w = o->width;
    w->h = o->height;
    w->format = o->xrgb ? WL_SHM_FORMAT_XRGB8888 : WL_SHM_FORMAT_ARGB8888;
    int t = o->timeout_ms > 0 ? o->timeout_ms : 3000;
    if (connect_display(w) != 0) goto fail;

    w->registry = w->next++;
    size_t at = msg_begin(w, DISPLAY, DISPLAY_GET_REGISTRY);
    put_u32(w, w->registry);
    msg_end(w, at);
    if (roundtrip(w, t) != 0) goto fail;
    if (!w->g_compositor.name || w->g_compositor.version < 3 || !w->g_shm.name || !w->g_shell.name || !w->g_foreign.name) {
        set_err(w, "the compositor lacks %s", !w->g_compositor.name ? "wl_compositor" : w->g_compositor.version < 3 ? "wl_compositor v3" : !w->g_shm.name ? "wl_shm" : !w->g_shell.name ? "wl_webos_shell" : "wl_webos_foreign");
        goto fail;
    }
    w->compositor = w->next++;
    bind_global(w, w->g_compositor.name, "wl_compositor", 3, w->compositor);
    w->shm = w->next++;
    bind_global(w, w->g_shm.name, "wl_shm", 1, w->shm);
    w->shell = w->next++;
    bind_global(w, w->g_shell.name, "wl_webos_shell", 1, w->shell);
    w->foreign = w->next++;
    bind_global(w, w->g_foreign.name, "wl_webos_foreign", 1, w->foreign);

    w->surface = w->next++;
    at = msg_begin(w, w->compositor, COMPOSITOR_CREATE_SURFACE);
    put_u32(w, w->surface);
    msg_end(w, at);
    w->shell_surface = w->next++;
    at = msg_begin(w, w->shell, SHELL_GET_SHELL_SURFACE);
    put_u32(w, w->shell_surface);
    put_u32(w, w->surface);
    msg_end(w, at);
    set_prop(w, "appId", o->app_id);
    if (o->title && *o->title) set_prop(w, "title", o->title);
    if (o->window_type && *o->window_type) set_prop(w, "_WEBOS_WINDOW_TYPE", o->window_type);
    if (o->play_video) set_prop(w, "playVideo", "true");
    at = msg_begin(w, w->shell_surface, SHELL_SURFACE_SET_STATE);
    put_u32(w, (uint32_t)o->state);
    msg_end(w, at);
    if (make_buffer(w) != 0) goto fail;
    if (o->no_export) {
        if (vpl_wl_commit(w, 0, 0, 0, 0) != 0) goto fail;
    } else if (o->export_first) {
        if (export_element(w, t) != 0) goto fail;
        if (vpl_wl_commit(w, 0, 0, 0, 0) != 0) goto fail;
    } else {
        if (vpl_wl_commit(w, 0, 0, 0, 0) != 0) goto fail;
        if (export_element(w, t) != 0) goto fail;
    }
    if (roundtrip(w, t) != 0) goto fail;
    *out = w;
    return 0;
fail:
    snprintf(err, cap, "%s", w->failed ? w->err : "the window could not be made");
    teardown(w);
    free(w);
    return -1;
}

void vpl_wl_close(vpl_wl *w) {
    if (!w) return;
    teardown(w);
    free(w);
}

int vpl_wl_fd(const vpl_wl *w) { return w->fd; }

void vpl_wl_size(const vpl_wl *w, int *width, int *height) {
    if (width) *width = w->w;
    if (height) *height = w->h;
}

int vpl_wl_resize(vpl_wl *w, int width, int height, char *err, size_t cap) {
    if (width <= 0 || height <= 0 || width > 4096 || height > 4096) {
        snprintf(err, cap, "a window buffer of %d x %d is not one", width, height);
        return -1;
    }
    if (w->closed) {
        snprintf(err, cap, "the window is closed");
        return -1;
    }
    if (width == w->w && height == w->h) return 0;
    /* the compositor keeps what it has of a destroyed buffer until it has read it (a shm pool lives until its last buffer is gone), and the releases
     * that arrive for the old ids match no buffer of the new pair */
    for (int i = 0; i < 2; i++) {
        size_t at = msg_begin(w, w->buffer[i], BUFFER_DESTROY);
        msg_end(w, at);
    }
    size_t at = msg_begin(w, w->pool, POOL_DESTROY);
    msg_end(w, at);
    if (w->map) munmap(w->map, w->px_len * 2);
    w->map = NULL;
    if (w->shm_fd > 0) close(w->shm_fd);
    w->shm_fd = -1;
    w->busy[0] = w->busy[1] = 0;
    w->back = 0;
    w->w = width;
    w->h = height;
    if (make_buffer(w) != 0) {
        snprintf(err, cap, "%s", w->err);
        return -1;
    }
    return flush_out(w);
}

unsigned vpl_wl_activity(vpl_wl *w) {
    unsigned a = w->activity;
    w->activity = 0;
    return a;
}
const char *vpl_wl_window_id(const vpl_wl *w) { return w->window_id; }
uint32_t *vpl_wl_back(vpl_wl *w) { return w->busy[w->back] ? NULL : w->px[w->back]; }
int vpl_wl_flush(vpl_wl *w) { return flush_out(w); }

void vpl_wl_counts(const vpl_wl *w, unsigned *sent, unsigned *received) {
    if (sent) *sent = w->sent;
    if (received) *received = w->received;
}

int vpl_wl_dispatch(vpl_wl *w, char *err, size_t cap) {
    pump(w, 0);
    if (w->failed) {
        snprintf(err, cap, "%s", w->err);
        return w->closed ? 1 : -1;
    }
    if (w->closed) {
        snprintf(err, cap, "the compositor closed the window");
        return 1;
    }
    return 0;
}
