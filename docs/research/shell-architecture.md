# LumaTV shell architecture: how much of LG's stack to replace

Evidence-based study (2026-10-23) of replacing LG's stack layer by layer for
performance, stability and full use of the C4's hardware. Reference set:
LG C4 OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, chip O22A3. All TV readings
in this document are read-only (`ps`, `/proc`, `/sys`, strings of binaries,
DRM read-only ioctls); nothing here changed the screen, power state or picture
mode. Probe sources and raw logs: `docs/research/arch/`.

## Resume here

- Done: file created; TV reachable (`192.168.1.242`, root, Homebrew default
  password, see `/data/home/lgtv/RESUME-NOTES.md`); process table
  (`arch/ps-01.txt`); DRM inventory (`arch/drmplanes.c` built with
  `zig cc -target aarch64-linux-musl -static`, output `arch/drmplanes-01.txt`,
  debugfs state `arch/drm-debugfs-01.txt`); 30 s per-process cost
  (`arch/proccost-30s.txt`, raw `arch/procsnap-a/b.txt`).
- Next: Wayland globals bound by Netflix / WAM / Flutter Home (strings of
  their libraries, not the tiny launcher binaries: `netflix/lib/*`,
  `/usr/lib/libcbe.so`, `/usr/lib/libflutter_engine.so`, `libwayland-webos-client`),
  luna-surfacemanager method usage (strings of SAM/WAM + OSE source),
  LumaShow compositor sizing, OSE source sizing, options and ranking.
- TV state left: untouched. Power state `Active`, picture mode `filmMaker`
  when read at 19:10. No tweak applied by this study. Nitin is using the
  set as of 19:20 (Main): only light read-only SSH until Main says it is
  free; no `ls-monitor`, no binary-wide greps.
- Continue with: `python3 arch/tv.py '<cmd>'` (paramiko helper), or the
  `tv()` function in the eval kernel.

## Findings log (chronological, raw numbers)

### DRM/KMS inventory (read-only, 19:13)

`arch/drmplanes` on `/dev/dri/card0`, driver `lg1k 1.0.0 (20210215) "LG
Graphics Output Driver"`, atomic and universal planes accepted, PRIME
import+export (cap 3), `ADDFB2_MODIFIERS` yes, `ASYNC_PAGE_FLIP` no,
cursor 128x128, no syncobj, one CRTC (38), one encoder (39), one connector
(31, type TV, connected, modes 1920x1080@60 / 3840x2160@60 / 1280x720@60,
all pixel clock 148.5 MHz: these are OSD canvas sizes, not panel timings).

Planes, all on the single CRTC, none with `zpos`, `alpha`, `pixel blend
mode` or `COLOR_ENCODING` properties exposed (debugfs shows the defaults
only):

| Plane | Type | Formats | Modifiers | State now |
|---|---|---|---|---|
| 32 | primary | AR24 XR24 AB24 XB24 | linear; AFBC `0x0800000000000062` (32x8 split sparse) for AB24/XB24 | fb 40, 1920x1080 AB24 AFBC, full CRTC |
| 34 | overlay | AR24 XR24 AB24 XB24 | same | **unused: no CRTC, no fb** |
| 36 | cursor | AR24 | none | **unused** (`hwcursor: false` in `/tmp/xdg/surface-manager.eglfs.json`) |

`surface-manager` (pid 7581) is the only DRM client and the master. It owns
three framebuffers, all AFBC AB24 and all `imported=yes` (dma-buf from the
Mali driver, not dumb buffers): 40 and 43 are 1920x1080 (double-buffered
`fb0`), 44 is 768x2160 (`fb1`, the second eglfs output), allocated but not
attached to any plane at the time of reading. `plane_mask=1`: the CRTC
scans out the primary plane only. The video path is not in DRM at all
(no YUV formats, no video planes): video runs through LG's `vsc`/`libvt`
mixer underneath the OSD, as expected.

