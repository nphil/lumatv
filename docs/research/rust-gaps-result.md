# Rust UI gaps: what is QML-only, what moved, what is left

The Rust shell has a kind for every user-visible QML screen. With `home` on, the QML Home is out of the stack and everything it drew around itself is
drawn by the Rust shell too: the launch screen, Quick Access, the on-screen remote, the push-to-talk voice bar and its microphone mark, the mini frame
and the moving wallpaper's controller. A Search answer that is a Settings page opens that page in the Rust settings kind, and the status bar's gear
opens the wallpaper popover as the QML's did. What is left needs the TV: the key codes of the mic, on-screen remote and Search keys, and the checks below.

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
| `panels/Splash.qml` | `home` (launch screen) | **done** (`extras_surface/splash.rs`). The spinner and hold ring have no caller in the QML and are not ported |
| `panels/QuickAccess.qml`, `settings/QuickAccess.js` | `home` | **done** (`extras_surface/access.rs`; the digit logic is in `home/model.rs`) |
| `panels/ScreenRemote.qml` | `home` | **done** (`extras_surface/remote.rs`) |
| `panels/VoiceBar.qml` | `home` | **done** (`extras_surface/voice.rs`: the bar and the mark) |
| `MiniFrame.qml` | `home` | **done** (`extras_surface/frame.rs`); a news story opens it instead of the browser |
| `WallVideo.qml`, `WallVideo.js` | `home` | **done** (`home/wallvideo.rs`) |
| Search's `pages` source (`Layers.settingsPages`, `LumaHome.searchPicked`) | `home` | **done**: the shell gives Search Settings' pages when it opens and a picked page opens in the settings kind |
| `StatusBar.qml`'s settings popover | `home` | **done** (`home/popover.rs`, `PopoverSurface`). Its profile popover is the picker (`Layer::Picker`) |
| `KitGallery.qml` | none | development only, skipped |
| `Layers.qml`, `HomePanels.qml`, `PanelMotion.qml`, `Warmup.qml`, `Resident.qml`, `LumaKeyBridge.qml`, `LumaBus.js`, `Glass.qml` | none | QML window glue, warm-up and residency; they go with the QML shell |
| `kit/*.qml`, `kit/*.js` | `host/*.rs`, `kit/*.rs` | services and widgets are replaced one for one |
| `logo/*.qml` (13 files) | none | used only by LG's BootLogo and MasterBrand windows under LG's compositor; the LumaTV compositor draws no power-on or power-off logo. Out of scope |
| `compositor/shell/Shell.qml` | compositor `shell_rust.rs` | stays: it is the fallback and LG's path |
| `payload/systemui`, `cover`, `splash`, `notification`, `voice`, `inputcommon`, `StarfishFullscreenContainer.qml` | none | restyle patches for LG's own compositor, not LumaTV's path |

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
- **The extras drawn and wired** (`system/extras_surface/`, one file per screen, a `Layer` field gated on `kinds.home`). They hang in Home's stack
  (below `system_z`): over Home and its layers, under every panel, and under the windows the compositor puts over Home (the mini frame's web half paints
  into the room the frame leaves clear). Home hands them notes after every input (`take_notes`: launch with the tile's rectangle grown by the focus
  lift, the digit's toast and question, the remote key, a story); what they ask of Home comes back as intents (`Assign` keeps the choice in
  `remote.quickAccess` and opens the app, `More` opens Settings at `remote`). While a question, the remote, the frame or the voice bar is up the
  extras have the keys and the pointer; the lint reads only them (like the entry sheet over Settings). Each is checked at 100 % and 200 % text in
  both looks (`docs/mocks/host-rust/gaps/`): the question's cards and the remote's pad grow with the text so no word is broken or crammed, the
  remote's sheet grows upward, the D-pad glyph is the first hint to give way. Hints are glyphs (D-pad, OK, Back, the voice key held), never
  sentences; hover is a cue, a click selects. The ring that turns is a dozen dots stepped in place every 75 ms (a group pose has no rotation).
  Nothing on the remote can say OK, Back or an arrow, and the question's and the bar's clicks only choose an app or end the microphone.
- **Keys** (`shell.rs`, compositor `keys.rs`, `shell_rust.rs`): the mic key goes to the extras before Home sees it; the report's `takes` adds `voice`
  while the helper can listen or a request is under way, and `screenremote` and `search` while Home is in front. The compositor learns the codes from
  `compositor/keys.conf` lines `voice N`, `screenremote N`, `search N` and sends those names only while the client lists them.
