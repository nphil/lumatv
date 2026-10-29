# Rust panels: result and hand-over

Module `rustui/ui/src/panels/`: the quick panel (with the profile strip and PIN pad), the input picker, the notification drawer, the
context menu, the confirm dialog and the app switcher's chrome. Three kinds are switched on in code and opt-in at run time:
`ctl shell rust +quick +picker +drawer` (the QML panels stay the fallback per kind).

## Resume here

**Done, pushed (`cargo test -p lumatv-ui` 615 lib tests, compositor `shell_rust` tests, `test_cutover`, `test_shell_client`,
`test_rust_shell`, `test_rust_system` green)**

- Kinds `quick`, `picker`, `drawer`: `panels/layer.rs` (`Panels`, shaped like `system/layer.rs::Layer`) is called from `shell.rs`
  (`Kinds`, `feed`, `key`, pointer, `tick`, `after`, `report` can/wants, `state`, `due_at`, `geometry`, `lint`, `restyle`) and
  the compositor lists the kinds (`shell_rust.rs::KINDS`). It takes the compositor's `show{kind,warm,n}` topic (a replayed `n` is
  ignored, the state replay skips `show`), keeps one panel up at a time (a second press of the same closes it, the picker goes on to
  the next input), closes after 45 s with no key/pointer, when an app comes to the front and when the screen goes off, routes keys
  (arrows repeat, OK/Back once, OK on an editable quick tile waits for tap or 700 ms hold) and clicks to the models, and turns
  `Ask::Settings(page)` into `Shell::open_settings(page)` (LG's Settings app if the `settings` kind is not on). The bell of the Rust
  Home (`Request::Notifications`) opens the drawer.
- Models with the QML's cases as tests: `quick/{mod,tiles,tests}.rs` (+ `arrange.rs`, `hold.rs`, `pin.rs`, `clock.rs`, `hub.rs`),
  `picker.rs`, `drawer.rs` (+ `notelog.rs`), `menu.rs`, `confirm.rs`, `switcher.rs`; `env.rs` is the shared state, `life.rs` the
  enter/exit/linger phases.
- Surfaces (layers, flowing focus, pointer, lint feed, geometry), tests lint clean at 100 % and 200 % text in both materials:
  `quick_layout.rs` + `quick_surface.rs` (sheet 560 wide at x 1320, list that scrolls on the focus spring, strip, PIN pad, edit list,
  value card), `picker_surface.rs`, `drawer_surface.rs` (only the items near the window are made); shared `stage.rs` (`Stage`, `Dim`)
  and `ui.rs`. Host renders: `docs/mocks/host-rust/panels/{quick,picker,drawer}-{dark-100,light-200}.png`.
- Kit: `kit/widgets.rs` (Plate, Hover, Toggle, Chip, Stepper, Slider, Px, tile), `kit/avatar.rs`, `Effect::Request/Channel`,
  `Button::feed_lint_at` and the icon-run fix in `Button::feed_lint`, host models `host/{inputs,net,power,apps,receiver}.rs`.

**Not done, in this order**

1. `menu_surface.rs`, `confirm_surface.rs`, `switcher_surface.rs` on `Stage` (copy `picker_surface.rs`: pure `layout()`, `build()`,
   `apply()`, `pointer()`, `feed_lint()`, `geometry()`, tests). Numbers: `payload/compositor/lumatv/panels/ContextMenu.qml` (rows
   `LuRow`, tray `LuSegmented`, card at `menu::Spec::place_card`, `raised` thing above the dim, glyph hints in place of
   `spec.hint`), `panels/Confirm.qml` (760 card, icon 56, a `Button` per answer, no click-to-close), `panels/Switcher.qml` +
   `SwitcherLayout.js` (chrome only; the cards are the compositor's, `switcher.rs::geo::card`). Scopes: menu 203, confirm 204,
   switcher 205 (quick 200, popup 206, picker 201, drawer 202). Then `Which::{Menu,Confirm,Switcher}` in `layer.rs` and `Kinds`
   fields; `menu.picked`/`confirm.answered` leave as `Out.events`, `Panels::take_events` hands them to `Shell::panel_events`.
2. Switcher plumbing: add `switcher` to `KINDS`; forward `Msg::Switcher` to `switcher_op` for the Rust client (`shell_rust_handle`
   drops it now); push topics `apps`/`switcher` to Rust when it answers `switcher` (`switcher.rs::switcher_sync_topics` pushes to QML
   only) and withhold `switcher` from QML meanwhile; `POST /v1/switcher` ops go out as `Effect::Channel`.
3. Per-plugin helper topics (`plugin.<id>.tiles`, `plugin.<id>.notifications`) are not subscribed by the host: `main.rs::HELPER_TOPICS`
   is static; the quick panel's plugin tiles and the drawer's plugin notes stay empty until the host follows the `plugins` topic's
   ids (RustSystem's `host/plugins.rs` has the same need; converge with `panels/hub.rs`). The `profiles` and `openapps` topics are
   already followed.
4. `tests/test_rust_panels.py` on the stand-in channel (cases from `test_quick_panels.py`, `test_panels.py`, `test_switcher.py`,
   `test_shell_switcher.py`, `test_plugins_surfaces.py`; `show` is a `--push MS:show:{"kind":"quick","warm":false,"n":1}` in the
   harness, keys are `--key MS:down`), a harness `--eval panels` if geometry of the panels is needed apart from `--eval geometry`
   (it is already in there under `quick`/`picker`/`drawer`), a section for widgets/avatar/panels in `rust-shell-kit.md`, renders of
   the PIN pad, edit list and value card.
5. Cleanup: `quick_surface.rs` keeps its own root/dim/hover/modal code (move onto `Stage`); `host/sio.rs` is now RustSettings' port.

**Decisions taken** (conservative standard option; change if Nitin disagrees)

- A click acts, hover is a cue; the dim behind a panel is a disabled full-screen focusable and `Dim` tells its click (Back; the
  confirm dialog has none). One OK on a profile switches; a profile with a PIN asks the pad in place; the owner is never asked; the
  strip is hidden for one profile.
- All three panels toggle on a second press and close on idle (QML `LumaOverlay.qml`: `idle` 45 s, `panel(kind)`).
- Drawer: an item with buttons is lit as a whole (`Flow::go` on its plate) while the focus flies between its buttons.
- Pictures come from files only (`avatar::picture`, `avatar::contained`); `menu::Raised.plate` is a `#RRGGBB` string.
- The drawer's head is as tall as its title when there is no Clear all (the QML's overflowed at 200 % text).

## TV checks for the next round

1. `ctl shell rust +quick +picker +drawer`; gear tap opens the quick panel at x 1320, 560 wide, in under 150 ms; Back closes; a second
   gear tap closes. Input key opens the picker, again goes on to the next input, OK launches it.
2. Quick panel: Up/Down/Left/Right move the lift between rows, OK on the sleep timer and on Wi-Fi/receiver acts, hold OK on a tile with
   `remote.quickPanelEdit` on picks it up; with two profiles the strip is the first row and a PIN profile asks the pad in place.
   "All settings" opens Settings (LG's when the `settings` kind is off).
3. Drawer: gear hold opens it, Clear all, an item's buttons, Back; 45 s idle closes any panel.
4. Pointer: hover shows a cue only, a click acts; a click beside a panel closes it.

## Changelog line

Rust panels: the quick panel, input picker and notification drawer are drawn by the Rust UI (flowing focus, click-to-select pointer,
lint-clean at 200 % text) and can be switched on per kind with `ctl shell rust +quick +picker +drawer`; the context menu, confirm
dialog and switcher chrome follow.
