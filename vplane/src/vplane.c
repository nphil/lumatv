/* lumatv-vplane: the TV's video decoder as the screen saver's picture, so the GPU is not.
 *
 * A small root service of LumaTV's package (32-bit, because LG's media libraries are). It sits idle on a mailbox (a directory with a
 * request file the shell's QML writes and a status file it writes back: no socket, no bus client, no library loaded, no window) until a
 * `start` request, and then:
 *   - makes a window on LG's compositor (vpl_wl.c) with a video element exported on it, transparent but for what the player draws;
 *   - plays the art mode's prints on that element through NDL DirectMedia (vpl_ndl.c): one clip per print, a key picture followed by
 *     predicted copies of it that the player feeds at the clip's rate and loops (vpl_vpc.c, tools/vplane_clips.py);
 *   - drifts the element a few pixels every few seconds (the art mode's burn-in rule, vpl_plan.c), dims it over the first hour,
 *     changes the print every N minutes through black, and draws the corner clock above the video (vpl_ui.c);
 *   - stops when told, when the requester stops renewing (ttl), when the compositor closes the window, or when something fails: the
 *     window is destroyed first, so the screen is the shell's again at once.
 * Nothing here decides anything about the TV beyond that window. docs/research/videoplane-saver-result.md has the design and the TV steps.
 *
 *   lumatv-vplane [-d MAILBOX] [-c CLIPS] [-L NDL_LIB] [-a APP_ID] [-w screensaver|card|overlay|default] [-m KB]
 *   lumatv-vplane -n [-c CLIPS] [-L NDL_LIB]        probe: resolve the library, read the clips, print one JSON line, exit
 *   lumatv-vplane --once CLIP SECONDS [options]      play one clip for SECONDS with no mailbox (a TV step), then exit
 *   lumatv-vplane --vpc-info FILE                    print a clip's header as JSON
 *   lumatv-vplane --calc WHAT ARGS...                the arithmetic of vpl_plan.c, for the tests
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#include "vpl.h"
#include "vpl_json.h"
#include "vpl_ndl.h"
#include "vpl_plan.h"
#include "vpl_ui.h"
#include "vpl_vpc.h"
#include "vpl_wl.h"

#define SCREEN_W 1920
#define SCREEN_H 1080
#define EVENT_TTL_MAX 600000
#define HOLD_MS 400
#define VEIL_STEP_MS 33
#define STATUS_PLAYING_MS 5000
#define STATUS_IDLE_MS 60000
#define QUEUE_LIMIT 12

/* ---- log, clock, parameters ---------------------------------------------------------------------------------------------- */

void vpl_say(const char *fmt, ...) {
    static char last[256];
    static int64_t last_ms;
    char line[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    int64_t now = vpl_ms();
    if (strcmp(line, last) == 0 && now - last_ms < 10000) return;
    snprintf(last, sizeof last, "%s", line);
    last_ms = now;
    time_t t = time(NULL);
    struct tm tm;
    char stamp[16] = "";
    if (localtime_r(&t, &tm)) strftime(stamp, sizeof stamp, "%H:%M:%S", &tm);
    fprintf(stderr, "%s lumatv-vplane: %s\n", stamp, line);
    fflush(stderr);
}

int64_t vpl_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}
int64_t vpl_ms(void) { return vpl_us() / 1000; }
int64_t vpl_wall_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (int64_t)tv.tv_sec * 1000 + tv.tv_usec / 1000;
}

void vpl_params_default(vpl_params *p) {
    memset(p, 0, sizeof *p);
    p->everyMin = 15;
    p->shuffle = 1;
    p->dim = 1;
    p->dimTo = 0.6;
    p->dimOverMin = 60;
    p->moveMs = 4000;
    p->fadeMs = 700;
    p->ampX = 30;
    p->ampY = 20;
    p->periodX = 540;
    p->periodY = 380;
    p->clock = 1;
    p->twelveHour = 1;
    p->drift = 1;
    p->overlay = 1;
    p->ink = 0xA8A4B0;
    p->inkAlpha = 178;
    p->ttlMs = 30000;
}

static int clampi(int64_t v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : (int)v; }

/* A request's parameters over the defaults (or over the running ones, for an update). Bounds are the player's, whatever the file says. */
static void params_from(vpl_params *p, const char *j, size_t n) {
    p->everyMin = clampi(vj_int(j, n, "everyMin", p->everyMin), 1, 24 * 60);
    p->shuffle = vj_bool(j, n, "shuffle", p->shuffle);
    p->dim = vj_bool(j, n, "dim", p->dim);
    p->dimTo = fmin(1.0, fmax(0.0, vj_num(j, n, "dimTo", p->dimTo)));
    p->dimOverMin = clampi(vj_int(j, n, "dimOverMin", p->dimOverMin), 1, 24 * 60);
    p->moveMs = clampi(vj_int(j, n, "moveMs", p->moveMs), 20, 600000);
    p->fadeMs = clampi(vj_int(j, n, "fadeMs", p->fadeMs), 20, 10000);
    p->ampX = clampi(vj_int(j, n, "ampX", p->ampX), 0, 200);
    p->ampY = clampi(vj_int(j, n, "ampY", p->ampY), 0, 200);
    p->periodX = clampi(vj_int(j, n, "periodX", p->periodX), 1, 86400);
    p->periodY = clampi(vj_int(j, n, "periodY", p->periodY), 1, 86400);
    p->clock = vj_bool(j, n, "clock", p->clock);
    p->twelveHour = vj_bool(j, n, "twelveHour", p->twelveHour);
    p->drift = vj_bool(j, n, "drift", p->drift);
    p->overlay = vj_bool(j, n, "overlay", p->overlay);
    char ink[16];
    if (vj_str(j, n, "ink", ink, sizeof ink) == 7 && ink[0] == '#') p->ink = (uint32_t)strtoul(ink + 1, NULL, 16) & 0xFFFFFF;
    p->inkAlpha = clampi(vj_int(j, n, "inkAlpha", p->inkAlpha), 0, 255);
    int64_t since = vj_int(j, n, "since", 0);
    if (since > 0) p->sinceMs = since;
    p->ttlMs = clampi(vj_int(j, n, "ttlMs", p->ttlMs), 1000, EVENT_TTL_MAX);
    char clip[64];
    if (vj_str(j, n, "clip", clip, sizeof clip) > 0) snprintf(p->clip, sizeof p->clip, "%s", clip);
    p->everyMs = clampi(vj_int(j, n, "everyMs", p->everyMs), 0, 24 * 3600 * 1000);
    char at[8];
    if (vj_str(j, n, "clockAt", at, sizeof at) >= 0) snprintf(p->clockAt, sizeof p->clockAt, "%s", at);
}

