#define main feeder_entry
#include "../src/feeder.c"
#undef main
#include <assert.h>
#include <dlfcn.h>
#include "fake_ndl.h"

static int64_t now = 500000000000LL;
int64_t gf_ns(void) { return now; }
static struct fake_ndl_state (*state)(void);
static void (*event)(int, long long);
static void (*reset)(void);
static const char *(*header)(void);
static struct feeder f;
static struct client client;
static const uint8_t picture[] = {0, 0, 0, 1, 0x26, 1, 0x80, 0x42};
static const uint8_t packet[] = {0xfc, 0x12, 0, 0};

static gf_load load_for(int audio, int hdr) {
    return (gf_load){.codec = 2, .width = 1280, .height = 720, .fps = 120, .audio = (unsigned)audio,
        .channels = 2, .sample_rate = 48000, .hdr = (unsigned)hdr, .hdr_info = generator_hdr(), .drop_threshold = -1, .pts_unit = GF_PTS_NS};
}

static void start(int audio, int hdr) {
    stop_pipeline(&f);
    reset();
    now += 1000000000LL;
    gf_load l = load_for(audio, hdr);
    char err[300];
    assert(start_pipeline(&f, &l, err, sizeof err) == 0);
    client.out_len = 0;
}

static void complete(void) {
    event(GF_EV_LOAD_COMPLETED, 1);
    maintain_pipeline(&f, NULL);
    assert(f.loaded);
}

static int video(int64_t pts, int key) {
    int q, wait;
    return feed_video(&f, picture, sizeof picture, pts, key, &q, &wait);
}

static int messages(int type) {
    int count = 0;
    for (size_t at = 0; at < client.out_len;) {
        gf_frame h;
        memcpy(&h, client.out + at, sizeof h);
        count += h.type == type;
        at += sizeof h + h.length;
        assert(at <= client.out_len);
    }
    return count;
}

static void timestamps_are_load_relative_ceiled_and_non_decreasing(void) {
    start(GF_AUDIO_OPUS, 0);
    assert(f.load_ns == now);
    assert(video(now + 1000001, 1) == 0 && state().video_pts == 2);
    assert(video(now - 1, 0) == 0 && state().video_pts == 2);
    assert(video(now + 2000001, 0) == 0 && state().video_pts == 3);
    assert(state().violations == 0);
}

static void an_audio_plane_is_primed_through_the_load_window(void) {
    start(GF_AUDIO_NONE, 0);
    assert(state().audio_type == 3 && state().channels == 2);
    assert(state().primes == 8 && state().audio_pts == 40);
    now += PRIME_INTERVAL_NS;
    maintain_pipeline(&f, NULL);
    assert(state().primes == 12 && state().audio_pts == 60);
    complete();
    now += PRIME_INTERVAL_NS;
    maintain_pipeline(&f, NULL);
    assert(state().primes == 12);
    assert(feed_audio(&f, packet, sizeof packet, now) < 0 && state().audio == 0);
}

static void audio_waits_for_load_and_uses_the_player_clock(void) {
    start(GF_AUDIO_OPUS, 0);
    assert(feed_audio(&f, packet, sizeof packet, INT64_MAX) < 0 && state().audio == 0);
    complete();
    now += 40000001;
    assert(feed_audio(&f, packet, sizeof packet, INT64_MIN) == 0 && state().audio_pts == 41);
    assert(feed_audio(&f, packet, sizeof packet, INT64_MAX) == 0 && state().audio_pts == 41);
    now += 20000000;
    assert(feed_audio(&f, packet, sizeof packet, 0) == 0 && state().audio_pts == 61);
    assert(state().violations == 0);
}

static void loss_holds_for_an_anchor_without_flushing(void) {
    start(GF_AUDIO_OPUS, 0);
    handle_message(&f, &client, GF_FLUSH, NULL, 0);
    report_player(&f, &client);
    handle_message(&f, &client, GF_FLUSH, NULL, 0);
    report_player(&f, &client);
    assert(messages(GF_KEYFRAME) == 1 && state().flushes == 0);
    assert(video(now, 0) == GF_DROP_HOLD && state().video == 0);
    assert(video(now, 1) == 0 && state().video == 1);
    complete();
    handle_message(&f, &client, GF_FLUSH, NULL, 0);
    report_player(&f, &client);
    assert(video(now + 1, 0) == GF_DROP_HOLD);
    assert(video(now + 1, 1) == 0);
    assert(messages(GF_KEYFRAME) == 2 && state().flushes == 0 && state().violations == 0);
}