Underneath DRM, LG's own fbdev driver (`/proc/lg/fbdev`, author
raxis.lim) exposes the same hardware as four OSD framebuffers: `fb0`
(osd0, 1920x1080 AFBC 32x8, double-buffered, 173 835 flips since boot),
`fb1` (osd1, 768x2160 AFBC, max 512x2160 by config, 1 flip since boot:
allocated and never used), `fb2` (osd2, 128x4, state 2), `fb3` (hardware
cursor, 256x256 raw ARGB, 0 flips). `/proc/lg/fbdev/status` shows the
cursor path scaling a 1920x1080 source region to a 3840x2160 destination:
the OSD is composed at 1080p and scaled by the display engine. The GFX
block (`/proc/lg/gfx`) is a 2D blitter with its own 79 MB region (33 MB
allocated, 3 surfaces: two 256x256 cursor images and one 384x216
thumbnail). The display engine (`/proc/lg/de/status`, firmware 7356 of
2025-04-14) holds the video path's memory in protected regions:
`g_de_svp_m1` 54 MB + `g_de_svp_m2` 83 MB secure video, `g_be_frc0` 33 MB
frame-rate conversion, `g_be_fpp` 19 MB, `g_fw_dnn` 8 MB (the picture
engine's own neural net), 2 video windows (`w0`, `w1`: PIP/PBP capable,
sources avd/adc/hdmi/vdec/jpeg, `hdr` per window sdr/h10/dovi/hlg).
Device holders (`arch/devholders-01.txt`): only `surface-manager` holds
`/dev/dri/card0`; `pqcontroller`, `eplmanager`, `captureservice`,
`gal_renderd` and `nn-proxy` hold `/dev/fb0`/`fb1`/`fb3` (reads: histogram,
panel care, capture); GPU clients (`/dev/mali0`) are surface-manager,
WebAppMgr, flutter-client, cobalt, inputcommon, gal_renderd, nn-proxy.

### Hardware (read-only, 19:22)

- CPU: 4x Cortex-A76 (part 0xd0b) online 0-3, cpufreq policy0
  `performance` pinned at 1 400 000 kHz (the only frequency listed);
  `/proc/lg/pm` reports `cpu_num: 3`, load 73-98 %, 52 °C. Features include
  `asimddp`, `fphp`, `atomics`.