/* ---- the player ---------------------------------------------------------------------------------------------------------- */

enum state { ST_IDLE, ST_OPENING, ST_PLAYING, ST_ERROR };
static const char *const STATE_NAME[] = {"idle", "opening", "playing", "error"};

struct options {
    const char *dir, *clips, *ndl_lib, *app_id, *window;
    int export_first, mem_floor_kb, open_timeout_ms;
    const char *once_clip;
    int once_seconds;
};

struct player {
    struct options o;
    enum state st;
    vpl_params prm;
    char reason[200];

    /* clips */
    int nclips;
    char names[VPL_MAX_CLIPS][64];
    vpc_t clip;
    int clip_open, cur;
    vpl_order order;
    vpl_rng rng;
    vpg_t glyphs;
    int have_glyphs;

    /* window and decoder */
    vpl_wl *wl;
    vpl_ndl *ndl;
    int ndl_ready;
    vui_canvas cv;
    int mx, my;                    /* where the picture sits on the screen with no drift */

    /* time */
    int64_t since_wall;            /* wall ms the saver came up */
    int64_t open_ms, playing_ms, ttl_ms, last_status_ms, once_end_ms;
    int64_t next_tick_us, pts_us;
    uint32_t unit;
    uint64_t fed, full, skipped;
    int64_t next_move_ms, next_change_ms, phase_ms, next_veil_ms, next_clock_ms;
    int phase;                     /* 0 steady, 1 fading out, 2 held black, 3 fading in */
    int switched;                  /* the new print has been taken during this hold */
    int veil;                      /* the veil's alpha now */
    int dx, dy;
    char clock_text[16];
    vui_rect clock_rect;
    int ev_loaded, ev_playing, ev_error;
    char events[96];
    int64_t last_seq;
    char window_id[64];
};

static volatile sig_atomic_t g_term;
static void on_term(int sig) { (void)sig; g_term = 1; }
static void on_alarm(int sig) { (void)sig; _exit(3); }   /* a teardown that hangs: the supervisor starts a fresh player */

static void meminfo(long *avail_kb, long *cma_kb) {
    *avail_kb = *cma_kb = -1;
    FILE *f = fopen("/proc/meminfo", "r");
    if (!f) return;
    char line[128];
    while (fgets(line, sizeof line, f)) {
        long v;
        if (sscanf(line, "MemAvailable: %ld", &v) == 1) *avail_kb = v;
        else if (sscanf(line, "CmaFree: %ld", &v) == 1) *cma_kb = v;
    }
    fclose(f);
}

static long rss_kb(void) {
    FILE *f = fopen("/proc/self/statm", "r");
    long a = 0, b = 0;
    if (f) {
        if (fscanf(f, "%ld %ld", &a, &b) != 2) b = 0;
        fclose(f);
    }
    return b * (sysconf(_SC_PAGESIZE) / 1024);
}

static void write_status(struct player *p) {
    char path[512], tmp[560], buf[1400];
    long avail, cma;
    meminfo(&avail, &cma);
    int n = snprintf(buf, sizeof buf,
                     "{\"v\":1,\"pid\":%d,\"ts\":%lld,\"state\":\"%s\",\"reason\":\"%s\",\"seq\":%lld,\"clip\":\"%s\",\"clips\":%d,"
                     "\"window\":\"%s\",\"windowId\":\"%s\",\"playingMs\":%lld,\"fed\":%llu,\"full\":%llu,\"skipped\":%llu,\"queue\":%d,"
                     "\"events\":\"%s\",\"drift\":[%d,%d],\"veil\":%d,\"phase\":%d,\"memAvailKb\":%ld,\"cmaFreeKb\":%ld,\"rssKb\":%ld}\n",
                     (int)getpid(), (long long)vpl_wall_ms(), STATE_NAME[p->st], p->reason, (long long)p->last_seq,
                     p->cur >= 0 && p->cur < p->nclips && p->st >= ST_OPENING && p->st != ST_ERROR ? p->names[p->cur] : "", p->nclips,
                     p->o.window, p->window_id, p->st == ST_PLAYING ? (long long)(vpl_ms() - p->playing_ms) : 0LL,
                     (unsigned long long)p->fed, (unsigned long long)p->full, (unsigned long long)p->skipped, p->ndl_ready && p->ndl ? vpl_ndl_queued(p->ndl) : -1,
                     p->events, p->dx, p->dy, p->veil, p->phase, avail, cma, rss_kb());
    snprintf(path, sizeof path, "%s/status.json", p->o.dir);
    snprintf(tmp, sizeof tmp, "%s.tmp", path);
    int fd = open(tmp, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (fd < 0) return;
    ssize_t w = write(fd, buf, (size_t)n);
    close(fd);
    if (w == n) rename(tmp, path);
    p->last_status_ms = vpl_ms();
}

/* ---- clips --------------------------------------------------------------------------------------------------------------- */

static int by_name(const void *a, const void *b) { return strcmp((const char *)a, (const char *)b); }

static void scan_clips(struct player *p) {
    p->nclips = 0;
    DIR *d = opendir(p->o.clips);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) && p->nclips < VPL_MAX_CLIPS) {
        size_t l = strlen(e->d_name);
        if (l < 5 || l - 4 >= sizeof p->names[0] || strcmp(e->d_name + l - 4, ".vpc") != 0) continue;
        memcpy(p->names[p->nclips], e->d_name, l - 4);
        p->names[p->nclips][l - 4] = 0;
        p->nclips++;
    }
    closedir(d);
    qsort(p->names, (size_t)p->nclips, sizeof p->names[0], by_name);
}

