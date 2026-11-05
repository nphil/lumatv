# Layout of the pop-ups (OverlayLayout, 2026-10-29)

Built on the offline host: the model, the editor's controller and a helper API in Rust (`placement/`, `helper/app/src/placement.rs`), the mocks of the editor,
and the fix for the camera pop-up's place in the QML shell. Nothing has run on the TV. The owner places the volume bar, the mute mark, the camera window, the
notification cards and the toasts himself, in **Settings › Home & look › Layout of pop-ups**, on a picture of the screen, with the D-pad and no numbers. The
screen that draws it is the Rust shell's (below).

## Resume here

- **Ships tonight (QML, tiny).** The camera pop-up and the cards under it end where Home's right edge does: `theme.json` `overlays.right` 96 to 64 (Home's
  `layout.json` `safe.x`), read by `panels/Cameras.qml` as well as by the cards, so the two cannot drift apart again; and the cards' and the pop-up's slide is
  animated (`panels/Cameras.qml`, `panels/OverlayCard.qml`). Check list below.
- **Built, in Rust.** `placement/` (crate `lumatv-placement`, serde only, 40 host tests): the registry of items, the zones, placements as an anchor and offsets,
  `layout.overlays` in `theme.json`, stepping and stops, the volume bar's three edges, stacking, overlaps, the mute mark's drift, and the editor's controller
  with the view a toolkit draws. `helper/app/src/placement.rs`: the same over `/v1/placement` for a toolkit that cannot link the crate.
- **Pending for the Rust shell** (the UI pieces; no new QML is written): (1) the editor screen, drawn from `Editor::view` (design: `docs/mocks/overlay-layout/`);
  (2) the Settings row and its child page (one row per registry item, valued with `Registry::words`, "Edit in preview", "Reset all"); (3) the surfaces reading their
  place from `Registry::rect_of`/`layout_all`: the volume bar on its edges (vertical on the sides, as wide as its mark needs), the mute mark's drift, the camera
  window, the cards and the toasts, each arriving from `slide_of`. Until then the QML surfaces keep the fixed places they have.
- **Decisions for Main.** (1) The toasts' default stays 24 px from the corner, as the QML shell draws them; LG's own toast view and `docs/mocks/07-toast.html` put the
  toast at 64 from the right and 137 from the top. One field in the registry (`toast` zone) moves it. (2) The capture mark (a privacy indicator), the full-view
  camera chrome and banners are not items. (3) Under LG's compositor the toasts are LG's view and are not placeable.

## Why the camera window was not in the corner

Two safe areas. The overlay's cards and the camera pop-up were laid out from the Lucent 5 % margin (`overlays.right` 96, and a literal 96 in `Cameras.qml`),
while Home's status bar, widgets and dock end at `layout.json` `safe.x` 64. The card's right edge was 1824 under a status bar and a dock that end at 1856: 32 px
short of the corner it is meant to sit in. Its top (134) already followed Home's content area (54 + 60 + 20); only the right edge did not. The slide it was meant
to have ("in from a touch to the right") was a `Translate` with no animation: it appeared in place and, going, jumped 24 px while it faded. The toast of LG's own view
(`tools/notification_patch.py`) already ends at 64, the house convention. `placement/tests/shipped.rs` holds the data to the model: `overlays.right`,
`overlays.top`, `overlays.gap`, `overlays.width` and `overlays.limit` of the shipped theme equal Home's layout, so the model and the QML cannot part.

## The model (`placement/src/model.rs`)

An item is placed by an anchor and offsets, never by pixels: `{"anchor": "top-right", "dx": 0, "dy": 0}`. The anchor is one of nine places on the item's zone (the four
corners, the middle of the four edges, the middle); `dx` and `dy` are fractions (-1 to 1) of the room the item has left in the zone (the zone less its own size). An item
flush in a corner is flush whatever its size, at any text size, in any toolkit, and cannot leave its zone. Offsets are kept to four decimals and the anchor is the
nearest of the nine, so a file reads "top right, a little lower". An item with no entry, an entry that is not whole and valid, or one equal to the default, is at its
default. The zones are the rules the overlays kept: nothing in the status bar's row (top 134), nothing in the bottom quarter (810, `overlays.limit`), edges at
Home's own 64 (the toasts' zone at their own 24).

