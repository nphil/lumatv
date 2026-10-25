# Surfaces, phase 1 (LumaTV)

Key filter, focus rule, wallpaper tint, saved-first cards, token source and component kit,
startup warm-up, the frame gate, and the preload trim. Measured on the LG C4 (webOS 10.2.1),
2026-10-25. Plan: `docs/PLAN.md` (Rules every surface meets, Phases 1 and 8).

## Resume here

State at 2026-10-25 01:45 EDT. Milestones 1 and 2 of 5 are built, deployed and verified on the
set and pushed; the gate numbers for 2 are measured after the set comes back from the compositor
work (section 2).

- Done: 1 (key filter, OK on release, focus rest), 2 (wallpaper tint, saved-first weather and news).
- Next: 3 (kit and tokens), 4 (warm-up and the frame gate), 5 (preload trim and the Home-return
  re-measure).
- The set: Active, LumaHome in front, dock order restored after test keys moved a tile (the
  list is `Apps, Netflix, PlxNative, Apple TV, GeForce NOW, Crunchyroll, ...`; a held OK picks a
  tile up and an OK release drops it, so test scripts must not leave one picked up).
- Tools used (not in the repo): a paramiko wrapper for the set, and `docs/research/perf/kb.py`
  (key injector with trace markers, copy it to `/tmp/kb.py` on the set).
- Deploy loop: `sh tools/build-ipk.sh`, push the ipk to `/tmp/lumatv.ipk`, `luna-send -n 4 -f
  luna://com.webos.appInstallService/dev/install '{"id":"org.nphil.lumatv","ipkUrl":"/tmp/lumatv.ipk","subscribe":true}'`,
  wait until the app directory matches the build (the install is asynchronous: an `apply` run
  one second after the call staged the previous files), then `lumatv apply` (13 s, restarts the
  compositor).

## 1. Key filter, OK on release, focus rest

What changed, in `payload/keyfilter/lumatv.js`, `payload/compositor/lumatv/` and the theme:

- The filter takes a key only while Home is in front, LumaHome is answering (a heartbeat from
  its 1 Hz timer, under 3 s old) and its handler takes the press. Any exception in the filter or
  the handler answers NextPolicy and logs one line per 5 s.
- Presses and releases are paired. A release goes where its press went, so an OK pressed on
  Home and released after an app has opened ends at LumaHome (which acts on it only while Home
  is up), and a release whose press went to an app is never taken. Repeats follow their press.
  Unit tests: `bun test tests/` (13 cases).
- OK acts on release. A press only starts the 700 ms hold timer (`motion.holdMs`); held, it picks
  a dock tile up. On any other layer a long press is an ordinary one (before, it did nothing).
  A held OK and a picked-up tile are cleared when Home is hidden or shown.
- Focus rest: the highlight (tint, edge, sheen, rim) follows every key press at once; lift,
  scale and shadow depth animate only after the focused item has rested `motion.restMs` (150 ms)
  and drop at once when it leaves. The name pill and the dim of unfocused cards follow the
  same rule; the dock's accent glow moves with the highlight. `FocusLift.qml` is the shared
  piece; `Glass.qml` takes `focusMix` (highlight) and `liftMix` (shadow depth) separately.

Checked on the set:

- Fault injection, key RIGHT on Home: with the normal filter no key reaches WebAppMgr; with a
  throw inside the filter, the key reaches it (`WaylandKeyboard::OnKey key:106` press and
  release) and one `[LumaKeys] the filter failed` line is logged; a second key within 5 s adds no
  line. A handler with no `try` at all also lets the key through on this firmware, so the catch
  is for the log, the pair record and the explicit answer, not a rescue from a swallowed key.
- OK: a press does nothing until the release; the release launches the tile; neither reaches
  the app that opens. Held 1.2 s on a tile, the tile is picked up and stays picked up after the
  release; Back puts it back; leaving Home while it is picked up and returning leaves the order
  and the dock untouched.
