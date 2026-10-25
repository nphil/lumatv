# LumaTV implementation plan

LumaTV (formerly LumaShell, LumaGlass) replaces every visible part of the LG
C4's interface with surfaces in the Lucent design language, while webOS keeps
providing tuning, inputs, playback and settings. The current Home is the
visual reference. Nothing is written to `/usr`; each surface is switched on
through a reversible hook and falls back to LG's own on failure.

Design language: `skill://design-language` (Lucent v2, TV profile). Surface
inventory snapshot of the firmware: `/data/home/lgtv/uiinv`.

## Status

Kept current with every change to this plan. Building starts on Nitin's go.

- Done: resident Quick Settings, resident LumaHome (idle while hidden), key
  filter written only on change, Home stub tested on the set and reverted,
  ambient-light hardware chosen, 64-bit drawing proven on screen, the env
  hook for compositor tuning (`env.extra`) in the tool.
- Research done: full catalogue of LG's UI (`docs/SURFACES.md`, 178 rows),
  72 mockups in three batches (`docs/mocks/`), the shell architecture study
  (`docs/research/shell-architecture.md`) and the performance study measured
  on the set (`docs/research/perf-tweaks.md`). X1 failed (LG's compositor
  holds 51.2 fps for a trivial client), X2 passed, X4 partly captured.
- Decided 2026-10-24: LumaTV replaces LG's compositor, in stages (Phases,
  track C). Waiting on Nitin's go to build.
- TV tests that need Nitin present: receiver volume drift, histogram during
  Netflix, contiguous memory during 4K HDR, which input device network and
  CEC keys arrive on, pointer cost.
- Next build step: phase 1.

## Rules every surface meets

- The rule (Nitin, 2026-10-23): smooth 60 fps everywhere, no stutter. Every
  frame of every LumaTV surface lands on its 60 Hz deadline, at rest and in
  motion; the gate below is how it is checked, and nothing ships that misses
  it. Measured causes of today's misses (`docs/research/perf-tweaks.md`):
  Qt's fixed 5 ms update timer before every animated frame, the serial
  render loop, full-screen redraws for small changes, and memory pressure on
  Home return.
- Phase 1 performance work, by measured gain:
  1. `QT_QPA_UPDATE_IDLE_TIME=0` in the compositor env, beside the disk
     cache: dropped animation frames 8-9 % to 3.4-4 % on a D-pad sweep,
     46-67 % to 2.5-4 % opening Quick Settings, 12-29 % to 3-8 % closing.
  2. Wallpaper motion at 10 updates/s on one shared 100 ms clock with the
     second hand: idle GPU 30 % to 16.5 %, compositor CPU 10.3 % to 5.6 % of
     a core, idle drops 7-10 % to 2.5 %.
  3. Memory headroom before any memory tweak: the Home-return gaps (50-117
     ms, 250-920 ms after a 450 MB launch) are the compositor paging itself
     back in (1,000-4,600 major faults) and LG's memchute closing LG Home.
     The preload trim and the Home stub remove both; re-measure after.
  4. New surfaces gated at p90 render under 3 ms with no ShaderEffectSource
     rebuild on show (the third vblank on every Home return is the blur and
     icon rebuild).
  Measured with no effect, not used: GPU always-on, IRQ affinity,
  SCHED_FIFO, CPU pinning, the CPU governor (one frequency step), stopping
  LG daemons (all background work is about 25 % of one core and never in a
  gap). The threaded render loop gives a black screen on this set. CPU3
  hot-plug off is optional (idle drops 7-10 % to 3-7 %, costs idle power).
- Output: the UI is drawn at 1920x1080, 60 Hz, and LG's scaler fills the 4K
  panel. The graphics plane also lists a 3840x2160 mode, but a 4K UI is not
  planned:
  - Visible gain is nil at the viewing distance. On the 65-inch panel
    (1439 mm wide) a 1080p pixel is 0.75 mm, which at 3 m subtends 0.86
    arcminutes, already below the 1-arcminute limit of 20/20 vision; it
    only becomes visible closer than about 2.6 m.
  - Cost is four times the pixels: blur and composition scale with area,
    and one 4K buffer is 33 MB, so a triple-buffered UI (100 MB) does not
    fit in the 41-65 MB of contiguous memory free.
  - 120 Hz: the compositor paces its clients at 60 Hz (frame callbacks
    peaked at 60.8 fps in every probe), so the UI clock is 60 Hz; 120 Hz
    would also halve the frame budget to 8.3 ms. Video and games keep the
    panel's own 120 Hz modes.
- Built once and kept resident; an open shows the item and moves focus. A
  cold build freezes one frame for the whole build: Quick Settings 215-360 ms,
  the input picker 110-190 ms. Resident Quick Settings is fully open at
  170 ms with no frame over 20 ms.
- Shaders, Manrope at every size and weight in use, and icons are warmed at
  compositor start, so a first open never compiles or rasterises.
- Nothing is created per open; list rows are reused; no per-row shadows and
  no live blur. Glass over Home uses the cached wallpaper blur; over video it
  is tinted glass without blur.
- Restyles of LG QML in other processes and of web apps add no cost over what
  they replace: flat fills and the shared shader quad only.
- Gate before cutover, measured with the frame probe over Home and over
  video: no frame over 20 ms and no missed 60 Hz deadline while opening,
  navigating and closing; first frame within 50 ms of the key; fully open by
  180 ms; compositor RSS flat across 500 open/close cycles. A surface that
  fails stays on LG's version.
- One token source: `theme.json` feeds compositor QML directly, QML in other
  processes through a file import, and web apps through generated CSS. One
  typeface: Manrope (the web theme's Inter is replaced).

## Hooks

| Hook | Mechanism | Used for |
|---|---|---|
| System UI redirect | configd `com.webos.surfacemanager.systemUIManager` `appinfo[].main` | Quick panel, input picker, context menu, on-screen remote, boot and power-off logos, nudges |
| View swap | `sysuicompmgr/changeSystemUIView {viewName, viewSource}`; falls back to LG's on load error | Toasts, alerts, PIN prompt, input popups (`notificationView`) |
| Shadow module | `QML2_IMPORT_PATH=/var/lib/lumaglass/qml` first | Volume OSD, launch splash, spinners, recents, window decorations |
| Overlay mount | lowerdir = LG's app dir, upperdir under `/var/lib/lumaglass` | Live TV banners (`inputcommon`, 273 QML), keyboard (maliit, 85 QML), voice/search (203 QML), web apps |
| Key filter and launch redirect | configd `keyFilters`, ours first | Apps tile, notification drawer, entry points |

## Surfaces

The full catalogue of LG's UI on the set is `docs/SURFACES.md` (178 rows:
62 replace, 25 restyle, 24 off, 67 keep, of which 31 cannot appear on this
set). The table below is the summary; SURFACES.md is the source.

