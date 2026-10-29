# Rust panels: result and hand-over

Module `rustui/ui/src/panels/`: the quick panel (with the profile strip and PIN pad), the input picker, the notification drawer, the
context menu, the confirm dialog and the app switcher's chrome. Three kinds are switched on in code and opt-in at run time:
`ctl shell rust +quick +picker +drawer` (the QML panels stay the fallback per kind).

## Resume here

**Done, pushed (`cargo test -p lumatv-ui` lib tests, compositor `shell_rust` and `ime_own` tests, `tests/test_rust_panels.py` host cases;
`clippy -D warnings` clean)**

- Kinds `quick`, `picker`, `drawer`, `menu`, `confirm`, `switcher`, `entry`, `keyboard`: `panels/layer.rs` (`Panels`, shaped like
  `system/layer.rs::Layer`) is called from `shell.rs` and the compositor lists the kinds (`shell_rust.rs::KINDS`). One panel is up at a
  time; `show{kind,warm,n}` opens quick/picker/drawer (a second press closes, the picker goes on to the next input); 45 s idle, an app
  coming to the front and the screen going off close what is up. Detail and the rules of each kind: `rust-shell-kit.md`, "The panels".
- Models with the QML's cases as tests: `quick/{mod,tiles,tests}.rs` (+ `arrange.rs`, `hold.rs`, `pin.rs`, `clock.rs`, `hub.rs`),
  `picker.rs`, `drawer.rs` (+ `notelog.rs`), `menu.rs`, `confirm.rs`, `switcher.rs`, `entry.rs`, `ime.rs`; `env.rs` is the shared state, `life.rs` the phases.
  Surfaces on `stage.rs`: `quick_surface.rs` (+ `quick_layout.rs`), `picker_surface.rs`, `drawer_surface.rs`, `menu_surface.rs`,
  `confirm_surface.rs`, `switcher_surface.rs`, `entry_surface.rs`, `ime_surface.rs`; all lint clean at 100 % and 200 % text in both materials.
- **menu / confirm**: opened by an owner with the topics `menu.open` / `confirm.ask`; `menu.picked` / `confirm.answered` go back as topics.
  Back, idle and an app coming to the front answer a dialog with its safe answer (so an owner is never left waiting). Nothing in the Rust
  shell asks for them yet: Home has its own menu and Settings its own sheets; they are for the screens that follow.
- **switcher** (real previews): the compositor draws each app's live window under the shell's layers (`switcher.rs` there); the shell
  draws the chrome and sends the cards' places (`POST /v1/switcher {op: layout}`, once per change) and `activate`/`quit`/`quitall`/`close`.
  The compositor (`shell_rust.rs`, `switcher.rs`) now takes `Msg::Switcher` from the Rust client, sends `apps`/`switcher` topics only to
  the client that answers the kind, counts its `can` and closes the switcher if the client goes. A video-plane app has no picture (the
  compositor says so): its card is the app's icon on a tile with "No preview". Opened by the Home key at rest (`homekey {switcher:true}` ->
  `Request::Switcher`) and by Back from the top of Home with `remote.recents`; both switches are off until the TV check.
- **entry**: the on-screen keyboard for the shell's own screens (Home's search keys, a sheet over Settings in `z::KEYBOARD`, solid on a
  hiding dim, a password as bullets, 128 characters). Settings' `ShellRequest::Text` opens it; Done gives the text back once to
  `settings.entered`, Back/idle/app-front to `settings.entry_cancelled` (never a topic, never printed). It takes every key while up, and the
  release of a key it took (Settings acts on a release). Kind off: the ask is cancelled at once.
