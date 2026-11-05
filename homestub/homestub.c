/* lumatv-homestub: LG's Home id as a native program instead of a 32 MB web renderer.
 *
 * SAM, bootd, the wake and the saver know Home by its app id and by a card of that id being mapped; LumaTV's compositor draws the real
 * Home itself. This program is that card: one 1x1 transparent buffer on a wl_webos_shell surface whose appId is com.webos.app.home,
 * mapped fullscreen, then poll() until the compositor closes it or SAM ends it. payload/homestub-native/appinfo.json makes it Home's
 * `type: native` app; tools/lumatv binds that directory over /usr/palm/applications/com.webos.app.home in place of the web stub's.
 * docs/research/boot-direct.md, "A native stub".
 *
 * What it does not do: talk to the luna bus. The hub names a client after its app id only if it has a role for the executable, and
 * Home's role is LG's Flutter client's; so the manifest names no nativeLifeCycleInterfaceVersion (SAM then expects no registerApp, as
 * for Moonlight and Punktfunk on this set) and SAM's relaunch, close and memory reclaim reach it as the signals they are.
 *
 * Static and libc-light on purpose: the TV's own libraries are 32-bit and this is not. The Wayland wire protocol is spoken by hand
 * (a dozen requests, one fd passed for the pixel's shared memory).
 *
 * Environment: XDG_RUNTIME_DIR (/tmp/xdg), WAYLAND_DISPLAY (wayland-0), WAYLAND_SOCKET (an inherited connected fd, as libwayland
 * reads it). A SAM launch argument that names a preload or a snapshot makes the window and maps nothing: a card that gets its first
 * buffer comes to the front in both compositors, and a preload must not take the screen from the app on it.
 * Exit status 0 when the card was closed, replaced or the compositor went away; 1 when it could not be made.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#define APP_ID "com.webos.app.home"
/* Abstract unix socket that keeps one card: a newer instance asks the one it finds to go. */
#define SINGLE_NAME "lumatv-homestub"
/* A compositor that is restarting is waited for this long in all: its socket every STEP_MS, and a connection that found no wl_webos_shell
 * (surface-manager registers its globals late) again after RETRY_MS. */
#define WAIT_MS 30000
#define STEP_MS 100
#define RETRY_MS 500

/* The objects of one connection, numbered as this client creates them. */
enum { DISPLAY = 1, REGISTRY, SYNC, COMPOSITOR, SHM, SHELL, SURFACE, SHELL_SURFACE, POOL, BUFFER };

enum { STATE_FULLSCREEN = 3 };
enum { FORMAT_ARGB8888 = 0 };

static void say(const char *what, const char *detail)
{
    const char *parts[] = { "lumatv-homestub: ", what, detail ? ": " : "", detail ? detail : "", "\n" };
    for (size_t i = 0; i < sizeof parts / sizeof *parts; i++)
        if (write(2, parts[i], strlen(parts[i])) < 0)
            return;
}

static void on_signal(int sig)
{
    (void)sig;
    _exit(0);
}

static long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

static void nap_ms(long ms)
{
    struct timespec t = { ms / 1000, (ms % 1000) * 1000000L };
    nanosleep(&t, NULL);
}

/* ---- the wire: little words in the host's order, strings padded to four bytes ---- */

struct msg {
    uint32_t w[48];
    size_t n;
};

static void m_begin(struct msg *m, uint32_t object, uint32_t opcode)
{
    m->w[0] = object;
    m->w[1] = opcode;
    m->n = 2;
}

static void m_u32(struct msg *m, uint32_t v)
{
    m->w[m->n++] = v;
}

static void m_str(struct msg *m, const char *s)
{
    uint32_t len = (uint32_t)strlen(s) + 1;
    m_u32(m, len);
    m->w[m->n + (len - 1) / 4] = 0;
    memcpy(&m->w[m->n], s, len);
    m->n += (len + 3) / 4;
}