static int open_clip(struct player *p, int idx, char *err, size_t cap) {
    char path[600];
    snprintf(path, sizeof path, "%s/%s.vpc", p->o.clips, p->names[idx]);
    vpc_t c;
    if (vpc_open(&c, path, err, cap) != 0) return -1;
    if (p->clip_open) {
        if (c.codec != p->clip.codec || c.width != p->clip.width || c.height != p->clip.height) {
            vpc_close(&c);
            snprintf(err, cap, "%s is not the size or codec of the clip playing (the pipeline is loaded for one)", p->names[idx]);
            return -1;
        }
        vpc_close(&p->clip);
    }
    p->clip = c;
    p->clip_open = 1;
    p->cur = idx;
    p->unit = 0;
    return 0;
}

/* ---- opening and closing -------------------------------------------------------------------------------------------------- */

static void drop_everything(struct player *p) {
    if (p->wl) {
        vpl_wl_close(p->wl);          /* the window is gone from the screen here */
        p->wl = NULL;
    }
    if (p->ndl_ready && p->ndl) {
        alarm(8);
        vpl_ndl_stop(p->ndl);
        alarm(0);
        p->ndl_ready = 0;
    }
    if (p->clip_open) {
        vpc_close(&p->clip);
        p->clip_open = 0;
    }
    if (p->have_glyphs) {
        vpg_close(&p->glyphs);
        p->have_glyphs = 0;
    }
    memset(&p->cv, 0, sizeof p->cv);
    p->window_id[0] = 0;
    p->phase = 0;
    p->veil = 0;
    p->dx = p->dy = 0;
    p->clock_text[0] = 0;
    p->clock_rect = (vui_rect){0, 0, 0, 0};
}

static void to_idle(struct player *p, const char *why) {
    int was = p->st;
    drop_everything(p);
    p->st = ST_IDLE;
    snprintf(p->reason, sizeof p->reason, "%s", why);
    if (was != ST_IDLE) vpl_say("idle: %s", why);
    write_status(p);
}

static void to_error(struct player *p, const char *why) {
    drop_everything(p);
    p->st = ST_ERROR;
    snprintf(p->reason, sizeof p->reason, "%s", why);
    vpl_say("error: %s", why);
    write_status(p);
}

static int64_t every_ms(const vpl_params *q) { return q->everyMs > 0 ? q->everyMs : (int64_t)q->everyMin * 60000; }

/* The picture's place: the clip's own size, centred on the screen, shifted by the drift. */
static void place_video(struct player *p) {
    if (!p->wl || !p->clip_open) return;
    vpl_wl_video(p->wl, 0, 0, (int)p->clip.width, (int)p->clip.height, p->mx + p->dx, p->my + p->dy, (int)p->clip.width, (int)p->clip.height);
}

static void draw_clock(struct player *p, int force);

