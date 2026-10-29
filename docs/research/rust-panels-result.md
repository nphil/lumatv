# Rust panels: result and hand-over

Module `rustui/ui/src/panels/` (quick panel, profile strip and PIN pad, input picker, drawer, context menu, confirm dialog, app switcher
chrome). Nothing here is reachable from the shell yet: no kind is switched on, so the QML screens are unchanged.

## Resume here

**Done and tested** (`cargo test -p lumatv-ui`: 298 lib tests green after the rebase on 74ea0ff)

- Models, one per screen, ported with the QML's cases: `panels/quick/{mod,tiles,tests}.rs` (+ `arrange.rs`, `hold.rs`, `pin.rs`,
  `clock.rs`, `hub.rs`), `picker.rs`, `drawer.rs` (+ `notelog.rs`), `menu.rs`, `confirm.rs`, `switcher.rs`; `env.rs` is the shared
  state (`Env`: sio, inputs, power, net, gate, topics, theme, foreground); `life.rs` is `Life`/`Phase` (enter/exit 110 ms, linger 4 s).
- Surfaces (layers, flowing focus, pointer, lint feed, geometry JSON), each with tests that lint clean at 100 % and 200 % text in
  both materials: `quick_layout.rs` + `quick_surface.rs` (sheet 560 wide at x 1320, list, strip, PIN pad, edit list, card),
  `picker_surface.rs`, `drawer_surface.rs`. Shared by them: `stage.rs` (`Stage`: root + dim + modal scope + first focus + pointer,
  `Dim` click on the dim) and `ui.rs` (`Root`, `Scroll`, `Words`, `Line`, `follow`).
- Kit additions: `kit/widgets.rs` (`Px`, `tile`, `Plate`, `Hover`, `Toggle`, `Chip`, `Stepper`, `Slider`, `Lift`), `kit/avatar.rs`,
  `Effect::Request` and `Effect::Channel` (`kit/screen.rs`, handled in `shell.rs::carry`), `Button::feed_lint_at` and the icon-run
  fix in `Button::feed_lint` (`kit/button.rs`, RustSystem's file), host models `host/{inputs,net,power,apps,receiver,sio}.rs`.

**Next, in this order**

1. `menu_surface.rs`, `confirm_surface.rs`, `switcher_surface.rs` on `Stage` (copy `picker_surface.rs`: `layout()` pure, `build()`,
   `apply()`, `pointer()`, `feed_lint()`, `geometry()`, tests). Numbers are in the QML: `panels/ContextMenu.qml` (rows `LuRow`, tray
   `LuSegmented`, card at `Spec::place_card`, `raised` thing above the dim, glyph hints instead of `spec.hint`), `panels/Confirm.qml`
   (760 card, icon 56, `Button` per answer), `panels/Switcher.qml` + `SwitcherLayout.js` (chrome only; the cards are the compositor's:
   `switcher.rs::geo::card`). Scopes: menu 203, confirm 204, switcher 205 (quick 200, popup 206, picker 201, drawer 202).
2. `Panels` aggregator in `panels/mod.rs`, shaped like `system/layer.rs::Layer`: `update/apply/on_key/pointer/tick/due_at/wants/can/
   state/geometry/feed_lint/restyle`; show/hide/toggle/next for `show{kind,warm,n}`, close on foreground change, 45 s idle close,
   key routing (ignore releases except the quick OK hold, ignore repeat of back/ok), `Out.events` fed back as `Input::Topic`
   (`menu.picked`, `confirm.answered`, `notelog.action`, `settings.open`), `Ask::Settings(page)` becomes `settings.open{page}`
   (until `home`/`settings` are Rust the fallback launches LG's Settings, `com.palm.app.settings`).
3. `shell.rs`: `Kinds` fields `quick`, `picker`, `drawer`, `switcher` (one per line), `parse`, `names`, report `can`/`wants`
   (OR in `Panels::wants`), subscribe NoteLog/apps/net at start, give `Panels` the data dir, `lint`/`geometry`/`restyle` hooks.
4. Compositor (`compositor/crates/lumatv-compositor/src/shell_rust.rs`): add the kinds to `KINDS`; forward `Msg::Switcher` to
   `switcher_op` for the Rust client; push topics `apps`/`switcher` to Rust when it answers `switcher` (`switcher.rs::
   switcher_sync_topics` pushes to QML only) and withhold `switcher` from QML meanwhile.
5. Harness `--eval panels`, `tests/test_rust_panels.py` (stand-in channel as in `tests/test_rust_system.py`; cases from
   `test_quick_panels.py`, `test_panels.py`, `test_switcher.py`, `test_shell_switcher.py`, `test_plugins_surfaces.py`), renders to
   `docs/mocks/host-rust/panels/` (dark/light x 100/200 %), a section in `rust-shell-kit.md` for widgets, avatar and panels.
6. Optional cleanup: `quick_surface.rs` keeps its own root/dim/hover/modal code; move it onto `Stage` as the other surfaces are.

**Decisions taken** (conservative standard option, change if Nitin disagrees)

- `quick`, `picker`, `drawer`, `switcher` are compositor kinds; `menu` and `confirm` are Rust-only components that Home/Settings drive.
- A click acts, a hover is a cue only; the dim behind a panel is a disabled full-screen focusable and `Dim` tells its click (Back; the
  confirm dialog has no click-to-close). One OK on a profile switches; a profile with a PIN asks the pad in place; the owner is never
  asked; the strip is hidden for one profile.
- Drawer: an item with buttons is lit as a whole (`Flow::go` on its plate) while the focus flies between its buttons; only the items
  near the window are made (the scene's budget is 256 layers).
- Pictures are only decoded from files (`avatar::picture`, `avatar::contained`); `Raised.plate` is a `#RRGGBB` string.
- `host/sio.rs` is the minimal copy panels needs; RustSettings planned a superset with the same method names: extend in place.
- Idle close (45 s) is modelled for the switcher; for the other panels it is planned as OLED care and not verified against QML.

## Changelog line

Rust panels: the quick panel, input picker and notification drawer are drawn by the Rust UI (models, layers, flowing focus, pointer,
lint-clean at 200 % text), not yet switched on; the context menu, confirm dialog and switcher chrome follow.
