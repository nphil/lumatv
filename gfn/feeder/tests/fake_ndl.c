/* A stand-in for LG's libNDL_directmedia.so.1 (API version 2) for the feeder's host tests: the demands the real one makes (the order of
 * DL_Initialize, SetWindowId, Init, Load, SetAppState(FOREGROUND); whole access units with a rising PTS) are enforced and a call that
 * breaks one is logged as VIOLATION; every call goes to $FAKE_NDL_LOG. vplane/tests/fake_ndl.c is the same idea for the saver; this one
 * also has the audio feed, the HDR info, the drop threshold and the flush the stream uses.
 *
 * FAKE_NDL_FAIL=dlinit|init|load|state     that step fails
 * FAKE_NDL_FAIL=opus                       a load that asks for Opus audio fails (the library on the set may refuse it); PCM loads
 * FAKE_NDL_AUDIO_DUMP=FILE                 the bytes of every NDL_DirectAudioPlay are appended to FILE (the PCM the feeder decoded)
 * FAKE_NDL_VIDEO_DUMP=FILE                 the bytes of every NDL_DirectVideoPlay the library took are appended to FILE (the access units the feeder gave it)
 * FAKE_NDL_NO_PLAYING=1                    the pipeline loads but never says it is playing
 * FAKE_NDL_ERROR_AFTER=N                   an error event after N video units
 * FAKE_NDL_FULL_EVERY=N                    every Nth NDL_DirectVideoPlay says the buffer is full (once; the retry is taken)
 * FAKE_NDL_FULL_STICKY=1                   with FULL_EVERY: the refusal lasts for the whole unit (every retry fails too)
 * FAKE_NDL_QUEUE=N                         the render queue's length (default: units fed in the last 8 ms, capped at 4)
 * FAKE_NDL_LOAD_MS=N                       the load takes this long (default 20)
 * FAKE_NDL_VERBOSE=1                       every unit is logged
 * FAKE_NDL_BUFFER_EVENTS=1                 a BufferFull (45) event at every 100th unit and a BufferLow (44) 10 units later
 */
#define _GNU_SOURCE
#include <pthread.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef void (*load_cb)(int, long long, const char *);

struct pcm_info { int type, unknown1; const char *format, *layout, *channelMode; int sampleRate; };
struct opus_info { int type, unknown1, channels, unknown2; double sampleRate; const char *streamHeader; };
struct data_info {
    struct { int width, height, type, unknown1; } video;
    union { int type; struct pcm_info pcm; struct opus_info opus; char padding[32]; } audio;
};
struct hdr_info { unsigned v[15]; };

static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;
static FILE *logf;
static int dl_inited, window_set, inited, loaded, foreground;
static char window_id[64], err[160] = "";
static load_cb cb;
static int played, shown, dropped, full_n, audio_played, offered, audio_channels, audio_type;
static long long offered_pts = -1;
static long long last_pts = -1, last_apts = -1;
static long long recent_ns[64];
static int recent_i;

static long long ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

static void say(const char *fmt, ...) {
    pthread_mutex_lock(&mu);
    if (!logf) {
        const char *p = getenv("FAKE_NDL_LOG");
        logf = p ? fopen(p, "a") : stderr;
        if (!logf) logf = stderr;
    }
    va_list ap;
    va_start(ap, fmt);
    fprintf(logf, "T %lld ", ns() / 1000000);
    vfprintf(logf, fmt, ap);
    fputc('\n', logf);
    fflush(logf);
    va_end(ap);
    pthread_mutex_unlock(&mu);
}

static int env_is(const char *k, const char *v) {
    const char *e = getenv(k);
    return e && !strcmp(e, v);
}
static int env_int(const char *k, int def) {
    const char *e = getenv(k);
    return e ? atoi(e) : def;
}

static int bad(const char *what) {
    say("VIOLATION %s", what);
    snprintf(err, sizeof err, "%s", what);
    return -1;
}

bool NDL_DirectMedia_DL_Initialize(void) {
    say("DL_Initialize");
    if (env_is("FAKE_NDL_FAIL", "dlinit")) return false;
    dl_inited = 1;
    return true;
}
bool NDL_DirectMedia_DL_IsInitialized(void) { return dl_inited; }
void NDL_DirectMedia_DL_Finalize(void) { say("DL_Finalize"); }

const char *NDL_DirectMediaGetError(void) { return err; }

