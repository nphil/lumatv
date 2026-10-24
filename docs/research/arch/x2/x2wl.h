// Minimal libwayland 1.20 declarations for the X2 probes (no wayland headers on the build host).
// Only what x2srv.c and x2cli.c use. ABI as in wayland-util.h / wayland-server-core.h / wayland-client-core.h.
#pragma once
#include <stdint.h>
#include <stddef.h>

struct wl_message { const char *name; const char *signature; const struct wl_interface **types; };
struct wl_interface { const char *name; int version; int method_count; const struct wl_message *methods; int event_count; const struct wl_message *events; };
struct wl_array { size_t size; size_t alloc; void *data; };
struct wl_list { struct wl_list *prev; struct wl_list *next; };
struct wl_listener;
typedef void (*wl_notify_func_t)(struct wl_listener *listener, void *data);
struct wl_listener { struct wl_list link; wl_notify_func_t notify; };
union wl_argument { int32_t i; uint32_t u; int32_t f; const char *s; void *o; uint32_t n; struct wl_array *a; int32_t h; };

// Core interfaces exported by both libwayland-client and libwayland-server.
extern const struct wl_interface wl_display_interface, wl_registry_interface, wl_callback_interface,
    wl_compositor_interface, wl_surface_interface, wl_region_interface, wl_shell_interface,
    wl_shell_surface_interface, wl_shm_interface, wl_shm_pool_interface, wl_buffer_interface,
    wl_seat_interface, wl_output_interface, wl_subcompositor_interface;

#define WL_MARSHAL_FLAG_DESTROY 1

#ifdef X2_SERVER
struct wl_display; struct wl_client; struct wl_resource; struct wl_global; struct wl_event_loop; struct wl_event_source;
typedef void (*wl_global_bind_func_t)(struct wl_client *client, void *data, uint32_t version, uint32_t id);
typedef void (*wl_resource_destroy_func_t)(struct wl_resource *resource);
typedef int (*wl_event_loop_timer_func_t)(void *data);
struct wl_display *wl_display_create(void);
void wl_display_destroy(struct wl_display *display);
int wl_display_add_socket(struct wl_display *display, const char *name);
int wl_display_init_shm(struct wl_display *display);
struct wl_event_loop *wl_display_get_event_loop(struct wl_display *display);
void wl_display_run(struct wl_display *display);
void wl_display_terminate(struct wl_display *display);
void wl_display_flush_clients(struct wl_display *display);
uint32_t wl_display_next_serial(struct wl_display *display);
void wl_display_add_client_created_listener(struct wl_display *display, struct wl_listener *listener);
void wl_client_add_destroy_listener(struct wl_client *client, struct wl_listener *listener);
void wl_client_get_credentials(struct wl_client *client, int32_t *pid, uint32_t *uid, uint32_t *gid);
struct wl_global *wl_global_create(struct wl_display *display, const struct wl_interface *interface, int version, void *data, wl_global_bind_func_t bind);
struct wl_resource *wl_resource_create(struct wl_client *client, const struct wl_interface *interface, int version, uint32_t id);
void wl_resource_set_implementation(struct wl_resource *resource, const void *implementation, void *data, wl_resource_destroy_func_t destroy);
void wl_resource_post_event(struct wl_resource *resource, uint32_t opcode, ...);
void wl_resource_post_error(struct wl_resource *resource, uint32_t code, const char *msg, ...);
void wl_resource_destroy(struct wl_resource *resource);
void *wl_resource_get_user_data(struct wl_resource *resource);
int wl_resource_get_version(struct wl_resource *resource);
uint32_t wl_resource_get_id(struct wl_resource *resource);
struct wl_client *wl_resource_get_client(struct wl_resource *resource);
struct wl_event_source *wl_event_loop_add_timer(struct wl_event_loop *loop, wl_event_loop_timer_func_t func, void *data);
int wl_event_source_timer_update(struct wl_event_source *source, int ms_delay);
struct wl_shm_buffer *wl_shm_buffer_get(struct wl_resource *resource);
#endif