| Surface | LG today | LumaTV | Hook |
|---|---|---|---|
| Home | LumaHome over LG's Flutter Home (67 MB resident) | LumaHome only | Home stub (phase 1) |
| Apps tile | LG Store (`com.webos.app.discovery`) | Own all-apps grid | Launch redirect |
| Quick panel | 165-file QML, rebuilt per open | Rebuilt resident | System UI redirect |
| Input picker | 51-file QML, rebuilt per open | Rebuilt resident, own inputs only | System UI redirect |
| Volume and mute OSD | Compositor QML; shows webOS's own counter | Rebuilt, fed by the receiver (below) | Shadow module |
| Toasts, alerts, PIN, input popups | `SystemUI/Notification`, 38 files | Rebuilt to the item contract | View swap |
| Launch splash, spinner, long-press ring | Compositor QML | Rebuilt | Shadow module |
| Live TV banner, info, no-signal | `inputcommon` QML | Restyled | Overlay mount |
| On-screen keyboard | maliit QML | Restyled | Overlay mount |
| Search and voice | `com.webos.app.voice` QML | Restyled or replaced | Overlay mount |
| Settings | Web app, +93 MB while open | Own Settings (below); LG's app themed only for its deep screens | Launch redirect + web theme |
| Settings sub-apps | Enact web apps | One shared theme | Overlay mount |
| Notification Centre | Web app | Own drawer | Launch redirect |
| Context menu, on-screen remote | Compositor QML | Rebuilt | System UI redirect |
| Recents | Drawn by LG's Home | Own switcher | Home stub |
| Multi-view frames | Compositor QML | Kept as LG's: Multi View is not used (Nitin, 2026-10-23) | None |
| Boot and power-off logo | Compositor QML | Own animation | System UI redirect |
| Screensaver | Flutter | Own, keeping OLED pixel shift | Research |
| Nudges, sports alarm, store promotions | Compositor QML | Off (empty view) | System UI redirect |
| Ad and content-recognition overlays | Native | Off through LG settings | Settings |
| HDMI input banner, OK/INFO banners, no signal | `inputcommon` QML | Rebuilt (input pill, detail, burn-in-safe no-signal) | Overlay mount |
| Power-off logo, startup cover | Compositor QML, `CoverUI` (bootd opens it; it launches the first app) | Own; CoverUI's launch duty kept | System UI redirect |
| Standby screen (Always Ready, Gallery+) | `lifeOnScreenMode: alwaysReady`, LG art and clock | Off: the panel goes fully dark in standby (Nitin, 2026-10-23); instant wake kept | Settings |
| Voice listening bar, microphone indicator | `VoiceAgent` QML | Own voice bar for search only: app and title results from what is on the set, no LG cloud answers (Nitin, 2026-10-23); mic indicator always shown | Shadow module |
| Pairing prompts (SSAP "Connection Request", mobile PIN) | SystemUI alert, configd popup | Rebuilt; answered only with the remote | View swap |
| Pointer right-click menu | Compositor QML (Back/Home) | Rebuilt | Shadow module |
| App launch prompts (update required, unavailable) | SAM alerts | Rebuilt through the item contract | View swap |
| Quick Access (number keys) | Toast and slot alert | Rebuilt | View swap |
| Game Dashboard | HDMI game-mode overlay | Restyled; a PC is used on HDMI (Nitin, 2026-10-23), so input details show PC facts (4:4:4 chroma, refresh, VRR, ALLM); game mode, VRR and latency untouched | Overlay mount |
| Setting confirmations and recommendations | pqcontroller, ocp alerts | Rebuilt | View swap |

Messages that are never filtered or delayed, whatever LumaTV's state: US
emergency alerts (EAS), thermal shutdown, Magic Remote battery, LG Remote
Service sessions, permission and pairing prompts. They are restyled only
where the restyle cannot fail; otherwise LG's own is shown.

Keys LumaTV must not break: the hidden combinations (gear x3 AV reset,
Yellow 5 s, Green/Blue x7) and the one-key TV button path. Holding Home
(650 ms) is the LG-Home escape only; Screen Zoom moves to the context menu
(Nitin, 2026-10-23), so LG's 2.5 s Home hold is filtered.

The notification drawer opens from the bell on Home's top bar and by holding
the Settings (gear) key; the library opens from the Apps tile (Nitin,
2026-10-23). The gear hold must not collide with the gear x3 AV reset: a
hold is a single press held 650 ms, the reset is three taps.

The remote's app keys (Netflix, Prime Video, Disney+, LG Channels and the
rest) are set per key in Settings › Remote & pointer: open any installed
app, or do nothing (Nitin, 2026-10-23). The default is LG's target where that
app is installed and nothing where it is not, so no key starts LG's
auto-install.

Not owned: third-party app UIs (Netflix, YouTube, PlxNative), video, and the
Magic Remote pointer image (native; swap unverified). The pointer stays
supported: every surface has a hover state, and Settings › Remote & pointer
turns it off through `settingsservice` `general.remotePointer` for a
button-only remote. Turning it off removes accidental hover moves; any frame
cost of the pointer itself is measured with the frame probe before claiming
one.

## Receiver: volume and controls

webOS's ARC volume is the TV's own up/down counter, not the receiver's level:
`audio/master/getVolume` reports `externalDeviceControl: true`,
`volumeSyncable: false`. It drifts whenever the Denon is adjusted from its own
remote or knob. The third-party Webos-EARC-Volume-Overlay reads that counter
and relaunches a web app per change, so it is neither accurate nor fast, and is
not used.

The LumaTV volume OSD takes its level from the Denon AVR on the LAN instead:
the receiver pushes every master-volume and mute change (`MV`, `MU` events),
so the OSD matches the front panel in the receiver's own units (dB or 0-98),
including changes made on the receiver. The connection lives in the native
helper (below) and is held open; the OSD only renders the last value. It can
also show input and sound mode (e.g. DTS Surround) on change.

Before building:

1. Confirm drift on the set: with the TV and AVR on, change volume on the
   Denon remote and compare `getVolume` with the receiver's level.