int NDL_DirectMediaSetWindowId(const char *id) {
    if (!dl_inited) return bad("SetWindowId before DL_Initialize");
    say("SetWindowId %s", id ? id : "(null)");
    snprintf(window_id, sizeof window_id, "%s", id ? id : "");
    window_set = 1;
    return 0;
}

int NDL_DirectMediaInit(const char *app_id) {
    if (!dl_inited) return bad("Init before DL_Initialize");
    if (!window_set) return bad("Init before SetWindowId (the library would make an SDL window of its own)");
    say("Init %s", app_id ? app_id : "(null)");
    if (env_is("FAKE_NDL_FAIL", "init")) {
        snprintf(err, sizeof err, "init refused");
        return -1;
    }
    inited = 1;
    return 0;
}

static void *events(void *arg) {
    (void)arg;
    usleep((useconds_t)env_int("FAKE_NDL_LOAD_MS", 20) * 1000);
    if (cb) cb(22, 1, "true");
    usleep(10000);
    if (cb) cb(4, 0, "{\"frameRate\":\"120/1\",\"hdrType\":\"HDR10\"}");
    usleep(20000);
    if (!env_is("FAKE_NDL_NO_PLAYING", "1") && cb) cb(26, 1, "true");
    return NULL;
}

int NDL_DirectMediaLoad(struct data_info *info, load_cb callback) {
    if (!inited) return bad("Load before Init");
    if (info->audio.type == 1)
        say("Load %dx%d type %d audio PCM %s %s %s rate %d", info->video.width, info->video.height, info->video.type, info->audio.pcm.format,
            info->audio.pcm.layout, info->audio.pcm.channelMode, info->audio.pcm.sampleRate);
    else if (info->audio.type == 3)
        say("Load %dx%d type %d audio OPUS %d ch %.1f kHz", info->video.width, info->video.height, info->video.type, info->audio.opus.channels,
            info->audio.opus.sampleRate);
    else say("Load %dx%d type %d audio %d", info->video.width, info->video.height, info->video.type, info->audio.type);
    if (info->audio.type == 1 && info->audio.pcm.sampleRate == 0) return bad("PCM sample rate 0 (bypass) takes the pipeline down");
    if (env_is("FAKE_NDL_FAIL", "load")) {
        snprintf(err, sizeof err, "load refused");
        return -1;
    }
    if (env_is("FAKE_NDL_FAIL", "opus") && info->audio.type == 3) {
        snprintf(err, sizeof err, "audio type 3 (Opus) is not supported");
        return -1;
    }
    audio_type = info->audio.type;
    audio_channels = info->audio.type == 1 ? (!strcmp(info->audio.pcm.channelMode, "mono") ? 1 : !strcmp(info->audio.pcm.channelMode, "6-channel") ? 6 : 2)
                                           : info->audio.type == 3 ? info->audio.opus.channels : 0;
    cb = callback;
    loaded = 1;
    last_pts = last_apts = -1;
    pthread_t t;
    pthread_create(&t, NULL, events, NULL);
    pthread_detach(t);
    return 0;
}

int NDL_DirectMediaSetAppState(int state) {
    say("SetAppState %d", state);
    if (env_is("FAKE_NDL_FAIL", "state")) return -1;
    foreground = state == 0;
    return 0;
}

int NDL_DirectVideoSetArea(int l, int t, int w, int h) {
    say("SetArea %d %d %d %d", l, t, w, h);
    return 0;
}

int NDL_DirectVideoSetHDRInfo(struct hdr_info h) {
    if (!loaded) return bad("SetHDRInfo before Load");
    say("SetHDRInfo primaries %u,%u %u,%u %u,%u white %u,%u lum %u,%u cll %u,%u vui %u,%u,%u", h.v[0], h.v[1], h.v[2], h.v[3], h.v[4], h.v[5], h.v[6],
        h.v[7], h.v[8], h.v[9], h.v[10], h.v[11], h.v[12], h.v[13], h.v[14]);
    return 0;
}

int NDL_DirectVideoSetFrameDropThreshold(int threshold) {
    if (!loaded) return bad("SetFrameDropThreshold before Load");
    say("SetFrameDropThreshold %d", threshold);
    return 0;
}

int NDL_DirectVideoFlushRenderBuffer(void) {
    if (!loaded) return bad("FlushRenderBuffer before Load");
    say("FlushRenderBuffer");
    return 0;
}

