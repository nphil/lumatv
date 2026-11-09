# Surfaces, phase 1 (LumaTV)

Key filter, focus rule, wallpaper tint, saved-first cards, token source and component kit,
startup warm-up, the frame gate, and the preload trim. Measured on the LG C4 (webOS 10.2.1),
2026-10-25. Plan: `docs/PLAN.md` (Rules every surface meets, Phases 1 and 8).

## Resume here

State at 2026-10-25 02:20 EDT. Milestones 1, 2 and 3 of 5 are built, deployed, verified on the
set and pushed. Milestones 4 (warm-up and the frame gate) and 5 (preload trim and the Home-return
re-measure) are not started: the run reached its request budget. Section 4 holds their designs,
worked out against the code, so a restart can begin building.

- Done: 1 (key filter, OK on release, focus rest), 2 (wallpaper tint, saved-first weather and
  news), 3 (token source, Manrope, component kit).
- Next: 4 (warm-up, then the frame gate in `docs/research/perf`), then 5. Both need the set to
  themselves with the screen on (`tvpower powerOn {"reason":"remoteKey"}`, `turnOnScreen`, and
  `getPowerState` reads Active).
- The set: `webglass` and `systemui` are on (they are how Settings, Notification Centre and Quick
  Settings get the theme and Manrope), the kit gallery is off, and the user's theme file holds the
  wallpaper-motion and idle-dim settings plus `kitGallery: false`. The dock order is as Nitin had it
  (a held OK picks a tile up and an OK release drops it, so test scripts must not leave one picked
  up). It is left in Active Standby (`tvpower powerOff {"reason":"remoteKey"}`).
- Tools used (not in the repo): a paramiko wrapper for the set, and `docs/research/perf/kb.py`
  (key injector with trace markers, copy it to `/tmp/kb.py` on the set).
- Deploy loop: `sh tools/build-ipk.sh`, push the ipk to `/tmp/lumatv.ipk`, `luna-send -n 4 -f
  luna://com.webos.appInstallService/dev/install '{"id":"org.nphil.lumatv","ipkUrl":"/tmp/lumatv.ipk","subscribe":true}'`,
  wait until the app directory matches the build (the install is asynchronous: an `apply` run
  one second after the call staged the previous files), then `lumatv apply` (13 s, restarts the
  compositor). `lumatv fps on|off` is the cheap way to restart it.
- Tests: `TZ=America/New_York bun test tests/` (41) and `python3 -m unittest discover -s tests
  -p 'test_*.py'` (11).

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
  and drop at once when it leaves. The name pill follows the same rule; the dock's accent glow
  moves with the highlight. The dim of unfocused cards this note once described is gone (`frames-120-result.md`). `FocusLift.qml` is the shared
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

Wallpaper tint (`payload/compositor/lumatv/Tint.js`, LumaHome.qml, Dock.qml, `tools/tokens.py`):

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
  restores it for the same picture before the first sample lands, and `tokens.py` reads the accent
  from it. The accent keeps the tint's hue at saturation 0.55 (0.35 on light glass) and lightness 0.74
  (0.42): `#9898e1` for the shipped wallpaper.
- Feeding the tokens: `accent: "tile"` (shipped) keeps each tile's own colour and gives a tile with
  no plate colour (white or black plate) the wallpaper accent where it had a fixed lavender;
  `accent: "wallpaper"` uses it for every tile and for the web menus' accent (`tokens.py css`:
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

Same-session gate, M1 build against M2 builds (idle, dpad and Quick Settings workloads, 2 runs;
two builds of M2 because the same code differed by 0.3 ms between runs):

| | M1 | M2 | M2 again |
|---|---|---|---|
| idle p90 render | 2.41 ms | 2.71 ms | 3.04 ms |
| dpad p90 render | 2.67 ms | 2.77 ms | 2.94 ms |
| Quick Settings open, close p90 | 2.98, 3.18 ms | 3.22, 3.44 ms | 3.08, 3.85 ms |
| idle CPU, GPU | 44, 102 ms/s | 46, 103 ms/s | |
| dpad CPU | 118 ms/s | 119 ms/s | |

Dropped-frame share and the count of frames over 20 ms did not move. The difference between M1
and M2 is inside the run-to-run spread, so it is read as noise. Quick Settings is LG's patched
panel and was already over 3 ms at the Phase 1 baseline (2.7-2.9 open, 3.4-3.9 close); it is
rebuilt in Phase 3.

