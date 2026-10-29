/* lumatv-gfn-feeder: the 32-bit process that puts the GeForce NOW stream on the TV's video plane through LG's NDL DirectMedia library.
 *
 * Two jobs, one binary (docs/research/gfn-app-result.md):
 *   --play FILE     the decode probe: an Annex B (H.264, H.265) or OBU (AV1) file fed at its frame rate with a monotonic PTS, the render
 *                   queue, waits, events and memory written as one JSON line a second on stdout, a summary at the end. Phase 1's E1/E2.
 *   --serve SOCKET  the engine's sink: the 64-bit stream engine connects on a Unix socket and sends LOAD, VIDEO, AUDIO, HDR, FLUSH, STATE
 *                   (gf_proto.h); the feeder answers HELLO, LOADED, ACCEPT/DROP per unit, EVENT, STATUS.
 * The window the video shows through is either this process's own (a bare Wayland client of LG's compositor, vplane/src/vpl_wl.c) or one
 * the app's UI process exported (`--window-id`), which is how the installed app runs: the UI owns the card window, this process owns the
 * pipeline. Whether the pipeline accepts an id exported by another process is a TV probe (the result note lists it).
 *
 *   lumatv-gfn-feeder --probe [-L LIB]
 *   lumatv-gfn-feeder --play FILE [--codec h264|h265|av1] [--size WxH] [--fps N] [--seconds S] [--loop] [--hdr] [--audio PKTS]
 *                     [--audio-mode auto|opus|pcm]
 *                     [--drop-threshold N] [--pts-unit ns|us|ms] [--full-wait-ms N] [--burst] [--no-ndl] [-w TYPE] [--window-id ID] [-a APPID]
 *   lumatv-gfn-feeder --serve SOCKET [--once] [-w TYPE] [--window-id ID] [-a APPID] [-L LIB]
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/prctl.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "gf_annexb.h"
#include "gf_log.h"
#include "gf_ndl.h"
#include "gf_proto.h"
#include "gf_sam.h"
#include "gf_opus.h"
#include "vpl_wl.h"

#define SCREEN_W 1920
#define SCREEN_H 1080
#define VIDEO_WINDOW 16 /* a video window (-w video) draws nothing: the compositor puts its video at the rectangle it granted */
#define SAMPLES 4096
#define DEFAULT_APP_ID "org.nphil.lumatv.gfn"
#define DEFAULT_SOCKET "/tmp/lumatv-gfn/feed.sock"
#define DEFAULT_WINDOW_ID_FILE "/tmp/lumatv-gfn/window-id"
#define DEFAULT_LOG "/tmp/lumatv-gfn/feeder.log"

/* --audio-mode: Opus goes to NDL as it is and, if the library refuses that load, is decoded here (auto); only the first; only the second. */
enum { AUDIO_AUTO = 0, AUDIO_OPUS, AUDIO_PCM };

/* The app id for NDL_DirectMediaInit: SAM passes no environment, so an installed app reads it off its own directory
 * (/media/developer/apps/usr/palm/applications/<id>/lumatv-gfn-feeder), as jellyfin-webos-native's ndlplay does. */
static const char *app_id_from_exe(char *buf, size_t cap) {
    char exe[512];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (n > 0) {
        exe[n] = 0;
        char *slash = strrchr(exe, '/');
        if (slash) {
            *slash = 0;
            const char *dir = strrchr(exe, '/');
            dir = dir ? dir + 1 : exe;
            if (strchr(dir, '.') && strlen(dir) < cap) {
                snprintf(buf, cap, "%s", dir);
                return buf;
            }
        }
    }
    return DEFAULT_APP_ID;
}

static volatile sig_atomic_t g_term;
static void on_term(int sig) { (void)sig; g_term = 1; }
static void on_alarm(int sig) { (void)sig; _exit(3); }

struct second {
    uint64_t fed, dropped, afed, adropped, retries;
    int q[SAMPLES], nq;
    int w[SAMPLES], nw;
    int low, full, errors;
    int64_t start_ns;
};

struct feeder {
    /* options */
    const char *lib, *app_id, *window_type, *window_id_opt, *window_id_file, *play_path, *audio_path, *sock_path;
    const char *ui_home;        /* the installed app's directory: the 64-bit UI is started from it and owns the window */
    pid_t ui_pid;
    /* SAM's side (gf_sam.c): started when SAM launched the app (no arguments, or its launch object) or with --sam. `chan` is this end of
     * the socket pair whose other end is the UI's descriptor 3 (rustui/app/src/lifecycle.rs): SAM's lines go down it, the UI's come back. */
    int sam_mode, preload, chan;
    gf_sam *sam;
    int64_t close_at;           /* SAM asked the app to end: when the feeder ends it if the UI has not */
    char chan_in[1024];
    size_t chan_len;
    const char *stats_path;
    int codec, width, height, fps, seconds, loop, hdr, drop_threshold, pts_unit, full_wait_ms, burst, no_ndl, no_window, once, export_first;
    /* state */
    gf_ndl *ndl;
    int ndl_ready, loaded, playing, error;
    vpl_wl *wl;
    char window_id[40];
    FILE *stats;
    struct second sec;
    uint64_t total_fed, total_dropped, total_afed, total_adropped, total_retries;
    int64_t first_loaded_ns, first_playing_ns, t0_ns;
    unsigned ev_count[64];
    char last_video_info[200];
    /* audio: Opus packets pass to NDL as they are (audio_mode auto or opus). When NDL refuses that load, or --audio-mode pcm asks, they
     * are decoded here (gf_opus.c) and fed as PCM; pcm_sticky keeps a refusal in mind for the loads after it. */
    int audio_mode, pcm, pcm_sticky, audio_streak, audio_channels;
    const char *audio_route;    /* none, opus, pcm (decoded here), pcm-in (the engine sent PCM) */
    gf_opus *opus;
    uint64_t decode_errors;
    int16_t pcm_buf[GF_OPUS_MAX_FRAMES * 2];
};

/* ---- stats ------------------------------------------------------------------------------------------------------------------ */