static void hdr_is_applied_after_ingest_once_per_change_never_for_sdr(void) {
    start(GF_AUDIO_NONE, 1);
    gf_hdr hdr = generator_hdr();
    handle_message(&f, &client, GF_HDR, (const void *)&hdr, sizeof hdr);
    complete();
    assert(state().hdr == 0);
    assert(video(now, 1) == 0 && state().hdr == 1);
    handle_message(&f, &client, GF_HDR, (const void *)&hdr, sizeof hdr);
    assert(video(now + 1, 0) == 0 && state().hdr == 1);
    hdr.maxContentLightLevel++;
    handle_message(&f, &client, GF_HDR, (const void *)&hdr, sizeof hdr);
    assert(state().hdr == 2);
    start(GF_AUDIO_NONE, 0);
    handle_message(&f, &client, GF_HDR, (const void *)&hdr, sizeof hdr);
    assert(video(now, 1) == 0 && state().hdr == 0 && state().violations == 0);
}

static void watchdog_and_fatal_errors_latch_until_a_new_load(void) {
    start(GF_AUDIO_OPUS, 0);
    now += LOAD_TIMEOUT_NS - 1;
    maintain_pipeline(&f, NULL);
    assert(!f.error);
    now++;
    maintain_pipeline(&f, NULL);
    report_player(&f, &client);
    report_player(&f, &client);
    assert(f.error && messages(GF_ERROR) == 1);
    assert(video(now, 1) == GF_DROP_FATAL && state().video == 0);
    event(GF_EV_LOAD_COMPLETED, 1);
    event(GF_EV_PLAYING, 1);
    maintain_pipeline(&f, NULL);
    assert(!f.loaded && !f.playing);
    start(GF_AUDIO_OPUS, 0);
    complete();
    event(GF_EV_INT_ERROR, 600);
    event(GF_EV_INT_ERROR, 600);
    maintain_pipeline(&f, NULL);
    report_player(&f, &client);
    assert(f.error && messages(GF_ERROR) == 1);
    assert(video(now, 1) == GF_DROP_FATAL);
    assert(feed_audio(&f, packet, sizeof packet, now) < 0);
    start(GF_AUDIO_OPUS, 0);
    assert(!f.error && video(now, 1) == 0);
}

static void identity_is_a_build_setting(void) {
#if GF_UI_GAME
    assert(!strcmp(DEFAULT_APP_ID, "org.nphil.lumagame"));
    assert(!strcmp(DEFAULT_SOCKET, "/tmp/org.nphil.lumagame/feed.sock"));
    assert(!strcmp(GF_UI_PROGRAM, "bin64/lumagame-ui"));
#else
    assert(!strcmp(DEFAULT_APP_ID, "org.nphil.lumatv.gfn"));
    assert(!strcmp(DEFAULT_SOCKET, "/tmp/lumatv-gfn/feed.sock"));
    assert(!strcmp(GF_UI_PROGRAM, "bin64/lumatv-gfn-app"));
#endif
}

static void frame_records_are_opt_in_bounded_and_measured(void) {
    int sv[2];
    assert(socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0);
    f.chan = sv[0];
    frames_note(&f, now, 0, 0);
    assert(f.frames.fed == 0);
    const char *on = "{\"kind\":\"frames\",\"on\":true}\n";
    assert(write(sv[1], on, strlen(on)) == (ssize_t)strlen(on));
    chan_pump(&f);
    assert(f.frames.on);
    frames_note(&f, now, 1, 0);
    now += 8300000;
    frames_note(&f, now - 600000, 2, GF_DROP_FULL);
    assert(f.frames.samples[1].gap_us == 7700 && f.frames.samples[1].feed_us == 600);
    now += FRAME_WINDOW_NS;
    frames_send(&f, now);
    char line[2048] = {0};
    assert(recv(sv[1], line, sizeof line - 1, MSG_DONTWAIT) > 0);
    assert(strstr(line, "\"fed\":2,\"accepted\":1,\"dropped\":[1,0,0,0,0]"));
    assert(strstr(line, "\"samples\":[[-1,0,1,0],[7700,600,2,1]]"));
    frames_send(&f, now);
    assert(recv(sv[1], line, sizeof line, MSG_DONTWAIT) < 0 && errno == EAGAIN);
    for (int i = 0; i < FRAME_SAMPLES + 1; i++) frames_note(&f, now, 0, 0);
    assert(f.frames.count == FRAME_SAMPLES && f.frames.omitted == 1);
    frames_set(&f, 0);
    frames_note(&f, now, 0, 0);
    assert(!f.frames.on && !f.frames.fed);
    chan_close(&f);
    close(sv[1]);
}