Milestone 3, after token, Manrope and kit deployment (`m3`, 2 runs each): idle p90 was 2.61 and
2.60 ms; D-pad p90 2.87 and 2.86 ms. Home therefore stays under 3 ms and within the same-session
M2 range. Quick Settings open p90 was 2.94, 3.10, 2.66, 3.00 and 3.21 ms; close p90 3.20, 3.30,
3.05, 3.13, 3.70 and 3.51 ms. That is not a regression against its pre-existing 2.7-2.9 / 3.4-3.9
spread, but it remains the explicit exception to the under-3-ms rule until the own panel in Phase 3.
The post-run log has no `ShaderEffectSource` report and no crash report; milestone 4 still needs to
turn that log observation into a gate assertion.

## 3. Token source, Manrope, component kit

Token source:

- `payload/theme/theme.json` (version 2) holds every value a surface may use, in Lucent's names:
  colour roles for both materials (`canvas`, `sheet`, `scrim`, `wallScrim`, `focus`, `focusInk`,
  `trackOff`, `accent`, `accentInk` and the semantic roles), `font`, the type roles (`display` 92,
  `title` 40, `body` 28, `label` 26, `caption` 24, `clock`, `temp`), `radii`, `space`, `target` 64,
  `row` 80, `focus`, `states` and `motion` (`focusMs`, `cardMs`, `layerMs`, `pressMs`, `restMs`,
  `holdMs`). The old `radius` and `tileRadius` became `radii.card` and `radii.tile`, and a
  material's `scrim` list became `wallScrim` (`scrim` is now the modal scrim colour).
- LumaHome no longer carries literal defaults. `stage_luma` stages the shipped files as
  `theme.default.json` and `layout.default.json` beside the QML, LumaHome reads them through
  `TextFile` and merges the user's files over them (`mergeDeep`: objects merge, arrays and scalars
  replace). `theme.py minimize` runs at `apply` and cuts the user's file to what differs from the
  shipped one, plus drops the retired keys, so what was never changed follows an update. On the
  set Nitin's file went from a full copy of the old defaults to his wallpaper-motion edits.