2. The receiver is a Denon AVR-S760H at 192.168.1.93 (D&M MAC prefix
   00:06:78). Its HTTP interface on port 8080 answers without a held
   connection (`/goform/formMainZone_MainZoneXmlStatusLite.xml`: power,
   input, absolute volume such as -34.5 dB, mute), so status reads never
   compete with Home Assistant. Push events need the telnet port (23), which
   Home Assistant's `denonavr` integration holds (`use_telnet: true`); check
   whether the S760H accepts a second telnet client. If not, events come
   from Home Assistant's WebSocket (`media_player.living_room_avr`, one hop)
   and commands go over HTTP.
3. Volume keys still go to the receiver through HDMI-CEC as today; LG's own
   volume popup is suppressed only once the OSD has passed the gate.

Receiver controls in Settings › Sound › Receiver, shown only while the AVR is
reachable, with the same reach as Home Assistant's Denon integration: power,
input (the receiver's own source names: Apple TV, Bluetooth, HEOS Music,
MAGNAVOX 4K BD, PS4, TV Audio), sound mode (MOVIE, MUSIC, GAME, DIRECT, PURE
DIRECT, STEREO, DOLBY DIGITAL, DTS SURROUND, AURO3D, MCH STEREO, and the DSP
styles under More), Audyssey (Dynamic EQ, Dynamic Volume, MultEQ), dialog
and subwoofer levels, bass and treble, Cinema EQ, ECO mode, Quick Select 1-4
and Zone 2. A receiver tile in the quick panel shows input, sound mode and
volume. Every command is the receiver's own (`SI`, `MS`, `PS`, `MV`...);
values are read back after each change, and any value older than 6 s
without an answer is hidden rather than shown.

## Native helper

A static aarch64 helper in Rust runs beside the compositor, in the language
of LumaShow OS so the two share crates rather than reimplement them
(`plugin-api`, `lumashow-sdk`, `settings-schema`, `service-api`, `ha-link`,
`fetch`, `mdns`). A static `aarch64-unknown-linux-musl` build, linked with
`rust-lld`, runs on the set as is (hello world 588 KB). It holds the receiver
connection, the MCP server, the plugin broker and the settings data layer.
The compositor's userspace is 32-bit and stays that way; the helper does not
change rendering cost. News and weather parsing does not justify it on its
own: measured at 0-1 ms per feed on the compositor thread.

## Plugins

LumaTV hosts LumaShow's plugins, so one plugin package runs on every Luma
device. LumaShow's plugin API (`lumashow/docs/plugin-api.md`, level 1)
already fits: a plugin never draws. It declares settings pages, cards, quick
tiles, notifications and timeline items as records, and each device's shell
draws them in its own sizes and for its own input, so the TV draws a card
with remote focus at TV scale and the Show draws it for touch. Plugins are
static aarch64 binaries, and the Show, the Lenovo and the TV are all aarch64,
so the same build installs on each.

- The broker in the helper speaks the same protocol; records become QML
  surfaces in LumaTV: cards on Home, tiles in the quick panel, pages in
  Settings, notifications as LumaTV toasts.
- Device facts in `welcome` (screen class, input methods: touch, remote,
  pointer, controller) are an additive field, for plugins with their own
  views that must adapt; record-only plugins need nothing per device.
- Home's own widgets (weather, news, at home) become first-party plugins once
  the host exists.
- The set's kernel has no user or network namespaces and no seccomp (only
  mount, pid and cgroup isolation), so the Show's sandbox cannot be
  reproduced: plugins run under their own user with mount, pid and memory
  limits, and only reviewed plugins are installed on the TV until network
  access per plugin is enforced another way.

Apps are the other kind of extension: full 64-bit programs with their own
window, drawn with LumaShow's toolkit on the 64-bit driver (Rendering stack),
listed and launched like any app. Plugins extend the shell; apps are places
of their own. An app runs as a card window registered with the app manager,
so Home, Back and the app switcher treat it like LG's; only the overlay
window type is proven so far.

## Rendering stack

LG's compositor (`surface-manager`) is Qt 5.12 eglfs on DRM/KMS (`card0`,
`lg1k`) with LG's Wayland server extensions (`libwayland-webos-server`,
`libWebOSCoreCompositor`) and `libvt` for video planes. Besides drawing, it
is the hub the rest of webOS talks to: SAM, WAM and other services call its
luna API (`com.webos.surfacemanager`: foreground app, recents, close by app,
input picker, quick settings, spinner, PIN overlay, multi-view, screen
visible, compositor capture and more), and every app, Netflix included,
reaches the screen through its Wayland extensions; protected video stays on
the hardware planes it places.

Hardware: four Cortex-A76 cores at 1.4 GHz, 2 GB RAM, a Mali-G52 with 3
cores, and a 64-bit kernel. Userspace is 32-bit throughout: LG ships only a
32-bit GPU driver (`libmali`, Bifrost DDK r46p0, 52 MB) for the 64-bit
`mali_kbase` module. The cost is CPU only, and smaller than it looks: with
2 GB of RAM the 4 GB address limit never binds, and the GPU, which carries
the blur and composition, runs the same shaders at the same speed for 32-bit
and 64-bit callers. Measured on the set with one Rust benchmark built both
ways (best of 3 runs): 64-bit is level at byte hashing (736 vs 686 ms), 8-10 %
faster at sorting and text parsing, and 31 % faster at float loops (91 vs
119 ms). The helper and plugins are 64-bit already.

- Compositor replacement: decided yes (Nitin, 2026-10-24) on the X1 and X2
  results below (`docs/research/shell-architecture.md`). LG's compositor
  repaints the whole 1080p window every frame on one thread, with LG's JS
  key filters and luna callbacks on the same thread; it holds 100 MB and
  13 % of a core at rest, and an idle LumaHome frame costs 2.8-3.4 ms CPU
  plus 8.1 ms GPU of the 16.7 ms budget. A Rust compositor would replace 56
  luna methods on 12 service names, the tvpower power handshake, 9 public
  webOS protocol files (84 requests, 49 events) and two small starfish
  protocols; WAM's contract is 8 window types. Video, copy protection and
  HDR never pass through a compositor: decoded frames sit in 137 MB of
  secure memory on the display engine's video windows, and the compositor
  only places them through videooutputd. Owning a spare plane beside LG's
  is not possible: the set has one CRTC, one connector and three planes
  (primary and overlay are LG's, the cursor plane carries LG's pointer), and
  only the DRM master can commit.
