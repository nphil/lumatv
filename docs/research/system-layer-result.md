# System layer in Rust: status

## Resume here (RustSystem2, 2026-10-29)

Worktree rule: `git fetch forge && git worktree add /tmp/<name>-wt forge/main`; push with `timeout 60 git push -q forge HEAD:main`. Tests: `cd rustui && cargo test -p lumatv-ui`
(about 25 s warm), `cargo clippy -p lumatv-ui --all-targets -- -D warnings`; stand-in tests `cd tests && python3 -m unittest test_rust_system` (needs
`rustui/target/debug/lumatv-ui`); renders go to `docs/mocks/host-rust/system/` (`RUST_SYSTEM_RENDERS`).

| Order | Piece | State |
|---|---|---|
| 1 | Saver surface, kind `saver` | not started; plan below |
| 2 | Cards surface, kind `cards` | not started |
| 3 | Camera pop-up (stills), kind `popup` | not started |
| 4 | Capture mark (`capture`), mute-mark drift (`osd`), layout editor (`layout`) | not started |
| 5 | Compositor `--shell-rust` handling per kind | not started |
| 6 | `lumatv-tokens` focus tests | done upstream (74ea0ff) |

Plan for each (decisions taken, so a fresh helper does not re-derive them):

- **Wiring pattern.** `Kinds` in `shell.rs` already names `cards, saver, capture, popup, layout`; `system/layer.rs` (`Layer`) holds the models and surfaces of the kinds
  that sit over everything and is the only place a new kind is added (`start`, `update`, `apply`, `tick`, `due_at`, `wants`, `takes`, `state`, `geometry`,
  `feed_lint`, `restyle`, `on_key`, `pointer`). Surfaces follow `alert_surface.rs` (layers made once, hidden until shown, dropped when gone; buttons from
  `kit/button.rs` in a modal scope of the shared `Engine`) and `volume_surface.rs` (placement through `lumatv-placement`). Offsets in `layer::z`.
- **Saver.** Model exists (`saver/model.rs`, `look.rs`, `clock.rs`); the surface is new (`saver/surface.rs`): opaque black plate over everything at `z::SAVER`,
  the analog face (`clock.rs`) as image layers (face, hands per minute, second hand per tick or a smooth sweep on a motion track), date and weather as `kit::text::Text`
  in the three inks of `look.rs` (never above `cfg.cap`), notice lines from `Hub::saver_doc()["lines"]`, complications from `Hub::complications(now)` as glyph + text +
  value, only the `inside` answers of `/v1/saver/rules`. The weather row comes from the helper topic `weather` (`doc.display.temp`, `cond`, `nowKind`); no topic, no row. `main.rs`
  `HELPER_TOPICS` lacks `saver` (add it). The saver wants every key while up (`Saver::wants_keys`).
- **Cards.** `cards.rs` has the queue (`Queue::offer/select/dismiss/armed`) and `plugins.rs` the hub; missing: the `Cards` model (Input to `Queue`, camera merge, door
  cards, `takes` ["ok","back"] after `armMs`, `syskey`-style OK/Back, drawer-ready flag) and `cards_surface.rs` (pane per card at `lumatv_placement` id `cards`, hint row from
  `keys_of`, action from `ok_of`). Switch: `overlays.enabled` or `overlays.rust` (already read by `cards::Config`).
- **Pop-up.** Stills only: a pane flush in the `popup` zone (64 px from the right edge), 560 wide, the helper's `cameras.popup` topic and `GET /v1/cameras/<id>/still`
  (see `docs/research/cameras-ui-spec.md`, the pop-up section); live video later through hardware video windows.
- **Capture mark.** `capture` topic (`recording`, `n`, `last`) as `panels/CaptureMark.qml` (dot top left, note 2.6 s); the report must carry `capture.dot` (the compositor
  records only while it is true: `screencap_publish` in `compositor/.../shell.rs` reads the QML client's `ui_state`; read the Rust client's too).
- **Layout editor.** `lumatv_placement::Editor` (`Editor::key`, `view(now)`, `pointer_*`, `panel_spot`) drawn on a live preview of the five items; `Effect::Save(map)` is written
  with `Effect::Theme` (`{"layout": {"overlays": map}}`, a whole map replaces); opened by `Shell::open_layout(item)` (RustSettings' `ShellRequest::Layout { item }` calls it)
  and by a `show` event of kind `layout`.
- **Compositor** (`compositor/crates/lumatv-compositor/src/shell_rust.rs`, `shell.rs`, `server/state.rs`): `KINDS` gains each finished kind; `saver`: the QML client's calls
  to tvpower's `registerScreenSaverRequest` are refused while the Rust shell answers it, and `scene.care.saver` reads either client's `ui_state.saver`; `alert`: the QML
  client's three notification feeds are withheld (only `toast` is today); `capture`: the `capture` topic goes to the Rust client and not the QML one, and the dot report is
  read from the Rust `ui_state`; `cards`/`popup` read helper topics the compositor cannot withhold, so the owner's switch is the gate (`overlays.enabled` / `cameras` options).

Module `rustui/ui/src/system/`. Kinds in the shell: `osd`, `toast` (core), `alert` (this slice, wired). The rest of the layer is partly built.

| Piece | State |
|---|---|
| Alerts, input popup, PIN (`alert.rs`, `alert_surface.rs`) | Done, wired as kind `alert`; QML cases ported, lint clean at 100 % and 200 % text, dark and light |
| Overlay queue (`cards.rs`, from `Overlays.js`) and plugin hub (`plugins.rs`, from `PluginHub`) | Models done with the QML tests ported (class order, dwell, wait, merge, doorbell, score, keys after the arm time); no surface and not wired: kind `cards` is not answered yet |
| Saver numbers, clock raster, model (`saver/look.rs`, `clock.rs`, `model.rs`) | Done and tested (cap, dim, drift, spots, vote on tvpower's lists, wake, standby); the surface (date, weather, notices, slots) and the wiring are not built, so kind `saver` is not answered yet and no saver render exists |
| Saver rules and slots (helper, crate `lumatv-saver`) | Done: `/v1/saver/*`, the `saver` topic, enforcement at the camera pop-up and the plugin post (`docs/research/saver-complications.md`) |
| Camera pop-up, capture mark, layout editor, mute-mark drift | Not started in Rust; the QML ones stay in force |

QML changes: the interim relief of the saver (`clockLevel` 0.4, `dimTo` 0.4 over 20 min), Home counts as busy while its saver is up (a camera
pop-up no longer covers it), and the compositor's `status` carries `oledcare.saver`.

## TV checks (next round)

1. Saver on Home: clock at 0.4 and dimmer, over 20 minutes the dim reaches 0.4 of that; wake on a key, the pointer and Back.
2. A camera event while the saver is up: no video over it (default `camera.popup`: hide); the event is a line in `lines` of the `saver` topic.
3. A doorbell while the saver is up (QML saver owned by LumaTV): it does not wake by itself (the next key does); with LG's saver (state `Screen Saver`) it wakes the screen.
4. `GET /v1/saver/rules` on the TV lists the shipped classes and one row per installed plugin; `POST` changes one and `saver-rules.json` appears in the data directory.
5. `ctl status` shows `oledcare.saver` true while the saver is up.
