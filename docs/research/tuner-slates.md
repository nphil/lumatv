# Tuner slates: what Plex's live grab tolerates

Nitin asked on 2026-11-05 for a player watching the Live TV Tuner to see why it waits (tuning, a weak signal, the TV taking its tuner)
instead of Plex's Error 16 or a frozen picture, done inside the MPEG-TS the tuner serves with Plex unmodified. This note holds what
Plex's transcoder was measured to accept, and the stream design that follows (`helper/tuner/src/splice.rs`, `slated.rs`).

## Method

Streams were served at their own pace from the Unraid host to Plex's own transcoder in the `plex` container, run with the live grab's
command line as Plex ran it on 2026-11-05 (`-fflags +discardcorruptts+fillwallclockdts -probesize 20000000 -i http://…/auto/v2.1`,
every stream copied, `-f ssegment -segment_time 1`), then a second-stage libx264/AAC transcode of the segments as a session does; each
in Plex's own form and with `-copyts`. Sources: an SD capture of RF 32 (program 4, 720x480i, one AC-3) and an HD one of 2.1
(1280x720p59.94, two AC-3). Error 16 itself: Plex's log says "Stream ends prematurely" and then cannot create the transcode session.

## Findings

| What changes where the slate meets the station | Plex's grab |
|---|---|
| times moved onto one continuous clock | every frame kept, segments continuous |
| times left as the station sent them (a jump forward or back) | corrected in Plex's form; with `-copyts` 78 segments, holes of 38,252 s and 95,343 s, frames lost |
| the discontinuity indicator set | no effect |
| the station on PIDs and a program of its own | **the station is dropped**: its streams appear after the start and are ignored, only the slate's frames come out |
| 1920x1080 slate, then 720x480 station (times moved) | 404 of 404 frames, stage 2 clean |
| 5.1 silence, then stereo | clean |
| a slate of two intra pictures a second | clean |
| an AC-3 frame cut at a splice | the decoder takes the silence after it for the rest of the frame: 0.26 to 0.35 s of the following frames lost |

Station streams: audio is private stream 1 (`0xBD`), a PES of 6 to 10 AC-3 frames (0.2 to 0.3 s) that does not end with a frame; a
picture is stamped 0.46 to 1.0 s ahead of the clock; GOPs are open. The demodulator's `/proc/lg/demod/signal_status` gives lock, packet
errors and MSE; 10·log10(21504 / MSE) read 28 to 29 dB on RF 32, 18 to 20 dB on RF 29 (weak) and nothing on RF 35 (unlocked).

## The stream that follows

One program throughout: its own PAT and PMT (the station's descriptors carried), the picture on 0x101 and the sounds from 0x102, one
clock. The station's times are moved so its first whole picture (an I picture with its sequence header; an open GOP's leading B
pictures left out) decodes a frame after the slate's last, and its counters carry on from the slate's with its own losses kept. Each
station PES goes on only once the next one starts, so a PES the stream breaks off in is left out; the AC-3 frame the last one ends in is
made whole from what came (zeros for the rest), and the first sound after a slate starts at a frame. The slate's silence keeps half a
second behind its picture and is filled up to the station's first sound.

The slate is MPEG-2 Main Profile in the channel's size, rate and shape once seen (1920x1080i at 29.97 before): an intra picture every
half second, flat blocks of DC only, with all-skipped P pictures between (300 to 800 bytes each), and a 32 kb/s stereo AC-3 frame of
silence on each sound. ffmpeg decodes the encoder's pictures at 43.4 dB PSNR (luma) against the drawn frame.

## The engine through Plex's grab

The `lumatuner` engine run on the captures with a fake tuner (a 1.5 s tune, a 3 s gap, a burst of lost packets), its output through the
grab and stage 2 (2026-11-05, final build):

| Run | Result |
|---|---|
| SD, tuning slate then the channel | continuous picture and sound, nothing back in time, stage 2 clean |
| SD, 2 s of heavy packet loss (a tenth of the stream's packets) | the slate stood in and the channel came back; no hole in the sound, the picture's only from its lost pictures (0.6 s) |
| HD 720p, a 3 s gap (1080i slate before, the channel's shape after), twice | continuous picture and sound, every frame kept, nothing back in time, both sounds; no corrupt packet in the grab, stage 2 clean |

Not measured: the stream on the set through Plex's own session (the TV was in use by other work), and E-AC-3 channels (their frames
are not read; the slate's silence is plain AC-3, which E-AC-3 decoders take).
