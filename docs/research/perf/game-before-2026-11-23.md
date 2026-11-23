# LumaGame frame gate: before the UI rework (2026-11-23)

The installed LumaGame UI (`org.nphil.lumagame`, today's UI) measured on the TV at 120 Hz (panel 119.9 Hz) with the new `game-*` surfaces of `gate.py`:

    gate.py --surfaces game-home,game-library,game-page,game-settings --cycles 3 --refresh 120

Verdict PASS, no captures discarded. Full output: `game-before-2026-11-23.txt`. The LumaTV compositor was live; its `ctl` was reached through `/tmp/lumatv` links to `/var/lib/lumatv/compositor`, since the bundle's copy is gone after a reboot.

| surface | what it walks | first_frame_ms | p90_render_ms | p90_window_ms | burst_over20 | dropped |
|---|---|---|---|---|---|---|
| `game-home` | nav tabs, hero buttons, shelves down and up, a held key | 18.6 | 1.42 | 1.43 | 0 | 0 |
| `game-library` | grid row across, held Down and Up twice | 18.4 | 1.57 | 1.72 | 0 | 0 |
| `game-page` | OK on a grid tile, Back (act on release) | 43.3 | 1.46 | 1.49 | 0 | 0 |
| `game-settings` | Settings from the nav tab, list, rail through six sections, Back | 8.2 | 1.43 | 1.46 | 0 | 0 |

Thresholds are the gate's: render p90 under 4.17 ms and window p90 under 5.56 ms at 120 Hz, first frame at most 50 ms, burst end switched off for walks.

Read with care:

- `burst_over20` and `dropped` count only intervals after a frame of a compositor motion. The app's own drawing is not flagged as one, so both are 0 by construction here. The trace-level counts per `open` window are from the run's JSON (not kept): `game-home` 21 to 23 intervals over 20 ms in the burst (30 to 32 over the window), `game-library` 6, `game-settings` 0 to 1 in the burst (16 to 17 over the window, 1 to 7 dropped). The rework should be compared on these, not on the gate's 0.
- `game-page` first frame of 43.3 ms is close to the 50 ms limit; the first cycle was 21 ms.
- The late frames in the log are all "not the kernel": the Settings rail walk shows 83 to 100 ms intervals between keys that redraw the whole page.
- `rss_growth_mb` covers one cycle only with `--cycles 3` (two are warm-up).

The surfaces assume a populated library and a text size of 100 %; none presses OK on a play button or changes a setting.