- **The moving wallpaper** (`home/wallvideo.rs`): the QML controller's state machine and timers (start within 6 s, poll 200/500 ms, renew 10 s, settle
  250 ms, stop 250 ms after the clip is let go, ask again every 15 s when no player, manifest every 5 s while the switch is on), the clip pick by the
  wallpaper, the look and the dark scrim (`wall.json`), and the mailbox files as effects confined to `/tmp/<shortId>-vplane/` (`Shell::file_of`; a test
  holds the directory to brand.json's short id). Home's own picture and scrim go only when `want && playing && settled && hole` (`WallVideo::up`),
  so a compositor hole without a playing clip no longer blanks Home, and they are back in the same update the clip stops being wanted. The
  compositor's side of the contract was already pinned (`luna::video::placement` sends `setDisplayWindow` with `originalInput`, the hole is the whole
  screen but Home's pixels, the `wall` topic); it is not changed. With the default `wallpaperMotion.source: gpu` nothing of the controller costs anything.
  The clip's glass alpha (0.80) is the shipped `readingAlpha` (0.80), so the panes' tint over the plane is the clip's; the QML's `backdrop` glass mode
  (blur of the still) is not ported: a hidden picture cannot be blurred.
- **Search pages** and **the settings popover**: see the table. The popover's rows are the QML's with their ranges and steps; the GPU wallpaper of the
  Rust shell carries the amount, the speed and the frame rate, the other sliders are written to the theme as the QML wrote them and decide the clip.

## Resume here

Needs the TV (no TV access when this was written), in this order:

1. **Teach the keys.** Press the mic, on-screen remote and Search buttons with `ctl journal` open and read `input: key N has no name in the key table`
   (it names codes from 768 up only; a lower code shows in `ctl keys` after a press with the journal's trace). Add `voice N`, `screenremote N` and
   `search N` to `/var/lib/lumatv/compositor/keys.conf` and start a session. Until then the three keys stay LG's, as they were under the Rust Home.
2. **TV checks** below, each twice, then the deletions in the table.
3. If the set's Magic Remote has no on-screen-remote or Search button, the checks for them are the harness's (`ctl shell show` has no door for them: press
   the Rust `screenremote` key by teaching any spare button).

## TV checks

1. `ctl shell rust +ai +banner`. `ctl shell show banner pill`, `bar`, `nosignal`: the pill and bar sit at the foot, the no-signal screen is black with a dim
   block that hops after a minute. Hover over a button shows a cue, a click selects. Inputs opens the Rust picker, Picture the quick panel.
2. Switch theme `banners.own` on, change input: the pill shows for four seconds with the port name. Off: nothing of LumaTV's shows.
3. With `ai` on, raise an AI prompt (helper topic `ai.prompt`) over Home, over an app and over the saver. Pressing OK or Back on the physical remote
   answers it; a click on the screen and the on-screen remote do not. Nothing opens over it.
4. Toast column from a bottom corner stacks upward; a low anchor's cards fit in the room left; raising the text size in the layout editor keeps the
   stand-ins whole.
5. **Launch screen**: open an app from the dock with the transitions on: the tile is what the card grows from, a launch the app answers inside 400 ms shows
   no screen, a slow one shows the icon, the name and the ring, and the card then grows out of the icon. With the transitions off it is there at once.
6. **Quick Access** (theme `remote.quickAccess.on`): a digit with a tile says "N Name / Opening." at the top right for 2.4 s; one with nothing behind it
   asks, hides the page, answers by the D-pad or a click; the choice is in `theme.json` and survives a restart; More opens Settings at Remote.
7. **On-screen remote** (the key taught as `screenremote`): opens over a quiet Home only, D-pad walks it, OK presses (play, the digits, the colours move the
   TV), Back or the key again closes it. No key of it moves the focus of a prompt: with an `ai.prompt` up it is not reachable at all.
8. **Voice bar** (the key taught as `voice`, the helper's `voice` topic ready): hold the key, the bar and the mark show, release works it out, the words
   land in Search. Back ends the microphone. With voice search off the key stays LG's.
9. **Mini frame**: OK on a news story grows the frame out of its card, the web half paints into the room under its bar, the ring goes when it is up,
   Back closes both.
10. **Moving wallpaper** (`lumatv wallpaper video`, after the steps of `videoplane-saver-result.md`): `ctl shell state` shows `home.wall` going
    `starting`, `playing`, `up: true`; Home's own picture goes only then; switching to `gpu` brings it back in the same frame and the player stops
    250 ms later. A look moved from the popover's amount or speed row draws the GPU wallpaper (the pick names the key).
11. **Search**: type `net`, the Network page is an answer, OK opens Settings there. **Gear**: OK on the gear opens the wallpaper popover, the cards and the
    dock step back, Left and Right change the wallpaper at once, Back keeps the changes, "All settings" and "TV settings" open the two Settings.
12. Each kind twice before any QML below is deleted. A theme edit made by the QML Settings reaches the Rust client at its next start (`banners.own`).

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
| `home` | `LumaHome.qml`, `Dock.qml`, `Tile.qml`, `StatusBar.qml` (its settings popover included), `StatusItem.qml`, `Widget.qml`, `widgets/*`, `panels/AppsGrid.qml`, `Search.qml`, `LayoutEdit.qml`, `LayoutGrid.js`, `Tint.js`, `Saved.js`, `Glass.qml`, `HomePanels.qml`, `Layers.qml`, `PanelMotion.qml`, `Warmup.qml`, `Resident.qml`, `LumaKeyBridge.qml`, `LumaBus.js`, `kit/*` |
| `home` extras (launch screen, Quick Access, on-screen remote, voice bar, mini frame) | `panels/Splash.qml`, `QuickAccess.qml`, `ScreenRemote.qml`, `VoiceBar.qml`, `MiniFrame.qml`, and in `LumaHome.qml` the parts that drive them (`numberKey`, `accessAsk`, `accessAssigned`, `accessMore`, `noteLaunchOrigin`, `noteSplashOrigin`, `openMini`). `settings/QuickAccess.js` stays: the QML Settings page reads it |
| `home` wallpaper video | `WallVideo.qml`, `WallVideo.js`, the `videoWall*` properties and `gpuFade` in `LumaHome.qml`, `videoWallBackend` and `videoWallHole` in `compositor/shell/Shell.qml`. The QML Settings and `lumatv wallpaper` read `wall.json` and the theme and stay |
| `home` keys | the mic, on-screen remote and Search key handling of `LumaHome.qml` (`remoteKeyPressed`, `Key_webOS_Voice`) once the three codes are taught and the checks pass |

The extras, the wallpaper video, Search's pages and the popover follow `home`: `ctl shell rust -home` gives all of them back to the QML together, so none
of these files can go before `home`'s own two passes. `KitGallery.qml` is development only and can go at any time. `logo/*` stays until the LG BootLogo and MasterBrand windows are replaced.