- **keyboard** (the apps' text fields, `Msg::Ime`): the QML `Keyboard.qml` + `KeyboardLayouts.js` ported, same layouts (letters, e-mail, URL,
  symbols, four number pads), enter keys, shift, D-pad walk and repeats, voice key, Back ends the microphone first. Not one of the panels: the
  compositor sends the `ime` topic and the `imekey` events to the client that answers the kind (the other is told there is no field) and takes
  `POST /v1/ime` from it (`text`, `key`, `clear`, `hide`, `shown`); the sheet is drawn at the foot (full width, or a narrow centred pad) and its
  rectangle is posted as `shown`, so the field keeps clear of it. Needs `lumatv keyboard luma` (the owner's flag) as before; without the kind
  the QML keyboard serves. **The Magic Remote pointer was not wired to the Rust client at all** (`shell_rust_pointer` had no caller): the
  compositor now sends pointer samples to it where its layer wants the keys and on its keyboard's sheet, and OK is its click while the cursor
  is over it (`seat.rs`, `shell_rust.rs::shell_rust_pointer_sample`).
- **Plugin tiles/notes**: the helper poll (`host/http.rs::follow_filter`) already follows the `plugins` roster; the base list is 18 names,
  past the 16 a poll may carry, so the poll asks for every topic (as the QML client's does) and `panels/hub.rs` shows what the roster says is
  connected. The earlier note that the host did not subscribe was wrong; `test_rust_panels.py` proves a connected plugin's tile reaches the
  panel and a plugin that is not connected shows none (the fake helper now has opt-in `keepalive` and `filter_topics`).
- **Notification drawer** (`panels/drawer.rs`, `drawer_surface.rs`): the quick panel's place and width (560 at x 1320, up to its full 968 high).
  Grouped as Android does: by source (AI access, each plugin, the TV), groups by their newest notification, newest first inside; what says
  exactly the same (title, text, button labels) is one entry with a count ("x3"). A group of one entry is a label and the entry; a group of
  several is a header the focus visits (OK opens or folds it, Right reaches its Clear, the hint is the OK glyph and expand/collapse) and its
  newest entry. Opening folds every group. Clear all, a group's Clear and OK on an entry clear every notification it stands for. It comes out
  of the bell (`Home::bell_frame`, handed to the panels by the shell each frame; the right end of the bar when Home does not show one) as a
  container transform on the shared spring (`Arrive::Container`, 280 ms of life) and goes back into it: the glass grows first, the content
  and the dim come in late, and a motion under way keeps the rectangles it started with. Reduced motion is a quick fade. The QML drawer got only
  the quick panel's width (560) and place (40 from the edge).
- PIN pad at 200 % text overflowed the sheet (found by the host lint): the pad now fits the room under the head (keys shrink to a label's
  height, then the avatar is left out). The pad shows OK and Back as glyphs in the head where the clock was; the edit list keeps every tile name
  whole at large text (the marks go under the words, a long detail takes a second line, every row alike); a pick card with more options than
  the screen holds (200 % text) is as tall as the safe area and scrolls its options in a window, the focused one always whole. The quick
  panel's root, dim, modal scope, first focus and pointer are `Stage`'s now, like every other panel's.
- Host renders (1280-wide JPEG): `docs/mocks/host-rust/panels/{switcher,menu,confirm,entry,keyboard,keyboard-email,keyboard-pad,quick-pin,quick-edit,quick-card,drawer,drawer-open,drawer-clear}-{dark-100,light-200}.jpg`
  (`RUST_PANELS_RENDERS=docs/mocks/host-rust/panels python3 -m unittest test_rust_panels`; the drawer's also at light-100 and dark-200); the three
  older PNGs stay except the drawer's. `drawer-motion-dark-100.jpg` is the frame strip of the drawer growing out of the bell (top row) and
  back into it (bottom row), the right half of the screen; the test that writes it needs the release build of `lumatv-ui` (the debug build
  takes longer to bring the drawer up than the whole motion).
- Harness: `lumatv-ui --host` prints the shell's `POST /v1/switcher` as `[switcher] ...`. `Move` in a menu spec uses `layout-grid` (the set
  has no `move` glyph).

**Not done**

1. The apps' keyboard on the TV: nothing is verified there (voice key, the sheet's place against a real field, the first frames, the
   pointer samples the compositor now sends). The QML keyboard's key lift and press shrink are not ported (the focus plate glides, as the
   entry sheet's does).
2. A pick card's scroll and the edit list's stacked rows have host renders at 200 % only through the quick panel's card and edit scenes;
   the cases with many options are unit tests.

**Decisions taken** (conservative standard option; change if Nitin disagrees)

- A click acts, hover is a cue; the dim behind a panel is a disabled full-screen focusable and `Dim` tells its click (Back; the
  confirm dialog has none). One OK on a profile switches; a profile with a PIN asks the pad in place; the owner is never asked; the
  strip is hidden for one profile.
- Panels toggle on a second press and close on idle (QML `LumaOverlay.qml`: `idle` 45 s, `panel(kind)`).
- The switcher chrome is not closed from the Rust side by idle/app-front (the compositor does that); it posts `close` after 45 s and when
  the screen goes off.
- Typed text is handed back by a direct call, not a topic: a topic would reach every model, and a password is typed there.
- The entry sheet is a kind of its own (`entry`), not `keyboard`: the compositor's `keyboard` topic and `can` mean the IME for apps.
- Drawer: an item with buttons is lit as a whole (`Flow::go` on its plate) while the focus flies between its buttons.
- Pictures come from files only (`avatar::picture`, `avatar::contained`); `menu::Raised.plate` is a `#RRGGBB` string.

## TV checks for the next round

1. `ctl shell rust +quick +picker +drawer`; gear tap opens the quick panel at x 1320, 560 wide, in under 150 ms; Back closes; a second
   gear tap closes. Input key opens the picker, again goes on to the next input, OK launches it.
2. Quick panel: Up/Down/Left/Right move the lift between rows, OK on the sleep timer and on Wi-Fi/receiver acts, hold OK on a tile with
   `remote.quickPanelEdit` on picks it up; with two profiles the strip is the first row and a PIN profile asks the pad in place.
   "All settings" opens Settings (LG's when the `settings` kind is off).
3. Drawer: gear hold or the bell opens it out of the bell; OK on a group header opens and folds it, Right then OK clears the group, an
   item's buttons, Clear all, Back; 45 s idle closes any panel.
4. Pointer: hover shows a cue only, a click acts; a click beside a panel closes it.
5. `ctl shell rust +switcher` with `remote.appSwitcher` on: open two apps, Home twice (or Home at rest with apps open): the pictures are
   the live windows in four cards, Left/Right moves the ring, OK opens the app, Down then OK quits it, Up then OK asks before Quit all; the
   Back key closes. An HDMI input shows its icon and "No preview". `ctl switcher` reports the cards' places.
6. `ctl shell rust +entry +settings`: Settings > Network > Other Network (or a secured network): the keyboard sheet opens over Settings,
   the arrows move over the keys, OK types, the number keys type digits, Done goes on to "Join ...?", Back cancels. Press neither Join nor
   Apply on the first pass.
7. `ctl shell rust +menu +confirm`: nothing opens them yet (no owner); nothing to check beyond the kinds being accepted.
8. `lumatv keyboard luma`, `ctl shell rust +keyboard`: open an app's search field (YouTube, Plex): the sheet comes up at the foot within two
   seconds (else LG's keyboard serves for a minute), the field moves clear of it, the arrows/OK/digits/Back act as before, Search/Done
   hide it, the Magic Remote's pointer hovers and clicks the keys and nothing under it, the mic key (when LG says it has voice) listens;
   a number field gets the narrow pad. `ctl shell rust -keyboard` gives the QML keyboard back.

## Changelog line

Rust panels: the quick panel, input picker, notification drawer, context menu, confirm dialog, app switcher chrome (over the compositor's
live pictures), an on-screen keyboard for Settings' typed text and the keyboard of the apps' text fields are drawn by the Rust UI and switch on per kind with `ctl shell rust
+quick +picker +drawer +menu +confirm +switcher +entry +keyboard`; the Magic Remote's pointer now reaches the Rust client.
