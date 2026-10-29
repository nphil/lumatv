# System layer in Rust: status

Module `rustui/ui/src/system/`. Every kind below is wired in `Layer` (`system/layer.rs`, the only place a kind of this layer is added) and answered by the
Rust client only when the compositor's `--shell-rust` list names it (`ctl shell rust +kind -kind`). The QML surfaces stay the fallback for a kind until it
has passed on the TV twice; nothing has run on the TV.

## Resume here (SystemResume2, 2026-10-29)

| Kind | Piece | State |
|---|---|---|
| `alert` | Alerts, input popup, PIN (`alert.rs`, `alert_surface.rs`) | Done; QML cases ported |
| `saver` | Clock, date, weather, slots and notices under the owner's rules (`saver/`), vote on tvpower's lists, any key wakes it | Done; renders in `docs/mocks/host-rust/system/saver-*` |
| `capture` | Recording dot and the saved/stopped note (`capture.rs`) | Done; the dot is reported in `state.capture.dot` from what the surface drew, and the compositor records only while it is true |
| `popup` | Camera pop-up, stills only (`popup.rs`, `popup_surface.rs`) | Done; live video later through hardware video windows |
| `cards` | Overlay cards (`overlay.rs` model over `cards.rs` and `plugins.rs`, `cards_surface.rs`) | Done; no `door` cards (the QML's test door file) |
| `layout` | Overlay layout editor (`layout_surface.rs` over `lumatv_placement::Editor`) | Done; launch contract below |
| `osd` | Mute mark | Done; a placed mark drifts about its place (four places of a 48 px square leaning into the screen), clear of the pop-up and cards that are up |

**Per-kind compositor switches** (`shell_rust.rs`, `shell.rs`, `screencap/mod.rs`): `saver` refuses the QML client's tvpower vote and reads either client's
`ui_state.saver`; `alert` withholds the three notification feeds; `capture` sends the `capture` topic to the Rust client only and reads `capture.dot`
from its `ui_state`; `cards`, `popup` and `layout` need no withholding of their own: the QML client is told the Rust list in the `rust` topic
(`{"kinds": [...]}`, pushed to both clients) and leaves out its overlay cards (`LumaOverlay.overlaysOn`) and its camera card (`Cameras.popupOff`; its
full viewer is unchanged). `ctl shell show layout [ITEM]` reaches the Rust client as a `show` event with the item as `page`.

### The pop-up and the cards

- Card of the camera window: 560 wide, flush in the corner `layout.overlays.popup` gives (top right by default, ending at x 1856), picture 520 by 292 at
  100 % text and no shorter than 120, the state (`Live`, `Event picture`, `Connecting`, `No picture`) and the keys as glyphs. It waits while the host is busy
  and, with `overApps` false, while an app is in front; steps aside (not dismissed) when the host turns busy; ends at `untilMs` and tells the helper.
- OK and Back are the top card's only 600 ms after it arrived, and the press, its repeats and its release are not seen by the screens under it. The camera
  card is above the cards. OK on the camera card opens the Cameras app's full view when `cameras` is a Rust kind, else it only dismisses (the hint then
  shows one key).
- Cards go under the camera window where the two meet (`Registry::layout_all`), end above 810, at most two; a second card that would not fit waits. A card
  anchored low shows one at a time.

### Layout editor launch contract

- `Shell::open_layout(item: Option<&str>, now) -> Vec<Act>`; `item` is a registry id (`volume`, `mute`, `popup`, `cards`, `toast`).
- Settings' `ShellRequest::Layout { item }` starts it in place (Settings stays open under it, the editor is its own screen at `kit::z::EDITOR`); the
  compositor's `show` event `{"kind": "layout", "item"|"page": ID}` does the same. The report's `can` lists `layout` while the kind is on.
- It takes every key and the pointer while up (`wants`), is left by Done, Back or the Home key, and writes `layout.overlays` whole through `Effect::Theme`
  on each put-down and Reset. The shell then gives every surface the new places and Settings the merged theme, so its rows (`Registry::words`) update.

### Known limits

- A card anchored low shows one at a time (the queue's limit check counts from the top of the column).
- The editor's stand-ins are simplified pictures of the real surfaces at their measured sizes; a text-size change while it is open is not followed until it
  is opened again.
- The toast column is not placed by the anchor's direction (anchored low it would run past its zone).

## TV checks (next round)

1. Saver on Home: clock at 0.4 and dimmer, over 20 minutes the dim reaches 0.4 of that; wake on a key, the pointer and Back. `ctl status` shows
   `oledcare.saver` true while it is up. A camera event while it is up shows no video over it.
2. `ctl shell rust +capture`: start a recording; the dot is top left and `ctl capture` reports `shell.client` as `rust`; stop it, the note reads
   "Recording saved" for 2.6 s; a recording does not start with the client down.
3. `ctl shell rust +popup +cards`: a camera event over Netflix is a card flush at the right edge under the status bar's row, stills about four a second;
   OK expands (with `+cameras`), Back clears; a plugin note or a doorbell is a card under it; the QML client draws neither.
4. `ctl shell show layout popup`: the editor opens on the camera window; move it, OK puts it down, the next event is there; Reset all; Done.
5. Place the mute mark in Settings > Home & look > Layout of pop-ups, mute the receiver: the mark drifts every 10 s and keeps clear of a card.
