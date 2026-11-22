/* Diagnostics-only timing capture for one 10 s window (docs/research/decoded-vrr-4k.md, "upstream timing capture").
 *
 * One record per VIDEO message: the engine's wire key exactly as sent, when the whole message had arrived, the first play() call's entry and the
 * last one's return, how many attempts, the exact player argument and NDL's own return. Nothing here changes a timestamp, an ACCEPT or a DROP.
 *
 * The UI arms it by writing `vrr-capture.arm` in the app's data directory (rustui/app/src/game/capture.rs): `vrr-capture 1`, a nonce, the output
 * directory and the LumaGame build id, one per line. The ring is allocated when a load finds that file (never on the frame path), is written
 * once and never wraps: a full ring counts what it omits. When the window ends it passes to a detached thread that writes
 * `<nonce>-feeder.json` and frees it, so the feeder's loop does no file I/O. Disarmed, the frame path costs a few well-predicted tests.
 *
 * Everything is CLOCK_MONOTONIC (gf_ns). The sender's media clock is not known here and is not claimed.
 */
#ifndef GF_CAPTURE_H
#define GF_CAPTURE_H

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "gf_annexb.h"
#include "gf_log.h"
#include "gf_pipe.h"
#include "gf_proto.h"

#define GF_CAP_CAPACITY 2048
#define GF_CAP_WINDOW_NS 10000000000LL
#define GF_CAP_EPOCHS 8

typedef struct {
    int64_t wire_key;           /* gf_media_head.pts_ns as the engine sent it */
    int64_t recv_ns;            /* the whole VIDEO message was in hand */
    int64_t first_call_ns;      /* entry of the first play() attempt; 0: none was made (held, refused before the pipeline, not loaded) */
    int64_t last_return_ns;     /* return of the last attempt; 0 as above */
    int64_t player_ms;          /* the last play() argument, as gf_ndl.c passed it; -1 if none */
    uint32_t epoch, seq;
    int32_t queue, wait_us;     /* render queue before the feed; time on a full buffer */
    int32_t play_rc;            /* NDL's own return from the last attempt; INT32_MIN if none */
    uint16_t attempts;
    uint8_t reason;             /* 0 accepted, else GF_DROP_* */
    uint8_t key;
    uint32_t bytes;
    uint32_t pad[3];
} gf_cap_rec;
_Static_assert(sizeof(gf_cap_rec) == 80, "a capture record is 80 bytes");

typedef struct {
    int on;                     /* a ring is filling */
    gf_cap_rec *ring;
    gf_cap_rec *cur;            /* the record of the VIDEO message being fed, NULL when none or not capturing */
    uint32_t count, epoch;      /* epoch: the feeder's load ordinal */
    int64_t start_ns;           /* the first record's receipt; 0 until then */
    uint64_t nonce, omitted, resets;
    char dir[400], build_id[72];
    char arm_path[480];         /* where the UI's arm request is: the app's data directory; empty: never armed */
    uint32_t loads;             /* loads seen by the feeder */
    struct { uint32_t epoch; int64_t load_ns; } epochs[GF_CAP_EPOCHS];
    uint32_t n_epochs;
    unsigned codec, width, height, fps;
} gf_cap;

/* The values a finished capture needs, handed to the writer thread with the ring. */
typedef struct {
    gf_cap_rec *ring;
    uint32_t count;
    gf_cap meta;
    const char *ended;          /* why the window did not run its course, or NULL */
} gf_cap_job;

static const char *gf_cap_codec(unsigned c) { return c == GF_CODEC_H264 ? "H264" : c == GF_CODEC_H265 ? "H265" : c == GF_CODEC_AV1 ? "AV1" : "unknown"; }

static void gf_cap_num(char *buf, size_t cap, long long v, int present) {
    if (present) snprintf(buf, cap, "%lld", v);
    else snprintf(buf, cap, "null");
}

