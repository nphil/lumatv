# Home in the Rust shell: the `home` kind (result, 2026-10-29)

The slice of `rust-shell-plan.md` phase 3, built on the host (Mesa) and not yet run on the set. The kind is `home`; the code is
`rustui/ui/src/home/`; the kit doc (`rust-shell-kit.md`, "Home") says how it is built. The QML stays (`LumaHome.qml` and its tree) until
the kind passes the TV gate twice.

## Resume here

Done (host-tested, pushed as "Home in the Rust shell: the home kind"):

- `home/grid.rs`: `LayoutGrid.js` ported whole (area, cells, `rect_of`, `to_uniform`, change/shift/resize/add/remove/hide/show/order/
  swap/nudge, `for_file` minimised against the shipped layout, `snap_move`/`snap_resize`, `neighbour`), tests on the shipped layout.
- `home/dock.rs`: the launch points as the person has them (`apps.dockHide`, `apps.dockOrder`, LG's Apps tile first), columns that fit the
  card, scrolled rows, a tile picked up and moved along the order, the accent a tile lends. `all_points` is the library's list.
- `home/model.rs`: the Home model (pure). Theme and layout merged (`apply_layout`, `preview_layout`, `set_theme`), the files read every
  2 s with `Effect::Read` (tags 310 theme, 311 layout, 312 tint) and the editor's save written with `Effect::Write`; the focus (the kit's
  `Focus`: status items `Id(100+i)`, cards `Id(200+i)`, dock tiles `Id(300+i)`; the layers over Home as modal scopes 1..6); keys (OK on
  release, the hold at `motion.holdMs` for a tile's options or the editor, Back one step, digits open dock tiles, the Menu key); the
  pointer (hover cue, click selects, the editor's drag); the feeds (`weather`, `news`, `plugin.homeassistant.cards`, `cameras`,
  `profiles`, `voice`); the launch points over luna (tags 300 homelaunchpoints, 301 applicationManager fallback); `state()` is what
  `--eval home` prints; `take_requests()` gives `Request::{Settings, Notifications}` for other kinds to open.
- `home/apps.rs` (the library), `home/search.rs` (search, the keyboard, `hook_text` for text typed elsewhere: topic `keyboard.text`
  `{"text"}` and the helper's `voice` `{state: "heard", text}`), `home/editor.rs` (the layout editor's state machine with the question on
  Back and the pointer's drag), `home/picker.rs` ("Who's watching?", the PIN pad, the switch veil, `pickerOnWake` on the `screen` topic).
- `home/surface.rs` + `widgets.rs` + `layers.rs` + `wallpaper.rs` + `clock.rs`: the drawing. Every focusable is a `Lift` (a group in the
  shared `Engine`'s flow; the lit glass under a child group). Cards: one `Words` block (lines kept beside the block for the lint) plus one
  or two image layers painted on the CPU (`Canvas`). The text size scales the status bar and the chip and re-plans the clock and the
  weather at the reduced room. `HomeSurface::lint` feeds the kit's lint; the unit tests hold the shipped Home and the halves to it at
  100 % and 200 %, dark and light.
- Kit additions: `Effect::Read`, `Effect::Write` (`kit/screen.rs`, carried out in `shell.rs`), `Flow::group`, `Flow::place`,
  `Flow::take_lift` (`kit/flow.rs`). Shell: the `home` kind in `Kinds`, `system_wants()` (a system-layer kind with the keys takes a key
  before Home), the report's `wants` while Home is in front, `--eval home`, `voice` in `HELPER_TOPICS`.
- Compositor: `home` in `KINDS`; `Scene::set_home_rust` (the QML Home window left out of the stack, Home counts as drawn while no app's
  card is in front); `shell_rust_sync_home` on the engine's handshake, the client going and a kinds change; `shell_home_drawn` reads it.
- `tests/test_rust_home.py`: the QML tests' cases on the harness; `test_renders` writes `docs/mocks/host-rust/home/*.png`.

Next, in this order:

1. The TV round (below): the first run of the kind on the set, with `ctl shell rust +home`; the QML Home is left out of the stack by the
   compositor while the Rust client is on the engine, so a bad frame is turned off with `ctl shell rust -home` (no deploy).
2. The other kinds' surfaces Home asks for: `Home::take_requests()`: Settings opens the `settings` kind when it is in the list
   (`shell.rs`, RustSettings), the drawer is logged until RustPanels lands; the tile menu and the confirm are Home's own (`layers.rs`
   `MenuSurface`, the editor's `Zone::Ask`) and can move to RustPanels' `panels::menu`/`confirm` when they exist
   (`shell.panels.open_menu(spec)` answered on `Input::Topic{"menu.picked"}`).
3. The dock's fade over overflowing rows draws (the `fade` shape) but the pill of a tile on the last visible row is not clipped by the
   card: a tile beyond the visible rows is reached by the arrows and scrolls into view on the spring.
4. Camera stills on the cameras card (the spec's `/frame` route needs `Effect::Fetch` with a binary answer, `Input::Bytes`): the card
   draws names, rooms, the online dot and the link's state in words today.
5. The status bar's bell badge (the toast log's unread count) is not drawn (`StatusItem.badge` is `None`).

Decisions taken (conservative, not asked):

- The Home model keeps its own `Focus` (pure, tested without a scene) and the surface mirrors its moves onto the shared `Engine`'s `Flow`;
  the kit's `Engine::step` is not used by Home. Ids 100..1199 are Home's.
- Up into the cards from the dock lands on the card the region remembers (the kit's rule), not the QML's "lowest by x"; Back from the
  dock keeps the QML's "lowest by x".
- `material: auto` is decided by the wallpaper's luma at load (`wallpaper::sample`), with a retheme of the kit when it differs; the lint
  composites a pane's glass over the mode's scrim over the canvas (a lighter picture flips the material).
- The chip's name width is `15 px × characters × text scale`; the bar is `60 × text scale` tall; pixel rows of the shipped layout shrink
  in proportion under a taller bar (`grid::with_status_height`), never written to the file.
- The news card slides its story every `news.slideMs` in the surface's own tick; inner focus on the news card picks the story shown.
- The cameras card is a new `cameras` layout type (`grid::TYPES`, addable, ladder 6x5 down to 3x3).

## The TV round

1. `LUMATV_FEATURES=ui sh compositor/tools/build-tv.sh`, deploy and cut over as the round does.
2. `ctl shell rust +home` (with `+osd +toast` as before). The journal says `shell-rust: Home is the Rust shell's: the QML Home window is left
   out of the stack`; `ctl shell` shows `rust.ui.kinds` with `home`, `rust.ui.home.home true` while Home is in front and `rust.wants true`.
3. The D-pad walks the status bar, the cards and the dock with the lift flowing; OK on a tile launches (the launch's `[log] launch` in the
   journal); Back from the app returns to Home drawn by the Rust client; a digit opens a dock tile; a held OK on a card opens the editor
   and on a tile its options.
4. `python3 compositor/tools/tv_gate.py --only shell,frames,refresh 120` with `home-dpad` at 120: zero `burst_over20`; `ctl status` has
   `gl errors 0`; `ctl capture /tmp/home.png` beside `docs/mocks/host-rust/home/36-home.png`.
5. The status bar's chip opens "Who's watching?"; a switch shows the veil and the new person's Home under 300 ms (the journal's
   `profiles: switch to … asked` against the `profiles` topic's `switchedAt`).
6. Off at any point: `ctl shell rust -home`.