#ifdef X2_CLIENT
struct wl_display; struct wl_proxy; struct wl_event_queue;
struct wl_display *wl_display_connect(const char *name);
void wl_display_disconnect(struct wl_display *display);
int wl_display_roundtrip(struct wl_display *display);
int wl_display_dispatch(struct wl_display *display);
int wl_display_dispatch_pending(struct wl_display *display);
int wl_display_flush(struct wl_display *display);
int wl_display_get_error(struct wl_display *display);
struct wl_proxy *wl_proxy_marshal_flags(struct wl_proxy *proxy, uint32_t opcode, const struct wl_interface *interface, uint32_t version, uint32_t flags, ...);
int wl_proxy_add_listener(struct wl_proxy *proxy, void (**implementation)(void), void *data);
uint32_t wl_proxy_get_version(struct wl_proxy *proxy);
void wl_proxy_destroy(struct wl_proxy *proxy);
struct wl_region; struct wl_surface; struct wl_shell_surface; struct wl_registry; struct wl_compositor; struct wl_shell; struct wl_callback; struct wl_shm; struct wl_shm_pool; struct wl_buffer;

#define P(x) ((struct wl_proxy *)(x))
static inline struct wl_registry *wl_display_get_registry(struct wl_display *d) { return (struct wl_registry *)wl_proxy_marshal_flags(P(d), 1, &wl_registry_interface, 1, 0, NULL); }
static inline void *wl_registry_bind(struct wl_registry *r, uint32_t name, const struct wl_interface *i, uint32_t v) { return wl_proxy_marshal_flags(P(r), 0, i, v, 0, name, i->name, v, NULL); }
static inline struct wl_surface *wl_compositor_create_surface(struct wl_compositor *c) { return (struct wl_surface *)wl_proxy_marshal_flags(P(c), 0, &wl_surface_interface, wl_proxy_get_version(P(c)), 0, NULL); }
static inline struct wl_shell_surface *wl_shell_get_shell_surface(struct wl_shell *s, struct wl_surface *su) { return (struct wl_shell_surface *)wl_proxy_marshal_flags(P(s), 0, &wl_shell_surface_interface, wl_proxy_get_version(P(s)), 0, NULL, su); }
static inline void wl_shell_surface_pong(struct wl_shell_surface *s, uint32_t serial) { wl_proxy_marshal_flags(P(s), 0, NULL, wl_proxy_get_version(P(s)), 0, serial); }
static inline void wl_shell_surface_set_toplevel(struct wl_shell_surface *s) { wl_proxy_marshal_flags(P(s), 3, NULL, wl_proxy_get_version(P(s)), 0); }
static inline void wl_shell_surface_set_title(struct wl_shell_surface *s, const char *t) { wl_proxy_marshal_flags(P(s), 8, NULL, wl_proxy_get_version(P(s)), 0, t); }
static inline void wl_surface_attach(struct wl_surface *s, struct wl_buffer *b, int32_t x, int32_t y) { wl_proxy_marshal_flags(P(s), 1, NULL, wl_proxy_get_version(P(s)), 0, b, x, y); }
static inline void wl_surface_damage(struct wl_surface *s, int32_t x, int32_t y, int32_t w, int32_t h) { wl_proxy_marshal_flags(P(s), 2, NULL, wl_proxy_get_version(P(s)), 0, x, y, w, h); }
static inline struct wl_callback *wl_surface_frame(struct wl_surface *s) { return (struct wl_callback *)wl_proxy_marshal_flags(P(s), 3, &wl_callback_interface, wl_proxy_get_version(P(s)), 0, NULL); }
static inline void wl_surface_commit(struct wl_surface *s) { wl_proxy_marshal_flags(P(s), 6, NULL, wl_proxy_get_version(P(s)), 0); }
static inline struct wl_shm_pool *wl_shm_create_pool(struct wl_shm *s, int fd, int32_t size) { return (struct wl_shm_pool *)wl_proxy_marshal_flags(P(s), 0, &wl_shm_pool_interface, wl_proxy_get_version(P(s)), 0, NULL, fd, size); }
static inline struct wl_buffer *wl_shm_pool_create_buffer(struct wl_shm_pool *p, int32_t off, int32_t w, int32_t h, int32_t stride, uint32_t fmt) { return (struct wl_buffer *)wl_proxy_marshal_flags(P(p), 0, &wl_buffer_interface, wl_proxy_get_version(P(p)), 0, NULL, off, w, h, stride, fmt); }
static inline void wl_buffer_destroy(struct wl_buffer *b) { wl_proxy_marshal_flags(P(b), 0, NULL, wl_proxy_get_version(P(b)), WL_MARSHAL_FLAG_DESTROY); }
#endif

// --- Private protocols, tables recovered from LG's libmali.so.0.46.0 (arch/x2-libmali32-protocols.txt) and Mesa's
// wayland-drm.xml / wayland-protocols linux-dmabuf-unstable-v1 v3. Defined in x2proto.c.
extern const struct wl_interface wl_drm_interface, mali_buffer_sharing_interface,
    zwp_linux_dmabuf_v1_interface, zwp_linux_buffer_params_v1_interface;