/* Fills `out` with the JSON text; returns its length, or -1 if `cap` bytes were not enough. */
static int gf_cap_json(const gf_cap_job *j, const char *ended, char *out, size_t cap) {
    const gf_cap *m = &j->meta;
    uint64_t no_play = 0;
    for (uint32_t i = 0; i < j->count; i++) no_play += j->ring[i].attempts == 0;
    size_t n = 0;
#define EMIT(...) do { int w_ = snprintf(out + n, cap - n, __VA_ARGS__); if (w_ < 0 || (size_t)w_ >= cap - n) return -1; n += (size_t)w_; } while (0)
    EMIT("{\"format\":\"lumagame-vrr-pts-capture\",\"version\":1,\"producer\":\"feeder\",\"clock\":\"CLOCK_MONOTONIC\",\"nonce\":%llu,",
         (unsigned long long)m->nonce);
    int incomplete = m->omitted > 0 || ended || m->start_ns == 0;
    EMIT("\"complete\":%s,\"incomplete\":[", incomplete ? "false" : "true");
    int first = 1;
    if (m->omitted) { EMIT("\"ring full: records omitted\""); first = 0; }
    if (m->start_ns == 0) { EMIT("%s\"no video message reached the feeder\"", first ? "" : ","); first = 0; }
    if (ended) { EMIT("%s\"%s\"", first ? "" : ",", ended); }
    EMIT("],\"header\":{\"build_id\":\"%s\",\"codec\":\"%s\",\"width\":%u,\"height\":%u,\"fps\":%u,\"rtp_clock_hz\":null,"
         "\"frame_pacing\":null,\"window_ns\":%lld,\"start_ns\":%lld,\"capacity\":%d,",
         m->build_id, gf_cap_codec(m->codec), m->width, m->height, m->fps, (long long)GF_CAP_WINDOW_NS, (long long)m->start_ns, GF_CAP_CAPACITY);
    EMIT("\"epochs\":[");
    for (uint32_t i = 0; i < m->n_epochs; i++) EMIT("%s{\"epoch\":%u,\"load_ns\":%lld}", i ? "," : "", m->epochs[i].epoch, (long long)m->epochs[i].load_ns);
    EMIT("],\"counts\":{\"omitted\":%llu,\"evicted\":0,\"unmatched\":%llu,\"resets\":%llu,\"stored\":%u},",
         (unsigned long long)m->omitted, (unsigned long long)no_play, (unsigned long long)m->resets, j->count);
    EMIT("\"definitions\":{\"omitted\":\"records not stored because the ring was full\","
         "\"evicted\":\"the feeder keeps no match table: always 0\","
         "\"unmatched\":\"video messages that never reached a play() call (held, not loaded, refused first)\","
         "\"resets\":\"loads after the first, flush messages and holds for a recovery anchor\"},"
         "\"unavailable\":[\"decoder completion\",\"selected-frame id\",\"release\",\"optical times\"]},");
    EMIT("\"fields\":[\"epoch\",\"seq\",\"wire_key_ns\",\"bytes\",\"key\",\"recv_ns\",\"first_call_ns\",\"last_return_ns\",\"attempts\",\"player_ms\","
         "\"play_rc\",\"queue\",\"wait_us\",\"reason\"],\"records\":[");
    for (uint32_t i = 0; i < j->count; i++) {
        const gf_cap_rec *r = &j->ring[i];
        char first_call[24], last_return[24], player[24], rc[24];
        gf_cap_num(first_call, sizeof first_call, r->first_call_ns, r->first_call_ns != 0);
        gf_cap_num(last_return, sizeof last_return, r->last_return_ns, r->last_return_ns != 0);
        gf_cap_num(player, sizeof player, r->player_ms, r->player_ms >= 0);
        gf_cap_num(rc, sizeof rc, r->play_rc, r->play_rc != INT32_MIN);
        EMIT("%s[%u,%u,%lld,%u,%u,%lld,%s,%s,%u,%s,%s,%d,%d,%u]", i ? "," : "", r->epoch, r->seq, (long long)r->wire_key, r->bytes, r->key,
             (long long)r->recv_ns, first_call, last_return, r->attempts, player, rc, r->queue, r->wait_us, r->reason);
    }
    EMIT("]}\n");
#undef EMIT
    return (int)n;
}