/* One VIDEO message as the engine sends it. Returns the key the feeder acknowledged, -1 if it dropped the unit. */
static int64_t send_video(int64_t pts, int key) {
    uint8_t m[sizeof(gf_media_head) + sizeof picture];
    gf_media_head h = {.pts_ns = pts, .key = (uint8_t)key};
    memcpy(m, &h, sizeof h);
    memcpy(m + sizeof h, picture, sizeof picture);
    handle_message(&f, &client, GF_VIDEO, m, sizeof m);
    int64_t acked = -1;
    gf_frame answer;
    gf_accept a;
    if (client.out_len >= sizeof answer + sizeof a) {
        memcpy(&answer, client.out, sizeof answer);
        memcpy(&a, client.out + sizeof answer, sizeof a);
        if (answer.type == GF_ACCEPT) acked = a.pts_ns;
    }
    client.out_len = 0;
    return acked;
}

static char *capture_file(const char *dir, unsigned nonce) {
    char path[600];
    snprintf(path, sizeof path, "%s/out/%u-feeder.json", dir, nonce);
    for (int i = 0; i < 3000; i++) {            /* the writer is a detached thread; the rename makes the file appear whole */
        FILE *fp = fopen(path, "re");
        if (fp) {
            fseek(fp, 0, SEEK_END);
            long n = ftell(fp);
            rewind(fp);
            char *text = calloc((size_t)n + 1, 1);
            assert(text && fread(text, 1, (size_t)n, fp) == (size_t)n);
            fclose(fp);
            return text;
        }
        usleep(1000);
    }
    return NULL;
}

static void arm(const char *dir, unsigned nonce) {
    char path[600];
    snprintf(path, sizeof path, "%s/arm", dir);
    FILE *fp = fopen(path, "we");
    assert(fp);
    fprintf(fp, "vrr-capture 1\n%u\n%s/out\nbuild-abc123\n", nonce, dir);
    fclose(fp);
    snprintf(f.cap.arm_path, sizeof f.cap.arm_path, "%s", path);
}

