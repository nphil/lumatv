# Shell host: a layer over apps, the gear hold, the heartbeat, the number keys (ShellHost)

## Resume here

Built and tested offline; on the TV on 2026-10-25 19:44 ("On the TV" below): the gear hold opens the drawer on Home and over HDMI 2
with a tap still LG's, the layer draws banners and the drawer over an input's video, `banners.own` works as a live switch. Two defects
came out of it, both fixed offline and **not yet re-run on the set** ("After round 7"): the drawer's footer text overflowed its
card, and OK on a banner's button opened nothing (the system window is now asked for through the key filter's own luna client).
Number keys, app keys, the frame cost and the standby heartbeat are not yet run on the set.
Everything is fail-open: with the files missing, a handler absent or an error anywhere, LG's chain gets the key as before.

**Round 8 (2026-10-25 evening, offline only, not run on the TV):** the gear's tap and the Input key show LumaTV's quick panel (left-docked, 492 x 1008 where LG's sat, sliding out of the left edge) and input picker on Home and over apps, in place of LG's Quick Settings and picker (section 6). LG's own stay reachable (the panel's "LG" button, the fail-open, the switches). Needs the compositor restarted (`lumatv apply`: a changed key-filter script is read once, and the QML is in the shadow module). The next TV slot runs steps 3a and 3b of "Set steps" and reports the gate's numbers.

**Rounds 11-14 (2026-10-26, gate FAIL; "Over video, round 11"):** fixed and measured on the set (r14, `2888577`): neither the scrim nor the sheet fades (each opacity change re-batched the scene), the picker no longer restarts its tile's lift at each opening, the inputs' luna subscriptions no longer pile up (every RSS check passes), LG's Dolby toast line is stock noise, and the gate times the picker from the Input key's press and keeps the film's markers. Left: quick p90 over Home 3.43 ms (the frames where the panel joins and leaves Home's scene, 3.2-4.4 ms), single burst-end and first-frame misses where other frames chained in, and over the Dolby Vision film the first frame can wait about 1.5 s for the display's OSD path (round 11 only; not the panel's doing; LG's own Quick Settings misses the bar there too). Next: the picker soak below.

