# Cameras in the Rust shell (kind `cameras`): result

The full-screen Cameras app of `docs/research/cameras-ui-spec.md` in `rustui/ui/src/cameras/`, drawn with the shell kit at
`system_z + z::CAMERAS` (+200). Switched per kind: `lumatv-compositor --shell-rust cameras` or `ctl shell rust +cameras`; the QML viewer
(`payload/compositor/lumatv/panels/Cameras.qml`) stays the fallback and is untouched.

## Resume here

Done and pushed to `forge/main` (see the commits below):

- Model, all host-tested (`cameras/*.rs`, 48 behaviour tests in `app_tests.rs` plus the unit tests of each module): `words` (clock, ages,
  refusal words), `data` (typed `cameras`, `cameras.live`, `cameras.events`, popup, recordings, ticks, clips, settings), `grid` (columns,
  layout, scroll window, keys; the name under a tile grows with the text), `strip` (span, rate, scrub steps, gaps, events), `sheet`,
  `options`, `pics` (the picture planner: pacing, tags `0x0CA0_0000..=0x0CAF_FFFF`), `view`, `app` (`App`: `Model` impl, `open_grid`,
  `open_full`, `open_events`, `close`, `take_intents`, `live_claim`).
- Surfaces (`cameras/surface/`): `grid`, `full` (hole, stills, chrome fade, note), `strip` (painted bar canvas, knob, bubble), `sheet`,
  `options`, `toast`, `pill`, `pic` (picture store, compositor-scaled layers), `root` (`CamerasSurface`), `common`.
- `cameras/host.rs` (`Cameras`: decodes the stills, joins model and surface) and the wiring in `shell.rs` (`Kinds.cameras`, feed, key,
  pointer, tick, report, state, geometry, lint, `cameras_asks`, `cameras()`).
- The grid's `Settings` button: the app closes and `Shell::cameras_asks` opens Settings at `CAMERAS_SETTINGS_PAGE` (`"cameras"`, the
  page RustSettings added); while Settings is open the app takes no key and no pointer sample.
- Video windows: `App::live_claim()` is the `camera-full` claim (`LiveClaim::request()`, id `CLAIM_ID`), posted by the shell's `Vwin`
  (HwPlanes2). The full view reads the grant back: `Shell::after` sets `App::set_window(Window::of(vwin.grant(CLAIM_ID)))`. Granted and
  not drawing yet: no hole (a hole over nothing is black); granted and placed: the hole as before; refused or taken back (anything but a
  switch that is `off`): the plane path goes on and a glyph-only cue (`triangle-alert` in a round pill beside the name pill) says so; no
  claim in force: as it always was. Tests: `app_tests.rs` (`a_refused_video_window_...`), `test_rust_cameras.py`
  (`test_a_refused_video_window_...`), scene `full-refused`.
- Kit additions: `z::CAMERAS`, `Flow::scroll_snap` / `drop_scroller`, `Button::icon_after` and glyph-name icons, the "dot" glyph, the
  muted rule in `Button::feed_lint` for ink that is not the reading ink.
- Compositor: `"cameras"` in `KINDS` (`compositor/crates/lumatv-compositor/src/shell_rust.rs`).
- Tests: `tests/test_rust_cameras.py` (14 cases against the fake helper; the lint matrix runs 11 scenes at both sizes and looks),
  `tests/rust_cameras_scenes.py` (the scenes; `python3 tests/rust_cameras_scenes.py` renders `docs/mocks/host-rust/cameras/*.jpg`, the
  committed set is `COMMITTED` in that file, 1280 wide), the fake helper's camera routes in `tests/qmlhost/fakehelper.py`.

Next, in this order:

1. Compositor suite: `cargo test --no-fail-fast` ran with these timing tests failing under a loaded machine, none touching the cameras (the
   only change there is the `KINDS` entry): lib `wake::tests::a_time_shared_probe_measures_a_sleeping_thread_and_stops`, and the
   integration tests `boxdim`, `livetv` (`a_launch_and_a_leave_of_live_tv...`) and `switcher` (`a_second_press_from_an_app...`). Rerun them
   on a quiet machine, then `cargo test --features ui`; the `shell_rust` filter passes.
