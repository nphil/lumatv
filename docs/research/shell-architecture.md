# LumaTV shell architecture: how much of LG's stack to replace

Evidence-based study (2026-10-23) of replacing LG's stack layer by layer for
performance, stability and full use of the C4's hardware. Set: LG C4
OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, chip O22A3. Every TV reading
here is read-only (`ps`, `/proc`, `/sys`, debugfs, strings of binaries,
read-only DRM ioctls, one `getForegroundAppInfo` call); nothing changed the
screen, power state or picture mode. Probe sources and raw logs:
`docs/research/arch/`.

## Resume here

- Done: stack map, DRM/KMS inventory, per-process cost, Wayland interfaces
  per client stack, luna API inventory and permission census, OSE source
  sizing, options A-E, hardware utilisation, experiments, ranked
  recommendation. The study is complete on offline and read-only evidence.
- Open (needs the TV free; each is listed with its command under
  Experiments): X1 floor of LG's compositor for a client and X1b its
  per-present charge for app frames (in PerfDeepDive's run plan, step 4b;
  results due in its "Compositor floor" section), X2 the 32-bit libmali
  buffer protocol against a private Wayland socket (no screen change), X4
  `ls-monitor` census of compositor luna traffic, X3/X6 (need a
  compositor restart, so a maintenance window with Nitin's OK).
- TV state left: untouched by this study. Read at 19:10: power `Active`,
  picture mode `filmMaker`. At 19:35 LG's screensaver was up over Home.
  No tweak applied, nothing to revert; `/tmp/drmplanes` (the read-only DRM
  tool) is the only file this study put on the set (`rm /tmp/drmplanes` to
  remove; `/tmp` is volatile anyway).
- Tools: `python3 docs/research/arch/tv.py '<cmd>'` runs one command on
  the set (paramiko; `pip install --break-system-packages paramiko` if
  missing); `arch/drmplanes.c` builds with
  `zig cc -target aarch64-linux-musl -static -O2 -o drmplanes drmplanes.c`
  and only issues GET ioctls.

## Verdict in brief

The earlier verdict ("do not replace the compositor: the visible gain is
small") is **overturned on the gain and confirmed on the timing**. LG's
compositor is a single-threaded Qt 5.12 scene graph that redraws the whole
1080p window for every presented frame, serially (CPU, then GPU, then
commit), with LG's JavaScript key filters and luna callbacks on the same
thread. That design is the measured ceiling of today's LumaTV and of any
Wayland client of it, so the gain from owning the compositor is large, not
small. The cost is equally large: 56 luna methods on 12 bus names, a power
handshake with `tvpower`, video placement through `videooutputd`, 9 public
and 3 private Wayland protocol families, and about 37 000 lines of LG TV
behaviour in QML and JavaScript that SAM and the system services rely on.

Ranked path: build the 64-bit Rust UI first as a client of LG's compositor
(option B), in a shape that later runs unchanged inside LumaTV's own
compositor; prove or kill the replacement compositor (option C) with the
seven experiments below, four of which need no compositor restart; adopt C
only if they pass. Option D (a second compositor on a spare plane) is
impossible on this hardware: one CRTC, and DRM leases must take a CRTC
and a connector.

## The stack, layer by layer

| Layer | What LG runs | Measured cost | What depends on it | What replacing it takes |
|---|---|---|---|---|
| Display, KMS planes | `lg1k` DRM driver (1 CRTC, 1 connector, planes: primary osd0, overlay osd1, cursor), LG fbdev (`osd0_fb`, `osd1_fb`, `osd2_fb`, `crsr_fb`), display engine firmware 7356 (video windows w0/w1, FRC, PQ, OSD over video, 1080p to 4K scaling) | hardware; compositor framebuffers 2 x 8.5 MB + 6.7 MB (AFBC) | everything visible | not replaceable: kernel and firmware. Only the DRM master (who drives the planes) can change |
| Compositor | `surface-manager`: Qt 5.12 eglfs, `libWebOSCoreCompositor` (OSE-derived), LG plugins, 37 000 lines of LG QML/JS | 100 MB RSS; 13 % of a core on Home at rest, 23 % under LG's screensaver; per presented frame 2.8-3.4 ms CPU + 8.1 ms GPU at idle LumaHome | every app, SAM, bootManager, media pipeline, IME, notifications, tvpower, voice, captures | option C: Wayland core + LG extensions + luna API + power handshake + video placement + input routing (sized below) |
| Window and app management | `sam` (2.5 MB binary) | 5.7 MB RSS, 0.2 % of a core | launching, lifecycle, preloads, Home, recents | not worth it and not safe: restarting SAM on webOS 10 has killed eARC/CEC until a power cut |
| Web runtime | `WebAppMgr` (`libcbe.so`, 150 MB Chromium) | 11 MB + renderers (6-7 MB each idle); LG Settings +93 MB while open | third-party web apps, LG Settings, Notification Centre, the Home stub | not the runtime; LumaTV replaces the surfaces drawn in it (Settings, Notification Centre) |
| LG Home | `flutter-client` (Flutter 3.22.2, eLinux Wayland) | 35.5 MB RSS here, 66-88 MB in earlier runs; 0 % CPU while hidden | Home key, SAM's Home slot | already planned: stub registered as Home (26-29 MB web stub tested; a native stub would be a few MB, estimated) |
| Media pipeline, video planes | uMediaServer, `starfish-media-pipeline`, `videooutputd`, decoders, DE video windows, secure video memory (54 + 83 MB SVP regions), HDR/DV in the PQ engine | `videooutputd` and the pipeline 0 % at rest | every app's playback, DRM, HDCP, HDR, Dolby Vision, Filmmaker | never: protected content lives here. Any compositor only places the window through `videooutputd` |
| Input | `lginput2` (IR decode, BLE Magic Remote, voice key, network keys) creating uinput devices | 6.1 MB, 3.6 % of a core, about 2 000 wakeups/s (SCHED_RR 99, PerfDeepDive) | the compositor reads its evdev nodes | no: a compositor reads the same nodes; lginput2 stays |
| Audio, eARC, CEC | `audiooutputd`, `arccontroller`, `extinput-integration`, `eim`, `avmutemanager`, CEC services | `arccontroller` 0.6 % | the Denon over eARC, CEC power and volume | never: no gain, and SAM restarts have broken it |
| Settings, configd | `SettingsService`, configd, mojodb (x3) | 9.5 MB, 0.4 % | every setting LG's processing reads | no: LumaTV writes through it (Settings plan) |
| LG daemons | `pqcontroller`, `iconnectivity`, `servicemanager`, `memchute`, `sdx`, `legacy-broadcast-atsc`, `ss.apiadapter`, `amazon-alexa-adapter`, voice preload, `admanager`, `acr2`, `uploadd`, `contentminer` and more | at rest: `pqcontroller` 8.4 %, `iconnectivity` 3.2 %, `servicemanager` 2.2 %, `memchute` 2.2 %, `sdx` 1.5 %; RAM: Alexa 41 MB, voice 48 MB, `ss.apiadapter` 33 MB, `iconnectivity` 25 MB, `pbs` 23 MB | features (picture processing, ThinQ, Alexa, voice, Live TV, ads) | not replaced by LumaTV code; unused ones are switched off through Glasshouse, per feature, as Nitin decides |

Two conclusions follow from the table. Below the compositor nothing is
replaceable or worth replacing: the display engine, media pipeline, SAM,
input, audio and settings either hold protected content, have no
measurable cost, or break eARC/CEC when restarted. The compositor is the
only layer where LG's code is both expensive and replaceable, and the LG
daemons are a separate memory and CPU question answered by switching
features off, not by writing code.

## Options

### A. Today: LumaTV QML inside LG's compositor

- **Performance ceiling:** LG's compositor itself. Qt Quick redraws the
  full window for every presented frame; the basic render loop runs QML,
  JavaScript, luna callbacks, the scene graph and GL on one thread, and
  the thread blocks for the GPU pass. Idle LumaHome uses about 12 of the
  16.7 ms (2.8-3.4 ms CPU; 8.1 ms GPU, of which 7.5 ms is the fragment
  pass; PerfDeepDive), so an animation has about 4.7 ms of headroom before
  a frame is late, and a burst of JavaScript (the 22-handler key-filter
  chain, a view transition, a luna reply) lands on the frame path. The
  known misses (57-87 ms gaps returning to Home) are this shape. Rendering
  runs through LG's 32-bit driver; the 64-bit one measured about 36 % less
  CPU per draw.
- **Work:** built; phases continue.
- **What breaks:** a LumaTV QML stall stalls every app, because it is the
  compositor's thread; a QML crash restarts the compositor and every app
  with it.
- **DRM, HDR, Dolby Vision:** untouched; LG places video as before.
- **Firmware updates:** LG can change the QML LumaTV shadows; the
  firmware fingerprint check falls back to LG's surface.
- **Fallback:** `lumaglass off`, per surface.
- **Lift available inside A:** the threaded render loop (PerfDeepDive
  T7) would overlap CPU and GPU if LG's QML tolerates it; presenting only
  on change (decided: wallpaper at 10 updates a second).

### B. The 64-bit Rust UI as Wayland clients of LG's compositor

- **Performance ceiling:** LumaTV's UI leaves LG's thread: its own render
  thread, the 64-bit driver, damage-aware partial redraw (LumaShow's
  renderer draws nothing for a still screen and only damaged regions
  otherwise), no JavaScript. But each LumaTV present still costs LG's
  compositor a full recomposite (it ignores client damage), and the frame
  reaches the panel only when LG's thread composites it, so LG's jitter
  stays on the path. Measured: a full-screen 64-bit client reached
  60.2-60.8 fps with 2-10 frames over 20 ms in 8 s (contiguous buffers);
  the gate allows none. Whether those late frames are the client's or LG's
  compositor's is not yet separated (X1).
- **Memory:** contiguous buffers are what make it fast: three 1080p
  dumb buffers take 25 MB of CMA, with 45-50 MB free on the set; the
  ordinary-page fallback costs about 4 fps and 35-41 late frames per 8 s.
  This is on top of LG's own framebuffers, because LG still composites.
- **Work:** medium. LumaShow's `ui` (45 500 lines) and renderer exist; new
  are a Wayland-window scan-out route in the renderer, the heap shim
  (written), LG window types and lifecycle (overlay proven; card windows
  for apps registered with SAM), key delivery through LG's focus and key
  filter, and each surface rebuilt in the toolkit.
- **What breaks and how:** a hung LumaTV client leaves LG's compositor and
  every app running; LumaTV's surface falls back to its QML version.
  Isolation is strictly better than A.
- **DRM, HDR, Dolby Vision:** untouched; LumaTV windows are transparent
  where video shows, as LG's own overlays are.
- **Firmware updates:** the `wl_webos_*` protocols are versioned and
  public (webOS OSE), so the client side is stable. The 64-bit blob ships
  with LumaTV and depends only on `mali_kbase` speaking kernel interface
  11.40; a kernel update that changes it makes the 64-bit path fail to
  start, and LumaTV draws through LG's 32-bit driver or falls back to QML.
- **Fallback:** A, per surface.

### C. A Rust compositor replacing `surface-manager`

- **Performance ceiling:** the highest of all options, for four reasons.
  1. LumaTV's UI is drawn in the compositor: no second composition pass.
  2. Composition follows damage: a still screen costs nothing, a focus
     move redraws a tile, not the panel. LumaShow's compositor on the Echo
     Show (Mali-T720 at 299 MHz, A53 at 600 MHz, far weaker than the C4)
     holds 59.64 fps with no missed refresh over 60 s, 1.9 ms of CPU per
     frame with direct scan-out, and 0 CPU ticks at rest.
  3. Direct scan-out: when one full-screen app is visible (Netflix,
     YouTube, Plex browsing), its buffer goes onto the primary plane with
     an atomic commit and no GPU pass. Today every app frame costs LG's
     compositor a full recomposite: plx-native measured LG's charge at
     23.8-26 % of a core per 60 fps of presents on a webOS 4.5 set
     (Mali-T820), larger than the app itself; on the C4 the screensaver
     alone keeps it at 23 %.
  4. Threads: Wayland dispatch, luna and input run beside the render
     thread, so a slow luna reply cannot make a frame late.
- **Work:** very large. Sizing from the sources:
  - Wayland core from Smithay (Rust): `wl_compositor`, `wl_surface`,
    `wl_region`, `wl_shm`, `wl_seat`, `wl_output`, `wl_subcompositor`,
    `wl_data_device`, `wp_presentation`, `zwp_linux_dmabuf_v1`. Added by
    hand: `wl_shell` (every LG client still uses it), `wl_drm` (Mesa's
    public protocol) and possibly `mali_buffer_sharing` (ARM-private, no
    public XML): LG's 32-bit `libmali` references only those two for
    window buffers, not `zwp_linux_dmabuf_v1` (X2 decides).
  - webOS extensions: 9 public XML files, 84 requests and 49 events
    (`wl_webos_shell`, `wl_webos_foreign` with punch-through,
    `wl_webos_surface_group`, input manager, xinput, `input_method` and
    `text_model` for the keyboard, system UX), plus LG-private
    `wl_starfish_output`, `wl_starfish_pointer` (WAM) and Qt's `qt_*`
    extensions (LG's Qt apps). OSE implements the public ones in 18 600
    lines of C++ on top of Qt's compositor library.
  - Behaviour: the subset of LG's 37 000 lines of QML/JS a TV needs:
    window types and the view state machine (card, overlay, popup,
    keyboard, screensaver, notification, volume, spinner), key routing
    with per-app key masks, `getForegroundAppInfo` and recents for SAM,
    `setPinOverlayView` for parental PINs, punch-through and
    `videooutputd` `setDisplayWindow`/`setCropRegion`/z-order/mute per
    video context, the `tvpower` `power2` prepare-power-off,
    prepare-power-on and power-off-logo handshake, screen capture,
    cursor through `libim`, the boot-logo hand-over. Estimate 20 000 to
    40 000 lines of new Rust; for scale, LumaShow's compositor reached
    54 000 lines in 174 commits over 11 days. Code volume is not the
    bottleneck; on-TV verification of LG behaviour is.
- **What breaks and how:** gaps show up in specific flows, not at
  start-up: an app that never becomes visible because SAM waited for a
  foreground event, video black or misplaced because a context was not
  registered, a PIN prompt that never appears, standby that stalls because
  `tvpower` waited for `responsePreparePowerOff`. A crash takes every app
  down, as an LG compositor crash does today. SSH and a plain `reboot`
  stay available throughout, so recovery never needs the remote.
- **DRM, HDR, Dolby Vision:** protected video never passes through any
  compositor: it is decoded into secure memory and shown on a display
  engine video window. A replacement places the window with the same
  `videooutputd` calls and never reads video, so copy protection is
  neither touched nor bypassed. HDR and Dolby Vision are the PQ engine's;
  LG's compositor QML contains no HDR logic (no `hdr`, `dolby` or
  luminance handling in its 180 QML and JS files).
- **Firmware updates:** everything LG-private must be re-verified on each
  update (SAM's expectations, `libmali`'s client protocol, `wl_starfish_*`,
  power handshake). The fingerprint check must start LG's compositor
  whenever the firmware differs from the verified one.
- **Fallback:** LG's `surface-manager` at the next compositor start. The
  swap is a bind mount set by the early boot hook; removing it and
  restarting through the tool's restart path (which already restarts the
  compositor on every boot) restores LG's.

### D. A Rust compositor on spare KMS planes beside LG's

Not possible as a separate compositor. `lg1k` has one CRTC and one
connector, `surface-manager` is DRM master and the only DRM client, and
atomic commits are master-only. A DRM lease must contain at least one CRTC
and one connector (kernel `validate_lease`), so leasing plane 34 would
take the whole display from LG. The overlay plane is also not spare: LG
allocates it for its secondary window (osd1, 768x2160, flipped once since
boot). Three workarounds exist and none is worth building:

- **D1, LG's own secondary window.** `WEBOS_COMPOSITOR_EXTRA_WINDOWS`
  (an environment variable, settable in LumaTV's
  `surface-manager.env`) puts a second QML window on osd1 with its own
  geometry and QML entry point. It is still LG's compositor drawing it, on
  the same thread. Useful only as the plane-split experiment E2.
- **D2, a shim inside `surface-manager`** that adds plane-34 properties
  to LG's atomic commits: races LG's own commits and crashes LG's process
  when it fails.
- **D3, `/dev/fb1` directly:** the same osd1 hardware behind LG's DRM
  driver; undefined ownership.

### E. Other uses of the hardware

- **E1, hardware cursor: already LG's.** The pointer is drawn by LG's
  `libim` (`IM_CURSOR_DRAW_Layer_Enable`, `IM_CURSOR_SURFACE_SetPosition`)
  on its own cursor layer from the `tCursor` thread, not in the scene
  graph, so moving it does not redraw the UI. Nothing to gain.
- **E2, split wallpaper and UI across the two OSD planes** (through D1 in
  A, or natively in C): a UI animation would stop redrawing the
  full-screen wallpaper, and a wallpaper update would stop redrawing the
  glass. Glass samples the baked blur texture, not the plane below, so
  the split keeps the look. Unknown: whether osd1 scales (its 768x2160
  configuration suggests it runs at native 4K, unscaled), its stacking
  order, and whether LG paces two windows independently (X6).
- **E3, Home's motion wallpaper as a looping video on a hardware video
  window:** decode and scaling cost no GPU or CPU, and the OSD above it is
  transparent. LG's own Flutter Home already plays video on Home
  (`libvideo_player_webos_plugin.so`). Costs: a decoder and CMA while Home
  is up, released on every app launch; glass over it becomes tinted glass
  without blur, as it is over video now. A design choice.
- **E4, present less:** the charge per present is fixed in LG's
  compositor, so every surface presents only when it changes. plx-native's
  presenting gate cut LG's compositor from 24.2 % to 0.62 % of a core on
  webOS 4.5. Applies to A and B now.
- **E5, a native Home stub** instead of the web stub: the tested stub is
  one WebAppMgr renderer (26-29 MB); a native app registered with
  `appCategory: home` that draws nothing would be a few MB (estimated:
  one small process registered with SAM, no renderer).
- **E6, LG daemons off** where Nitin does not use the feature. Candidates
  measured at rest: `iconnectivity` 3.2 % of a core and 25 MB, `sdx`
  1.5 % and 14 MB, `legacy-broadcast-atsc` (Live TV) 1.0 % and 11 MB,
  `ss.apiadapter` 0.8 % and 33 MB, the Alexa adapter 41 MB, the voice
  preload 48 MB: about 6.5 % of a core and 170 MB together, each needing
  a dependency check first. `pqcontroller` (8.4 %, the picture
  processing), `servicemanager` and `memchute` are core services and stay.
- **E7, a 4K UI on osd1:** rejected; PLAN's viewing-distance arithmetic
  holds, and a full 4K buffer does not fit the free CMA.

### Comparison

| | A today | B Rust clients | C own compositor | D own plane |
|---|---|---|---|---|
| Frame path | LG thread: JS + scene + GPU, serial | ours, then LG's full recomposite | ours, damage-aware, threaded | not possible |
| LumaTV frame cost | full redraw each frame (8.1 ms GPU at idle Home) | partial redraw + LG's full recomposite | partial redraw only | - |
| App frames | LG recomposites each | LG recomposites each | direct scan-out when alone | - |
| Can meet "no frame over 20 ms" | only if LG's thread allows (T7) | only if LG's thread allows (X1) | designed to (LumaShow: 0 missed in 60 s on weaker hardware) | - |
| Work | built | medium | very large | - |
| Risk to apps, video, eARC/CEC | low | lowest | high until verified | - |
| Firmware updates | QML fingerprints | stable public protocols | re-verify every update | - |

## Hardware utilisation

| Resource | What it is | A | B | C |
|---|---|---|---|---|
| CPU | 4x Cortex-A76, one OPP at 1.4 GHz, `performance` governor, CPU3 hot-plugged by LG's PM driver | LG's single GUI thread does LumaTV's logic and rendering (2.8-3.4 ms per frame at idle) | LumaTV on its own threads, 64-bit (36 % less CPU per draw); LG still spends CPU per present | render thread plus protocol and luna threads; app frames cost a commit |
| GPU | Mali-G52 MP3 r1p0, `coarse_demand`, clock not exposed | full redraw every presented frame | LumaTV partial redraw plus LG's full-screen recomposite | partial composition; zero for direct scan-out |
| OSD planes | primary (osd0, 1080p AFBC, scaled to 4K), overlay (osd1), cursor | LG: primary, osd1 for its secondary window, cursor via `libim` | same; LumaTV cannot reach planes | primary for composition or direct scan-out, osd1 for overlays if it scales (X6), cursor |
| AFBC | Arm frame-buffer compression, modifier `0x0800000000000062`, on both OSD planes | LG's framebuffers use it | LumaTV's shim buffers are linear (more bandwidth) | LumaTV renders AFBC if the 64-bit blob allocates it (X5) |
| Video windows, scaler, FRC | DE windows w0/w1 (PIP and PBP), 4K scaler, frame-rate conversion | LG's pipeline; HDMI live preview on Home planned | same | same, placed by LumaTV's calls to `videooutputd` |
| Video decode | hardware decoders (HEVC, VP9 and others) behind uMediaServer | apps | apps | apps; optional wallpaper video (E3) |
| PQ engine | HDR10, HLG, Dolby Vision, Filmmaker processing, panel care, own 8 MB net (`g_fw_dnn`) | untouched | untouched | untouched; never reads video |
| Histogram | `libvtcapture` zone statistics | ambient light (PLAN) | same | same |
| GFX 2D blitter | LG's GAL engine, own 79 MB region (cursor images, thumbnails) | LG's | LG's | not useful: no public API, the GPU does more |
| NPU | LG DNE, 0.4 TOPS, idle in Filmmaker | unused | unused | unused; its uses (ambient scene class, voice) are independent of the shell |

## Experiments

Each risky step has one experiment that proves or kills it. X2 shows
nothing on screen; X1, X1b and X4 use the screen but need no compositor
restart; X3, X5 and X6 restart the compositor.

| Id | Question | Method | Kills | Proves |
|---|---|---|---|---|
| X1 | Is LG's compositor the floor that blocks the gate for a client? | A trivial full-screen 64-bit client (the m64 probe, dumb buffers, one small moving rect) presenting at 60 Hz for 60 s over Home with wallpaper motion off; count presented frames over 20 ms with PerfDeepDive's `drm_vblank_event_delivered` harness; repeat with the threaded loop (T7). Requested from PerfDeepDive. | B as the end state if late frames persist with a trivial client (then only C meets the rule) | B as the end state if zero late frames |
| X1b | What does LG charge per app frame? | `surface-manager` CPU per presented frame and GPU busy during YouTube, still versus animating UI (PerfDeepDive's video workload). | C's direct scan-out gain if the charge is small | the gain if it is a large share of a core |
| X2 | Can a non-LG compositor receive LG's 32-bit EGL buffers? | A Rust Wayland server on a private socket (no DRM, nothing on screen) offering `wl_drm` and `zwp_linux_dmabuf_v1` but not `mali_buffer_sharing`; a 32-bit test client linked against LG's `libmali` with `WAYLAND_DEBUG=1`; import the received dma-buf into the 64-bit blob and read back pixels. Then `eglBindWaylandDisplayWL` from the 64-bit blob, to see whether it serves `mali_buffer_sharing` to 32-bit clients. | C if neither works and `mali_buffer_sharing` cannot be reproduced | C's buffer path |
| X3 | What do the real apps use of the protocol? | `WAYLAND_DEBUG=server` in `surface-manager.env`, compositor restart through the tool, a scripted session (Netflix, YouTube, Prime, Disney+, PlxNative, LG Settings, keyboard, Live TV), env line removed, restart. Needs a maintenance window. | nothing on its own | the exact request and event subset C must implement |
| X4 | Which luna methods are live? | `ls-monitor` filtered to `com.webos.surfacemanager*`, `sysuicompmgr`, `systemUIManager`, `service.recent`, `coverUI` and `tvpower` `power2` during the same scripted session plus standby and wake; read-only. | nothing on its own | the method subset and the power handshake timing |
| X5 | Can app buffers be scanned out directly? | In the C prototype: `ADDFB2` with the client dma-buf and modifier, test-only atomic commit (`DRM_MODE_ATOMIC_TEST_ONLY`) before any real one. | direct scan-out (C keeps the other gains) | the app-frame gain |
| X6 | Does osd1 scale, and where does it stack? | D1: a test QML on the secondary window through `WEBOS_COMPOSITOR_EXTRA_WINDOWS` at 1920x1080, compositor restart through the tool, capture, revert. Maintenance window. | E2 if osd1 is 4K-native only | E2, and osd1 overlays for C |

Running X1, X1b, X4 needs only the TV free. X3, X5, X6 restart the
compositor, which restarts every app; the mod already does this on every
boot, and SSH plus `reboot` recover from any state, but each run is
followed by a check that the Denon still follows volume over eARC and
that CEC still lists it on HDMI 2.

## Evidence log

### DRM/KMS inventory (read-only, 19:13)

`arch/drmplanes` on `/dev/dri/card0`: driver `lg1k 1.0.0 (20210215) "LG
Graphics Output Driver"`; atomic and universal planes accepted, PRIME
import and export (cap 3), `ADDFB2_MODIFIERS` yes, `ASYNC_PAGE_FLIP` no,
cursor 128x128, no syncobj; one CRTC (38), one encoder (39), one connector
(31, type TV, connected, modes 1920x1080, 3840x2160 and 1280x720 at 60 Hz,
all at a 148.5 MHz pixel clock: OSD canvas sizes, not panel timings).
Output: `arch/drmplanes-01.txt`; debugfs state: `arch/drm-debugfs-01.txt`.

No plane exposes `zpos`, `alpha`, `pixel blend mode` or `COLOR_ENCODING`
(debugfs shows the defaults only):

| Plane | Type | Formats | Modifiers | State |
|---|---|---|---|---|
| 32 | primary | AR24 XR24 AB24 XB24 | linear; AFBC `0x0800000000000062` (32x8, split, sparse) for AB24/XB24 | fb 40, 1920x1080 AB24 AFBC, full CRTC |
| 34 | overlay | AR24 XR24 AB24 XB24 | same | no CRTC, no fb at the time of reading |
| 36 | cursor | AR24 | none | no CRTC, no fb at the time of reading |

`surface-manager` (pid 7581) is the only DRM client and the master. It owns
three framebuffers, all AFBC AB24 and `imported=yes` (dma-bufs from the
Mali driver, not dumb buffers): 40 and 43 are 1920x1080 (double-buffered
osd0), 44 is 768x2160 (osd1, its secondary output), not attached to a plane
at the time. `plane_mask=1`: the CRTC scans out the primary plane only.
Video is not in DRM at all (no YUV formats, no video planes): it runs
through the display engine's video windows under the OSD.

LG's fbdev driver (`/proc/lg/fbdev`) exposes the same hardware as four OSD
framebuffers: `fb0` (osd0, 1920x1080 AFBC 32x8, 2 buffers, 173 835 flips
since boot), `fb1` (osd1, 768x2160 AFBC, `max_xres` 512 by default and 768
by LG's `WEBOS_COMPOSITOR_EXTRA_WINDOWS`, 1 flip since boot), `fb2` (osd2,
128x4), `fb3` (cursor, 256x256 ARGB). `/proc/lg/fbdev/status` shows the
cursor path mapping a 1920x1080 source to a 3840x2160 destination: the OSD
is composed at 1080p and scaled by the display engine. The GFX block
(`/proc/lg/gfx`) is a 2D blitter with its own 79 MB region (33 MB
allocated: two 256x256 cursor images and one 384x216 thumbnail). The
display engine (`/proc/lg/de/status`, firmware 7356 of 2025-04-14) holds
the video path's memory in its own regions: `g_de_svp_m1` 54 MB and
`g_de_svp_m2` 83 MB secure video, `g_be_frc0` 33 MB frame-rate conversion,
`g_be_fpp` 19 MB, `g_fw_dnn` 8 MB (the PQ engine's own net); two video
windows `w0`, `w1` (PIP and PBP capable; sources avd, adc, hdmi, vdec,
jpeg; HDR mode per window: sdr, h10, dovi, hlg).

Device holders (`arch/devholders-01.txt`): only `surface-manager` holds
`/dev/dri/card0`; `pqcontroller`, `eplmanager`, `captureservice`,
`gal_renderd` and `nn-proxy` hold `/dev/fb0`, `fb1` and `fb3` (histogram,
panel care, capture); `/dev/mali0` is held by `surface-manager`,
WebAppMgr, `flutter-client`, cobalt, `inputcommon`, `gal_renderd` and
`nn-proxy`.

### Hardware (read-only, 19:22, `arch/hw-01.txt`)

- CPU: 4x Cortex-A76 (part 0xd0b), online 0-3, policy0 `performance` at
  1 400 000 kHz, the only frequency listed; `/proc/lg/pm` reports
  `cpu_num: 3`, 52 °C. Features include `asimddp`, `fphp`, `atomics`.
- GPU: Mali-G52 3 cores r1p0, `coarse_demand`, no devfreq node, all three
  cores in every job-slot mask.
- Memory: 2 030 392 kB, 81 MB free, 778 MB available; CMA 692 MB total,
  49.8 MB free, 258 MB device-allocated (45 MB free at 19:35).
- `/proc/stat`'s aggregate `cpu` line is unreliable on this kernel: its
  idle counter jumped by 62 118 ticks in 10 s (CPU3 hot-plug), so
  per-process figures below use wall-clock intervals.

### Process cost (19:17, 36 s, Home showing)

`arch/proccost-30s.txt`, raw `arch/procsnap-a.txt`/`-b.txt`. A sibling
agent's `grep` pinned one core during the window (87.9 %) and drove
`kswapd0` to 10 %, so these are upper bounds for a resting Home.
"Switches/s" counts the main thread's context switches.

| Process | CPU (% of one core) | Switches/s | RSS | Threads |
|---|---|---|---|---|
| surface-manager | 13.1 | 167 | 100.5 MB | 21 |
| pqcontroller | 8.4 | 0 | 4.4 MB | 37 |
| lginput2 | 3.6 | 0 | 6.1 MB | 13 |
| iconnectivity | 3.2 | 0 | 25 MB | 56 |
| systemd | 2.6 | 70 | 8.5 MB | 1 |
| servicemanager | 2.2 | 60 | 3.2 MB | 1 |
| memchute | 2.2 | 26 | 2.4 MB | 6 |
| ls-hubd | 1.8 | 44 | 7.7 MB | 5 |
| sdx | 1.5 | 11 | 13.6 MB | 12 |
| legacy-broadcast-atsc | 1.0 | 0 | 11.4 MB | 86 |
| ss.apiadapter | 0.8 | 13 | 33 MB | 13 |
| WebAppMgr | 0.2 | 7 | 11 MB | 35 |
| sam | 0.2 | 5 | 5.7 MB | 4 |
| flutter-client (LG Home) | 0.0 | 0 | 35.5 MB | 18 |

At 19:35, with LG's screensaver (a Flutter app) animating over Home,
`surface-manager` used 228 ticks in 10 s, 22.8 % of a core, with 540
main-thread switches a second (`arch/fg-sample-01.txt`).

`surface-manager` threads since boot (`arch/sm-libs-threads.txt`): main
62 790 ticks, `tCursor` 18 064 (LG's pointer thread, moving the cursor
layer), `mali-cmar-backe` 3 757, `QQuickPixmapReader` 580, `FM-watchdog`
432, `tInput` 396. `WEBOS_GL_DISABLE_THREADED_RENDERING=1`: QML, scene
graph and GL on one thread. Mapped: `libmali.so.0.46.0` 52.9 MB,
`libicudata` 29 MB, Qt 5 Core/Gui/Qml/Quick 14.6 MB,
`libQt5WaylandCompositor` 0.9 MB, `libWebOSCoreCompositor` 0.65 MB,
`libStarfishInput` 204 kB, `libim` 120 kB, `libvt` 87 kB.

Resident apps at rest (`arch/ps-01.txt`): Netflix 72 MB (13 % CPU while
preloading), YouTube 87 MB, Prime 56 MB, voice 48 MB, browser 43 MB, Alexa
adapter 41 MB, LG Home 35 MB, `ss.apiadapter` 35 MB, Glasshouse's node
31 MB, `iconnectivity` 25 MB, `pbs` 23 MB, Homebrew service 19 MB.

### Wayland interfaces per client stack (19:20)

From the `*_interface` symbols of the libraries each app maps
(`arch/wayland-globals-libs.txt`, `arch/app-libs-01.txt`). Every app
reaches the compositor through LG's shared libraries, not its own code:

| Client | Stack | Interfaces |
|---|---|---|
| Netflix (`libwebosPlugin.so.1`, `libgm-wayland`) | own EGL window | `wl_compositor`, `wl_shell`, `wl_seat` (keyboard, pointer), `wl_shm`, `wl_region`, `wl_webos_shell`, `wl_webos_foreign`/`exported`/`imported`, `wl_webos_input_manager`, `wl_webos_surface_group` |
| YouTube (cobalt), Prime (`libgm-wayland`, `libwayland-input-helper`) | own EGL window | `wl_compositor`, `wl_shell`, `wl_seat`, `wl_region`, `wl_webos_shell`, `wl_webos_foreign`, `wl_webos_surface_group` |
| WebAppMgr (`libcbe.so`, `libwayland-webos-client`, `libwayland-starfish-client`) | Chromium Ozone | the above plus `text_model_factory`, `input_method`, `input_panel`, `wl_webos_seat`, `wl_webos_xinput`, accelerometer, gyroscope, `wl_starfish_output`, `wl_starfish_pointer` |
| LG Home (`libflutter_elinux_wayland.so`) | flutter-elinux | `wl_compositor`, `wl_shell`, `wl_seat` (keyboard, pointer, touch), `wl_shm`, `wl_output`, `wl_data_device_manager`, `wp_presentation`, `text_model_factory`, `input_panel`, `wl_webos_shell`, `wl_webos_foreign`, `wl_webos_input_manager`, `wl_webos_surface_group` with layers, `wl_webos_xinput` |
| LG Qt apps (`libQt5WaylandClient` 5.12.4, `libwebos-platform-interface`) | Qt Wayland | `wl_shell`, `wl_subcompositor`, `wl_shm`, `wl_data_*`, `qt_hardware_integration`, `qt_surface_extension`, `qt_windowmanager`, `qt_server_buffer`, `qt_touch_extension`, `zwp_text_input_v2`, and the `wl_webos_*` set |
| Window buffers (`libmali.so.0.46.0`, every GL client) | Mali EGL | `wl_drm`, `mali_buffer_sharing`, `zwp_linux_explicit_synchronization_v1`, `zwp_linux_surface_synchronization_v1`, `zwp_linux_buffer_release_v1`; no `zwp_linux_dmabuf_v1` (the server offers dmabuf v3, which the 64-bit probe used) |

Video is never a Wayland buffer: Netflix, cobalt, Prime and WebAppMgr map
`libumedia_api`, `libums_connector` and `libwaylandpipeline-1.0`. An app
exports a window (`wl_webos_foreign.export_element`, answered with a
window id such as `_Window_Id_12`), passes the id to the media pipeline,
the pipeline imports it and attaches punch-through with its video context,
and the compositor tells `videooutputd` where that context's window is
(`display/setDisplayWindow`, `setCropRegion`, z-order and mute per
context; OSE `webosforeign.cpp`, LG `StarfishFullscreenContainer.qml`).

Protocol size (webOS OSE `webos-wayland-extensions`): 9 XML files, 84
requests and 49 events: `webos-shell` 8/6, `webos-foreign` 16/6,
`webos-surface-group` 14/3, `webos-input-manager` 6/4, `webos-extension`
4/2, `input-method` 15/12, `text` 16/12, `webos-system-ux` 5/3,
`webos-tablet` 0/1. `wl_drm` is Mesa's public protocol;
`mali_buffer_sharing` (ARM) has no public XML. LG's `wl_starfish_*` are
tiny (`arch/starfish-proto-strings.txt`): `wl_starfish_pointer` has
`set_cursor_position(uu)`, `set_mrcu_standby_timer`,
`get_mrcu_standby_timer` and the event `timer_value`;
`wl_starfish_output` has `set_stereoscope(uuo)` and `stereoscope_hint`.

### The compositor's luna API (LG's QML, `uiinv/qml`)

| Bus name | Methods |
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

56 methods on 12 bus names. Named callers (strings,
`arch/sm-callers-01.txt`): `sam` uses `getForegroundAppInfo`,
`getRecentsAppList`, `setPinOverlayView` and `coverUI/getStatus`; LG Home
uses `systemUIManager/launch`, `closeByAppId` and `recent/listRecents`;
`libStarfishServiceIntegration` (LG Qt apps) uses
`input/getKeyInputBlockState` and `systemUIManager/getForegroundAppInfo`.

WebAppMgr names none of these methods (`libWebAppMgr*`,
`arch/wam-callers-01.txt`): its contract with the compositor is Wayland
window properties, eight window types (`_WEBOS_WINDOW_TYPE_CARD`,
`OVERLAY`, `FLOATING`, `POPUP`, `RESTRICTED`, `TOPUI`, `SYSTEM_UI`,
`SCREENSAVER`) and access policies (`_WEBOS_ACCESS_POLICY_KEYS_BACK`,
`KEYS_EXIT`, `KEYS_GUIDE`, `TOASTS`, `POPUP`, `ALERTS`), all of which a
replacement compositor must honour.

Permission census (`/usr/share/luna-service2`, `arch/sm-perm-02.txt`): the
compositor-specific groups (`launcher.*`, `surfaces.*`) are granted to
Home (three variants), `com.webos.applicationManager` and
`com.webos.bootManager`; system services also reach these methods through
the generic `private` group, so only X4 gives the live set.

The compositor is itself a client of about 25 services: subscriptions to
bootManager, alwaysready, eim, irdbmanager, multiviewcontroller,
rollingscreen, settingsservice, tvpower, utp.channel, factorymanager,
devicereset, msc and homelaunchpoints, and calls to videooutput,
applicationManager, notification, tts, panelcontroller, capture, audio,
airplay and voiceagent. It takes part in `tvpower`'s power state machine:
`power2/registerPreparePowerOff`, `registerPreparePowerOn`,
`registerPowerOffLogoRequest`, answered with the matching `response*`
calls (`StarfishPowerService.qml`).

### Input (`arch/input-devices-01.txt`, `arch/cursor-strings.txt`)

`lginput2` holds `/dev/uinput` six times and creates the devices the
compositor reads: `LGE RCU` (IR), three `LGE M-RCU - Builtin` (Magic
Remote, with pointer axes), `LGE TONE+`, `LGE W-RCU`, `LGE Simple
Premium`, `IoT keypad`, `Smart Remote RCU Input`, `LGE Network Input`,
`LGE Smart Remote - TouchPad`, `ClickableMouse`, `Bluetooth-audio-source`.
`surface-manager` opens `/dev/input/event0-13` itself
(`QT_QPA_EGLFS_DISABLE_INPUT=1`; `libStarfishInput` reads evdev) and
`/dev/uinput` once. The pointer image is LG's `libim` cursor layer
(`IM_SetCursorDrmDevice`, `IM_CURSOR_DRAW_Layer_Enable`,
`IM_CURSOR_SURFACE_SetPosition`), outside the scene graph.

### Reference sizes

- webOS OSE `luna-surfacemanager` (Apache 2.0): 101 C++ files, 18 613
  lines (`weboscorecompositor.cpp` 1 657, `webossurfaceitem.cpp` 1 912,
  `webosforeign.cpp` 1 796, `webossurfacegroup.cpp` 598,
  `webosshellsurface.cpp` 483, input method and text model about 1 400),
  plus 120 QML/JS files, 11 151 lines, on top of Qt's compositor library
  (core protocols, `wl_shell`, `qt_*`, buffer integration).
- LG's QML on the set: `WebOSCompositorBase` 67 QML + 2 JS, 6 850 lines;
  `WebOSCompositor` 105 QML + 6 JS, 24 892 lines; `KeyFilters` 4 JS,
  5 627 lines: about 37 000 lines, LG-private.
- LumaShow: `crates/compositor` 84 files, 54 330 lines (11 714 of tests;
  GL renderer, glass and blur, damage, pacer, planes route); 174 commits
  between 2026-09-29 and 2026-10-09. `crates/ui` 61 files, 45 506 lines.
  Its protocol is its own (one client, layers as records), not Wayland.

## Ranked recommendation

1. **Now, inside A (no new architecture):** finish the gate work that
   helps every option: present only on change (E4; wallpaper at 10
   updates a second is decided), PerfDeepDive's threaded render loop
   (T7), and the native Home stub (E5, about 20-25 MB back). Keep LG-process
   restyles (Live TV banners, keyboard, voice) in QML under every option:
   they run in LG's own apps.
2. **Build first: the Rust UI as a client of LG's compositor (B).** One
   resident 64-bit process on LumaShow's `ui` and renderer with a
   Wayland-window route, the heap shim and contiguous buffers, drawing
   LumaHome first, then Quick Settings and Settings. Build it so the same
   renderer can later run inside LumaTV's own compositor: the surface
   code never talks Wayland, only the route under it does. This is the
   work C needs anyway, so nothing is lost if C follows. Gate before
   cutover, per surface: PLAN's frame-probe gate over Home and over
   video, plus 4K HDR playback in Netflix and PlxNative with LumaTV's
   buffers allocated (the CMA check).
3. **In parallel, the cheap proofs for C:** X1 and X1b (PerfDeepDive),
   X2 off-screen, X4 with the set free. Decision rule: if X1 shows LG's
   compositor cannot hold zero frames over 20 ms even for a trivial
   client, the 60 fps rule cannot be met under A or B, and C proceeds; if
   it can, B is the end state and C stays a research branch.
4. **C, only after X2 passes and with X3/X4 in hand:** a Rust compositor
   on Smithay and LumaShow's renderer, developed against a private
   Wayland socket first (no screen), then in maintenance windows on the
   set with LG's compositor restored after each run. Order of work:
   buffers and `wl_shell`/`wl_webos_shell` for LumaTV's own windows; the
   `tvpower` handshake and SAM's foreground and recents methods;
   punch-through and `videooutputd` placement; key routing, masks and
   IME; then direct scan-out (X5) and osd1 overlays (X6). Production only
   behind a watchdog that brings back LG's compositor after a crash or a
   stall, and a firmware fingerprint that does the same after any LG
   update.
5. **Rejected:** D (one CRTC; leases need a CRTC and a connector), E7 (4K
   UI), the GFX blitter. E1 is already LG's.

Decisions only Nitin can make:

- Whether the end state may be a replacement compositor that LumaTV
  maintains and re-verifies on every LG firmware update, which in practice
  ties updates to a LumaTV release (firmware updates are his under
  Principle 0).
- Whether new surfaces after phase 1 are built in the Rust toolkit (B)
  instead of QML, which reorders PLAN's phases 2-6.
- Maintenance windows for the experiments that restart the compositor (X3,
  X5, X6, the C trials), each followed by an eARC and CEC check with the
  Denon.
- Which LG features he does not use, so their daemons can be switched off
  (E6): Alexa, voice, ThinQ, Live TV tuner services, AirPlay, Chromecast.
- Home's look over a video wallpaper (E3): tinted glass instead of
  blurred glass, in exchange for a wallpaper that costs no GPU.
- Already open in PLAN: the kernel module for `/dev/dma_heap`, which
  needs LG's GPL source.