- Path: (1) the 64-bit Rust UI from LumaShow's toolkit as a client of LG's
  compositor, written so it runs unchanged inside LumaTV's own; (2) LumaTV's
  compositor built beside it (Phases, track C) and switched to only when it
  passes the gate on the set. The experiments behind the decision:
  - X1, failed (2026-10-24): a trivial 64-bit client over idle Home with
    wallpaper motion off held 51.2 fps for 60 s on contiguous buffers, 530
    of 3,085 presented frames took two vblanks (17 %), worst 33.4 ms; 47.9
    fps on `dma_buf_te` buffers. With a full-screen client in front LG's
    compositor spends 6.3 ms GPU and about 5 ms CPU per presented frame on
    composition alone, and the UI plane follows the content's refresh rate
    (50 Hz behind a browser page). LG's compositor cannot meet the 60 fps
    rule for any separate window, LumaTV's 64-bit UI included.
    What an own compositor gains, measured: direct scan-out of a full-screen
    app on the unused overlay plane (no composition cost), committing with
    an in-fence and rendering one frame ahead (CPU and GPU each get the full
    16.7 ms), damage tracking (a 4 px ripple now redraws 1080p for 7.5 ms),
    and a UI clock independent of the content's rate.
  - X2, passed (2026-10-24): every LG app draws through LG's 32-bit
    `libmali`, which binds only `mali_buffer_sharing` v5 (three messages)
    and hands over AFBC-compressed dma-bufs from `/dev/dma_buf_te`. A test
    compositor on a private socket imported them pixel-exact, 6 of 6
    frames, both with LG's driver (`eglBindWaylandDisplayWL`) and with the
    64-bit Rockchip blob, so an own compositor can be 64-bit and needs no
    buffer protocol code of its own.
  - X4, partly captured (launches, Quick Settings, Settings, browser,
    YouTube): the compositor's live contract is about ten pushed
    subscriptions (foreground app, focused window, window list, video
    window, input type, Home UI state) plus a few calls (`listRecents`,
    `systemUIManager/launch`, `launchDefaultApp {home}`, per-video
    `setDisplayWindowProperty`). Apps place their own video rectangle; the
    compositor reports it. Standby and wake, Netflix, PlxNative, the input
    picker and PIN are still to capture.
  - Still open for an own compositor: LG's window and view behaviour (the
    webOS Wayland extensions), the tvpower handshake, punch-through and key
    routing.
  X1 failed and X2 passed, so LumaTV's compositor replaces surface-manager,
  with a watchdog and a firmware check that both fall back to LG's.
- 64-bit drawing path: works on screen (2026-10-22). LG's `mali_kbase`
  speaks kernel interface 11.40 and accepts 64-bit callers. Rockchip's
  aarch64 Mali-G52 Bifrost builds (`libmali-bifrost-g52-g24p0`, g13p0 too),
  run with Debian bookworm arm64 glibc, libstdc++, libdrm, libwayland-client
  and libffi through their own loader from a private directory, report
  Mali-G52 / OpenGL ES 3.2.
  - Offscreen, against LG's 32-bit r46p0 driver with the same probe: a
    full-HD shader fill is equal (22.2 vs 21.9 ms, GPU-bound); 2000 small
    draws per frame cost 15.3-17.5 ms against 23.9-27.8 ms, about 36 % less
    CPU per draw.
  - On screen: the `-wayland-gbm` build is a client of LG's compositor
    (`wl_shell` plus `wl_webos_shell`, window type
    `_WEBOS_WINDOW_TYPE_OVERLAY`) and hands frames over through
    `zwp_linux_dmabuf_v1`. It allocates from `/dev/dma_heap/*`, which kernel
    5.4 lacks (`/dev/ion` exists with no heaps), so a preloaded shim answers
    those opens and `DMA_HEAP_IOCTL_ALLOC`. Full-screen window over Home,
    8 s per run, by buffer source:

    | Buffers | fps | Over 20 ms |
    |---|---|---|
    | 64-bit, DRM dumb on `card0` (contiguous CMA) | 60.2-60.8 | 2-10 |
    | 64-bit, Arm `/dev/dma_buf_te` (ordinary pages) | 55.9-56.6 | 35-41 |
    | LG's 32-bit driver, its own `mali_buffer_sharing` | 55.5-55.8 | 42-46 |

    The on-screen gain comes from contiguous buffers, not from 64-bit.
    Contiguous memory is scarce: 41-65 MB of CMA free (692 MB total, mostly
    held for video), and in standby a second 8.3 MB 1080p dumb buffer
    already failed. `dma_buf_te` (the exporter LG's compositor holds open)
    allocated twelve 1080p buffers from ordinary RAM without trouble.
  - Own swapchain without an interposer (2026-10-23): works and shows the
    right picture, but is slower. The windowless blob renders into three
    dma-bufs allocated once, imported with `EGL_EXT_image_dma_buf_import`
    and attached through `zwp_linux_dmabuf_v1` (the compositor offers only
    the linear modifier for ARGB8888), paced by `wl_surface.frame`:
    50.2-51.7 fps with 122-153 frames over 20 ms in 8 s. The cost is the
    import itself. The same 1080p frame (three clears and `glFinish`),
    offscreen, per render target:

    | Render target | 64-bit g24p0 | LG 32-bit r46p0 |
    |---|---|---|
    | Driver's own pbuffer or texture | 1.3-1.4 ms | 1.4-2.4 ms |
    | Imported DRM dumb buffer | 8.5 ms | 6.3 ms |
    | Imported `dma_buf_te` buffer | 29.0 ms | 30.4 ms |

    Imported memory costs both drivers 5-20 times the driver's own, so a
    renderer cannot draw into buffers it hands in. The driver's window path
    is fast because it allocates through what it takes for a dma-heap
    ("system-uncached") and treats the memory as its own.
  - Production shape: the driver's own window path (`-wayland-gbm`) with a
    minimal heap shim: it answers only opens of `/dev/dma_heap/*` and their
    `DMA_HEAP_IOCTL_ALLOC`, allocating DRM dumb buffers on `card0` and
    falling back to `dma_buf_te` when contiguous memory runs out; every
    other call passes straight through. The blob ships with LumaTV, so its
    version changes only with a LumaTV release that passes the gate. If the
    64-bit path fails to start, LumaTV draws through LG's 32-bit driver.
    The kernel-side alternative, a module adding `/dev/dma_heap`, needs LG's
    GPL source for 5.4.268 and is Nitin's decision.
    Before relying on contiguous buffers, check that holding about 25 MB of
    CMA never starves 4K HDR decoding in Netflix and PlxNative.
  - Probe sources kept in `docs/research/m64/`.

- NPU: LG's own (`DNE`, vendor LGE, driver 1.0, `/dev/npu0`, module
  `npu_drv`), 0.4 TOPS, managed by `com.webos.service.aiinferencemanager`
  (`open`, `requestResource`, `installModel`, `getDeviceCapability`). Its only
  registered user is the picture controller's AI picture (`ai_pq`, 0.29 TOPS,
  priority secondary), which Filmmaker mode leaves idle; usage reads 0. The
  userspace is `libnpu` (`lgnpu_init/prepare/invoke`, offline-compiled
  `.lgnpu` packages through `libwebos_offline_nnhal`), with TensorFlow Lite
  and `libtflite-npu-delegate` on top. On CPU, TFLite runs LG's YOLOv3 int8
  detector in 421 ms and its sound model in 66 ms (4 threads); the delegate,
  loaded through `benchmark_model --external_delegate_path`, applies to none
  of LG's shipped models, so it needs options or a resource lease first.
  Spike: reverse the delegate's options and the inference manager's
  resource flow, then run one LG package with `run_lgnpu` and time it against
  the CPU. Uses for LumaTV come after that number: on-device scene
  classification for ambient light and picture profiles, or local voice
  commands (the microphone is Nitin's decision).

## Settings

LG's Settings (`com.palm.app.settings`, an Enact web app, 25 MB on disk,
+93 MB while open) is replaced by LumaTV's own Settings, drawn in the
compositor and resident like Quick Settings. It writes the same places LG's
app does (`settingsservice` categories such as `picture`, `sound`, `general`,
`option`, and the audio, network and input services), so LG's processing,
panel care and stored values are unchanged; only the screens are new.

