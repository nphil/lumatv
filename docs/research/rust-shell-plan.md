# The Rust shell: LumaTV's screens out of QML and into the 64-bit Rust UI

Plan only (2026-10-29); nothing here is built. Decision it carries out (Nitin, 2026-10-29 00:30 EDT): every LumaTV
screen moves from the QML shell (LG's 32-bit Qt 5.12 in `qml-runner`) to LumaTV's own Rust UI, for performance,
reliability and extensibility. Binding since 00:31 EDT: no new QML; logic and data of every feature in flight go into
Rust or plain data files behind the helper API, and today's QML screens stay only until each Rust replacement is
ready, then go.

Sources: the QML under `payload/compositor/lumatv/` (read 2026-10-29), `compositor/crates/lumatv-compositor/src/`
(`shell.rs`, `ui/`, `luna.rs`), `rustui/`, LumaShow at `/data/home/LumaShow/wt/lumatv` (`f73f0b49`), and the
measurements in `rustui-result.md`, `ui120-result.md`, `compositor-surfaces-result.md`, `shell-architecture.md`,
`lumashow-lessons.md`, `compositor/slot-2026-10-28-1300/`. Figures without a source are marked [INFERENCE].

## 0. Is Rust the best choice for the UI?

The set: Mali-G52 MP3 through the 64-bit Rockchip blob (`gfx64/`), 1080p UI over 4K120, 2 GB RAM with CMA at
48-68 MB free, LumaTV's own compositor live (Rust, `lumatv-compositor`, 51 MB RSS), the shell's budget at 120 Hz
about 7.5 ms of GPU per frame (`ui120-result.md`).

| | 1. Own Rust stack (LumaShow `ui` + renderer, `rustui/`) | 2. Slint | 3. Flutter | 4. Own 64-bit Qt/QML |
|---|---|---|---|---|
| Frame cost, G52 at 120 Hz | Full Home redraw 5-6 ms GPU measured on the set (`rustui-result.md`); damage-only redraw built (host: an idle tick 36 % of the panel, a D-pad frame 24 %); drawn inside the compositor's frame: no copy, no second composition. Fits the 7.5 ms budget [INFERENCE until `tv_ui_measure.py` runs] | GL renderers (FemtoVG on GLES 2, Skia) redraw the window each frame; partial redraw is the software renderer's [INFERENCE]. No backdrop blur primitive: glass is a custom pass. A Wayland client only: plus the compositor's composition. Near Qt's 8-13 ms a frame [INFERENCE] | Impeller on GLES or Skia, whole frame per frame on its own raster thread, a client process: plus composition. 8-12 ms a frame [INFERENCE]. Impeller on GLES in a custom embedder is Android's fallback path, not a measured embedded-Linux one | The measured today: Qt Quick redraws the whole window; a slide 11-13.5 ms GPU at 60 Hz (`ui120-result.md`) |
| Memory | Compositor +40-60 MB (atlas, wallpaper, cached blur; the driver is already in the process) and a 10-25 MB logic process [INFERENCE]; no client window buffers (saves 25 MB of CMA, `gfx64/README.md`) | 40-80 MB client [INFERENCE] + 25 MB CMA window buffers | 100-150 MB (Dart AOT, Skia/Impeller) [INFERENCE] + CMA buffers | `qml-runner` today: 227-232 MB RSS measured (slot 2026-10-28-1300) |
| Start time | Home built in 255 ms on the set (`rustui-result.md`) | ~0.3 s [INFERENCE] | 0.5-1 s to first frame [INFERENCE] | Home window mapped 1.3 s, panels ready 2.4 s (`compositor-gate9-result.md`) |
| Reliability | LumaShow's renderer holds the Show's release gate (0 missed refreshes in 60 s, 1.9 ms CPU per frame, 0 ticks at rest, `shell-architecture.md`); the LumaTV embedding is host-tested only | Mature on desktop and MCUs; unknown on this blob and kernel | Mature; the custom embedder on this set is new C++ | Known: LG's Qt wayland client cadence failed the 120 Hz bar (round 14) |
| Effort to parity | 24 k lines of QML/JS to port; the toolkit's buttons, rows, toggles, sliders, lists, keyboard, toast, text field, glass and tokens exist; new: TV focus model, screens. 35-50 k lines of Rust with tests [INFERENCE] | Every screen in `.slint` plus Rust; glass, D-pad focus and the compositor coupling from scratch | Every screen in Dart; an arm64 engine build from source (no prebuilt for custom embedders), a C++ embedder for our compositor and channel; LG's `flutter-webos` needs webOS 26 Re:New and this set is webOS 24 (platform 10.2.1), and it hosts SAM apps, not a shell | Same QML, rebuilt on Qt 6 |
| Reuse with the Echo Show | Direct: the helper already links LumaShow's `settings-schema`, `plugin-api`, `service-api`, `ha-link` by path; the renderer and `ui` are synced verbatim (`rustui/tools/sync-lumashow.sh`) | None for UI; data crates still shared | None (LumaShow is Rust) | None |
| Licence | MIT (ours) | GPLv3, or a paid seat licence: the royalty-free licence excludes embedded systems (slint.dev/pricing) | BSD-3 | LGPL-3 or commercial |
| Maintainability | One language across helper, compositor, shell and the Show; no vendor; the design language's tokens are code | A second language (`.slint`) and a vendor's roadmap | Three languages (Dart, C++, Rust) and an engine toolchain (GN, depot_tools) | Rejected by Nitin |

Recommendation: the own Rust stack, option 1. It is the only option that draws inside the compositor's frame (no copy,
no second composition, damage-only) which is where the 120 Hz budget is won; its per-frame cost is the only one
measured on this GPU; it is the only one that shares code with the Echo Show, where the same toolkit already passes a
release gate; it carries no licence condition; and the helper, compositor and plugins are Rust already, so the shell
being Rust removes the last toolkit from the product rather than adding one. Slint and Flutter would each be a new
runtime beside the compositor, as a window client: one extra pass per frame is what the Rust client measured as the
thing that tipped it over the gate (`rustui-result.md`: 5 % of frames over 20 ms with the copy pass, 0.4 % without),
and neither can reuse a screen of LumaShow. What option 1 still owes is one measurement: the engine in the compositor has not run
on the set (`compositor/tools/tv_ui_measure.py`, about 5 minutes, written and not run), and the first slice below
starts with it.

## 1. Inventory

### 1.1 The QML shell