2. TV: a granted window and a refused one (`ctl vwin`): the picture is a hole either way; the glyph shows only on a refusal.
3. Peers: the popup (RustSystem) calls `shell.cameras().open_full(id, now)`; Settings (RustSettings) renders the per-camera page with
   `cameras::options` and may give the cameras a page of their own (`CAMERAS_SETTINGS_PAGE`); the quick panel's Cameras tile
   (RustPanels) calls `shell.cameras().open_grid(now)`.
4. After a TV round, tune what it finds (the checks below). Delete `Cameras.qml` only after Main says the Rust kind passed the TV
   gate twice.

Decisions taken (conservative, no question to Nitin needed):

- The spec's numbers over the mock's pixels (grid room y 150 to 966); chrome pills and panels use the glass at `readingAlpha`.
- Hold OK on a tile opens the per-camera options page (a modal sheet over a dimmed grid); the hint is the kit's "hold OK, options".
- Stills are layers at their own size scaled by the compositor (`Props.scale` and a clip): no CPU resampling at 40 stills a second.
- The feeder's own window (`<brand app id>.cameras`, `helper/app/src/scrypted/mod.rs`) in the `foreground` topic is ignored, so a
  stream over Home does not read as an app in front.
- A refusal is a toast above the strip and its bubble; words come from `words::refusal`; no instruction sentences anywhere (glyph hints).
- The grid scrolls by whole rows on the shared flowing-focus spring; rows outside the shown block are built but answer no hit.
- The Settings button closes the app (Settings replaces it; Back in Settings goes to what was under the app).

## Parity

| Screen | Status |
|---|---|
| Grid (tiles, keys, pacing, banner, foot, empty states) | QML cases reproduced as host tests; spec additions (header buttons, scroll) done; TV unchecked |
| Full view (hole, stills, chrome fade, live open, switch, stop, over apps) | QML cases reproduced; TV unchecked |
| Strip (scrub, thumbnails, events, playback, rate, pause) | new in the spec (no QML twin); host-tested; TV unchecked |
| Events and clips sheet | new in the spec; host-tested; TV unchecked |
| Per-camera options page | model shared with Settings; host-tested; TV unchecked |

## TV checks for the next round

1. `ctl shell rust +cameras`, `ctl shell show cameras`: the grid is up at once; stills fill in at about 5 a second per tile; Back closes.
2. OK on a tile over Home: the stream starts, the picture is a hole over the video plane; the chrome fades after 2.5 s and the first key
   only brings it back; Left and Right switch; Back stops the stream. Look at the report's `appFront` while the feeder window is up: it must
   stay `false`.
3. Over an app: stills only (no `POST /v1/cameras/live`), unless `theme.cameras.liveOverApps` is true.
4. The QML viewer reads the `cameras*` topics itself, so with only `cameras` switched to Rust a pop-up card and its OK still belong to
   QML: switch `popup` (RustSystem) together with it, and look for a double draw.
5. Up on the live view opens the strip: scrub with Left and Right, a thumbnail follows 250 ms after the knob settles, the playback
   starts 450 ms after the last step; a gap says "No recording at that time" as a toast.
6. Down opens the events or clips sheet; OK on a clip plays it.
7. Hold OK on a tile: the options page; a switch writes `notify.<id>` (the helper's schema) and the page follows the answer.
8. Pointer: hover is a cue only, click selects; the focus lift is the shared spring on every tile, button, card and row.
9. Cost: `RESULT ... cpu mean` of the harness with the grid streaming (40 stills a second at 8 tiles) and the layer count (`Kit::room()`,
   at most 73 layers in the scenes).

## Changelog line

Cameras app in the Rust shell: grid of live stills, full view with the live stream on the video plane, scrub strip with thumbnails and
event marks, events and clips sheet and the per-camera pop-up options page, behind `--shell-rust cameras` with the QML viewer as fallback.