/* The size goes in the header's high half. `fd` >= 0 travels with the message. */
static int m_send(int sock, struct msg *m, int fd)
{
    char control[CMSG_SPACE(sizeof(int))];
    struct iovec iov = { m->w, m->n * 4 };
    struct msghdr mh = { .msg_iov = &iov, .msg_iovlen = 1 };
    m->w[1] |= (uint32_t)(m->n * 4) << 16;
    if (fd >= 0) {
        memset(control, 0, sizeof control);
        mh.msg_control = control;
        mh.msg_controllen = sizeof control;
        struct cmsghdr *c = CMSG_FIRSTHDR(&mh);
        c->cmsg_level = SOL_SOCKET;
        c->cmsg_type = SCM_RIGHTS;
        c->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(c), &fd, sizeof fd);
    }
    return sendmsg(sock, &mh, MSG_NOSIGNAL) == (ssize_t)(m->n * 4) ? 0 : -1;
}

/* ---- what the compositor says ---- */

struct events {
    uint32_t compositor, shm, shell; /* registry names, 0 when not advertised */
    int synced;
    int closed;  /* wl_webos_shell_surface.close */
    int failed;  /* wl_display.error */
};

static uint8_t inbuf[4096];
static size_t inlen;

static void on_global(struct events *e, const uint8_t *p, uint32_t len)
{
    uint32_t name, slen;
    if (len < 12)
        return;
    memcpy(&name, p, 4);
    memcpy(&slen, p + 4, 4);
    if (slen == 0 || 8 + (size_t)slen + 4 > len)
        return;
    const char *iface = (const char *)p + 8;
    if (!strcmp(iface, "wl_compositor"))
        e->compositor = name;
    else if (!strcmp(iface, "wl_shm"))
        e->shm = name;
    else if (!strcmp(iface, "wl_webos_shell"))
        e->shell = name;
}

static void on_message(struct events *e, uint32_t object, uint32_t opcode, const uint8_t *p, uint32_t len)
{
    if (object == DISPLAY && opcode == 0) { /* error(object_id, code, message) */
        e->failed = 1;
        if (len >= 16 && p[12 + 0] != 0)
            say("the compositor says", (const char *)p + 12);
    } else if (object == REGISTRY && opcode == 0) {
        on_global(e, p, len);
    } else if (object == SYNC && opcode == 0) {
        e->synced = 1;
    } else if (object == SHELL_SURFACE && opcode == 2) {
        e->closed = 1;
    }
}

/* Reads what is waiting. 0: go on, 1: the compositor is gone, -1: the stream is broken. */
static int pump(int sock, struct events *e)
{
    ssize_t n = recv(sock, inbuf + inlen, sizeof inbuf - inlen, MSG_DONTWAIT);
    if (n == 0)
        return 1;
    if (n < 0)
        return errno == EAGAIN || errno == EINTR ? 0 : -1;
    inlen += (size_t)n;
    size_t off = 0;
    while (inlen - off >= 8) {
        uint32_t object, header;
        memcpy(&object, inbuf + off, 4);
        memcpy(&header, inbuf + off + 4, 4);
        uint32_t size = header >> 16;
        if (size < 8 || size > sizeof inbuf)
            return -1;
        if (inlen - off < size)
            break;
        on_message(e, object, header & 0xffff, inbuf + off + 8, size - 8);
        off += size;
    }
    memmove(inbuf, inbuf + off, inlen - off);
    inlen -= off;
    return 0;
}

/* ---- the connection ---- */