static void begin(struct player *p, const char *req, size_t n) {
    char err[300];
    long avail, cma;
    meminfo(&avail, &cma);
    if (avail >= 0 && avail < p->o.mem_floor_kb) {
        snprintf(err, sizeof err, "memory: %ld MB available, the floor is %d MB", avail / 1024, p->o.mem_floor_kb / 1024);
        to_error(p, err);
        return;
    }
    vpl_params_default(&p->prm);
    params_from(&p->prm, req, n);
    scan_clips(p);
    if (p->nclips == 0) {
        to_error(p, "no clips");
        return;
    }
    vpl_order_init(&p->order, p->nclips);
    vpl_rng_seed(&p->rng, (uint32_t)(vpl_wall_ms() ^ ((int64_t)getpid() << 16)));
    int first = -1;
    if (p->prm.clip[0])
        for (int i = 0; i < p->nclips; i++)
            if (!strcmp(p->names[i], p->prm.clip)) first = i;
    if (first < 0) first = vpl_order_next(&p->order, p->prm.shuffle, &p->rng);
    p->order.current = first;
    p->clip_open = 0;
    if (open_clip(p, first, err, sizeof err) != 0) {
        to_error(p, err);
        return;
    }
    char path[600];
    snprintf(path, sizeof path, "%s/clock.vpg", p->o.clips);
    if (p->prm.overlay && vpg_open(&p->glyphs, path, err, sizeof err) == 0) p->have_glyphs = 1;
    else if (p->prm.overlay) vpl_say("no corner clock: %s", err);

    if (!p->ndl && vpl_ndl_open_lib(&p->ndl, p->o.ndl_lib, err, sizeof err) != 0) {
        to_error(p, err);
        return;
    }
    vpl_wl_opts wo = {.app_id = p->o.app_id, .title = "LumaTV saver", .width = SCREEN_W, .height = SCREEN_H, .state = 3,
                      .export_first = p->o.export_first, .timeout_ms = 3000};
    if (!strcmp(p->o.window, "screensaver")) wo.window_type = "_WEBOS_WINDOW_TYPE_SCREENSAVER", wo.play_video = 1;
    else if (!strcmp(p->o.window, "card")) wo.window_type = "_WEBOS_WINDOW_TYPE_CARD";
    else if (!strcmp(p->o.window, "overlay")) wo.window_type = "_WEBOS_WINDOW_TYPE_OVERLAY";
    if (vpl_wl_open(&p->wl, &wo, err, sizeof err) != 0) {
        to_error(p, err);
        return;
    }
    snprintf(p->window_id, sizeof p->window_id, "%s", vpl_wl_window_id(p->wl));
    p->cv = (vui_canvas){vpl_wl_pixels(p->wl), SCREEN_W, SCREEN_H, SCREEN_W};
    p->mx = (SCREEN_W - (int)p->clip.width) / 2;
    p->my = (SCREEN_H - (int)p->clip.height) / 2;
    p->dx = p->dy = 0;
    place_video(p);
    if (vpl_ndl_start(p->ndl, p->o.app_id, p->window_id, p->clip.codec, (int)p->clip.width, (int)p->clip.height, err, sizeof err) != 0) {
        char why[300];
        snprintf(why, sizeof why, "%s", err);
        vpl_ndl_stop(p->ndl);
        to_error(p, why);
        return;
    }
    p->ndl_ready = 1;
    int64_t now = vpl_ms();
    p->st = ST_OPENING;
    p->reason[0] = 0;
    p->ev_loaded = p->ev_playing = p->ev_error = 0;
    p->events[0] = 0;
    p->open_ms = now;
    p->since_wall = p->prm.sinceMs > 0 ? p->prm.sinceMs : vpl_wall_ms();
    p->ttl_ms = now + p->prm.ttlMs;
    p->fed = p->full = p->skipped = 0;
    p->pts_us = 0;
    p->next_tick_us = vpl_us();
    p->next_move_ms = now + p->prm.moveMs;
    p->next_change_ms = now + every_ms(&p->prm);
    p->next_veil_ms = now;
    p->next_clock_ms = now;
    p->phase = 0;
    p->veil = 0;
    p->clock_text[0] = 0;
    vpl_say("opening: window %s as %s, clip %s (%ux%u, %u units), %d clips", p->window_id, p->o.window, p->names[p->cur], p->clip.width, p->clip.height, p->clip.count, p->nclips);
    write_status(p);
}

/* ---- what the window draws ----------------------------------------------------------------------------------------------- */

static double level_now(struct player *p) {
    double minutes = (double)(vpl_wall_ms() - p->since_wall) / 60000.0;
    return vpl_level(minutes, p->prm.dim, p->prm.dimTo, p->prm.dimOverMin);
}

static int veil_now(struct player *p, int64_t now) {
    return vpl_veil(level_now(p), p->phase, p->phase ? now - p->phase_ms : 0, p->prm.fadeMs);
}

static void commit_rect(struct player *p, vui_rect r) {
    r = vui_clip(r, &p->cv);
    if (vui_empty(r)) return;
    vpl_wl_commit(p->wl, r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0);
}

/* The corner clock at its place plus the drift; redrawn when the minute or the drift changed (or `force`). */
static void draw_clock(struct player *p, int force) {
    if (!p->wl || !p->prm.overlay || !p->prm.clock || !p->have_glyphs) return;
    char text[16];
    time_t t = time(NULL);
    struct tm tm;
    localtime_r(&t, &tm);
    if (p->prm.clockAt[0]) snprintf(text, sizeof text, "%s", p->prm.clockAt);
    else vpl_clock_text(text, sizeof text, tm.tm_hour, tm.tm_min, p->prm.twelveHour);
    int right = (int)p->glyphs.right + p->dx, base = (int)p->glyphs.baseline + p->dy;
    vui_rect want = vui_text_rect(&p->glyphs, text, right, base);
    if (!force && strcmp(text, p->clock_text) == 0 && !memcmp(&want, &p->clock_rect, sizeof want)) return;
    vui_rect damage = vui_union(p->clock_rect, want);
    vui_fill(&p->cv, p->clock_rect, (uint8_t)p->veil);
    vui_draw_text(&p->cv, &p->glyphs, text, right, base, p->prm.ink, p->prm.inkAlpha, (uint8_t)p->veil);
    snprintf(p->clock_text, sizeof p->clock_text, "%s", text);
    p->clock_rect = want;
    commit_rect(p, damage);
}

static void set_veil(struct player *p, int alpha) {
    if (alpha == p->veil || !p->wl || !p->prm.overlay) {
        p->veil = alpha;
        return;
    }
    p->veil = alpha;
    vui_fill(&p->cv, (vui_rect){0, 0, SCREEN_W, SCREEN_H}, (uint8_t)alpha);
    p->clock_text[0] = 0;               /* the fill wiped it: draw it again */
    p->clock_rect = (vui_rect){0, 0, 0, 0};
    draw_clock(p, 1);
    commit_rect(p, (vui_rect){0, 0, SCREEN_W, SCREEN_H});
}

