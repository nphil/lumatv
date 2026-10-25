# Shell slice: library, search, voice bar, banners, drawer and saver wiring, shared web theme

Everything here is on `main` and passes on the offline host: `test_shell_surfaces.py` (12), `test_settings.py` (31), `test_panels.py` (13), `test_volume_osd.py` (2), `test_logos.py` (7), `test_scenes_loader.py` (3), `test_tokens.py` (15), `test_webapps.py` (2), `test_gate.py` (46), `test_surfaces_registry.py` (5), `bun test tests/` (42), and the shell scenes in dark and light. Nothing has run on the TV, and no timing was measured.

## Resume here

Needs the set:

1. `python3 docs/research/perf/gate.py --surfaces apps` with LumaTV's Home in front and its Apps tile focused: first frame inside 50 ms and p90 render inside 3 ms for the library. Search, the drawer and the voice bar sit behind key paths a gate surface cannot spell out yet.
2. `docs/research/perf/startup.py` A/B on the warm-up order (see "Build order" below). The warm-up still waits on `layers.built`; that was kept, and its cost on the set is unmeasured.
3. The luna calls below were written from the service names and never run. If one fails, only the named action fails.
4. Banner, voice bar and the on-screen remote over video need the system-window host that is not built; inside Home they work.

Open for Nitin:

- Voice: PLAN says search only, no LG cloud. Nothing starts the bar: `voiceStart()`, `voiceHeard(words)` and `showMic(on)` on Home are the API a recogniser would call, and the mic key stays LG's (SURFACES: untouched). The search keyboard's "Hold the mic key to speak" hint shows only when `theme.voice.enabled`.
- Web search engine: `theme.search.url`, default `https://duckduckgo.com/?q=`.

Offline, not done:

- Restyles of LG's own processes (the `inputcommon` banners, maliit keyboard, voice app) through the overlay mount.
- The input banner's Details card (mock 25-input-info): the button opens Settings > Inputs instead. The update badge on the banner. Search answers beyond apps, Settings pages and the web (no "Network for AI" style answers).
- The gear hold (below) and the pointer's right-button menu; the power menu opener (no key reaches Home).
- The launch redirect for LG's Settings app does not exist yet: nothing in `tools/lumatv`, the key filter or the compositor patch names `com.palm.app.settings` (PLAN's "Launch redirect" rows are plans). Whoever builds it must let a launch with `params.target` through (LumaTV's own Settings opens LG's deep screens that way) and redirect only plain launches (gear hold, remote); there is no code to test until then.

## What exists and where it is wired

