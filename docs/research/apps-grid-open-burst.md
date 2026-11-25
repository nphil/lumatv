# Apps-grid bursts at 120 Hz (2026-11-25)

The gate's `apps-grid-open` window holds two whole-screen changes, and every flip the compositor counts as dropped in the
463b41b3 rings (`caba1`-`caba4`: 8 flips, 3 on the first and 5 on the second) is the second or third flip of one of them:

| burst | commit | frames | owed | GPU done per frame |
|---|---|---|---|---|
| the library opens | about 560 operations, one motion | 94 / 94 / 94 layers, 129-132 draws | 1.93-2.00 Mpx | 16.5-19.5 ms (after 200-400 ms idle), then 8.4-8.9 |
| Back, Home returns | 60 operations | 94 / 95 / 95 layers, 129-133 draws | 2.07 Mpx | 9.0, then 9-10, then 10.7-11.6 |

A refresh is 8.33 ms, so three back-to-back frames of 9 ms each cannot all flip on time; the third (or the second after the
open's long first frame) flips 16.7 ms after the one before.

The frames are three because the ring of three pages owes each change three times. `kms::frames` copies what a page owes from
the page drawn last only over Home and in calm frames (round 27: Home's walks dropped frames with copies, and their owed areas
are 0.8-1.3 Mpx), so with nothing over Home the library's open and Home's return draw 94 layers three times where one frame of
drawing and two blits give the same pixels.

## Change

A page also copies when the copy (what it owes minus what this frame changes) covers at least 75 % of the panel. The apps-grid
walk frames owe at most 1.31 Mpx (63 %); the two bursts owe 93-100 %. Frames 2 and 3 of a burst become one blit plus what the
frame changes: 1.6 ms by the set's model (1.5 ms + 0.085 ms per layer, a blit counting as one layer) against 9.6 ms for 94
layers. The pixels are the same: a 1:1 blit of the page the last frame drew. The first frame of each burst stays a full draw
(9.6 ms by the model).

The host mirrors the rule (`LUMATV_PAGE_COPY=0` switches both off) and compares every frame with the same frame drawn whole.

## The 0.3 ms between 463b41b3 and b4041d2b

The source difference (4b931a87 to baac5b10 in `rustui/render` and `compositor`) is accounting (`count()` for `touch()`, upload
bytes, the target-cost cell, the ring's ` tgts` suffix) and the creation of render targets (cleared by the GPU). The two bursts
make no render target. The same 95-layer frames take 4.8-9.1 ms of GPU done between opens of one build, so a median of 8.8
against 9.1 ms is inside that spread: no per-frame regression is in the diff.

## Measured on the set (compositor 4626ca8d, main ad6b324d)

Six apps-grid runs of 3 cycles at 120 Hz dropped 2, 2, 0, 1, 1, 2 (b4041d2b 1, 0, 2, 1; 463b41b3 0, 1, 0, 0). Settings-scroll
dropped 0, 0, 0, home-widgets 0, weather-day 0. Burst flips that came more than 10 ms after the one before fell from 8 of 85
(463b41b3) to 6 of 137. GPU done over all burst flips: median 8.5 ms, max 15.2 ms (18.1 ms for the first frame after idle, which
the gate does not judge).

The open burst is fixed: frame 1 draws 84 layers (1.93 Mpx), frames 2 and 3 draw 7 layers (23,296 px) and flip on time. What
remains is Home's return. Its first frame draws 85 layers, 123 draws, 2.07 Mpx and takes 11.0 ms of GPU done against 8.33 ms;
that is the late flip in the first run's ring (11.0 and 10.9 ms in its two closes). Its frames 2 and 3 still draw the whole
screen: their own damage is 0.83 Mpx (40 %), so the copy would cover 60 %, under the 75 % line, while the page owes all of it.

Not done:

- Judging the line by the area a page owes (at least 75 %; an apps-grid walk owes at most 63 %) instead of by the copy. Frames
  2 and 3 of the return would draw about 5 ms each by the model [INFERENCE], but frame 1, the late flip, is untouched.
- Frame 1 of the return under 8.3 ms: a Home page kept aside while a full-screen surface is open (a copy, 1.6 ms, against 9.6 ms)
  or less glass (changes the look). This is what 0 drops needs.
- Fewer pages in the ring, a shorter settle, a cheaper or lower-resolution blur: each changes the look or the timing.

Untested: the capture diff on the set. The host compares every frame with the same frame drawn whole: 0 differ.