/* ---- the main loop's steps ----------------------------------------------------------------------------------------------- */

static void note_event(struct player *p, int type) {
    size_t l = strlen(p->events);
    char add[16];
    int k = snprintf(add, sizeof add, "%s%d", l ? "," : "", type);
    if (l + (size_t)k < sizeof p->events) memcpy(p->events + l, add, (size_t)k + 1);
}

static void drain_events(struct player *p) {
    if (!p->ndl) return;
    vpl_ndl_event_clear(p->ndl);
    int type;
    long long num;
    char text[160];
    while (vpl_ndl_next_event(p->ndl, &type, &num, text, sizeof text)) {
        if (type == NDL_EV_LOAD_COMPLETED) p->ev_loaded = 1;
        else if (type == NDL_EV_PLAYING) p->ev_playing = 1;
        else if (type == NDL_EV_INT_ERROR) p->ev_error = 1;
        if (type != NDL_EV_BUFFER_LOW && type != NDL_EV_STR_BUFFER_LOW) note_event(p, type);
        vpl_say("ndl event %d (%lld) %.100s", type, num, text);
    }
}

/* Feeds the unit that is due; the clip loops from its loop target. Late ticks are fed at most four at a time and a stall resets the schedule. */
static void feed_due(struct player *p) {
    int64_t now = vpl_us();
    int64_t period = 1000000LL * p->clip.fps_den / p->clip.fps_num;
    if (now - p->next_tick_us > 250000) p->next_tick_us = now;
    for (int burst = 0; burst < 4 && now >= p->next_tick_us; burst++) {
        int q = vpl_ndl_queued(p->ndl);
        if (q > QUEUE_LIMIT) {
            p->skipped++;
            p->next_tick_us += period;
            continue;
        }
        uint32_t size;
        int key;
        const uint8_t *u = vpc_unit(&p->clip, p->unit, &size, &key);
        int rc = vpl_ndl_feed(p->ndl, u, size, p->pts_us);
        if (rc != 0) {
            p->full++;
            p->next_tick_us = now + 5000;       /* the pipeline's buffer is full: try again shortly, do not skip the unit */
            break;
        }
        p->fed++;
        p->pts_us += period;
        p->next_tick_us += period;
        if (++p->unit >= p->clip.count) p->unit = p->clip.loop_from;
    }
}

/* The change of print: out through black, the new clip's first unit while it is black, back in. The window is black (and the compositor has been
 * told) for HOLD_MS; the switch is half way through that, so the new picture can only appear under a black window. Every phase change asks for
 * a veil step at once: the veil is otherwise looked at once a second. */
static void to_phase(struct player *p, int phase, int64_t now) {
    p->phase = phase;
    p->phase_ms = now;
    p->next_veil_ms = now;
}

static void switch_print(struct player *p) {
    int next = vpl_order_next(&p->order, p->prm.shuffle, &p->rng);
    char err[300];
    if (next >= 0 && open_clip(p, next, err, sizeof err) == 0) {
        p->order.current = next;
        vpl_say("print: %s", p->names[next]);
    } else {
        vpl_say("the next print was not taken, the same one stays: %s", err);
    }
}

static void step_change(struct player *p, int64_t now) {
    if (p->phase == 0) {
        if (p->nclips > 1 && now >= p->next_change_ms) {
            if (p->prm.overlay) {
                to_phase(p, 1, now);
                p->switched = 0;
            } else {                                              /* no veil to fade with: a cut */
                switch_print(p);
                p->next_change_ms = now + every_ms(&p->prm);
            }
        }
        return;
    }
    int64_t into = now - p->phase_ms;
    if (p->phase == 1 && into >= p->prm.fadeMs) to_phase(p, 2, now);
    else if (p->phase == 2) {
        if (!p->switched && into >= HOLD_MS / 2) {
            switch_print(p);
            p->switched = 1;
        }
        if (into >= HOLD_MS) to_phase(p, 3, now);
    } else if (p->phase == 3 && into >= p->prm.fadeMs) {
        p->phase = 0;
        p->next_change_ms = now + every_ms(&p->prm);
    }
}

static void step_drift(struct player *p, int64_t now) {
    if (now < p->next_move_ms) return;
    p->next_move_ms = now + p->prm.moveMs;
    if (!p->prm.drift) return;
    int ax = p->prm.ampX < p->mx ? p->prm.ampX : p->mx, ay = p->prm.ampY < p->my ? p->prm.ampY : p->my;
    int dx, dy;
    vpl_drift((double)(vpl_wall_ms() - p->since_wall) / 1000.0, ax, ay, p->prm.periodX, p->prm.periodY, &dx, &dy);
    if (dx == p->dx && dy == p->dy) return;
    p->dx = dx;
    p->dy = dy;
    place_video(p);
    draw_clock(p, 0);
}

