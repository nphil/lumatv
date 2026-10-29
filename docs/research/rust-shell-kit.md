# The Rust shell kit: how a screen of LumaTV is built

The kit is what every screen of the Rust shell (`docs/research/rust-shell-plan.md`) is made from. It lives in `rustui/ui/`
(crate `lumatv-ui`, the 64-bit logic process) over LumaShow's toolkit (`ui`: components, glass, glyph text, motion tracks) and
LumaTV's token layer `rustui/tokens/` (crate `lumatv-tokens`). A screen builder reads this file, the toolkit's `ui::components`
docs, and the system layer's two screens in `rustui/ui/src/system/` as the worked example.

## Crates and processes

| Crate | Path | Content |
|---|---|---|
| `lumatv-tokens` | `rustui/tokens` | `theme.json` (shipped under the owner's edits, deep-merged) resolved into the toolkit's `Look` and the TV profile's tokens: roles per mode, type roles, radii, spacing, targets, focus scales, states, the motion set |
| `lumatv-ui` (lib + bin) | `rustui/ui` | `host` (the channel and helper feeds, the key socket), `kit` (this file), `system` (volume display, toasts), `shell` (the system layer as one app), `main` (the set loop and the host harness) |
| `lumatv-placement` | `placement` | where the overlay items sit (OverlayLayout's crate): the volume bar's edges, the toasts' corner, the cards |
| `lumatv-render` | `rustui/render` | LumaShow's renderer; `host.rs` is the engine the compositor runs (feature `ui`) and the harness runs in-process |

On the set the compositor (`compositor/crates/lumatv-compositor/src/shell_rust.rs`) starts `lumatv-ui` with the engine's socket
(`LUMATV_UI_FD`), the key socket (`LUMATV_UI_KEYS_FD`), the UI channel (`LUMATV_SHELL_PORT`, `LUMATV_SHELL_TOKEN_FILE`), the
kinds it answers (`LUMATV_UI_KINDS`, the `--shell-rust` list), the data and module directories and `LUMATV_UI_SYSTEM_Z` (where
stack 1 begins). The process has no GL and no Wayland: its layers are drawn by the engine inside the compositor, in the
compositor's own frame.

## The kit's parts (`rustui/ui/src/kit/`)

**`Kit`** (`kit/mod.rs`) is what a screen is built with: `kit.ui` (the toolkit context: `scene`, `fonts`, `look`), `kit.tokens`,
`kit.mode`, `kit.text_scale`. `Kit::new(scene, tokens, mode, tier, text_scale)` turns glyph text on and warms the TV type roles
(label, caption, body, title, the label in SemiBold), so no screen rasters a glyph on a frame. `kit.ty(TypeRole)` is a role at the
TV's size and the person's text size, `kit.line(role)` its line height, `kit.space(n)` is `Lu.space(n)`, `kit.retheme(tokens, mode,
text_scale)` installs a new look (a new generation; surfaces make their layers again). Components read tokens only.

**Tokens** (`lumatv_tokens::Tokens`): `Tokens::load(data_dir)` (the shipped theme under `<data dir>/theme/theme.json`),
`roles(mode)`, `theme(mode)` (the toolkit's `ResolvedTheme` through its own resolver, so the recipes and the readable track are the
Show's), `look(mode, generation, tier, text_scale)`, `ty(role)`, `spec(role)` (size, weight, line), `radius(Radius)`, `space(n)`,
`target`, `row`, `focus` (tile/card/row/button/press scales, card lift, landing), `states` (focus/hover/selected wash, disabled
opacity), `motion` (press, focus, card, scroll, label, layer, rest, hold ms; reduced; transitions), `blur`, `saturation`,
`extra(mode)` (scrim hide, focused glass edge, `readingAlpha`), `mode(wallpaper_luma)` for `material: auto`, `json()` for a
screen's own block.

**Focus** (`kit/focus.rs`), the one focus engine. `Focus` is pure data: `register(id, frame, region, scope)`, `unregister`,
`place`, `guide(from, dir, to)`, `set_enabled`, `set_inert(scope)`, `open_modal(scope)` / `close_modal()`, `focus(id)`, `ensure()`,
`step(dir) -> Moved`, `next(from, dir)`, `order()`, `remembered(region)`; the pointer: `hover(x, y, moved) -> Hovered`, `hovered()`,
`click(x, y) -> Option<Id>`, `pointer_gone()`, `key()`. Rules: spatial, nearest in the direction pressed among the focusables that
lie beyond the current one in the beam the move casts (they overlap it across the axis); the gap along the axis plus twice the gap
across it, the centres' offset as the tie-breaker; Up and Down may leave the beam into another region (the status bar above the
widgets, the dock below), Left and Right never do, so a row's end holds; no wrap; a region remembers its last focus and a move into
it lands there when it lies in the direction; a guide wins over the search; inert scopes are absent; a modal scope owns input and
gives the focus back. Pointer: hover is a cue only (never moves the focus, opens or picks), click focuses and activates, the next
key hides the cue, a synthesised hover (content moving under a still pointer) brings no cue back.

**Flow** (`kit/flow.rs`), the flowing focus. `Flow::new(scene, Spec, pool)` holds a pool of lift tracks (`POOL` = 8 of the
compositor's 32); `flow.lift(id, Lifted { group, rest, scale, rise, lit })` gives a focusable its lift (the group that holds its
layers at its rest pose with the pivot at its centre, the scale at full lift, the rise in pixels, an optional group whose opacity
carries the lit glass); `flow.go(from, to, now_ns)` settles the outgoing to 0 and the incoming to 1 at the same time, each a
critically damped spring from wherever it is and however fast it is going; `snap`, `tidy(now_ns)` (frees slots at rest), `scroller
(scene, region, group, rest, horizontal, extent)` + `scroll_to(region, offset)` (a row scrolls on the same spring), `settling_at`,
`lift_at(id, at_ns)`, `velocity_at(id, at_ns)`, `in_flight()`. `Spec::default()` is the 260 ms spring (Lucent
`motion.spring.responseMs`); `Spec { reduced: true, .. }` is the crossfade. `kit::Engine { focus, flow }` joins the two:
`engine.step(dir, now_ns)`, `engine.focus(id, now_ns)`, `engine.click(x, y, now_ns)`, `engine.clear(now_ns)`. Every screen uses the
`Engine`; no screen animates its own focus.

**Glyph hints** (`kit/glyph.rs`, `kit/hint.rs`). `glyph::draw(name, side, colour, pixels)` rasters any glyph of
`assets/glyphs/index.json` (and the kit's `volume-x`, `volume`, `info`) into premultiplied pixels; `glyph::known(name)`. `Hint { keys,
action, label }` composes a hint as the rule says (key glyphs, then the action glyph, then a label only where no glyph can say it;
`Hint::key`, `Hint::hold`, `Hint::labelled`); `hint.glyphs()`, `hint.name()` (the accessible name: "OK, expand", "Hold OK,
options"); `HintRow::new(kit, &hint, role, RowAt { x, y, h, z }, ink)` draws one at 1.45 times the role's size; the colour keys in their
colours.

**Motion** (`kit/motion.rs`): `Beat::{Press, Label, Focus, Card, Layer, Scroll, Rest}` name the theme's durations;
`kit.beat_ms(beat)` applies reduced motion; `kit.arrive(layer, to, beat, at_once)` (in with `Ease::Out`, or at once), `kit.leave
(layer, from, beat)` (out with `Ease::In`, hidden at the end), `kit.glide(layer, to, beat)`.

**Panes** (`kit/pane.rs`): `Pane::new(kit, frame, z, PaneKind::{Sheet, Card, Pill, Control}, over_video)` is a glass pane of the
Lucent material (the toolkit's recipe for the mode: edge, lip, shadow) at the theme's radius; over an app's picture the sheet is
painted at `readingAlpha`. `pane.layer()`, `frame()`, `at()`, `move_to`, `restyle(kit, over_video)`. The backdrop is the tint alone
until Home is a layer stack of the engine (a window's buffer is not a layer; the video plane cannot be sampled).

**Text** (`kit/text.rs`): `role_style(kit, role, weight, colour, align)`, `width_of`, `widest` (size a number's box to "100");
`Text::new(kit, frame, z, style, text)`, `Text::line_at(kit, x, cy, w, z, style, text)`, `set(kit, text)`, `set_colour`,
`set_props`, `layer()`, `is_glyph_run()`.

**Screen contract** (`kit/screen.rs`): a screen's model is pure, `impl Model { fn update(&mut self, &Input, now_ms) ->
Vec<Effect>; fn view(&self) -> View; fn due_at(&self) -> Option<u64> }`. `Input::{Topic, Luna, Helper, Fetched, Key, Pointer,
Tick}`; `Effect::{Luna, Cancel, Post, Fetch, Theme, Log, Redraw}`. The answer to a `Fetch` is `Input::Fetched` (a JSON body) or
`Input::Bytes` (a body that is not JSON: a still, a thumbnail); `Effect::Theme { tag, patch }` merges a patch into the owner's
`theme/theme.json` one level deep (`host/theme.rs`: a section's keys are merged, a map inside it is replaced whole, a file that
cannot be read is never taken for an empty one) and answers `Input::Fetched` with the tag and the merged file. The surface draws
the view with the kit; the host (`shell.rs`) owns the feeds and the clock. Models run on wall-clock milliseconds (the helper's
stamps), the flow on the engine's monotonic nanoseconds.

**Icons** (`kit/icon.rs`): `kit.icons.draw(name, side, colour) -> Option<Vec<u8>>`, a Lucide icon of `<module>/icons/lucide` (64 by
64 alpha masks) scaled to `side` and tinted into premultiplied pixels, kept per size and colour; a name that is not in the folder
draws nothing. `icon::layer(kit, name, side, ink, z, at)` makes the image layer, `icon::repaint` paints a new ink or name into one.

**Pictures** (`kit/picture.rs`): `picture::decode(bytes)` reads a JPEG, a BMP (24 or 32 bit, bottom-up or top-down: the set's
`/frame` answer) or a PNG into a premultiplied `Picture`; `fitted(w, h, Fit::{Cover, Contain})` scales it (area mean down,
bilinear up), `round_corners(radius)` antialiases the corners, `paint(&mut ImageLayer)` copies it into a layer made at that size.

**Lint** (`kit/lint.rs`): `tests/qmlhost/layoutaudit.py`'s rules as a check on what a surface says it drew. A surface describes
its view to a `Lint` (`panel(name, rect, paint, safe)`, `word(kit, name, text, frame, style, panel)`) and `findings()` reports
words outside their panel (1 px), crammed against it (8 px at the sides, 4 above and below), on top of other words, off the
centre of a button, panels outside the safe area, and ink under 4.5 to 1 (3 to 1 from 48 px and for ink2) on the panel's paint
over the worst of black, white and mid grey. Host tests run it at 100 % and 200 % text.

## Where a kind draws: z

Everything at the compositor's `system_z` or more is stack 1, over the windows (`compositor/src/ui/mod.rs`). `kit::z` gives each
kind a band of 200 above it: `PANELS` (+100: the quick panel, the picker, Quick Access, the drawer, menus, confirm, the switcher's
chrome), `SETTINGS` (+300), `EDITOR` (+500, the overlay layout editor), `KEYBOARD` (+600), `SYSTEM` (+800: the volume display, the
toasts, the alerts and cards, the capture mark, the saver), so a bar or a toast stays over a panel or Settings. A kind draws at
`system_z + z::<BAND> + its own offsets` and never outside its band. Home (stack 0) is below `system_z`.

## The scene's budget

Every kind of one process draws in the one scene the engine gives it, under the compositor's ceilings (`ui::limits`: 256
layers, 96 groups, 32 motion tracks). The rule: a modal kind (a panel, Settings, the keyboard) builds its layers when it opens
and drops them when it closes; nothing keeps hidden layers alive while another kind is up; the system layer (the volume display
and the toasts) holds at most 16 layers and no groups; the shared `Engine`'s `Flow` holds `POOL` tracks plus one per scroller,
so a kind that needs more room lowers the pool it makes the `Flow` with. `Kit::room()` answers what is left (layers, groups,
tracks) for a kind to assert its budget against when it opens.

## The channel

Topics and events as `shell.rs` serves them (`GET /v1/state`, `GET /v1/events?after=`, `POST /v1/ui`, `/v1/luna`, `/v1/cancel`),
unchanged. The report (`POST /v1/ui`) is `{"can": [...], "wants": bool, "live": bool, "state": {...}}`; the state carries
`kinds`, `osd`, `source`, `level`, `muted`, `mark`, `markPlace`, `logo`, `target`, `toasts`, `live`.

The key socket (`host/keys.rs`): JSON lines from the compositor, `{"key": {"k": "up", "d": true, "r": false, "seat": 0}}` while the
shell's stack has the keys (Home drawn means Home has them; `wants` means the system layer has them; `takes` as before for an
overlay card's OK and Back), and `{"pointer": {"x", "y", "buttons", "visible"}}` samples at the pacer's rate while the pointer is
over a shell layer, `visible: false` once when it goes.

## The host harness

`lumatv-ui --host` runs the shell against the engine in this process on Mesa (the renderer as the compositor runs it, three pages,
damage-only redraw, paced at 60 Hz) and a script or the compositor's stand-in channel:

    lumatv-ui --host [--port N --token-file F] [--helper-port N] [--wait MS] [--push MS:topic:JSON] [--luna MS:TAG:JSON]
              [--helper MS:topic:JSON] [--key MS:name[:up|:repeat]] [--pointer MS:x:y[:buttons[:hidden]]]
              [--eval MS:state|geometry|report|atlas] [--snap MS:FILE.png] [--frames MS:LABEL]
              [--backdrop FILE.png] [--theme JSON] [--mode dark|light] [--text-scale 1..2] [--kinds osd,toast] [--log FILE]

It prints `[report]`, `[luna]`, `[helper]`, `[eval]`, `[frames] LABEL N`, `shot FILE` and, with `--log`, a `[frame]` line per frame
(damage, pixels drawn, layers, CPU and GPU time), then `RESULT frames N ... drew X % of the panel per frame, text pictures P`.
`tests/test_rust_shell.py` drives it through `tests/qmlhost/channel.py`, the stand-in the QML tests use, with the same cases.

## Building a screen

1. Model first (`system/<name>.rs`): the state, `update`, `view`, `due_at`, with host tests of the behaviour the QML tests state.
2. Surface (`system/<name>_surface.rs`): layers made once per layout (hidden), moved, shown and restyled from then on; panes from
   `Pane`, text from `Text`, marks from `glyph`, placement from `lumatv-placement` for anything that sits in an overlay zone.
3. Focus: register every focusable with the `Engine`, give the ones that lift a `Lifted`, and route D-pad keys through
   `engine.step`; hints through `HintRow`.
4. Wire it in `shell.rs` (a kind name, the feeds it needs, its part of the report), add the kind to the compositor's switch, and
   render its scenes dark and light with the harness beside the mocks.

## The flowing focus (a Lucent motion rule of the TV profile, 2026-10-29)

A focus move on the TV is one continuous motion: the outgoing and the incoming focusables move at the same time, each on a
critically damped spring (natural period 260 ms, landing in about 270 ms, 33 frames at 120 Hz) from wherever it is and however
fast it is going, so a held or a rapid D-pad retargets the springs mid-flight and the motion never stops and starts; the lift is
one value shared by every kind of focusable (tile, card, row, button, dock icon), scaled by each kind's own numbers, so it hands over
between shapes and sizes; a row that scrolls to show the focus scrolls on the same spring; no frame shows the old and the new both at
rest between moves. Reduced motion: no scale and no rise, the lit state crossfades within the reduced-motion ceiling. The tracks
are the compositor's, evaluated at its predicted present time, so a frame of the motion costs this process nothing. The tilt
of tvOS is not in it (nothing measured at 120 Hz yet says it is cheap). `rustui/ui/tests/flow.rs` holds the measurement on the
mirrors of the tracks: at no frame are both tiles at rest between moves, and a retarget keeps the velocity.