static int dial(void)
{
    const char *inherited = getenv("WAYLAND_SOCKET");
    if (inherited && *inherited) {
        int fd = atoi(inherited);
        unsetenv("WAYLAND_SOCKET");
        fcntl(fd, F_SETFD, FD_CLOEXEC);
        return fd;
    }
    const char *display = getenv("WAYLAND_DISPLAY");
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    struct sockaddr_un addr = { .sun_family = AF_UNIX };
    int n;
    if (!display || !*display)
        display = "wayland-0";
    if (!runtime || !*runtime)
        runtime = "/tmp/xdg";
    if (*display == '/')
        n = (int)strlen(display);
    else
        n = (int)(strlen(runtime) + 1 + strlen(display));
    if ((size_t)n >= sizeof addr.sun_path) {
        say("the compositor's socket path is too long", NULL);
        return -1;
    }
    if (*display == '/')
        strcpy(addr.sun_path, display);
    else {
        strcpy(addr.sun_path, runtime);
        strcat(addr.sun_path, "/");
        strcat(addr.sun_path, display);
    }
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return -1;
    if (connect(fd, (struct sockaddr *)&addr, sizeof addr) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

/* The shell surface of the card and the one pixel on it (not the pixel, for a preload). 1: made; 0: the compositor lacks what is needed
 * (yet); -1: it went away or refused. */
static int map_card(int sock, int hidden)
{
    struct events e = { 0 };
    struct msg m;

    inlen = 0;
    m_begin(&m, DISPLAY, 1); /* get_registry */
    m_u32(&m, REGISTRY);
    if (m_send(sock, &m, -1))
        return -1;
    m_begin(&m, DISPLAY, 0); /* sync */
    m_u32(&m, SYNC);
    if (m_send(sock, &m, -1))
        return -1;
    while (!e.synced) {
        struct pollfd p = { sock, POLLIN, 0 };
        if (poll(&p, 1, 5000) <= 0 || pump(sock, &e) || e.failed)
            return -1;
    }
    if (!e.compositor || !e.shm || !e.shell)
        return 0;

    /* All three at version 1: wl_webos_shell is version 1 on the set and the rest is all this needs. */
    static const char *const names[] = { "wl_compositor", "wl_shm", "wl_webos_shell" };
    const uint32_t globals[] = { e.compositor, e.shm, e.shell };
    for (uint32_t i = 0; i < 3; i++) {
        m_begin(&m, REGISTRY, 0); /* bind(name, interface, version, id) */
        m_u32(&m, globals[i]);
        m_str(&m, names[i]);
        m_u32(&m, 1);
        m_u32(&m, COMPOSITOR + i);
        if (m_send(sock, &m, -1))
            return -1;
    }
    m_begin(&m, COMPOSITOR, 0); /* create_surface */
    m_u32(&m, SURFACE);
    if (m_send(sock, &m, -1))
        return -1;
    m_begin(&m, SHELL, 1); /* get_shell_surface (request 0 is the deprecated get_system_pip) */
    m_u32(&m, SHELL_SURFACE);
    m_u32(&m, SURFACE);
    if (m_send(sock, &m, -1))
        return -1;
    static const char *const props[][2] = {
        { "_WEBOS_WINDOW_TYPE", "_WEBOS_WINDOW_TYPE_CARD" }, { "appId", APP_ID }, { "title", "Home" },
    };
    for (size_t i = 0; i < sizeof props / sizeof *props; i++) {
        m_begin(&m, SHELL_SURFACE, 2); /* set_property */
        m_str(&m, props[i][0]);
        m_str(&m, props[i][1]);
        if (m_send(sock, &m, -1))
            return -1;
    }
    if (hidden)
        return 1;
    m_begin(&m, SHELL_SURFACE, 1); /* set_state: a card that asks for fullscreen is raised over the one it covers */
    m_u32(&m, STATE_FULLSCREEN);
    if (m_send(sock, &m, -1))
        return -1;

    int pixel = memfd_create("lumatv-homestub", MFD_CLOEXEC);
    if (pixel < 0 || ftruncate(pixel, 4) < 0) { /* zeros: ARGB 0, fully transparent */
        say("no shared memory for the pixel", NULL);
        return -1;
    }
    m_begin(&m, SHM, 0); /* create_pool(id, fd, size) */
    m_u32(&m, POOL);
    m_u32(&m, 4);
    int sent = m_send(sock, &m, pixel);
    close(pixel);
    if (sent)
        return -1;
    static const uint32_t buffer[] = { BUFFER, 0, 1, 1, 4, FORMAT_ARGB8888 };
    m_begin(&m, POOL, 0); /* create_buffer(id, offset, width, height, stride, format) */
    for (size_t i = 0; i < sizeof buffer / sizeof *buffer; i++)
        m_u32(&m, buffer[i]);
    if (m_send(sock, &m, -1))
        return -1;
    m_begin(&m, POOL, 1); /* destroy: the buffer keeps the memory */
    if (m_send(sock, &m, -1))
        return -1;
    m_begin(&m, SURFACE, 1); /* attach(buffer, x, y) */
    m_u32(&m, BUFFER);
    m_u32(&m, 0);
    m_u32(&m, 0);
    if (m_send(sock, &m, -1))
        return -1;
    static const uint32_t damage[] = { 0, 0, 1, 1 };
    m_begin(&m, SURFACE, 2); /* damage(x, y, width, height) */
    for (size_t i = 0; i < sizeof damage / sizeof *damage; i++)
        m_u32(&m, damage[i]);
    if (m_send(sock, &m, -1))
        return -1;
    m_begin(&m, SURFACE, 6); /* commit */
    return m_send(sock, &m, -1) ? -1 : 1;
}

/* ---- one card at a time: the newest instance wins ---- */

static socklen_t single_addr(struct sockaddr_un *a)
{
    memset(a, 0, sizeof *a);
    a->sun_family = AF_UNIX;
    memcpy(a->sun_path + 1, SINGLE_NAME, sizeof SINGLE_NAME - 1);
    return (socklen_t)(offsetof(struct sockaddr_un, sun_path) + 1 + sizeof SINGLE_NAME - 1);
}

/* Binds the name. Whoever holds it is asked to go by a connection to it, and its end is waited for. The card is mapped before this, so
 * that the one that ends is replaced on the screen and not uncovered. -1 when the name is not to be had: this one runs without it. */
static int claim(void)
{
    struct sockaddr_un a;
    socklen_t len = single_addr(&a);
    for (int tries = 0; tries < 10; tries++) {
        int s = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (s < 0)
            return -1;
        if (bind(s, (struct sockaddr *)&a, len) == 0 && listen(s, 2) == 0)
            return s;
        close(s);
        int c = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (c < 0)
            return -1;
        if (connect(c, (struct sockaddr *)&a, len) == 0) {
            struct pollfd p = { c, POLLIN, 0 };
            poll(&p, 1, 1000); /* readable when the other has gone and its listener with it: the name is free at once */
            close(c);
            continue;
        }
        close(c);
        nap_ms(STEP_MS); /* nobody answered: the name is being given up */
    }
    return -1;
}

/* ---- the idle ---- */

static int idle(int sock, int single)
{
    struct events e = { 0 };
    struct pollfd p[2] = { { sock, POLLIN, 0 }, { single, POLLIN, 0 } };
    for (;;) {
        if (poll(p, single >= 0 ? 2 : 1, -1) < 0) {
            if (errno == EINTR)
                continue;
            return 1;
        }
        if (single >= 0 && (p[1].revents & POLLIN))
            return 0; /* replaced: the one asking is mapped already */
        if (p[0].revents) {
            int r = pump(sock, &e);
            if (r > 0 || (p[0].revents & (POLLHUP | POLLERR)))
                return 0; /* the compositor is gone: SAM and the boot flow start Home again */
            if (r < 0 || e.failed)
                return 1;
            if (e.closed)
                return 0;
        }
    }
}

static int is_hidden_launch(const char *json)
{
    return json && (strstr(json, "\"preload\"") || strstr(json, "\"snapshot\":true") || strstr(json, "\"snapshot\": true"));
}

int main(int argc, char **argv)
{
    int hidden = argc > 1 && is_hidden_launch(argv[1]);
    struct sigaction sa = { .sa_handler = on_signal };
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    signal(SIGPIPE, SIG_IGN); /* a closed stderr must not end the card */

    long deadline = now_ms() + WAIT_MS;
    int sock = -1;
    for (;;) {
        int inherited = getenv("WAYLAND_SOCKET") != NULL;
        sock = dial();
        if (sock >= 0) {
            int r = map_card(sock, hidden);
            if (r > 0)
                break;
            close(sock);
            if (inherited)
                return 1;
        }
        if (now_ms() >= deadline) {
            say(sock < 0 ? "no compositor to connect to" : "the compositor has no wl_webos_shell", NULL);
            return 1;
        }
        nap_ms(sock < 0 ? STEP_MS : RETRY_MS);
    }
    return idle(sock, claim());
}