static void the_timing_capture_is_armed_by_the_ui_bounded_and_written_off_the_frame_path(void) {
    char dir[] = "/tmp/gfcapXXXXXX";
    assert(mkdtemp(dir));
    gf_load l = load_for(GF_AUDIO_NONE, 0);
    reset();
    f.cap.arm_path[0] = 0;
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&l, sizeof l);
    complete();
    send_video(now, 1);
    assert(!f.cap.on && !f.cap.ring);           /* no request: nothing allocated, nothing recorded */

    /* A full ring omits and says the capture is incomplete; it never waits. */
    arm(dir, 1);
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&l, sizeof l);
    complete();
    assert(f.cap.on && f.cap.ring);
    for (int i = 0; i < GF_CAP_CAPACITY + 5; i++) {
        now += 1000000;
        send_video(now, i == 0);
    }
    assert(f.cap.count == GF_CAP_CAPACITY && f.cap.omitted == 5);
    const gf_cap_rec *r = &f.cap.ring[1];
    assert(r->attempts == 1 && r->play_rc == 0 && r->player_ms >= 0 && r->first_call_ns && r->last_return_ns >= r->first_call_ns && r->reason == 0);
    assert(r->wire_key == f.cap.ring[0].wire_key + 1000000 && r->seq == 1 && r->bytes == sizeof picture);
    now += GF_CAP_WINDOW_NS;
    gf_cap_tick(&f.cap, now);
    assert(!f.cap.on && !f.cap.ring);
    char *text = capture_file(dir, 1);
    assert(text);
    assert(strstr(text, "\"complete\":false") && strstr(text, "ring full: records omitted"));
    assert(strstr(text, "\"build_id\":\"build-abc123\"") && strstr(text, "\"codec\":\"H265\"") && strstr(text, "\"width\":1280,\"height\":720,\"fps\":120"));
    assert(strstr(text, "\"omitted\":5,\"evicted\":0,\"unmatched\":0,\"resets\":0,\"stored\":2048"));
    free(text);

    /* The request that has its file is not armed again; a new one is. A flush, a hold and a frame that never reached play() are counted. */
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&l, sizeof l);
    assert(!f.cap.on);
    arm(dir, 2);
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&l, sizeof l);
    complete();
    assert(f.cap.on);
    now += 1000000;
    send_video(now, 1);
    handle_message(&f, &client, GF_FLUSH, NULL, 0);
    now += 1000000;
    send_video(now, 0);                         /* held until a key: refused before play() */
    now += GF_CAP_WINDOW_NS;
    send_video(now, 1);                         /* past the window: closes the capture, is not recorded */
    assert(!f.cap.on);
    text = capture_file(dir, 2);
    assert(text);
    assert(strstr(text, "\"complete\":true") && strstr(text, "\"unmatched\":1,\"resets\":2,\"stored\":2"));
    assert(strstr(text, "\"codec\":\"H265\"") && strstr(text, "\"rtp_clock_hz\":null,\"frame_pacing\":null"));
    free(text);
    gf_cap_close(&f.cap, NULL);
    f.cap.arm_path[0] = 0;
    char cmd[700];
    snprintf(cmd, sizeof cmd, "rm -rf %s", dir);
    assert(system(cmd) == 0);
}
static void surround_header_and_load_validation(void) {
    stop_pipeline(&f);
    reset();
    gf_load_opus51 l = {.load = load_for(GF_AUDIO_OPUS, 0), .opus = {4, 2, {0, 1, 4, 2, 3, 5}}};
    l.load.channels = 6;
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(f.pipe_ready && f.has_opus51 && state().channels == 6);
    assert(header() && !strcmp(header(), "T3B1c0hlYWQBBgAAAAAAAAAAAQQCAAEEAgMF"));
    assert(state().primes == 8);
    const char *saved = header();
    complete();
    assert(!strcmp(saved, "T3B1c0hlYWQBBgAAAAAAAAAAAQQCAAEEAgMF"));
    stop_pipeline(&f);
    l.load.hdr = 1;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(f.pipe_ready && header() && !strcmp(header(), "T3B1c0hlYWQBBgAAAAAAAAAAAQQCAAEEAgMF"));
    stop_pipeline(&f);
    l.opus.mapping[0] = l.opus.mapping[1];
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(!f.pipe_ready && messages(GF_LOADED) == 1);
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l - 1);
    assert(!f.pipe_ready && messages(GF_LOADED) == 1);
    l.opus.mapping[0] = 0;
    l.opus.streams = 3;
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(!f.pipe_ready && messages(GF_LOADED) == 1);
    l.opus.streams = 4;
    l.load.sample_rate = 44100;
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(!f.pipe_ready && messages(GF_LOADED) == 1);
    l.load.sample_rate = 48000;
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&l, sizeof l);
    assert(!f.pipe_ready && messages(GF_LOADED) == 1);
    setenv("FAKE_NDL_FAIL", "opus", 1);
    client.out_len = 0;
    handle_message(&f, &client, GF_LOAD_OPUS51, (const uint8_t *)&l, sizeof l);
    assert(!f.pipe_ready && !f.pcm && messages(GF_LOADED) == 1);
    unsetenv("FAKE_NDL_FAIL");
    gf_load stereo = load_for(GF_AUDIO_OPUS, 0);
    handle_message(&f, &client, GF_LOAD, (const uint8_t *)&stereo, sizeof stereo);
    assert(f.pipe_ready && !header() && !f.has_opus51);
    stop_pipeline(&f);
    f.has_opus51 = 0;
}

