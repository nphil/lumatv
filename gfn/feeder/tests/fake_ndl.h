#ifndef FAKE_NDL_H
#define FAKE_NDL_H

struct fake_ndl_state {
    int video, audio, primes, hdr, flushes, audio_type, channels, violations;
    long long video_pts, audio_pts;
};
struct fake_ndl_state FakeNDLState(void);
void FakeNDLEvent(int type, long long num);
void FakeNDLReset(void);

#endif
