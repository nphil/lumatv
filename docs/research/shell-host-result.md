# Shell host: a layer over apps, the gear hold, the heartbeat, the number keys (ShellHost)

## Resume here

Built and tested offline, **not verified on the TV**. Everything is fail-open: with the files missing, a handler absent or an error anywhere, LG's chain gets the key as before.

| What | Where | Switch (owner's, `theme.json`, defaults in code) |
|---|---|---|
| A layer of LumaTV's over an app: banners and the notification drawer | `payload/compositor/lumatv/LumaOverlay.qml`, loaded by `payload/compositor/StarfishFullscreenContainer.qml` beside LumaHome | gear hold: `remote.gearHold` (on); own banners: `banners.own` (off) |
| Gear key: a tap is LG's, a hold of 650 ms opens the drawer, LG's Settings long press does not run | `payload/keyfilter/lumatv.js` (the state machine), `LumaBus.js`, `LumaKeyBridge.qml`, `LumaHome.qml` (`gearReady`, `gearHold`) | `remote.gearHold` |
| Number keys 1-9 open dock tiles 1-9, 0 the tenth | `LumaHome.qml` (`numberKey`), `Dock.qml` (`openTile`) | `remote.numberKeys` (on), read live |
| The remote's app keys (Netflix, Prime Video, Disney+, LG Channels) open the owner's choice in any foreground | `lumatv.js` (`appKey`), `LumaBus.js`, `LumaHome.qml` (`appKey`), contract in `settings/AppKeys.js` | `remote.appKeys` (absent = LG's own where installed) |
| Heartbeat written at load, gaps logged | `LumaHome.qml` (`beatOnce`) | none |

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
- The hold's timers are LG's `genericTimer` (the filter's own timer service), so the state machine is plain JS and is tested in bun. A watch of 4 s after the last event clears a hold whose release never came.

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

## Tests

From the repo root. The key filter against stand-ins for LG's globals, and against the real `LumaBus.js`:

- `TZ=America/New_York bun test tests/` (87): `keyfilter.test.js` (49; 36 new: the gear's tap, three taps, hold, repeats, release, watch, lost release, new press, broken timer service, `Key_Option` both ways, no `WebOS`, overlay keys, pairing, focus; the app keys: taken, LG's, each key's name, Home first, the saver, the TV off, LG's overlay on top, not answering, lost release, no code in scope, an error), `bus.test.js` (9: Home or overlay asked for the gear, answering window, throwing handlers, the app key).

LumaHome and the overlay through the offline host (each is one run of about 20 s: `python3 -m unittest discover -s tests -p '<file>'`):

- `test_shell_host.py` (6): number keys (tile 2, 9, 0, 1 = library, no digit in the library, while moving, off Home, the release after Home left, the switch off and live), the gear hold from Home, Settings and the library, a second hold, the switch, the beat at load and with Home hidden and the silent-beat line, the app keys through LumaHome (an app, nothing, gone, in front, unset, over an app).
- `test_shell_overlay.py` (7): idle until an app is in front, the flat glass and Home's backdrop back, a pill that takes no key, buttons and the luna launches, Back, no-signal, the volume key is the app's, the drawer from the gear and Back, the switch, the input pill and its wait for the app, `banners.own` off, the door.
- Existing: `test_shell_surfaces.py` (12), `test_panels.py` (13), `test_plugins_surfaces.py` (6) pass unchanged.
- Scenes, dark and light, looked at: `python3 tests/qmlhost/scenes.py --only 25-overlay-input-pill,25-overlay-input-banner,20-overlay-channel-banner,18-overlay-no-signal,13-overlay-drawer --out /tmp/host-shots`. A generated picture stands in for video (`tests/qmlhost/scenes_host.py`; the host's `--overlay` and `--bgimage`).

The offline host is Qt 5.15; a `Binding` that switches off warns there and not on the set's 5.12, so the backdrop is handed over by assignment (Home on `hostActive`, the overlay on `appFront`).

## Set steps (not verified on the TV)

One slot, Home up, the receiver muted, Picture Mode `filmMaker` as found. Deploy `tools/`, `payload/` and restart the compositor as `lumatv apply` does (the container and the new files are in the shadow module).

1. The compositor's QML logs to `/var/log/messages` (`journalctl` hangs on the set): `grep -E '\[Luma(Keys|Home|Overlay)\]|LumaOverlay failed' /var/log/messages | tail` shows `[LumaKeys] bridge ready`, `[LumaHome] loaded, first beat written` and `[LumaOverlay] loaded`; no `LumaOverlay failed to load` from the container and no `[LumaKeys] ... failed`.
2. Number keys on Home: 2 opens the second tile, 1 the library, 0 the tenth; Back returns with the focus on it; Settings > Remote & pointer off: the digits do nothing. Hold a digit with the switch off: LG's quick access as before.
   App keys: Settings > Remote & pointer > App keys, set Netflix to another installed app; the Netflix button opens that app from Home and from over an app; set Prime Video to nothing: nothing; an unset key opens LG's app where it is installed. With the TV in standby the button still powers it on and opens LG's app.
3. Gear: a tap opens Quick Settings (LG's, redirected); three taps inside 3 s still start the AV reset (do not confirm it); a hold opens the drawer on Home and the long-press ring closes at 650 ms; `/var/log/messages` has `[LumaKeys] gear held, the notification drawer is open`; release, and Quick Settings does not open; LG's Settings app does not open. A second hold closes the drawer. `theme.json` `remote.gearHold` false: a hold is LG's Settings again.
4. Gear over an app (Netflix, YouTube, an HDMI input, Live TV): the drawer draws over the picture, over video included (the overlay's own claim, the one to look at), the D-pad moves it and Back closes it, the app's picture and sound go on; the volume keys still reach the app. Home closes it.
5. Banners (`banners.own` true): switch to HDMI 2 from Home: the pill at the bottom left for 4 s. LG's own banner draws as well (expected: the switch is for when LG's are hidden). `echo '{"kind":"input","icon":"cable","name":"HDMI 2","facts":"","buttons":true,"actions":["inputs","picture"],"holdMs":60000}' > /tmp/<shortId>-overlay` over an input shows a banner with buttons: Right, OK opens LG's Quick Settings, Back leaves; `{"open":"drawer"}` opens the drawer.
6. The frame cost of the overlay over video: `python3 docs/research/perf/gate.py` has no banner surface; `lumatv fps on`, then the compositor's `[FPSLOG]` and `[FPSJANK]` lines in `/var/log/messages` while the pill and the drawer are up over Live TV or HDMI.
7. The heartbeat: with the set in Active Standby, `lumatv apply` or a restart; then `ls -l /tmp/<shortId>-beat` (is there a file and how old), `grep -E 'LumaHome\]|LumaOverlay\]|LumaKeys\]' /var/log/messages*` (is there a `loaded` line with the panel off), and after the wake `the beat was silent for` (timers stopped) or not. The result says which candidate of section 3 holds.

## Not done

- A feed for the channel banner, the pill's resolution and HDR line and the no-signal screen (the pipeline id, section 1). LG's own banners draw those moments.
- The voice bar and the on-screen remote over an app (they are Home surfaces and the voice bar fills Search only): the overlay has the way to host them, not the wiring.
- The fallback system-window route (way 1) is described, not built.
- Whether `global.powerState` reads `active` (the string `StarfishViewStateController.qml:1056` compares) while the TV is on and the key filter runs: if it reads anything else the app keys stay LG's, and step 2 of the TV list shows it. On this set `netflix`, `amazon` and `com.webos.app.lgchannels` are installed and Disney+ is not (`/data/home/lgtv/uiinv/listApps.json`), so an unset Disney+ key does nothing.
