# Cameras in the Rust shell (kind `cameras`): result

The full-screen Cameras app of `docs/research/cameras-ui-spec.md` in `rustui/ui/src/cameras/`, drawn with the shell kit at
`system_z + z::CAMERAS` (+200). Switched per kind: `lumatv-compositor --shell-rust cameras` or `ctl shell rust +cameras`; the QML viewer
(`payload/compositor/lumatv/panels/Cameras.qml`) stays the fallback and is untouched.

## Resume here

Done (all on `forge/main` unless marked):

- Model, all host-tested (`cameras/*.rs`, 47 behaviour tests in `app_tests.rs` plus the unit tests of each module): `words` (clock, ages,
  refusal words), `data` (typed `cameras`, `cameras.live`, `cameras.events`, popup, recordings, ticks, clips, settings), `grid` (columns,
  layout, scroll window, keys), `strip` (span, rate, scrub steps, gaps, events), `sheet`, `options`, `pics` (the picture planner: pacing,
  tags `0x0CA0_0000..=0x0CAF_FFFF`), `view`, `app` (`App`: `Model` impl, `open_grid`, `open_full`, `open_events`, `close`, `take_intents`).
- Surfaces (`cameras/surface/`): `grid`, `full` (hole, stills, chrome fade, note), `strip` (painted bar canvas, knob, bubble), `sheet`,
  `options`, `toast`, `pill`, `pic` (picture store, compositor-scaled layers), `root` (`CamerasSurface`), `common`.
- `cameras/host.rs` (`Cameras`: decodes the stills, joins model and surface) and the wiring in `shell.rs` (`Kinds.cameras`, feed, key,
  pointer, tick, report, state, geometry, lint, `take_cameras_intents()`, `cameras()`).
- Kit additions: `z::CAMERAS`, `Flow::scroll_snap` / `drop_scroller`, `Button::icon_after` and glyph-name icons, "dot" glyph, the muted rule
  in `Button::feed_lint` for ink that is not the reading ink.
- Compositor: `"cameras"` in `KINDS` (`compositor/crates/lumatv-compositor/src/shell_rust.rs`).
- Tests: `tests/test_rust_cameras.py` (13 cases against the fake helper; the lint matrix runs 11 scenes at both sizes and looks),
  `tests/rust_cameras_scenes.py` (the scenes; `python3 tests/rust_cameras_scenes.py` renders `docs/mocks/host-rust/cameras/*.jpg`), the
  fake helper's camera routes in `tests/qmlhost/fakehelper.py`.

Next, in this order:

1. **Names under the tiles at 200 % text** overlap the next row's pictures: `NAME_H` (44) is a constant in `cameras/grid.rs`. Make it
   scale (`(44 * text_scale.max(1.0)).round()`) through `Room`/`Layout` (`layout()`, `Layout::pitch()`, the two `block_h` sums) and
   `surface/grid.rs` (`total_h`, the name's place); the spec's vectors at 100 % stay as they are. Then re-render.
2. Re-render the matrix and commit `docs/mocks/host-rust/cameras/` (68 JPEGs, 16 MB, not committed yet: commit them once).
3. Run the whole compositor suite (`cd compositor && cargo test`, then `cargo test --features ui`); only `shell_rust` was run.
4. After rebasing, the `KINDS` line of `shell_rust.rs` conflicts with the other slices' kinds: keep every name.
5. Peers' side: the popup (RustSystem) calls `shell.cameras().open_full(id, now)`; Settings (RustSettings) reads
   `take_cameras_intents()` (`Intent::OpenSettings`) and renders the per-camera page with `cameras::options`; the quick panel's
   Cameras tile (RustPanels) calls `shell.cameras().open_grid(now)`. Until then the grid's `Settings` button does nothing.
6. Delete `Cameras.qml` only after Main says the Rust kind passed the TV gate twice.

Decisions taken (conservative, no question to Nitin needed):

- The spec's numbers over the mock's pixels (grid room y 150 to 966); chrome pills and panels use the glass at `readingAlpha`.
- Hold OK on a tile opens the per-camera options page (a modal sheet over a dimmed grid); the hint is the kit's "hold OK, options".
- Stills are layers at their own size scaled by the compositor (`Props.scale` and a clip): no CPU resampling at 40 stills a second.
- The feeder's own window (`<brand app id>.cameras`, `helper/app/src/scrypted/mod.rs`) in the `foreground` topic is ignored, so a
  stream over Home does not read as an app in front.
- A refusal is a toast above the strip and its bubble; words come from `words::refusal`; no instruction sentences anywhere (glyph hints).
- The grid scrolls by whole rows on the shared flowing-focus spring; rows outside the shown block are built but answer no hit.

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
