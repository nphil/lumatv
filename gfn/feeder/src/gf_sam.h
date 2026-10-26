/* lumatv-gfn-feeder: the app's life with SAM, the system application manager (docs/research/cardapp-result.md, "what SAM does with a
 * native app"; the same registration rustui/app/stub/lumatv-card.c makes for the card app).
 *
 * SAM starts a version 2 native app's `main` as `<main> '<json>'`: {"@system_native_app":true,"reason":"undefined"|"preload",
 * "interfaceVersion":2,"event":"launch","interfaceMethod":"registerApp","appId":...,"parameters":{...}}. The app registers a bus
 * name equal to its app id and calls luna://com.webos.applicationManager/registerApp {"appId":...,"subscribe":true}; SAM answers
 * {"event":"registered"} and then pushes on that call {"event":"relaunch","reason":...,"parameters":{...}} (the app is launched
 * again while it runs) and {"event":"close","reason":...} (the app is to end).
 *
 * luna-service2 and glib are 32-bit libraries the set has in /usr/lib, and the registration needs a glib main loop: both are loaded
 * with dlopen, on a thread of their own, so that this program stays a libc, libdl and libpthread program and the feeder's poll loop
 * is not a glib loop. The thread turns SAM's events into lines in the shape the UI's channel carries (rustui/app/src/lifecycle.rs:
 * {"kind":"sam","registered":true|false}, {"kind":"relaunch","reason":R,"parameters":{...}}, {"kind":"close","reason":R}) and the
 * feeder reads them from a descriptor. */
#ifndef GF_SAM_H
#define GF_SAM_H

#include <stddef.h>

typedef struct gf_sam gf_sam;

/* What SAM's launch argument says. `preload` is a start that must not show anything yet (reason "preload", or parameters.snapshot). */
struct gf_launch {
    char app_id[128];
    char reason[40];
    int preload;
};

/* 0 and `out` filled when `json` is a launch object (it has "event":"launch" or "@system_native_app"); -1 otherwise. */
int gf_sam_parse_launch(const char *json, struct gf_launch *out);

/* Registers `app_id` with SAM on a new thread. The libraries are named as the loader finds them (LD_LIBRARY_PATH moves them for the
 * host test). NULL, with `err` filled, when they cannot be loaded or the thread cannot start; a hub that refuses the name is not an
 * error here: the thread says {"kind":"sam","registered":false} and ends, and the app runs without SAM's events. */
gf_sam *gf_sam_start(const char *app_id, char *err, size_t cap);

/* Readable when a line is waiting. */
int gf_sam_fd(const gf_sam *s);

/* The next line SAM's events made, without its newline: 1 when there was one, 0 when none. */
int gf_sam_next(gf_sam *s, char *line, size_t cap);

/* Cancels the subscription, unregisters, ends the thread. */
void gf_sam_stop(gf_sam *s);

#endif