static void surround_packets_preserve_speaker_identity(void) {
    void *lib = dlopen("libopus.so.0", RTLD_NOW);
    assert(lib);
    void *(*enc_create)(int, int, int, int, const unsigned char *, int, int *);
    int (*encode)(void *, const short *, int, unsigned char *, int);
    void (*enc_destroy)(void *);
    void *(*dec_create)(int, int, int, int, const unsigned char *, int *);
    int (*decode)(void *, const unsigned char *, int, short *, int, int);
    void (*dec_destroy)(void *);
    *(void **)(&enc_create) = dlsym(lib, "opus_multistream_encoder_create");
    *(void **)(&encode) = dlsym(lib, "opus_multistream_encode");
    *(void **)(&enc_destroy) = dlsym(lib, "opus_multistream_encoder_destroy");
    *(void **)(&dec_create) = dlsym(lib, "opus_multistream_decoder_create");
    *(void **)(&decode) = dlsym(lib, "opus_multistream_decode");
    *(void **)(&dec_destroy) = dlsym(lib, "opus_multistream_decoder_destroy");
    assert(enc_create && encode && enc_destroy && dec_create && decode && dec_destroy);
    const unsigned order[6] = {0, 2, 1, 4, 5, 3};
    const unsigned char mappings[2][6] = {{0, 4, 1, 5, 2, 3}, {0, 1, 2, 3, 4, 5}};
    const unsigned char lg[6] = {0, 1, 4, 5, 2, 3};
    for (int mode = 0; mode < 2; mode++) {
        int streams = mode ? 6 : 4, coupled = mode ? 0 : 2, error;
        unsigned char vorbis[6], data[8192];
        for (unsigned i = 0; i < 6; i++) vorbis[i] = mappings[mode][order[i]];
        short input[960 * 6], wave[960 * 6], output[960 * 6], wrong[960 * 6], default_out[960 * 6];
        for (int n = 0; n < 960; n++) for (int c = 0; c < 6; c++)
            input[n * 6 + c] = ((n / (7 + c * 3)) % 2 ? 1 : -1) * (3000 + c * 300);
        void *enc = enc_create(48000, 6, streams, coupled, mappings[mode], 2049, &error);
        assert(enc && !error);
        int bytes = encode(enc, input, 960, data, sizeof data);
        assert(bytes > 0);
        enc_destroy(enc);
        void *a = dec_create(48000, 6, streams, coupled, mappings[mode], &error); assert(a && !error);
        void *b = dec_create(48000, 6, streams, coupled, vorbis, &error); assert(b && !error);
        void *c = dec_create(48000, 6, streams, coupled, mappings[mode], &error); assert(c && !error);
        void *d = dec_create(48000, 6, streams, coupled, lg, &error); assert(d && !error);
        assert(decode(a, data, bytes, wave, 960, 0) == 960);
        assert(decode(b, data, bytes, output, 960, 0) == 960);
        assert(decode(c, data, bytes, wrong, 960, 0) == 960);
        assert(decode(d, data, bytes, default_out, 960, 0) == 960);
        int incorrect = 0, lg_incorrect = 0;
        for (int n = 0; n < 960; n++) for (int ch = 0; ch < 6; ch++) {
            assert(output[n * 6 + ch] == wave[n * 6 + order[ch]]);
            incorrect |= wrong[n * 6 + ch] != output[n * 6 + ch];
            lg_incorrect |= default_out[n * 6 + ch] != output[n * 6 + ch];
        }
        assert(incorrect && lg_incorrect);
        dec_destroy(a); dec_destroy(b); dec_destroy(c); dec_destroy(d);
        f.has_opus51 = 1;
        f.opus51 = (gf_opus51){(uint8_t)streams, (uint8_t)coupled, {0, 1, 2, 3, 4, 5}};
        gf_load l = load_for(GF_AUDIO_OPUS, 0); l.channels = 6;
        char err[300];
        assert(start_pipeline(&f, &l, err, sizeof err) == 0);
        a = dec_create(48000, 6, streams, coupled, vorbis, &error); assert(a && !error);
        assert(decode(a, f.prime_packet, (int)f.prime_size, wave, 960, 0) == 240);
        dec_destroy(a);
        stop_pipeline(&f);
    }
    f.has_opus51 = 0;
    dlclose(lib);
}