- **Library** (`panels/AppsGrid.qml`, data in `kit/Apps.qml`): the Apps tile's own grid. Rail of All apps, categories, On the dock and Hidden at the foot; five columns, three rows in view; search button; hold OK on an icon opens its options (open, move on the dock, hide or show again, rename, uninstall). Apps come from `launchPointsList` minus bookmarks and LG's hidden system apps (Live TV stays, the store is "LG Content Store"). Hide and rename are the owner's (`theme.apps.hidden`, `theme.apps.names`, written through `patchTheme`/`persistTheme`); the shipped `theme.json` is untouched.
- **Search** (`panels/Search.qml`, matching in `kit/Match.js`): the keyboard sheet of mock 09 with up to four answers (two apps, Settings pages, the web). The same sheet is the rename keyboard (`mode: "entry"`). Opens from the status bar's search icon, the Search key and the library's button.
- **Voice bar** (`panels/VoiceBar.qml`): listening, heard and options states, and the mic indicator. Back stops it.
- **Banners** (`panels/Banner.qml`, `BannerButton.qml`): channel banner, input pill (grows into Details, Inputs, Picture), no-signal screen whose block drifts to a new spot each minute. Live TV has the banner only, no guide.
- **Context menu and confirm** (`panels/ContextMenu.qml`, `Confirm.qml`): the menu sits beside its tile or icon, which stays lit above the scrim; every answer is followed by `closeRequested`. Uninstall asks first and Keep it is the default; only apps LG marks removable and that are not protected ids get Uninstall.
- **Residents** (`Layers.qml`, `Resident.qml`): apps (step 8, z 61), search (9, z 62), voice (10, z 91), banner (11, z 72), drawer (12, z 67), saver (13, z 95); `steps` is 14. `below[]` keeps the surface under the open one drawn without keys, so a menu over the library and the keyboard over the library return to the library on Back. A resident reports `done` on the next event-loop turn: a synchronous build otherwise moved the step from inside its own `build` binding, which the gate's `Binding loop` pattern would flag.
- **Home** (`LumaHome.qml`): layer stack (`openLayer`, `closeLayer`, `closeAllLayers`, `closeLayerById`), the openers and the actions behind them (`openApps`, `openSearch`, `appMenu`, `menuPicked`, `askUninstall`, `renameApp`, `showRemote`, `showBanner`, `voice*`). An unknown layer name is refused with a warning and a stale one never swallows keys. Home's old `showMenu`, `showConfirm` and `showRemote` have real openers and their signals (`menuPicked`, `confirmAnswered`, `remoteKey`) are connected.
- **Dock** (`Dock.qml`): the Apps tile opens the library instead of LG's store; options on a tile; a moved tile is written back to LG's dock list; renamed titles show.
- **Status bar** (`StatusBar.qml`): search item opens Home's search; the bell opens the drawer and its badge is `NoteLog.unread`. The old toast popover and its own toast subscription are gone (the log owns the subscription).
- **Drawer and saver** (PluginsSlice's panels, wired here): the bell opens `panels/Drawer.qml`, Back closes it and the bell keeps the focus. `panels/Screensaver.qml` is shown while its `active` is true; while it is up every key wakes the TV through `item.key()` and does nothing else, and the matching release is taken too (an OK release alone would open the focused tile). The AI prompt is the exception: it is drawn above the saver and only the physical remote answers it.
- **Scenes harness** (`tests/qmlhost/scenes.py`): any `scenes_*.py` module registers scenes with `@scene(name, runner=False, materials=("dark","light"))`; each snap writes `<name>.png` (dark) and `<name>-light.png`; a failed module load, a missing PNG or a duplicate name fails the run. `qmlhost.py` retries the helper port after the lock and parks the pointer before the window maps (it used to hover the news widget and start Home on the widgets layer).
- **Shared web theme**: `tools/tokens.py ... web` writes Lucent's tokens as CSS for both materials (`:root, [data-theme="dark"]` and `[data-theme="light"]`, four Manrope faces, type, space, radii, motion). The frame app (`frame/`) now reads it: `tools/build-ipk.sh` generates `theme.css` beside it, `frame.js` sets `data-theme`, `frame.css` uses only `var(--lu-*)`. `payload/webapps/apps.txt` lists the 16 transparent Enact apps `lumatv webglass on` mounts the existing `lumatv.css` on (Settings, Notification Centre, the wizards, tuning, support, scheduler, channel editor and the rest); it was two apps. Rendered offline in Chromium in both materials.
- **Host stubs**: `FakeApps.js` (23 library apps, an HDMI bookmark, hidden system apps) and the `WebOS.Key_webOS_ScreenRemote` constant.

## Keys

| Key | Does |
|---|---|
| OK on the Apps tile | Opens the library; Back returns to the tile |
| Hold OK (0.7 s) on a dock tile or a library icon | That app's options |
| Menu (a keyboard's; the remote has none) | Library: options of the icon. Dock tile: its options. Elsewhere: the quick panel when `theme.quickPanel.own`, else the library. Consumed whenever Home is active |
| Search (keyboard) or the bar's search icon | Search |
| 123 (Screen Remote) | The on-screen remote; again closes it |
| Bell on the bar | Notification drawer |
| Back | Closes the top surface; the one under it takes the keys again, and the focus is where it was |
| Anything while the saver is up | Wakes the TV; nothing else |
| Mic, volume | Untouched (volume is only watched for the receiver's OSD) |
| Digits | Untouched: they could be swallowed from the PIN prompt |

The gear hold is not wired. The gear is `WebOS.Key_webOS_Settings`: a tap opens Quick Settings on release and LG's 1 s hold turns into `Key_Option` and launches LG Settings, so a hold that opens the drawer needs the key filter (`payload/keyfilter/lumatv.js`) to cancel LG's long-press timer the way the Home hold does. It cannot be tried without the set and a mistake there breaks every key, so the bell is the way in.

## Not verified on the TV

- `homelaunchpoints/hideLaunchPoint` (hide from the dock) and `appInstallService/remove` (uninstall) parameters. The uninstall path is guarded by the confirm dialog and the removable flag; LG's own apps are never offered it.
- `micomservice/sendKeycode` with decimal IR codes for the on-screen remote's keys (marked [INFERENCE] in Home).
- Moving a dock tile writes LG's list through `moveLaunchPoint`; the host records the call, the set decides if it sticks.
- The frame app with the generated `theme.css`, and the Enact apps with the longer `webglass` list. `webglass on` is an owner switch, off by default.
- Everything about speed: resident builds, first-frame times, the drift animation, the library's scroll.

## Build order

The build chain is meant to be serial. Always-shown surfaces (the OSD, the splash, the banner) are wanted as soon as Home is active, finish early, and call `advance(step)` while the chain is still at -1, which jumps it past the steps in between. In the host the surfaces therefore all build at once, in reverse step order (instrumented run), and `layers.built` is true long before Settings' pages are. The real set builds asynchronously, so the effect there is concurrent incubation instead of one surface per turn. A strict `buildStep === step` would make it serial, but then a call on a surface not yet built (`layers.searchOpen(spec)` before search's turn) drops its payload, so that change needs a queue on the resident first. Left as found; the `startup.py` A/B is what decides.

## Tests and scenes

Run from the repo root.

- `python3 -m unittest discover -s tests -p 'test_shell_surfaces.py'`: library open/Back/focus return, Menu key, dock move written back, hide and rename written to the theme file, uninstall asks and Keep it is safe, search from the bar, voice, banners with and without buttons, the on-screen remote, an unknown layer name, the bell and the drawer, the saver's keys.
- `python3 -m unittest discover -s tests -p 'test_settings.py'`, `test_panels.py`, `test_volume_osd.py`, `test_logos.py`, `test_scenes_loader.py`, `test_tokens.py`, `test_webapps.py`, `test_gate.py`, `test_surfaces_registry.py`; `TZ=America/New_York bun test tests/` (includes `match.test.js`).
- Scenes (dark and light, 16 of them, one per mock plus the Hidden section, the dock tile's menu, the uninstall question and the rename keyboard): `python3 tests/qmlhost/scenes.py --out /tmp/shell-shots --only 12-all-apps,12-all-apps-all,12-all-apps-hidden,12-all-apps-menu,16-context-menu-dock,06-confirm-uninstall,09-search-keyboard,09-search-rename,28-voice-listening,28-voice-heard,28-voice-results,28-mic-indicator,20-channel-banner,25-input-pill,25-input-banner,18-no-signal`. All 32 renders were looked at against `docs/mocks/shots`.
- Web theme: `python3 tools/tokens.py payload/theme/theme.json /dev/null web`.
