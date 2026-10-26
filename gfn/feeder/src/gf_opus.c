#include "gf_opus.h"

#include <stdio.h>
#include <stdlib.h>

#include "opus.h"

struct gf_opus { OpusDecoder *dec; unsigned channels; };

gf_opus *gf_opus_open(unsigned rate, unsigned channels, char *err, size_t cap) {
    if (channels < 1 || channels > 2) {
        snprintf(err, cap, "the PCM fallback decodes mono and stereo, not %u channels", channels);
        return NULL;
    }
    int rc = 0;
    OpusDecoder *dec = opus_decoder_create((opus_int32)rate, (int)channels, &rc);
    if (!dec) {
        snprintf(err, cap, "opus_decoder_create(%u Hz, %u ch): %s", rate, channels, opus_strerror(rc));
        return NULL;
    }
    gf_opus *d = calloc(1, sizeof *d);
    if (!d) {
        opus_decoder_destroy(dec);
        snprintf(err, cap, "out of memory");
        return NULL;
    }
    d->dec = dec;
    d->channels = channels;
    return d;
}

int gf_opus_decode(gf_opus *d, const uint8_t *packet, size_t n, int16_t *pcm, int max_frames) {
    return opus_decode(d->dec, packet, (opus_int32)n, pcm, max_frames, 0);
}

void gf_opus_close(gf_opus *d) {
    if (!d) return;
    opus_decoder_destroy(d->dec);
    free(d);
}