static void handle_request(struct player *p, const char *req, size_t n) {
    char op[24];
    if (!vj_valid(req, n) || vj_str(req, n, "op", op, sizeof op) <= 0) {
        vpl_say("a request that is not JSON with an op was ignored");
        return;
    }
    int64_t seq = vj_int(req, n, "seq", 0);
    int64_t sent = vj_int(req, n, "sent", 0);
    int ttl = clampi(vj_int(req, n, "ttlMs", 30000), 1000, EVENT_TTL_MAX);
    if (seq == p->last_seq && seq != 0) return;                         /* the same file read again */
    if (seq != 0 && seq < p->last_seq && p->st != ST_IDLE && p->st != ST_ERROR) return;       /* an older request than the one acted on */
    if (sent > 0 && vpl_wall_ms() - sent > ttl && strcmp(op, "stop") != 0) {
        vpl_say("a request of %lld s ago (ttl %d s) was ignored", (long long)((vpl_wall_ms() - sent) / 1000), ttl / 1000);
        return;
    }
    p->last_seq = seq;
    if (!strcmp(op, "start")) {
        if (p->st == ST_PLAYING || p->st == ST_OPENING) {
            params_from(&p->prm, req, n);
            p->ttl_ms = vpl_ms() + p->prm.ttlMs;
        } else begin(p, req, n);
    } else if (!strcmp(op, "update")) {
        if (p->st == ST_PLAYING || p->st == ST_OPENING) {
            params_from(&p->prm, req, n);
            p->ttl_ms = vpl_ms() + p->prm.ttlMs;
            draw_clock(p, 1);
        }
    } else if (!strcmp(op, "renew")) {
        if (p->st == ST_PLAYING || p->st == ST_OPENING) p->ttl_ms = vpl_ms() + ttl;
    } else if (!strcmp(op, "status")) {
        write_status(p);                                      /* a fresh reading (the tests, and the TV steps, ask for one) */
    } else if (!strcmp(op, "next")) {
        if (p->st == ST_PLAYING) p->next_change_ms = 0;
    } else if (!strcmp(op, "stop")) {
        if (p->st != ST_IDLE) to_idle(p, "stopped");
        else write_status(p);
    } else {
        vpl_say("unknown op %s", op);
    }
}

static void read_request(struct player *p) {
    char path[560], buf[4096];
    snprintf(path, sizeof path, "%s/req.json", p->o.dir);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return;
    ssize_t n = read(fd, buf, sizeof buf - 1);
    close(fd);
    if (n > 0) handle_request(p, buf, (size_t)n);
}

/* ---- the loop ------------------------------------------------------------------------------------------------------------ */

static void prepare(struct player *p) {
    mkdir(p->o.dir, 01777);
    chmod(p->o.dir, 01777);
    p->st = ST_IDLE;
    p->cur = -1;
    scan_clips(p);
}

/* The monotonic time (in microseconds) the loop must wake at, whatever it is waiting for. */
static int64_t next_wake_us(struct player *p, int64_t now_ms) {
    int64_t wake = (now_ms + (p->st == ST_IDLE || p->st == ST_ERROR ? STATUS_IDLE_MS : STATUS_PLAYING_MS)) * 1000;
#define SOONER(t_us) do { int64_t t_ = (t_us); if (t_ < wake) wake = t_; } while (0)
    if (p->st == ST_OPENING || p->st == ST_PLAYING) {
        SOONER(p->ttl_ms * 1000);
        SOONER(p->next_tick_us);
        SOONER(p->next_move_ms * 1000);
        SOONER(p->next_veil_ms * 1000);
        SOONER(p->next_clock_ms * 1000);
        if (!p->phase) SOONER(p->next_change_ms * 1000);
        if (p->st == ST_OPENING) SOONER((p->open_ms + p->o.open_timeout_ms) * 1000);
        if (p->once_end_ms) SOONER(p->once_end_ms * 1000);
    } else if (p->st == ST_ERROR) {
        SOONER((p->last_status_ms + 30000) * 1000);
    }
#undef SOONER
    return wake;
}

