# Rust UI gaps: what is QML-only, what moved, what is left

The Rust shell now has a kind for every user-visible QML screen except the extras Home draws around itself (launch screen, Quick Access, on-screen
remote, voice bar, mini frame, wallpaper video). This round added the `ai` and `banner` kinds, fixed the three leftovers of the system layer, and wrote
the models of five of the six extras. The extras are not drawn yet.

## Inventory

Paths are under `payload/compositor/lumatv/` unless noted. "Guard" is the QML-side switch that hides a screen when its kind is on.

| QML | Rust kind | Status |
|---|---|---|
| `panels/Notify.qml` | `toast`, `alert` | done earlier |
| `panels/VolumeOsd.qml` | `osd` | done earlier |
| `panels/Screensaver.qml`, `SaverVote.qml`, `ArtPiece.qml`, `ArtDeck.js` | `saver` | done earlier |
| `panels/SaverVideo.qml` | `saver` | not checked one by one |
| `panels/CaptureMark.qml` | `capture` | done earlier |
| `panels/Overlays.qml`, `OverlayCard.qml`, `Overlays.js` | `cards` | done earlier |
| `panels/Cameras.qml`, `kit/CameraFrame.qml` | `popup` (pop-up), `cameras` (viewer) | done earlier |
| `panels/Quick*.qml`, `QuickArrange.js` | `quick` | done earlier |
| `panels/InputPicker.qml` | `picker` | done earlier |
| `panels/Drawer.qml` | `drawer` | done earlier |
| `panels/ContextMenu.qml` | `menu` | done earlier |
| `panels/Confirm.qml` | `confirm` | done earlier |
| `panels/Switcher.qml`, `SwitcherLayout.js` | `switcher` | done earlier |
| `panels/Keyboard.qml`, `KeyboardLayouts.js` | `keyboard` | done earlier (PanelsFinish) |
| `Settings.qml`, `settings/*` | `settings` | done earlier |
| `LayoutEdit.qml` | `home` editor, `layout` (overlay editor) | done earlier |
| `LumaHome.qml`, `Dock.qml`, `Tile.qml`, `StatusBar.qml`, `StatusItem.qml`, `Widget.qml`, `widgets/*`, `panels/AppsGrid.qml`, `panels/Search.qml`, `LayoutGrid.js`, `Tint.js`, `Saved.js` | `home` | done earlier, except the rows below |
| `AiPrompt.qml` | `ai` | **done**: guard `drawn` |
| `panels/Banner.qml`, `BannerButton.qml` | `banner` | **done**: guard `rustBanner` in `LumaOverlay.qml` |
| `panels/Splash.qml` | `home` (launch screen) | model written, no surface. The spinner and hold ring have no caller in the QML and are not to be ported |
| `panels/QuickAccess.qml` | `home` | model written, no surface; the digit logic is not in the Rust Home |
| `panels/ScreenRemote.qml` | `home` | model written, no surface |
| `panels/VoiceBar.qml` | `home` | model written, no surface |
| `MiniFrame.qml` | `home` | model written, no surface; a news item still opens the browser |
| `WallVideo.qml`, `WallVideo.js` | `home` | not started: the Rust Home fades its wallpaper on the `wall` topic and never asks the player to play |
| `KitGallery.qml` | none | development only, skipped |
| `Layers.qml`, `HomePanels.qml`, `PanelMotion.qml`, `Warmup.qml`, `Resident.qml`, `LumaKeyBridge.qml`, `LumaBus.js`, `Glass.qml` | none | QML window glue, warm-up and residency; they go with the QML shell |
| `kit/*.qml`, `kit/*.js` | `host/*.rs`, `kit/*.rs` | services and widgets are replaced one for one |
| `logo/*.qml` (13 files) | none | used only by LG's BootLogo and MasterBrand windows under LG's compositor; the LumaTV compositor draws no power-on or power-off logo. Out of scope |
| `compositor/shell/Shell.qml` | compositor `shell_rust.rs` | stays: it is the fallback and LG's path |
| `payload/systemui`, `cover`, `splash`, `notification`, `voice`, `inputcommon`, `StarfishFullscreenContainer.qml` | none | restyle patches for LG's own compositor, not LumaTV's path |

Gaps found in the Rust Home besides the extras: a Search answer that is a Settings page is only logged (the search is not given the pages, and
`Request` has no page), and the status bar's settings popover (live wallpaper sliders) opens Settings instead.

## Done