static int cmp_int(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

static int pct(int *v, int n, int p) {
    if (n <= 0) return 0;
    qsort(v, (size_t)n, sizeof *v, cmp_int);
    int i = (n * p + 99) / 100 - 1;
    return v[i < 0 ? 0 : i >= n ? n - 1 : i];
}

static void sample_q(struct second *s, int q) {
    if (q >= 0 && s->nq < SAMPLES) s->q[s->nq++] = q;
}
static void sample_w(struct second *s, int us) {
    if (s->nw < SAMPLES) s->w[s->nw++] = us;
}

static void fill_status(struct feeder *f, gf_status *st) {
    struct second *s = &f->sec;
    memset(st, 0, sizeof *st);
    st->video_fed = f->total_fed;
    st->video_dropped = f->total_dropped;
    st->audio_fed = f->total_afed;
    st->audio_dropped = f->total_adropped;
    st->full_retries = f->total_retries;
    if (s->nq) {
        long sum = 0;
        int mn = s->q[0], mx = s->q[0];
        for (int i = 0; i < s->nq; i++) {
            sum += s->q[i];
            if (s->q[i] < mn) mn = s->q[i];
            if (s->q[i] > mx) mx = s->q[i];
        }
        st->queue_min = (uint32_t)mn;
        st->queue_max = (uint32_t)mx;
        st->queue_mean_x100 = (uint32_t)(sum * 100 / s->nq);
        st->queue_p99 = (uint32_t)pct(s->q, s->nq, 99);
    }
    if (s->nw) {
        int mx = 0;
        for (int i = 0; i < s->nw; i++) if (s->w[i] > mx) mx = s->w[i];
        st->wait_max_us = (uint32_t)mx;
        st->wait_p99_us = (uint32_t)pct(s->w, s->nw, 99);
    }
    st->buffer_low = (uint32_t)s->low;
    st->buffer_full = (uint32_t)s->full;
    st->errors = (uint32_t)s->errors;
    st->loaded = (uint32_t)f->loaded;
    st->playing = (uint32_t)f->playing;
    long cma, avail, rss;
    gf_meminfo(&cma, &avail, &rss);
    st->cma_free_kb = (uint32_t)cma;
    st->mem_available_kb = (uint32_t)avail;
    st->rss_kb = (uint32_t)rss;
    int aa = f->ndl_ready ? gf_ndl_audio_avail(f->ndl) : -1;
    st->audio_avail = aa < 0 ? 0 : (uint32_t)aa;
}

static void write_second(struct feeder *f, int t) {
    gf_status st;
    fill_status(f, &st);
    struct second *s = &f->sec;
    fprintf(f->stats,
            "{\"t\":%d,\"fed\":%llu,\"dropped\":%llu,\"afed\":%llu,\"adropped\":%llu,\"retries\":%llu,"
            "\"queue\":{\"min\":%u,\"max\":%u,\"mean\":%.2f,\"p99\":%u,\"samples\":%d},\"wait_max_us\":%u,\"wait_p99_us\":%u,"
            "\"buffer_low\":%u,\"buffer_full\":%u,\"errors\":%u,\"loaded\":%u,\"playing\":%u,"
            "\"cma_free_kb\":%u,\"mem_avail_kb\":%u,\"rss_kb\":%u,\"audio_avail\":%u}\n",
            t, (unsigned long long)s->fed, (unsigned long long)s->dropped, (unsigned long long)s->afed, (unsigned long long)s->adropped,
            (unsigned long long)s->retries, st.queue_min, st.queue_max, st.queue_mean_x100 / 100.0, st.queue_p99, s->nq, st.wait_max_us,
            st.wait_p99_us, st.buffer_low, st.buffer_full, st.errors, st.loaded, st.playing, st.cma_free_kb, st.mem_available_kb, st.rss_kb,
            st.audio_avail);
    fflush(f->stats);
    memset(s, 0, sizeof *s);
    s->start_ns = gf_ns();
}

/* ---- NDL events ------------------------------------------------------------------------------------------------------------ */

static void drain_events(struct feeder *f, void (*emit)(struct feeder *, int, long long, const char *)) {
    gf_ndl_event_clear(f->ndl);
    int type;
    long long num;
    char text[200];
    while (gf_ndl_next_event(f->ndl, &type, &num, text, sizeof text)) {
        if (type >= 0 && type < 64) f->ev_count[type]++;
        switch (type) {
        case NDL_EV_LOAD_COMPLETED:
            f->loaded = 1;
            if (!f->first_loaded_ns) f->first_loaded_ns = gf_ns();
            break;
        case NDL_EV_PLAYING:
            f->playing = 1;
            if (!f->first_playing_ns) f->first_playing_ns = gf_ns();
            break;
        case NDL_EV_INT_ERROR:
            f->error = 1;
            f->sec.errors++;
            break;
        case NDL_EV_INT_BUFFER_LOW:
        case NDL_EV_STR_BUFFER_LOW: f->sec.low++; break;
        case NDL_EV_STR_BUFFER_FULL: f->sec.full++; break;
        case NDL_EV_STR_VIDEO_INFO: {
            size_t o = 0;        /* the text is quoted into the summary's JSON: escape what would end the string */
            for (const char *s = text; *s && o + 3 < sizeof f->last_video_info; s++) {
                if (*s == '"' || *s == '\\') f->last_video_info[o++] = '\\';
                f->last_video_info[o++] = (*s >= 32) ? *s : ' ';
            }
            f->last_video_info[o] = 0;
            break;
        }
        default: break;
        }
        if (type != NDL_EV_INT_BUFFER_LOW && type != NDL_EV_STR_BUFFER_LOW && type != NDL_EV_STR_BUFFER_FULL)
            gf_say("event %d %lld %s", type, num, text);
        if (emit) emit(f, type, num, text);
    }
}

/* ---- the feed ----------------------------------------------------------------------------------------------------------------- */

/* 0 fed, 1 dropped after waiting on a full buffer, -1 not loaded. `wait_us` says how long it took. */
static int feed_video(struct feeder *f, const uint8_t *u, size_t n, int64_t pts_ns, int *queue, int *wait_us) {
    *queue = -1;
    *wait_us = 0;
    if (f->no_ndl) {
        f->sec.fed++;
        f->total_fed++;
        return 0;
    }
    if (!f->ndl_ready) return -1;
    int64_t t0 = gf_ns();
    *queue = gf_ndl_queued(f->ndl);
    sample_q(&f->sec, *queue);
    long long pts = pts_ns / f->pts_unit;
    int rc, waited = 0;
    for (;;) {
        rc = gf_ndl_feed_video(f->ndl, u, (unsigned)n, pts);
        if (rc == 0) break;
        if (gf_ns() - t0 >= (int64_t)f->full_wait_ms * 1000000) break;
        waited = 1;
        usleep(500);
    }
    *wait_us = (int)((gf_ns() - t0) / 1000);
    sample_w(&f->sec, *wait_us);
    if (waited) {
        f->sec.retries++;
        f->total_retries++;
    }
    if (rc == 0) {
        f->sec.fed++;
        f->total_fed++;
        return 0;
    }
    f->sec.dropped++;
    f->total_dropped++;
    return 1;
}

static int feed_audio(struct feeder *f, const uint8_t *d, size_t n, int64_t pts_ns) {
    if (f->no_ndl) {
        f->sec.afed++;
        f->total_afed++;
        return 0;
    }
    if (!f->ndl_ready) return -1;
    const void *data = d;
    size_t size = n;
    if (f->pcm) {
        int frames = f->opus ? gf_opus_decode(f->opus, d, n, f->pcm_buf, GF_OPUS_MAX_FRAMES) : -1;
        if (frames <= 0) {
            if (!f->decode_errors++) gf_say("opus: a %zu byte packet did not decode (%d); further ones are only counted", n, frames);
            f->sec.adropped++;
            f->total_adropped++;
            return 1;
        }
        data = f->pcm_buf;
        size = (size_t)frames * (size_t)f->audio_channels * sizeof(int16_t);
    }
    int rc = gf_ndl_feed_audio(f->ndl, data, (unsigned)size, pts_ns / f->pts_unit);
    if (rc == 0) {
        f->audio_streak = 0;
        f->sec.afed++;
        f->total_afed++;
        return 0;
    }
    if (++f->audio_streak == 100) gf_say("ndl: 100 audio units in a row were refused (%s audio); the pipeline is not taking them", f->audio_route);
    f->sec.adropped++;
    f->total_adropped++;
    return 1;
}

/* ---- window and pipeline ------------------------------------------------------------------------------------------------------ */

/* The window id the UI process exported (bin/gfn.rs writes it once the compositor assigned it). Read at every load, so a UI that was
 * restarted is followed. */
static int read_window_id_file(struct feeder *f) {
    if (!f->window_id_file) return -1;
    FILE *fp = fopen(f->window_id_file, "r");
    if (!fp) return -1;
    char line[64] = "";
    if (!fgets(line, sizeof line, fp)) line[0] = 0;
    fclose(fp);
    line[strcspn(line, "\r\n")] = 0;
    if (strncmp(line, "_Window_Id_", 11) != 0) return -1;
    snprintf(f->window_id, sizeof f->window_id, "%s", line);
    return 0;
}

static int open_window(struct feeder *f, char *err, size_t cap) {
    if (f->window_id_opt) {
        snprintf(f->window_id, sizeof f->window_id, "%s", f->window_id_opt);
        return 0;
    }
    if (f->window_id_file) {
        if (read_window_id_file(f) != 0) snprintf(f->window_id, sizeof f->window_id, "_Window_Id_pending");
        return 0;
    }
    if (f->no_window) {
        snprintf(f->window_id, sizeof f->window_id, "_Window_Id_none");
        return 0;
    }
    vpl_wl_opts wo = {.app_id = f->app_id, .title = "GeForce NOW", .width = SCREEN_W, .height = SCREEN_H, .state = 3,
                      .export_first = f->export_first, .timeout_ms = 3000};
    if (!strcmp(f->window_type, "card")) wo.window_type = "_WEBOS_WINDOW_TYPE_CARD";
    else if (!strcmp(f->window_type, "overlay")) wo.window_type = "_WEBOS_WINDOW_TYPE_OVERLAY";
    else if (!strcmp(f->window_type, "screensaver")) wo.window_type = "_WEBOS_WINDOW_TYPE_SCREENSAVER", wo.play_video = 1;
    else if (!strcmp(f->window_type, "video")) {
        /* LumaTV's own video window (the compositor's `vwin`): it draws the hole and the placement only for a granted claim, at the claim's rectangle */
        wo.window_type = "_WEBOS_WINDOW_TYPE_LUMATV_VIDEO";
        wo.title = "LumaTV video";
        wo.width = wo.height = VIDEO_WINDOW;
    }
    if (vpl_wl_open(&f->wl, &wo, err, cap) != 0) return -1;
    snprintf(f->window_id, sizeof f->window_id, "%s", vpl_wl_window_id(f->wl));
    return 0;
}

static void place_video(struct feeder *f, int w, int h) {
    if (f->wl) vpl_wl_video(f->wl, 0, 0, w, h, 0, 0, SCREEN_W, SCREEN_H);
}

static int open_decoder(struct feeder *f, const gf_load *l, char *err, size_t cap) {
    if (f->opus) return 0;
    f->opus = gf_opus_open(l->sample_rate, l->channels, err, cap);
    if (!f->opus) return -1;
    f->audio_channels = (int)l->channels;
    return 0;
}

static void close_decoder(struct feeder *f) {
    gf_opus_close(f->opus);
    f->opus = NULL;
    f->pcm = 0;
}

static int start_pipeline(struct feeder *f, const gf_load *l, char *err, size_t cap) {
    if (f->window_id_file && read_window_id_file(f) != 0) {
        snprintf(err, cap, "no window id in %s yet (the UI has not exported its video element)", f->window_id_file);
        return -1;
    }
    f->audio_route = l->audio == GF_AUDIO_OPUS ? "opus" : l->audio == GF_AUDIO_PCM_S16LE ? "pcm-in" : "none";
    if (f->no_ndl) {
        f->loaded = f->playing = 1;
        return 0;
    }
    if (!f->ndl && gf_ndl_open_lib(&f->ndl, f->lib, err, cap) != 0) return -1;
    place_video(f, (int)l->width, (int)l->height);
    int pcm = l->audio == GF_AUDIO_OPUS && (f->audio_mode == AUDIO_PCM || (f->audio_mode == AUDIO_AUTO && f->pcm_sticky));
    char refused[300] = "";
    for (;;) {
        gf_load use = *l;
        if (pcm) {
            if (open_decoder(f, l, err, cap) != 0) return -1;
            use.audio = GF_AUDIO_PCM_S16LE;
        }
        int rc = gf_ndl_start(f->ndl, f->app_id, f->window_id, &use, err, cap);
        if (rc == 0) break;
        gf_ndl_stop(f->ndl);
        if (rc != GF_NDL_LOAD_FAILED || pcm || l->audio != GF_AUDIO_OPUS || f->audio_mode != AUDIO_AUTO) {
            close_decoder(f);
            if (refused[0]) {
                char second[300];
                snprintf(second, sizeof second, "%s", err);
                snprintf(err, cap, "%s; PCM instead: %s", refused, second);
            }
            return -1;
        }
        snprintf(refused, sizeof refused, "%s", err);
        gf_say("ndl: %s: the library refused the Opus load; decoding Opus here and loading PCM", err);
        pcm = 1;
        f->pcm_sticky = 1;
    }
    if (pcm) {
        f->pcm = 1;
        f->audio_route = "pcm";
    }
    f->audio_streak = 0;
    f->ndl_ready = 1;
    f->loaded = f->playing = f->error = 0;
    f->first_loaded_ns = f->first_playing_ns = 0;
    return 0;
}

static void stop_pipeline(struct feeder *f) {
    if (f->ndl_ready) {
        alarm(8);
        gf_ndl_stop(f->ndl);
        alarm(0);
    }
    close_decoder(f);
    f->ndl_ready = 0;
    f->loaded = f->playing = 0;
}

static void close_all(struct feeder *f) {
    stop_pipeline(f);
    if (f->wl) {
        vpl_wl_close(f->wl);
        f->wl = NULL;
    }
    if (f->ndl) {
        gf_ndl_free(f->ndl);
        f->ndl = NULL;
    }
}

static int dispatch_window(struct feeder *f) {
    char err[300];
    if (!f->wl) return 0;
    int rc = vpl_wl_dispatch(f->wl, err, sizeof err);
    if (rc != 0) {
        gf_say("window: %s", rc < 0 ? err : "closed by the compositor");
        return -1;
    }
    return 0;
}

static gf_hdr generator_hdr(void) {
    /* The mastering display gfn_streams.py writes: BT.2020 primaries, D65, 1000 nits, 0.0001; MaxCLL 1000, MaxFALL 400; PQ/BT.2020. */
    gf_hdr h = {13250, 34500, 7500, 3000, 34000, 16000, 15635, 16450, 10000000, 1, 1000, 400, 16, 9, 9};
    return h;
}

/* ---- play: the probe ----------------------------------------------------------------------------------------------------------- */

static uint8_t *read_file(const char *path, size_t *n) {
    FILE *fp = fopen(path, "rb");
    if (!fp) return NULL;
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    uint8_t *d = malloc((size_t)len + 1);
    if (!d || fread(d, 1, (size_t)len, fp) != (size_t)len) {
        free(d);
        fclose(fp);
        return NULL;
    }
    fclose(fp);
    *n = (size_t)len;
    return d;
}

struct packets { uint8_t *data; size_t n; size_t *off; uint16_t *len; size_t count; };

static int read_packets(const char *path, struct packets *p) {
    memset(p, 0, sizeof *p);
    p->data = read_file(path, &p->n);
    if (!p->data) return -1;
    size_t pos = 0, count = 0;
    while (pos + 2 <= p->n) {
        uint16_t l = (uint16_t)(p->data[pos] | (p->data[pos + 1] << 8));
        pos += 2 + l;
        count++;
    }
    p->off = calloc(count, sizeof *p->off);
    p->len = calloc(count, sizeof *p->len);
    pos = 0;
    for (size_t i = 0; i < count; i++) {
        p->len[i] = (uint16_t)(p->data[pos] | (p->data[pos + 1] << 8));
        p->off[i] = pos + 2;
        pos += 2 + p->len[i];
    }
    p->count = count;
    return 0;
}

static int codec_of(const char *path) {
    const char *dot = strrchr(path, '.');
    if (!dot) return 0;
    if (!strcmp(dot, ".h264") || !strcmp(dot, ".264") || !strcmp(dot, ".avc")) return GF_CODEC_H264;
    if (!strcmp(dot, ".hevc") || !strcmp(dot, ".h265") || !strcmp(dot, ".265")) return GF_CODEC_H265;
    if (!strcmp(dot, ".obu") || !strcmp(dot, ".av1")) return GF_CODEC_AV1;
    return 0;
}

static int run_play(struct feeder *f) {
    size_t n;
    uint8_t *data = read_file(f->play_path, &n);
    if (!data) {
        gf_say("cannot read %s: %s", f->play_path, strerror(errno));
        return 2;
    }
    if (!f->codec) f->codec = codec_of(f->play_path);
    if (!f->codec) {
        gf_say("--codec is needed: the extension of %s says nothing", f->play_path);
        return 2;
    }
    size_t count = gf_split(data, n, f->codec, NULL, 0);
    gf_unit *units = calloc(count ? count : 1, sizeof *units);
    gf_split(data, n, f->codec, units, count);
    if (!count || !units[0].has_sps) {
        gf_say("%s: %zu units, the first %s a parameter set; the decoder needs one to start", f->play_path, count, count && units[0].has_sps ? "has" : "lacks");
        if (!count) return 2;
    }
    struct packets audio = {0};
    if (f->audio_path && read_packets(f->audio_path, &audio) != 0) {
        gf_say("cannot read %s", f->audio_path);
        return 2;
    }
    char err[300];
    if (open_window(f, err, sizeof err) != 0) {
        gf_say("window: %s", err);
        return 3;
    }
    gf_load l;
    memset(&l, 0, sizeof l);
    l.codec = (uint32_t)f->codec;
    l.width = (uint32_t)f->width;
    l.height = (uint32_t)f->height;
    l.fps = (uint32_t)f->fps;
    if (audio.count) {
        l.audio = GF_AUDIO_OPUS;
        l.channels = 2;
        l.sample_rate = 48000;
    }
    l.hdr = (uint32_t)f->hdr;
    l.hdr_info = generator_hdr();
    l.drop_threshold = f->drop_threshold;
    l.pts_unit = f->pts_unit;
    int64_t open_ns = gf_ns();
    if (start_pipeline(f, &l, err, sizeof err) != 0) {
        gf_say("pipeline: %s", err);
        close_all(f);
        return 4;
    }
    gf_say("playing %s: %zu units, %dx%d at %d fps, codec %d, window %s, audio %zu packets, hdr %d, pts unit %d", f->play_path, count, f->width,
           f->height, f->fps, f->codec, f->window_id, audio.count, f->hdr, f->pts_unit);
    fprintf(f->stats, "{\"start\":true,\"file\":\"%s\",\"units\":%zu,\"width\":%d,\"height\":%d,\"fps\":%d,\"codec\":%d,\"window\":\"%s\",\"audio\":%zu,"
                      "\"audio_route\":\"%s\",\"hdr\":%d,\"pts_unit\":%d,\"library\":\"%s\",\"features\":%u}\n",
            f->play_path, count, f->width, f->height, f->fps, f->codec, f->window_id, audio.count, audio.count ? f->audio_route : "none", f->hdr, f->pts_unit,
            f->ndl ? gf_ndl_lib_name(f->ndl) : "none", f->ndl ? gf_ndl_features(f->ndl) : 0);
    fflush(f->stats);

    int64_t period = 1000000000LL / f->fps, aperiod = 20000000LL;
    int64_t t0 = gf_ns(), next_v = t0, next_a = t0, next_sec = t0 + 1000000000LL, end = f->seconds > 0 ? t0 + (int64_t)f->seconds * 1000000000LL : 0;
    f->t0_ns = t0;
    f->sec.start_ns = t0;
    size_t vi = 0, ai = 0;
    int64_t pts_v = 0, pts_a = 0;
    int sec_no = 1, rc = 0, done = 0;
    while (!g_term && !done) {
        int64_t now = gf_ns();
        int64_t wake = next_sec;
        if (vi < count && next_v < wake) wake = next_v;
        if (audio.count && ai < audio.count && next_a < wake) wake = next_a;
        if (end && end < wake) wake = end;
        int64_t left = wake - now;
        if (f->burst) left = 0;
        struct pollfd fds[2];
        int nf = 0;
        if (f->wl) fds[nf++] = (struct pollfd){vpl_wl_fd(f->wl), POLLIN, 0};
        if (f->ndl_ready) fds[nf++] = (struct pollfd){gf_ndl_event_fd(f->ndl), POLLIN, 0};
        struct timespec to = {(time_t)(left > 0 ? left / 1000000000LL : 0), (long)(left > 0 ? left % 1000000000LL : 0)};
        int r = ppoll(fds, (nfds_t)nf, &to, NULL);
        if (r < 0 && errno != EINTR) break;
        if (f->wl && dispatch_window(f) != 0) {
            rc = 5;
            break;
        }
        if (f->ndl_ready) {
            drain_events(f, NULL);
            if (f->error) {
                gf_say("the pipeline reported an error; stopping");
                rc = 6;
                break;
            }
        }
        now = gf_ns();
        /* video: late ticks are fed at most four at a time; a stall longer than a quarter second resets the schedule */
        if (now - next_v > 250000000LL && !f->burst) next_v = now;
        for (int b = 0; b < 4 && vi < count && (f->burst || now >= next_v); b++) {
            int q, w;
            int res = feed_video(f, data + units[vi].offset, units[vi].size, pts_v, &q, &w);
            if (res < 0) break;
            pts_v += period;
            next_v += period;
            if (++vi >= count && f->loop) {
                vi = 0;             /* the clips start with an IDR and its headers, so the loop is a clean restart of the GOP */
            }
            if (f->burst && q > 8) break;
        }
        while (audio.count && ai < audio.count && now >= next_a) {
            feed_audio(f, audio.data + audio.off[ai], audio.len[ai], pts_a);
            pts_a += aperiod;
            next_a += aperiod;
            if (++ai >= audio.count && f->loop) ai = 0;
        }
        if (now >= next_sec) {
            write_second(f, sec_no++);
            next_sec += 1000000000LL;
        }
        if ((vi >= count && !f->loop && (!audio.count || ai >= audio.count)) || (end && now >= end)) done = 1;
    }
    /* the last units are shown after they arrive; give the pipeline a moment and read its events once more */
    if (f->ndl_ready) {
        struct pollfd pf = {gf_ndl_event_fd(f->ndl), POLLIN, 0};
        poll(&pf, 1, 300);
        drain_events(f, NULL);
    }
    write_second(f, sec_no);
    int64_t total_ns = gf_ns() - t0;
    fprintf(f->stats, "{\"summary\":true,\"units\":%zu,\"fed\":%llu,\"dropped\":%llu,\"afed\":%llu,\"adropped\":%llu,\"retries\":%llu,\"seconds\":%.2f,"
                      "\"loaded_ms\":%lld,\"playing_ms\":%lld,\"loaded\":%d,\"playing\":%d,\"error\":%d,\"audio_route\":\"%s\",\"adecode_errors\":%llu,\"events\":{",
            count, (unsigned long long)f->total_fed, (unsigned long long)f->total_dropped, (unsigned long long)f->total_afed,
            (unsigned long long)f->total_adropped, (unsigned long long)f->total_retries, total_ns / 1e9,
            f->first_loaded_ns ? (long long)((f->first_loaded_ns - open_ns) / 1000000) : -1LL,
            f->first_playing_ns ? (long long)((f->first_playing_ns - open_ns) / 1000000) : -1LL, f->loaded, f->playing, f->error,
            audio.count ? f->audio_route : "none", (unsigned long long)f->decode_errors);
    int first = 1;
    for (int i = 0; i < 64; i++)
        if (f->ev_count[i]) {
            fprintf(f->stats, "%s\"%d\":%u", first ? "" : ",", i, f->ev_count[i]);
            first = 0;
        }
    fprintf(f->stats, "},\"video_info\":\"%s\",\"exit\":%d}\n", f->last_video_info, rc);
    fflush(f->stats);
    close_all(f);
    free(units);
    free(data);
    free(audio.data);
    free(audio.off);
    free(audio.len);
    return rc;
}

/* ---- the UI process (the installed app) ------------------------------------------------------------------------------------------- */

/* Starts the 64-bit UI the way the card app's launcher does (rustui/app/stub/lumatv-card.c): through gfx64/launch.sh with the
 * package's own loader, heap shim and Mali driver, in its own process group, gone with this process. What SAM gave this 32-bit
 * program (LD_PRELOAD, LD_LIBRARY_PATH) must not reach the 64-bit loader. `channel` is the UI's end of the socket pair (its
 * descriptor 3), -1 for none; `hidden` is a preload. */
/* A directory this user can keep files in across a restart of the TV, for the account the core signs in and the app's preferences. SAM's
 * jail user cannot write /var/lib; the places webOS keeps an app's own data are tried in turn and every answer is logged, so the TV shows
 * which one the jail allows. Nothing found: an empty string, and the UI keeps its files under /tmp. */
static const char *persistent_dir(const char *app_id, char *out, size_t cap) {
    const char *base[] = {"/media/developer/apps/usr/palm/data", "/media/cryptofs/apps/usr/palm/data", "/media/internal", "/var/luna/preferences", "/var/palm/data", "/media/developer/temp"};
    const char *only = getenv("LUMATV_GFN_DATA_BASE");       /* the host tests name their own */
    for (size_t i = 0; i < (only ? 1 : sizeof base / sizeof *base); i++) {
        char dir[400], probe[460];
        snprintf(dir, sizeof dir, "%s/%s", only ? only : base[i], app_id);
        int made = mkdir(dir, 0700) == 0 || errno == EEXIST;
        int made_errno = errno;
        int ok = 0;
        if (made) {
            snprintf(probe, sizeof probe, "%s/.write-test-%d", dir, (int)getpid());
            int fd = open(probe, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
            if (fd >= 0) {
                ok = write(fd, "x", 1) == 1;
                close(fd);
                unlink(probe);
            }
        }
        gf_say("data directory %s: %s", dir, ok ? "writable" : made ? "not writable" : strerror(made_errno));
        if (ok && !out[0]) snprintf(out, cap, "%s", dir);
    }
    return out;
}

static pid_t start_ui(const char *home, const char *app_id, int channel, int hidden) {
    char launch[600], bundle[600], shim[600], app[600], assets[600], run64[600], engine[600];
    snprintf(run64, sizeof run64, "%s/run64.sh", home);
    snprintf(engine, sizeof engine, "%s/bin64/opennow-streamer", home);
    snprintf(launch, sizeof launch, "%s/gfx64/launch.sh", home);
    snprintf(bundle, sizeof bundle, "%s/gfx64/bundle", home);
    snprintf(shim, sizeof shim, "%s/gfx64/heapshim.so", home);
    snprintf(app, sizeof app, "%s/bin64/lumatv-gfn-app", home);
    snprintf(assets, sizeof assets, "%s/assets", home);
    if (access(app, X_OK) != 0) {
        gf_say("no UI at %s: serving the engine only", app);
        return 0;
    }
    char data[460] = "";
    persistent_dir(app_id, data, sizeof data);
    pid_t pid = fork();
    if (pid < 0) {
        gf_say("fork: %s", strerror(errno));
        return 0;
    }
    if (pid == 0) {
        setpgid(0, 0);
        prctl(PR_SET_PDEATHSIG, SIGTERM);
        unsetenv("LD_PRELOAD");
        unsetenv("LD_LIBRARY_PATH");
        setenv("PATH", "/usr/sbin:/usr/bin:/sbin:/bin", 0);
        if (channel >= 0) {
            if (channel != 3) dup2(channel, 3);
            else fcntl(3, F_SETFD, 0);
        }
        for (int fd = channel >= 0 ? 4 : 3; fd < 256; fd++) close(fd);
        const char *argv[24];
        int n = 0;
        argv[n++] = "sh";
        argv[n++] = launch;
        argv[n++] = "--bundle";
        argv[n++] = bundle;
        argv[n++] = "--shim";
        argv[n++] = shim;
        argv[n++] = "--";
        argv[n++] = app;
        argv[n++] = "--app-id";
        argv[n++] = app_id;
        argv[n++] = "--assets";
        argv[n++] = assets;
        argv[n++] = "--shm";
        argv[n++] = "/tmp/lumatv-gfn-shm";
        /* The UI finds its neighbours from its own executable's directory, which under launch.sh is the loader's: say where they are. */
        argv[n++] = "--core";
        argv[n++] = run64;
        argv[n++] = "--engine";
        argv[n++] = engine;
        if (channel >= 0) {
            argv[n++] = "--channel-fd";
            argv[n++] = "3";
        }
        if (hidden) argv[n++] = "--hidden";
        if (data[0]) {
            argv[n++] = "--data";
            argv[n++] = data;
        }
        argv[n] = NULL;
        execv("/bin/sh", (char *const *)argv);
        _exit(127);
    }
    setpgid(pid, pid);
    gf_say("the UI started: pid %d%s", (int)pid, hidden ? " (hidden: a preload)" : "");
    return pid;
}

/* The UI's process group is asked to end and, if it has not in two seconds, killed: an app that ignores SAM's close is not left behind. */
static void stop_ui(struct feeder *f) {
    if (f->ui_pid <= 0) return;
    kill(-f->ui_pid, SIGTERM);
    for (int i = 0; i < 40; i++) {
        if (waitpid(f->ui_pid, NULL, WNOHANG) == f->ui_pid) {
            f->ui_pid = 0;
            return;
        }
        usleep(50000);
    }
    gf_say("the UI ignored SIGTERM: killed");
    kill(-f->ui_pid, SIGKILL);
    waitpid(f->ui_pid, NULL, 0);
    f->ui_pid = 0;
}

/* ---- SAM's events, and the UI's channel -------------------------------------------------------------------------------------------- */

/* How long a UI that was asked to close has before the feeder ends it (the card launcher's 4 s). */
#define CLOSE_GRACE_NS 4000000000LL

static void chan_close(struct feeder *f) {
    if (f->chan >= 0) close(f->chan);
    f->chan = -1;
}

/* SAM's lines to the UI. A `close` also starts the clock on the UI's end: with no UI (or none that listens) the feeder is the app. */
static void sam_pump(struct feeder *f) {
    char line[2048];
    while (gf_sam_next(f->sam, line, sizeof line)) {
        size_t n = strlen(line);
        if (strstr(line, "\"kind\":\"close\"")) {
            f->close_at = gf_ns() + (f->ui_pid > 0 && f->chan >= 0 ? CLOSE_GRACE_NS : 0);
            gf_say("SAM asks the app to close");
        }
        if (f->chan < 0) continue;
        line[n++] = '\n';
        if (send(f->chan, line, n, MSG_NOSIGNAL | MSG_DONTWAIT) != (ssize_t)n) gf_say("the UI's channel took no line (%s)", strerror(errno));
    }
}

/* What the UI says on the channel (ready, state, page, exit): the log has it. */
static void chan_pump(struct feeder *f) {
    for (;;) {
        ssize_t r = recv(f->chan, f->chan_in + f->chan_len, sizeof f->chan_in - f->chan_len - 1, MSG_DONTWAIT);
        if (r == 0 || (r < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
            chan_close(f);
            return;
        }
        if (r < 0) break;
        f->chan_len += (size_t)r;
        f->chan_in[f->chan_len] = 0;
        char *nl;
        while ((nl = memchr(f->chan_in, '\n', f->chan_len))) {
            *nl = 0;
            gf_say("UI: %s", f->chan_in);
            f->chan_len -= (size_t)(nl + 1 - f->chan_in);
            memmove(f->chan_in, nl + 1, f->chan_len);
        }
        if (f->chan_len >= sizeof f->chan_in - 1) f->chan_len = 0;
    }
}

/* Adds SAM's descriptor and the channel to `fds` (at most two) and returns how many; `serve_extras` handles what came back. */
static int extras_add(struct feeder *f, struct pollfd *fds) {
    int n = 0;
    if (f->sam) fds[n++] = (struct pollfd){gf_sam_fd(f->sam), POLLIN, 0};
    if (f->chan >= 0) fds[n++] = (struct pollfd){f->chan, POLLIN, 0};
    return n;
}

static void extras_serve(struct feeder *f, const struct pollfd *fds, int n) {
    for (int i = 0; i < n; i++) {
        if (!(fds[i].revents & (POLLIN | POLLHUP | POLLERR))) continue;
        if (f->sam && fds[i].fd == gf_sam_fd(f->sam)) sam_pump(f);
        else if (fds[i].fd == f->chan) chan_pump(f);
    }
    if (f->close_at && gf_ns() >= f->close_at) {
        gf_say("the UI did not end in time after SAM's close: ending");
        g_term = 1;
    }
}

/* 1 when the UI process has ended (the app is over: SAM sees this process end with it). */
static int ui_gone(struct feeder *f) {
    if (f->ui_pid <= 0) return 0;
    int status;
    pid_t r = waitpid(f->ui_pid, &status, WNOHANG);
    if (r == f->ui_pid) {
        gf_say("the UI ended (%s %d)", WIFSIGNALED(status) ? "signal" : "status", WIFSIGNALED(status) ? WTERMSIG(status) : WEXITSTATUS(status));
        f->ui_pid = 0;
        return 1;
    }
    return 0;
}

/* ---- serve: the engine's sink ---------------------------------------------------------------------------------------------------- */

struct client {
    int fd;
    uint8_t *in;
    size_t in_len, in_cap;
    uint8_t *out;
    size_t out_len, out_cap;
    int broken;
};

static int push_out(struct client *c, uint8_t type, const void *a, size_t na, const void *b, size_t nb) {
    size_t need = sizeof(gf_frame) + na + nb;
    if (c->out_len + need > c->out_cap) {
        if (c->out_cap > (4u << 20)) {              /* the engine is not reading: it is gone, or stuck; nothing is worth blocking the feed for */
            c->broken = 1;
            return -1;
        }
        size_t cap = c->out_cap ? c->out_cap * 2 : 65536;
        while (cap < c->out_len + need) cap *= 2;
        c->out = realloc(c->out, cap);
        c->out_cap = cap;
    }
    gf_frame h = {type, 0, 0, (uint32_t)(na + nb)};
    memcpy(c->out + c->out_len, &h, sizeof h);
    if (na) memcpy(c->out + c->out_len + sizeof h, a, na);
    if (nb) memcpy(c->out + c->out_len + sizeof h + na, b, nb);
    c->out_len += need;
    return 0;
}

static int flush_out(struct client *c) {
    while (c->out_len) {
        ssize_t w = send(c->fd, c->out, c->out_len, MSG_NOSIGNAL | MSG_DONTWAIT);
        if (w < 0) {
            if (errno == EAGAIN || errno == EINTR) return 0;
            c->broken = 1;
            return -1;
        }
        memmove(c->out, c->out + w, c->out_len - (size_t)w);
        c->out_len -= (size_t)w;
    }
    return 0;
}

static struct client *g_client;     /* the event callback's way to the engine */

static void emit_event(struct feeder *f, int type, long long num, const char *text) {
    (void)f;
    if (!g_client) return;
    gf_event e = {type, 0, num};
    push_out(g_client, GF_EVENT, &e, sizeof e, text, strlen(text));
}

static void send_text(struct client *c, uint8_t type, int32_t code, const char *text) {
    push_out(c, type, &code, sizeof code, text, strlen(text));
}

static void handle_message(struct feeder *f, struct client *c, uint8_t type, const uint8_t *p, size_t n) {
    switch (type) {
    case GF_LOAD: {
        if (n < sizeof(gf_load)) {
            send_text(c, GF_ERROR, 1, "LOAD is short");
            return;
        }
        gf_load l;
        memcpy(&l, p, sizeof l);
        if (f->ndl_ready) stop_pipeline(f);
        f->pts_unit = l.pts_unit > 0 ? l.pts_unit : GF_PTS_NS;
        char err[300];
        if (start_pipeline(f, &l, err, sizeof err) != 0) {
            gf_say("load: %s", err);
            send_text(c, GF_LOADED, 0, err);
            return;
        }
        gf_say("loaded: codec %u %ux%u at %u fps, audio %u (%u ch, %u Hz) as %s, hdr %u, drop threshold %d, pts unit %d", l.codec, l.width, l.height, l.fps,
               l.audio, l.channels, l.sample_rate, f->audio_route, l.hdr, l.drop_threshold, l.pts_unit);
        f->total_fed = f->total_dropped = f->total_afed = f->total_adropped = f->total_retries = 0;
        memset(&f->sec, 0, sizeof f->sec);
        f->sec.start_ns = gf_ns();
        send_text(c, GF_LOADED, 1, f->window_id);
        return;
    }
    case GF_VIDEO: {
        if (n < sizeof(gf_media_head)) return;
        gf_media_head h;
        memcpy(&h, p, sizeof h);
        int q, w;
        int res = feed_video(f, p + sizeof h, n - sizeof h, h.pts_ns, &q, &w);
        if (res == 0) {
            gf_accept a = {h.pts_ns, q, w};
            push_out(c, GF_ACCEPT, &a, sizeof a, NULL, 0);
        } else {
            gf_drop d = {h.pts_ns, res < 0 ? GF_DROP_NOT_LOADED : GF_DROP_FULL, 0};
            push_out(c, GF_DROP, &d, sizeof d, NULL, 0);
        }
        return;
    }
    case GF_AUDIO: {
        if (n < sizeof(gf_media_head)) return;
        gf_media_head h;
        memcpy(&h, p, sizeof h);
        feed_audio(f, p + sizeof h, n - sizeof h, h.pts_ns);
        return;
    }
    case GF_FLUSH:
        if (f->ndl_ready) {
            int rc = gf_ndl_flush(f->ndl);
            gf_say("flush: %d", rc);
        }
        return;
    case GF_UNLOAD:
        stop_pipeline(f);
        gf_say("unloaded");
        return;
    case GF_STATE:
        if (f->ndl_ready && n >= 1) gf_ndl_set_state(f->ndl, p[0]);
        return;
    case GF_HDR: {
        if (n < sizeof(gf_hdr)) return;
        gf_hdr h;
        memcpy(&h, p, sizeof h);
        if (f->ndl_ready) {
            int rc = gf_ndl_set_hdr(f->ndl, &h);
            gf_say("hdr: %d", rc);
        }
        return;
    }
    case GF_PING: {
        int64_t echo = 0;
        if (n >= 8) memcpy(&echo, p, 8);
        int64_t reply[2] = {echo, gf_ns()};
        push_out(c, GF_PONG, reply, sizeof reply, NULL, 0);
        return;
    }
    default:
        gf_say("unknown message %u (%zu bytes)", type, n);
        return;
    }
}

static int read_client(struct feeder *f, struct client *c) {
    if (c->in_len + 65536 > c->in_cap) {
        size_t cap = c->in_cap ? c->in_cap * 2 : 1 << 20;
        while (cap < c->in_len + 65536) cap *= 2;
        c->in = realloc(c->in, cap);
        c->in_cap = cap;
    }
    ssize_t got = recv(c->fd, c->in + c->in_len, c->in_cap - c->in_len, MSG_DONTWAIT);
    if (got == 0) return -1;
    if (got < 0) return (errno == EAGAIN || errno == EINTR) ? 0 : -1;
    c->in_len += (size_t)got;
    size_t pos = 0;
    while (c->in_len - pos >= sizeof(gf_frame)) {
        gf_frame h;
        memcpy(&h, c->in + pos, sizeof h);
        if (h.length > GF_MAX_PAYLOAD) {
            gf_say("a %u byte message: the engine is confused", h.length);
            return -1;
        }
        if (c->in_len - pos < sizeof h + h.length) {
            if (c->in_cap < sizeof h + h.length + 65536) {
                size_t cap = sizeof h + h.length + 65536;
                c->in = realloc(c->in, cap);
                c->in_cap = cap;
            }
            break;
        }
        handle_message(f, c, h.type, c->in + pos + sizeof h, h.length);
        pos += sizeof h + h.length;
    }
    if (pos) {
        memmove(c->in, c->in + pos, c->in_len - pos);
        c->in_len -= pos;
    }
    return 0;
}

static int run_serve(struct feeder *f) {
    char err[300];
    if (open_window(f, err, sizeof err) != 0) {
        gf_say("window: %s", err);
        return 3;
    }
    if (!f->no_ndl && gf_ndl_open_lib(&f->ndl, f->lib, err, sizeof err) != 0) gf_say("no library: %s (the engine will be told)", err);
    int ls = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_un sa;
    memset(&sa, 0, sizeof sa);
    sa.sun_family = AF_UNIX;
    snprintf(sa.sun_path, sizeof sa.sun_path, "%s", f->sock_path);
    unlink(f->sock_path);
    if (bind(ls, (struct sockaddr *)&sa, sizeof sa) != 0 || listen(ls, 1) != 0) {
        gf_say("cannot listen on %s: %s", f->sock_path, strerror(errno));
        return 3;
    }
    chmod(f->sock_path, 0660);
    int ui_end = -1;
    if (f->sam_mode && f->ui_home) {
        int sv[2];
        if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sv) == 0) {
            f->chan = sv[0];
            ui_end = sv[1];
        } else {
            gf_say("socketpair: %s (the UI gets no SAM events)", strerror(errno));
        }
    }
    if (f->sam_mode) {
        f->sam = gf_sam_start(f->app_id, err, sizeof err);
        if (!f->sam) gf_say("SAM: %s (the app runs without SAM's events)", err);
    }
    if (f->ui_home) f->ui_pid = start_ui(f->ui_home, f->app_id, ui_end, f->preload);
    if (ui_end >= 0) close(ui_end);
    gf_say("serving %s, window %s, library %s", f->sock_path, f->window_id, f->ndl ? gf_ndl_lib_name(f->ndl) : "none");
    fprintf(f->stats, "{\"serving\":\"%s\",\"window\":\"%s\",\"library\":\"%s\",\"features\":%u,\"pid\":%d}\n", f->sock_path, f->window_id,
            f->ndl ? gf_ndl_lib_name(f->ndl) : "none", f->ndl ? gf_ndl_features(f->ndl) : 0, (int)getpid());
    fflush(f->stats);
    int rc = 0;
    while (!g_term) {
        struct client c;
        memset(&c, 0, sizeof c);
        c.fd = -1;
        while (!g_term && c.fd < 0) {
            struct pollfd fds[4];
            int nf = 0;
            fds[nf++] = (struct pollfd){ls, POLLIN, 0};
            if (f->wl) fds[nf++] = (struct pollfd){vpl_wl_fd(f->wl), POLLIN, 0};
            int nx = extras_add(f, fds + nf);
            int r = poll(fds, (nfds_t)(nf + nx), 1000);
            if (r > 0) extras_serve(f, fds + nf, nx);
            else if (f->close_at) extras_serve(f, fds + nf, 0);
            if (r < 0 && errno != EINTR) {
                rc = 3;
                goto out;
            }
            if (ui_gone(f)) goto out;
            if (f->wl && nf > 1 && (fds[1].revents & (POLLIN | POLLHUP | POLLERR)) && dispatch_window(f) != 0) {
                rc = 5;
                goto out;
            }
            if (fds[0].revents & POLLIN) c.fd = accept4(ls, NULL, NULL, SOCK_CLOEXEC);
        }
        if (c.fd < 0) break;
        gf_say("engine connected");
        g_client = &c;
        gf_hello hello;
        memset(&hello, 0, sizeof hello);
        hello.version = GF_PROTO_VERSION;
        hello.library = f->ndl != NULL;
        hello.pid = (uint32_t)getpid();
        hello.window = f->wl != NULL;
        snprintf(hello.window_id, sizeof hello.window_id, "%s", f->window_id);
        snprintf(hello.library_name, sizeof hello.library_name, "%s", f->ndl ? gf_ndl_lib_name(f->ndl) : "");
        push_out(&c, GF_HELLO, &hello, sizeof hello, NULL, 0);
        flush_out(&c);
        int64_t next_status = gf_ns() + 1000000000LL;
        while (!g_term && !c.broken) {
            struct pollfd fds[5];
            int nf = 0;
            fds[nf++] = (struct pollfd){c.fd, POLLIN | (c.out_len ? POLLOUT : 0), 0};
            if (f->wl) fds[nf++] = (struct pollfd){vpl_wl_fd(f->wl), POLLIN, 0};
            if (f->ndl_ready) fds[nf++] = (struct pollfd){gf_ndl_event_fd(f->ndl), POLLIN, 0};
            int64_t left = (next_status - gf_ns()) / 1000000;
            int nx = extras_add(f, fds + nf);
            int r = poll(fds, (nfds_t)(nf + nx), left < 0 ? 0 : (int)left);
            if (r > 0) extras_serve(f, fds + nf, nx);
            else if (f->close_at) extras_serve(f, fds + nf, 0);
            if (r < 0 && errno != EINTR) break;
            if (ui_gone(f)) {
                g_term = 1;
                break;
            }
            if (fds[0].revents & (POLLIN | POLLHUP | POLLERR)) {
                if (read_client(f, &c) != 0) break;
            }
            if (f->wl && (fds[1].revents & (POLLIN | POLLHUP | POLLERR)) && dispatch_window(f) != 0) {
                rc = 5;
                g_term = 1;
                break;
            }
            if (f->ndl_ready) {
                drain_events(f, emit_event);
                if (f->error) {
                    send_text(&c, GF_ERROR, 2, "the pipeline reported an error");
                    f->error = 0;
                }
            }
            if (gf_ns() >= next_status) {
                if (f->ndl_ready || f->no_ndl) {
                    gf_status st;
                    fill_status(f, &st);
                    push_out(&c, GF_STATUS, &st, sizeof st, NULL, 0);
                    memset(&f->sec, 0, sizeof f->sec);
                    f->sec.start_ns = gf_ns();
                }
                next_status += 1000000000LL;
            }
            flush_out(&c);
        }
        gf_say("engine gone");
        g_client = NULL;
        close(c.fd);
        free(c.in);
        free(c.out);
        stop_pipeline(f);
        if (f->once) break;
    }
out:
    close(ls);
    unlink(f->sock_path);
    close_all(f);
    stop_ui(f);
    if (f->sam) gf_sam_stop(f->sam);
    chan_close(f);
    return rc;
}

/* ---- probe and entry ----------------------------------------------------------------------------------------------------------------- */

static int run_probe(struct feeder *f) {
    char err[300] = "";
    gf_ndl *n = NULL;
    int ok = gf_ndl_open_lib(&n, f->lib, err, sizeof err) == 0;
    printf("{\"library\":%s,\"name\":\"%s\",\"features\":%u,\"error\":\"%s\",\"app_id\":\"%s\",\"proto\":%d,\"sizes\":{\"load\":%zu,\"status\":%zu,\"hello\":%zu}}\n",
           ok ? "true" : "false", ok ? gf_ndl_lib_name(n) : "", ok ? gf_ndl_features(n) : 0, err, f->app_id, GF_PROTO_VERSION, sizeof(gf_load),
           sizeof(gf_status), sizeof(gf_hello));
    if (ok) gf_ndl_free(n);
    return ok ? 0 : 1;
}

static void usage(void) {
    fprintf(stderr,
            "usage: lumatv-gfn-feeder --probe | --play FILE [--codec h264|h265|av1] [--size WxH] [--fps N] [--seconds S] [--loop] [--hdr]\n"
            "         [--audio PKTS] [--audio-mode auto|opus|pcm] [--drop-threshold N] [--pts-unit ns|us|ms] [--full-wait-ms N] [--burst] [--no-ndl] [--stats FILE]\n"
            "       | --serve SOCKET [--once]\n"
            "  common: [-w card|overlay|default|screensaver|video] [--window-id ID] [--window-id-file F] [--no-window] [-a APP_ID] [-L NDL_LIB] [-e first|after]\n"
            "  serve: [--ui-home DIR] starts DIR/bin64/lumatv-gfn-app through DIR/gfx64/launch.sh (SAM mode does this from the app's own directory);\n"
            "         [--sam] registers with SAM (no arguments, or SAM's launch object as the one argument, means serve with --sam)\n");
}

int main(int argc, char **argv) {
    struct feeder f;
    memset(&f, 0, sizeof f);
    f.chan = -1;
    static char launch_id[128], launch_note[96];
    static char app_id_buf[128];
    f.app_id = getenv("APPID") ? getenv("APPID") : app_id_from_exe(app_id_buf, sizeof app_id_buf);
    f.window_type = "card";
    f.width = 3840;
    f.height = 2160;
    f.fps = 120;
    f.drop_threshold = -1;
    f.pts_unit = GF_PTS_NS;
    f.full_wait_ms = 8;
    f.export_first = 1;
    f.stats = stdout;
    int mode = 0;
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        const char *v = i + 1 < argc ? argv[i + 1] : NULL;
        if (!strcmp(a, "--probe")) mode = 1;
        else if (!strcmp(a, "--play") && v) mode = 2, f.play_path = argv[++i];
        else if (!strcmp(a, "--serve") && v) mode = 3, f.sock_path = argv[++i];
        else if (!strcmp(a, "--codec") && v) {
            i++;
            f.codec = !strcmp(argv[i], "h264") ? GF_CODEC_H264 : !strcmp(argv[i], "h265") || !strcmp(argv[i], "hevc") ? GF_CODEC_H265 : !strcmp(argv[i], "av1") ? GF_CODEC_AV1 : 0;
            if (!f.codec) {
                usage();
                return 2;
            }
        } else if (!strcmp(a, "--size") && v) {
            if (sscanf(argv[++i], "%dx%d", &f.width, &f.height) != 2) {
                usage();
                return 2;
            }
        } else if (!strcmp(a, "--fps") && v) f.fps = atoi(argv[++i]);
        else if (!strcmp(a, "--seconds") && v) f.seconds = atoi(argv[++i]);
        else if (!strcmp(a, "--loop")) f.loop = 1;
        else if (!strcmp(a, "--hdr")) f.hdr = 1;
        else if (!strcmp(a, "--audio") && v) f.audio_path = argv[++i];
        else if (!strcmp(a, "--audio-mode") && v) {
            i++;
            f.audio_mode = !strcmp(argv[i], "auto") ? AUDIO_AUTO : !strcmp(argv[i], "opus") ? AUDIO_OPUS : !strcmp(argv[i], "pcm") ? AUDIO_PCM : -1;
            if (f.audio_mode < 0) {
                usage();
                return 2;
            }
        }
        else if (!strcmp(a, "--drop-threshold") && v) f.drop_threshold = atoi(argv[++i]);
        else if (!strcmp(a, "--pts-unit") && v) {
            i++;
            f.pts_unit = !strcmp(argv[i], "ns") ? GF_PTS_NS : !strcmp(argv[i], "us") ? GF_PTS_US : !strcmp(argv[i], "ms") ? GF_PTS_MS : 0;
            if (!f.pts_unit) {
                usage();
                return 2;
            }
        } else if (!strcmp(a, "--full-wait-ms") && v) f.full_wait_ms = atoi(argv[++i]);
        else if (!strcmp(a, "--burst")) f.burst = 1;
        else if (!strcmp(a, "--no-ndl")) f.no_ndl = 1;
        else if (!strcmp(a, "--no-window")) f.no_window = 1;
        else if (!strcmp(a, "--once")) f.once = 1;
        else if (!strcmp(a, "--stats") && v) f.stats_path = argv[++i];
        else if (!strcmp(a, "-w") && v) f.window_type = argv[++i];
        else if (!strcmp(a, "--window-id") && v) f.window_id_opt = argv[++i];
        else if (!strcmp(a, "--window-id-file") && v) f.window_id_file = argv[++i];
        else if (!strcmp(a, "--ui-home") && v) f.ui_home = argv[++i];
        else if (!strcmp(a, "-a") && v) f.app_id = argv[++i];
        else if (!strcmp(a, "-L") && v) f.lib = argv[++i];
        else if (!strcmp(a, "-e") && v) f.export_first = strcmp(argv[++i], "after") != 0;
        else if (!strcmp(a, "--sam")) f.sam_mode = 1;
        else if (a[0] == '{') {
            /* SAM starts a version 2 native app as `<main> '<json>'` (gf_sam.h) */
            struct gf_launch l;
            if (gf_sam_parse_launch(a, &l) != 0) {
                gf_say("not a launch object: %.120s", a);
                usage();
                return 2;
            }
            f.sam_mode = 1;
            f.preload = l.preload;
            if (l.app_id[0]) {
                snprintf(launch_id, sizeof launch_id, "%s", l.app_id);
                f.app_id = launch_id;
            }
            snprintf(launch_note, sizeof launch_note, "launched by SAM: reason %s%s", l.reason, l.preload ? " (a preload)" : "");
        } else {
            usage();
            return 2;
        }
    }
    if (!mode) {
        /* SAM starts the app's `main` with no arguments: start the 64-bit UI from the app's directory and serve the engine on the
         * default socket; the UI owns the card window and writes the id of its video element to the window-id file. */
        mode = 3;
        f.sam_mode = 1;
        f.sock_path = DEFAULT_SOCKET;
        mkdir("/tmp/lumatv-gfn", 0770);
        static char home[512];
        ssize_t n = readlink("/proc/self/exe", home, sizeof home - 1);
        if (n > 0) {
            home[n] = 0;
            char *slash = strrchr(home, '/');
            if (slash) *slash = 0;
            f.ui_home = home;
        }
        if (!f.window_id_file) f.window_id_file = DEFAULT_WINDOW_ID_FILE;
    }
    if (f.fps <= 0 || f.width <= 0 || f.height <= 0) {
        usage();
        return 2;
    }
    if (f.sam_mode) {
        /* SAM gives the app no environment and no terminal: the compositor's socket and the shell's PATH are the card launcher's defaults
         * (lumatv-card.c), and what the feeder and the UI say goes to a file, the only way to see it on the set. */
        setenv("XDG_RUNTIME_DIR", "/tmp/xdg", 0);
        setenv("WAYLAND_DISPLAY", "wayland-0", 0);
        setenv("PATH", "/usr/sbin:/usr/bin:/sbin:/bin", 0);
        const char *log = getenv("LUMATV_GFN_LOG");
        if (!log) log = DEFAULT_LOG;
        if (strcmp(log, "-") != 0) {
            mkdir("/tmp/lumatv-gfn", 0770);
            int fd = open(log, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666);
            if (fd >= 0) {
                dup2(fd, 1);
                dup2(fd, 2);
                if (fd > 2) close(fd);
            }
        }
        if (launch_note[0]) gf_say("%s", launch_note);
    }
    if (f.stats_path) {
        f.stats = fopen(f.stats_path, "w");
        if (!f.stats) {
            gf_say("cannot write %s: %s", f.stats_path, strerror(errno));
            return 2;
        }
    }
    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);
    signal(SIGALRM, on_alarm);
    signal(SIGPIPE, SIG_IGN);
    int rc = mode == 1 ? run_probe(&f) : mode == 2 ? run_play(&f) : run_serve(&f);
    if (f.stats != stdout) fclose(f.stats);
    return rc;
}
