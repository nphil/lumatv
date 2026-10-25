# Track C2: a 64-bit Rust compositor on a private socket

Design and first-light implementation of LumaTV's own compositor, built
beside LG's `surface-manager` (never replacing it in this step). Code lives
in `compositor/` (Cargo workspace; crates `lumatv-wl-protocols` and
`lumatv-compositor`). Everything here was done offline on the workstation;
TV runs are listed as ready scripts with their expected results. Facts
marked [INFERENCE] were not observed on the set.

## Resume here

State (2026-10-25 14:45 EDT, code at `dd793ff`): C2 passed; the fourth gate
ran on the set 14:18-14:27 with the fullscreen-request, placement and loop
changes of `221b75b`, `168f69a` and `8f69c05` ("Fourth gate on the set";
files in `docs/research/compositor/slot-2026-10-25-1418/`): live, volume,
netflix, standby/wake and escape PASS; x1 FAIL by the rule but down to 2
gaps over 20 ms (59.95 fps, worst 24.7 ms in the first second, `late` 2);
hdmi FAIL on the buffer only: the preloaded HDMI 2 instance is raised and
SAM's foreground now, and draws nothing because this compositor never sends
`wl_webos_shell_surface.exposed`, which LG's Qt platform plugin turns into
Qt's expose (open; the fix is to send it). LG's compositor is live (set in
Active Standby, `enabled` absent, Home stub on, receiver unmuted).
`tv_ui_measure.py` ran after it: idle 0.51 ms of GPU a frame at 10 fps,
the RustUi gate passes; DIRECT shows no direct frame for a libmali client,
as known. Host: `cd compositor && cargo test` (73 unit, 25 protocol, 1 bridge, 1 control,
2 LG-table), the same with `--features ui` plus `ui`, `ui_output` and
`ui_scanout` under `LIBGL_ALWAYS_SOFTWARE=1`, `sm/build.sh`, clippy clean
both ways; `sh compositor/tools/build-tv.sh` builds the bundle.

- Proven on the set: all 12 bus names are LumaTV's at LG's path; a 120 s
  trial and three gate runs live with no watchdog fallback, `READY=1`
  accepted, WebAppMgr reconnecting; KMS pacing 59.82-59.94 fps over 60 s;
  standby and wake through the tvpower rounds; the hold-Home escape (LG's
  back in 14-15 s); direct scan-out of LG's AFBC buffer on the primary
  plane (own buffers only; driver buffers compose). Second gate: LG's
  protocol tables (Netflix plays Dolby Vision, 3 placements, no protocol
  errors), SAM's queued subscription (Home, Netflix and HDMI 2 reported in
  turn), HDMI 2 mapped and in front, Volume keys as LG's audio calls (the
  receiver stepped 0.355 → 0.36 → 0.355), clients named at accept (the 7 s
  client is gone), frame callbacks at the page flip.
- Run on the set in the third gate: the frame latch learns the GPU's
  completion from the in-fence's signal time (x1's `late` 11 → 2 a minute,
  gaps over 20 ms 16 → 6, the worst one at start); odd page-flip stamps
  counted (7 in 160 s) and logged; the covered card minimized (Home under
  Netflix) and fullscreen again when the card goes. Still unverified live:
  composed frames top row first, flattened luna payloads.
- Run on the set in the fourth gate ("Fourth gate on the set"): a card's
  `set_state(fullscreen)` makes it the foreground before its first buffer
  (Netflix 6 ms before, HDMI 2 with no buffer at all; Home back over each);
  placements 3840x2160 with `fullScreen`, one per attach, none before the
  rectangle, 0x0 to hide; video mute is the app's `set_property`, relayed;
  `gl warm-up 9.3 ms`, `render:` lines attribute slow frames (x1's
  first-second stall of 18.8 ms has none: not in the draw); the capture's
  loop pass 573-590 ms → 117-136 ms (the read-back remains on the loop).
  Netflix's player error of the third gate did not recur (one pass each
  way; the cause is not proven). Still open: the x1 rule (2 gaps in 60 s),
  the `exposed` event (HDMI 2's buffer), Netflix's three restarts in the
  third gate's trial (none in the fourth).
- Next: send `exposed` (the window's rectangle when a card comes to the
  front or maps, empty when minimized), then `python3
  compositor/tools/tv_next_slot.py` again with the HDMI 2 app preloaded
  (the memory note keeps it so); the set needs the current `tools/lumatv`
  (`comp_live` guard) or the script turns the Home stub off for the session
  and back on. `tv_ui_measure.py` need not run again unless the engine
  changes (idle 0.51 ms, anim60 0.40 ms, volume 0.65 ms of GPU a frame;
  results in `docs/research/compositor/ui-measure-2026-10-25-1430/`).
- Go-live: not before the surfaces exist under LumaTV's compositor
  ("Surfaces under LumaTV's compositor": key filters and IME, LumaHome,
  the volume OSD, toasts, alerts and PIN, quick panel and picker). The
  engine host (feature `ui`) is built, host-tested and measured on the set
  (idle gate passed). The surface gate (`perf/gate.py`) under this
  compositor waits on a client for the QML surfaces, which does not exist.
- Built and host-tested, unverified on the set: the Magic Remote's pointer
  (`ctl pointer X Y`, `ctl click`).

## Design

### 1. What C2 is and what "first light" means

C2 is the compositor process that will replace `surface-manager` in C4,
grown far enough that an unmodified LG app can draw into it. In C2 it
listens on a private socket (`/tmp/lumatv/wayland-lumatv`), LG's compositor
keeps the screen, the DRM master and every luna name, and nothing LumaTV
does is visible on the panel.

First light = all of the following on the set, in one run of
`compositor/tools/tv-first-light.sh`:

1. `lumatv-compositor` (64-bit) starts on the private socket with the
   64-bit Mali blob bound to its `wl_display` (`eglBindWaylandDisplayWL`,
   the X2 r8b path) and advertises `wl_compositor`, `wl_subcompositor`,
   `wl_shm`, `wl_shell`, `wl_seat`, `wl_output`, `wl_webos_shell`,
   `wl_webos_surface_group_compositor`, `wl_webos_foreign`,
   `wl_webos_input_manager`, `text_model_factory`, `input_method`,
   `input_panel`, `wl_webos_xinput_extension`, `wl_webos_system_ux`,
   `zwp_linux_dmabuf_v1` and the blob's `mali_buffer_sharing` v5.
2. A 32-bit LG client drawing through LG's own `libmali.so.0.46.0` (the X2
   client, 1920x1080, six frames) connects, creates a `wl_shell` toplevel
   with a `wl_webos_shell_surface` of type `_WEBOS_WINDOW_TYPE_CARD`, and
   receives `configure`, `state_changed(fullscreen)` and a frame callback
   per commit.
3. Every committed AFBC buffer is imported with `EGL_WAYLAND_BUFFER_WL`,
   composed into the compositor's offscreen 1920x1080 target, and the
   target is read back and written as PNG. The PNG's pixels equal the
   colour the client cleared that frame to, pixel-exact (same check as X2:
   frame k is R = 25k, G = 0x80, B = 255 - 25k).
4. The same run repeated with the compositor's own `mali_buffer_sharing`
   global and `EGL_EXT_image_dma_buf_import_modifiers` import (the path C3
   needs for direct scan-out, since only it keeps the dma-buf fd). Result
   recorded either way; this one was never tried on the set.
5. One unmodified LG app launched by hand against the socket (see §10)
   maps a window and its first frame is captured the same way; here the
   check is "a non-blank frame with the app's expected size", not
   pixel-exact, because the app's content is not known in advance.

### 2. Reuse or new

Read: LumaShow `compositor` (its own single-client layer protocol, Android
Mali GL, Echo Show planes), `compositor-protocol` (that protocol),
`planes` (device-agnostic plane trait), `displayd` and `display-mtk`
(MediaTek display manager in a 64-bit helper), the X2 harness, the OSE
`luna-surfacemanager` sources (`/tmp/ose/luna-surfacemanager`, read only),
LG's compositor QML on the set (`/data/home/lgtv/uiinv/qml`).

- New: the Wayland server, LG protocol handling, scene, EGL import and the
  offscreen output. LumaShow's compositor is not a Wayland compositor and
  its renderer draws its own layer records; nothing of it maps onto
  `wl_surface` trees, foreign buffers or LG's window types. Its *patterns*
  are taken (listed per item in §9).
- Reused later, unchanged, by path dependency (not now, because C2 has no
  screen): `planes` (the plane contract: offer, begin, buffer, present,
  check, capture, reset) as the interface C3's KMS backend implements;
  `pacer.rs`'s start-late/aim-early logic once there is a vsync timestamp;
  `damage.rs`'s region type; `timeline` for the per-frame trace.
  `display-mtk`/`displayd` are MediaTek-only and stay out.
- `wayland-server` 0.31 (Smithay's protocol crates, not Smithay itself):
  Smithay dropped `wl_shell`, which every LG client uses, needs
  `libxkbcommon` and `gbm` on the set, and its EGL display setup assumes a
  GBM device, which the 64-bit blob refuses on LG's render node (X2 r8b:
  `gbm_create_device` returned NULL and a no-config surfaceless context
  was used instead). The thin crates give the wire protocol, generated
  code for any XML, and both backends.

### 3. Process model

One 64-bit process, `lumatv-compositor`, `aarch64-unknown-linux-gnu`,
dynamically linked against glibc 2.36 and run through the private Debian
loader exactly like `x2srv64` (`ld-linux-aarch64.so.1 --library-path
<dir>`). It must be a glibc binary because it `dlopen`s the 64-bit Mali
blob (`libmali-bifrost-g52-g24p0-wayland-gbm.so`, needs `libc.so.6`,
`libstdc++.so.6`, `libwayland-server.so.0`, `libwayland-client.so.0`,
`libgcc_s.so.1`, `libdrm`). A static musl binary cannot load it.

Threads:

- Dispatch thread (main): the Wayland socket, the 60 Hz tick, the control
  socket. Owns the protocol state.
- Render thread (C3): takes a scene snapshot (plain values: window
  rectangles, buffer handles, damage) and draws; in C2 the draw runs on the
  dispatch thread right after the tick, behind the same `Output` trait, so
  the split is a move, not a redesign. PLAN notes the threaded Qt render
  loop blanks this set; that is Qt's loop, not ours, and the split is
  measured on the set before it is kept.
- Luna thread (C4): a Unix socket to the 32-bit luna bridge (§5).

Libraries on the set: the Wayland server runs on the C
`libwayland-server.so.0` (Debian 1.21, from the same private directory),
loaded at run time (`wayland-backend` `server_system` + `dlopen`), so the
blob's `eglBindWaylandDisplayWL` and our server share one `wl_display`;
the blob's `wl_*` symbols resolve to the same library. Host builds use the
pure-Rust backend: same API, no C library, and the tests run the real
protocol over socketpairs.

Sockets: `XDG_RUNTIME_DIR=/tmp/lumatv`, `WAYLAND_DISPLAY=wayland-lumatv`.
Nothing listens on the network (Principle 0): Wayland and the control
socket are Unix sockets, mode 0700 directory.

### 4. Protocol coverage

Versions LG's clients bind on the set are read with
`arch/wlproto_from_elf.py` from `libwayland-webos-client.so.1.0.0`,
`libgm-wayland.so.1.0.0`, `libflutter_elinux_wayland.so`,
`libwayland-starfish-client.so.1.0.0` and `libQt5WaylandClient.so.5.12.4`
the first time the TV is available (the first-light script does it). Until
then the public OSE XMLs are used; a client binds `min(server, client)`,
and a request a newer LG version added would arrive as an opcode the table
does not have and kill the client with a protocol error, which the log
makes obvious.

