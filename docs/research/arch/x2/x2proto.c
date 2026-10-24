// Protocol tables (what wayland-scanner would emit) for the three buffer protocols LG's libmali speaks.
// mali_buffer_sharing v5 as recovered from libmali.so.0.46.0 on the C4 (two requests, one event);
// wl_drm v2 from Mesa's wayland-drm.xml; zwp_linux_dmabuf_v1 v3 from wayland-protocols.
#include "x2wl.h"

static const struct wl_interface *types_none[12] = {0};
static const struct wl_interface *types_buf[12] = { &wl_buffer_interface };
static const struct wl_interface *types_params_new[2] = { &zwp_linux_buffer_params_v1_interface };

static const struct wl_message mali_buffer_sharing_requests[] = {
    { "create_buffer", "niiuuuuh", types_buf },
    { "auth", "u", types_none },
};
static const struct wl_message mali_buffer_sharing_events[] = {
    { "alloc_device", "s", types_none },
};
const struct wl_interface mali_buffer_sharing_interface = { "mali_buffer_sharing", 5, 2, mali_buffer_sharing_requests, 1, mali_buffer_sharing_events };

static const struct wl_message wl_drm_requests[] = {
    { "authenticate", "u", types_none },
    { "create_buffer", "nuiiuu", types_buf },
    { "create_planar_buffer", "nuiiuiiiiii", types_buf },
    { "create_prime_buffer", "2nhiiuiiiiii", types_buf },
};
static const struct wl_message wl_drm_events[] = {
    { "device", "s", types_none },
    { "format", "u", types_none },
    { "authenticated", "", types_none },
    { "capabilities", "u", types_none },
};
const struct wl_interface wl_drm_interface = { "wl_drm", 2, 4, wl_drm_requests, 4, wl_drm_events };

static const struct wl_message zwp_linux_dmabuf_v1_requests[] = {
    { "destroy", "", types_none },
    { "create_params", "n", types_params_new },
};
static const struct wl_message zwp_linux_dmabuf_v1_events[] = {
    { "format", "u", types_none },
    { "modifier", "3uuu", types_none },
};
const struct wl_interface zwp_linux_dmabuf_v1_interface = { "zwp_linux_dmabuf_v1", 3, 2, zwp_linux_dmabuf_v1_requests, 2, zwp_linux_dmabuf_v1_events };

static const struct wl_message zwp_linux_buffer_params_v1_requests[] = {
    { "destroy", "", types_none },
    { "add", "huuuuu", types_none },
    { "create", "iiuu", types_none },
    { "create_immed", "2niiuu", types_buf },
};
static const struct wl_message zwp_linux_buffer_params_v1_events[] = {
    { "created", "n", types_buf },
    { "failed", "", types_none },
};
const struct wl_interface zwp_linux_buffer_params_v1_interface = { "zwp_linux_buffer_params_v1", 3, 4, zwp_linux_buffer_params_v1_requests, 2, zwp_linux_buffer_params_v1_events };