- Layout follows LumaShow's Settings ("Live list"): a rail of sections, each
  with a glyph and a one-line summary of its current value, and a page with
  tabs, rows, sliders and switches. Same component kit as LumaShow, Lucent
  tokens, TV sizes.
- Sections: Picture, Sound, Inputs & devices, Network, Ambient light, Home &
  look, Remote & pointer, AI access, System. LG's roughly hundred items are
  curated into these; rare deep screens (expert calibration, channel tuning,
  legal, LG account, software update) stay LG's, themed, behind "More in LG
  settings" rows that open them directly.
- The Settings key, the gear on Home and Quick Settings' "All settings" all
  open it (launch redirect); LG's app remains reachable from System.
- Mockups: `docs/mocks/` (gallery `index.html`, mapping of LG's items to the
  sections in `settings-map.md`).

## AI access (MCP)

The native helper serves an MCP server so AI assistants (Cody, Home
Assistant's agents) can use the TV, with every permission held on the TV and
changed only with the remote. Principle 0 sets the shape: a confirmation an
AI can reach is not a control, so nothing an AI can do is able to grant,
widen or approve its own access.

- Assistants connect straight to the TV (Nitin, 2026-10-22). Off by default.
  Settings › AI access turns the listener on (LAN only, bound
  to the LAN address, private source addresses only). Turning it on opens a
  network entrance, so it is done by Nitin with the remote, never through
  MCP, a script or chat.
- Pairing: the TV shows a one-time code; the assistant presents it once and
  receives its own token, stored hashed on the TV. Tokens are created and
  removed only from the TV's screen. "Lock out all AI" revokes every token
  and stops the listener.
- Typed tools only: no raw luna, shell, key-injection or file tools, so an
  assistant cannot drive LG's screens or LumaTV's own menus. Each tool has
  a level set per assistant: Always, Ask on TV, or Never.

| Group | Default |
|---|---|
| See what is playing, power and input state | Always |
| Play, pause, volume, mute | Always |
| Open apps, switch inputs | Ask on TV |
| Change picture and sound settings | Ask on TV |
| Turn the TV off | Ask on TV |
| See the screen (capture) | Never |

- Never exposed, at any level: AI access itself, pairing, network and
  developer settings, uninstalling apps, resets, software updates, LumaTV
  install or updates, the microphone.
- "Ask on TV" opens a LumaTV prompt over whatever is playing: who asks,
  the action in LumaTV's own words, the assistant's reason labelled as
  its own, Allow once or Deny, expiring after 30 s. A denial pauses that
  assistant's requests until it is resumed in Settings. Every call is logged
  and shown in Settings › AI access.
- The prompt and the AI access page answer only to keys from a physical
  remote. All remotes reach the compositor as `lginput2` virtual devices (IR
  `LGE RCU`, Magic Remote `LGE M-RCU`, `IoT keypad` and others), and Home
  Assistant's `webostv.button` presses (SSAP, port 3001) arrive the same way.
  Before building, measure which device SSAP and CEC keys appear on; if
  network keys cannot be told apart from the remote, approvals go through
  HomeLabber's Face ID gate instead of the on-screen prompt.
- TV root bypasses all of this. While an AI holds root SSH on the set (the
  development path today), the permissions bind only assistants that use the
  MCP server; root access for development stays a separate decision of
  Nitin's.

## Compositor startup

`QML_FORCE_DISK_CACHE=1` in `/var/systemd/system/env/surface-manager.env`
overrides LG's global `QML_DISABLE_DISK_CACHE=1`. It cut restart-to-Home from
about 5.8 s to 4.6-5.2 s and does not change cold opens. Adopt through the
tool, with the cache (6.2 MB, `/home/root/.cache/surface-manager/qmlcache`)
cleared on every stage change.

## Ambient light

LumaTV reads the colour of whatever the set is showing and drives the Hue
backlights and an RGBIC strip from it, with no Hue Bridge and no Hue Sync app.
It works during DRM playback (Netflix, Prime, Disney+), where frame capture
returns black on webOS 24 and 25.

Source: LG's own `libvtcapture.so.1` (32-bit, `/usr/lib`) has a histogram API
beside the frame capture: `vtCapture_initHistogram`,
`vtCapture_setPresetHistogram`, `vtCapture_getHistogram`,
`vtCapture_finalizeHistogram`. It returns colour statistics for 16 screen
zones straight from the video hardware. LG's Hue Sync service on the set
(`com.lighting.huesync.service/hue_sync_service`, read only as a reference)
`dlopen`s the library and calls exactly these, logging one r/g/b triple per
zone. hyperion-webos's `HISTOGRAM_OUTPUT` dump location is ordinary frame
capture, not this API.

Design:

- Capture runs in a small 32-bit process, because LG's library is 32-bit; it
  `dlopen`s the library as Hue Sync and hyperion-webos do, polls the zones and
  hands them to the helper. It holds the driver only while there is video and
  the screen is on, and releases it on standby.
- The helper maps zones to lights and smooths them. Features taken from
  HyperHDR as reference: time-based smoothing, brightness and saturation
  gain, per-light white balance and gamma, a dark threshold so black scenes
  turn lights low rather than flicker, black-bar handling for letterboxed
  films, and a delay setting to line light changes up with picture and sound.
- Hue backlights: Hue Entertainment streaming (HueStream over DTLS, the
  protocol Hue Sync and HyperHDR's Hue driver use) to Bifrost, which drives
  the bulbs through Zigbee2MQTT in Zigbee entertainment mode. Plain MQTT light
  commands are too slow for sync and are not used. Gradient lightstrips take
  one colour per segment.
- RGBIC strip: WLED's UDP realtime protocol (DDP), per segment or per LED.
  ESPHome's WLED effect accepts the same protocol, so an ESPHome strip works.
  Vendor strips (Govee and similar) depend on their LAN protocol and are
  checked per model.
- Controls: an Ambient light tile in the quick panel (on/off, intensity), and
  a settings screen that places each light around a picture of the TV, with a
  test pattern. Off on Home and menus unless chosen.
- Gate: no measurable compositor cost; under 1% CPU for capture; light lag
  measured end to end and tuned with the delay setting; capture never stalls
  playback; tested on Netflix, Prime, Disney+, PlxNative, HDMI and Live TV.

Before building, with the set free:

1. Probe: call the four functions during Netflix playback and log the zones;
   find the zone layout (grid or edges), the value range (8-bit, float,
   SDR or HDR), the update rate and the CPU cost. The zones come from the
   video plane, so LumaTV's own UI is likely not in them.
2. Lights: pair the Hue backlights to Zigbee2MQTT (2.14.2, running as a Home
   Assistant app). Bifrost is not installed anywhere yet; installing it is
   Nitin's step. It needs its own IP on the LAN, since it answers as a Hue
   Bridge on ports 80 and 443.
3. Strip, on order: BTF-Lighting FCOB WS2814 RGBW 4000K, 24 V, 5 m (14
   addressable sections per metre, 71 mm each), BTF 10 mm 3-pin FCOB connector
   kit for the corners, Gledopto Elite GL-C-619WL (ESP32, WLED, Ethernet,
   level shifter, relay) and a 24 V 100 W supply. About 58 sections around
   the 65-inch frame; bottom edge depends on wall mount or stand.

## Reference: ecklf/lg-luma

Luma (MIT, Rust/SDL app) is a separate launcher app that the Home key is
rerouted to; LumaTV already owns Home inside the compositor, so only its
ideas are taken, not its architecture.

- Inputs on Home: `com.webos.service.eim/getAllInputStatus` gives each HDMI
  port's label and `connected` flag. Home gets an Inputs row (label, device
  icon, connected state), and each input has Auto / Always / Hidden, Auto
  meaning shown only while something is connected. The same data feeds the
  input picker.
- Device icons per input (console, PlayStation, Apple TV, streamer, computer),
  chosen from the CEC label or by the user.
- All-apps library for the Apps tile: categories (Streaming, Music, Gaming,
  Utilities, matched from app id and title), search with the system keyboard,
  favourites, hide and restore. Luma's filter for what counts as a user app
  (drop invisible apps and system `com.webos.`/`com.palm.`/`com.lge.` ids
  unless marked visible; keep Live TV) is a sound starting rule.
- Key hints along the bottom edge ("OK Open", "Hold OK for options").
- Icons: Lucide (ISC licence) as the UI icon source, rendered once to the
  sizes in use, replacing the hand-drawn PNG set.

## Reference: rudra-avatar/Hearth

Hearth (MIT) is a web launcher that a watcher brings forward over LG's Home.
Taken:

- Boot logo: LG's hidden "Show LG logo when turning on" is
  `settingsservice` `other.lgLogoDisplay` ("on" on this set). Part of the
  clean cold start with the Home stub; offered as a setting, put back on
  `lumaglass off`.
- Saved first, fetched later: the last weather reading and news items are
  written to `/var/lib/lumaglass/cache` and drawn at once on a cold start,
  then refreshed; a reading older than 6 hours is not shown, and failed
  fetches retry after 2, 10 and 30 minutes.
- App library: hold OK for Move to start/left/right/end, Hide and
  Uninstall. Uninstall is offered only with positive evidence the app is
  user-installed (under `/media/developer/apps` or `/media/cryptofs/apps`,
  not a `com.webos.`/`com.palm.`/`com.lge.` id, never LumaTV, Home or the
  Homebrew Channel).
- Back on the dock row jumps to its first tile.
- Wallpaper slideshow from a USB drive or `/media/developer`, with interval
  and shuffle; pictures over 8 MB or 25 megapixels are skipped with a reason.

## Reference: other webOS projects

Survey of the GitHub and r/webos homebrew projects from 2025-26. Phase in
brackets.

Rendering and measurement (plx-native, liquid-glass launcher):

- Blur cost follows the merged blurred area, not the number of panels: on
  plx-native's webOS 4.5 set 60 fps held under about 300k blurred pixels and
  fell to 24 fps full screen, and two panels far apart cost more than two side
  by side. Glass stays grouped; the budget is re-measured on the C4 [1].
- Each presented frame carries a fixed compositor charge (23-26 % of
  compositor time there, more than the whole app). Anything with a clock or
  animation, including the screensaver and ambient overlays, presents only
  when something changes [1].
- The GPU profiler read 45 fps on a 60 fps run; frame-rate claims come from
  unprofiled runs that count presented frames [1].
- Focus: the highlight moves at once; lift and scale animate only after focus
  rests 150 ms, and further moves cancel the pending animation. Animating
  glass per key press took that launcher from 16.8 ms to 127 ms mean frame
  time [1].

Input (rawin-launcher, HomeBack):

- OK acts on release, not press: acting on press lets the release reach the
  app underneath, which can launch a second app. Hold OK (700 ms) is the
  secondary action [1].
- The key filter fails open: a key is swallowed only while LumaHome is
  answering, so a hung LumaHome never traps the remote. Holding Home (650 ms)
  opens LG's Home as a manual escape, beside the heartbeat fallback [1].
- Magic Remote pointer off is `settingsservice` `general.remotePointer` ("on"
  on this set); no input injection needed.

Home and inputs:

- Live HDMI preview on the Inputs row (webos-nxe): a muted
  `service/webos-external` video only after focus rests on a connected port,
  torn down on the next move, so no video pipeline is built while scrolling.
  `getAllInputStatus` reports presence as `activate` on webOS 10 [3].
- Favourites: defaults written once, so an update never reorders the saved
  list (rawin). Per-app rename, recolour and custom icon (webos-23-custom-home)
  [3].
- Dynamic tint from the wallpaper: sample at 32x18, bin each channel into 8
  levels, skip near-black, take the most common bin; a newer wallpaper cancels
  a running sample (liquid-glass launcher) [1].
- Hero row of trending titles filtered to installed apps, OK opening the app
  (lglow; needs a TMDB key) [7, optional].
- Trial deploys (webos-home): file hashes checked before each bind mount, and
  a new Home build rolls back after 120 s unless confirmed. Used when the Home
  stub is productionised [1].

Screensaver [8]:

- LG's screensaver is not a foreground app; `tvpower/power/getPowerState`
  reports `"Screen Saver"` and `turnOnScreen` ends it (lumen-webos).
- `com.webos.app.screensaver` on this set is a Flutter bundle, so the QML
  bind-mount of webosbrew/custom-screensaver does not apply; the replacement
  is drawn in the compositor on that state.
- Burn-in rules (tv-tools, zzeppieri): mostly black, moving elements, scenes
  45-75 s with 4 s black fades, dim to 60 % over an hour, a clock that drifts
  to a new spot every 60 s; standby after a set number of hours.
- Art mode: public-domain paintings with mattes and frames (Lumen Gallery),
  stored on the set with per-image licence checks rather than fetched at
  view time. Next picture decoded before the swap, 1.4 s opacity crossfade,
  broken pictures skipped after 300 ms, weighted shuffle across
  collections, a quick bar (Previous, Like, Hide, Next). Frames and mattes
  orbit and dim with the picture; Lumen Gallery's static layers have no
  burn-in protection.

From Lumen (`docs/research/lumen.md`) for Home [1]: the focused tile grows
without a clip (a clipped scale rebuilds a mask every frame); wallpaper
image and motion are released 5 s after Home is hidden; only the first row
is built at start, the rest 3.5 s later or at the first key, and wallpaper
motion starts after 1.5 s without a key; a hover counts only when the
pointer moved (a resting pointer otherwise fires hover on whatever slides
under it); wallpaper dim Off / At night / Always; focus strength Subtle /
Normal / Strong.

Receiver OSD [4]: receiver values older than 6 s without a successful read are
hidden rather than shown stale (webos-av-overlay).

Platform findings:

- `libvtcapture` has one owner: a second `vtCapture_init` fails with code 11
  (scaler busy), so the ambient histogram and any frame capture cannot run
  together (lg-hue-sync). That project's bridge client accepts only Hue's own
  certificates, so it would not talk to Bifrost.
- LG Home can be restored from a CRIU snapshot that ignores changed files; a
  stale one is dropped by touching `invalid_img` in its checkpoint directory.
  On this set `/mnt/lg/cmn_data/checkpoint` holds only `inputcommon`.
- HbbTV is already off (`option.hbbTV`).
- LG hard-codes public DNS servers and DNS-over-TLS, so a hosts-file blocker
  is incomplete without redirecting DNS; that redirect is a network change and
  Nitin's step. If blocking stops LG's time sync, the clock is set from the
  HTTP `Date` of two agreeing sites.

Not taken: locking the SSAP port 3001 to localhost (Home Assistant controls
the set through it), neutralising the remote microphone (voice uses it),
lginputhook's admin server (LAN port with path traversal), self-updaters, and
webos-mcp (raw control with no on-TV permissions; LumaTV's own MCP server
above replaces it).

## Phases

1. Foundation: component kit, tokens, Manrope everywhere including the web
   theme, startup warm-up, frame-probe harness, a registry of every surface's
   state and fallback, the native helper, the disk cache, and LumaHome in
   LG's Home's place (Home stub, see Glasshouse).
2. Notifications: toasts, alerts, PIN prompt, input popups.
3. Quick panel, Settings, and AI access (MCP server, permissions, prompt).
4. Input picker, the receiver-fed volume and mute OSD, and ambient light.
5. Launch splash, spinners, boot and power-off logos, context menu, on-screen
   remote.
6. Restyles in other processes: Live TV banners, keyboard, search and voice,
   the shared web-app theme.
7. Removal: nudges, ads and store promotions off; LG's background services
   off through Glasshouse (below); Apps tile to the own grid; Notification
   Centre to the own drawer.
8. Trim background preloads, replace the
   screensaver. Research first, under the app-manager constraint below.
9. Plugins: the broker on LumaShow's plugin API, TV drawing of cards, tiles,
   settings pages and notifications, and Home's widgets moved to plugins.

Track C, the compositor, runs beside phases 1-9 and switches over only
when it passes on the set:

- C1: finish the X4 capture (standby and wake, Netflix, PlxNative, the
  input picker, PIN) so the whole contract LG's apps and services rely on
  is known before code is written.
- C2: a 64-bit Rust compositor on a private socket, built on the X2 import
  path, running LG's apps unchanged: the webOS Wayland extensions, the 56
  luna methods, the tvpower handshake, key routing, punch-through and video
  placement through videooutputd.
- C3: the 60 fps work LG's compositor cannot do: a full-screen app scanned
  out directly on the overlay plane, commits with an in-fence and one frame
  rendered ahead, damage tracking, and a UI clock independent of the
  content's refresh rate.
- C4: trial cutover with LG's compositor kept installed: a watchdog and a
  boot-time firmware check switch back to it on any failure, and the hold-
  Home escape forces it. Gate: the X1 test with zero frames over 20 ms in
  60 s, every surface gate met, and Netflix, HDR, Dolby Vision, PIN, input
  switching, standby and wake all passing scripted tests.
- Every LG firmware update: LumaTV starts on LG's compositor until the
  contract checks pass again on the new firmware.

## Glasshouse

Glasshouse (rorygallagher2024/lg-webos-dashboard) is installed on the set: a
Node server (`/var/lib/tvweb`, about 34 MB resident) that bridges the TV to
Home Assistant over MQTT and holds LG's ad and telemetry hosts blocked with a
bind-mounted `/etc/hosts`. LumaTV coexists with it and does not duplicate
what it already does.

- Background services. Glasshouse masks LG daemons under
  `/run/systemd/transient` and holds them down from a boot hook. Running now
  and switchable there: `uploadd`, `rdxd`, `contentminer`, `nudge`,
  `remotelogger`. Phase 7 switches them off through Glasshouse rather than a
  second mechanism; `nudge.service` off stops nudges at the source.
- App manager restarts. Swapping the screensaver's runtime (Flutter to QML on
  webOS 10) or hiding system apps through `appinfo.json` bind-mounts needs SAM
  restarted, and on webOS 10 and 11 that restart has been followed by lost
  picture, lost ARC sound and dead HDMI-CEC until a power cut (Glasshouse
  #366). This set sends sound to the Denon over eARC, so LumaTV never
  restarts SAM: a replacement screensaver either lands before SAM first starts
  (the early boot hook, if it runs first) or is drawn by the compositor.
  System apps are hidden in LumaTV's own launcher, not through
  `appinfo.json`.
- OLED care. The luna calls Glasshouse uses are a ready source for an OLED
  care screen in the curated settings: `panelcontroller/getPanelUsageTime`,
  `tv.display/getClearPanelNoiseStatus` and `requestClearPanelNoise` (Pixel
  Refresher), `oledepl/getGlobalStressReduction` and `getTemporalPeakControl`,
  `tv.systemproperty` `OledGSR`/`OledTPC`.
- Game Optimizer. Its per-input settings (genre, prevent input delay, VRR,
  FreeSync, ALLM, black stabiliser) and the live VRR refresh readout are
  candidates for a quick-panel game card.
- Screensaver designs. Its QML screensavers (clock, starfield, bokeh) keep
  every element drifting to avoid retention; LumaTV's screensaver follows
  the same rule.
- Version on the set predates Glasshouse Home and Remote keys (no
  `home.json`, no `glasshouse.remoteKeys`). Both hook what LumaTV owns: the
  Home key and the first slot of the compositor's key-filter chain. Neither is
  switched on while LumaTV runs; an update of Glasshouse is checked for
  that.
- Key-filter writes: writing `com.webos.surfacemanager.keyFilters` reloads
  every filter and reruns LG's `init()`, which in Game mode opens the Game
  Dashboard. Glasshouse keeps one fixed filter file in the chain and passes
  its settings through a configd value the filter subscribes to. LumaTV's
  filter does the same and is written to the chain only when absent.
- Apps kept ready (phase 8): preloads are configd
  `com.webos.service.preloadmanager.supportAppList`; on webOS 10 the preload
  manager forgets the stored value at every switch-on, so `setPreloadPolicy`
  is sent again at each wake. Glasshouse already does this; LumaTV uses it.
  On the set: Netflix, YouTube, Prime Video, the browser and LG Channels are
  enabled.
- Retiring LG's Home: tested on the set on 2026-10-22 (webOS 10.2.1). A web
  stub with `appCategory: ["home"]` and `visible: false`, bind-mounted over
  `/usr/palm/applications/com.webos.app.home`; `blockSystemApp` then
  `releaseBlockSystemApp` made SAM report the stub's type within 0.5 s, with
  `blockedSystemAppList/USA.json` byte-identical afterwards and no SAM
  restart. Killing LG's Flutter process left the stub in front 3 s later and
  LumaHome drawing over it. Passed: Home key from YouTube, closing the last
  app, standby and wake, remote focus in LumaHome, volume to the AVR over
  eARC, AVR-S760H still listed on HDMI 2. Home's process went from 66-88 MB
  (Flutter) to 26-29 MB (one WebAppMgr renderer). Reverted by unmounting, the
  same block and release, and WebAppMgr `killApp`; LG's Home was back in 2 s.
  Moves into phase 1, with a heartbeat that gives LG's Home back if
  LumaHome stops drawing, a boot hook that mounts before SAM first starts
  Home, and LG's `homeAutoLaunch` held off while it is in place.
- Recent apps: `com.webos.service.recent/listRecents` with snapshots, closing
  with `closeByAppId` (native) or WebAppMgr `killApp` (web) before
  `removeRecent`, since removing the card alone leaves the app running on
  webOS 10. Back on Home opens it.
- Picture details for the input picker and quick panel: the HDMI signal
  (resolution, refresh, HDR10/HLG/Dolby Vision, VRR, ALLM) and the picture
  mode LG keeps for that input and dynamic range.
- Last opened ordering for rows, and a "what was last on" card on Home.

## Safety

- Each surface has its own switch and falls back to LG's; `lumaglass off`
  reverts everything.
- Boot verifies every redirect and restores LG's entry for any that will not
  load; the firmware fingerprint check catches LG rewriting the underlying
  files.
- PIN prompt, modal alerts, input switching and power controls get scripted
  tests before cutover. Picture Mode stays FILMMAKER MODE throughout.

## Decided

- Focus cue: no line under the focused item (Nitin, 2026-10-23, replacing
  the earlier "keep the bright lower edge"). The focused item lifts, its
  glass brightens and a soft accent halo spreads under it; tabs and
  segments, which do not lift, rely on the brighter glass and the halo.
- Dark-mode depth: a step stronger than today's Home, so panes lift further
  off the wallpaper (Nitin, 2026-10-23).
- Home wallpaper motion: about 10 updates a second, so idle Home stops
  redrawing at an uneven 36 fps (Nitin, 2026-10-23). Measured cause
  (`docs/research/perf-tweaks.md`): the wallpaper's 33 ms timer and the
  clock's 160 ms second-hand animation run on separate clocks, adding about
  9.6 frames a second and a 50 ms hitch every 1.5 s. Both move to one
  shared 100 ms clock, with the wallpaper phase taken from elapsed time.
- QML disk cache on, cleared on every stage change (Nitin, 2026-10-23).
- The set is wall-mounted: the strip runs all four edges, the 5 m strip on
  order (Nitin, 2026-10-23).
- Live TV is rarely used: phase 6 restyles the channel banner only; no
  guide (Nitin, 2026-10-23).
- Background preloads: only Netflix stays warm; YouTube (135 MB), LG
  Channels (100 MB) and the web browser (72 MB) are dropped from
  `preloadmanager.supportAppList`, about 307 MB, re-sent at every wake as
  webOS 10 requires (Nitin, 2026-10-23). Phase 8.
- Name: LumaTV (Nitin, 2026-10-23), kept easy to change again. The shown
  name and every internal identifier come from one file (`brand.json`:
  display name, short id, app id, data directory), read by the build, the
  tool, QML and the web theme; nothing hard-codes the name. Phase 1 starts
  with the switch from `lumaglass`: app id `org.nphil.lumatv`, data in
  `/var/lib/lumatv`, repo renamed on the forge, and the tool migrating the
  old data directory and removing the old app on first run.
- LumaTV replaces LG's compositor (Nitin, 2026-10-24): it is the only way
  to meet the 60 fps rule (X1: 51.2 fps, 17 % of frames late for a trivial
  client). Built in stages (track C) beside the surfaces, switched to only
  after passing on the set, with automatic fallback to LG's compositor and
  a re-check after every firmware update.

## Open decisions

- Curated settings list and layout.