- `tools/tokens.py <shipped> <user> qml|css|js` replaces `palette.py`. `qml` is the settings-sheet
  palette Quick Settings is patched with and was byte-identical to `palette.py` for the same theme;
  `css` writes `--lu-*` (Lucent's tokens.css names, plus `--lu-weight-*`), the Sandstone `--sand-*`
  block and the menu scrim; `js` writes `theme/Tokens.js` for QML in other processes. A sheet
  below 7:1 against the ink is darkened as before. `lumatv.css` has no colour of its own now and a
  test fails on any `var(--...)` the generator does not write.

Manrope:

- Four static faces (400, 500, 600, 700) are cut from the pinned variable font (Manrope 4.505,
  `generators/fonts/`, sha256 in `payload/fonts/Manrope.SOURCE`) by `generators/gen-fonts.py`,
  named family "Manrope" with the weight as the style. They render pixel-identically to the 4.504
  statics they replace (13 sizes compared). Inter is removed from the repo and from the data dir.
- QML loads them from `theme.font.files`. A Repeater delegate must be an `Item` that contains the
  FontLoader: a FontLoader delegate fails with "Delegate must be of Item type".
- Web: `lumatv.css` declares the four faces and `font-family` is `var(--lu-font)`. Verified on the
  set: the Settings app draws in Manrope on the token colours.
- Quick Settings: `systemui_patch.py` rewrites `LanguageStyle.qml` so every LG family (16) maps to
  Manrope, LG's regular to Medium and its light to Regular, and `getFontWeight` to match. Verified on
  the set after `systemui on` and a MENU key. Manrope is a tenth wider than the face LG sized the
  tiles for, so the tile title and subtitle are set at 0.9 of LG's size; the Picture Mode tile's
  subtitle (FILMMAKER MODE) still marquees when the tile is focused, which is LG's own handling of
  text that does not fit.

Component kit (`payload/compositor/lumatv/kit/`):

- `Lu.qml` is a QML singleton (`singleton Lu 1.0 Lu.qml` in the kit's `qmldir`); LumaHome feeds
  it through four `Binding`s (theme, `isLight`, wall accent, backdrop). Components take every value
  from it, so a number that is not in `theme.json` does not exist. `LuText`, `LuHalo`, `LuRow`,
  `LuToggle`, `LuSlider`, `LuButton`, `LuSegmented`, `LuChip` and `FocusLift` use it; each kit file
  has `import "."`, files in `lumatv/` use `import "kit"`.
- Qt 5.12 has no `Qt.color()`; `Lu.asColor` uses `Qt.lighter(value, 1.0)`. `Text.style` collides
  with a custom property of that name, so `LuText` takes `spec`.
- `KitGallery.qml` shows every component on one sheet in the current material
  (`lumatv theme set '{"kitGallery":true}'`, Back closes it); a `Loader` in LumaHome keeps it
  unloaded otherwise. Home's own `Text` items were not converted to `LuText`: Home keeps its
  values from the approved mock and the kit is for the surfaces built next. The dock's glow is
  the one Home piece that now uses a kit component (`LuHalo`); the static Home regions are
  pixel-identical before and after.
- Checked on the set: the gallery in dark and light glass, D-pad focus through rows, toggle,
  slider (adjusts), segmented control and buttons; Home text unchanged and Manrope loaded. The log
  scan has no LumaHome or kit QML error. Quick Settings logs a stock
  `Controllers/ScreenOffInterfaces/Interfaces.qml:52` TypeError when its response callback sees a
  changed foreground-app array; the font patch does not touch that file. `tests/kit.test.js` checks
  that every `Lu.` member used is declared, that every kit file is in the `qmldir` and that the
  shipped theme has every key `Lu` asks for.

## 4. Not done: the designs for milestones 4 and 5

Milestone 4, startup warm-up. The costs to remove come from LumaShow's measured lessons
(`docs/research/lumashow-lessons.md`, glyph-cache fills and shader compiles on first use); they are
[INFERENCE] for LumaTV until the first-frame times below are measured here. Blank to first present
after a compositor restart is 4.6-4.8 s today.

- Fonts: at LumaHome start, walk its item tree once and collect every distinct (weight, pixel
  size) among the `Text` items, so the list cannot drift from the surfaces. Draw the glyph set
  (digits, Latin, punctuation, degree sign) at each in `Text` items placed offscreen but inside
  the window's bounds with `opacity` 0.01, for one frame, then remove them. Do the same for the
  kit's type roles from `theme.type`.
- Shaders: one tiny `ShaderEffect` per distinct fragment source in use (the glass material, the
  wallpaper, the halo), drawn offscreen for a frame; a `ShaderEffectSource` that is built by
  showing a layer is the cost the gate forbids, so the effect sources in use are created hidden
  with `live: false` and `recursive: false` and updated once.
- Icons: load every dock and status PNG into the Image cache (`cache: true`) with
  `asynchronous: false` once, in the same warm frame.
- Frame gate in `docs/research/perf`: a table, per surface, of the key sequence that shows it and
  thresholds on the windows `analyse.py` already reports: p90 `anim_render_ms` under 3, no
  animated frame over 20 ms, first frame within 50 ms of the key (needs `@name` markers from
  `kb.py` in the trace), fully open by 180 ms, `.sys` RSS flat over 10 open and close cycles.
  Surfaces to list now: Home (idle, D-pad, sweep), Quick Settings, kit gallery. The gate exits
  non-zero on a miss, so it can run before every commit that changes a surface.
- Check after building: Home's time from `lumatv fps on` restart to first present, and the first
  D-pad sweep's worst frame, with and without the warm-up in the same session.

Milestone 5, preload trim.

- The key is `com.webos.service.preloadmanager.supportAppList` in configd; the target value
  keeps Netflix and drops YouTube, LG Channels and the browser. Read it first, keep the original
  in `/var/lib/lumatv/preload.orig` so `revert` restores it, and set it through configd only (no
  rootfs write). Re-send `setPreloadPolicy` at every wake, as Glasshouse already does.
- Measure with `run.py --tag preload --repeat 2 --workloads apphome`, against the same workload
  with the list restored, in one session. The baseline to beat is in HANDOFF: Home-return gaps of
  50-117 ms and 250-920 ms after a 450 MB hog, unchanged by the Home stub. Record whether the
  gaps are gone; a gap that remains with a smaller preload set is page-in cost, not preload
  contention, and the answer is then the heap shim and a smaller resident Home.

Findings kept from this run:

- `Canvas.drawImage` paints nothing on this firmware (Image item, PNG at 1:1 or scaled, icon).
  Reading pixels back works through `grabToImage` and XPM (`Tint.js`).
- `Qt.color` does not exist in Qt 5.12; `tintedMaterial` had called it, so `tint.mix` had never
  run before this build.
- A key handler with no `try` at all lets the key through on this firmware; the filter's catch is
  for the log and the pair record.