static void picture_only_starfish_skips_audio_queries(const char *ndl_path) {
    char path[4096], err[300];
    assert(snprintf(path, sizeof path, "%s", ndl_path) < (int)sizeof path);
    char *name = strrchr(path, '/');
    assert(name);
    assert((size_t)(name + 1 - path) + sizeof "libplayerAPIs.so.1" <= sizeof path);
    strcpy(name + 1, "libplayerAPIs.so.1");
    void *lib = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    assert(lib);
    int (*queries)(void);
    *(void **)(&queries) = dlsym(lib, "FakeSMPAudioQueries");
    assert(queries);
    gf_pipe *pipe;
    assert(gf_smp_ops.open_lib(&pipe, path, err, sizeof err) == 0);
    struct feeder status_feeder = {.pipe = pipe, .pipe_ready = 1, .ops = &gf_smp_ops};
    gf_pipe_opts opts = {0};
    gf_load l = load_for(GF_AUDIO_NONE, 0);
    gf_status status;
    int before = queries();

    assert(gf_smp_ops.start(pipe, "test.app", "window", &l, &opts, err, sizeof err) == 0);
    fill_status(&status_feeder, &status);
    assert(status.audio_avail == 0 && queries() == before);
    gf_smp_ops.stop(pipe);
    assert(gf_smp_ops.audio_avail(pipe) == -1 && queries() == before);

    l.audio = GF_AUDIO_OPUS;
    assert(gf_smp_ops.start(pipe, "test.app", "window", &l, &opts, err, sizeof err) == 0);
    fill_status(&status_feeder, &status);
    assert(status.audio_avail == 65536 && queries() == ++before);
    gf_smp_ops.stop(pipe);

    setenv("FAKE_SMP_FAIL", "load", 1);
    assert(gf_smp_ops.start(pipe, "test.app", "window", &l, &opts, err, sizeof err) == GF_PIPE_LOAD_FAILED);
    assert(gf_smp_ops.audio_avail(pipe) == -1 && queries() == before);
    unsetenv("FAKE_SMP_FAIL");
    gf_smp_ops.stop(pipe);

    l.audio = GF_AUDIO_NONE;
    assert(gf_smp_ops.start(pipe, "test.app", "window", &l, &opts, err, sizeof err) == 0);
    fill_status(&status_feeder, &status);
    assert(status.audio_avail == 0 && queries() == before);
    gf_smp_ops.stop(pipe);

    assert(gf_es_ops.start(pipe, "test.app", "window", &l, &opts, err, sizeof err) == 0);
    fill_status(&status_feeder, &status);
    assert(status.audio_avail == 0 && queries() == before);
    gf_es_ops.stop(pipe);
    gf_smp_ops.free(pipe);
    dlclose(lib);
}

static void a_released_load_is_not_queried(void) {
    gf_status status;
    start(GF_AUDIO_NONE, 0);
    fill_status(&f, &status);
    assert(status.audio_avail == 65536 && gf_ndl_ops.queued(f.ndl) >= 0);
    handle_message(&f, &client, GF_UNLOAD, NULL, 0);
    f.pipe_ready = 1;           /* what a status tick would see if anything kept the pipeline marked ready */
    fill_status(&f, &status);
    assert(status.audio_avail == 0 && gf_ndl_ops.queued(f.ndl) == -1);
    assert(state().violations == 0);
    f.pipe_ready = 0;
}

/* The sender's own time for the VIDEO that follows (the engine sends it only when its sender-time switch is on). */
static void sender_clock(int64_t key, uint32_t rtp, uint32_t flags) {
    gf_video_clock c = {.pts_ns = key, .rtp = rtp, .clock_hz = 90000, .flags = flags};
    handle_message(&f, &client, GF_VIDEO_CLOCK, (const void *)&c, sizeof c);
}

static void a_sender_stamp_is_the_players_timestamp_for_its_unit_only_and_the_key_stays_the_acknowledgement(void) {
    start(GF_AUDIO_OPUS, 0);
    /* without a stamp: the handover time, as always */
    assert(send_video(now + 3000000, 1) == now + 3000000 && state().video_pts == 3);
    /* the first stamped picture sits where its handover time puts it; the next follows the sender's 750 ticks (8.33 ms), not its 3 ms of handover time */
    sender_clock(now + 9000000, 5000, 0);
    assert(send_video(now + 9000000, 0) == now + 9000000 && state().video_pts == 9);
    sender_clock(now + 12000000, 5750, 0);
    assert(send_video(now + 12000000, 0) == now + 12000000 && state().video_pts == 18);
    /* a stamp is for the unit that follows it: one naming another key is ignored, and the unit after that has none */
    sender_clock(now + 1, 6500, 0);
    assert(send_video(now + 20000000, 0) == now + 20000000 && state().video_pts == 20);
    assert(send_video(now + 21000000, 0) == now + 21000000 && state().video_pts == 21);
    assert(f.vclock.epochs == 1 && !f.stamp_valid);
    handle_message(&f, &client, GF_VIDEO_CLOCK, (const void *)&f.stamp, sizeof f.stamp - 1);
    assert(!f.stamp_valid);                     /* a short message is no stamp */
    /* audio stays on the player's clock whatever the video is stamped with */
    complete();
    now += 40000001;
    assert(feed_audio(&f, packet, sizeof packet, 0) == 0 && state().audio_pts == 41);
    assert(state().violations == 0);
}