| Global | Version offered | Who binds (set) | C2 status |
|---|---|---|---|
| `wl_compositor` | 4 | all | full: surfaces, regions, double-buffered state, damage, frame callbacks |
| `wl_subcompositor` | 1 | Qt apps, LG Home | full: subsurface tree, sync/desync, place above/below |
| `wl_shm` | 1 | all | full: ARGB8888/XRGB8888 pools, mmap, composite |
| `wl_shell` | 1 | all | full: toplevel, `ping`/`pong`, `configure` at output size, title, class |
| `wl_seat` | 4 | all | keyboard and pointer objects, focus on map; keys and the Magic Remote's absolute pointer from evdev (`input.rs`), OK as `BTN_LEFT` while the cursor shows, the sprite composed above the scene |
| `wl_output` | 2 | LG Home, Qt, WAM | geometry 1920x1080, 1439x809 mm, 60 Hz, scale 1 |
| `wl_data_device_manager` | 1 | Qt, LG Home | objects only, no offers |
| `wl_webos_shell` | 2 | all | `get_shell_surface`, `set_property`, `set_state`, `set_location_hint`, `set_key_mask`, `set_addon` (answers `addon_status_changed(denied)`), events `state_changed`, `position_changed`, `close`, `exposed`, `state_about_to_change`; `get_system_pip` ignored |
| `wl_webos_surface_group_compositor` | 1 | Netflix, cobalt, LG Home, WAM, Qt | groups by name, layers with z and key index, attach/detach, anonymous layers, owner_destroyed; scene stacking honoured |
| `wl_webos_foreign` | 1 (exported/imported 2) | Netflix, cobalt, Prime, LG Home, WAM, Qt | `export_element` answers `window_id_assigned("_Window_Id_N", type)`; `set_exported_window`/`set_crop_region` keep rects; `attach_punchthrough[_with_context]` makes a transparent hole in the scene and emits a video placement request (§7). LG creates exported/imported at version 2 under the version-1 global (OSE `webosforeign.cpp`), so the copy of the XML drops the eight `since="2"` marks: with them, libwayland refuses the version-2 requests of a version-1 child ("invalid method 7 (since 1 < 2)", seen in the host test) |
| `wl_webos_input_manager` / `wl_webos_seat` | 1 | WAM, LG Home, Netflix, Qt | `get_webos_seat` answers `info`; `set_cursor_visibility` stored and echoed; gyroscope/accelerometer objects answer nothing |
| `wl_webos_xinput_extension` / `wl_webos_xinput` | 1 | WAM, LG Home, Qt | objects created; `activated`/`deactivated`/`invoke_action` logged |
| `text_model_factory`, `input_method`, `input_panel` | 1 / 2 / 1 | WAM, LG Home, Qt (`zwp_text_input_v2` not offered) | objects created and tracked; no IME routing (maliit is phase 6) |
| `wl_webos_system_ux` | 1 | system UI | requests logged |
| `zwp_linux_dmabuf_v1` | 3 | LumaTV's own 64-bit client | formats AR24/XR24/AB24/XB24 linear and ARM AFBC `0x0800000000000151`; import through EGL dma-buf import |
| `mali_buffer_sharing` | 5 | every GL client (through libmali) | `--buffers driver`: served by the blob after `eglBindWaylandDisplayWL` (proven X2 r8b); `--buffers own`: our global, `create_buffer(niiuuuuh)` → dma-buf import with the modifier, `alloc_device("/dev/dma_buf_te")` on bind, `auth` logged |
| `wl_starfish_pointer`, `wl_starfish_output` | 1 | WAM | XML written from LG's strings; **opcode order unverified**, so off by default (`--starfish`) until `wlproto_from_elf.py` confirms the tables |
| `wl_drm`, `zwp_linux_explicit_synchronization_v1`, `wp_presentation`, `qt_*`, `wl_webos_tablet` | – | – | not offered; explicit sync and presentation time are C3 (in-fence, presented timestamps) |

Window types honoured from `set_property("_WEBOS_WINDOW_TYPE", …)` (LG's
QML names them): CARD, OVERLAY, FLOATING, POPUP, RESTRICTED, TOPUI,
SYSTEM_UI, SCREENSAVER, HOME, KEYBOARD, SCREENCOVER, VOLUME, VOICEAGENT,
TOAST, SPLASH, SPINNER, PINCODEPROMPT, LONGPRESSSPINNER, INFINITESPINNER,
INDICATOR, COVERUI, BOOTPOPUP, ALERT, ADVANCEDALERT. Each maps to a
stacking layer (§6). Other properties kept per window: `appId`, `title`,
`subtitle`, `params`, `displayAffinity`, `_WEBOS_WINDOW_CLASS`,
`_WEBOS_ACCESS_POLICY_KEYS_BACK|EXIT|HOME|META|GUIDE`,
`_WEBOS_ACCESS_POLICY_TOASTS|POPUP|ALERTS|RIBBON|WINDOW`,
`_WEBOS_LAUNCH_PREV_APP_AFTER_CLOSING`,
`_WEBOS_LAUNCH_LAST_INPUT_APP_AFTER_CLOSING`, `_WEBOS_SUBSCRIBE_NUMERIC_KEY`,
`needFocus`, `keepAlive`, `playVideo`, `punchThroughRegion`,
`windowPosition`/`windowPosX`/`windowPosY`, `moveArea*`, `cursor_display`,
`cursor_fps`, `restore_cursor_position`, `handleFocusByExitKey`,
`delayCloseWindowTimeout`. Unknown names are stored too and listed by the
control socket so a TV session shows what a real app sends.

### 5. The luna service surface