- GPU: Mali-G52 3 cores r1p0 (`/sys/class/misc/mali0/device/gpuinfo`),
  power policy `coarse_demand`, no devfreq node (no DVFS visible from
  userspace: the clock is LG's), all three cores in every job-slot mask.
- Memory: 2 030 392 kB total, 81 MB free, 778 MB available, CMA 692 MB
  total / 49.8 MB free / 258 MB device-allocated.
- NPU: `/dev/npu0` present; `g_fw_dnn` 8 MB is the picture engine's own
  net, separate from the application NPU (PLAN.md).

### Process cost, 35 s window, TV active on Home with Nitin using it (19:17)

`arch/proccost-30s.txt`. System 69.9 % busy of 4 cores over the window,
but 89.8 % of one core was a sibling agent's `grep` and 10 % `kswapd0`
(memory pressure from that grep), so the figures below are an upper bound
for an idle-ish Home:

| Process | CPU (% of one core) | Wakeups/s | RSS | Threads |
|---|---|---|---|---|
| surface-manager | 13.4 | 170 | 100.5 MB | 21 |
| pqcontroller | 8.6 | 0 | 4.4 MB | 37 |
| lginput2 | 3.6 | 0 | 6.1 MB | 13 |
| iconnectivity | 3.2 | 0 | 25 MB | 56 |
| systemd | 2.6 | 72 | 8.5 MB | 1 |
| servicemanager | 2.3 | 61 | 3.2 MB | 1 |
| memchute | 2.2 | 26 | 2.4 MB | 6 |
| ls-hubd | 1.8 | 45 | 7.7 MB | 5 |
| sdx | 1.5 | 11 | 13.6 MB | 12 |
| legacy-broadcast-atsc | 1.0 | 0 | 11.4 MB | 86 |
| ss.apiadapter | 0.8 | 13 | 33 MB | 13 |
| WebAppMgr | 0.2 | 7 | 11 MB | 35 |
| sam | 0.2 | 5 | 5.7 MB | 4 |
| flutter-client (LG Home) | 0.0 | 0 | 35.5 MB | 18 |

surface-manager's thread split since boot (`arch/sm-libs-threads.txt`):
main thread 62 790 ticks, `tCursor` 18 064 (29 % of the main thread's
cost: LG's pointer thread), `mali-cmar-backe` 3 757, `QQuickPixmapReader`
580, `tInput` 396, `FM-watchdog` 432. The compositor runs with
`WEBOS_GL_DISABLE_THREADED_RENDERING=1`: QML, the scene graph and GL all on
one thread. Its mapped libraries: `libmali.so.0.46.0` 52.9 MB,
`libicudata` 29 MB, Qt5 Core/Gui/Qml/Quick 14.6 MB, `libQt5WaylandCompositor`
0.9 MB, `libWebOSCoreCompositor` 0.65 MB, `libvt` 87 kB, `libStarfishInput`
204 kB, `liblinux-dmabuf-unstable-v1.so` 42 kB, `libwayland-server` 51 kB.

Preloaded apps while idle (`arch/ps-01.txt`): netflix 72 MB (13 % CPU
while preloading), cobalt (YouTube) 87 MB, ignition (Prime) 56 MB,
com.webos.app.voice 48 MB, chrome (browser) 43 MB, amazon-alexa-adapter
41 MB, flutter-client (LG Home) 35 MB, ss.apiadapter 35 MB, node (tvweb)
31 MB, iconnectivity 25 MB, pbs 23 MB, hbchannel service 19 MB.

### Wayland interfaces the clients bind (strings of the mapped libraries, 19:20)

`arch/wayland-globals-libs.txt`, `arch/app-libs-01.txt`. Every app goes
through one of three client stacks, and all of them are LG's shared
libraries, not the app's own code:

| Client | Stack | Interfaces bound |
|---|---|---|
| Netflix (`libwebosPlugin.so.1` + `libgm-wayland`) | own EGL window | `wl_compositor`, `wl_shell`/`wl_shell_surface`, `wl_seat` (keyboard, pointer), `wl_shm`, `wl_region`, `wl_webos_shell`/`_surface`, `wl_webos_foreign`/`exported`/`imported`, `wl_webos_input_manager`, `wl_webos_surface_group(_compositor)` |
| YouTube (cobalt) + Prime (`libgm-wayland`, `libwayland-input-helper`) | own EGL window | `wl_compositor`, `wl_shell`, `wl_seat`, `wl_region`, `wl_webos_shell`, `wl_webos_foreign`, `wl_webos_surface_group(_compositor)` |
| WAM (`libcbe.so` 150 MB, `libwayland-webos-client`, `libwayland-starfish-client`) | Chromium Ozone | the above plus `text_model_factory`/`input_method`/`input_panel`, `wl_webos_seat`, `wl_webos_xinput(_extension)`, `wl_webos_accelerometer`/`gyroscope`, `wl_starfish_output`, `wl_starfish_pointer` |
| Flutter Home (`libflutter_elinux_wayland.so` 1.2 MB) | flutter-elinux | `wl_compositor`, `wl_shell`, `wl_seat` (keyboard, pointer, touch), `wl_shm(_pool)`, `wl_output`, `wl_data_device(_manager)`, `wp_presentation(_feedback)`, `text_model_factory`, `input_panel`, `wl_webos_shell`, `wl_webos_foreign`/`exported`, `wl_webos_input_manager`, `wl_webos_surface_group(_compositor,_layer)`, `wl_webos_xinput(_extension)` |
| Qt apps (inputcommon, LG QML apps; `libQt5WaylandClient 5.12.4`) | Qt wayland | `wl_shell`, `wl_subcompositor`/`wl_subsurface`, `wl_shm`, `wl_data_*`, `qt_hardware_integration`, `qt_surface_extension`, `qt_windowmanager`, `qt_server_buffer`, `qt_touch_extension`, `zwp_text_input_v2`, plus `wl_webos_shell` through `libwebos.so` shell integration |
| GPU buffers (`libmali.so.0.46.0`, every GL client) | Mali EGL | `wl_drm`, `mali_buffer_sharing`, `zwp_linux_explicit_synchronization_v1`/`zwp_linux_surface_synchronization_v1`/`zwp_linux_buffer_release_v1` (the server side offers `zwp_linux_dmabuf_v1` v3; the 64-bit probe used it) |

Video is not a Wayland buffer anywhere: Netflix, cobalt, Prime and WAM all
map `libumedia_api`, `libums_connector`, `libwaylandpipeline-1.0`
(161 kB) and `libvt` (87 kB, YouTube and WAM): the media pipeline
(`starfish-media-pipeline`, uMediaServer) decodes to the display engine's
video windows, and the compositor only places the hole and the window
rectangle (`luna://com.webos.service.videooutput/display/setDisplayWindow`,
called from `StarfishFullscreenContainer.qml` and
`ViewStateController.qml`; `punchThrough.js` draws the transparent region).

Protocol size (webOS OSE `webos-wayland-extensions`, `arch/` clone in
`/tmp/ose`): 9 XML files, 84 requests and 49 events in all:
`webos-shell` 8/6, `webos-foreign` 16/6, `webos-surface-group` 14/3,
`webos-input-manager` 6/4, `webos-extension` (xinput) 4/2, `input-method`
15/12, `text` 16/12, `webos-system-ux` 5/3, `webos-tablet` 0/1.
`wl_starfish_output`, `wl_starfish_pointer`, `mali_buffer_sharing` and
`wl_drm` are not in the public XML: their protocol would have to be
recovered from `libwayland-starfish-client` (9.6 kB) and `libmali`.

### The luna API of `com.webos.surfacemanager` (uiinv QML, 19:25)

From `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/services/*.qml`
(LG's shipped QML, webOS 10.2.1). The compositor registers these
services and methods:

| Service name | Methods |
|---|---|
| `com.webos.surfacemanager` | `closeByAppId`, `getForegroundAppInfo`, `captureCompositorOutput`, `clearRecentsList`, `closeLRUSurface`, `getCurrentState`, `getFocusedWindowInfo`, `getRecentsAppList`, `showInputPicker`, `showQuickSettings`, `showSpinner`, `showIndicator`, `getForegroundVideoWindow`, `getWindowGroupInfo`, `getForegroundWindowInfo`, `getScreenVisible`, `setScreenVisible`, `getOverlayPlayVideoState`, `getForegroundAppState`, `changeMultiViewLayout`, `setMultiViewLayout`, `getLastInputType`, `getMultiViewLayoutAnimationState`, `setPinOverlayView`, `getZoomStatus`, `setZoom`, `alertChoice` (27) |
| `com.webos.service.recent` | `listRecents`, `removeAllRecents`, `removeRecent`, `requestScreenCapture` |
| `com.webos.coverUI` | `open`, `close`, `getStatus` |
| `com.webos.surfacemanager.screenSaver` | `restartUserActivityTimer` |
| `com.webos.surfacemanager.hcap` | `getHcapOpacity`, `setHcapOpacity` |
| `com.webos.service.systemUIManager` | `launch`, `closeByAppId`, `getForegroundAppInfo` |
| `com.webos.surfacemanager.systemui` | `closeAllSystemUI` |
| `com.webos.surfacemanager.home` | `getCurrentHomeUIState` |
| `com.webos.surfacemanager.extended` | `getTextureRenderingStatus`, `setTextureRenderingStatus` |
| `com.webos.keyfilter.magicnumber` | `notifyChannelToRegister` |
| `com.webos.surfacemanager.input` | `blockKeyInput`, `cancelBlockKeyInput`, `getKeyInputBlockState`, `getForegroundAppKeymask`, `registerAllowedKey` |
| `com.webos.service.sysuicompmgr` | `changeSystemUIView`, `changeSystemUIViewStateTransition`, `launch`, `close` |

56 methods across 12 bus names. What calls them (strings of the callers,
`arch/sm-callers-01.txt`): `sam` (2.5 MB) names only
`com.webos.surfacemanager/getForegroundAppInfo`, `/getRecentsAppList`,
`/setPinOverlayView` and `com.webos.coverUI/getStatus`; LG's Flutter Home
(`libapp.so`) names `com.webos.service.systemUIManager/launch`,
`/closeByAppId` and `com.webos.service.recent/listRecents`;
`libStarfishServiceIntegration` (loaded by LG QML apps) names
`com.webos.surfacemanager.input/getKeyInputBlockState` and
`systemUIManager/getForegroundAppInfo`. The rest of the 56 are called by
services not yet scanned (notification, tvpower, multiview, voice,
settings, HCAP) and by LG's own QML: a full caller census needs
`ls-monitor` once the set is free.

The compositor is itself a client of 25 luna services (subscriptions in
`StarfishLunaServiceSubscriptions.qml`): bootManager, alwaysready (AOD),
eim, irdbmanager, multiviewcontroller (3), rollingscreen (2),
settingsservice, tvpower, utp.channel (2), factorymanager, devicereset,
msc, homelaunchpoints, plus videooutput, applicationManager, notification,
tts, panelcontroller, capture, audio, airplay and voiceagent from the
views.

### Input

`arch/input-devices-01.txt`. `lginput2` (13 threads, 3.6 % of a core at
rest) owns `/dev/uinput` six times over and creates the virtual devices
the compositor reads: `LGE RCU` (IR), three `LGE M-RCU - Builtin` (Magic
Remote, with REL and ABS axes for the pointer), `LGE TONE+`, `LGE W-RCU`,
`LGE Simple Premium`, `IoT keypad`, `Smart Remote RCU Input`, `LGE Network
Input` (SSAP keys from Home Assistant), `LGE Smart Remote - TouchPad`,
`ClickableMouse`, `Bluetooth-audio-source`. surface-manager opens
`/dev/input/event0-13` directly (`QT_QPA_EGLFS_DISABLE_INPUT=1`, then
`libStarfishInput` reads evdev itself) and `/dev/uinput` once (its own
injection). A replacement compositor would read the same evdev nodes;
lginput2 (IR decode, BLE remote, voice button, SSAP keys) stays.

### Reference implementation sizes

- webOS OSE `luna-surfacemanager` (Apache 2.0): 101 C++ files, 18 613
  lines (core `weboscorecompositor.cpp` 1 657, `webossurfaceitem.cpp` 1 912,
  `webosforeign.cpp` 1 796, `webossurfacegroup.cpp` 598, `webosshellsurface.cpp`
  483, input method/text model 1 400), plus 120 QML/JS files, 11 151
  lines. It is built on `libQt5WaylandCompositor`, which supplies
  `wl_compositor`, `wl_shell`, `wl_seat`, `wl_shm`, `wl_subcompositor`, the
  `qt_*` extensions and the dmabuf/`wl_drm` buffer integration.
- LG's shipped QML on the C4: `WebOSCompositorBase` 67 QML + 2 JS,
  6 850 lines; `WebOSCompositor` (Starfish) 105 QML + 6 JS, 24 892 lines;
  `KeyFilters` 4 JS, 5 627 lines. 37 000 lines of TV behaviour on top of
  the OSE core, all of it LG-private and all of it what SAM, the services
  and the apps have been written against.
- LumaShow compositor (`crates/compositor`): 84 Rust files, 54 330 lines
  (11 714 of tests): GL renderer, glass/blur, damage, pacer, planes
  route, replay tests. Its protocol is its own (`compositor-protocol`,
  one client, layers as records), not Wayland. `crates/ui`: 61 files,
  45 506 lines.