static void *gf_cap_writer(void *arg) {
    gf_cap_job *j = arg;
    size_t cap = 4096 + (size_t)j->count * 200;
    char *text = malloc(cap);
    int n = text ? gf_cap_json(j, j->ended, text, cap) : -1;
    char path[520], tmp[530];
    snprintf(path, sizeof path, "%s/%llu-feeder.json", j->meta.dir, (unsigned long long)j->meta.nonce);
    snprintf(tmp, sizeof tmp, "%s.tmp", path);
    int ok = 0;
    if (n > 0) {
        mkdir(j->meta.dir, 0700);
        int fd = open(tmp, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
        if (fd >= 0) {
            size_t off = 0;
            while (off < (size_t)n) {
                ssize_t w = write(fd, text + off, (size_t)n - off);
                if (w < 0 && errno == EINTR) continue;
                if (w <= 0) break;
                off += (size_t)w;
            }
            ok = off == (size_t)n && fsync(fd) == 0;
            close(fd);
            ok = ok && rename(tmp, path) == 0;
            if (!ok) unlink(tmp);
        }
    }
    if (ok) gf_say("timing capture: %s (%u records%s)", path, j->count, j->meta.omitted ? ", INCOMPLETE" : "");
    else gf_say("timing capture: could not write %s: %s", path, n < 0 ? "too large" : strerror(errno));
    free(text);
    free(j->ring);
    free(j);
    return NULL;
}

/* Ends the capture: the ring and the figures go to a detached writer thread. `ended` names why the window did not run its course, or NULL. */
static void gf_cap_close(gf_cap *c, const char *ended) {
    if (!c->on) return;
    c->on = 0;
    c->cur = NULL;
    if (!c->start_ns) {
        /* No video message came: nothing to write, and the request stays armed for the next load. */
        free(c->ring);
        c->ring = NULL;
        gf_say("timing capture: no video message arrived, nothing written");
        return;
    }
    gf_cap_job *j = malloc(sizeof *j);
    if (!j) {
        free(c->ring);
        c->ring = NULL;
        return;
    }
    j->ring = c->ring;
    j->count = c->count;
    j->meta = *c;
    j->ended = ended;
    c->ring = NULL;
    pthread_t t;
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setdetachstate(&a, PTHREAD_CREATE_DETACHED);
    if (pthread_create(&t, &a, gf_cap_writer, j) != 0) {
        gf_say("timing capture: no thread to write it");
        free(j->ring);
        free(j);
    }
    pthread_attr_destroy(&a);
}

/* A load: arms from the UI's request if there is one and none is running, otherwise notes the new epoch of the running capture. Off the frame path. */
static void gf_cap_load(gf_cap *c, const gf_load *l, int64_t now) {
    c->loads++;
    if (c->on) {
        c->resets++;
    } else {
        if (!c->arm_path[0]) return;
        FILE *fp = fopen(c->arm_path, "re");
        if (!fp) return;
        char head[32] = "", nonce[32] = "", dir[sizeof c->dir] = "", build[sizeof c->build_id] = "";
        int ok = fgets(head, sizeof head, fp) && fgets(nonce, sizeof nonce, fp) && fgets(dir, sizeof dir, fp) && fgets(build, sizeof build, fp);
        fclose(fp);
        head[strcspn(head, "\n")] = nonce[strcspn(nonce, "\n")] = dir[strcspn(dir, "\n")] = build[strcspn(build, "\n")] = 0;
        char *end = NULL;
        unsigned long long id = strtoull(nonce, &end, 10);
        if (!ok || strcmp(head, "vrr-capture 1") || !nonce[0] || *end || dir[0] != '/' || !build[0]) return;
        char path[520];
        snprintf(path, sizeof path, "%s/%llu-feeder.json", dir, id);
        if (access(path, F_OK) == 0) return;            /* this request has its file */
        gf_cap_rec *ring = calloc(GF_CAP_CAPACITY, sizeof *ring);
        if (!ring) return;
        c->ring = ring;
        c->count = 0;
        c->start_ns = 0;
        c->omitted = c->resets = 0;
        c->n_epochs = 0;
        c->nonce = id;
        snprintf(c->dir, sizeof c->dir, "%s", dir);
        snprintf(c->build_id, sizeof c->build_id, "%s", build);
        c->on = 1;
        gf_say("timing capture armed: request %llu", id);
    }
    c->epoch = c->loads;
    c->codec = l->codec;
    c->width = l->width;
    c->height = l->height;
    c->fps = l->fps;
    if (c->n_epochs < GF_CAP_EPOCHS) {
        c->epochs[c->n_epochs].epoch = c->epoch;
        c->epochs[c->n_epochs].load_ns = now;
        c->n_epochs++;
    }
}

/* A VIDEO message has arrived whole. Returns its record, or NULL (window over, ring full: counted, never waited for). */
static gf_cap_rec *gf_cap_begin(gf_cap *c, int64_t now, int64_t wire_key, int key, size_t bytes) {
    if (!c->start_ns) c->start_ns = now;
    else if (now - c->start_ns >= GF_CAP_WINDOW_NS) {
        gf_cap_close(c, NULL);
        return NULL;
    }
    if (c->count >= GF_CAP_CAPACITY) {
        c->omitted++;
        return NULL;
    }
    gf_cap_rec *r = &c->ring[c->count];
    memset(r, 0, sizeof *r);
    r->epoch = c->epoch;
    r->seq = c->count++;
    r->wire_key = wire_key;
    r->recv_ns = now;
    r->player_ms = -1;
    r->play_rc = INT32_MIN;
    r->queue = -1;
    r->key = (uint8_t)key;
    r->bytes = (uint32_t)bytes;
    return r;
}

/* The feeder's loop: a window whose video stopped still ends and is written. */
static inline void gf_cap_tick(gf_cap *c, int64_t now) {
    if (c->on && c->start_ns && now - c->start_ns >= GF_CAP_WINDOW_NS) gf_cap_close(c, NULL);
}

#endif