**Round 15 (2026-10-26, offline only, not run on the set):** a closed panel stays in the scene graph, off the screen, for 4 s (`panels/PanelMotion.qml`: `parked`, `held`; Resident, Layers, LumaOverlay keep it visible), so a closing leaves nothing and an opening again joins nothing: those were the frames that held quick over Home at p90 3.43 ms. The gear's press joins the quick panel (the key filter's first entry), so the first opening after 4 s joins nothing either. Home idle is unchanged by construction (the scene is Home's own again once the 4 s are over). The picker's readings wait 310 ms instead of 170 (their answers' frames chained onto the key's burst). Needs `lumatv apply` (the key filter is read once, with the compositor restarted). Next on the set: the re-gate under "Round 15".

**Resume here (round 15, stopped on Main's order, nothing run on the set):** pushed `9be5560` (panels stay in the scene, 4 s), `bfb799e` (picker readings at 310 ms), `3dbf68e` (the gear's press joins the quick panel). Compile gate 132 files, 0 failed; `bun test tests/` 158 pass; `test_panels`, `test_shell_overlay`, `test_shell_host`, `test_shell_surfaces`, `test_volume_osd` pass; `test_quick_panels` passed 25 of 26 on its last full run, and the one failure (the overlay warm-up test's probe, 400 ms after a press, ran before the sheet's 30 ms timer because the software renderer's first frame held it up) was given more time and then passed 5 of 5, 2 of 2 of them under CPU load; the module was not run whole after that edit. `test_a_row_that_opens_a_page_waits_for_Home_with_the_page` fails about half of its runs on `1b39e45` too (the page request is gone when the probe takes it). Next: the TV check under "Round 15" (`lumatv apply` first); only if quick p90 is still over 3.0 ms, read `burst_first_render_ms` of its six windows.

| What | Where | Switch (owner's, `theme.json`, defaults in code) |
|---|---|---|
| A layer of LumaTV's over an app: banners and the notification drawer | `payload/compositor/lumatv/LumaOverlay.qml`, loaded by `payload/compositor/StarfishFullscreenContainer.qml` beside LumaHome | gear hold: `remote.gearHold` (on); own banners: `banners.own` (off) |
| Gear key: a tap is LG's, a hold of 650 ms opens the drawer, LG's Settings long press does not run | `payload/keyfilter/lumatv.js` (the state machine), `LumaBus.js`, `LumaKeyBridge.qml`, `LumaHome.qml` (`gearReady`, `gearHold`) | `remote.gearHold` |
| Number keys 1-9 open dock tiles 1-9, 0 the tenth | `LumaHome.qml` (`numberKey`), `Dock.qml` (`openTile`) | `remote.numberKeys` (on), read live |
| The remote's app keys (Netflix, Prime Video, Disney+, LG Channels) open the owner's choice in any foreground | `lumatv.js` (`appKey`), `LumaBus.js`, `LumaHome.qml` (`appKey`), contract in `settings/AppKeys.js` | `remote.appKeys` (absent = LG's own where installed) |
| Heartbeat written at load, gaps logged | `LumaHome.qml` (`beatOnce`) | none |
| The gear's tap and the Input key: LumaTV's quick panel and input picker, on Home and over apps | `lumatv.js` (`handleLumaQuick`, the second entry before LG's `handleHotkeys`, placed by `tools/keyfilter.py`), `LumaBus.js`, `HomePanels.qml`, `LumaOverlay.qml`, `panels/QuickPanel.qml` (+ `QuickList`, `QuickTile`, `QuickStep`, `PanelMotion`), `panels/InputPicker.qml`, `Resident.qml` | `remote.quickPanel`, `remote.inputPicker` (on) |

Next, on the TV, in order (the one slot): "Set steps" below. Open for Nitin: whether he wants `banners.own` on once LG's own restyled banners are hidden (see "Banners").

## 1. A layer over apps

### What LG offers above an app card (read from the firmware's QML, `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor`)

The compositor's views, bottom to top (`views/StarfishViewsRoot.qml`): fullscreen (cards), HCAP, floating, spinners, boot popup, image splash, **Home** (system UI type `home`), cover UI, **Overlay** (system UI type `overlay`: Quick Settings, the input picker, the PIN popup, the store streamer; and Wayland windows of `_WEBOS_WINDOW_TYPE_OVERLAY`), overlay group, screen remote, popup, keyboard, **notification**, voice agent, nudge, volume, top UI, indicator, long-press spinner, screen saver, screen cover.

Ways for LumaTV's QML to be drawn above a card:

1. **A system-UI entry.** configd's `com.webos.surfacemanager.systemUIManager.appinfo[]` gives `{id, defaultWindowType, main}` and `systemUIManager/launch {appId}` loads `main` as a window (`views/systemUi/StarfishSystemUIManager.qml`; a launch payload's `main` and `windowType` win over the entry, lines 97-99; `services/StarfishExtendedLunaServiceAPI.qml:190-221`). Quick Settings, the logos and the context menu are this. The window root is `StarfishSystemUIWindow` (`SystemUIComponent`): `show()`, `hide()`, `exit()`, `webOSShell.setWindowProperty`, a `needFocus` property that defaults to true.
2. **A view swap.** `sysuicompmgr/changeSystemUIView {viewName, viewSource}`, which the toast, alert, PIN prompt and input popup use (`tools/notification_patch.py`). One item with LG's data contract; not a place for other surfaces.
3. **A Wayland client** of type OVERLAY, TOPUI, POPUP or SYSTEM_UI: the 64-bit Rust UI's route. LG's compositor cannot give any separate window 60 fps (X1, PLAN: 51.2 fps), and Qt 5.12 here is 32-bit.
4. **A layer inside the fullscreen container**, which LumaHome already is (`StarfishFullscreenContainer.qml` loads it). The container draws the app's surface item and LumaHome's item in one scene; an item of ours above the surface draws over the card, and over video, because video is on a plane under the OSD (a card's transparent hole).

LG draws its own banners in the app card's window: `com.webos.app.inputcommon` is `main` of livetv, hdmi1-4, dp and usbc, and draws every banner, popup and no-signal screen (`docs/research/surfaces/InputCommonUI.json`).

### Why a layer in the container and not a system-UI window

LG's compositor ties every system-UI window to its own state, and a window of ours would join it:

- Window lifecycle. `StarfishViewStateController.qml` closes the overlay view on app launches and switches and on power suspend (lines 275, 327, 375, 521, 561, 591, 595, 1012, 1390, 1435); a closed system window is destroyed one second later (`closeTimer`, `StarfishViewContainer.qml`), so a banner would be built on every show unless it were preloaded and re-preloaded after each close.
- Key filters branch on the overlay view: `overlayView.visible` and `.activeFocus` in `appLaunch.js:2252, 3513`, `systemUi.js:402, 905, 933, 1017`, `easytv.js:118, 131`. The gear's own release answers NotAccepted while an overlay has focus. While one of ours were up, Back, Home and the gear would change meaning.
- The window is published in `getForegroundAppInfo` (`StarfishSystemUIManager.qml:57-79`), which LG's Qt apps read through `libStarfishServiceIntegration`; `inputcommon` (the very process whose banners this replaces) would be told a system UI is in front. What it does with that is unread.
- `StarfishMagicNumber.qml:445-463` registers number keys against the overlay's current item.
- A window takes the focus unless it says `needFocus` false.

A layer of ours inside the container is none of that: LG never learns of it, it needs no configd edit and no luna call, and the key filter (which runs first in the chain) is the one path for its keys. Its costs: it is below LG's own views (toasts, alerts, the volume, Quick Settings), which is where they belong; and it touches LG's `StarfishFullscreenContainer.qml` (the existing patch, 23 more lines, a second Loader and a Binding, the same shape as LumaHome's).

**If it does not draw over a card on the TV**, the fallback is way 1: an `appinfo` entry `{"id":"org.nphil.lumatv.overlay","defaultWindowType":"overlay","main":"file:///var/lib/lumatv/systemui/Overlay/Overlay.qml"}` added through `setConfigs` (`tools/systemui.py` has `on` and `off` for existing entries only, so it would need an `add`), a root of `StarfishSystemUIWindow` with `webOSShell.setWindowProperty('needFocus', false)`, loading the same panels from the module directory (same URLs, so the same singletons), and `luna-send luna://com.webos.service.systemUIManager/launch '{"appId":"org.nphil.lumatv.overlay"}'`. [INFERENCE: not run.]

### What is built

`LumaOverlay.qml` is built once, eight seconds after the compositor started, and kept (`Resident.qml`). Nothing draws until something is shown (`visible: up`), it takes no focus, and the container hides it while Home is the surface (Home draws its own banner and drawer). `appFront` comes from the container: an app is its surface and it is on screen.

- **Banners**: `showBanner(spec)` with the specs of `panels/Banner.qml` (`channel`, `input`, `nosignal`). A banner with buttons takes the D-pad (Left, Right, OK, Back); over an app the buttons are Inputs and Picture (`spec.actions`), which open LG's input picker and Quick Settings (`systemUIManager/launch`), system windows that LG draws over anything. Back closes; 45 s without a key closes (`idle`).
- **The drawer**: `gearHold()` opens `panels/Drawer.qml`, a second hold or Back closes it. A banner does not cover it; it replaces a banner.
- **The input pill by itself**: when `Inputs.foreground` (SAM's foreground app, kit/Inputs.qml) becomes `com.webos.app.hdmiN`, `livetv`, `dpN` or `usbcN` and `banners.own` is on, the overlay shows the pill: the port, then the device that named itself over CEC, "eARC" for a receiver. SAM names the app a little before its window maps, so the pill waits up to 6 s for `appFront`.
- **Glass over video**: the panes sample one flat colour of the material's own (`Lu.backdrop` is pointed at it while an app is in front; Home points it back at its blur when it is shown). PLAN: over video, tinted glass without blur. `Glass.qml` draws opaque, so the pane is an opaque dark (or light) pill, as LG's own are.
- **No signal** is drawn in the dark material's inks in both materials (it is black on purpose). Its light render was unreadable (dark ink on black), here and on Home.
- **A door**: a JSON spec written to `/tmp/<shortId>-overlay` is shown (`{"open":"drawer"}` opens the drawer); the same text twice is not shown twice. For the gate and the TV steps.
- **Keys**: the filter gives the overlay a key only while the overlay has a surface that wants keys, an app is in front with the focus (`fullscreenView.activeFocus`: no LG overlay, popup, keyboard or alert is on top) and the layer is answering. A release goes where its press went. Everything else, the volume and channel keys included, is the app's.

### Banners: what feeds them, and what does not exist

- Fed: the input pill (above). `banners.own` is **off**: LG's `inputcommon` draws the same moments and LgRestyles restyles those (`docs/research/lg-restyles-result.md`, `tools/inputcommon.sh`), so a second pill would double. It is the owner's switch for a day LG's are hidden; hiding them is `inputcommon`'s QML, not this layer's. The mock's bottom-left pill with the lead icon and Details/Inputs/Picture is this layer's, LG's restyled pill is LG's layout.
- Not fed, for one reason: **the pipeline id**. LG's `inputcommon` reads `utp.extinputs/getVideoInfo {pipelineId}` (width, height, `scanType`, `ratio`, `HDRType`/`hdrType`, `IsBT2020`, `timingMode`: `Interfaces/InputBannerInterfaces/InputDetailBannerController.qml`), `getSignalState`, `utp.broadcast/getCurrentChannel {broadcastId}` and iepg's `getCurrentNextEventList {channelId}` (`Interfaces/ChannelBannerInterfaces`), with the id it gets from its own pipeline open (`DisplayInterfaces.qml:769-834`). Nothing outside that process knows it yet. So the pill has no resolution or HDR line, the channel banner has no feed, and no-signal is not raised. `facts` and the channel spec take whatever a feed gives; the day something can read the pipeline id (`utp.extinputs` list, or the compositor's `getForegroundWindowInfo`), the feeds are a luna subscription each. Live TV is rarely used (Nitin, 2026-10-23).

## 2. The gear hold

LG's chain for the gear (`Key_webOS_Settings`), fourth after this filter: `handleLongPressKeys` (`preProcess.js:168-219`) swallows the press and starts a 1 s timer with the long-press ring; before 1 s the release **replays** the key (press and release again through the chain), which `handleAvReset` counts (three within 3 s: the AV reset) and `handleHotkeys` turns into Quick Settings; at 1 s it injects `Key_Option`, which launches LG's Settings app (`appLaunch.js:2258-2276`) and, held 4 s more, the hotel-mode-ready key (`appLaunch.js:1380-1390`).

The filter (`lumatv.js`) is first in the chain and only watches the press, so a tap is LG's from end to end and the three-tap reset is unchanged. Held 650 ms (PLAN: a hold is one press held 650 ms), if something of ours has the screen and can show the drawer (`bridge.gearReady`: Home active or the overlay with an app in front, `remote.gearHold` not off, the layer answering), it asks (`gearHold`: Home first, then the overlay). When the drawer opened:

- LG's own timer for the press is cancelled the way the Home hold cancels Screen Zoom's: `longPressTimer.setBlockCancel(false); cancel(); setDeviceId(-1)` (`StarfishKeyFilter.qml:1508-1600`: `cancel()` also closes the ring). LG's state is then as after a cancelled press (`skipKeyPress`, `skipKeyRelease` and `lastKeyPress` untouched).
- The repeats of the hold and its release end at the filter, so LG never replays a tap and never opens Quick Settings after a drawer.
- The hold's timers are LG's `genericTimer` (the filter's own timer service), so the state machine is plain JS and is tested in bun. A watch of 30 s after the last event (each repeat restarts it) clears a hold whose release never came; the next press clears it too.

When nothing can show the drawer, or the drawer does not open, nothing is touched and the hold is LG's (Settings at 1 s). The safety net is `Key_Option`: while the gear is down, LG's 1 s conversion is taken as the hold (the drawer opens, LG's Settings does not), and if the hold had already been taken and the cancel failed, the conversion is swallowed. In that one case LG must see the gear's release to end its own hold, so the filter lets it through, and takes the `Key_Option` release LG answers it with. A `Key_Option` with no gear down is not ours.

What a hold replaces: LG's Settings app by hold, and the hotel-mode-ready combination (hold the gear about 5 s), which needs `Key_Option`. Neither is used here. With `remote.gearHold` off both are LG's again. A tap, tap, hold, tap inside 3 s counts the last tap as the third (LG's own hold resets the count, ours does not); the reset needs three taps.

The saver takes the gear like any key (it wakes the TV and nothing else, press and release), and the AI prompt owns the remote: a hold there is taken and does nothing, so LG's Settings does not open behind either.

## 3. The heartbeat while the panel is off

What is known: after the 16:34 restart from standby, `/tmp/<shortId>-beat` never existed in that boot (`notify-slice-result.md`, "Restart on the set"). The 1 s timer in `LumaHome.qml` has no panel condition, and the base container is a static child of the fullscreen view, so LumaHome is built at compositor start (`StarfishFullscreenView.qml:110`). LG's compositor runs the basic render loop, where QML timers are Qt's unified timer and do not depend on a window being exposed or a frame being presented; the compositor keeps handing out frame callbacks in standby (152.7 fps, `gfx64-result.md`). So the timer should tick; why no file was written is not known. One candidate is that the compositor's thread was held at start while services that only answer with the screen on held their replies (the preload manager did exactly that for 140 s), so LumaHome was not built until the screen came on. Nothing offline can tell the candidates apart.

What changed:

- The first beat is written at load, from `Component.onCompleted`, before any timer has ticked (`beatOnce`), with the line `[LumaHome] loaded, first beat written`. A layer that is built while the panel is off now leaves a file and a line; a layer that is not built leaves neither.
- A beat that was silent for more than 3 s says so when it ticks again: `[LumaHome] the beat was silent for N s`. A timer that stopped with the panel shows as one such line at the first wake.
- A test pins that the beat counts with Home hidden (`hostActive` false), which is what the container says in standby.

What cannot be done from here: a beat written by anything inside the compositor stops when the compositor's thread stops, so it cannot be made independent of whatever blocks that thread. The guard is outside the compositor and is the part that does not depend on the panel (BootPathFix: panel off, no fault; a fresh 30 s at the first screen-on). The TV step below decides which candidate holds.

## 4. The number keys

`remote.numberKeys` (theme.json, default true, written by Settings > Remote & pointer) is read in `LumaHome.key()` on every digit, so the switch is live. 1-9 open dock tiles 1-9 and 0 the tenth (the remote's number row ends in 0). Only on Home with no layer, no moving tile, no AI prompt, no trial; a digit with no tile behind it is LG's. The press acts and its repeats and release are taken with it; the release is the filter's pair, so a digit never leaks into the app it opened. The tile takes Home's focus (`Dock.openTile`), so Back from the library returns to it. The Apps tile (the first) opens LumaTV's library, not LG's store. With the switch off no digit is taken, and LG's quick access (hold a digit) works on Home as before.

## 5. The remote's app keys

`remote.appKeys` (SettingsRest's contract, `settings/AppKeys.js`): `{netflix, amazon, disneyplus, lgchannels}`, each an app id, `""` for nothing, absent for LG's default. The filter watches `Key_webOS_Netflix`, `_Amazon`, `_Disneyplus` and `_LGChannels` in any foreground and asks LumaHome (`appKey(id, frontAppId)`) on a new press, but only with the TV on (`global.powerState` is `active`: with it off the key is LG's, which powers the TV on and opens the app), the fullscreen view holding the focus (no LG overlay, popup, keyboard or alert on top) and the layer answering. LumaHome answers true (taken) for an app that is installed and not in front (it opens, from Home with the splash), an app already in front (nothing: LG would give the key to the app), `""`, an app that is gone, and a key never set whose app is not installed (so no key starts LG's auto-install); false (LG's) for a key never set whose app is installed. The repeats and release of a taken press end at the filter. The saver and the AI prompt take every key first.

## 6. The gear's tap and the Input key: LumaTV's quick panel and input picker (offline, 2026-10-25 evening, not run on the TV)

**What LG does.** The gear is swallowed on its press by `handleLongPressKeys`; its release replays a short press, `handleAvReset` counts it (three in 3 s is the hidden AV menu), and `handleHotkeys` (appLaunch.js 2243-2256) opens `quicksettings` through `systemUIManager/launch` on the release. The Input key (2304-2316) opens `quickinputpicker` the same way on a press. Both are system windows of LG's own state machine, which is why neither is a layer of ours (section 1).

**Mechanism.** A second entry of `payload/keyfilter/lumatv.js`, `handleLumaQuick`, registered by `tools/keyfilter.py` just before LG's `handleHotkeys` (the first entry stays first). LG builds one loader object per file, so both entries share the script's state. By the time a key reaches it, the long-press replay has come round, the AV-reset counter has counted it, a gear hold the first entry took never gets there, and the key blocks and the saver's wake have run. It takes a key only where LG would have opened its window (`access.keyAction.launchApp`, not first-use setup, the Input key not over `partialview`), the TV is on, Home or an app has the screen and the focus (no LG overlay, popup or alert on top), the layer answers and the panel is built; else it returns NextPolicy and LG's window opens as it always did. The gear is acted on at the release, the Input key at the press, as LG's are; repeats and releases of a taken press end there. A second gear tap hides the quick panel; a second Input press moves the picker to the next input. An error anywhere is NextPolicy and one log line. The Input key held 1 s (LG's Home Hub, `Key_Video`) is left to LG.

**Where it shows.** `LumaBus.js` asks Home first (`setPanels`, `HomePanels.qml`: Layers' residents `quick` and `picker`) and then the overlay (`panelReady`/`panel` in the object `LumaOverlay.qml` registers: its own two residents, built 9.5 s and 11 s after the compositor starts, one at a time). Whatever else is up (Settings, the library, a menu, the other panel, the drawer, a banner) gives way; a question waiting for its answer, a listening microphone, the AI prompt and the screensaver own the remote (the key is taken and does nothing). Over an app "All settings" and the rows that open a page take the TV to Home and Home opens Settings there (`Bus.wantSettings`/`takeSettings`, dropped after 15 s).

**LG's Quick Settings stays reachable** three ways: the panel's "LG" button (through the key filter's own luna client, as in section 1), the fail-open above, and the owner's switches `remote.quickPanel` and `remote.inputPicker` (on; off gives LG the key back). The three-tap AV reset is unchanged (taps 1 and 2 show and hide the panel, the third is `handleAvReset`'s).

**The panel.** Left-docked, 492 x 1008, 24 px from the left and top edges: LG's own panel as measured on `caps/luma/qs_res_warm.png` (edges at x 24 and 516, y 24 and 1032). Its rows are the Settings engine's descriptors drawn for a narrow column (`QuickList`, `QuickTile`, `QuickStep`): a stepper's value has a line of its own, words wrap, nothing is cut, and the list shows whole rows only (a window that moves by rows, so a row is never half drawn at any text size). Minimal text: no hint line, the clock beside the title while both fit and under it when not, a second line only for state (an input, "Not answering", the sleep time left). The picker is mock 08 with the same slide from the bottom edge, the "Sound out" chip a speaker mark (the head names the output) and the faded text of an empty input now readable. Motion (`panels/PanelMotion.qml`): one `progress` 0..1, the sheet's `x` (or `y`) and an opacity are the only things that move, no layout; 150 ms in (out-cubic) and 150 ms out (in-cubic), the shorter of Lucent's layer token (220/180) and 150 so the gate's "fully open by 180 ms" holds; reduced motion is a 120 ms fade with no travel. `Resident.qml` keeps a panel drawn while it `leaving`, so Back slides it away instead of cutting it. The glass finds its place in the wallpaper's blur as it moves (`LuPane.locate`). LG's own: 180 ms pause, then 150 ms of a 150 px slide that changes `anchors.leftMargin` (a layout per frame), 250 ms out (`QuickSettings/Controllers/UIController.qml`).

**Speed.** Built once and kept: Home's residents at step 2 (quick) and 1 (picker) of `Layers`, the overlay's at 9.5 s and 11 s; `tests/test_quick_panels.py` counts the items under the panel before and after it opens (equal: nothing is built on open). Text styles and icons are in the start-up warm-up already (every theme role, every shipped icon, the Glass, LuIcon and LuHalo programs), and no new shader exists. Not measured on the TV.

**Gate** (`docs/research/perf/gate.py`, bar = the PLAN's: first frame 50 ms, burst end 180 ms, no frame over 20 ms, p90 render under 3 ms, RSS flat, no `ShaderEffectSource` in the log): `quick`, `picker` over Home; `quick-netflix`, `picker-netflix` over the 4K Dolby Vision test film, with LG's own Quick Settings there as `lgqs-netflix`; `quick-hdmi`, `picker-hdmi` over HDMI input `--hdmi N` (default 3, the game console). The gear's surfaces start on the key's release (LG replays on release), the picker's on the press (the Input key opens on it); none is in the default run. The Input key is evdev 241 (`Key_webOS_InputSource`, round 8); `python3 /tmp/kb.py --learn 15` on the set reads another remote's, and `--input-key CODE` passes it on.

## Tests

From the repo root. The key filter against stand-ins for LG's globals, and against the real `LumaBus.js`:

- `TZ=America/New_York bun test tests/` (87): `keyfilter.test.js` (49; 36 new: the gear's tap, three taps, hold, repeats, release, watch, lost release, new press, broken timer service, `Key_Option` both ways, no `WebOS`, overlay keys, pairing, focus; the app keys: taken, LG's, each key's name, Home first, the saver, the TV off, LG's overlay on top, not answering, lost release, no code in scope, an error), `bus.test.js` (9: Home or overlay asked for the gear, answering window, throwing handlers, the app key).

LumaHome and the overlay through the offline host (each is one run of about 20 s: `python3 -m unittest discover -s tests -p '<file>'`):

- `test_shell_host.py` (6): number keys (tile 2, 9, 0, 1 = library, no digit in the library, while moving, off Home, the release after Home left, the switch off and live), the gear hold from Home, Settings and the library, a second hold, the switch, the beat at load and with Home hidden and the silent-beat line, the app keys through LumaHome (an app, nothing, gone, in front, unset, over an app).
- `test_shell_overlay.py` (7): idle until an app is in front, the flat glass and Home's backdrop back, a pill that takes no key, buttons and the luna launches, Back, no-signal, the volume key is the app's, the drawer from the gear and Back, the switch, the input pill and its wait for the app, `banners.own` off, the door.
- Existing: `test_shell_surfaces.py` (12), `test_panels.py` (13), `test_plugins_surfaces.py` (6), `test_volume_osd.py` (2), `test_settings.py` (44) pass unchanged.
- `test_container_patch.py` (2, no host): the patched `StarfishFullscreenContainer.qml` is LG's file with lines added and none changed or removed (against LG's copy in `/data/home/lgtv/uiinv`, skipped where it is absent), and the QML parser reports no syntax error in it. The container is the one file of LG's whose breaking would leave no fullscreen view, and nothing offline can load it whole.
- Scenes, dark and light, looked at: `python3 tests/qmlhost/scenes.py --only 25-overlay-input-pill,25-overlay-input-banner,20-overlay-channel-banner,18-overlay-no-signal,13-overlay-drawer --out /tmp/host-shots`. A generated picture stands in for video (`tests/qmlhost/scenes_host.py`; the host's `--overlay` and `--bgimage`).
- The panels (section 6): `bun test` has `keyfilter.test.js` (the late entry: tap, second tap, over an app, keyboards, LG's window when nothing can show the panel, the Input key, an error; the pointer's show and hide keys) and `bus.test.js` (Home first, then the overlay, a throwing registry, the request for a Settings page). `python3 -m unittest test_quick_panels` (15 host runs, each 100 s or more on a loaded machine; `HOST_TIMEOUT=600`): the panel rests at 492 x 1008, 24 from the edges, and nothing is built when it opens, the gear again slides it away (drawn while it leaves), reduced motion is a fade with no travel, the picker goes on to the next input and wraps, one panel gives way to the other and to Settings, the owner's switches leave the key to LG, the buttons (All settings, LG's own panel), a Settings page asked for over an app, the door. `python3 -m unittest tests.test_keyfilter_tool tests.test_gate` (the two entries' places and no needless write; the surfaces and the placeholders).
- Scenes, dark and light, looked at: `python3 tests/qmlhost/scenes.py --only 10-quick-panel,08-input-picker --audit --out /tmp/panel-shots` (the layout audit: no finding at 100 % text, none at 200 %; the 200 % pass first found text overlapping where the list was clipped and where the picker's names wrapped, so the list shows whole rows only and the picker's tiles and sheet grow with their words) and, over a picture standing in for video, `10-overlay-quick`, `10-overlay-quick-lg` and `08-overlay-picker` (rendered and looked at; runner scenes have no audit, and they draw the same files).

The offline host is Qt 5.15; a `Binding` that switches off warns there and not on the set's 5.12, so the backdrop is handed over by assignment (Home on `hostActive`, the overlay on `appFront`).

## Set steps (not verified on the TV)

One slot, Home up, the receiver muted, Picture Mode `filmMaker` as found. Deploy `tools/`, `payload/` and restart the compositor as `lumatv apply` does (the container and the new files are in the shadow module).

1. The compositor's QML logs to `/var/log/messages` (`journalctl` hangs on the set): `grep -E '\[Luma(Keys|Home|Overlay)\]|LumaOverlay failed' /var/log/messages | tail` shows `[LumaKeys] bridge ready`, `[LumaHome] loaded, first beat written` and `[LumaOverlay] loaded`; no `LumaOverlay failed to load` from the container and no `[LumaKeys] ... failed`.
2. Number keys on Home: 2 opens the second tile, 1 the library, 0 the tenth; Back returns with the focus on it; Settings > Remote & pointer off: the digits do nothing. Hold a digit with the switch off: LG's quick access as before.
   App keys: Settings > Remote & pointer > App keys, set Netflix to another installed app; the Netflix button opens that app from Home and from over an app; set Prime Video to nothing: nothing; an unset key opens LG's app where it is installed. With the TV in standby the button still powers it on and opens LG's app.
3. Gear: a tap opens Quick Settings (LG's, redirected); three taps inside 3 s still start the AV reset (do not confirm it); a hold opens the drawer on Home and the long-press ring closes at 650 ms; `/var/log/messages` has `[LumaKeys] gear held, the notification drawer is open`; release, and Quick Settings does not open; LG's Settings app does not open. A second hold closes the drawer. `theme.json` `remote.gearHold` false: a hold is LG's Settings again.
3a. **The gear's tap and the Input key (section 6)**, after the apply: `luna-send -n 1 luna://com.webos.service.config/getConfigs '{"configNames":["com.webos.surfacemanager.keyFilters"]}'` lists `handleLumaHome` first and `handleLumaQuick` just before `handleHotkeys`; `/var/log/messages` has `[Resident] quick built`, `[Resident] picker built`, `[Resident] overlay quick panel built` and `overlay input picker built` (the last two about 10 s after the compositor starts; before that the keys are LG's). Home: a gear tap slides the panel in from the left edge (492 x 1008, 24 px from the edges: the same place and size as LG's); the gear again, or Back, slides it away; Left and Right on Picture mode change it (put it back to Filmmaker), the volume row moves the receiver, Down past AI access is All settings and then LG's own Quick Settings (Right, OK). Over HDMI 2 and the Netflix film: the same, the picture and sound go on, the volume keys still work with the panel up. Input key: the picker rises from the bottom; the key again moves to the next input, OK opens it, Back closes. Three gear taps inside 3 s: the AV reset menu as before (do not confirm it). A gear hold: the drawer (and it replaces the panel). `remote.quickPanel` false: the gear's tap is LG's Quick Settings again.
3b. **The gate over Home and over playback** (the numbers the slice is judged by; mute the receiver first and restore it after): `python3 /tmp/kb.py --learn 15` on the set, press the remote's Input key, read its code N; then from the repo `python3 docs/research/perf/gate.py --surfaces quick,picker,quick-hdmi,picker-hdmi,quick-netflix,picker-netflix --input-key N --hdmi 3 --json /tmp/panels-gate.json`. Pass: first frame 50 ms or less, burst end 180 ms or less, no interval over 20 ms in the burst, p90 animated-frame render under 3 ms, RSS flat over the cycles, no `ShaderEffectSource` line in `/var/log/messages`. Report the numbers per surface (`--json`), not the verdict alone; a miss in `-netflix` or `-hdmi` only is the video plane's, one in all of them is the panel's.
4. Gear over an app (Netflix, YouTube, an HDMI input, Live TV): the drawer draws over the picture, over video included (the overlay's own claim, the one to look at), the D-pad moves it and Back closes it, the app's picture and sound go on; the volume keys still reach the app. Home closes it.
5. Banners (`banners.own` true): switch to HDMI 2 from Home: the pill at the bottom left for 4 s. LG's own banner draws as well (expected: the switch is for when LG's are hidden). `echo '{"kind":"input","icon":"cable","name":"HDMI 2","facts":"","buttons":true,"actions":["inputs","picture"],"holdMs":60000}' > /tmp/<shortId>-overlay` over an input shows a banner with buttons: Right, OK opens LG's Quick Settings, Back leaves; `{"open":"drawer"}` opens the drawer.
6. The frame cost of the overlay over video: `python3 docs/research/perf/gate.py` has no banner surface; `lumatv fps on`, then the compositor's `[FPSLOG]` and `[FPSJANK]` lines in `/var/log/messages` while the pill and the drawer are up over Live TV or HDMI.
7. The heartbeat: with the set in Active Standby, `lumatv apply` or a restart; then `ls -l /tmp/<shortId>-beat` (is there a file and how old), `grep -E 'LumaHome\]|LumaOverlay\]|LumaKeys\]' /var/log/messages*` (is there a `loaded` line with the panel off), and after the wake `the beat was silent for` (timers stopped) or not. The result says which candidate of section 3 holds.

## On the TV (2026-10-25 19:44-19:46)

Set on Home, LG's compositor, receiver muted through Home Assistant, `filmMaker`; the ipk of `4ee268a` installed and applied at 19:28
(the container, the key filter and `LumaOverlay.qml` in the shadow module). Keys were injected on the SSAP key device (`Smart Remote
RCU Input`, `/tmp/kb.py`); the gear is evdev `KEY_MENU` (139): a tap opened LG's Quick Settings on Home (`slot7/gear-tap-home.png`).
Files in `docs/research/round7-2026-10-25/slot7/` (`log.txt`, captures at 960x540). Steps 2 (number and app keys), 6 (frame cost)
and 7 (the heartbeat in standby) were not run in this round.

- Gear hold on Home (held 1 s): the drawer opened at the top right ("Notifications / Nothing new", `gear-hold-home.png`) and
  `/var/log/messages` has `gearFire, [LumaKeys] gear held, the notification drawer is open`; LG's Settings app did not open and
  Quick Settings did not open on the release; Back closed the drawer. The drawer's footer text ("What your assistants, plugins and
  this TV tell you shows here.") is drawn below the card's bottom edge, on Home and over HDMI 2 alike: a layout defect of the drawer,
  not of the overlay.
- `banners.own` on (`lumatv theme set '{"banners":{"own":true}}'`, read live within 2 s): the switch to HDMI 2 from Home showed
  LumaTV's pill at the bottom left ("HDMI 2 · AVR-S760H / eARC", `own-pill.png`) beside LG's own name line, as expected while LG's
  banners are not hidden.
- The door over HDMI 2: the input spec with `buttons` drew the banner with "HDMI 2 | Inputs | Picture" over the picture (the Apple TV
  behind the receiver; `door-banner.png`), Right moved the focus to Inputs (`door-banner-right.png`), OK closed the banner and the
  foreground stayed `com.webos.app.hdmi2`; LG's Quick Settings was not in the capture 1.8 s later (`door-ok-quicksettings.png`),
  so what OK on a button opens over an input is still to be read with the physical remote. `{"open":"drawer"}` opened the drawer
  over the picture (`door-drawer.png`), Down moved in it, Back closed it.
- Gear held over HDMI 2: the drawer over the video with the picture dimmed under it (`gear-hold-hdmi.png`), the same
  `[LumaKeys] gear held` line, foreground still `com.webos.app.hdmi2`; Back closed it (`gear-hold-hdmi-closed.png`). The DISPLAY
  capture includes this input's picture (the Apple TV; Netflix's stayed black in the gates).
- End: Home, `banners.own` false again (`theme get` shows `"banners": {"own": false}`), no `LumaOverlay failed` or `[LumaKeys] ...
  failed` line, 0 `[FPSJANK]` lines (`fps` off).

The one defect found is the drawer's overflowing footer. The question for Nitin stands: `banners.own` stays off until LG's own
banners are hidden.

## After round 7 (offline, 2026-10-25, not run on the set)

- **The drawer's footer.** With nothing to list the sheet stayed at its 240 px minimum while the empty state (the bell, "Nothing new" and a
  caption that wraps to two lines) needs 318: the caption ran out of the card, over the picture (`slot7/door-drawer.png`, `gear-hold-hdmi.png`).
  `panels/Drawer.qml` now sizes the sheet to what it holds, the list or the empty column's own height, under the same floor and the screen
  as ceiling. Reproduced in the host first (`13-notifications-empty`), then rendered dark and light after (the caption sits inside, 16 px
  from the bottom). `test_the_drawer_with_nothing_in_it_is_as_tall_as_its_words` reads the geometry (the words must end inside the
  card, and must be taller than the old 240, so the test fails if the sheet goes back to a fixed height).
- **OK on a banner's button.** The overlay asked for Quick Settings and the input picker with `LS.adhoc.call("luna://com.webos.service.systemUIManager",
  "/launch", ...)`. The capture shows the key reached the banner (Right moved the focus, OK closed it) and no window opened. The
  compositor hosts `systemUIManager` itself, and every call LG's own QML makes through `LS.adhoc` is to a service another process hosts
  (applicationManager, notification, videooutput, avmutemanager: `grep -rn "LS.adhoc.call"` over the firmware's QML); LG's key filters,
  which do open Quick Settings (the gear tap in the same slot), use a luna client of their own, `lunaCall` (appId
  `com.webos.starfish.keyfilters`). Whether the hub refused the `LS.adhoc` client or the call went wrong some other way is not readable
  offline; the next step is to take the path that is known to work. The filter now gives the bridge a `launchHook` that calls
  `lunaCall.send("com.webos.service.systemUIManager", "/launch", {appId, params?})`, the bridge registers it on the bus
  (`Bus.systemLaunch`), and the overlay's Inputs and Picture buttons use it; `LS.adhoc` is used only where the bus has no launcher (the
  offline host). Never both: a second launch of an open Quick Settings closes it. The log says which way each button took:
  `[LumaOverlay] com.webos.app.quicksettings asked for through the key filter's client` and `[LumaKeys] system window asked for: ...`.
  Tests: `keyfilter.test.js` (the hook sends LG's call), `bus.test.js` (the launcher, none, false, throwing), and
  `test_shell_overlay.py` (with a launcher registered the buttons use it and make no `LS.adhoc` call).
- **Next on the set** (round 8, one step): the door banner again (`/tmp/<shortId>-overlay` with `actions: ["inputs","picture"]`) over HDMI 2, Right, OK:
  Quick Settings opens over the picture, Back closes it, and `/var/log/messages` shows both lines above. With the physical remote as
  well as the injected key. If it still opens nothing, the next reading is `luna-send -n 1 luna://com.webos.service.systemUIManager/launch
  '{"appId":"com.webos.app.quicksettings"}'` from a shell during the same state, then the log line of LG's `LAUNCH_SYSTEMUI`.
- A test of mine was flaky under load and is fixed: with `banners.own` on, the host's make-believe TV answers `Inputs` when Home's build
  first reaches a page that uses it, which can be mid-test and puts a pill over the banner a step is about to key. The overlay tests run
  with the banners of their own off, and the one that needs them primes `Inputs` first.

## Over video, round 11 (the quick and picker gate, 2026-10-26 04:15-04:23, `round9/r11-2026-10-26/quick-picker-gate.json`)

The gate failed on four counts: quick p90 render 3.70 ms over Home, picker RSS +1.67 MB over 8 cycles over Home, the quick panel's first
frame over the film 1479 ms (no burst at the key, one interval over 20 ms), and two of six picker markers lost over the film. What each was,
read from the `full` captures (`/tmp/perf/gate-r11-qp-*`):

- **The first frame over the film is the display's, not the panel's.** The OSD interrupt (`osd_irq`, and `DE2VSYNC` with it) was silent
  from the capture's start, 1.1 s before the gear tap, until 1171 ms after it. The compositor drew the panel's slide all the same: GPU jobs
  every 16.7 ms from 7 to 397 ms after the tap, 25 frames, none of them presented (no page-flip event); the first flip landed at 1479 ms.
  Over Home and HDMI 3 the interrupt ran at 120 Hz through every capture. The picker over the film found it on: in that capture the
  compositor had drawn something 1583 ms before the key and the interrupt came back 1154 ms after that, before the key. Once on it stayed
  on (the next opens took 10-27 ms, and the soak 11 s later 6-40 ms). So Dolby Vision playback with nothing drawn over it keeps the OSD
  path off, and the first frame drawn over it waits about 1.16 s for it, then about 0.3 s for the first flip. The key reached the panel at
  once and LG's handler was not first. Whether LG's own Quick Settings waits the same is the new baseline surface `lgqs-netflix` (the
  same film, `remote.quickPanel` off for the run). The gate reports it now: `osd_wake_ms` per window (`analyse.py`: the interrupt silent
  for the 100 ms before the marker, then the time to its return) and the first-frame check names it. Nothing in the overlay can turn the
  path on ahead of the key short of drawing over the picture all the time, which would change what Dolby Vision does with the picture
  [INFERENCE] and is the owner's call; not done.
- **Services.qml:513** (`SystemUI/Notification/Services.qml`, LG's toast model reading `userData.id` of a toast that has no `userData`)
  fired at each `-netflix` setup's OK, when playback starts, 25 s before the first measured window. Which toast it was is not known: the
  set's log had rotated before it was read. The re-gate tails the toast lines. The Netflix `[CrashReporter] Previous CrashReport ...
  REASON_USER_KILL` line in the same run is the setup's own `closeByAppId` and is stock noise for the gate now.
- **Two picker markers lost over the film**: the ring overflowed (832993 events written, 660544 kept; CPU 0 lost its oldest 172k, with
  the first cycle's markers). Over the film the trace takes about 79k events/s against the 44k the 11 s budget was set for; the
  `-netflix` surfaces now take one cycle per full capture (`full_budget_s`).
- **The picker was timed from the wrong edge.** The Input key opens the picker on its press (LG's and ours, `lumatv.js` `panelKey`); the
  gate pressed it 50 ms before its marker and timed from the release, so round 11's picker windows hold the slide's tail (first frame
  9-16 ms, burst end 76-99 ms after the release, no animated frame in most over HDMI). The picker surfaces are timed from the press now;
  round 11's picker numbers do not count.
- **Quick p90 over Home.** The scrim, a full-screen rectangle, faded with the slide. In the offline host with Qt's renderer statistics
  (`QSG_RENDERER_DEBUG=render`, LumaHome, four opens and closes) every frame of the slide after the third rebuilt either the batches or
  the whole render list of the scene (153 nodes in 98 batches): a change of a full-screen item's opacity invalidates every batch it
  overlaps, which is all of Home's. With the scrim whole from the first frame the same frames rebuilt nothing; taking away the list's clip,
  the glass's per-frame `locate()` or the sheet's own fade did not change the count. Over an input little is under the scrim, which is
  why the same panel measured 2.31 ms there. Both panels now dim the page at once and slide the sheet alone; the first three frames still
  rebuild (the panel's nodes appear, the focused row's halo, the sheet's fade passing 1). Test:
  `test_quick_panels.HomePanels.test_nothing_that_covers_the_screen_fades_while_a_panel_slides`.
- **Picker RSS over Home**: 45 pages a cycle, a steady stair in the 1 s samples, flat over HDMI 3 (0.25 MB over 8 cycles) and the film
  (0.08 MB), where the overlay's picker opens the same file. Not reproduced offline: 20 picker cycles over Home kept the item count
  (19571) and the host's RSS flat, as did 20 quick-panel cycles. The quick panel over Home makes the same readings (`Inputs.refresh`, a
  settings read) and stayed flat on the set. Left for the set: the door (`/tmp/lumatv-layer` = `picker`, then empty) opens the same
  picker without the Input key; if 12 door cycles grow the compositor's RSS as the key's did, the cause is in the picker's QML or its
  readings, if not it is the key's path through LG's chain.
- **Input subscriptions left open, fixed**: `Inputs.refresh()` (each panel opening, each Settings page that shows inputs, the 30 s watch) asked
  `eim/getAllInputStatus` and `getForegroundAppInfo` with `subscribe: true` again each time. LG's `Service.callService` returns the
  call's token and keeps the call open until it is cancelled (`libwebosserviceplugin.so`: `callService` returns what its `call` made;
  `cancel(LSMessageToken)` is on the type), so every ask left one more open subscription per service, each answering every input change
  and app switch again. The offline host counted 8 left open on the input status after start-up and three openings. Each ask now cancels
  the one before (`kit/Inputs.qml`); test: `test_quick_panels.HomePanels.test_each_reading_of_the_inputs_lets_the_last_subscription_go`.
  It is the same for the quick panel, so it does not explain the picker's stair alone.

### Round 12 (TvRound11, on `4ab9243`: the scrim fix and the gate's changes; `round9/r11-2026-10-26/quick-picker-gate-r12.json`, `overlay-toast-rss.txt`)

VERDICT FAIL. First frames 33-42 ms and burst ends 142-174 ms everywhere but one picker opening over the film (a 1.5 s burst, frames
chained by something else drawing). The OSD was on at every key this time (`osd_wake_ms` none): the round 11 wait did not recur.

| Surface | p90 render | Other misses |
|---|---|---|
| quick (Home) | 4.23 ms | one interval over 20 ms |
| picker (Home, timed from the press now) | 3.13 ms | RSS +1.85 MB |
| quick-hdmi | pass | |
| picker-hdmi | pass | RSS +1.82 MB |
| quick-netflix | 3.10 ms | the toast line below |
| picker-netflix | 3.11 ms (one window 4.27) | burst end 1541 ms, one interval over 20 ms |
| lgqs-netflix (LG's own Quick Settings) | 2.69 ms (one window 4.77) | first frame 81 ms, no burst at the key, 4 log lines of LG's panel |

- **The slow frames left** over Home were the first two of an opening (3.1-4.5 ms) and the first and the fourth and fifth of a closing
  (3.7-5.5 ms), about 2 ms for the rest. The second of an opening and the fourth and fifth of a closing are where the sheet's own fade
  passed 1 (`min(1, progress * 2.5)`: 17 ms into the OutCubic opening, 92 ms into the InCubic closing), a whole render-list rebuild
  there and a re-batch of the sheet on every frame below 1. The sheet now slides without a fade (both panels; reduced motion is the fade
  alone). Test: `test_nothing_fades_while_a_panel_slides` (the scrim and the sheet at opacity 1 whenever the panel is drawn).
- **The picker restarted its own lift**: `pickFront` set `index = 0` and then the front tile's, so every opening, and again the readings'
  answer 170 ms later, took the focus off the front tile and gave it back; its lift and halo ran again, 180 ms of frames after the slide,
  each re-batching the tiles. It assigns once now. Test: `test_the_picker_opening_again_does_not_start_its_tiles_lift_over`.
- **The toast**: at each film's OK `com.webos.service.audio` shows its "Dolby Vision / Dolby Atmos via Audio Device" toast, and LG's
  toast model throws reading its `userData.id` 20-30 ms later (all three setups). LG's own; the gate counts it as stock noise.
- **The picker's RSS over the door** (Home, 12 cycles, no key): +624 KB, in steps of about 236 KB every third cycle, a quarter of the
  key-driven rate (+1.85 MB over 8). So part of it is the picker's own path (its readings: the open subscriptions above are fixed in
  `2886b62`, which this round did not run) and the larger part comes with the key.

### Rounds 13 and 14 (TvRound11; `quick-picker-gate-r13.json` on `1aaf632`, `quick-picker-gate-r14.json` and `picker-rss-r14.txt` on `2888577`)

- **RSS**: with the inputs' subscriptions cancelled (r13 on) the picker's RSS passes over Home and HDMI 3; in r14 every RSS check passes
  and 12 door cycles stayed flat for 9 of them (+352 KB, one step). Round 13 had +1.97 MB on quick-hdmi and +1.10 MB on quick-netflix
  that r14 did not repeat: steps of about 236 KB land in whichever surface's 8 cycles they fall in.
- **The fade frames are gone** (r14, sheet without a fade): the second frame of an opening and the fourth and fifth of a closing in r13
  (3.25-5.55 ms, the picker's worst) are 1.7-2.6 ms. Picker p90 passes (3.50 in r13).
- **What keeps quick over Home at p90 3.43 ms** (36 frames; the bar is 3): the frame where the panel's nodes join the scene (an
  opening's first, 3.2-3.4 ms) and the one where they leave it (a closing's last before Home alone, 4.0-4.4 ms). The offline renderer's
  statistics show a rebuild of the whole render list at exactly these two frames, and Home's list is long; over HDMI 3 the same two
  frames take 1.6-1.9 ms. A node cannot join or leave without that rebuild, and keeping the panel's nodes in the scene while it is
  hidden would make every frame of Home pay for them [INFERENCE]. What would help: fewer nodes and batches in Home's scene under a
  layer. Not done.
- **The other r14 misses**: quick-hdmi first frame 50.51 ms in one window with no burst after it (a single late first frame); picker-hdmi
  and picker-netflix burst ends 206 and 301 ms and one interval over 20 ms on each film surface, with frames that the panel's 110 ms
  slide does not account for, so something else drew in the same windows [INFERENCE: the input's or Netflix's own frames]. LG's own
  Quick Settings over the film (r12, r13): first frame 81 and 88 ms, no burst at the key, its own log lines.

Next on the set: `gate.py --surfaces picker --input-key 241 --soak` (500 cycles, about 26 min) tells a heap that grows to its collection
threshold and stays (steps that stop) from a leak (steps that go on); the 236 KB steps look like the former [INFERENCE].

### Round 15 (offline, 2026-10-26: a closed panel stays in the scene, off the screen; not run on the set)

**The slow frames.** Quick's p90 over Home (3.43 ms) is the fifth largest of 36 frames, and the frames above it and just under it are where the panel's nodes left Home's scene (3.6-4.4 ms, four frames of the three closings) and joined it (3.2-3.4 ms, one in each of the three openings); the rest are about 2 ms. Qt's batch renderer says which frames rebuilt what (`tests/qmlhost/renderstats.py`: `QSG_RENDERER_DEBUG=render`, the same in the host's Qt 5.15 and in 5.12.10, the set's, run with `PYTHONPATH` at a PyQt5 5.12 install). Every opening is `FF` (the nodes join Home's scene, then the sheet's transform node becomes a batch root at its first move) and every closing `F` (the nodes leave), each a rebuild of the render list and the batches of the whole scene. Home alone is 107 nodes in 69 batches; the quick panel adds 45 nodes in 29-32 batches, the picker 31 in 23.

**Why the nodes do not stay for good.** To the renderer `visible: false` and `opacity: 0` are the same: a blocked subtree is taken out of its lists and put back. Only nodes that are drawn stay in them, and every batch of the scene is drawn in every frame, on the screen or off it (no culling), so a panel kept for good would add its 23-32 batches to every frame of Home, the idle cost to avoid (Home idle p90 0.16 ms).

**What is built.** After its slide a panel is parked: the scrim and the sheet move off the screen (a transform; they stay nodes of the scene) and `Resident` keeps it visible for `lingerMs`, 4 s from the close (`PanelMotion`; the gate opens a panel again 1.5 s after it closed it). An opening in that time joins nothing and the closing leaves nothing. When another surface opens (`Layers.releaseParked`, and the overlay's: also when the app goes) the parked panels go in that frame, whose rebuild the new surface needs anyway. After the 4 s the scene is Home's again, 107 nodes in 69 batches: one full rebuild, in an idle frame. `up` (presented or leaving) replaces `visible` for the clock, the receiver gate and the settings answers. Reduced motion is unchanged: its fade rebuilds the scene on most frames, parked or not.

**The gear's press** joins the quick panel to the scene before its tap opens it: `warmQuick` in the key filter's first entry (`lumatv.js`; it sees the press, LG replays the tap on the release, so the press leads the opening by the length of the tap, 50 ms in the gate), `Bus.warmPanel`, `HomePanels.warm` and the overlay's `warmPanel`. The panel joins parked and moves its sheet once 30 ms later (`PanelMotion.warm`), which is where the sheet's transform node becomes a batch root; the opening then starts in a scene that has both, and its frames read `p.......pF` against the `FF` and `F` of the cold join (the `p` is the focus ring joining the sheet; the later `pF` are the readings' answers). A hold becomes the drawer, which lets the panel go (`releaseParked`); nothing is warmed while something is up, while the screensaver or the AI prompt has the keys, for the picker (the Input key acts on its press) or when the panel could not open. LG's chain still gets the press.

**The picker's readings** are asked for 310 ms after the opening (170 ms before). The gate counts a frame within 50 ms of the burst's last as part of it, the slide's last frame is up to 177 ms after the key (a first frame at 50 ms, the 110 ms slide, one frame), and the answers to `Inputs.refresh()` rebind every tile: the answers' frames are the likeliest cause of r14's two burst ends over 180 ms (picker-hdmi 206 ms, picker-netflix 301 ms, both opening windows) and of an interval over 20 ms [INFERENCE: the traces were not read frame by frame]. The quick panel keeps 170 ms: its first tile takes the focus when the settings answer comes, and its burst ends were 139-172 ms.

Frames of three open/close cycles over Home, Qt 5.15 (`python3 tests/qmlhost/renderstats.py quick|picker --cycles 3 --hold 900 --gap 700`; `.` nothing rebuilt, `b` the one batch of the scrim, `F` the whole scene; 5.12.10 reads the same, a frame or two apart):

| Window | Quick before | Quick after | Picker before | Picker after |
|---|---|---|---|---|
| open 1 | `FF......pFbbbbbbbbbb` | `F.......pFbbbbbbbbbb` | `FFbbbbbbbbbpb` | `FFbbbbbbbbbp` |
| close 1 | `......F..b..` | `b....b.....b` | `......F...b..` | `b......b.....` |
| open 2 | `FF...........b.` | `b..............` | `FF...........b.` | `b..............` |
| close 2 | `......F.....` | `b....bb......` | `......F.....` | `b.....bb.....` |
| open 3 | `FF.....b.......` | `b........b.....` | `FF......b.......` | `b.........b....` |

Only the first opening joins: the panel had left the scene before it. The tests are in `tests/test_quick_panels.py`: the parked state and its end (Home, overlay), the release by the surface that opens, and the renderer's own figures above as assertions (`test_the_renderer_builds_nothing_of_the_scene_again_for_a_panel_that_is_parked`).

**On the set** (one slot, Home in front, the receiver muted as for round 14): `gate.py --surfaces quick,picker,quick-hdmi,picker-hdmi,quick-netflix,picker-netflix --input-key 241 --hdmi 3 --tag r15-qp`, then `gate.py --surfaces home-idle` at least 10 s after the last panel was closed (the nodes leave 4 s after a closing). Pass:

- quick over Home: p90 render under 3.0 ms (r14: 3.43; the p90 allows four of the 36 frames over it). r14's frames over 3.0 ms, by window: the join in each opening (3.43, 3.21, 3.38), the sheet becoming a batch root in the third (3.14), the leave in each closing (4.16, 3.97, 4.37), and three of no known cause (3.40 the third frame of the first opening, 3.01 the second and 3.55 the first frame of a closing). The first seven are what this round removes; `burst_first_render_ms` of the six windows in `gate-r15-qp-quick-m0.json` says what is left.
- picker over Home: p90 under 3.0 ms (r14: 2.51), RSS passes.
- `-hdmi` and `-netflix`: first frame within 50 ms, burst end within 180 ms and no interval over 20 ms in every window (r14: 50.51 ms; burst ends 206 and 301 ms; one interval on each film surface), RSS passes.
- `home-idle`: p90 render 0.16 ms as in round 8; the panels never stay in the scene once their 4 s are over.

If p90 stays over the bar the frames left are not the panel's structure (a longer `lingerMs` would only keep the nodes in every frame of Home for longer): the windows' `burst_first_render_ms` say which frame of the burst they are.

**On the set, 2026-10-26 13:52** (ipk of `87e0fa5`, LG's compositor, receiver muted; log `/tmp/r8/qp-r15.log`): VERDICT FAIL.
Over Home, quick p90 3.12 ms (r14 3.43) and burst end 180.76 ms (bar 180); picker p90 2.97 ms with one burst interval over
20 ms. Over HDMI 3 both panels pass every bar (quick p90 2.58, burst end 138 ms; picker burst end 142 ms). Over Netflix the
quick panel's burst ends at 370 ms with two intervals over 20 ms, the picker's p90 is 3.55 ms. Memory passes on all six.
The HDMI surfaces leave `com.webos.app.hdmi3` running behind Home with audio port 1 open (`lumatv inputs status`); it was
closed through SAM before the compositor gate. `home-idle` not run.

### Round 16 (2026-10-26)

**Resume here (stopped on Nitin's order 17:41; no build installed, the set was left on Home then Active Standby, receiver unmuted as found, ports free, LG's compositor, stub on, fps off):** pushed `bac83ba` (HDMI teardown closes the input through SAM; `inputs_free` check) and the commit below (gate JSON keeps per-frame evidence; `run.Set` reads the HA token only for `key()`; repeated input answers no longer rebind the picker's tiles; a Settings request from a panel over an app survives a Home return that is cancelled). Compile gate 133 files, 0 failed; `test_gate` 65 pass; the four touched `test_quick_panels` tests pass; the module was not run whole after the last edit. Deployed build is still `87e0fa5`.

Baseline on the deployed build (`--tag r16-base`, `/tmp/r8/r16-baseline.log`, per-window traces `/tmp/perf/gate-r16-base-*`): quick p90 3.21 ms (fail), picker p90 3.02 ms (fail) and RSS +1.71 MB (fail, r15 passed), quick-hdmi and picker-hdmi pass (burst ends 147/140 ms, `inputs_free` pass), quick-netflix passes (burst end 166 ms, p90 2.83), picker-netflix burst end 944 ms (fail). home-idle p90 4.13 ms on 5 animated frames (the 10 Hz wallpaper tick frames classed as animated; no panel involved; round 8 had 1 such frame).

Causes found: picker-netflix's 944 ms is Netflix's own drawing (tgid `netflix` submitting GPU jobs at 60 Hz from 0.1 to 2.5 s after the open, across the close; r15's `picker-netflix-m1` shows the same film moment), so the burst chains through frames that are not the panel's; r15's quick-netflix 370 ms had only compositor jobs (the readings' answers at 170 ms chaining onto the slide). Over Home the slow frames are the first frames of each opening and closing (3.2-4.5 ms against about 2.1 ms). The panel adds 29 batches to every frame (152 nodes/101 batches against Home's 107/69 offline).

Next, in order: (1) `analyse.burst`: frames whose only new content is a client's buffer (a non-compositor GPU job since the last flip and not animation-driven) keep the chain but do not extend the burst end; (2) quick readings at `enterMs + 200` like the picker's; (3) collapse the panel's content to fewer batches. A `layer.enabled` on the sheet's content cut it to 110/72 offline but re-rendered the layer in every slide frame (`b` each frame), because the focus lift and halo animate inside it while it slides; it needs the lift and halo settled instantly while `motion.moving` first, and the parked node-count assertion in `test_quick_panels` changed. Then deploy and re-gate all six plus home-idle (at least 60 s after any compositor restart).

## Under LumaTV's own compositor

The key filter, the overlay and the container patch run only inside LG's compositor. At the cutover LG's key filters go away (`compositor-c2-result.md`, "Surfaces under LumaTV's compositor") and the QML shell with them; the rules here then belong to the compositor's evdev key routing and the Rust UI: a gear that is LG's on a tap and ours at 650 ms (the three-tap AV reset stays three taps), the app keys with their owner's choice and LG's own where installed, the number keys on Home, and a layer over an app that takes keys only while it has a surface that wants them. `tests/keyfilter.test.js` and `tests/bus.test.js` are written as that rule set.

## Not done

- A feed for the channel banner, the pill's resolution and HDR line and the no-signal screen (the pipeline id, section 1). LG's own banners draw those moments.
- The voice bar and the on-screen remote over an app (they are Home surfaces and the voice bar fills Search only): the overlay has the way to host them, not the wiring.
- The fallback system-window route (way 1) is described, not built.
- Whether `global.powerState` reads `active` (the string `StarfishViewStateController.qml:1056` compares) while the TV is on and the key filter runs: if it reads anything else the app keys stay LG's, and step 2 of the TV list shows it. On this set `netflix`, `amazon` and `com.webos.app.lgchannels` are installed and Disney+ is not (`/data/home/lgtv/uiinv/listApps.json`), so an unset Disney+ key does nothing.
- The panels share that dependency: `lgWouldOpen` and `panelWanted` in `lumatv.js` need `global.powerState === "active"` (LG's own string, `StarfishViewStateController.qml:1056`, `StarfishPowerService.qml:30`) and read `access.keyAction.launchApp`, `bootManager.bootFirstUse`, `systemPropertyService.powerOnlyMode` and `global.isForegroundApp` the way `appLaunch.js` does. If the gear tap still opens LG's Quick Settings once the panels are built, one of these reads fails or differs: a throw leaves one `[LumaKeys] the quick panel key failed` line per 5 s in `/var/log/messages`; a plain false logs nothing, and the next step is a `console.info` of each read inside `panelWanted`.