A 64-bit process cannot load LG's 32-bit `libluna-service2.so.3`, and the
LS2 hub protocol is private, so luna lives in a separate 32-bit C program,
the **luna bridge**, linked against LG's own library (built with `zig cc
-target arm-linux-gnueabi.2.28` like the X2 server). The bridge registers
the bus names, forwards every call and subscription to the compositor over
a Unix socket as JSON lines, and sends the compositor's calls out. The
compositor's `luna` module is the contract (`compositor/crates/
lumatv-compositor/src/luna.rs`): `Inbound {token, service, method,
payload, subscribe, sender}`, `Reply {token, payload, final}`, `Outbound
{token, uri, payload, subscribe}`, `Event {token, payload}`. Every push
subscription in the table below is a `Reply` stream keyed by token.

Who is allowed to own `com.webos.surfacemanager`: ls-hubd matches a role
file by the executable path of the connecting process (`exeName`)
[INFERENCE from LS2's security model; check
`/usr/share/luna-service2/roles.d/*surfacemanager*` on the set]. In C4 the
bridge is therefore the file the early hook bind-mounts over
`/usr/bin/surface-manager` and the thing `surface-manager-daemon.service`
starts; it spawns the 64-bit compositor as its child (§8). In C2 the
bridge does not run: LG's compositor owns every name.

The 56 methods, by when they are needed (X4 pass 1-3 and the named callers
in `arch/sm-callers-01.txt`):

Tier 1, needed for the first SAM-launched app (C4 trial):

| Bus name / method | Who | Behaviour |
|---|---|---|
| `com.webos.surfacemanager/getForegroundAppInfo` (subscribe) | SAM, activitymanager, voiceconductor, ics | push `{mode, foregroundAppInfo:[{appId, windowType, processId, displayId, order, pip, multiview, primary, viewType, windowGroup, params}]}` on every change of the top card |
| `getFocusedWindowInfo` (subscribe) | multiviewcontroller, eim, browser | push `{focusedWindowInfo:[{appId, type}]}` |
| `getCurrentState`, `getForegroundWindowInfo` | alwaysready, voiceclick, ghp.runtime | window list `{windows:[{appId, windowType}]}` |
| `getForegroundVideoWindow` (subscribe) | screenmanager, YouTube, acr | `{windowId, appId, context, appWindow, displayOutput, fullScreen, isPig}` from the foreign table |
| `getLastInputType` (subscribe) | YouTube, Settings, input picker | `{lastInputType: "key"|"pointer"}` |
| `closeByAppId` | LG Home, services | sends `wl_webos_shell_surface.close` to that app's windows |
| `com.webos.surfacemanager.home/getCurrentHomeUIState` (subscribe) | livetv, hdmi2 | `{homeUIState:[…visibility flags…]}` |
| `com.webos.service.systemUIManager/launch`, `closeByAppId`, `getForegroundAppInfo` | key filters, Home, platformstarfish | system UI windows (quick settings, input picker) |
| `com.webos.service.recent/listRecents`, `removeRecent`, `removeAllRecents` | LG Home | recents list with snapshots; `eventType: removed` pushes |
| `getRecentsAppList`, `clearRecentsList` | SAM | same list |
| `com.webos.coverUI/getStatus`, `open`, `close` | SAM, bootd | boot cover; `getStatus` answered `{status: closed}` |
| `setPinOverlayView` | SAM | PIN prompt overlay; answered `returnValue: true` and the prompt view opened |
| `com.webos.surfacemanager.input/getKeyInputBlockState`, `getForegroundAppKeymask` | LG Qt apps | block state and the top app's `set_key_mask` |
| `getScreenVisible`, `setScreenVisible` | services | render on/off |
| `showInputPicker`, `showQuickSettings` | luna-send, key filters | open system UI |

Tier 2 (later phases): `captureCompositorOutput`, `closeLRUSurface`,
`showSpinner`, `showIndicator`, `getWindowGroupInfo`,
`getOverlayPlayVideoState`, `getForegroundAppState`, multi-view
(`changeMultiViewLayout`, `setMultiViewLayout`,
`getMultiViewLayoutAnimationState`), zoom (`getZoomStatus`, `setZoom`),
`alertChoice`, `screenSaver/restartUserActivityTimer`, `hcap/*`,
`extended/*TextureRenderingStatus`, `keyfilter.magicnumber/*`,
`input/blockKeyInput`, `cancelBlockKeyInput`, `registerAllowedKey`,
`systemui/closeAllSystemUI`, `sysuicompmgr/*`, `recent/requestScreenCapture`.

Calls the compositor makes (through the bridge): `applicationManager/
launchDefaultApp {category: home, params: {launchType: homeKey,
activateType: home}}` on the Home key, `applicationManager/closeByAppId
{reason: windowPolicy, id}`, `multiviewcontroller/setInputMode`, the
`videooutput`/`avmutemanager` placement calls (§7), and the subscriptions
it holds: `tvpower/power2/getPowerState`, `bootManager/getBootStatus`,
`applicationManager/running`, `settingsservice/getSystemSettings` (pointer
on/off), `homelaunchpoints/listLaunchPoints` (for LumaHome, already used
by the QML).

### 6. Scene, window types and key routing

The scene is an ordered list of layers, bottom to top, each a list of
mapped windows: `cards` (CARD, HOME, SCREENSAVER as a card), `overlays`
(OVERLAY, FLOATING, RESTRICTED), `system_ui` (SYSTEM_UI, TOPUI, VOLUME,
INDICATOR, SPINNER family, COVERUI), `popups` (POPUP, ALERT, ADVANCEDALERT,
PINCODEPROMPT, TOAST, BOOTPOPUP), `keyboard` (KEYBOARD, VOICEAGENT),
`cover` (SCREENCOVER, SPLASH). Surface groups attach a surface into a
named group's layer order instead of the type layer. Only one card is
visible at a time (LG's fullscreen container: the newest mapped card is
the foreground app, the previous one is hidden, not destroyed); the
foreground card is what `getForegroundAppInfo` reports.

Key routing (C4; the data model is in C2): keys come from the `lginput2`
evdev devices (`LGE RCU`, `LGE M-RCU`, `IoT keypad`, `Smart Remote RCU
Input`, …) read by the compositor. The focused window is the topmost
window with `needFocus` not false; a window's `set_key_mask` (the
`webos_key` bits: home, back, exit, navigation, numeric, colour, playback,
guide, info, …) says which keys it receives; keys outside the mask are the
compositor's (Home → `launchDefaultApp`, Back → the access policy
`_WEBOS_ACCESS_POLICY_KEYS_BACK`: close the window or pass it). Keys are
delivered as `wl_keyboard.key` with the evdev code; whether LG's clients
expect the raw code or code+8 with an xkb keymap is checked with the X3
tap (`wl_keyboard.keymap` format and the first key of a session). The
keymap served is a generated xkb file covering KEY_PREVIOUS (412, the
remote's Back), colour keys, channel and playback keys; `no_keymap` is a
switch. LG's hidden combinations and the hold-Home escape stay in the
compositor's key handler, never in a client.

### 7. Video placement through videooutputd

Apps never hand video to the compositor. The flow, from OSE
`webosforeign.cpp` and LG's `StarfishViewStateController.qml` (payloads
below are LG's own):

1. The app calls `wl_webos_foreign.export_element(surface, type)`; the
   compositor answers `window_id_assigned("_Window_Id_N", type)`.
2. The app hands the window id to its media pipeline; the pipeline (or
   the app) calls `wl_webos_foreign.import_element(window_id, type)` and
   `wl_webos_imported.attach_punchthrough_with_context(context)`; the
   compositor records context ↔ window and punches a transparent hole
   where the exported element is (`set_exported_window(source,
   destination)` gives the rect; `set_crop_region` the crop).
3. The compositor tells videooutputd where the hole is:
   `luna://com.webos.service.videooutput/display/setDisplayWindow
   {displayOutput: {x, y, width, height}, context, fullScreen: false
   [, appOutput: {…}]}`, or `setDisplayWindow {originalInput, sourceInput,
   displayOutput, context, fullScreen}` when a crop is set; properties
   with `display/setDisplayWindowProperty {context, <name>: <value>}`
   (`appRotation: Deg0`, `aspectRatio: original`, `fastMode: on`,
   `textureRotation: Deg0` were the four X4 saw per context); z-order with
   `display/setCompositing {composeOrder: {context, zOrder}}`; mute on
   hide with `luna://com.webos.service.avmutemanager/setAvMute {contextId,
   muteType: videoMute, muteOn}` after `avmutemanager/register
   {contextId}`.
4. The app itself calls `setDisplayWindow` with its own rect in normal
   play (YouTube did, X4); the compositor's call matters when the window
   moves or is hidden (`setDisplayWindow` 0x0 hides it).

The compositor never reads the video: decoded frames stay in the display
engine's secure windows under the OSD; the hole is cleared to transparent
black with blending off. `getForegroundVideoWindow` reports the table.

### 8. The tvpower handshake, watchdog and fallback

tvpower (`StarfishPowerService.qml`, LG's): on `com.webos.service.bus/
signal/registerServerStatus {serviceName: com.webos.service.tvpower}`
connected, the compositor subscribes with `{clientName:
"com.webos.surfacemanager", subscribe: true}` to
`power2/registerPreparePowerOff`, `power2/registerPreparePowerOn`,
`power2/registerPowerOffLogoRequest`, and `power2/getPowerState`. Each push
carries `timestamp` and `next sub state`; the answer is `power2/
responsePreparePowerOff|PowerOn|PowerOffLogoRequest {clientName,
timestamp, ack: true}`. Off: `next sub state` in {active standby, suspend,
always ready} → stop rendering, answer; {reboot, cold} → same plus
teardown. On: {active, always on display} → first frame, answer (LG uses
a 1 s safety timer that answers regardless; ours answers within 200 ms).
Off logo: draw the power-off logo, answer when it is on the panel. X4
measured 28 services answering within 160 ms and tvpower moving on 0.27 s
after asking; a registered client that never answers would stall standby,
an unregistered one is not waited for [INFERENCE from the protocol shape].

Watchdog and fallback (C4, designed now):

- The bridge (the bind-mounted `/usr/bin/surface-manager`) is the
  supervisor. It decides at start: LumaTV only if the firmware fingerprint
  matches the verified one (`/etc/starfish-release` and the sizes/md5 of
  `libmali.so.0.46.0`, `libwayland-webos-server`, `libWebOSCoreCompositor`,
  `surface-manager`), the previous run ended cleanly (a state file under
  `/var/lib/lumatv/compositor/`), and the boot counter of consecutive
  failures is under 2. Otherwise it unmounts the bind over
  `/usr/bin/surface-manager` and `execv`s LG's binary, so systemd's unit
  runs LG's compositor with LG's exe path and role.
- While LumaTV runs: a heartbeat on the control socket every 500 ms; two
  missed beats or a child exit → kill the child, unmount, exec LG's. The
  hold-Home escape (650 ms) does the same on request. LG's crash
  behaviour today (apps die with the compositor) is unchanged.
- The compositor itself: `panic = abort`, every protocol error from a
  client kills only that client, luna replies always within the dispatch
  tick, no allocation on the frame path once warm (LumaShow lesson 20).

### 9. Rendering and the C3 hooks

C2 renders to an offscreen target; C3 owns the planes. The seam is the
`Output` trait (`output.rs`): `size()`, `begin_frame() -> Target`,
`present(damage) -> Presented {at, seq}`, `capture() -> Rgba`. Two
implementations now: `Offscreen` (GL FBO on the blob; or a CPU buffer in
the software renderer used by the host tests) and nothing else. C3 adds
`Kms` (atomic commits on `card0` once LumaTV is the DRM master). The
renderer (`render/`) has `Software` (shm only, exact, host-tested) and
`Gl` (`egl.rs`: dlopen of the blob, no-config surfaceless context,
`EGL_WAYLAND_BUFFER_WL` and dma-buf import, textured quads into the FBO,
`glReadPixels`).

Hooks laid in C2 for C3, each a type or field already in the code path:

- Frame pacing: a `Tick` source drives `present`; now a 16.667 ms timerfd,
  later the KMS vblank timestamp feeding LumaShow's pacer (start late, aim
  early; `pacer.rs`, reused by path). Frame callbacks fire from the tick,
  not from commit, so a client paces to the output as X1 measured LG's
  does.
- Damage: every commit accumulates `wl_surface.damage[_buffer]` into the
  window and the output's damage list; the software renderer already
  repaints only damaged rectangles, the GL renderer repaints the union
  rect (C3: 16 px tiles and the merge pricing of LumaShow lesson 10).
- Direct scan-out: `Scene::sole_fullscreen()` returns the one opaque
  full-output window when nothing else is mapped above it; C3 puts its
  dma-buf on the overlay plane (`--buffers own` keeps the fd for
  `ADDFB2`; X5 is the test-only commit first). With the driver-served
  path the fd is not available, which is why both buffer paths exist.
- In-fence render-ahead: buffers carry an optional acquire fence
  (`zwp_linux_explicit_synchronization_v1`, in LG's libmali, not offered
  yet) and `present` takes an optional out-fence slot; the GL renderer's
  `Fences::kick` pattern (fence after draw, not `glFlush`: lesson 3) is
  checked on the G52 the first time GL is on the set.
- Release: `wl_buffer.release` goes out when the frame that read the
  buffer has been presented (or at once in the software path), never at
  commit.

### 10. First app

Launching an LG app by hand against the private socket means the app's
own luna traffic still goes to LG's services (SAM, settings), only its
Wayland goes to us; X4 pass 2 showed Netflix and PlxNative do no luna
with the compositor to bring up their first screen, so the window should
map. Candidates, from easiest: the X2 client (`x2cli`, LG's libmali,
proven), a native LG app binary with its appinfo `main` and the env
`XDG_RUNTIME_DIR=/tmp/lumatv WAYLAND_DISPLAY=wayland-lumatv`, LG Home's
`flutter-client` with its bundle argument. The script tries the client,
then the first native app whose appinfo names a binary, and records the
registry binds, the window properties and the first frame.

## Resume checklist for the TV run

1. Confirm idle (`getPowerState` Active, `getForegroundAppInfo`), mute the
   receiver if sound could play.
2. `compositor/tools/tv-first-light.sh deploy` copies `deploy/` (the
   binary, the Debian loader and libraries, the blob) to `/tmp/lumatv/`.
3. `… run` starts the server, the X2 client, both buffer paths, captures
   PNGs to `/tmp/lumatv/out/` and prints PASS/FAIL per frame.
4. `… app` launches the first LG app and captures.
5. `… clean` kills everything of ours; nothing of LG's was touched.

## What was built (2026-10-25)

`compositor/` (Cargo workspace, Rust 2024):

- `crates/wl-protocols`: server and client code generated by
  `wayland-scanner` from `protocol/webos/*.xml` (OSE, pinned above) and
  `protocol/private/{mali-buffer-sharing,starfish}.xml`.
- `crates/lumatv-compositor`: `server/` (one module per global: compositor,
  shm, shell, webos_shell, foreign, group, seat, output, input_manager,
  misc, dmabuf, mali, starfish, state), `surface.rs` (double-buffered
  state, subsurfaces, damage, frame callbacks, retired buffers),
  `window.rs` (LG's 24 window types and properties), `scene.rs` (layers,
  foreground card, groups, holes, `sole_fullscreen()`), `render/software.rs`
  (CPU, exact), `render/egl.rs` + `render/gl.rs` (the blob, offscreen FBO,
  `EGL_WAYLAND_BUFFER_WL` and dma-buf import, fence kick), `output.rs` (the
  C3 seam), `luna.rs` (bridge contract, LG's placement and power payloads),
  `control.rs` (status/windows/journal/capture/key/quit), `capture.rs`
  (PNG, the X2 colour check), `luna_methods.rs` (tier-1 methods, pushes,
  tvpower rounds), `main.rs` (epoll: Wayland, listener, 60 Hz timerfd,
  control), `bin/lumatv-testclient.rs` (shm X2 pattern).
- Host checks: `cargo test` in `compositor/` (71 unit incl. the vendored
  pacer tests, 18 protocol, 1 bridge). Tests cover the advertised globals, a card reaching the output pixel-exact, a small card over black,
  properties/type/key mask/state, foreground switching on close, overlay
  stacking at `windowPosX/Y`, release after replacement, damage after a
  4 px commit, group members above the owner, export → hole + placement
  calls, own `mali_buffer_sharing` create_buffer, synchronized subsurfaces,
  keyboard enter and key delivery, the luna tier-1 answers and pushes.
- Build for the set: `sh compositor/tools/build-tv.sh` → `compositor/deploy/`
  (ignored by git). Toolchain: `cargo zigbuild --target
  aarch64-unknown-linux-gnu.2.36`; host linker is zig's clang
  (`compositor/.cargo/config.toml`, the workstation has no `cc`).

## TV run (ready; not yet run)

`sh compositor/tools/tv-first-light.sh deploy` builds and copies the bundle
to `/tmp/lumatv` on the set (37 MB of libraries, the same files X2 r8b used
from `/tmp/m64`). Then `… run all` executes `/tmp/lumatv/run.sh` on the
set: four cases, each prints `PASS`/`FAIL` plus the server's summary line;
logs and PNGs land in `/tmp/lumatv/out/<case>/`; `… fetch` pulls them and
writes `docs/research/compositor/first-light-<date>.txt`. LG's compositor,
its socket, the screen and the picture mode are never touched; `… clean`
kills anything of ours.

Expected results:

| Case | What runs | Pass means |
|---|---|---|
| `first-light` (driver) | server `--driver <blob> --buffers driver --check x2 --frames 6`; `x2cli /usr/lib/libmali.so.0 --frames 6 --size 1920x1080` | client log lists `mali_buffer_sharing v5` among the globals and 6 `eglSwapBuffers ok`; server log has `egl: eglBindWaylandDisplayWL ok`, `window 1 mapped (Card … size Some((1920, 1080)))`, six `check x2 frame k … ok` lines with centres `0080ffff`, `1980e6ff`, `3280cdff`, `4b80b4ff`, `64809bff`, `7d8082ff`; summary `check ok 6 bad 0`; six `frame-000N.png` |
| `own` | same client; server `--buffers own` | server log: `client bound mali_buffer_sharing v5 (own)`, six `mali_buffer_sharing.create_buffer … 1920x1080 stride 7680 AB24 modifier 0x0800000000000151`, same six `ok` lines. A `FAIL` here with `import: eglCreateImageKHR(dma-buf …)` in the log means the blob will not import LG's AFBC layout through dma-buf import, and C3's direct scan-out needs another way to the fd (the X2 doc's open question) |
| `shm` | 64-bit `lumatv-testclient` (glibc, through the private loader) | `check ok 6 bad 0`; proves the loader, the C libwayland backend and the GL upload path without the 32-bit driver |
| `protocols` | `wlproto_from_elf.py` on LG's client libraries (needs python3 on the set; otherwise fetch the libraries and run it here) | `out/protocols.txt`: versions of every `wl_webos_*`, `text_model`, `input_method`, `wl_starfish_*` table. Compare with §4; anything newer than the OSE XML is a difference to fold into `protocol/`; the starfish opcode order unblocks `--starfish` |
| `app` | `/usr/bin/com.webos.exampleapp.systemui` then `/usr/bin/com.webos.app.tvhotkey` by hand with the socket env | `windows-<bin>.json` shows a window with its `_WEBOS_WINDOW_TYPE`, `appId`, size; `app-<bin>.png` is not all black. A client that connects and exits at once points at a missing global (its log says which) |

If `first-light` fails before any frame: `egl:` lines name the step
(dlopen, display, context, bind); `eglBindWaylandDisplayWL failed` means
the blob did not find the same `libwayland-server.so.0` as the compositor
(both must come from `/tmp/lumatv/lib`); a client stuck at `eglInitialize`
means no `mali_buffer_sharing` global was advertised (check the server's
`listening on` line for the buffer path).

## Protocol versions on the set (from `protocols`, 2026-10-25)

Read by `wlproto_from_elf.py` on the set's client and server libraries
(`docs/research/compositor/first-light-2026-10-25.txt` from line 251).
Compared with the OSE XML copies in `compositor/protocol/webos/` and
folded in where they differ (opcodes are positional, so LG's order is the
one that works):

- `text_model` v1 requests: LG has `set_enter_key_type` at 6 before
  `invoke_action`, and `set_max_text_length`/`set_platform_data` (9, 10)
  before `show_input_panel`/`hide_input_panel` (11, 12); OSE orders them
  differently. `text.xml` reordered. Events match.
- `input_method_context` v1 events: LG has `enter_key_type` at 3, OSE
  last. `input-method.xml` reordered. Requests match.
- `input_method` v2, `input_panel` v1, `input_panel_surface` v1,
  `text_model_factory` v1: match.
- The `wl_webos_*` and `wl_starfish_*` tables came out of the second run
  (line 413 on) and are folded in ("After the first gate");
  `protocol/lg-tables.txt` is the reference the tests check. Netflix maps
  `libwayland-webos-client.so.1.0.0` itself (with `libgm-wayland` and
  `liblsm-connector`, which link it), so its media connection uses these
  tables (probe 2026-10-25).
- Qt apps also want `qt_extended_surface`, `qt_surface_extension`,
  `qt_server_buffer`, `zqt_key_v1`, `zwp_text_input_v2`,
  `zxdg_output_v1` (tables in the log); none is offered and the example
  app ran without them.

## C3 status (offline, 2026-10-25)

Built, host-tested where pure, compiled for the set, not yet run on it:

- Tile damage and scan-out rings (`damage.rs`): 16 px tiles as a fixed
  bitmap (no allocation on the frame path), runs merged into at most 12
  rectangles, a 3-page ring where each page owes the union of every
  frame's damage since its turn (LumaShow lesson 10). Tests cover the
  tiling, the merge, the ring's account and the forget.
- Pacer: LumaShow's `pacer.rs` copied verbatim by `tools/sync-lumashow.sh`
  (`vendor/LUMASHOW_REV`), with its tests minus the quality-ladder ones
  (the ladder is LumaShow's layer model). The KMS output feeds it the
  page-flip timestamps and asks it when the next frame should start; the
  main loop re-arms the tick at that latch (start late, aim early) instead
  of the fixed 16.7 ms timer whenever the output paces itself.
- Allocation-free planning: `Scene::plan` writes into storage the scene
  keeps (items, holes, damage rectangles, the window stack, group members,
  surface trees); `planning_a_live_scene_allocates_nothing_once_warm`
  plans 200 frames of a card + group member + overlay with fresh damage
  under a counting allocator on the server thread and sees 0 allocations.
- Frame skip: a tick with empty damage renders nothing and (KMS) commits
  nothing; the planes keep what they show.
- KMS output (`kms/mod.rs`): DRM master on `card0`, universal planes and
  atomic caps, mode blob for 1920x1080@60, properties looked up by name,
  `IN_FORMATS` parsed; a ring of three dumb-buffer pages (XRGB8888,
  linear) imported into GL as render targets, each frame drawn only into
  the tiles its page owes; a native fence exported from EGL
  (`EGL_ANDROID_native_fence_sync`, present on the set's blob) as the
  plane's `IN_FENCE_FD`; non-blocking commits with page-flip events, one
  frame rendered ahead (the wait for the previous flip sits right before
  the next commit); direct scan-out of a sole full-screen opaque client
  dma-buf by prime import + `ADDFB2` with its modifier, falling back to
  composition and remembering refused modifiers (X5 in code: the plane
  lists AFBC `…62` and LG's clients produce `…151`; on the set the plane
  took `…151` all the same, WAM's Home in the 2026-10-25 trial); plane state
  cached so unchanged planes stay out of the request. `assign.rs` is the
  pure decision (tested).
- Rendering into dumb buffers through dma-buf import was measured slow in
  PLAN (8.5 ms per 1080p frame for three clears, against 1.3 ms in the
  driver's own memory). It is the path that proves KMS; the fast path is
  the blob's own window allocation (GBM on `card0` with the heap shim's
  dumb buffers, as the X1 client did on the Wayland platform) and is the
  first thing to add once the window run says GBM accepts `card0`.
- Not built: the overlay plane for a UI layer above a direct-scanned app
  (needs X6: whether osd1 scales like osd0), explicit sync for clients,
  `wp_presentation`, a render thread.

TV step (maintenance window, ~60 s of black screen, LG's compositor
stopped and restarted; SAM untouched): `sh compositor/tools/tv-first-light.sh
deploy && sh compositor/tools/tv-first-light.sh kms`. The script
(`tools/tv_kms.py` driving `tools/tv-kms-window.sh`) stops
`surface-manager-daemon`, runs two cases, restarts it and sweeps the
`librdx` report LG's inputcommon leaves (HANDOFF "Standby"). After the
run, check that the Denon still follows volume over eARC and is listed on
HDMI 2.

| Case | What runs | Pass means |
|---|---|---|
| A | KMS output + the 64-bit shm client, 600 frames | `a-server.log`: `kms: crtc 38 connector 31 mode 1920x1080@60 primary plane 32 … overlay 34`, no `atomic commit:` error, summary `drawn ≈ 600`, `flips` close to drawn; `panel-a.png` (LG's capture service, taken while the client holds `ff8000`) centre ≈ orange; `frame-NNNN.png` captures from the dumb page match |
| B | KMS output + `x2cli` (LG's libmali, AFBC) | either `direct: fb N for 1920x1080 AB24 modifier 0x800000000000151` (the plane took LG's layout: direct scan-out of LG apps is on) or `direct: ADDFB2 refused … composing instead` (expected from the plane's format list); `panel-b.png` shows the X2 colour either way |
| restore | `systemctl start surface-manager-daemon` | LG's Home back on the panel within ~15 s; `librdx` swept; `systemctl is-active` = active |

If the server fails at start with `DRM master on /dev/dri/card0: … (is
LG's compositor still running?)`, something else holds the master; the
script's `restore` runs on any exit. `kms-restore` restarts LG's compositor
by itself if the window was interrupted.

## C3 on the set (first window, 2026-10-25 02:37)

Two cases, logs in `/tmp/lumatv-tv-out/out/kms/` (workstation), run by the
main agent:

- `kms: crtc 38 connector 31 mode 1920x1080@60 primary plane 32 (6
  formats) overlay 34`; no `atomic commit:` error; `gl errors 0`; LG's
  capture service read the compositor's frame off the panel in both cases
  (centre `(25,128,230)` = the X2 pattern's frame 1, `1980e6`).
- A (shm client): 3 commits, 3 drawn, ticks 1074 in 30.6 s; the client saw
  callbacks after 41.0 and 22.6 ms and then none. B (x2cli, AFBC
  `0x0800000000000151` through `mali_buffer_sharing` own): 4 commits, 3
  drawn, render 414 µs (1.6 ms max) for a 1080p dma-buf quad; the shm
  upload in A cost 12-21 ms. `direct:` never appeared: no sole-fullscreen
  direct scan-out attempt was logged for the x2cli buffer (to read again
  in the re-run; the plane lists AFBC `…62`, not `…151`).
- Cause of the 3 frames: the pacer's clock (fixed above). Status at 7.5 s
  already showed 1073 ticks and no further tick until the client left.
- After `restore`, `webapp-mgr.service` was `failed` and the panel black
  until the main agent ran `systemctl reset-failed webapp-mgr; systemctl
  start webapp-mgr` and launched Home (fixed above).
- `/etc/ld.so.preload` names a 32-bit `/lib/libSegFault.so`; the 64-bit
  loader prints an `ELFCLASS32 … ignored` line in every 64-bit process.
  Harmless; the scripts' `error` greps skip it.

## C4 status (offline, 2026-10-25)

Built and host-tested where possible; the on-set steps are scripts with
expected results below.

- `compositor/sm/lumatv-sm.c`: the 32-bit supervisor and luna bridge
  (24 KB, glibc 2.28 soft-float, `DT_NEEDED libluna-service2.so.3`,
  `libglib-2.0.so.0`; built by `sm/build.sh` against stub libraries,
  bound to LG's real ones on the set). At start it decides (enabled file,
  firmware fingerprint over `/etc/starfish-release` and the GPU/Wayland/
  compositor/LS2 libraries, no recorded escape, fewer than two consecutive
  failures), otherwise unmounts itself from `/usr/bin/surface-manager`
  and execs LG's binary at that path (unit, role and exe name stay LG's).
  Then it opens the bridge socket, registers the 12 bus names and their
  56 methods at category `/`, spawns the 64-bit compositor through the
  private loader on `/tmp/xdg/wayland-0` with `--renderer kms`, and
  heartbeats it every 500 ms; a child exit or two missed beats is a
  failure (counted; two in a row keep LG's), a `fallback` message from the
  compositor (hold-Home) records an escape that keeps LG's until the tool
  re-enables. Ten minutes healthy resets the failure count. `--dry-run`
  registers the names with a `.lumatv` suffix, runs the compositor
  offscreen on the private socket and exits 75 instead of falling back,
  so the bridge is testable beside LG's compositor. The JSON-line scanner
  has host tests (`sm/build.sh` runs them).
- Compositor side: `ToBridge::Cancel` and `ToBridge::Fallback`; the
  control socket's `fallback [reason]`; `input.rs`: evdev readers for
  `/dev/input/event*` (`--input`), `EVIOCGNAME` to tell the network key
  device (`Smart Remote RCU Input`) from the remotes, key routing by the
  focused window's mask and `_WEBOS_ACCESS_POLICY_KEYS_BACK/HOME`
  (tested), Home → `launchDefaultApp`, Back on a window that does not
  take it → `close` + `closeByAppId {reason: windowPolicy}`, and the
  hold-Home escape (650 ms) → `Fallback`. The evdev key-code convention
  for `wl_keyboard.key` (raw code; the keymap maps code+8) is unverified
  on LG's clients.
- `tools/tv-cutover.sh`: `install` (bundle to
  `/var/lib/lumatv/compositor`, fingerprint recorded, failures 0, not
  enabled), `trial SECS` (flag, bind-mount `lumatv-sm` over
  `/usr/bin/surface-manager`, restart the unit, watch, keep the evidence,
  then flag removed and revert), `enable`, `disable`, `status`, `revert`.
  The boot-time re-mount and the tool's commands are `tools/lumatv`'s
  (`tools/compositor.sh`); the early hook mounts only while `enabled`
  exists.

TV steps, in order, each a separate slot:

1. Bridge dry run beside LG's compositor (no risk, ~1 min):
   `sh compositor/tools/tv-first-light.sh deploy && sh compositor/tools/tv-first-light.sh sm`.
   Expected: `sm.log` has `registered com.webos.surfacemanager.lumatv (27
   methods)` and the other 11 names, `compositor started`, `bridge:
   compositor connected`; `luna-send … com.webos.surfacemanager.lumatv/getForegroundAppInfo`
   answers `{"returnValue":true,"mode":"normal","foregroundAppInfo":[]}`
   before the client and an entry with `"appId":""`, `"windowType":"_WEBOS_WINDOW_TYPE_CARD"`
   while the X2 client is mapped; `getFocusedWindowInfo` with subscribe
   gets a first reply with `"subscribed":true`; the `fallback` command
   makes `lumatv-sm` log `fallback: dry-run-test` and exit 75 with
   `last-fallback` and `escaped` written. An `LSRegister` error names the
   hub's objection (role/permission) and is the thing to fix before 3.
2. The C3 maintenance window (above).
3. Trial cutover (~3 min, LG's compositor replaced then restored):
   `tv.py 'sh /tmp/lumatv/cutover.sh install && sh /tmp/lumatv/cutover.sh trial 120'`
   with the receiver muted and the set idle. Expected: `exe=/usr/bin/surface-manager`
   stays (the mount), `compositor=<pid>` present, `sm.log` shows the 12
   registrations and heartbeats, no `fallback:` line; LG's apps (WAM,
   SAM) reconnect to `/tmp/xdg/wayland-0` and their windows appear in
   `ctl windows`; after 120 s `revert` restores LG's Home. A `fallback:`
   line within the trial names the reason (the supervisor already execd
   LG's binary at that point) and the trial is over; `status` shows the
   state files.
4. The gate (`tools/tv_gate.py` + `tv-gate.sh`, `tv-first-light.sh gate
   [--enable-after]`): `cutover.sh enable`, then `live` (mounted, 12
   names, no fallback, WAM active, Home window mapped, panel not blank),
   `x1 60` (the X1 client as an overlay over Home: no gap over 20 ms,
   pacer `late`/`lost` unchanged), `netflix` (deep link, ENTER through
   `ctl remote 28`, card mapped, Picture Mode `dolbyHdrCinema` while
   playing and `filmMaker` after Home), `hdmi` (`com.webos.app.hdmi2` in
   front, then Home), `standby` (`power2/powerOff remoteKey`, the driver
   wakes the set by magic packet 25 s later, `woke` wants PreparePowerOff
   and PreparePowerOn acks in the journal, state Active, Home on the
   panel, still mounted), `escape` (Home held 1 s through the compositor's
   routing: the supervisor falls back, `escaped` written, LG's Home back
   through the recovery). The PIN prompt stays with Nitin (a lock is his
   setting). Result lines go to `docs/research/compositor/gate-<date>.txt`.

## C4 on the set (2026-10-25 06:02-06:26)

One exclusive slot, receiver muted, Netflix closed first. Files in
`docs/research/compositor/c4-2026-10-25/` (`trial-own`, `gate-own`,
`gate-driver`, `buffers-test`); gate lines in
`docs/research/compositor/gate-2026-10-25.txt`.

Unit facts read on the set: `surface-manager-daemon.service` is
`Type=notify`, `NotifyAccess=main`, `TimeoutStartUSec=1min 30s`,
`KillMode=control-group`; `/usr/bin/surface-manager.sh` ends in `exec
/usr/bin/surface-manager -platform ${WEBOS_COMPOSITOR_PLATFORM}` (the
running one has `-platform eglfs_webos`), so the supervisor is the unit's
main process and its `READY=1` counts. LG's socket is `0660
root:compositor`.

| Step | Result |
|---|---|
| `inspect` | hung for 6 min (fixed: regular files only, timeouts); the facts above read by hand |
| `cutover install` + `probe` | 12 allowed, 0 free, 0 refused; probe from `/usr/bin/surface-manager` |
| `trial 120` (own buffers) | live 120 s, no fallback, WAM active; Home (WAM) mapped and scanned out directly (`direct: fb 45 … AB24 modifier 0x800000000000151`); five `atomic commit: Invalid argument` between 11 and 25 s; 18 clients, 1 left at the end: `hdmi2` and LG's Flutter Home (back after the stub guard fired) died after binding `mali_buffer_sharing`; revert and recovery clean |
| gate (own) | live FAIL (no Home window, guard had removed the stub), x1 FAIL (59.94 fps, 8 gaps 20-33.5 ms, worst 28.3), netflix FAIL (no window), hdmi FAIL (no window), standby PASS, escape PASS (LG's back in 15 s) |
| buffers test (private socket beside LG's compositor) | `com.webos.app.tvhotkey`: exit 139 (segfault) 0.5 s after binding own `mali_buffer_sharing`; with the driver's it runs until the timeout |
| gate (driver) | live PASS (Home mapped, drawing), x1 FAIL (59.89 fps, 11 gaps 20-33.5 ms, 1 over, worst 58.3), netflix FAIL (card mapped, the error screen tvq-pb-101 drawn upside down; media connection dropped on `wl_webos_imported` opcode 7; Picture Mode stayed `filmMaker`), hdmi FAIL (card mapped, app manager still reports Home in front), standby PASS, escape PASS (14 s) |
| end | `homestub on` (stub, guard on, faults 0), LumaHome in front, `filmMaker`, tmpfs freed, flag absent, Active Standby |

Not run: the separate KMS window re-run; the gate's X1 measured the live
KMS pacing instead, and stopping LG's compositor again would have tripped
the stub guard a third time.

Next slot: see "Next slot" under "After the first gate" below.

## After the first gate (offline, 2026-10-25 06:40-07:30)

Host: `cargo test` (73 unit, 22 protocol, 1 bridge, 2 LG-table),
`sm/build.sh` (scanner and queue tests), clippy clean; `sh
compositor/tools/build-tv.sh` builds the bundle.

- Protocol tables. LG's `wl_webos_*` tables had been dumped in the second
  first-light run but never folded in. Six interfaces differed from OSE
  and are LG's now: `wl_webos_shell`/`_surface` version 1 with request 4
  `set_size(ii)` (no addon requests or event); `wl_webos_exported` and
  `imported` version 1, `imported` with eight requests,
  `attach_punchthrough(s context)` at 2 and `set_z_index(i)` at 7 (OSE's
  `attach_punchthrough_with_context` at 7 is what turned Netflix's
  `set_z_index` into a too-short string); `wl_webos_surface_group.destroy`
  is request 8, not 5 (OSE's order read `focus_owner` as a destroy);
  `wl_webos_input_manager` has `set_cursor_visibility` at 0;
  `wl_webos_xinput` has `invoke_action_with_window(uuuu)` at 1;
  `wl_starfish_pointer` starts with the two timer requests.
  `crates/wl-protocols/tests/lg_tables.rs` checks every generated table
  against `protocol/lg-tables.txt` (order, signatures, versions, no
  `since` marks on child objects); `build.rs` regenerates on XML changes.
- SAM's foreground. The supervisor registers the bus names before the
  compositor connects to the bridge and dropped every call that came in
  between, so a subscription made then was never answered; SAM's
  `getForegroundAppInfo`, made when `com.webos.surfacemanager` appears, is
  the likely casualty (SAM kept Home in front with Netflix and HDMI 2 up).
  Those calls are queued and sent on connect (`sm.log`: `bridge: N
  call(s) that arrived before the compositor sent on`, and a
  `subscription tN: service/method from sender` line for each). The pushes
  follow LG's `StarfishForegroundAppInfoMgr.qml` (read from the copy in
  `/data/home/lgtv/uiinv/qml`): the foreground card, then mapped overlay,
  floating and popup windows; no item without an appId; `multiview`,
  `pip`, `primary`, `viewType` and `order` on CARD and RESTRICTED only;
  surface-group owners and members flagged (`windowGroupOwner`,
  `windowGroupOwnerId`); `processId` a string; no `subscribed` flag on
  pushes. Every push is journalled (`luna: foreground [...] pushed to
  ...`); `ctl luna` lists the subscriptions.
- HDMI 2 stayed fullscreen but unmapped: `set_state` was confirmed only
  for mapped windows. Every `set_state` is answered at once now
  (`state_about_to_change`, `state_changed`), and a window that commits
  without a buffer is journalled once. Its `getCurrentHomeUIState`
  subscription had also been broken by the pretty-printed payload (fixed
  before the gate).
- X1 gaps. The per-second counts stayed at 60 while 11 gaps of 20-33.5 ms
  were counted, and the pacer added only 2 late and 3 lost refreshes:
  frames were not lost, callbacks came late and the next one early. They
  were sent at the tick, which runs at the pacer's latch, and the latch
  moves with the slowest of the last 8 frames (plus 6 ms of lead and
  slack), so a slow frame entering or leaving that window shifted every
  client's callback by the difference. Now frame callbacks go out when
  their frame's page flip lands; the loop never blocks on a flip (a frame
  drawn while one is pending waits and is committed from the flip event;
  it used to wait up to 100 ms in `poll`, holding every client); no extra
  frame is drawn for a vanishing surface while frames are paced (each 7 s
  crash of LG's Flutter Home did that in the own-buffers gate); the
  planes' recorded state follows only commits the kernel took. Log lines
  for the next run, on `CLOCK_MONOTONIC` like the X1 log: `loop: N ms in
  one pass (what)` above 8 ms and `frame gap N ms on surface S (app)` for a
  client that was keeping up; the gate's x1 step cuts them out for the
  run (`x1-compositor.txt`). The 58.3 ms gap was in the X1 client's first
  second (its buffer allocations); whether the rule skips a warm-up second
  is Nitin's call.
- The 7 s client. In the own-buffers gate it was LG's Flutter Home
  (`com.webos.app.home`, uid 0) crashing on the compositor's own
  `mali_buffer_sharing` and restarted every 7 s. In the driver-buffers gate
  a client came and went every 7 s without one request reaching the
  compositor's handlers; only touched clients got a pid, because the C
  backend never queues the connect event (no `client connected` line in
  either log). Clients are identified at accept now: `client connected:
  pid uid cmd` and `client disconnected: pid uid after N ms (reason);
  bound [...]; cmd`. Candidates from the read-only probe (processes with
  `libwayland-client` mapped under LG's compositor): the jailed preloaded
  apps (YouTube's `cobalt` uid 5272, Amazon's `ignition` uid 6367, Netflix
  uid 6579) and `inputcommon`; the trial names it.
- Volume and Mute. LG's key filters (`KeyFilters/systemUi.js`) turn them
  into audio service calls and no app sees them; nothing did that under
  LumaTV's compositor. The compositor's routing does it now with LG's own
  calls, read from its audio plugin's strings
  (`AudioService/libaudioserviceplugin.so`): `master/volumeUp` and
  `master/volumeDown` with `{}` on every press and repeat,
  `volumeKeeping` `{}` on release, `master/muteVolume {"mute": …}` on
  Mute's release, toggled from `volumeStatus.muteStatus` of a `getVolume`
  subscription. With the receiver on ARC the service passes the steps on
  over CEC. The gate checks it end to end (receiver level from Home
  Assistant after one Volume+ and one Volume-).
- Gate evidence: `gate.sh evidence` (SAM's subscriptions, clients with
  command lines, both logs, `ctl luna`), Netflix placements and protocol
  errors, HDMI 2 mapped or not; the trial keeps `luna.json` and SAM's own
  foreground; `tv_gate.py` copies the session's full logs after the
  disable.

The slot that ran next (11:24, "Second gate on the set"): `python3
compositor/tools/tv_next_slot.py` from the repository root. It wakes the
set, checks nothing of ours runs, turns the Home stub off for the session
if the installed `tools/lumatv` lacks the `comp_live` guard (and back on at
the end), deploys, installs, probes, runs `cutover trial 120` with the
receiver muted and checks its evidence (SAM's subscription, the 7 s client,
protocol errors, stalls), runs `tv_gate.py` (volume, x1, netflix, hdmi,
evidence, standby, escape; no `--enable-after`), and leaves LG's compositor
live and the set in Active Standby. Results:
`docs/research/compositor/slot-<date>-<HHMM>/summary.txt` with `trial/`
and `gate/`. Expected now: SAM subscribed; netflix placements and no
protocol errors (Dolby Vision follows if videooutputd takes the window);
hdmi mapped with `fg com.webos.app.hdmi2`; volume PASS; x1 either clean or
explained by its stall lines. `live` passes on the stub's window; the
family-visible Home under LumaTV's compositor is the stub's black page
until LumaHome is drawn there (below), so no `--enable-after`.

## Second gate on the set (2026-10-25 11:24-11:31)

One slot from `compositor/tools/tv_next_slot.py` at `bdfbb1e`, run from a
clean worktree so only committed code reached the set (the worktree needs
`LumaShow` beside it: the compositor's optional path dependencies are
resolved even when their features are off). Receiver muted by the script,
Home stub on with the `comp_live` guard (nothing turned off), preload list
as found. Files in `docs/research/compositor/slot-2026-10-25-1124/`
(`summary.txt`, `trial/`, `gate/`); gate lines in `gate-2026-10-25.txt`.
The script took 8 minutes end to end.

| Step | Result |
|---|---|
| `install` + `probe` | 12 allowed, 0 free, 0 refused |
| `trial 120` (driver buffers) | live 120 s, no fallback, WAM active, Home mapped; SAM's `getForegroundAppInfo` subscription reached the compositor (9 calls queued before it connected, sent on); 1 client disconnect in 120 s (WebAppMgr's first connection, 2.1 s, before the recovery restarted it); the 7 s client is gone; protocol errors 0; two loop passes over 8 ms at start (21.1 ms draw-now, 12.9 ms); SAM reports Home |
| live | PASS: mounted, 12 names, WAM active, Home (stub) mapped |
| volume | PASS: receiver 0.355 → 0.36 after Volume+ → 0.355 after Volume- |
| x1 | FAIL: 3590 frames in 60.01 s = 59.82 fps; gaps ≤17.5 ms 3527, 17.5-20 ms 46, 20-33.5 ms 16, over 33.5 ms 0; worst 28.1 ms. Compositor side: 11 gaps of a whole refresh in the X1 surface's commits (33.3-35.0 ms eight times, 26.8, 25.2, 20.8), one loop pass over 8 ms in the minute (8.6 ms, `flip`, at the last gap), no clients coming or going, no refused commits; pacer late 3 → 11, lost 342 → 2141 |
| netflix | PASS: card mapped, `_Window_Id_3` exported, punch-through attached, 3 placements (`setDisplayWindow` 1920x1080 from a 3840x2160 source), protocol errors 0, Picture Mode `dolbyHdrCinema` while playing and `filmMaker` after; one 33.4 ms gap in Netflix's own commits at start. The DISPLAY capture is black during playback (`netflix.png`, 6863 bytes): the video plane is not in that capture [INFERENCE] |
| hdmi | PASS: `com.webos.app.hdmi2` (inputcommon) mapped, its surface group, export and punch-through 1920x1080, SAM reports hdmi2 while open. After `launch com.webos.app.home` SAM still reported hdmi2 (below) |
| standby | PASS: PreparePowerOff and PreparePowerOn acked, state active, still mounted; the panel after the wake shows HDMI 2's "No Signal" page (`woke.png`), hdmi2 still in front |
| escape | PASS: Home held 650 ms, `escaped` written, LG's compositor back, recovery 15 s |
| end | LG's compositor live, flag absent, stub on (guard on, faults 0), `filmMaker`, `external_arc`, receiver unmuted at 0.355 (off with the set), tmpfs freed, Active Standby |

Two findings, both with their change:

- X1, the one failure. The third run in a row with 8-16 gaps a minute (own
  buffers 8, driver 11, now 16 on the client's clock and 11 whole-refresh
  gaps on the compositor's), so it is not the loop: one pass over 8 ms in
  the minute, no churn, no refused commits. LG's own compositor measured
  555 such gaps in 60 s on the same probe (`perf-tweaks.md`, "Compositor
  floor"); the rule says none. The pacer counted 8 flips landing one
  refresh after the one they were planned for: the commit went out too
  late for its vblank. The latch starts a frame `slowest pipeline of 8 +
  6 ms` before the refresh, and the pipeline it learns is CPU time only:
  `frame_ready_ns` is stamped when the draw is submitted (0.2-9 ms), the
  GPU finishes later, and the kernel waits on the in-fence before the
  flip. A composition the GPU finishes after the driver's cutoff slips a
  refresh, and nothing measures that. Five of the eleven gaps fall 10 s
  apart at the same phase (26350.82, 26370.85, 26380.83, 26390.82,
  26400.79): a periodic job on the set delaying the frame (the stub guard
  forks a grep every 10 s; LG's services) [INFERENCE]. Change, in
  `kms/mod.rs` (not `vendor/pacer.rs`, LumaShow's verbatim): `commit_frame`
  keeps a duplicate of the in-fence fd it exports; `flip_landed` reads
  the fence's signal time (`SYNC_IOC_FILE_INFO` on the sync file,
  `sync_fence_info.timestamp_ns`, CLOCK_MONOTONIC) into `frame_ready_ns`
  before building `Shown`, then closes it; the latch then includes the
  GPU and starts the frame early enough. Two more for the same step: the
  `lost` counter cannot be a gate criterion (it sums idle refreshes:
  +3957 over 62 s of a still screen between `evidence` and `woke`), so
  `tv-gate.sh x1` compares `late` only (`late_before=$(... $4)`); and it
  moved by 1799 over 3592 consecutive flips, 1.5 refreshes a flip, which
  vblank-stamped consecutive flips cannot give. Page-flip stamps arriving
  in equal pairs would [INFERENCE]; one `kms:` line per flip whose stamp
  delta is not one refresh, during the x1 step, settles it.
- Home's relaunch over a card does nothing. After the hdmi step's `launch
  com.webos.app.home`, Home's surface sent no request (journal: nothing
  between hdmi2's title and `PowerOffLogoRequest`), no foreground push
  went out, SAM kept reporting hdmi2, and the panel after the wake showed
  HDMI 2. The netflix step passed only because the card was closed first.
  LG's fullscreen container minimizes the card it covers and raises a card
  that asks for fullscreen again; this compositor's `on_mapped` makes the
  new card fullscreen and leaves the covered one in state fullscreen, so a
  client that restores fullscreen on relaunch (WAM, from a minimized
  window) has no state change to send [INFERENCE]. Change, in
  `server/webos_shell.rs`: `on_mapped` of a card sends
  `state_about_to_change`/`state_changed(minimized)` to every other mapped
  card and records it; `SetState(fullscreen)` on a card that is not the
  top card raises it (`scene`: make it the newest card), minimizes the one
  it covers, pushes the foreground and refocuses; a card unmapping gives
  the one below `state_changed(fullscreen)` back.

Both changes are built and host-tested (not run on the set), in the order
above: the KMS output keeps each frame's timing with the frame (the next
one is drawn while a flip is pending; `Shown` used to carry whichever frame
had been drawn last) and a duplicate of the in-fence with the flip;
`flip_landed` reads `SYNC_IOC_FILE_INFO` and takes the fence's
`timestamp_ns` as `ready_ns` when it is after the submit and before the
flip (otherwise the submit time stands, as before); a flip stamped other
than one refresh after the one before while frames follow one another is
counted (`odd stamps N` in the status line) and the first 200 are logged
as `kms: page-flip stamp X ms after the last`, which the gate's
`x1-compositor.txt` filter collects; `tv-gate.sh x1` compares `late` only.
The host cannot exercise the fence or a flip (no DRM): the cross-build
passes, and `fence_signal_ns` answers `None` for anything the kernel does
not confirm, which leaves the pacer as it was. The card states follow
LG's fullscreen container in `server/webos_shell.rs`: the newest mapped
card is fullscreen and every other mapped card is told it is minimized
(`settle_cards`, from `on_mapped`, from a card unmapping or its window
going, and from `set_state`); `set_state(fullscreen)` from a covered card
raises it (`Scene::raise`), minimizes the one it covers, pushes the
foreground and refocuses; `set_state(minimized)` from the front card
gives the one below back. Each transition is journalled (`window N:
minimized (covered by a newer card)`, `raised to the front`, `fullscreen
(the front card)`). Host test `tests/protocol.rs`,
`a_covered_card_is_minimized_and_comes_back_when_it_asks_for_fullscreen_or_the_card_over_it_goes`:
Home fullscreen; HDMI 2 maps, Home gets minimized; Home's
`set_state(fullscreen)` raises it (HDMI 2 minimized, Home's pixels on the
output, SAM's foreground Home); Home goes, HDMI 2 is fullscreen again.

Not run in that slot: `tv_ui_measure.py` (the slot was closed on the gate's
result); it ran in the third slot below.

## Third gate on the set (2026-10-25 12:19-12:27) and the UI engine's cost (12:29-12:33)

One slot from `compositor/tools/tv_next_slot.py` at `c91148d` (code of
`8f250c1`: the latch learns the GPU's finish, covered cards minimized), from
a clean worktree with `LumaShow` beside it, then `compositor/tools/
tv_ui_measure.py`. Receiver muted by the scripts, Home stub on with the
`comp_live` guard, preload list of the memory note's fix (Netflix, Home and
the HDMI input apps warm: the HDMI 2 app was preloaded, pid 2513, which the
second gate did not have). Files in
`docs/research/compositor/slot-2026-10-25-1219/` (`summary.txt`, `trial/`,
`gate/`) and `ui-measure-2026-10-25-1229/`; gate lines in
`gate-2026-10-25.txt`. The slot took 7 minutes, the measurement 3.5.

| Step | Result |
|---|---|
| `install` + `probe` | 12 allowed, 0 free, 0 refused |
| `trial 120` | live 123 s, no fallback, 2 clients (3 in all), SAM subscribed (9 calls queued before the compositor connected), 1 disconnect (WebAppMgr's first connection, 2.1 s), protocol errors 0, two loop passes over 8 ms, `late 0 lost 471 odd stamps 0`; windows Home and `com.webos.app.hdmi2` (the preloaded input app connected and created window 3 without a buffer, state Default) |
| live | PASS: mounted, 12 names, WAM active, Home (stub) mapped |
| volume | PASS: receiver 0.355 → 0.36 → 0.355 |
| x1 | FAIL by the rule, better by 10: 3596 frames in 60.02 s = 59.92 fps; gaps ≤17.5 ms 3551, 17.5-20 ms 38, 20-33.5 ms 5, over 33.5 ms 1 (43.5 ms, in the first second: a 13.7 ms loop pass `clients draw-now` and two page-flip stamps 33.36 ms apart); compositor side 2 frame gaps on the X1 surface (21.8 ms after 291 steady frames, 20.0 ms after 1971), 1 loop stall; pacer `late` 0 → 2 (second gate 3 → 11), `lost` 469 → 2132; 59-60 swaps in every second after the first. Session end: flips 4363, late 4, odd stamps 7, gl errors 0 |
| netflix | FAIL, in Netflix's player, not in the compositor: card mapped, Home minimized, foreground pushed, `_Window_Id_3` exported, punch-through attached, `setDisplayWindow` 1920x1080 from 3840x2160, then the pipeline unloaded (0.9 s), the same again on Netflix's retry (0.6 s); Picture Mode stayed `filmMaker`, `video` empty. The set's log (`messages.2.gz`): `VideoESPlayerNative::decoderTask::getNextMediaSample returns ERROR`, `PF_EVENT_TYPE_INT_ERROR 18`, `PlaybackDevice::errorReported code 2331 Playing Error`, pipeline state `Loading`, stream `video/x-h265 3840x2160 59.94, audio/x-eac3, Dolby Vision`. The same error class stopped the ambient slot's Netflix at 05:19 under LG's compositor (`ambient-result.md`, Audio results), so it is a Netflix-side failure [INFERENCE]; the 11:24 gate played the film. Netflix also restarted three times in the trial (crash reports count 1-3 at 12:19:49, 12:21:55, 12:22:13; `reason: preload`), which the second gate's trial did not show and which is open |
| hdmi | FAIL: `com.webos.app.hdmi2` window 1 (the preloaded instance's window 3 from the trial), mapped 0, bufferless commits 0, SAM's foreground stayed Home. After `launch com.webos.app.hdmi2` the app exported surface 6 as `_Window_Id_5` (type 2) and subscribed to `getForegroundAppInfo`, `getLastInputType` and `getForegroundWindowInfo`, but attached no buffer and asked for no state; in the trial it had subscribed to `getCurrentHomeUIState` about 30 times in one second, twice over (12:19:53, 12:22:17), a retry loop on that reply [INFERENCE]. The second gate's HDMI 2 launched fresh (not preloaded) and mapped. A preloaded (`snapshot`) card relaunched to the front is therefore the open case; the covered-card change was exercised only by Netflix (Home minimized at 12:23:45, fullscreen again at 12:24:22 when Netflix went) |
| standby | PASS: PreparePowerOff and PreparePowerOn acked, state active, still mounted, panel the stub's (6863 bytes) |
| escape | PASS: Home held 650 ms, `escaped` written, LG's compositor back, recovery 14 s |
| end | LG's compositor live, flag absent, stub on, `filmMaker`, `external_arc`, receiver unmuted, Active Standby |

`tv_ui_measure.py` (engine host, `LUMATV_FEATURES=ui`, LG's compositor
stopped for the window and restored by the script): idle10 GPU busy 0.49 %
over 28 s = 0.49 ms a frame at 10 fps, compositor CPU 3.3 %, **idle gate
PASS** (under 1 ms); anim60 2.35 % = 0.39 ms a frame at 60 fps, CPU 9.2 %;
volume 0.69 % = 0.69 ms a frame at 10 fps, CPU 4.1 %; direct (x2cli at
1920x1080 through LG's libmali, 1500 frames) GPU 47.34 % = 7.89 ms a
composed frame at 60 fps, CPU 9.5 %, `direct 0`, gl errors 0, five frame
gaps of 32-33.5 ms on the client's surface and one 209 ms loop pass
(`control-client`) at the end: the script's DIRECT check fails because no
frame went to the plane, which is the known rule of this driver (driver
buffers compose; only the compositor's own buffers scan out directly),
not a GL error. Engine figures: 259 frames in idle10 at 75,631 px a frame
(26,009 changed), 10 draws and 5 layers a frame, 651 us CPU a frame.

## After the third gate (offline, 2026-10-25)

Three changes, host-tested, none run on the set.

- The preloaded card. LG preloads HDMI 2 with `launch {launchHidden: true,
  preload: "partial"}` (X4 pass 1, 163 s): the app registers with SAM
  (`registerApp`, `getAppLifeStatus`), makes its window, commits it without
  a buffer and waits. On its relaunch it sent this compositor
  `set_state(fullscreen)` (window 3 went Default to Fullscreen between the
  netflix and hdmi snapshots), exported its surface and subscribed, and
  drew nothing in 10 s, while SAM's foreground stayed Home. The X4
  captures show what LG's compositor does with that request: preloaded
  YouTube relaunched is SAM's foreground 120 ms after the launch call
  (pass 1, 114.3 s), preloaded Netflix 640 ms after (pass 3), too soon for
  a first frame, and LG's `FullscreenView` takes `fullscreenRequested` (the
  client's `set_state(fullscreen)`) exactly as a mapping: the current item
  minimized, the item fullscreen and current, the foreground pushed, with
  or without a buffer. SAM then tells the app it is foreground, which is
  when a hidden app draws [INFERENCE: the app's trigger is not captured;
  the compositor's side now matches LG's either way]. `Window::front` is
  that order, set at a mapping and at a fullscreen request; `foreground`,
  `front_card` and `settle_cards` use it, a card's `set_state(fullscreen)`
  raises it mapped or not, and its first buffer keeps it in front. The
  covered card is minimized at once, so the output shows no card until the
  new one draws (LG hides the old item then too, behind its capture of it).
  The note's "`getCurrentHomeUIState` subscribed about 30 times a second"
  is not in the logs kept (one subscription line per compositor run); not
  chased. Host test `tests/protocol.rs`,
  `a_preloaded_card_relaunched_is_the_foreground_before_its_first_buffer`.
- The loop. `ctl capture` read the page back and encoded a 1080p PNG on
  the loop (the 209 ms `control-client` pass of the measurement's direct
  phase); control clients were blocking streams, so a client sending half
  a line held the loop. Now the loop reads the page back and a thread
  encodes and answers (`capture::Writer` for `--capture-every` too, one
  picture at a time), control clients are non-blocking with a line buffer,
  and a client that stops reading loses its reply instead of holding the
  loop. Host test `tests/control.rs` (the real binary: a silent client, a
  half line, a capture answered once the file is there, the loop ticking
  throughout). The 13.7 ms `clients draw-now` pass in X1's first second
  cannot be reproduced here (the Mali driver's first import and draw of a
  client's buffer are the suspects); the GL first-use costs are now paid at
  start (`Gl::warm_up`: a quad through the frame's program from an uploaded
  texture and from a dma-buf page, logged as `kms: gl warm-up N ms`), and a
  frame over 4 ms logs its new imports and their time (`render: N ms for
  ...`), so the next slot says what the pass was.
- Video placement, against LG's (X4 captures; the compositor's own calls
  were captured in pass 1 and their replies in passes 2 and 3; LG's QML
  `StarfishFullscreenContainer.qml` and `punchThrough.js`):

  | | LG's compositor | This compositor, before | Now |
  |---|---|---|---|
  | `setDisplayWindow` space | the panel's pixels: after a full-screen placement videooutputd holds `displayOutput` 3840x2160 (Netflix, YouTube); the Plex first screen leaves 134,196 3584x1964, twice its 1080p rectangle (`makeScreenRequest` scales by `screenRatio`) | the output's: 1920x1080 | `--panel WxH` (3840x2160), the rectangle scaled |
  | `fullScreen` | true for a full-screen video (videooutputd holds `fullScreen: true`); LG's multiview path sends it with every rectangle | always false | true when the element covers the output |
  | when | the real rectangle 0.5 s after the attach, once the pipeline's `setVideoData` is in; never an empty one at attach; 0x0 hides a video ("Hidden: call setDisplayWindow 0x0") | 0x0 at attach (the element had no rectangle yet), then the rectangle, then 0x0 again when the app emptied it | none while the element has no rectangle; once with the rectangle; once to hide when it is emptied, detached or destroyed |
  | `setDisplayWindowProperty` | appRotation Deg0, aspectRatio original, fastMode on, textureRotation Deg0 at attach | the same | the same |
  | video mute | `avmutemanager/register` and `setAvMute videoMute on` at attach, off once the pipeline plays (2.7 s later); whether the compositor or the app's `wl_webos_exported.set_property("mute")` drives it is not in the capture | the app's `set_property` is passed through (`mute`, `registerMute`), nothing of the compositor's own | the same, and every exported property is journalled so the next slot shows who drives it |

  Netflix's error (`getNextMediaSample returns ERROR`, 2331 at Loading) came
  200 ms after the 1920x1080, `fullScreen: false` placement; the same title
  played with that placement on the second gate, and the error class
  stopped Netflix once under LG's compositor (the ambient slot). The
  placement differences are fixed; the error's cause is not proven. The
  three Netflix restarts during the trial (`reason: preload`) happened
  while Netflix had no connection to the compositor (its preload makes no
  window), so they are not a protocol matter; open. Host tests
  `tests/protocol.rs`, `exported_video_region_becomes_a_hole_and_a_placement_call`
  (a partial window scaled) and
  `a_full_screen_video_is_placed_once_its_rectangle_is_known_in_the_panels_pixels`
  (Netflix's order).

## Fourth gate on the set (2026-10-25 14:18-14:27) and the engine measured again (14:30-14:34)

One slot from `compositor/tools/tv_next_slot.py` at `dd793ff` (code of
`221b75b`, `168f69a`, `8f69c05`: a card's fullscreen request makes it the
foreground, placements in the panel's pixels, the control socket off the
loop and the GL warm-up), from a clean worktree with `LumaShow` beside it,
then `compositor/tools/tv_ui_measure.py`. Receiver muted by the scripts,
Home stub on with the `comp_live` guard, the HDMI 2 app preloaded (pid 2513,
as in the third gate). Files in
`docs/research/compositor/slot-2026-10-25-1418/` (`summary.txt`, `trial/`,
`gate/`) and `ui-measure-2026-10-25-1430/`; gate lines in
`gate-2026-10-25.txt`. The slot took 9 minutes, the measurement 4. Three of
the four open checks moved: Netflix plays, HDMI 2 is SAM's foreground, x1
is down to 2 gaps; HDMI 2 still draws nothing, and the cause is now read.

| Step | Result |
|---|---|
| `install` + `probe` | 12 allowed, 0 free, 0 refused |
| `trial 120` | live 123 s, no fallback, 2 clients (3 in all), SAM subscribed (9 calls queued before the compositor connected), 1 disconnect (WebAppMgr's first connection, 2.2 s), protocol errors 0, two loop passes (14.8 ms `clients draw-now` at Home's mapping, 8.6 ms `tick`), `late 1 lost 450 odd stamps 0`, `kms: gl warm-up 9.2 ms`; windows Home and `com.webos.app.hdmi2` (window 3, bufferless, state Default, one `getCurrentHomeUIState` subscription, not thirty) |
| live | PASS: mounted, 12 names, WAM active, Home (stub) mapped |
| volume | PASS: receiver 0.355 → 0.36 → 0.355 |
| x1 | FAIL by the rule, better by 4: 3598 frames in 60.01 s = 59.95 fps; gaps ≤17.5 ms 3555, 17.5-20 ms 40, 20-33.5 ms 2, over 33.5 ms 0; worst 24.7 ms, in the first second (61 swaps, then 60 in every second); compositor side frame gaps 0 on the X1 surface, loop stalls 2: an 18.8 ms `tick` pass 40 ms after x1's mapping between two page-flip stamps 33.4 ms apart (the third gate's first-second shape, 13.7 ms then) with no `render:` line, so the time was not in the GL draw, and a 19.7 ms `clients tick flip` pass at x1's disconnect, after the run; pacer `late` 0 → 2, `lost` 494 → 2136. Session end: flips 3973, late 2, odd stamps 4 (two at x1's start, one at its end, one 1.5 s into Netflix), gl errors 0, render max 18.8 ms |
| netflix | PASS: Netflix (launched `reason: preload`) asked for fullscreen 165 ms after its window and 6 ms before its first buffer; raised, Home minimized, foreground pushed; `_Window_Id_3` exported, punch-through attached twice, `setDisplayWindow` 3840x2160 `fullScreen: true` once, 0.69 s after the first attach (LG's 0.5 s), 0x0 once at the end; Picture Mode `dolbyHdrCinema` while playing, `filmMaker` after; one frame gap of 23.1 ms on Netflix's surface after 36 steady frames; the same title that failed in the third gate (error 2331 at Loading) played |
| hdmi | FAIL on the buffer, PASS on the foreground: `com.webos.app.hdmi2` window 3 (the preloaded instance's) sent `set_state(fullscreen)` on the relaunch, was raised, Home minimized, SAM's foreground `com.webos.app.hdmi2` (the third gate's stayed Home); `extinput-integration` imported `_Window_Id_4`, attached its punch-through twice and the input was placed 3840x2160 `fullScreen` with `sourceInput` 1920x1080, twice (the second gate's fresh instance attached twice too); title `AVR-S760H`; no buffer in 10 s, mapped 0 |
| Home over HDMI 2 | `launch com.webos.app.home`: Home's `set_state(fullscreen)` raised window 2, HDMI 2 minimized, foreground Home, HDMI 2 cancelled its subscriptions; the input's placement was sent once more at full size and 0x0 9 ms later (the attach is journalled, the detach is not; the order says the app detached as it went background [INFERENCE]) |
| standby | PASS: PreparePowerOff and PreparePowerOn acked, state active, still mounted, panel the stub's (6863 bytes, a black page) |
| escape | PASS: Home held 650 ms, `escaped` written, LG's compositor back, recovery 14 s |
| end | LG's compositor live, flag absent, stub on, `filmMaker`, `external_arc`, receiver unmuted (off with the set afterwards), Active Standby |

What the three changes did on the set:

- The fullscreen request raises the card before its buffer: exercised by
  Netflix (foreground 6 ms before the first buffer) and HDMI 2 (SAM told
  the preloaded app it was foreground). `settle_cards` minimized the covered
  card each time and gave Home back when Netflix closed and when Home
  relaunched over HDMI 2.
- Placements: 3840x2160 with `fullScreen: true` for both, one call per
  attach, none before the element had its rectangle (no 0x0 at attach), 0x0
  once to hide. `ctl windows` lists the rectangle in the output's 1920x1080
  (the compositor's own units); the luna call carries the panel's. Video
  mute is the app's: Netflix sent `set_property("mute", "on")` 1 ms before
  its punch-through attach, when the export had no context, so nothing was
  relayed, and `mute off` right after the placement, relayed as
  `setAvMute videoMute off`; the compositor made no avmute call of its own
  and playback was fine.
- The loop: `kms: gl warm-up 9.3 ms` at start (9.2 in the trial). `render:`
  lines over 4 ms: 7.6 ms for 0 items when WebAppMgr's first connection
  went (the page redrawn empty), 18.8 ms for 1 item when x1 left (Home's
  first draw in 60 s), 4.0 ms for 1 item when Netflix closed; none in x1's
  first second, so that stall is outside the draw (the fence wait or the
  import; not yet attributed). The measurement's `control-client` passes
  went from 573-590 ms (third) to 117-136 ms: the PNG encode is off the
  loop, the page read-back (`glReadPixels` of 1920x1080) is still on it.

HDMI 2's missing buffer: this compositor never sends
`wl_webos_shell_surface.exposed`. The event is in `webos-shell.xml` and
nothing in `server/` emits it. LG's `libwebos-platform-interface.so.1`
carries `WebOSShellSurfacePrivate::exposed(void*, wl_webos_shell_surface*,
wl_array*)` next to `QWindowSystemInterface::handleExposeEvent`, and
`libQt5WaylandClient.so.5` the string "Didn't receive frame callback in
time, window should now be inexposed" (both read on the set, 14:30): LG's Qt
apps take the compositor's `exposed` as Qt's expose, and QtWayland unexposes
a window whose frame callback never comes. A Qt window that is not exposed
does not render. A freshly shown window exposes itself (the second gate's
HDMI 2 mapped in 160 ms); the preloaded instance made its window at the
enable, committed it without a buffer, got no frame callback and waits for
an expose that LG's compositor would send at the relaunch [INFERENCE on the
app's wait; the symbols and the missing sender are read]. Next: `exposed`
with the window's rectangle when a card comes to the front or maps, an empty
one when it is minimized. Also seen, both gates: `getWindowGroupInfo`
answered "unknown method" (the fresh instance mapped regardless).

`tv_ui_measure.py` (engine host, `LUMATV_FEATURES=ui`): idle10 GPU busy
0.51 % over 28 s = 0.51 ms a frame at 10 fps, compositor CPU 3.3 %, **idle
gate PASS**; anim60 2.42 % = 0.40 ms at 60 fps, CPU 9.1 %; volume 0.65 % =
0.65 ms at 10 fps, CPU 3.9 %; direct 48.72 % = 8.12 ms a composed frame at
60 fps, CPU 9.5 %, `direct 0`, gl errors 0, five frame gaps of 32-33 ms on
the client's surface: the DIRECT check fails by the known rule of this
driver. Within 0.02 ms of the third gate's figures. One odd page-flip stamp
in idle10 and in anim60.

## Surfaces under LumaTV's compositor (plan, 2026-10-25)

What goes away at the cutover is everything LG's compositor draws or runs
in its own QML: LumaHome (shadow module), the volume and mute OSD, toasts,
alerts and the PIN prompt (`SystemUI/Notification`), Quick Settings and
the input picker (system UI QML the compositor loads), splash and
spinners, the power-off logo (`MasterBrand`), the boot cover (`CoverUI`),
context menu and on-screen remote, and LG's key filters (`KeyFilters/*.js`:
Volume, Mute, gear, Input, app keys, Quick Access digits, the hidden
combinations). What keeps working is every app process: WAM's web apps
(Settings, the Home stub), Netflix, YouTube, Prime, `inputcommon` (HDMI
and Live TV banners), maliit's keyboard, the voice app; restyles made
through overlay mounts keep applying because they live in those processes.

Drawing. LumaShow's engine (session, scene, renderer: `rustui/render`)
runs inside `lumatv-compositor`; the UI logic stays in one 64-bit process
(`lumatv-ui`, today rustui's app) that speaks LumaShow's
`compositor-protocol` over a socket in `/tmp/xdg`. Under LG's compositor
the same app runs with the engine in its own process (rustui as built),
the PLAN's "runs unchanged inside LumaTV's own". In the compositor rather
than as a client window because QML cannot run in a Rust compositor and
LG's Qt is 32-bit, and because the RustUi spike measured what a client
window costs (`rustui-result.md`, as a client of LG's compositor): idle
GPU 23 % against QML's 10 % (6.2 ms of GPU a frame, 1.1 ms of it the copy
that turns the window over), 4.8-5.4 % of D-pad frames over 20 ms with
that copy and 0.4 % without it (QML: 0.4 %). In the compositor the scene is
drawn straight into the KMS page, which is laid out top row first like
LumaShow's targets since the C4 fix, so there is no copy, no window buffer
(three of 8 MB per full-screen window), no second composition pass and no
frame-callback round trip; an OSD over video appears without a window
being made and mapped; and the scene animates on the compositor's slots.

The spike's other finding is the prerequisite: a full redraw costs 5-6 ms
of GPU whatever changed, so before Home moves to Rust an idle 10 fps frame
must cost under 1 ms of GPU, which means the toolkit's renderer redraws
only what changed (not in it today), and Home must pass `gate.py` (animated
p90 under 3 ms, none over 20 ms) under LumaTV's compositor. In the
compositor that redraw is driven by the page ring: each page is owed the
union of the damage since it was last drawn (`damage.rs`, 16 px tiles), and
the engine draws only those rectangles, as the compositor already does for
windows. Until that gate passes, the interim for go-live is LumaTV's
existing QML in a standalone 32-bit Qt client on LG's own Qt 5.12 (the
shadow module's views, with the compositor-only imports replaced by the UI
channel below): a client window, so it pays one composition pass, and it
needs no rewrite; the same QML drops 0.4 % of D-pad frames inside LG's
compositor today, and how it does as a client is measured, not assumed.
Which of the two carries Home at go-live is decided by which passes the
frame gate under LumaTV's compositor first; the toolkit stays the
compositor's own renderer either way.

- One engine, two stacks, drawn into the page with the windows. Stack 0
  is Home, the bottom layer while the Home stub's transparent card is the
  foreground card (the stub keeps SAM's Home contract: `com.webos.app.home`
  is a real app SAM launches and sees in front). Stack 1 is the system
  layer above every window (OSD, toasts, alerts, PIN, quick panel, picker,
  splash). A page is drawn as stack 0, then the windows, then stack 1, each
  only in the rectangles the page owes.
- GL: the engine draws on the compositor's own context and thread (no
  texture hand-over, no fence); each pass sets the GL state it uses, and
  the toolkit's `Gles::enter`/`leave` save and put back the compositor's
  state round every call (`rustui/patches/lumatv-gl.patch`). Done; host
  tests `tests/ui.rs` (the ring) and `tests/ui_output.rs` (the live path).
- Direct scan-out stays with the UI attached (`kms/frames.rs`, decided
  2026-10-25): a full-screen opaque window goes to the plane whenever the
  plan allows it and nothing of stack 1 shows (stack 0 is under that
  window, so Home alone costs no composed frame); the frame a system
  layer appears is composed and drawn whole, since the pages missed the
  window's frames, and the frame it goes is direct again. Host test
  `tests/ui_scanout.rs`.
- Keys and state. The compositor stays the only reader of the remotes. A
  LumaTV UI channel (JSON lines, `/tmp/xdg/lumatv-ui-input`): to the UI,
  keys while a LumaTV layer holds focus (Home while the stub card is in
  front, the system layer while a panel is open), the foreground app and
  the cursor; from the UI, panels opening and closing (focus) and luna
  calls through the bridge from an allowlist (app launch and close, audio,
  settings reads).
- Key filters move into the compositor's routing, without drawing: Volume
  and Mute (done), gear (quick panel; three taps keep LG's AV-reset menu
  through `tvhotkey`), Input (picker), the app keys per Settings › Remote,
  Quick Access digits, Mute held 2 s (accessibility), Exit.

Minimum set before Nitin's go-live can apply (PLAN: every surface gate
met), in build order:

1. Compositor only: the key filters above (Volume and Mute done) and IME
   routing (`text_model` to `input_method`; maliit draws its own keyboard
   window). Without them the remote's volume stops working and no text
   field gets a keyboard.
2. The engine host, the UI channel and LumaHome on stack 0 with its
   Settings layer, once its damage-only redraw passes the frame gate (or
   the QML client in the interim).
3. Stack 1: the volume and mute OSD (receiver-fed through the helper),
   toasts and alerts including the never-filtered messages (EAS, thermal,
   remote battery, pairing prompts), the PIN prompt (its test needs Nitin).
4. Quick panel and input picker.

Not needed for go-live (absent means nothing is shown): launch splash and
spinners (apps draw their own first frame), the power-off logo (black; the
tvpower handshake is already answered), context menu, on-screen remote,
recents (Home draws them), multi-view frames (unused). They follow 4 in the
same engine. The gate runs again after 4.

Started: Volume and Mute (step 1). The engine host is built and host-tested
(feature `ui`: the toolkit's damage-only redraw, a `Gles` over the current
context, the two stacks drawn round the windows on the live tick path, the
receipts carried through the KMS flips; `docs/research/rustui-result.md`,
"Engine in the compositor"), not run on the set; the UI channel and Home
on stack 0 are not started. The other key filters wait on the remote's
codes for the gear, Input and app keys (an evdev tap on the set while
Nitin presses them, or LG's keymap, which the read-only probe now looks
for).

## Enable path and the hook request for tools/lumatv

Nitin's decision: LumaTV's compositor goes live as soon as it passes the
gate, LG's staying as the automatic fallback. What exists in
`compositor/tools` and what `tools/lumatv` (SurfacesP2b's) has to add.

In `compositor/tools` (`tv-cutover.sh`, on the set as `cutover.sh`):
`install` puts the bundle under `/var/lib/lumatv/compositor` (`bin/
lumatv-sm`, `bin/lumatv-compositor`, `bin/recover-home.sh`, `lib/`),
records the firmware fingerprint (`fingerprint`), LG's binary's size and
mtime (`lg-binary`) and `failures 0`, and leaves the set not enabled;
`enable` touches `enabled`, mounts `bin/lumatv-sm` over
`/usr/bin/surface-manager`, restarts the unit and runs the WebAppMgr and
Home recovery (WebAppMgr is `PartOf` the oneshot wrapper
`surface-manager.service`, not the daemon unit, so a restart of the daemon
leaves it to its start limit); `disable` removes `enabled`, unmounts,
restarts LG's and runs the recovery. The supervisor decides at every start
(enabled, fingerprint, no `escaped`, `failures < 2`) and falls back by
itself; the watchdog (heartbeat 500 ms, two misses or a child exit) and the
hold-Home escape fall back at run time; every fallback brings WebAppMgr and
Home back. Before the exec of LG's binary the supervisor unregisters every
bus name it holds and closes every descriptor above stderr: otherwise the
luna-service2 sockets would live on inside LG's process and ls-hubd would
refuse LG's own registration of the same names.

Requested in `tools/lumatv` and landed there as `tools/compositor.sh`
(458fc0b, SurfacesP2b: `lumatv compositor-boot` from the autostart
worker, `lumatv compositor on|off|status` implemented natively over the
same state files, the guards below, 17 host tests; not yet run on the
set, which needs `cutover.sh install` first):

1. Boot: in the early hook, before the restart of `surface-manager-daemon`
   that picks up the env file, place the mount when the state says so:
   `S=/var/lib/lumatv/compositor; [ -f $S/enabled ] && [ -x $S/bin/lumatv-sm ]
   && [ "$(stat -c '%s %Y' /usr/bin/surface-manager)" = "$(cat $S/lg-binary)" ]
   && ! grep -q ' /usr/bin/surface-manager ' /proc/self/mountinfo
   && mount --bind $S/bin/lumatv-sm /usr/bin/surface-manager`. The
   `lg-binary` check is the firmware guard the supervisor cannot do itself
   (it cannot see through the mount); a changed LG binary means no mount
   and LG's compositor, with a log line. Everything else (fingerprint of
   the libraries, escape, failure count) the supervisor decides.
2. Commands: `lumatv compositor on` = `cutover.sh enable` (needs a prior
   `install`, which stays a `compositor/tools` step until the bundle ships
   in the ipk), `lumatv compositor off` = `cutover.sh disable`, `lumatv
   compositor status` = the state files and `sm.log` tail. `revert`/
   `persist off` also unmount and remove `enabled`.
3. Guards: with the mount in place the compositor process is still
   `/usr/bin/surface-manager` by exe path, so "is the new compositor
   stock?" must not read `QML2_IMPORT_PATH` of that process: LumaTV's
   compositor is `pgrep -f bin/lumatv-compositor` alive and the mount
   present. The homestub guard must not hand LG's Home back in that
   state, and nothing restarts `surface-manager-daemon` while
   `/tmp/lumatv-recover.lock` exists (a recovery is in progress).
4. Home after a compositor restart under LumaTV: the same
   `relaunch_home` rule applies (WebAppMgr's new start stamp +
   `listRunningApps`, one launch, wait for pixels); `recover-home.sh` is
   that rule as a standalone script and may be replaced by a call into the
   tool once the tool owns it.

## What C3 and C4 need next

C3 (60 fps work, on the screen):

- The KMS `Output`: become DRM master of `card0` (only possible once
  LG's compositor is stopped, so this is the first C4-style maintenance
  window), atomic commits of the composed buffer on the primary plane with
  the GL fence as the in-fence, presented timestamps from the page-flip
  event into the pacer (LumaShow `pacer.rs` by path dependency).
- Render into scan-out buffers instead of the FBO: the heap shim (phase 1)
  gives the blob's window path contiguous dumb buffers; or `gbm` on the
  blob if it accepts `card0` as a GBM device (it refused the render node).
- Direct scan-out (X5): `Plan::sole_fullscreen()` + the dma-buf fd kept by
  `--buffers own` → `ADDFB2` with modifier `0x0800000000000151` (the plane
  lists `…62`; a test-only commit says whether the 16x16 layout scans out).
- Damage: tile the output damage (16 px, merge pricing), partial-update
  with `EGL_KHR_partial_update` on the blob.
- Explicit sync: offer `zwp_linux_explicit_synchronization_v1` (LG's
  libmali binds it); acquire fences replace the implicit wait.
- `wp_presentation` for flutter and LumaTV's own client.
- A render thread behind the `Output` seam, measured against the
  single-thread loop.

C4 (cutover, LG's compositor kept installed):

- The 32-bit luna bridge (§5) at the bind-mounted `/usr/bin/surface-manager`
  path with the watchdog and the fallback (§8); the tier-1 methods; the
  tvpower handshake (`luna::power`).
- Input: evdev readers for the `lginput2` devices, key masks and access
  policies, the Home key → `launchDefaultApp`, the hold-Home escape; the
  pointer (done, composed sprite; the cursor plane is the open optimisation);
  `getLastInputType`.
- Boot: the early hook's bind mount, the firmware fingerprint, the boot
  cover (`coverUI`), the hand-over from LG's boot logo.
- Verify on the set: `wl_keyboard` keymap and key code convention (X3
  tap), the starfish tables, LG's protocol versions, SAM's reaction to the
  first foreground push, standby and wake with the handshake.