| Item (id) | Zone | Default (where it was) | Notes |
|---|---|---|---|
| Volume bar (`volume`) | x 64 to 1856, y 134 to 1016 | bottom edge, centred, 64 above the edge; 120 higher over an app | left, bottom or right edge and slides along it; vertical on the sides; only the bottom edge rises over an app (subtitles at 905 to 1010) |
| Mute mark (`mute`) | x 64 to 1856, y 134 to 810 | not placed: the four corners over an app, the status bar's row over Home | placed, it visits four places of a 48 px square that leans into the screen, in turn, every 10 s, never under a card that is up |
| Camera pop-up (`popup`) | the column: x 32 to 1888, y 32 to 810 (half of Home's larger safe margin from the edges, never under 24; 2026-10-30, "much less padding") | top right, 560 wide, 469 tall at 100 % text | ends at x 1888 (was 1856); not held to the lint's safe area, like the toasts' corner |
| Notifications (`cards`) | the column | top right; under the window where they meet (above it when anchored low) | a card that does not fit beside the window waits (`waits`) |
| Toasts (`toast`) | x 24 to 1896, y 24 to 810 | top right, 24 from the edges | up to three |

`Registry::layout_all(user, frame, sizes, work, over_app)` gives every item's rectangle from what the owner placed, the frame (`Frame::from_json(layout.json, theme overlays.limit)`)
and the sizes the toolkit measured (`Sizes`; the registry's samples where it measured none: volume 440 by 60, mute 64, camera window 560 by 469, cards 560 by 165, toast 560 by
98 at 100 % text). A surface asks `rect_of(id, user, frame, size)` for its own place and keeps no margin of its own. Also: `lift_over` (the volume bar's rise over an app),
`slide_of` (the edge an item arrives from), `drift_spots`/`drift_box` (the mark's places), `words` ("Top right", "Left edge, lower third", "Corners", "..., moved"),
`blocker` (the item a move would land on), `column_room_for` (the room the cards have beside the window).

## The editor's controller (`placement/src/editor.rs`)

`Editor` holds the state of the screen (the chosen item, what is in the hand, the buttons) and takes keys and a pointer; a toolkit draws `Editor::view(now)` and
forwards input. It never saves: a drop or a Reset returns `Effect::Save(<layout.overlays, the whole map>)` and the caller writes it to its theme (a theme patch merges one level
deep, and the key is personal: it goes where the caller keeps a profile's own keys); `Effect::Close` leaves.

- Browse: Left/Right choose an item, OK picks it up, Down goes to the buttons (Reset this, Reset all, Done), Back leaves.
- Move: one press is a fine step of 12 px; a tap stops on the next corner, third or middle when one is in the way (or within half a step beyond it); a held key (presses under 220 ms
  apart) glides over them, faster the longer it is held, and stops flush at the zone's edge. OK puts the item down and saves; Back puts it back.
- The volume bar slides along its edge; at the end of an edge one more press (a press of its own, not a held key's) turns the corner onto the next edge, and the opposite key flips a
  side edge to the other side. That is how the edge is chosen: it becomes vertical on the sides and horizontal at the bottom.
- An item is never put on another (8 px of air): the move is refused and the notice names the one in the way ("Camera window is in the way", gone after 2.6 s). A move may leave an
  overlap the item started with (text at 200 % makes items taller than the room between them) and never makes a new one. The cards and the window, which stack by design, are not in each
  other's way. Nothing goes into the bottom quarter: the zones keep it.
- A pointer (screen pixels): `pointer_down(item, x, y)`, `pointer_move(x, y, pressed)`, `pointer_up()`: it takes the item after 12 px, the item follows it (a few pixels from a stop it lands on the stop)
  and is put down where the pointer lets go. `press_button(index)` is a pointer on a button.
- `View` carries everything to draw: the guides (safe area, status bar's row, the bottom quarter, the subtitles' band), each item's rectangle, words, `custom`, `waits` (draw it faint),
  `inHand`, `flagged`, `reach` (the box the mark visits), where the item in the hand was, the panel's copy (heading, "Camera window  ·  Top right, moved", the keys line), the buttons and
  whether each is open, and `panel_spot(w, h)` for where the toolkit's panel goes: the first place nearest the middle that no item is in or beside, the item in the hand first.

## The helper API (`helper/app/src/placement.rs`)

JSON over the helper's loopback API, for a toolkit that cannot link the crate. Nothing is stored: the caller sends what the owner placed and writes what `effects` ask.

| Request | Answer |
|---|---|
| `GET /v1/placement` | the registry (items, constants of the rules) and the default frame |
| `POST /v1/placement/layout` `{layout?, limit?, user?, sizes?, overApp?, work?, avoid?}` | every item's `rect`, `placement`, `custom`, `waits`, `words`, `slide`, `lift`, and `spots` for a placed mark |
| `POST /v1/placement/editor/open` `{layout?, limit?, user?, sizes?, item?, panel?}` | `{view, effects, panel?}`; one editor session for the TV |
| `GET /v1/placement/editor` | the same as it is now; 409 `no_editor` when it is not open |
| `POST /v1/placement/editor/key` `{key, t?}` | `ok`, `back`, `left`, `right`, `up`, `down`; `effects` is `[{save}]`, `[{close}]` or empty |
| `POST /v1/placement/editor/pointer` `{phase, item?, x, y, pressed?, t?}` | `down`, `move`, `up` |
| `POST /v1/placement/editor/button`, `/sizes`, `/close` | a press on a button by its place in the view's `buttons` (Reset this, Reset all, Done; the status bar has Full width before them); sizes measured again; close |

## Adding an overlay

One entry in `ITEMS` of `placement/src/model.rs` (id, title, glyph, zone, default, sample size) and a surface that puts itself where `rect_of(id, ...)` says. The editor's list, the page's row,
the file, the overlap rule, the stepping and the words come with the entry (a test adds an item to a copy of the registry and moves it).

## Burn-in

The mute mark is the only overlay that stays: placed, it never holds one pixel; `drift_spots` gives four places of a 48 px square leaning into the screen, visited in turn every 10 s (the
surface's timer), never under a card that is up (`avoid`). Not placed, it keeps its four corners. The compositor's own orbit (`oledcare`) applies to whatever place an item is in.

## Tests

- `placement/`: `./build.sh test` (40: model 24, editor 13, shipped data 3; clippy `-D warnings`). The model's rules are one test each (flush corners at any size, defaults equal to where
  every item was, the file keeps only what differs, stepping and stops, the volume bar's edges and corners, stacking and waiting, overlaps and what an item may leave, drift, a
  pointer's drop, words, a new item). The editor's: pick up, move, put down and the saved map, Back, a held key to the edge, refusals and their words, the corner turned, Reset this and
  Reset all through the buttons, Done and Back, a pointer drag and a refused one, the panel's place, a map changed under it. The shipped data: the model's fallbacks and the QML's margins equal
  Home's layout.
- `helper/`: `placement::tests` (the registry, the layout for sent numbers, the editor over keys and a pointer, refusals, the route table).
- QML host: `test_camera_popup` (the card's right edge is 1856 and it ends above 810), `test_overlays` (the cards end at 1856).

## TV check list (not run)

1. A camera event over Home (`POST /v1/cameras/event`, `ha-cameras-result.md` check 4): the card's right edge is level with the status bar's last icon and the dock's right edge (x 1856), top 134; it
   slides in from the right and, going, slides out without a 24 px jump.
2. The same over Netflix; a door card (`{"card":...}`) under it ends at the same edge.
3. `gate.py --surfaces overlay-netflix`: no late frame beyond the baseline (a slide runs only while the card comes and goes).
4. After the helper is flashed: `GET /v1/placement` answers the registry, `POST /v1/placement/layout` with `{}` answers the popup at `{"x":1296,"y":134,"w":560,"h":469}`.

## Not built

- The editor screen, the Settings row and the surfaces' own placement are the Rust shell's (no new QML). A QML editor was built and host-tested first; it is not shipped, and `docs/mocks/overlay-layout/`
  and `Editor::view` carry its design.
- Placing LG's own toasts under LG's compositor (the view is LG's, its place is fixed when the patch is staged).

## Changelog line

Camera pop-up and notification cards: end where Home's right edge does (64 px from the screen's, were 96) and slide in and out as designed. New `placement/` crate and `/v1/placement` API for the layout of the pop-ups (volume bar on the left, bottom or right edge, mute mark, camera window, cards, toasts): anchors and offsets in fractions of the room, no overlaps, nothing in the bottom quarter, the editor's controller; the screen that draws it is the Rust shell's.
