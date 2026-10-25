/* Host test double for LG's libvtcapture histogram API (never shipped). Behaviour is steered by the environment:
 *   FAKE_VT_DENY=1            vtCapture_initHistogram returns NULL
 *   FAKE_VT_POLICY_MS=n       n ms after init, the registered policy callback is called from a thread
 *   FAKE_VT_FIN_LOG=path      vtCapture_finalizeHistogram appends a line there
 *   FAKE_VT_EAGAIN_EVERY=n    every n-th getHistogram returns 11 (EAGAIN) without data
 * The picture is a new frame every 33 ms: area i is a flat colour that drifts with the frame number. */
#define _GNU_SOURCE
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "vthist.h"

static int g_n;
static vth_policy_cb g_cb;
static int g_calls;

static uint64_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000u + (uint64_t)ts.tv_nsec / 1000000u;
}

static void *policy_thread(void *arg) {
    usleep((useconds_t)(intptr_t)arg * 1000u);
    if (g_cb) g_cb();
    return NULL;
}

void *vtCapture_initHistogram(vth_policy_cb cb) {
    if (getenv("FAKE_VT_DENY")) return NULL;
    g_cb = cb;
    if (getenv("FAKE_VT_POLICY_MS")) {
        pthread_t t;
        pthread_create(&t, NULL, policy_thread, (void *)(intptr_t)atoi(getenv("FAKE_VT_POLICY_MS")));
        pthread_detach(t);
    }
    return (void *)0x1234;
}

int vtCapture_setPresetHistogram(void *h, int preset) {
    if (!h) return 2;
    if (preset < 0 || preset > 3) return 3;
    g_n = preset == 3 ? 14 : 16;
    return 0;
}

int vtCapture_getHistogram(void *h, struct vth_zone *out) {
    if (!h) return 2;
    if (!out) return 3;
    const char *e = getenv("FAKE_VT_EAGAIN_EVERY");
    if (e && ++g_calls % atoi(e) == 0) return 11;
    uint64_t frame = now_ms() / 33;
    memset(out, 0, (size_t)g_n * sizeof *out);
    for (int i = 0; i < g_n; i++) {
        out[i].area_id = (uint32_t)i;
        out[i].r[(i * 16 + frame) % 256] = 1000;
        out[i].g[(i * 8) % 256] = 1000;
        out[i].b[255 - (i * 4) % 256] = 1000;
    }
    return 0;
}

int vtCapture_finalizeHistogram(void *h) {
    if (!h) return 2;
    const char *p = getenv("FAKE_VT_FIN_LOG");
    if (p) {
        FILE *f = fopen(p, "a");
        if (f) { fputs("finalized\n", f); fclose(f); }
    }
    return 0;
}