- Frames, `run.py` workloads `sweep` (held RIGHT 1.5 s, LEFT 1.5 s, UP, DOWN) and `dpad` (HA keys
  at 350 ms), 2 runs each, HEAD against this build:

  | | frames | GPU busy | compositor CPU | render p90 |
  |---|---|---|---|---|
  | sweep before | 224, 220 | 1760, 1775 ms | 805, 690 ms | 3.36, 2.95 ms |
  | sweep after | 150, 157 | 1195, 1271 ms | 584, 502 ms | 3.11, 2.78 ms |
  | dpad before | 324, 322 | 2597, 2589 ms | 984, 970 ms | 2.92, 2.91 ms |
  | dpad after | 243, 263 | 1955, 2110 ms | 731, 801 ms | 2.40, 2.66 ms |

  A sweep now draws one frame per key press. The harness's `dropped` count rose (6, 1 to 14, 6 on
  the sweep): it counts an animated frame that follows an idle gap, and the gap before the lift
  that starts 150 ms after the last key is one; the render time of those frames is 0.7-3.7 ms,
  so none was late to render. The gate in step 4 counts only frames inside a running animation.

## 2. Wallpaper tint and saved-first cards

Wallpaper tint (`payload/compositor/lumatv/Tint.js`, LumaHome.qml, Dock.qml, `tools/palette.py`):

- LumaHome decodes the wallpaper a second time at 64x36, grabs that item at 32x18, saves the grab
  as XPM (the one text image format Qt writes) and reads it back through `TextFile`. Canvas could
  not be used: on this firmware `Canvas.drawImage` paints nothing, for an Image item, for a
  loaded PNG at 1:1 or scaled, and for an icon PNG alike (a `fillRect` on the same canvas read back
  correctly). The stock 16x9 luma sampler therefore always read zero, so the "auto" glass was
  always dark; it now reads the wallpaper (luma 0.293 for the shipped one, dark; with
  `materialThreshold` set to 0.2 the Home turns light).
- Each channel is cut into 8 levels (512 bins), transparent and near-black pixels (every channel
  under 32) are skipped, the most common bin wins and a tie goes to the lower bin. Shipped
  wallpaper: `#505070`, 24 % of the picture, which a PIL cross-check of the same rule reproduces;
  the parser reads the XPM the set wrote to the same value on the workstation.
- A newer wallpaper cancels a running sample: every decoded copy that becomes ready bumps a
  generation and a grab from an older one is dropped. [INFERENCE: not provoked on the set, the 2 s
  theme poll never changes the wallpaper twice inside one decode.]
- The result is `theme/tint.json` (`wallpaper`, `tint`, `accent`, `light`, `luma`); a cold start
  restores it for the same picture before the first sample lands, and `palette.py` reads the accent
  from it. The accent keeps the tint's hue at saturation 0.55 (0.35 on light glass) and lightness 0.74
  (0.42): `#9898e1` for the shipped wallpaper.
- Feeding the tokens: `accent: "tile"` (shipped) keeps each tile's own colour and gives a tile with
  no plate colour (white or black plate) the wallpaper accent where it had a fixed lavender;
  `accent: "wallpaper"` uses it for every tile and for the web menus' accent (`palette.py css`:
  `#A79BF5` becomes `#9898e1`); `tint.mix` (0 shipped) moves the focused glass toward the tint
  (a focused card went from rgb 55,59,87 to 82,85,103 at 0.8).

Saved-first cards (`Saved.js`, WeatherWidget.qml, NewsWidget.qml):

- Both cards write `cache/weather.json` and `news.json` (the helper's schema, same directory the
  helper uses) after each fetch and read them at creation, which is compositor start.
  `Saved.js` carries the helper's policy and its documents: 15 min fresh, stale to 6 h, expired
  after 6 h or stamped more than 5 min ahead, retries after 2 and 10 min then every 30. Its
  weather document is checked against the helper's fixture and expected rows in `tests/saved.test.js`.
  A newer file than what is on screen replaces it (when Home is shown), so the helper can fetch.
- On the set, with a restart (the compositor restarts at `fps on`) and the panel watched from
  blank to first present, the documents edited to a distinctive place and headline:
  stale (20 min old): the first presented frame shows SAVEDVILLE 88 and the saved headline with its
  hero picture, the refetch replaces the weather within 3 s and the headline later;
  fresh (2 min old): shown, not refetched (the file is unchanged 143 s later);
  expired (7 h old): not shown, both cards empty in the first frame, real data by 12 s.
  Panel blank to first present: 4.6, 4.8 and 4.8 s (stale, fresh, expired).
- A request that never answers is cut at 30 s, a zip the lookup cannot place is an error (the
  old card fell back to Atlanta), and a reading for another zip or unit is not used.
