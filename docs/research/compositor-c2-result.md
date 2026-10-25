# Track C2: a 64-bit Rust compositor on a private socket

Design and first-light implementation of LumaTV's own compositor, built
beside LG's `surface-manager` (never replacing it in this step). Code lives
in `compositor/` (Cargo workspace; crates `lumatv-wl-protocols` and
`lumatv-compositor`). Everything here was done offline on the workstation;
TV runs are listed as ready scripts with their expected results. Facts
marked [INFERENCE] were not observed on the set.

## Resume here

State (2026-10-25 01:05 EDT): design written; the compositor builds on the
host (`cargo build` in `compositor/`); host tests and the aarch64 build are
next.

- Done: design (below), workspace `compositor/` (crates `wl-protocols`
  with the public webOS XMLs + `mali_buffer_sharing` + unverified
  `starfish`, and `lumatv-compositor`: wl_compositor/subcompositor/shm/
  shell/seat/output, webOS shell, surface groups, foreign punch-through
  with the videooutputd call shapes, input manager, xinput/text/input
  method/system UX stubs, linux-dmabuf v3, own mali_buffer_sharing, scene
  with LG's window layers, software renderer, GL renderer on the blob
  (EGL_WAYLAND_BUFFER_WL + dma-buf import, offscreen FBO, readback), 60 Hz
  timerfd tick, frame callbacks and releases from the tick, PNG capture,
  X2 self-check, control socket, luna bridge contract).
- Half-done: host integration tests (none yet); `cargo zigbuild` for
  aarch64 not yet run on this crate; `compositor/tools/tv-first-light.sh`
  not written.
- Next command: `cd /data/home/lumaglass/compositor && cargo test`, then
  `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36`.
- TV state: not touched by this slice.

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
| `wl_seat` | 4 | all | keyboard and pointer objects, focus on map; no evdev yet (C4) |
| `wl_output` | 2 | LG Home, Qt, WAM | geometry 1920x1080, 1439x809 mm, 60 Hz, scale 1 |
| `wl_data_device_manager` | 1 | Qt, LG Home | objects only, no offers |
| `wl_webos_shell` | 2 | all | `get_shell_surface`, `set_property`, `set_state`, `set_location_hint`, `set_key_mask`, `set_addon` (answers `addon_status_changed(denied)`), events `state_changed`, `position_changed`, `close`, `exposed`, `state_about_to_change`; `get_system_pip` ignored |
| `wl_webos_surface_group_compositor` | 1 | Netflix, cobalt, LG Home, WAM, Qt | groups by name, layers with z and key index, attach/detach, anonymous layers, owner_destroyed; scene stacking honoured |
| `wl_webos_foreign` | 1 (exported/imported 2) | Netflix, cobalt, Prime, LG Home, WAM, Qt | `export_element` answers `window_id_assigned("_Window_Id_N", type)`; `set_exported_window`/`set_crop_region` keep rects; `attach_punchthrough[_with_context]` makes a transparent hole in the scene and emits a video placement request (§7) |
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