148 QML files (21,392 lines, 1.21 MB) and 23 JS files (2,562 lines, 138 KB) under `payload/compositor/lumatv/`,
plus `compositor/shell/Shell.qml` (the two `WebOSWindow`s under LumaTV's compositor) and its three shim imports.

| Area | Files, lines | Screens and components | Rust owner after the port |
|---|---|---|---|
| Root | 19 QML + 9 JS, 6,023 + 1,400 lines | `LumaHome.qml` (1,812 lines: the Home scene, focus, layers, theme merge, tint sampler, trial and mode files), `LumaOverlay.qml` (the system layer over apps), `Layers.qml`, `Dock.qml`, `StatusBar.qml`/`StatusItem.qml`, `Widget.qml`/`Tile.qml`, `LayoutEdit.qml` + `LayoutGrid.js` (the Home layout editor), `Settings.qml` (the Settings shell), `AiPrompt.qml`, `MiniFrame.qml`, `WallVideo.qml` (the video-plane wallpaper's hole), `Warmup.qml`, `Resident.qml`, `KitGallery.qml`, `Glass.qml` (the reference glass), `LumaKeyBridge.qml`, `LumaBus.js` (the key and panel dispatch), `Overlays.js`, `ArtDeck.js`, `Tint.js`, `Saved.js` | `lumatv-ui` crate: `home`, `overlay`, `layers`; models in `lumatv-home-model` |
| `panels/` | 32 QML + 3 JS, 6,331 lines | `Screensaver.qml`, `SaverVideo.qml`, `SaverVote.qml`, `ArtPiece.qml`; `Cameras.qml`; `Switcher.qml` + `SwitcherLayout.js`; `Notify.qml` (toast, alert, input popup, PIN prompt); `Keyboard.qml` + `KeyboardLayouts.js`; `QuickPanel.qml`, `QuickTile.qml`, `QuickList.qml`, `QuickEdit.qml`, `QuickPopup.qml`, `QuickMore.qml`, `QuickStep.qml`, `QuickArrange.js`, `QuickAccess.qml`; `VolumeOsd.qml`; `InputPicker.qml`; `AppsGrid.qml`; `Drawer.qml`; `Overlays.qml`, `OverlayCard.qml`; `Search.qml`; `Banner.qml`, `BannerButton.qml`; `ContextMenu.qml`; `Confirm.qml`; `Splash.qml`; `CaptureMark.qml`; `ScreenRemote.qml`; `VoiceBar.qml`; `PanelMotion.qml` | `lumatv-ui`: `system` (volume, notify, saver, overlays, capture, banner), `panels` (quick, picker, access, drawer, menu, confirm), `apps`, `search`, `switcher`, `keyboard`, `cameras` (the `lumatv-cameras` model's views) |
| `settings/` | 46 QML + 9 JS, 4,410 lines | `PageList.qml` (the page index), 36 `Page*.qml` (Look, Picture, Sound, Equalizer, White balance, Fine tune, Captions, Remote, App key, Inputs, Bluetooth, Bt device, Wifi, IP, Network, Clock, Timer, Language, Channels, Signalling, Permissions, Safety, Delete data, Support, System, Capture, Ambient, Widgets, Home Assistant, Receiver, Ai, Usb sound, …), `HelperSettings.qml`, `LunaCall.qml`, `Atsc3.qml`, `PluginTiles.qml`; tables `Rows.js`, `Choice.js`, `Names.js`, `AppKeys.js`, `QuickAccess.js`, `Capture.js`, `Children.js`, `PluginRows.js` | `lumatv-ui::settings`: one schema-driven page renderer; pages become data (section 2.11) |
| `kit/` | 32 QML + 3 JS, 2,485 lines | `Lu.qml` (the token layer), `LuButton`, `LuRow`, `LuSegmented`, `LuSlider`, `LuToggle`, `LuChip`, `LuTile`, `LuCard`, `LuPane`, `LuText`, `LuIcon`, `LuAppIcon`, `LuLogo`, `LuRing`, `LuHalo`, `LuHelp`, `LuInfo`, `FocusLift`, `Warm`, `CameraFrame`; services `Helper.qml` (the helper client), `Sio.qml` (LG's settings service), `Inputs.qml`, `Apps.qml`, `Net.qml`, `Power.qml`, `PluginHub.qml`, `ReceiverGate.qml`, `TvVolume.qml`, `NoteLog.qml`; `Spatial.js`, `Match.js`, `AudioFormat.js` | LumaShow `ui::components` (button, row, toggle, slider, choice, list, card, plate, icon, label, toast, keyboard) + `lumatv-ui::kit` for the TV-only ones (tile, app icon, halo, ring, pane); the service singletons become the `lumatv-ui::host` client of the channel and the helper |
| `widgets/` | 7 QML, 1,542 lines | Clock (analog face, hands, week strip), Weather, News, Home Assistant, Unknown; `RangeBar`, `WeatherDayRow` | `lumatv-ui::widgets`; LumaShow `shell-smart` has clock face, weather and HA cards to adapt |
| `logo/` | 12 QML + 1 JS, 601 lines | The LumaTV mark and the power-on/off logo variants | `lumatv-ui::logo` (paths as meshes) |

Qt features the QML leans on: `Rectangle` 166, `Timer` 104, `Item` 103, `Repeater` 63, `Text` 55 (plus `LuText`),
`Column`/`Row` 98, `MouseArea` 42, `Image` 27, `NumberAnimation` 17, `Loader` 5, `Canvas` 3 (the clock face, rings),
`ShaderEffect` 15 and `ShaderEffectSource` 6 (the glass: a blurred wallpaper sampled by every pane), `layer.enabled` 3
(the 120 Hz cache of the widgets and the dock), `GaussianBlur` 1. No `ListView`/`GridView` (the lists are hand-built
`Repeater`s with their own focus), no `qsTr` (English copy only), no text-size setting.

### 1.2 LG's Qt plugins and imports

| Import | Files | What is used | Under LumaTV's compositor today | In the Rust shell |
|---|---|---|---|---|
| `QtQuick 2.4/2.9/2.12` | 148 | items, text, animations, shaders | LG's Qt 5.12 in `qml-runner` (32-bit) | LumaShow `ui` + renderer |
| `WebOSServices 1.0` | 21 | `Service { appId, service, method }` ×9 instances; `call`, `callService`, `cancel`, `response(method, payload, token)`; `settings/LunaCall.qml` wraps it as `luna.call` ×37, `luna.act` ×29, `luna.subscribe` ×7 | shadowed by `compositor/shell/imports/WebOSServices/Service.qml` over the UI channel (`POST /v1/luna`) | `host::luna` on the channel; same tags and `cancel` |
| `WebOSCompositorBase 1.0` | 20 | `LS.adhoc.call` ×36, `LS.appId` ×9, `LS.audioService` (volume state) ×2, `LS.applicationManager.launchPointsList` ×2, `LS.calls` ×2 | shadowed by `imports/WebOSCompositorBase/LS.qml` (the `audio` topic, a `listLaunchPoints` subscription) | the `audio` and `launchpoints` topics |
| `TextFile 1.0` (LG) | 8 | 14 instances: `theme.default.json`, `layout.default.json`, `layout.json`, `tint.json`, the beat/mode/confirm files of the Home stub, `/tmp/<id>-layer` and `-overlay` (the development doors), `wallsample.xpm`, `saver-deck.json`, `cache/news.json`, `cache/weather.json` | LG's plugin, still loaded | `std::fs` in the logic process; the doors become `ctl` commands; the caches are the helper's |
| `Eos.Window 0.1` (LG) | `Shell.qml` | `WebOSWindow` ×2 (`_WEBOS_WINDOW_TYPE_LUMATV_HOME`, `_LUMATV_SYSTEM`) | LG's Wayland window | none: the shell has no window (section 2.1) |
| `WebOS.Global 1.0` (LG) | 2 | `WebOS.Key` ×6 | LG's plugin | `input::code` constants |
| `QtQuick.Window 2.2` | 4 | `Screen`, window size | Qt | the `screen` topic |
| `QtGraphicalEffects 1.0` | 1 | `GaussianBlur` | Qt | the renderer's blur |
| `LumaShell 1.0` (ours) | `Shell.qml` | `Link.qml` (the channel client), `Keymap.js` (evdev to Qt keys) | ours | `host::link` |

So under LumaTV's compositor the QML already speaks to nothing of LG's but Qt itself, `TextFile` and the window
type: the port replaces Qt, not an LG contract.

### 1.3 Luna methods the shell calls

All go through the compositor's bridge (`shell.rs` `ALLOWED_SERVICES`, 41 names; the 32-bit supervisor
`sm/lumatv-sm.c` holds the bus), 195-208 calls per gate run. By service, the methods the QML uses:

| Service | Methods | Who needs it after the port |
|---|---|---|
| `com.webos.applicationManager` | `launch` ×8 (the dock, layers, switcher, overlay, status bar), `closeByAppId` ×2, `launchDefaultApp {home}` ×2, `running` (subscription, the frame app) | the Home and overlay models |
| `com.webos.service.homelaunchpoints` | `listLaunchPoints` (subscription), `moveLaunchPoint`, `hideLaunchPoint` | the dock model; the subscription becomes a compositor topic `launchpoints` |
| `com.webos.notification` | `getToastNotification`, `getAlertNotification`, `getInputAlertNotification`, `getPincodePromptNotification` (4 subscriptions), `closePincodePrompt` ×2, `createToast`, `createPincodePrompt` | the system layer (notify); the four feeds become compositor topics so a toast's delivery is timed by the compositor |
| `com.webos.service.audio` | `master/volumeUp`, `volumeDown`, `setSoundOutput`, `tv/getSoundCardsState` | quick panel, Settings |
| `com.webos.service.tvpower` | `power/turnOnScreen`, `power/powerOff` | screensaver |
| `com.webos.service.settings` | `getSystemSettings`, `getSystemSettingDesc`, `setSystemSettings`, `getSystemSettingValues` (`kit/Sio.qml`) | Settings: the LG settings owner (section 2.11) |
| `com.webos.service.config` | `getConfigs` ×4 | Settings |
| `com.webos.service.connectionmanager` | `getStatus`, `getinfo`, `setipv4`, `setdns` | Settings network |
| `com.webos.service.wifi` | `findnetworks`, `connect` | Settings |
| `com.palm.systemservice` | `getPreferences` (sub), `getPreferenceValues`, `setPreferences`, `time/setSystemTime`, `time/getEffectiveBroadcastTime`, `deviceInfo/query`, `osInfo/query` | Settings clock, support |
| `com.webos.service.recent` | `listRecents`, `removeRecent`, `removeAllRecents` | switcher (the compositor owns the list already) |
| `com.webos.surfacemanager` | `setLaunchOrigin` ×2 | Home launches (compositor-internal) |
| `com.webos.service.systemUIManager` | `launch` | overlay fallbacks |
| `com.webos.service.eim`, `applicationmanager` | input list, `getForegroundAppInfo` (`kit/Inputs.qml`) | picker (a topic) |
| `com.webos.service.tvReservationAgent` | `timer/sleep/add`, `getRemainTime` | sleep timer |
| `panelcontroller`, `pqcontroller`, `ppm`, `attachedstoragemanager`, `update`, `sm/deviceid`, `sm/crypto`, `accountmanager`, `admanager`, `userintentmanager`, `usercontextmanager`, `devicereset`, `sdx`, `micomservice`, `miracast`, `asc`, `btaudiosnk`, `btaudiosrc`, `bthidmanager`, `ics/blaster`, `ftms`, `appInstallService`, `lg.thinqai.adapter`, `org.webosbrew.hbchannel.service` | one or two methods each (Settings support, privacy, Bluetooth, inputs, uninstall) | Settings pages |

About 60 distinct methods. None needs a new luna client: the Rust shell calls them as the QML does, through the
channel, and the compositor keeps the allowlist. The helper's own five calls stay `luna-send` (`helper/app/src/ai/luna.rs`).

### 1.4 The compositor ↔ shell channel (`shell.rs`)

Transport today: HTTP on the loopback (`--shell-port`, default 18750), a root-only token, a 25 s long poll.

| Direction | What | Carried as |
|---|---|---|
| to the UI (topics, `push_if_changed`) | `foreground {app, home, covered}`, `screen {on}`, `cursor {visible}`, `keyboard {up}`, `wall {hole}`, `refresh {hz}`, `transitions {on, reduced}`, `audio` (the service's volume state), `capture` (the recording dot and notes), `ime` (the text field), `switcher` (the list and layout) | `GET /v1/state` once, `GET /v1/events?after=` |
| to the UI (events) | `show {kind, warm}` (the gear's tap or Input: `quick`, `picker`, `drawer`), `homekey {switcher}`, `syskey {k, d, r}` (OK and Back taken for an overlay card), `imekey`, `luna {tag, payload}` | the same stream |
| from the UI | `can [quick, picker, drawer, keyboard]`, `wants` (the system layer takes the keys), `live`, `state` (for the gate), `takes [ok, back]` | `POST /v1/ui` |
| from the UI | luna calls `{tag, uri, payload, subscribe}` and `cancel {tag}`; the keyboard (`POST /v1/ime`); the switcher's `open, layout, close, activate, quit, quitall` | `POST /v1/luna`, `/v1/cancel`, `/v1/ime`, `/v1/switcher` |
| not on the channel | keys: `wl_keyboard` on the focused shell window; the pointer: `wl_pointer`; the frame clock: the window's frame callback | Wayland |

The channel's state machine (what the shell can show, who has the keys, the restart policy, the luna allowlist, the
trace markers `Q:<panel>-show`) is the part worth keeping; only the transport and the key path change.

### 1.5 What the shell reads from the helper (`kit/Helper.qml`, API v1 on 18731)

Topics `screen, weather, news, receiver, ai, ai.prompt, plugins, ha.link, updateblock, cameras, cameras.popup,
cameras.live` and per-plugin `plugin.<id>.{cards,tiles,notifications,facts}`; `POST /v1/plugins/<id>/actions/<a>`;
the settings layer `GET/POST /v1/settings/<owner>` on LumaShow's `settings-schema`. The Rust shell keeps this API
unchanged; it is where the in-flight features put their data (section 4.4).

### 1.6 The tests that define parity

`tests/qmlhost/`: the offline host (`qmlhost.py`, Qt 5.15 on software GL), 94 `tests/test_*.py`, 33 of them
screenshot-based; `scenes.py` renders every surface dark and light next to its mock (`docs/mocks/shots/NN-name.png`),
driven by steps `eval, keys, publish, snap, theme, layout`; `layoutaudit.py` finds clipped, crammed, overlapping and
low-contrast words at 100 % and 200 % text (`--audit`); `shellhost.py` runs the client against a stand-in channel;
`compile_all.py` (134 files). On the set: `tv_gate.py` (`shell`, `frames`, `keys`, `ime`, `mem`, `refresh`).

## 2. Target architecture

### 2.1 Where the shell runs

Two shapes were weighed. Both are 64-bit and both reuse the same crates; they differ in who holds the GL context.

| | A. A client process with a window (as `qml-runner` and `lumatv-card-app` are) | B. Renderer in the compositor, logic in its own process (the `ui` feature, built) |
|---|---|---|
| Frame path | app draws into a window buffer, commits; the compositor composes it or scans it out | the engine draws the two layer stacks straight into the scan-out page, inside the owed tiles, in the compositor's own tick |
| Latency, key to pixels | the client's frame, then the compositor's: about two refreshes; measured with the QML client: quick 34.6 ms, picker 37.0 ms, toast 80.6 ms first frame (gate 2026-10-28) | one refresh: a commit wakes the loop and is drawn in the same frame (host: commit to receipt 0.32 ms median, `rustui-result.md`) [INFERENCE on the set] |
| GPU per frame | full window redraw unless a damage ring of its own; plus the flip copy (1.1 ms measured) or a flipped projection; plus composition whenever a system layer is up over an app | damage tiles only; no copy; one pass; a system layer over an app is drawn in the same pass as the app's buffer |
| Direct scan-out | an opaque Home window alone goes to the plane whole (built for the QML Home); a system layer over an app forces composition | Home is stack 0 in the page (a tile redraw per tick, under 1 ms targeted); a full-screen opaque app with nothing of stack 1 up goes to the plane directly (`kms/frames.rs`, host-tested) |
| 120 Hz | the client must make a frame per 8.33 ms callback and the compositor compose it: two pipelines to pace | one pacer (`kms/timing.rs` keyed to 8.33 ms, measured p99 4.85 ms at 120 with the QML shell idle) |
| Memory | a second EGL context and the driver's own pages (the Rust client measured 132-138 MB RSS); 3 contiguous 1080p buffers (25 MB CMA) | one context; the engine's textures (atlas, wallpaper, cached blur, plates) [INFERENCE: 40-60 MB]; the logic process has no GL (10-25 MB) |
| Crash isolation | the client dies, the compositor and the apps keep running; the compositor restarts it (backoff 2-32 s, given up after 5 in 10 min) | the logic process dies: its layers go in one frame, the compositor restarts it; a renderer fault is in the compositor's process (mitigations below) |
| Glass over an app | the panel's window cannot sample the app's buffer (flat colour today, `Lu.flatBackdrop`) | the renderer can sample the page under stack 1 (`Backdrop::Live`), so the quick panel over a composed app gets real glass at the tier's cost; over the video plane neither can |
| Status | the window path is proven (card app, GFN app, the spike) | host-tested in `compositor/tests/ui*.rs`; `tv_ui_measure.py` written and not run |

Choice: B. The renderer runs in the compositor's GL context (`src/ui/mod.rs`, LumaShow's `Engine` from
`rustui/render/host.rs` with `HostConfig::external`), and the shell's logic runs in a separate 64-bit process,
`lumatv-ui`, which owns the layers over the socket pair `Engine::new` returns (LumaShow's `compositor-protocol`,
unchanged) and gets everything else over the UI channel. Why not the logic in a compositor thread: the logic is
where the bugs are (luna replies, JSON, state machines, the helper's long polls), it must never sit on the loop's
thread, and a crash of it must cost one frame, not the compositor. Why not shape A: one extra pass per frame is what
the spike measured as the cost that tipped the Rust client over the gate, the window buffers cost 25 MB of CMA the
set does not have to spare, and the 120 Hz bar leaves no room for two pipelines.

Renderer faults in the compositor's process are contained three ways: every engine call is wrapped (`catch_unwind`
at the `Ui` boundary; a panic disables the UI and the compositor keeps composing windows, as it does when the app
hangs up), the GL error count is already the compositor's (`ctl status`), and the supervisor's fallback to LG's
compositor stays the last resort as it is today. The renderer is the one piece of the shell that is LumaShow's
gate-tested code, which is the reason it may sit in the compositor at all.

### 2.2 Processes and channels

```
lumatv-compositor (64-bit, nice -19, DRM master)
  ├─ scene, windows, keys.rs, input.rs, luna bridge (32-bit sm), switcher, transitions, screencap, oledcare, ime
  ├─ ui/: Engine (LumaShow renderer in the compositor's GLES 3 context): stack 0 under the windows, stack 1 over them
  └─ shell.rs: the UI channel's state (can, wants, takes, show, syskey, luna allowlist, restart policy)
        │ socket pair: compositor-protocol (layers, atlases, backdrops, motions, frame receipts)
        │ unix socket:  the UI channel as JSON lines (topics, events, luna, keys, pointer)
lumatv-ui (64-bit, nice 10, no GL, no Wayland)
  ├─ host: channel client, helper client (API v1), theme.json, fonts, clock
  ├─ screens: system (volume, notify, saver, overlays), panels, home, apps, settings, cameras, keyboard, switcher
  └─ models: pure crates (update(Input, now) -> effects, view(), due_at): cameras, profiles, privacy, layout, …
lumatv-helper (64-bit): feeds, settings store, plugins, cameras, AI, receiver, HA link
```

The UI channel moves from HTTP long-poll to a Unix socket with the same messages, in the same `shell.rs`
(`Msg`, `Outbox` unchanged; `serve` grows a second listener). Two additions: `key {k, d, r, seat}` events while the
shell's stack has the focus (the scene's rule as today: Home drawn means Home has the keys; `wants` means stack 1
has them; `takes` as now for an overlay card's OK and Back), and `pointer {x, y, buttons, visible}` samples at the
pacer's rate while the pointer is over a shell layer. The first slice keeps HTTP for the topics and adds only the
socket for keys, so the compositor change is small and `tests/test_shell_client.py`'s stand-in still serves.

### 2.3 GPU path

64-bit throughout: the compositor already runs the Rockchip Mali-G52 blob (`render/egl.rs`); the engine needs a
GLES 3.0 context (the toolkit's shaders are GLSL ES 3.00; `Ui::new` refuses ES 2, and the blob is expected to answer
3.x to an ES 2 request [INFERENCE: seen on Mesa, not on the set]). Damage tiles of 16 px (`render/src/tiles.rs`),
the page ring's debts (`damage.rs`), the draw cost model and pass merging, covered-layer culling, the defined-texture
rule and the fence-after-draw kick come with the renderer (`lumashow-lessons.md` items 3, 10-15). The 2D blitter,
hardware JPEG and the NPU stay out of the shell's path (`hardware-census.md`). The video-plane wallpaper keeps its
hole through the `wall` topic; the switcher's previews stay the compositor's (`scene/backdrop.rs`).

### 2.4 Text and fonts

Glyph runs from one atlas (`ui::atlas`, `fontdue` raster, every label a run of quads: the spike drew LumaHome with
zero text pictures). Manrope as the one typeface, subset per language actually shipped (`font-subset` crate). No
shaping engine: Latin, digits and the Lucide glyphs cover today's copy and the remote-key hint glyphs
(`assets/glyphs`). A script that needs shaping (Devanagari, Arabic) is a later `rustybuzz` step in `ui::text` and is
not in this plan's sizes.

### 2.5 Glass and effects

The Lucent material from the toolkit: `Backdrop::Cached` (the 30 px blur of the wallpaper, taken once per wallpaper
step), `still_glass` plates for standing glass, `adaptive` (a moving pane loses refraction, keeps blur), half-res
blur, and the forecast-driven quality ladder (live blur to cached, wallpaper to 30 fps, to still, shadows halved,
glass to tint) when the pacer predicts late frames (`lumashow-lessons.md` 6-9). The tier is measured on the set at
120 Hz (`tv_ui_measure.py` idle, slide and D-pad runs), recorded in the design-language ledger as the TV profile's
tier; T1 (cached glass) is the baseline, T2 live glass only where the ladder allows.

### 2.6 Input and focus

Keys and the pointer arrive from the compositor (section 2.2); the shell has no Wayland focus of its own. The model:

- D-pad: spatial, nearest-in-direction between focusables in a scope (the spike's `home.rs`: status bar to cards to
  dock), with focus memory per region, no wrap, explicit guides where geometry misleads (`ten-foot-input.md`);
  LumaShow's `ui::input::focus` registry (scopes, modal scopes own input, hidden content absent) is the base and
  gains the direction search (new, about 600 lines with tests).
- The focus rule: the highlight at once on every key, the lift (1.12 / 1.03) and the name after 150 ms of rest
  (`LumaHome.qml`, already ported in the spike's `home.rs`; a motion track with a hold).
- Pointer, the click-to-select rule: the Magic Remote's hover moves the focus highlight only; a click selects what is
  under it; a D-pad press hides the cursor (the compositor's `cursor` topic) and spatial focus resumes from the
  hovered item. PointerHover's rules land in Rust as data (`lumatv-ui::input::rules`, host tests) and the shell
  reads them. Hit testing is `Focus::hit` with half-open frames.
- OK on release, Back one step at a time, holds timed by `keys.rs` in the compositor (gear, Input, digits, Back,
  Mute), app keys, the number row: unchanged, all compositor-side.

### 2.7 Animation at 120 Hz

Motions run on the compositor's clock (LumaShow `motion` tracks: settle, catch, rest; the pacer latches a frame per
refresh while `every_refresh` holds). The budget is the pacer's: 8.33 ms less the flip's lead and the in-fence
cutoff, about 7.5 ms of GPU, with the next frame's CPU overlapped. What makes it fit: a slide redraws the panel's
tiles and the strip it uncovers, not Home; the dock's focus move redraws two tiles; the wallpaper steps at 10 fps
on its own demand (`Demand::At`). The bar is the gate's: zero `burst_over20` (intervals over 10 ms) on `quick`,
`picker` and `home-dpad` at 120, `toast` first frame under 50 ms.

### 2.8 Accessibility: text size

A text-size setting (100 %, 200 %) in `ShellEnvironment` (LumaShow `shell-api`), applied by the type roles, with the
overlap checker at both sizes as a host test on every screen (the universal rule: no overlapping text at 100 % and
200 %; `layoutaudit.py`'s rules move to a Rust `lint` on the view tree, as `shell-smart::lint`). The TV profile's
sizes come from the design language's `profile.tv`; body at 1080p is never under 28 px.

### 2.9 Localisation

All copy in one place per screen (`words.rs` per area, the pattern of `shade/words.rs`), with a locale table keyed
by LG's `localeInfo.locales.UI` from the settings service; English ships, the table is the extension point. Dates,
times and numbers through `civil.rs`/`calendar.rs` from LumaShow. Fonts follow section 2.4.

### 2.10 Theming: Lucent tokens

One token layer, `lumatv-tokens` (serde): reads the merged `theme.json` (shipped defaults under the owner's edits,
the wallpaper's sampled accent, light or dark by the wallpaper's luma or the setting) and resolves it into the
toolkit's `lucent::Material` and palette roles, the structural tokens from the design language's `tokens.json`
TV profile. The same file feeds `tools/tokens.py` for the web pieces today, so a theme edit reaches every surface.
This is the Lucent v2 port of LumaTV the ledger is waiting for (profile `tv`, tier measured, evidence from the gate).

### 2.11 Settings data model

Every Settings page is data: a schema document (LumaShow `settings-schema`, already the helper's store for
LumaTV's own settings and the plugins') rendered by one page renderer (sections, rows of kinds: toggle, choice,
slider, segmented, action, info, child page). Three owners:

- LumaTV's own settings: the helper's store as today (`GET/POST /v1/settings/<owner>`).
- LG's settings (`picture.backlight`, `general.remotePointer`, `localeInfo`, …): a `lg` owner served by `lumatv-ui`
  over the channel's luna calls (`getSystemSettings`, `getSystemSettingDesc`, `setSystemSettings`), keeping
  `Sio.qml`'s rules: a write shows at once, a key the service does not have is left out, a description that says
  "not now" dims the row, per-app keys ask with the app id, reads one at a time with 2 s each.
- Profiles (the Profiles slice): the profile store and switching live in the helper; the shell shows the profile pill
  and the switch sheet from the `profiles` topic and writes through `POST /v1/profiles/...`. A profile scopes the
  owner's values (theme, layout, quick tiles, app keys); the shell reads the effective values and never merges.

### 2.12 Plugins

The helper's broker speaks LumaShow's `plugin-api` level 1 (cards, tiles, notifications, facts) and the shell draws
those documents as `PluginHub.qml` does; level 2 (plugin-drawn app views as layer scenes) is what the in-process
engine makes possible later: a plugin's scene is a layer group of stack 0, sandboxed by the broker, drawn by the
same renderer. Not in this plan's sizes beyond level 1.

### 2.13 Crash and watchdog behaviour

`lumatv-ui` is supervised by `shell.rs` as `qml-runner` is (spawn, nice 10, `PR_SET_PDEATHSIG` on a long-lived spawn thread,
backoff, give-up after 5 starts in 10 minutes, `ctl shell restart`); its liveness is the channel's `Seen` plus the
engine's commits, so a hung logic process is detected within the beat (3 s) and its layers are dropped with the
restart. LG's Home stub and the `answering()` rule stay: a shell that is not answering never traps the remote.

### 2.14 Crates

| Crate | Where | Content |
|---|---|---|
| `lumatv-ui` (binary) | `rustui/ui/` | the logic process: host, screens, kit, widgets, settings renderer |
| `lumatv-tokens` | `rustui/tokens/` | theme.json to Lucent material and tokens |
| `lumatv-home-model`, `lumatv-panels-model`, `lumatv-cameras`, … | `rustui/models/` | pure screen models with host tests: `App::update(Input, now) -> Vec<Effect>`, `App::view() -> View`, polled `due_at`; serde only, no I/O |
| `ambient` | repo root | existing pure crate, the pattern |
| `lumatv-render` | `rustui/render/` | LumaShow's renderer and host (synced) |
| `ui`, `compositor-protocol`, `shell-api`, `settings-schema`, `plugin-api` | LumaShow by path | unchanged |

## 3. Reuse map

| Piece | From | Ports as-is | Needs TV adaptation | New |
|---|---|---|---|---|
| Renderer, scene, session, atlas, backdrop, glass, plates, damage, ladder, idle, wallpapers | LumaShow `compositor` via `rustui/render` | yes (synced verbatim, two patches) | the page orientation and `Gles::enter/leave` are already LumaTV's patches | |
| Engine in the compositor (`host.rs`, `ui/mod.rs`, `kms/frames.rs`) | `rustui/render`, compositor `ui` feature | yes | run on the set | |
| `ui::components`: button, row, toggle, slider, choice, list, scroll, card, plate, surface, container, icon, label, text, text_field, keyboard, toast, overlay, progress, pager, empty_state | LumaShow `ui` | the drawing and states | sizes and targets from the TV profile; press from OK, not touch; focus visuals (lift, halo) | tile, app icon, halo, ring, pane, status item, quick tile |
| `ui::lucent` (tokens, material, palette, resolve) | LumaShow `ui` | the resolution | the TV profile's numbers; theme.json as the source | `lumatv-tokens` |
| `ui::input::focus` (scopes, hit test), `gesture`, `resample` | LumaShow `ui` | scopes, hit test, resampling | D-pad direction search, focus memory, guides | pointer rules |
| `motion` tracks, `hold`, `feedback` | LumaShow `ui` | yes | the TV motion set (`ten-foot-motion.md`) | |
| `shell-api` (`ShellEnvironment`: size, text size, reduced motion, measured effects; `FrameTime`) | LumaShow | yes | | |
| Clock face, calendar, civil time, weather card, HA card, volume sheet, toast, status bar, dock, launcher, settings page machinery (`settings/`: page, column, cells, pick, slider, wheel, summary), `lint` | LumaShow `shell-smart` | the models and the drawing logic | the Show's numbers are the Show's: relaid out for 1080p at 3 m, D-pad focus instead of touch | |
| Home focus rule, nearest-in-direction, Silk wallpaper tick, theme/layout loading, glass look | `rustui/app/src/home.rs`, `look.rs` | yes | | |
| App lifecycle, window, seats | `rustui/app` (card, GFN) | for the apps | not used by the shell (no window) | |
| Key routing, holds, app keys, switcher model, transitions, IME server, OLED care, capture | compositor | yes, untouched | | |
| Settings store, plugins broker, feeds, cameras, AI, receiver | helper | yes, untouched | | the `profiles` and `privacy` topics (in flight) |
| Mocks (`docs/mocks/`), scene scripts, layout audit rules | tests | the mocks are the spec | scene steps re-expressed for the Rust host | Rust host harness |

## 4. Migration

### 4.1 The switch and the fallback

Per surface, in the compositor: `--shell-rust LIST` (and `ctl shell rust +quick -picker …`) names the kinds the
Rust shell answers; `can` from each client is intersected with it, so for a kind on the list the compositor's
`show`, the feeds and the keys go to `lumatv-ui`, for the rest to the QML client. Both clients run during the
migration (a memory peak of one more process, about 30 MB [INFERENCE]; the QML client loses surfaces as the list
grows and is not started at all once the list is complete). The QML shell stays the fallback for a kind until its
Rust surface has passed the gate on the set twice; the switch is a `ctl` command, so a bad slice is turned off from
the workstation without a deploy. Home is one kind (`home`), Settings another (`settings`), and while Home is QML
the Rust client draws only on stack 1.

### 4.2 Parity tests

- Host renders: the Rust host (`lumatv-ui --host` on Mesa, the engine in-process as the compositor runs it) renders
  every scene of `tests/qmlhost/scenes*.py` dark and light into `docs/mocks/host-rust/`, same names, so the Rust
  picture sits beside the QML one and the mock. The scene steps (`publish`, `keys`, `snap`, `theme`, `layout`) are
  the same vocabulary over a stand-in channel and a fake helper (`tests/qmlhost/fakehelper.py`, reused as is).
- Behaviour: `tests/test_shell_client.py`, `test_notify_panel.py`, `test_quick_panels.py`, `test_switcher.py`,
  `test_overlays.py`, `test_settings*.py` state what each surface does on a topic, a feed or a key; each is rerun
  against the Rust client through the same stand-in before its kind flips. A test that passes on both is the parity.
- Look: `layoutaudit` rules (clipping, crowding, overlap at 100 % and 200 %, composited contrast 4.5:1 and 3:1) as a
  Rust lint on the view tree plus the picture, run on every scene; the contrast audit stays pixel-based.
- Set: `tv_gate.py` unchanged in its checks (`shell`, `frames`, `keys`, `ime`, `mem`, `refresh 120`); the per-kind
  flip is what the gate's `--shell-rust` option sets before the run.

### 4.3 Order

1. System layer, small and keyless: volume OSD (the `audio` topic), the toast; then alerts, the input popup, the
   PIN prompt (the four feeds), the capture mark, the screensaver, the power logos, the overlay cards and banners.
2. Panels: the quick panel (tiles, list, edit, popups, more), the input picker, Quick Access, the drawer, the
   context menu, confirm, the app switcher's chrome, the own keyboard. This is where keys, `can`/`show`, warm-up
   and the 120 Hz slides are proven.
3. Home: wallpaper (still, Silk, the video hole), status bar and popovers, widgets (clock, weather, news, HA,
   plugin cards), dock and tiles, the apps grid, search and the voice bar, the AI prompt, the mini frame, the layout
   editor, cameras (the `lumatv-cameras` model's views), the saver on Home, the warm-up. Home flips last of the three
   because it is the largest and because everything above it must already be Rust so that stack 0 and stack 1 are
   one scene with real glass between them.
4. Settings: the page renderer, the LG owner, then the pages as data, support and privacy pages last (they are
   luna-heavy and rare).
5. Retire: `--shell auto` loses the QML path, `compositor/shell/` and `payload/compositor/lumatv/` are deleted, the
   `qmlhost` tests go with them, `tools/lumatv` stops staging the module, the ipk shrinks by the QML and its PNGs.

### 4.4 In-flight work carries over

| Slice | Where its logic and data live (Rust/helper, under the no-new-QML rule) | What the Rust shell draws |
|---|---|---|
| Overlay layout editor (OverlayLayout) | built: the pure crate `placement/` (`lumatv-placement`: registry, zones, anchors and offsets, `layout_all`/`rect_of`, stepping, volume bar edges, drift; `Editor::key(Key, now) -> Vec<Effect>`, `view(now) -> View`, pointer, `panel_spot`) and the stateless `/v1/placement` API in the helper; the owner's map stays `layout.overlays` in theme.json (profile scope), the editor asks its caller to save (`Effect::Save(map)`); `docs/research/overlay-layout-result.md`, mocks `docs/mocks/overlay-layout/` | the overlay cards and the volume bar read `rect_of` (order 1); the editor scene on stack 1 and its Settings row (order 2) |
| Privacy menu (PrivacyMenu, AmazOff) | the switches and their mechanisms in the helper (`privacy` owner in the settings store; the luna acts behind `PageDeleteData`/`PageSupport` move with them) | one schema page in order 4 |
| Profiles | built: `helper/app/src/profiles.rs` (topic `profiles` with `canAdmin`; `GET /v1/profiles`, `POST /v1/profiles/switch {id, pin}`, create, update, delete, `pin`, `avatar`, `options`, `gallery`); design `docs/research/profiles-design.md` | seven Rust screens, none in QML: the status-bar chip (rounded-square avatar and name) and the profile strip as the quick panel's first row (order 2); the "Who's watching?" picker with its PIN pad (digits auto-submit, 3x4 pad for the D-pad, shake on wrong, `pin_locked` with `retryAfterS`; on wake when `pickerOnWake`, under 150 ms), the switch veil (opaque while the new Home is swapped in, 300 ms budget) and a profile's first run (order 3); Settings > Profiles (add, name, colour, avatar from gallery, photo or initial, PIN, remove with danger and confirm, the options; rows locked by `canAdmin` with "Only <owner> can change this") (order 4) |
| Glyph hints (GlyphHints) | `assets/glyphs` SVGs and index (toolkit-neutral); the hint model names glyph tokens | glyph meshes in the atlas; hints on panels and cameras from order 2 |
| Cameras UI (ScryptedShell, ScryptedBridge, ScryptedHelper) | no crate yet: the spec `docs/research/cameras-ui-spec.md` (behaviour, numbers, words, timings, test vectors) and the mocks `docs/mocks/cameras-scrypted/` (every view, dark/light, 100/200 %) are the input; `lumatv-cameras` is built from them in phase 3, its test vectors the host-test seed; streams and events in the helper (`cameras.*` topics) | the pop-up, grid, full view, timeline strip and sheet from `View` (order 3); the Settings rows from the `scrypted-cameras` schema (order 4) |
| Pointer hover and click-to-select (PointerHover) | the rules as data with host tests | section 2.6 |
| App update block (AppUpdateBlock) | the helper's `updateblock` topic and actions | a Settings row and a toast (orders 1 and 4) |
| Tiny QML fixes shipping tonight | stay in QML until their kind flips; nothing new is added | |

The rule that makes the ports thin: a screen model is a pure crate (`update(Input, now) -> Vec<Effect>`, `view() ->
View`, polled `due_at`), I/O is the host's, and the QML of the same screen is deleted the day the Rust view passes
its parity tests.

## 5. Gains, risks, roadmap

### 5.1 Expected gains

| | Today (QML in `qml-runner`) | Rust shell | Source |
|---|---|---|---|
| Shell memory | 227-232 MB RSS, plus 25 MB CMA for the Home window's buffers | compositor +40-60 MB, `lumatv-ui` 10-25 MB, no CMA buffers: 120-150 MB less [INFERENCE] | slot 2026-10-28-1300; `rustui-result.md` (client 132-138 MB with its own driver pages) |
| Full Home frame | 11-13.5 ms GPU (a slide at 60 Hz), 8.1-9 ms idle | 5-6 ms for a whole redraw measured; 1.5-2.5 ms with damage tiles [INFERENCE from 24-36 % of pixels] | `ui120-result.md`, `rustui-result.md` |
| Idle Home | 10 commits/s, 3 % of a core at nice 10, the whole window per commit | tiles of the clock hand and the wallpaper step; under 1 ms GPU per 10 fps frame is the gate | `compositor-surfaces-result.md`, `tv_ui_measure.py` |
| First frame of a panel | quick 34.6 ms, picker 37.0 ms, toast 80.6 ms (limit 50) | one refresh after the key: 16.7 ms at 60, 8.3 at 120 [INFERENCE] | gate 2026-10-28 |
| 120 Hz | quick `burst_over20` 32, picker 35, cadence-bound (Qt) | the compositor's pacer only; p99 pipeline 4.85 ms already | round 14 |
| Start to first picture | Home mapped 1.3 s, panels 2.4 s | Home built 255 ms; process start under 100 ms [INFERENCE] | `compositor-gate9-result.md`, `rustui-result.md` |
| Crash | the client restarts in 2-32 s; a QML exception leaves a surface half-built | the logic process restarts in under a second, layers gone in one frame; renderer faults contained as 2.1 says | `shell.rs` |
| Qt libraries mapped | LG's 32-bit Qt 5.12 and its plugins (page cache) | none | |
| Glass over apps | flat colour outside Home's window | real glass over composed apps | section 2.1 |

### 5.2 Risks

1. The engine has not run on the set: the ES 3 context from the blob, the idle cost, the GPU per slide. Settled by
   `tv_ui_measure.py` in the first slice; a fail here reverts to shape A for the shell (the window path is proven)
   with the same crates.
2. A renderer fault takes the compositor down with every app. Contained by the `Ui` boundary; the supervisor's
   fallback stays. The renderer's own history (the Show's gate, `compositor/tests/ui*.rs`) is the argument.
3. Pixel parity is not the goal and must not become one: the mocks are the spec, the tests are behaviour and the
   look rules; a port that copies QML quirks pixel for pixel is slower to write and worse.
4. Two shells during the migration: more memory for a few weeks, and a kind that is half on each side (Home QML,
   panels Rust) has flat glass between them. The order in 4.3 keeps each kind whole.
5. Text: no shaping; a non-Latin UI language is a later step.
6. Pointer: new code with no Show precedent; PointerHover's data and the gate's pointer tests are the net.
7. In-flight QML that is not tiny: the rule stops it; what shipped stays until its kind flips.

### 5.3 Roadmap

| Phase | Content | Size [INFERENCE] | Exit |
|---|---|---|---|
| 0 | `tv_ui_measure.py` on the set; the `lumatv-ui` skeleton (channel client over the socket pair, keys over the socket, theme.json tokens, fonts, host harness with scenes); the per-kind switch in `shell.rs` | S-M, 4-6 k lines | the idle gate (under 1 ms GPU a frame), direct scan-out under a card, a demo layer over Home on the set |
| 1 | system layer: volume, toast, alert, input popup, PIN, capture mark, saver, logos, overlay cards, banners | M, 6-8 k | `shell` and `frames` gates with `toast` under 50 ms; QML `Notify`, `VolumeOsd`, `Screensaver`, `Overlays` deleted |
| 2 | panels: quick (all parts), picker, Quick Access, drawer, context menu, confirm, switcher chrome, keyboard | L, 8-10 k | `refresh 120` gate: zero `burst_over20` on quick and picker; QML panels deleted |
| 3 | Home: wallpaper, status bar, widgets, dock, apps grid, search, AI prompt, mini frame, layout editor, cameras views | XL, 12-15 k | `home-dpad` at 120 with no drop; `LumaHome.qml` and its tree deleted; LG's Settings page of Home gone |
| 4 | Settings: renderer, LG owner, pages as data, profiles and privacy pages | L, 6-8 k | every `settings` scene passes the lint at 100 % and 200 %; `settings/` deleted |
| 5 | retire `qml-runner`, delete the module, the shims and the `qmlhost` tests; ledger entry Lucent@2 TV profile with the measured tier | S | the ipk without QML; `--shell` has no QML mode |

About 40-50 k lines of Rust with tests over the five phases; phases 1-2 and 3-4 can each run as two parallel
slices once phase 0 has fixed the host harness and the channel.

### 5.4 First slice to build

Phase 0 plus the volume OSD and the toast of phase 1, as one deliverable:

1. Run `python3 compositor/tools/tv_ui_measure.py` in a KMS window (5 min): the idle, slide, volume and direct
   runs, `ctl status`'s `ui frames … gl errors` and the capture. Decides shape B.
2. `rustui/ui/`: `lumatv-ui` with the channel client (HTTP topics as today, the new key socket), `lumatv-tokens`
   from `theme.json`, Manrope subset, the Lucent material on stack 1, a host harness that renders scenes to PNG on
   Mesa through the same `Engine` path.
3. `shell.rs`: `--shell-rust osd,toast`, the key socket, the `can` intersection, the second client's supervision.
4. The volume OSD from the `audio` topic and the toast from the toast feed, with their scenes rendered dark and
   light beside `docs/mocks/shots/`, `test_shell_client.py`'s OSD and toast cases passing against the Rust client,
   and `tv_gate.py --only shell,frames` on the set with `osd ok` and `toast` first frame under 50 ms.

Nothing of the QML is touched by this slice; `VolumeOsd.qml` and the toast of `Notify.qml` are deleted when the gate
has passed twice.