static int run(struct player *p) {
    int ino = inotify_init1(IN_CLOEXEC | IN_NONBLOCK);
    if (ino < 0 || inotify_add_watch(ino, p->o.dir, IN_CLOSE_WRITE | IN_MOVED_TO) < 0) {
        vpl_say("cannot watch %s: %s", p->o.dir, strerror(errno));
        return 1;
    }
    if (!p->once_end_ms) {
        vpl_say("ready: %d clips in %s, mailbox %s, window %s", p->nclips, p->o.clips, p->o.dir, p->o.window);
        write_status(p);
        read_request(p);          /* a request written while the player was starting (a stale one is ignored by its age) */
    }
    for (;;) {
        if (g_term) break;
        int64_t now = vpl_ms();
        struct pollfd fds[3];
        int nf = 0;
        fds[nf++] = (struct pollfd){ino, POLLIN, 0};
        int wl_i = -1;
        if (p->wl) {
            wl_i = nf;
            fds[nf++] = (struct pollfd){vpl_wl_fd(p->wl), POLLIN, 0};
        }
        if (p->ndl_ready) fds[nf++] = (struct pollfd){vpl_ndl_event_fd(p->ndl), POLLIN, 0};
        int64_t left_us = next_wake_us(p, now) - vpl_us();
        if (left_us < 0) left_us = 0;
        struct timespec to = {(time_t)(left_us / 1000000), (long)(left_us % 1000000) * 1000};
        int r = ppoll(fds, (nfds_t)nf, &to, NULL);
        if (r < 0 && errno != EINTR) break;
        now = vpl_ms();

        if (r > 0 && (fds[0].revents & POLLIN)) {
            char evbuf[1024];
            ssize_t got;
            int touched = 0;
            while ((got = read(ino, evbuf, sizeof evbuf)) > 0) {
                for (char *q = evbuf; q < evbuf + got;) {
                    struct inotify_event *e = (struct inotify_event *)q;
                    if (e->len && !strcmp(e->name, "req.json")) touched = 1;
                    q += sizeof *e + e->len;
                }
            }
            if (touched && !p->once_end_ms) read_request(p);
        }
        if (p->wl && wl_i >= 0 && r > 0 && (fds[wl_i].revents & (POLLIN | POLLHUP | POLLERR))) {
            char err[300];
            if (vpl_wl_dispatch(p->wl, err, sizeof err) != 0) {
                to_idle(p, err);
                if (p->once_end_ms) break;
                continue;
            }
        }
        if (p->ndl_ready) {
            drain_events(p);
            if (p->ev_error && (p->st == ST_OPENING || p->st == ST_PLAYING)) {
                to_error(p, "the pipeline reported an error");
                if (p->once_end_ms) break;
                continue;
            }
        }
        if (p->st == ST_OPENING || p->st == ST_PLAYING) {
            feed_due(p);
            if (p->st == ST_OPENING && p->ev_playing) {
                p->st = ST_PLAYING;
                p->playing_ms = now;
                p->reason[0] = 0;
                vpl_say("playing: %s", p->names[p->cur]);
                draw_clock(p, 1);
                write_status(p);
            } else if (p->st == ST_OPENING && now - p->open_ms > p->o.open_timeout_ms) {
                to_error(p, p->ev_loaded ? "no playing event after the load" : "the pipeline did not load");
                if (p->once_end_ms) break;
                continue;
            }
            if (p->st == ST_PLAYING) {
                step_change(p, now);
                if (now >= p->next_veil_ms) {
                    int a = veil_now(p, now);
                    if (a != p->veil) set_veil(p, a);
                    p->next_veil_ms = now + (p->phase ? VEIL_STEP_MS : 1000);
                }
                step_drift(p, now);
                if (now >= p->next_clock_ms) {
                    draw_clock(p, 0);
                    p->next_clock_ms = now + 60000 - (vpl_wall_ms() % 60000) + 20;
                }
            }
            if (now >= p->ttl_ms) {
                to_idle(p, "ttl: no request renewed it");
                if (p->once_end_ms) break;
                continue;
            }
            if (p->once_end_ms && now >= p->once_end_ms) {
                to_idle(p, "the test time is over");
                break;
            }
        }
        if (now - p->last_status_ms >= (p->st == ST_IDLE ? STATUS_IDLE_MS : p->st == ST_ERROR ? 30000 : STATUS_PLAYING_MS)) {
            if (p->st == ST_ERROR) p->st = ST_IDLE;               /* the error has been on offer long enough */
            write_status(p);
        }
    }
    if (p->st != ST_IDLE) to_idle(p, "the player was stopped");
    return 0;
}

/* ---- probe, tests, entry ------------------------------------------------------------------------------------------------- */

static int probe(struct player *p) {
    char err[300] = "", nerr[300] = "";
    vpl_ndl *n = NULL;
    int ndl = vpl_ndl_open_lib(&n, p->o.ndl_lib, nerr, sizeof nerr) == 0;
    scan_clips(p);
    int ok = 0;
    char first[600] = "";
    for (int i = 0; i < p->nclips; i++) {
        vpc_t c;
        char path[600];
        snprintf(path, sizeof path, "%s/%s.vpc", p->o.clips, p->names[i]);
        if (vpc_open(&c, path, err, sizeof err) == 0) {
            ok++;
            vpc_close(&c);
        } else if (!first[0]) snprintf(first, sizeof first, "%s", err);
    }
    long avail, cma;
    meminfo(&avail, &cma);
    printf("{\"ndl\":%s,\"ndlError\":\"%s\",\"clips\":%d,\"clipsOk\":%d,\"clipError\":\"%s\",\"memAvailKb\":%ld,\"cmaFreeKb\":%ld}\n", ndl ? "true" : "false", nerr, p->nclips, ok, first, avail, cma);
    if (n) vpl_ndl_free(n);
    return ndl && ok > 0 && ok == p->nclips ? 0 : 2;
}

static int vpc_info(const char *path) {
    vpc_t c;
    char err[300];
    if (vpc_open(&c, path, err, sizeof err) != 0) {
        printf("{\"error\":\"%s\"}\n", err);
        return 2;
    }
    uint32_t size;
    int key;
    vpc_unit(&c, 0, &size, &key);
    printf("{\"codec\":%d,\"width\":%u,\"height\":%u,\"fpsNum\":%u,\"fpsDen\":%u,\"units\":%u,\"loopFrom\":%u,\"keyBytes\":%u,\"bytes\":%zu}\n", c.codec, c.width, c.height, c.fps_num, c.fps_den, c.count, c.loop_from, size, c.map_len);
    vpc_close(&c);
    return 0;
}