int NDL_DirectVideoGetRenderBufferLength(int *len) {
    const char *e = getenv("FAKE_NDL_QUEUE");
    if (e) {
        *len = atoi(e);
        return 0;
    }
    long long now = ns();
    int n = 0;
    for (int i = 0; i < 64; i++)
        if (recent_ns[i] && now - recent_ns[i] < 8000000LL) n++;
    *len = n > 4 ? 4 : n;
    return 0;
}

int NDL_DirectAudioGetAvailableBufferSize(int *avail) {
    *avail = 65536;
    return 0;
}

int NDL_DirectVideoPlay(void *buf, unsigned size, long long pts) {
    if (!loaded) return bad("Play before Load");
    int every = env_int("FAKE_NDL_FULL_EVERY", 0);
    if (every) {
        /* counted by units offered, not by units taken: a refused unit the feeder retries is the same offer (same PTS) */
        if (pts != offered_pts) {
            offered_pts = pts;
            offered++;
            full_n = 0;
        }
        if (offered % every == 0) {
            if (env_is("FAKE_NDL_FULL_STICKY", "1") || !full_n) {
                if (!full_n) say("FULL offer %d (%s)", offered, env_is("FAKE_NDL_FULL_STICKY", "1") ? "sticky" : "once");
                full_n++;
                return -1;
            }
        }
    }
    const unsigned char *b = buf;
    int annexb = size > 4 && b[0] == 0 && b[1] == 0 && (b[2] == 1 || (b[2] == 0 && b[3] == 1));
    int obu = size > 2 && ((b[0] >> 3) & 0xF) == 2;      /* a temporal delimiter opens an AV1 temporal unit */
    if (!annexb && !obu) bad("a unit that starts with neither an Annex B start code nor a temporal delimiter");
    if (pts <= last_pts) bad("a PTS that does not rise");
    last_pts = pts;
    played++;
    if (getenv("FAKE_NDL_VIDEO_DUMP")) {
        FILE *d = fopen(getenv("FAKE_NDL_VIDEO_DUMP"), "ab");
        if (d) {
            fwrite(buf, 1, size, d);
            fclose(d);
        }
    }
    recent_ns[recent_i++ % 64] = ns();
    if (foreground) shown++;
    else {
        dropped++;
        if (dropped == 1) say("DROPPED units until SetAppState(FOREGROUND): the real library logs 'Video feed in the background state' and shows nothing");
    }
    if (played <= 3 || played % 120 == 0 || env_is("FAKE_NDL_VERBOSE", "1"))
        say("Play #%d size %u pts %lld head %02x%02x%02x%02x%02x%02x", played, size, pts, b[0], b[1], b[2], b[3], b[4], b[5]);
    int errAfter = env_int("FAKE_NDL_ERROR_AFTER", 0);
    if (errAfter && played == errAfter && cb) cb(18, 2331, "play error");
    if (env_is("FAKE_NDL_BUFFER_EVENTS", "1") && cb) {
        if (played % 100 == 0) cb(45, 0, "{\"bufferFull\":true}");
        if (played % 100 == 10) cb(44, 0, "");
    }
    return 0;
}

int NDL_DirectAudioPlay(void *buf, unsigned size, long long pts) {
    if (!loaded) return bad("AudioPlay before Load");
    if (pts <= last_apts) bad("an audio PTS that does not rise");
    last_apts = pts;
    if (audio_type == 1 && (size == 0 || size % (unsigned)(2 * audio_channels))) bad("a PCM unit that is not whole S16 frames");
    if (audio_type == 3 && size > 1275 * 3) bad("an Opus unit larger than any packet");
    audio_played++;
    if (getenv("FAKE_NDL_AUDIO_DUMP")) {
        FILE *d = fopen(getenv("FAKE_NDL_AUDIO_DUMP"), "ab");
        if (d) {
            fwrite(buf, 1, size, d);
            fclose(d);
        }
    }
    if (audio_played <= 2 || audio_played % 50 == 0 || env_is("FAKE_NDL_VERBOSE", "1")) say("AudioPlay #%d size %u pts %lld", audio_played, size, pts);
    return 0;
}

int NDL_DirectMediaUnload(void) {
    say("Unload");
    loaded = 0;
    cb = NULL;
    return 0;
}

int NDL_DirectMediaQuit(void) {
    say("Quit played=%d shown=%d dropped=%d audio=%d", played, shown, dropped, audio_played);
    inited = 0;
    window_set = 0;
    foreground = 0;
    last_pts = last_apts = -1;
    played = shown = dropped = audio_played = 0;
    return 0;
}