static void a_hold_a_reported_gap_and_a_load_each_end_the_sender_epoch_and_the_stamps_never_go_back(void) {
    start(GF_AUDIO_OPUS, 0);
    sender_clock(now + 1000000, 100000, 0);
    send_video(now + 1000000, 1);
    sender_clock(now + 5000000, 100750, 0);
    send_video(now + 5000000, 0);
    assert(state().video_pts == 10 && f.vclock.epochs == 1);
    /* a flush holds for an anchor: the unit that is not one is dropped (its stamp used up), and the recovery keyframe opens the next epoch, no earlier
     * than the last stamp */
    handle_message(&f, &client, GF_FLUSH, NULL, 0);
    sender_clock(now + 6000000, 101500, 0);
    assert(send_video(now + 6000000, 0) == -1 && state().video_pts == 10 && !f.stamp_valid);
    sender_clock(now + 7000000, 900000, 0);
    assert(send_video(now + 7000000, 1) == now + 7000000 && state().video_pts == 10 && f.vclock.epochs == 2);
    sender_clock(now + 8000000, 900750, 0);
    send_video(now + 8000000, 0);
    assert(state().video_pts == 19 && f.vclock.epochs == 2);
    /* a gap the engine reported, with no flush: the epoch restarts at that picture */
    sender_clock(now + 30000000, 5000000, GF_VCLOCK_GAP);
    send_video(now + 30000000, 0);
    assert(state().video_pts == 30 && f.vclock.epochs == 3);
    /* a load is a new timeline, and a stamp still waiting is dropped with the old one */
    sender_clock(now + 1, 1, 0);
    start(GF_AUDIO_OPUS, 0);
    assert(f.vclock.epochs == 0 && !f.vclock.open && !f.stamp_valid);
    sender_clock(now + 2000000, 77, 0);
    send_video(now + 2000000, 1);
    assert(state().video_pts == 2 && f.vclock.epochs == 1 && state().violations == 0);
}

int main(int argc, char **argv) {
    assert(argc == 2);
    setenv("FAKE_NDL_MANUAL_EVENTS", "1", 1);
    setenv("FAKE_NDL_LOG", "/dev/null", 1);
    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    assert(lib);
    *(void **)(&state) = dlsym(lib, "FakeNDLState");
    *(void **)(&event) = dlsym(lib, "FakeNDLEvent");
    *(void **)(&reset) = dlsym(lib, "FakeNDLReset");
    *(void **)(&header) = dlsym(lib, "FakeNDLHeader");
    assert(header);
    assert(state && event && reset);
    f.app_id = DEFAULT_APP_ID;
    f.lib = argv[1];
    f.chan = -1;
    snprintf(f.window_id, sizeof f.window_id, "_Window_Id_42");
    char err[300];
    assert(gf_ndl_ops.open_lib(&f.ndl, argv[1], err, sizeof err) == 0);
    timestamps_are_load_relative_ceiled_and_non_decreasing();
    an_audio_plane_is_primed_through_the_load_window();
    audio_waits_for_load_and_uses_the_player_clock();
    loss_holds_for_an_anchor_without_flushing();
    hdr_is_applied_after_ingest_once_per_change_never_for_sdr();
    watchdog_and_fatal_errors_latch_until_a_new_load();
    identity_is_a_build_setting();
    a_released_load_is_not_queried();
    frame_records_are_opt_in_bounded_and_measured();
    the_timing_capture_is_armed_by_the_ui_bounded_and_written_off_the_frame_path();
    surround_header_and_load_validation();
    picture_only_starfish_skips_audio_queries(argv[1]);
    surround_packets_preserve_speaker_identity();
    a_sender_stamp_is_the_players_timestamp_for_its_unit_only_and_the_key_stays_the_acknowledgement();
    a_hold_a_reported_gap_and_a_load_each_end_the_sender_epoch_and_the_stamps_never_go_back();
    close_all(&f);
    free(client.out);
    puts("policy_test: 15 rules passed (injected clock, fake NDL)");
    return 0;
}
