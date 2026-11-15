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

int main(int argc, char **argv) {
    assert(argc == 2);
    setenv("FAKE_NDL_MANUAL_EVENTS", "1", 1);
    setenv("FAKE_NDL_LOG", "/dev/null", 1);
    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    assert(lib);
    *(void **)(&state) = dlsym(lib, "FakeNDLState");
    *(void **)(&event) = dlsym(lib, "FakeNDLEvent");
    *(void **)(&reset) = dlsym(lib, "FakeNDLReset");
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
    close_all(&f);
    free(client.out);
    puts("policy_test: 7 rules passed (injected clock, fake NDL)");
    return 0;
}
