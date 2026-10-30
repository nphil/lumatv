# Round 27 on the set (forge main 93496e61, includes 35023b49 and the take-in-ahead commit)

Installed with grant cf1e58be (1 use; 1 left). Pop-ups off, receiver muted during the runs, `panel_refresh` 59.9 / 119.9 on every surface. Compositor log: no
`page copy: GL error`. Toast raised through `ctl shell show toast`. The 60 Hz run was the quick set at the gate's three cycles.

Dropped (the compositor's own count), `burst_over20` in brackets:

| surface | 60 Hz | 120 Hz | 120 Hz, `LUMATV_FLOOR=0 PAGE_COPY=0 AHEAD=0` | round 26 (60 / 120) |
|---|---|---|---|---|
| home-dpad | 0 (1) | 0 (0) | | 0 / 1 |
| quick | 0 (0) | 0 (0) | | 0 / 0 |
| picker | 0 (0) | 0 (0) | | 0 / 0 |
| toast | 0 (0), burst_end 252 ms | 0 (0), burst_end 237 ms | | not measured |
| settings-scroll | 5 (0) | 9 (11) | | 3 / 5 |
| apps-grid | 1 (1) | 29 (3) | 3 (3) | 2 / 13 |
| home-widgets | 6 (4) | 28 (19) | 3 (1) | 6 / 24 |

The merge bound (35023b49) did not bring apps-grid and home-widgets back at 120 Hz: 29 and 28 against round 23's 3 and 3. All three switches off together (the
old path, run from the compositor's environment, then put back) gives 3 and 3. On the same walks the ring's render time is p50/p90 1.06/2.17 ms (apps-grid)
and 1.02/1.28 ms (home-widgets) with the new path against 0.74/0.92 and 0.75/0.95 ms with the old, and the draw-to-GPU time p90 8.1 and 5.9 ms against 4.8
and 5.3 ms (`rings/120-*-m1-*.ring.json`). Which of the three switches causes it was not separated.

settings-scroll is worse than round 26 at both rates (60 Hz 5, 120 Hz 9).

## Item 10: settings-scroll at 60 Hz, the frame after OK

Not fixed: 5 dropped (round 26: 3), 1-2 a cycle. In the ring of m1 (`rings/60-settings-scroll-m1.ring.json`) the frame after OK is still the one that misses
its refresh, flipping 33.4 ms after the last (`late 1 lost 1 missed anim`). Its pass is `P 7.7`, `9.6`, `10.8`, `10.9`, `10.4 ms in one pass` against `render`
2.3-2.7 ms (round 26: 7.0-7.2 against 2.8-3.0), so taking the commit in ahead did not shorten the pass: the pass is about 10 ms with 2.5 ms in the renderer, the rest is not
render. The frame draws 75-110 layers over 2.07 Mpx in one rectangle. `LUMATV_AHEAD=0` was not compared on its own.

## Reboot repro

Not run (skipped on the owner's instruction for the time left).

## End state

LumaTV compositor persistent, Home in front, 120 Hz (`ctl refresh 120`, 120 osd_irq/s), popups.on off, receiver 0.52 unmuted. The environment file
(`surface-manager.env`) is back to its three original lines.