static int calc(int argc, char **argv) {
    if (argc >= 2 && !strcmp(argv[0], "drift")) {     /* drift T ampX ampY periodX periodY */
        int dx, dy;
        vpl_drift(atof(argv[1]), argc > 2 ? atoi(argv[2]) : 30, argc > 3 ? atoi(argv[3]) : 20, argc > 4 ? atoi(argv[4]) : 540, argc > 5 ? atoi(argv[5]) : 380, &dx, &dy);
        printf("%d %d\n", dx, dy);
    } else if (argc >= 2 && !strcmp(argv[0], "level")) {   /* level MINUTES dim dimTo dimOverMin */
        printf("%.6f\n", vpl_level(atof(argv[1]), argc > 2 ? atoi(argv[2]) : 1, argc > 3 ? atof(argv[3]) : 0.6, argc > 4 ? atoi(argv[4]) : 60));
    } else if (argc >= 4 && !strcmp(argv[0], "veil")) {     /* veil LEVEL PHASE INTO_MS FADE_MS */
        printf("%d\n", vpl_veil(atof(argv[1]), atoi(argv[2]), atoll(argv[3]), argc > 4 ? atoi(argv[4]) : 700));
    } else if (argc >= 2 && !strcmp(argv[0], "ease")) {
        printf("%.6f\n", vpl_ease(atof(argv[1])));
    } else if (argc >= 5 && !strcmp(argv[0], "order")) {    /* order N SHUFFLE SEED COUNT */
        vpl_order o;
        vpl_rng r;
        vpl_order_init(&o, atoi(argv[1]));
        vpl_rng_seed(&r, (uint32_t)strtoul(argv[3], NULL, 10));
        for (int i = 0, k = atoi(argv[4]); i < k; i++) {
            int v = vpl_order_next(&o, atoi(argv[2]), &r);
            o.current = v;
            printf("%d%c", v, i + 1 < k ? ' ' : '\n');
        }
    } else if (argc >= 4 && !strcmp(argv[0], "clock")) {    /* clock HOUR MINUTE TWELVE */
        char t[16];
        vpl_clock_text(t, sizeof t, atoi(argv[1]), atoi(argv[2]), atoi(argv[3]));
        printf("%s\n", t);
    } else {
        fprintf(stderr, "usage: --calc drift T [ax ay px py] | level MIN [dim to over] | veil LEVEL PHASE INTO FADE | ease T | order N SHUFFLE SEED COUNT | clock H M TWELVE\n");
        return 2;
    }
    return 0;
}

int main(int argc, char **argv) {
    static struct player p;
    p.o = (struct options){.dir = getenv("LUMATV_VPLANE_DIR") ? getenv("LUMATV_VPLANE_DIR") : "/tmp/lumatv-vplane",
                           .clips = getenv("LUMATV_VPLANE_CLIPS") ? getenv("LUMATV_VPLANE_CLIPS") : "/var/lib/lumatv/vplane/clips",
                           .app_id = "org.nphil.lumatv.vplane", .window = "screensaver", .export_first = 1, .mem_floor_kb = 100 * 1024, .open_timeout_ms = 8000};
    int do_probe = 0;
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        if (!strcmp(a, "-d") && i + 1 < argc) p.o.dir = argv[++i];
        else if (!strcmp(a, "-c") && i + 1 < argc) p.o.clips = argv[++i];
        else if (!strcmp(a, "-L") && i + 1 < argc) p.o.ndl_lib = argv[++i];
        else if (!strcmp(a, "-a") && i + 1 < argc) p.o.app_id = argv[++i];
        else if (!strcmp(a, "-w") && i + 1 < argc) p.o.window = argv[++i];
        else if (!strcmp(a, "-m") && i + 1 < argc) p.o.mem_floor_kb = atoi(argv[++i]);
        else if (!strcmp(a, "-e") && i + 1 < argc) p.o.export_first = !strcmp(argv[++i], "first");
        else if (!strcmp(a, "-o") && i + 1 < argc) p.o.open_timeout_ms = atoi(argv[++i]);
        else if (!strcmp(a, "-n")) do_probe = 1;
        else if (!strcmp(a, "--vpc-info") && i + 1 < argc) return vpc_info(argv[i + 1]);
        else if (!strcmp(a, "--calc")) return calc(argc - i - 1, argv + i + 1);
        else if (!strcmp(a, "--once") && i + 2 < argc) {
            p.o.once_clip = argv[i + 1];
            p.o.once_seconds = atoi(argv[i + 2]);
            i += 2;
        } else {
            fprintf(stderr, "usage: lumatv-vplane [-d MAILBOX] [-c CLIPS] [-L NDL_LIB] [-a APP_ID] [-w screensaver|card|overlay|default] [-m MEMFLOOR_KB] [-e first|after]\n"
                            "       lumatv-vplane -n | --once CLIP SECONDS | --vpc-info FILE | --calc WHAT ARGS...\n");
            return 2;
        }
    }
    if (strcmp(p.o.window, "screensaver") && strcmp(p.o.window, "card") && strcmp(p.o.window, "overlay") && strcmp(p.o.window, "default")) {
        fprintf(stderr, "lumatv-vplane: -w takes screensaver, card, overlay or default\n");
        return 2;
    }
    if (do_probe) return probe(&p);
    signal(SIGPIPE, SIG_IGN);
    struct sigaction sa = {.sa_handler = on_term};
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    struct sigaction al = {.sa_handler = on_alarm};
    sigaction(SIGALRM, &al, NULL);
    if (p.o.once_clip) {
        mkdir(p.o.dir, 01777);
        char req[512];
        snprintf(req, sizeof req, "{\"seq\":1,\"op\":\"start\",\"clip\":\"%s\",\"ttlMs\":%d}", p.o.once_clip, (p.o.once_seconds + 30) * 1000);
        p.o.once_seconds = p.o.once_seconds > 0 ? p.o.once_seconds : 10;
        prepare(&p);
        begin(&p, req, strlen(req));
        if (p.st == ST_ERROR) {
            fprintf(stderr, "lumatv-vplane: %s\n", p.reason);
            return 2;
        }
        p.once_end_ms = vpl_ms() + p.o.once_seconds * 1000;
        int rc = run(&p);
        return rc;
    }
    prepare(&p);
    return run(&p);
}