- **System layer leftovers** (`0cddf37`): the toast column is placed by its anchor and the room of its zone; the cards queue is asked with the column's
  room and stacks upward from a low anchor; the layout editor re-measures its stand-ins when the text size changes. Described in
  `system-layer-result.md`, "Placement of the columns".
- **`ai`** (`a09ddfd`, `6478b3d`): the AI access prompt over everything, Home, an app or the saver. No key or click of the shell answers it (the helper
  reads OK and Back off the remote's own device), the pointer cannot reach what is under it, and no panel opens over it. The hint rows are glyphs with
  what each key would do ("Allow once", "Deny"). `AiPrompt.qml` draws only when `ai` is not a Rust kind.
- **`banner`** (`4f283ff`): the input pill, the channel bar and the no-signal screen. The pill shows with theme `banners.own` (off by default, LG's
  restyled banners draw the same moment); the other two come from the `banner.show` topic or `ctl shell show banner pill|bar|nosignal`. The no-signal
  screen is always the dark look, its block hops every minute, its Inputs and Picture buttons open the Rust picker and quick panel when those are on
  and LG's Home Hub and Settings otherwise. Buttons are the kit's: hover is a cue, a click selects.
- **The models of the extras** (`system/extras.rs`, 12 behaviour tests): launch screen timing and the compositor's launch-origin notes, Quick Access
  toast and question, the on-screen remote, push-to-talk voice, the mini frame. The remote's key list has no OK, Back or arrow and a test holds it to
  that (Principle 0).

## Resume here

Order matters; each step is testable alone.

1. **Surfaces**: `system/extras_surface.rs`, after `ai_surface.rs` and `banner_surface.rs` (a `Root` per sheet, rebuilt when its signature changes).
   Specs from the QML: launch screen is `scrim_hide` over everything, a 180 px icon (radius 38), the name as Title, a 28 px ring and "Opening"; toast
   at the top right (`x = 1920 − 64 − w`, `y = 137`), a 64 px digit plate, "N · Name" and the line, 2.4 s; question 820 wide with four 140 px picks,
   More and Not now; remote pane at `x 64`, `y 656`, 1792 × 360 with the keys at `remote_box(i)` from `x 104`, `y 752`; voice bar 760 × 110 at
   `x 64`, 56 from the bottom, and a 72 px mic mark at the bottom right while the microphone is open; frame 1520 × 860 with a 64 px bar. No
   instruction text: the remote and bar take glyph hints (D-pad, OK, Back, `Hint::hold(Key::Mic, …)`), not the QML's sentences. `rewind` has no Lucide
   mask: mirror `fast-forward`. A group pose has no rotation, so the ring is a dozen dots stepped every `SPIN_MS`.
2. **`Layer`**: one `extras` field with the usual touchpoints (`update`, `wants`, `busy`, `apply`, `tick`, `due_at`, `state`, `geometry`,
   `feed_lint`, `restyle`, `on_key`, `pointer`), gated on `kinds.home`. No new compositor kind: the extras follow `home`, so `ctl shell rust -home`
   gives all of them back to the QML together.
3. **Home** (`home/model.rs`): a `notes` list with `take_notes()`. `launch()` gains the icon and pushes `Note::Launch` with the tile rectangle (dock
   `slot(i)` plus `dock_rect()` minus `scroll_px()`, grown by `focus.tileScale` for the focused tile, radius `Radius::Tile`). `digit()` follows
   `settings/QuickAccess.js` when `remote.quickAccess.on` (`target`, `suggestions`, `chosen`; the choice is written with `Effect::Theme` and
   `{remote: {quickAccess}}`). The key name `screenremote` pushes `Note::Remote`. A news item's OK pushes `Note::Frame` (`to` = 1520 × 860, centred
   under the status bar, `y = 150` in the shipped layout). `Intent::Assign` becomes `launch` plus a toast "Assigned. Opening."; `Intent::More` asks
   Settings for page `remote` (`Request` must stop being `Copy` and gain `SettingsPage(String)`; the same fixes Search's Settings pages).
4. **Shell** (`shell.rs`): the mic key goes to `extras.voice_key` before Home sees it; `report()["takes"]` adds `voice` while `takes_voice()`, and
   `screenremote` and `search` while Home is in front; the notes and intents are handed over after each input; `extras.start()` joins the start
   effects; the `transitions` topic already reaches the client.
5. **Compositor keys**: the evdev codes of the mic, on-screen remote and Search keys are unknown (no TV access). Teach them by name: `keys.conf`
   lines `voice N`, `screenremote N`, `search N` fill a second list in `KeyTable` (`shell_name(code)`; `knows()` and `named()` include it);
   `shell_rust_takes_key` then uses `own_key_name(code).or(table.shell_name(code))` and sends those names only when the client lists them in
   `takes`, so a mic key that is not ready stays LG's. The journal line "input: key N has no name in the key table" names unknown codes, but only
   from 768 up.
6. **Wallpaper video**: port `WallVideo.js` as a controller in the Rust Home: the mailbox `/tmp/<shortId>-vplane/req.json` and `status.json`, the
   manifest `<data>/vplane/clips/wall.json`. It needs file effects outside the data directory.
7. **Tests**: scenes and renders in `tests/test_rust_gaps.py` at 100 % and 200 %, dark and light, as for `ai` and `banner`; then the guard in
   `LumaHome.qml` that hides the QML extras when `home` is a Rust kind (they vanish anyway while the QML Home is out of the stack).

## TV checks

1. `ctl shell rust +ai +banner`. `ctl shell show banner pill`, `bar`, `nosignal`: the pill and bar sit at the foot, the no-signal screen is black with a dim
   block that hops after a minute. Hover over a button shows a cue, a click selects. Inputs opens the Rust picker, Picture the quick panel.
2. Switch theme `banners.own` on, change input: the pill shows for four seconds with the port name. Off: nothing of LumaTV's shows.
3. With `ai` on, raise an AI prompt (helper topic `ai.prompt`) over Home, over an app and over the saver. Pressing OK or Back on the physical remote
   answers it; a click on the screen and the on-screen remote do not. Nothing opens over it.
4. Toast column from a bottom corner stacks upward; a low anchor's cards fit in the room left; raising the text size in the layout editor keeps the
   stand-ins whole.
5. Each kind twice before any QML below is deleted. A theme edit made by the QML Settings reaches the Rust client at its next start (`banners.own`).

## Deletable QML

Delete a row's files only after its kind has passed the TV twice, and only once LumaTV stops offering the QML fallback (`ctl shell rust -kind`) and
LG's compositor path no longer loads the module.

| Kind | QML |
|---|---|
| `toast`, `alert` | `panels/Notify.qml` |
| `osd` | `panels/VolumeOsd.qml` |
| `saver` | `panels/Screensaver.qml`, `SaverVote.qml`, `ArtPiece.qml`, `SaverVideo.qml`, `ArtDeck.js` |
| `capture` | `panels/CaptureMark.qml` |
| `cards` | `panels/Overlays.qml`, `OverlayCard.qml`, `Overlays.js` |
| `popup`, `cameras` | `panels/Cameras.qml`, `kit/CameraFrame.qml` |
| `quick` | `panels/Quick*.qml`, `QuickArrange.js` |
| `picker`, `drawer`, `menu`, `confirm`, `switcher` | `panels/InputPicker.qml`, `Drawer.qml`, `ContextMenu.qml`, `Confirm.qml`, `Switcher.qml`, `SwitcherLayout.js` |
| `keyboard` | `panels/Keyboard.qml`, `KeyboardLayouts.js` |
| `settings` | `Settings.qml`, `settings/*` |
| `layout` | the overlay part of `LayoutEdit.qml` |
| `ai` | `AiPrompt.qml`, its `drawn` guard in `LumaHome.qml`, the `rustKinds` binding in `compositor/shell/Shell.qml` |
| `banner` | `panels/Banner.qml`, `BannerButton.qml`, `showBanner` and `rustBanner` in `LumaOverlay.qml` |
| `home` (after the extras) | `LumaHome.qml`, `Dock.qml`, `Tile.qml`, `StatusBar.qml`, `StatusItem.qml`, `Widget.qml`, `widgets/*`, `panels/AppsGrid.qml`, `Search.qml`, `Splash.qml`, `QuickAccess.qml`, `ScreenRemote.qml`, `VoiceBar.qml`, `MiniFrame.qml`, `WallVideo.*`, `LayoutEdit.qml`, `LayoutGrid.js`, `Tint.js`, `Saved.js`, `Glass.qml`, `HomePanels.qml`, `Layers.qml`, `PanelMotion.qml`, `Warmup.qml`, `Resident.qml`, `LumaKeyBridge.qml`, `LumaBus.js`, `kit/*` |

`KitGallery.qml` is development only and can go at any time. `logo/*` stays until the LG BootLogo and MasterBrand windows are replaced.
