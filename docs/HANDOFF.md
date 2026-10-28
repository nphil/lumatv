# LumaGlass — engineering handoff

Everything a person or model needs to pick this project up cold: how the mod
works, what was measured on the set, which approaches are dead and why, and
how to measure again. Facts here were observed on one set unless marked
[INFERENCE]. Credentials and the set's address are deliberately absent; they
live with the owner.

## Resume here (LumaTV, 2026-10-26 19:05 PDT)

- Source of truth: `docs/PLAN.md` (phases, gates, decisions), `docs/SURFACES.md`
  (inventory), each `docs/research/*-result.md` (its worker's recovery point). An
  implemented file is not proof that its feature passed on the TV.
- TV installs: every LumaTV install on the set goes through Nitin's Face ID
  (HomeLabber `run_command` on beastnas running the installer in the Cody container,
  approved on his phone). A yes in chat does not count (PLAN, Decided). A request that
  expires pauses HomeLabber approvals until Nitin resumes them.
- Helpers: at most 12 at once; briefs forbid spawning. One TV owner at a time,
  deploying from a clean worktree; the compile gate (`tests/qmlhost/compile_all.py`)
  runs before every package or hand deploy. Helpers commit from their own worktrees.
- Installs: HomeLabber standing grant on `sh /mnt/nvme/appdata/cody/home/tmp/lumatv-install/tv-install.sh`
  (installs whatever `/data/home/tmp/lumatv-next` points at; one Face ID per 12 h).
- On the set at 06:30 PDT (10-28): **LumaTV's compositor is live and persistent** on
  forge/main 0f2190c (full package; firmware 10.2.1-5202 in `compositor/firmware/verified.list`),
  LG's compositor the automatic fallback, Active Standby. Every blocking gate check passed
  (round 12); MINOR items are the Optimisation list in `compositor-gate9-result.md`. LG's
  daily reboot came back headless with LumaTV's compositor live (reboot check). On:
  `inputcommon`, `voice`, `remote.quickAccess`, `remote.quickPanelEdit`, keys restyle. Off
  until Nitin decides or a TV check passes: `store` (rebuilt 42050ce, not on the set),
  `keyboard luma`, `cameras.*`, `wallpaper video` (needs `vplane install`), `cover`,
  `splash`, `standbyscreen`, `remote.recents`. venc's first TV probe needs Nitin's OK.
- Done 2026-10-26: compositor gate 9 (every safety check passes, frame timing fails:
  `compositor-gate9-result.md`); compositor step 1, the remote's keys and the keyboard
  under LumaTV's compositor, passed on the set (`compositor-step1-result.md`); quick
  panel and picker round 16 (`shell-host-result.md`): over Netflix and HDMI every panel
  opens in 146-159 ms, over Home p90 2.42/2.54 ms.
- Open, in order (also the todo list):
  1. Installed 3036de9 through Face ID (2026-10-26 23:28 EDT, during Netflix, which kept
     playing): c496611's still mute mark, the Zigbee2MQTT OTA guard, `revert` removing
     `vt_devmode`. Not yet re-measured: the one late frame per panel opening over Home.
  2. Light sync: the Zigbee2MQTT OTA guard and back-off are built (4dd5a5b, offline);
     `vt_devmode` is on for good (Nitin, 2026-10-26). There is no Hue light by the TV:
     Nitin picks which lights follow the picture; WLED strip not set up (PLAN, Ambient
     light, Status).
  3. TV checks: Live TV restyle, Settings look-only walk, frame-rate blink with Nitin
     watching.
  4. Parked by Nitin: compositor frame timing and moving Home and the panels to
     LumaTV's compositor; Music Assistant.
  5. Nitin's steps: GFN sign-in, HA token and plugin enable, Bluetooth proxy link, LG
     store check, leak sensor OTA retry, Denon OSD off, Apple TV CEC off.
- Netflix tvq-pb-101 came from an input app left holding the audio decoder port;
  `lumatv inputs status|release`, and gate.py now closes the HDMI app it opens.
- Backups of uncommitted work: `/data/home/lumatv-backups/wip-*`.

Written so any model can continue cold. The plan is `docs/PLAN.md` (Status,
Phases, Decided); this file holds the proven mechanics.

- State: the build is under way (PLAN.md track C and the surfaces). Proven on
  the set and in the repository:
  - The rename to LumaTV (`docs/research/rename-result.md`), the Home stub
    (`docs/research/homestub-result.md`), the performance work of phase 1
    (`docs/research/perf/summary-2026-10-24-phase1.txt`).
  - Surfaces phase 1 (`docs/research/surfaces-p1-result.md`): fail-open key
    filter, OK on release, the focus rule, wallpaper tint, one token source,
    Manrope, the component kit.
  - Surfaces phase 2 (`docs/research/surfaces-p2-result.md`, which has its own
    Resume here): the startup warm-up, the per-surface frame gate and the
    startup A/B in `docs/research/perf`, the offline QML host
    (`tests/qmlhost`), Home's weather and news from the native helper, the
    helper's start and stop in the tool, the preload trim, the boot and
    power-off logo windows, `lumatv surfaces` (the registry of every switch and
    its way back) and the notification view (`lumatv notifications on|off`: LG's
    tree copied on the set and patched there with LumaTV's own toast laid over
    it, nothing of LG's shipped; `tests/qmlhost/notify.py` runs LG's real view
    offline). LumaTV's Settings (`Settings.qml`, `settings/`: Home & look and AI
    access pages) is a layer of Home opened from the gear's popover, and the AI
    prompt (`AiPrompt.qml`) draws the helper's `ai.prompt` over Home; both are
    tested offline against a fake helper (`tests/qmlhost/fakehelper.py`, which
    models `/v1/ai`) and have not run on the set. The set has run the warm-up and
    the Home gate (slot 1); that note's Log has the numbers.
  - The native helper (`helper/`, `docs/research/native-helper-result.md`):
    proven on the set (smoke test 35 of 35; AI-access smoke 42 of 42, which skips
    the two checks that touch the TV and the receiver). The plugin broker
    (LumaShow's plugin API level 1) is built and host-tested, not yet run on the
    set. Its API is on `127.0.0.1:18731`; files under `/var/lib/lumatv`:
    `log-helper`, `helper.lock`, `helper.json` (optional), `cache/weather.json`
    and `news.json`, `settings/<owner>.json`, `bin/lumatv-helper` (the staged copy
    the tool runs). Stop it with `lumatv helper stop` or `lumatv-helper stop
    --brand <app dir>/brand.json`; if TLS misbehaves set `"fetch":
    {"backend":"curl"}` in `helper.json`.
  - 64-bit drawing (`gfx64/`, `docs/research/gfx64-result.md`): heap shim,
    launcher, pinned blob fetch; smoke run on the set 60.63 fps over idle Home.
    The Rockchip blob bundle is fetched by `gfx64/fetch-blob.sh` (about 37 MB)
    and never committed; an ipk that ships it must run that at build time.
  - LumaTV's own compositor (`compositor/`, `docs/research/compositor-c2-result.md`):
    C2 passed on the set; C3 and C4 built offline, first window and bridge dry
    run on the set. The tool owns the enable path now (`tools/compositor.sh`,
    `lumatv compositor on|off|status`, `lumatv compositor-boot` in the boot
    worker): the bundle itself is put in `/var/lib/lumatv/compositor` by
    `compositor/tools/tv-cutover.sh install` until it ships in the ipk. LG's
    compositor is the fallback on any failure; `lumatv revert` unmounts and
    forgets the flag. Fragility: after LG's compositor was away, `webapp-mgr`
    can be left `failed` and the web Home stub cannot launch (black panel):
    `systemctl reset-failed webapp-mgr; systemctl start webapp-mgr`, then
    launch `com.webos.app.home` (`tools/tv-recover-home.sh` does it, and
    holds `/tmp/lumatv-recover.lock` while it runs; the tool never restarts the
    compositor's unit while that exists).
  - Hardware census (`docs/research/hardware-census.md`), the top five by benefit
    over risk, none run yet and none needing a kernel module: 1 the video tap and
    picture histogram (`libvtcapture`, for ambient light), 2 the TV's own audio
    mix (`hw:1,2`, `hw:1,3`), 3 the hardware JPEG block, 4 hardware video decode
    for a video-plane wallpaper, 5 Bluetooth LE through LG's `bluetooth2` service
    (PLAN.md, Hardware census). LumaShow lessons:
    `docs/research/lumashow-lessons.md` (nice -19 on surface-manager is LG's own,
    nothing to adopt).
  - Declutter (`tools/declutter.sh`, `docs/research/declutter-result.md`): the
    `lean` profile is on and persisted on the set (uploaders, ads, push,
    settings, apps, voice, connectivity, share, misc; `ai`, `livetv` and `cast`
    off). Rebooted clean at 04:08 and checked at 05:15: Netflix, YouTube,
    PlxNative, Apple TV (about 20 s), Crunchyroll and Prime Video launch;
    GeForce NOW, a web app, opens the browser as before; the receiver is
    connected on HDMI 2 with sound on `external_arc`; Home Assistant sees the TV.
- Nitin's decisions of 2026-10-25 (also in PLAN.md Decided): the approved
  mockups in `docs/mocks` are the design, Settings layout included; LumaTV's
  compositor goes live as soon as it passes the C4 gate, LG's staying as the
  automatic fallback; LG's telemetry, ads, promotion and app-push services are
  removed entirely and webOS is trimmed to essentials through two reversible
  profiles, `ads` and `lean` (`docs/research/declutter-result.md`); LG's power-on and
  power-off logos are replaced by LumaTV's, three options each in
  `docs/mocks/logo`, option 1 the default; Prime Video stays unless an LG
  surface clearly pushes it. `lumatv preload on` keeps Netflix and Prime Video
  warm and drops YouTube, LG Channels and the browser from the list; the A/B on
  the set (slot 2) showed no gain, since it closes nothing that runs, so it is
  off until a cold boot shows one.
- Remote tests: done 2026-10-24 for Netflix 4K Dolby Vision playback (X4
  pass 3, contiguous-buffer hold) and the SSAP key device; results in
  PLAN.md. Still open: X4 during PlxNative playback
  (`docs/research/arch/x4/x4-monitor.sh`, `x4-analyse.py`); the histogram
  probe (its ABI was recovered offline, commit dd3c57a). Mute
  the receiver first (`media_player.living_room_avr`) and restore it after.
  Netflix deep link for the 4K Dolby Vision test film: launch `netflix` with
  `contentTarget` `m=https://www.netflix.com/watch/80141336&source_type=4`,
  then ENTER. Protected video captures black; the tests record compositor
  behaviour. During Dolby Vision playback Picture Mode reads `dolbyHdrCinema`;
  it returns to `filmMaker` on Home.
- Tests that need the owner: PIN prompt (a lock is a security setting he
  sets), pointer cost (Magic Remote moved by hand), CEC keys from other
  remotes, receiver volume drift.
- Set rules: confirm it is idle first (`getPowerState`,
  `getForegroundAppInfo`); Picture Mode stays `filmMaker`; never restart
  SAM; write only under `/var/lib/lumatv`, `/tmp`, configd, bind mounts
  and `/var/systemd/system/env`; revert every experiment; leave it in
  Active Standby unless the owner is about to watch; no DRM bypass. Slots on
  the set are asked of the main agent and answered "go <name>".
- Tests: `python3 -m unittest discover -s tests -p 'test_*.py'` (101, about
  55 s; the ones that start the offline QML host need PyQt5 and an X display,
  `tests/xdisplay.py` starts Xvfb itself) and `TZ=America/New_York bun test
  tests/` (36). The frame gate on the set: `python3 docs/research/perf/gate.py`
  (every number and rule is in its docstring); startup and preload A/B:
  `startup.py`, `preload_ab.py` beside it.
- Working style: one step at a time, commit and push after each
  (`git push -q forge main`), results written to files with their own
  "Resume here" note, so a model switch mid-task loses nothing.

## The rule that must not be broken

https://rootmy.tv/warning — **never write to the kernel, rootfs or tvservice
partitions.** A set bricked that way is not recoverable in software.

Everything this project does stays inside that boundary, deliberately:

- Only bind mounts and files under `/var` (writable ext4) and
  `/media/developer`. Never a raw write to a block device.
- The single exception is `snapshot-boot-manager --remove`, which clears the
  hibernation image on `mmcblk0p54`. That is LG's own tool on its own
  scratch partition, the kernel rebuilds the image on the next boot, and the
  worst case is one slow cold boot. It is not a firmware partition.
- `/usr`, `/etc`, `/lib` and `/mnt` are read-only overlays with lowerdir only
  and cannot be written even as root, which is why the mod shadows the
  compositor module and bind-mounts over Home's assets instead of editing
  them in place.
- No firmware flashing, no `dd` to `mmcblk0*`, no epk handling.

If a future change needs a partition write, stop and reconsider the design.

## The set

- LG OLED65C4PUA, model code `o22n2`, webOS 10.2.1, firmware 33.22.52, MediaTek
  (`chip=O22A3`), 2 GB RAM, armv7 userspace (`GLIBC_2.28`).
- Rooted with Homebrew Channel. SSH root login via Homebrew Channel's sshd; no
  passwordless sudo, the login is root. `luna-send` from a non-PTY SSH exec
  hangs — allocate a PTY. `journalctl` hangs — read `/var/log/messages`
  (monotonic timestamps in brackets), which rotates within minutes.
- A plain `reboot` returns powered on and is safe to drive over SSH. The luna
  `machineReboot` drops to standby and does not power back on. Wake-on-LAN
  works with `wolwowlOnOff` on (built-in LAN port), so a set in standby or
  fully off can be woken with a magic packet.
- `/sys/power/mode` reads `resume` on a normal boot and `making` on a boot
  that rebuilds the hibernation image.
- Screen capture: `luna://com.webos.service.capture/executeOneShot` with
  `{"path":"/tmp/x.png","method":"DISPLAY","format":"PNG","width":960,"height":540}`,
  then sftp the file. A blank panel returns ~835 bytes at 320x180.

## What the mod is

Two halves, one owner each.

- Compositor: a full shadow copy of the `WebOSCompositor` QML module (318
  files) at `/var/lib/lumaglass/qml`, with `views/fullscreen/
  StarfishFullscreenContainer.qml` swapped for stock-plus-one-`Loader` and a
  `lumaglass/` directory added beside it (LumaHome.qml, Glass, StatusBar,
  Widget, Dock, Tile, the widgets, pre-rendered icons). `surface-manager-
  daemon.service` has `EnvironmentFile=-/var/systemd/system/env/surface-
  manager.env`; the mod writes `QML2_IMPORT_PATH=/var/lib/lumaglass/qml:/usr/
  lib/qt5/qml` there. `/usr` is a lowerdir-only overlay and can never be
  written. The Loader instantiates LumaHome above the Home surface while
  Home is the container's app; the stock Flutter Home stays resident and
  unseen underneath, or the web stub that `homestub on` puts in its place (see
  "The Home stub").
- Home: `mount --bind /var/lib/lumaglass/assets` over
  `/usr/palm/applications/com.webos.app.home/data/flutter_assets/assets`
  (decluttered `home.xml`, strings, blank hero banners), then relaunch Home.
  A bind does not survive a reboot.

Input: the compositor's `KeyFilter` sends remote keys straight to the focused
Wayland window, so a QML item never sees them through QtQuick focus. The
stock `StarfishKeyFilter.qml` builds its handler chain from the configd list
`com.webos.surfacemanager.keyFilters` (`[{file, handler}]`, loaded through
`StarfishKeyFilterLoader.qml` + `Qt.include`, re-read live on change). The
tool appends `{file: /var/lib/lumaglass/keyfilter/lumaglass.js, handler:
handleLumaHome}` at apply and boot-bind and removes it at revert. That script
runs inside the compositor's QML engine, instantiates `LumaKeyBridge.qml` from
the module and dispatches through the `LumaBus.js` pragma-library instance
LumaHome registered with; it accepts a key only while Home is the active
surface and LumaHome consumed it, everything else falls through to LG's
chain. Pointer input is ordinary QtQuick mouse events (LumaHome sits above
the surface item and swallows what no card or tile takes).

Rendering: one `GaussianBlur` + `HueSaturation` of the wallpaper at half
resolution, baked once into a `ShaderEffectSource`; every card, tile and the
status bar is a single `ShaderEffect` quad that samples that texture at its
own screen rect and draws tint, inset edge, sheen, rim and drop shadow from a
rounded-rect signed distance. No layers, no per-frame effects; focus is a
uniform plus a transform. Text is `NativeRendering` on integer positions and
never inside a scaled item (the glass under it scales, the content does not).
Icons are PNGs rendered by `generators/gen-icons.py` at display size (no
SVG plugin on the set).

Frame budget, measured with `focusMs: 5000` and the per-second `[FPSLOG]`
counter (middle seconds are exact): the set is fill-rate bound. Dock alone
60; each widget card's glass quad costs ~2 fps until the whole layout sat at
50, and a two-read live wallpaper took 10 more. What brought it back to 60:
shadow-only early return outside the card/tile rect, margins trimmed to the
shadow reach, mediump colour math, scrim folded into the wallpaper pass, the
covered stock Home surface at opacity 0, and the wallpaper warp moved into
the vertex shader on a GridMesh. Every new full-screen or card-sized quad
must be re-measured this way. Measured: idle 0 frames; the analog second hand's
160 ms tick renders ~10 frames/s; a 150 ms focus change renders 9 frames,
i.e. 60 fps.

Theme: `/var/lib/lumatv/theme/` (`theme.json`, `layout.json`, assets) polled
every 2 s and applied live. The files there hold only what the owner changed:
LumaHome merges them over the shipped `payload/theme` files, which `stage_luma`
stages beside the QML as `theme.default.json` and `layout.default.json`
(objects merge key by key, arrays and scalars replace), and `lumatv apply` runs
`tools/theme.py minimize` on them so what was never changed follows an update.
`theme.json` is the one token source (colour roles of both materials in
Lucent's names, type roles, radii, spacing, focus, states, motion, font);
`tools/tokens.py` resolves it for the web CSS (`theme.css` in the web apps'
overlay), for the settings-sheet palette Quick Settings is patched with, and
for `theme/Tokens.js`, which QML in other processes imports by file URL. The
compositor's QML reads it through `kit/Lu.qml`. `layout.grid.rows` is a count
or an array of pixel heights; the shipped layout pins the mock's 222/238/392.

Key codes: the remote's Back arrives as evdev KEY_PREVIOUS (412), not
KEY_BACK (158); injecting 158 does nothing on Home. Qt 5.12's QML
XMLHttpRequest can PUT to file://, which is how the Home popovers save
theme.json (the compositor is not allowed to call Homebrew's exec service).
It goes through the network access manager's queue, though: PUTs of the Home
stub's heartbeat twice landed 5 s late while the timer ticked on time. Where
timing matters LG's own `TextFile` (`import TextFile 1.0`; `name` as a
`file:///` URL, `exists`, `read()`, `write(text)`, all synchronous) is used
instead, as in the stub's heartbeat, mode, escape and confirm files.

Luna from the layer: `Service { appId: LS.appId }` (the compositor's own
identity; a made-up appId gets no reply, silently). Account data:
`accountmanager/getLoginUserData {serviceName:"LGE", subscribe:true}`
(`profileNick`, `iconNick`, `profileBg`, `id`). Launch-point order:
`homelaunchpoints/listLaunchPoints` (subscribe) carries `unmovable`;
`moveLaunchPoint {launchPointId, position}` writes it.

Mini apps: LumaHome's `openMini(spec, fromRect)` grows `MiniFrame.qml` (one
Glass quad) from a card to the centred frame; `kind: "web"` launches
`org.nphil.lumaglass.frame` (appinfo `defaultWindowType: "overlay"`,
`transparent: true`, `visible: false`) with `{url, title, source, ago, mode,
light, rect}`; the app positions its pane at `rect` and leaves the rest clear.
WAM honours `defaultWindowType` from appinfo; overlay surfaces are accepted by
StarfishOverlayView and the compositor's own `handleSystemKeys` closes them on
Back. LumaHome subscribes to `applicationManager/running` to see the app
appear (spinner off) and disappear (frame closes). The app reaches the root
service through `PalmServiceBridge` (`window.webOS` is not present in a WAM
app without the webOS.js library); fonts must live inside the app directory
(file:// access to other directories is blocked). `kind: "qml"` content goes
in `MiniFrame.contentSlot` for native mini apps (weather, HA, calendar).
`appInstallService/dev/install {id, ipkUrl, subscribe}` installs a local ipk
on the rooted set; the frame ipk is built by `tools/build-ipk.sh` into
`payload/frame/` and installed by `install_frame_app` at apply.

Not built yet: widget rearrange/resize (hold on a card, move/snap on the
grid with animated repositioning, span/rows changes written back to
layout.json). The dock's move mode is the pattern to extend: pick up on
hold, ListModel.move so existing delegates animate, persist on drop.

State lives in `/var/lib/lumaglass/`; the CLI is `tools/lumaglass`, the boot
worker is `tools/autostart.sh`. `status` is the source of truth for what is
live.

## Boot: the proven mechanism

The set does not cold-boot from standby. `/proc/cmdline` carries
`snapshot resume=/dev/mmcblk0p54`, boot status reports
`snapshot-resume-done: true`, and the compositor restored from that image
carries the pid and start time it had when the image was made. A "first
start" of the compositor is therefore a memory restore, and systemd
timestamps read on a resumed boot are values baked into the image, not this
boot's.

On a genuine cold boot (`mode=making`), measured:

| event | monotonic |
| :--- | ---: |
| `mount-dynamic-partition.service` | 1.07–1.32 s |
| `ls-hubd` ExecMainStart | 1.32 s |
| `var.mount` (p58 on /var) active | 1.98 s |
| `surface-manager-daemon` ExecMainStart | 2.63 s |
| `sam.service` | 5.28 s |
| `mount-readwrite.service` active | 10.8–13.1 s |
| `kdump.service` ExecMainStart | 12.1–14.3 s |
| image capture (making boot) | ~15 s |
| Home first paint | ~12–18 s |
| Homebrew Channel `init.d` run-parts | ~33 s |

The compositor's first start can never load the modded QML, and the reason is
the filesystem, not ordering. Until `mount-readwrite.service` relocates it,
`/var` does not show the writable ext4 subtree, so `/var/systemd/system/env`
is not a path that exists yet; the daemon's optional `EnvironmentFile` read at
2.63 s is a silent no-op. Proof:

- The env file's atime, set to 2020 before a cold boot, was untouched after it.
- Marker variables planted in `ls-hubd.env`, `configd.env`, `memchute.env`,
  `tvpowerd.env`, `bootd.env` and `sam.env` were absent from every one of
  those processes' `/proc/<pid>/environ`. LG's own env files are inert that
  early too.
- `/var` is a subtree mount (`mountinfo` root `/var` on p58).
  `/var/systemd/system/env/surface-manager.env` and
  `/mnt/lg/cmn_data/var/systemd/system/env/surface-manager.env` are the same
  inode; `/mnt/lg/cmn_data/systemd` does not exist. A copy staged at p58's
  root was not read either, so the early `/var` is the firmware overlay
  (which holds only `lib`), not any view of p58.

Consequences: one compositor restart per boot is structural, ~13 s is the
floor for anything a root mod can do, and every restart after ~13 s picks the
env file up reliably.

### The early hook

Homebrew Channel's run-parts fires at ~33 s, after Home has painted, which
produced the stock-Home-then-blink-then-mod sequence. The mod now triggers
from `kdump.service`: it starts at 12–14 s, reads
`/var/systemd/system/env/kdump.env`, and its script's first act is a bare
`grep`. `persist on` writes `PATH=/var/lib/lumaglass/earlybin:...` into that
env file and a shim named `grep` into `earlybin` that starts the worker once
per boot and then `exec /bin/grep "$@"`. On a set without `crashkernel` on
the kernel command line the script is a no-op that exits 0, and the unit is
`Restart=no`. The shim uses absolute paths throughout (a bare `grep` inside
it would re-enter itself) and launches the worker with a plain PATH. The late
hook stands down when `/tmp/.lumaglass-worker` names a live pid or the boot
completed.

### Readiness

`systemctl is-active`, the Home process, `NL_HOME_SHOWN` and
`getForegroundAppInfo` all go true ~8 s before anything is composited. The
only honest gate is the panel: `executeOneShot` at 320x180 returns 835 bytes
while blank and >79000 once Home is up. Measured after a restart: content at
+12 s, and asking for Home at +4 s does not delay it. The old sequence slept
14 s before launching Home and 8 s after; both sleeps are gone. The gate is
bounded at 30 s, and on the boot path a timeout is not an error (a boot into
standby never presents).

### Result

Wake from cold, frame-captured every 2 s (`evidence/boot/`): live TV at 12 s
(the image's restored state), black from ~17 s (our restart pre-empts Home's
first paint), modded home fully formed at 33–36 s. Previously ~55–60 s with a
stock-Home phase in between. Whether Home paints before the restart lands is
a race that varies by a second or two per boot.

## Standby: the proven mechanism

After power-off the set enters active standby (RAM alive) within ~3.4 s. A
power-on from there is instant with the mod still on screen. What made it go
cold was `faultmanager`: at power-off it scans
`/tmp/var/log/reports/librdx`, and any report there makes it answer
`responseSuspendRequest` with `ack:false, reason:"rebootToSuspend",
rebootReason:"rebootByFault"`; tvpowerd then reboots ~6 s after entering
standby. Captured on the bus with
`ls-monitor -f com.webos.service.tvpower` written to `/var` (not `/tmp`,
which does not survive the reboot).

Two reports were arming it.

1. The compositor restart makes `com.webos.app.inputcommon` segfault on
   teardown (`QFontDatabase::removeAllApplicationFonts`), landing ~3 s after
   the restart. The sweep meant to remove it never worked: it read names back
   from `ls`, which word-splits on the spaces every name contains and prints
   the 0x02 bytes rdxd uses in place of `/` as `?`; `rm -f` on those names
   was a no-op counted as success. The sweep now uses pathname expansion
   (raw bytes) and runs again once Home is back on the panel.
2. `webos-sddp` (Control4 discovery) segfaults in `SDDPSetDevice` (NULL
   dereference, backtrace `main → start_sddp → SDDPStart → SDDPSetDevice`)
   whenever it reads the interface list and finds a point-to-point interface
   carrying the multicast flag: the Tailscale `tun` (`/32`, no broadcast) on
   the reference set. Proven both ways: with `tailscaled` stopped it starts
   and stays active; with the tunnel present it dies within 3 s of
   `systemctl start`. A stock set has no such interface. At boot it starts at
   ~42 s, before Tailscale's hook (which sleeps 30 s) creates the tunnel, so
   it survives until the first wake re-reads interfaces - which is why it
   first looked like a wake crash. Not this mod's and not LG's for normal
   customers. Its script is gated on the set's own `enableSDDP` setting, so
   the **Instant on** toggle (`standby warm`) turns that off and Quick
   Start+ on, remembering the previous value; `standby cold` restores it and
   turns Quick Start+ off. Removing Tailscale from the set would make the
   SDDP part unnecessary.

   Nothing else on the set is affected by the tunnel: `avahi-daemon`
   (AirPlay/HomeKit), `chromecast-provisioning`, `miracast`, `appcasting`,
   `connman` all active; no faultmanager detections; no crashd or rdxd
   history; the three failed units (`broadcast-configd`, `f2io-on`, `gibbs`)
   are tuner one-shots that fail at 12-18 s, before the tunnel exists.

   Tailscale was removed from the set on 2026-09-14 (node logged out, daemon,
   init.d hook and `/media/developer/tailscale` deleted). Remote terminal
   access goes through the Unraid node's subnet routes instead; nothing on
   the set depends on a tunnel any more, and `enableSDDP` is off only because
   the Instant on toggle sets it.

Measured after both: three power-off / wake cycles of 150 s each with no
reboot, uptime carrying across, the same compositor still running, and Home
from a warm wake painting in 8 s (`evidence/home-after-warm-wake.png`).

Related facts from `tvpowerd`: `com.webos.service.tvpower.instantBootRefreshPeriod`
is 24 (hours) via configd, after which a warm set is refreshed cold once
("Wait for 1 Minute at active standby" then "Go to Cold"). There is a
`warmOwner` / `registerActiveStandbyRequest` protocol
(`{"clientName","timestamp","ack"}` responses) and a learned power-off
history at `/mnt/lg/cmn_data/tvpower/webos_offhistory`; neither needed
touching.

## The Home stub: the proven mechanism

LG's Home (`com.webos.app.home`, a Flutter app) is retired behind a web stub:
`payload/homestub` (same id, `type: web`, `appCategory: ["home"]`,
`visible: false`, a blank page) staged at `/var/lib/lumatv/homestub` and
bind-mounted over `/usr/palm/applications/com.webos.app.home`, so LumaHome is
the only Home. `lumatv homestub on|off|confirm|status`; results and the way
back in `docs/research/homestub-result.md`.

- **Order.** The Home assets bind goes in first and stays under the stub (the
  mount over the parent hides it; unmounting shows it again). Then SAM:
  `blockSystemApp` followed at once by `releaseBlockSystemApp` makes it read
  the directory again in under 0.1 s, with no SAM restart, and
  `blockedSystemAppList/USA.json` is byte-identical afterwards (md5 `26e1a4ce...`
  after dozens of cycles). `getAppInfo` then reports `type: web`. The release is
  retried: an app left blocked cannot launch. Only then is LG's instance
  stopped (`pkill` on `flutter-client -i com.webos.app.home`); with Home in
  front, SAM's `last_input_handler` opens the stub about a second later. Going
  back: unmount, block and release (`type: flutter`), then WebAppMgr's `killApp`
  on the stub page (`closeByAppId` only backgrounds it); SAM opens LG's Home
  about 3 s later.
- **The manifest must not set `handlesRelaunch`.** WAM then logs "handlesRelanch
  : true; Do not call raise()" and the stub stays behind the app the Home key
  was pressed in. Home launches skip memchute's memory check ("launch without
  memory check").
- **`/usr` is a shared mount**, so the bind and the unmount propagate into every
  app jail's mirror of the directory.
- **WebAppMgr is the stub's clock.** A compositor restart kills WAM, systemd
  restarts it, and the stub can only launch once it is back: 3 s on two boots
  and on every restart on an idle set, 56 s (stub at 65 s) on the first boot
  after an install. The fullscreen view is hidden and the container 0x0 while no
  surface exists, so nothing of LumaHome can cover that gap: black until Home
  is up. LumaHome is therefore built when the compositor starts (the container
  loads it for the base container at once), which also gives the guard its beat
  and a load error immediately.
- **WebAppMgr dies with the compositor.** After `systemctl restart
  surface-manager-daemon` (boot: restart at 35-36 s) the old WAM closes about
  2.5-3 s later and systemd starts a new one 0.1-0.3 s after that
  (`webapp-mgr.service`, `Restart=on-failure`; serving at 39 s). A web Home
  launched in between goes with the old one. SAM's own `last_input_handler`
  launches Home at about +3.4 s, into that gap, and loses it too; the tool's
  launch (`relaunch_home`) therefore waits until `webapp-mgr.service`'s
  `ExecMainStartTimestampMonotonic` differs from the value taken before the
  restart and `com.palm.webappmanager/listRunningApps` answers. Before that
  gate a boot left the panel black (the boot-5 failure in the result file),
  because the old rule took "Home running, boot path" for a boot into
  standby; it now needs a power state other than Active.
- **Qt 5.12's `Binding` does not restore a property** when its `when` ends over
  a surface that stays. The container sets the Home surface's opacity to 0 or 1
  explicitly; before that, LG's Home returning under a LumaHome that stepped
  aside stayed at opacity 0, a black screen.
- **Boot.** `autostart.sh --worker` runs `homestub-boot` first: stub in place at
  uptime 15 s in three boots, before the compositor restart (36 s), so Flutter
  Home never ran. It skips the stub when the previous boot never reached
  `boot-ok`, and a boot-time fault or two in a row turns the switch off.
  bootd's own first app is launched at 13.9 s: on a `reboot` that is the last
  input (reason `micomPower` is not a full-home boot), but a power-on that
  opens Home would start Flutter Home about 1 s before the 15 s mount and be
  killed 1.5 s later. `homeAutoLaunch` was left as the owner set it: holding
  it off would make every wake land on the last input.
- **The guard** (`lumatv homestub-watch`, one detached shell, 0.05 % of a
  core: builtin reads, a fifo for the one-second wait, one `grep` every 10 s)
  watches `/tmp/lumatv-beat`, which LumaHome rewrites once a second for as long
  as it lives. A beat unchanged for 5 s (and the file 3 s old) is a fault; the
  guard also acts at once when the compositor restarts without the module (its
  environment lacks the shadow path) and when the container writes `error` in
  place of a beat because LumaHome failed to load. A fault drops the stub
  (mode `lg`), opens Home if the stub was in front, and counts a strike; two in
  a row, or an unfinished boot, switch the stub off. Strikes clear after a
  minute of beats.
- **LumaHome and the tool talk through `/tmp`:** `lumatv-beat` (LumaHome writes),
  `lumatv-homemode` (tool writes: `stub|lg|legacy <token> [trial=<s>]`;
  `lg` steps the layer aside, the token changes with every write), `lumatv-escape`
  (the Home key writes) and `lumatv-confirm` (OK on the trial toast writes).
  The compositor may not run commands, so every action goes through a file the
  guard reads. LumaHome and the key bridge use LG's `TextFile` for all of them,
  synchronously; `XMLHttpRequest` PUTs for the beat were seen to land 5 s late.
- **Home held 650 ms** is watched by the key filter's first handler (the press
  still reaches LG's chain untouched, so a short press behaves as before). At
  650 ms it writes the request, cancels LG's own 2.5 s hold (Screen Zoom) with
  `longPressTimer.setBlockCancel(false); cancel()`, and swallows the repeats
  and the release. With no stub the layer hides itself, since LG's Home is
  already underneath.
- **Trial.** `on` writes `/tmp/.lumatv-homestub-trial` (a deadline in uptime
  seconds); the guard drops the stub when it passes. Only `confirm`, or OK on
  LumaHome's toast, writes `"homestub":true` to `config.json`; a reboot in the
  middle of a trial ends it, since the file is in tmpfs.
- **Do not run `apply` over an SSH command that can time out**: the shell dying
  mid-restart leaves the compositor on whatever it had loaded (a stale module
  that `apply` then calls current). Detach it (`nohup`) and read its output.

## Tried and dead

- **A LumaHome boot cover for the stub's late launch.** Built and logged on the
  set: the cover state flipped on, and the screen stayed black. The base
  container is 0x0 and invisible, and its parent view invisible too, while no
  surface exists, so a layer inside it cannot draw. Removed. Covering that gap
  needs the layer above the view (track C's compositor).
- **Holding `homeAutoLaunch` off while the stub is in place.** Nothing brought
  Flutter Home back in any test, so there was nothing for it to fix, and it
  changes where every wake lands (bootd opens the last input instead of Home).
- **Baking the mod into the hibernation image.** LG's own remake path is
  `snapshot-boot-manager --remove` then `reboot`; the next boot comes up
  `mode=making` and the kernel writes a fresh image to p54 (250 MB
  partition). Safe — worst case one cold boot, as after a firmware update —
  and exercised several times. It cannot carry the mod: the image is captured
  at ~15 s, before the 13 s floor plus a restart can finish, so the resumed
  compositor is always stock and the asset bind is absent. `snap_list` has no
  Home/sam entry, and `-m` from a live desktop would not fit against ~1.2 GB
  in use.
- **Staging the env file on p58's root** so the early `/var` would see it:
  not read (see above).
- **A unit drop-in or a `/run/systemd/system` unit**: `/etc` and `/usr` are
  read-only overlays; `/run` is writable but volatile and nothing early can
  populate it.
- **Making the restart cheaper**: the 12–16 s from restart to pixels is the
  compositor's cold start plus Home's cold start (`surface-manager.sh` kills
  every client on restart), not padding.
- **Shadowing `controllers/StarfishKeyFilter.qml`** to hook `preProcess`:
  never loaded. A marker in the shadow copy stayed silent across restarts and
  the log's own "Created a StarfishKeyFilterLoader object for /usr/lib/qt5/
  qml/KeyFilters/..." line shows the stock file running every time. Types
  reached through the module's qmldir resolve to `/usr/lib/qt5/qml` whatever
  `QML2_IMPORT_PATH` says; only relative-file references from an already-
  shadowed view (`Qt.createComponent("fullscreen/...")`, a Loader's relative
  `source`) follow the shadow. The configd key-filter list is the hook.
- **Stealing QtQuick activeFocus for the Home layer**: keys never arrive;
  the KeyFilter delivers them to the Wayland window, not through the item
  tree.
- **A multi-row app grid from `home.xml`**: appList is a fixed single row and
  a duplicated item is the same list.
- **Rewriting each app's appinfo/icon for tile plates** (the `tileicons.js`
  hack): worked, but touched app files and needed `sam` restarts; replaced by
  in-memory plate derivation in `Dock.qml`.
- **A 4K UI plane**: KMS accepts 3840x2160 but the OSD stream stays 1080p
  (torn lines), +290 MB RAM, capture breaks; the UHD OSD mode is chosen from
  boot `hwopt`, unreachable without a partition write. The OSD scaler index
  (`/sys/module/lg1k/parameters/gfx_o22_scaler_index`) changes nothing
  visible (0 breaks output).

## The other mod on this set

Starfish (`nphil/starfish`, formerly `lg-webos-mqtt`; still installed on the
set under `/var/lib/tvweb`) bind-mounts its own adblock `/etc/hosts`. That
bind stacks on top of Homebrew Channel's, which is how Homebrew's OS-update
block (four LG hosts pointed at loopback) was silently defeated: the TV found
a 1.7 GB firmware update and put its NSU alert up at boot+34 s for 30 s on
every boot. Starfish 0.34.3 carries those hosts whenever
`/var/luna/preferences/webosbrew_block_updates` exists.

tvweb used to offer QML screen saver replacements. They are gone as of tvweb
0.35.0, and the reason is worth keeping because it applies to any webOS 10
set.

The community mechanism (`webosbrew/custom-screensaver` and forks such as
`aabytt/custom-screensaver-aerial`) is a **single-file** bind:

    mount --bind our.qml /usr/palm/applications/com.webos.app.screensaver/qml/main.qml

Nothing else is touched, so the app SAM scanned is unchanged: no type change,
no SAM restart, and a boot hook re-applying it is enough. It works because on
webOS 5 to 23 the stock screen saver is a QML app - exactly the compatibility
range those projects advertise.

webOS 10 rewrote it in Flutter. The app directory holds `appinfo.json` with
`"type": "flutter"`, `data/flutter_assets/` and `lib/libapp.so`, and there is
no `qml/` directory at all, so aerial's own `apply.sh` exits "Target file does
not exist" here. tvweb's screen savers descended from that lineage, which is
why they never ran: every launch failed "Unable to start engine without AOT
data" and exited 85 ms later, and after a few of those tvpower refused all
further requests (`InvalidStatechangeRequest`) until a power cycle - the set
showed no screen saver at all. That, not the compositor widgets, was the cause
of "the screen saver never kicks in".

Rejected alternatives, all tested on the set:

- Binding our own directory over the whole app with a QML `appinfo.json` does
  run, but only after SAM re-reads the app type, which it caches at its boot
  scan (5s) while any writable filesystem arrives at ~13s. `systemctl restart
  sam` forces it and works, at the cost of a dropped Home and an `eim` crash
  whose report makes faultmanager reboot the set at the next power-off. It
  would also need repeating every boot.
- A copy under `/media/developer/apps` or `/media/cryptofs/apps` with the same
  app id is ignored: SAM dedupes by id and the system copy wins. Confirmed on
  two cold boots.

With stock restored, LG's idle detector `com.webos.service.nop` calls
`power2/turnOnScreenSaver` exactly 180 s after the last input and the stock app
stays up. Idle time is 3 minutes; `screenSaverTime` is not a settingsservice
key on this firmware. A Flutter build of our own would be the only native route
and was not attempted.

LumaTV's own saver does not replace that app. While Home is in front it is on
tvpower's screen saver list and answers the idle question with no, so the app
is never started (`docs/research/saver-own-result.md`; on the set the list, the
no on both lists, the idle timer's question about 176 s after the last key and
every 180 s after that, the key that ends it and a power-key standby (the list
keeps the client and the question after the wake is answered no) work; the
Magic Remote's pointer and the video saver are not yet tried).

## Compositor restart side effects still worth knowing

A bare `systemctl restart surface-manager-daemon` on stock firmware, no mod,
makes stock apps segfault on teardown. `faultmanager`'s policy is in
`/etc/faultmanager/faultconf.json`; `getFaultStatus` lists what it has seen.
The mod removes only the reports its own restart produced.

## Before a package or a hand deploy

`python3 tests/qmlhost/compile_all.py` compiles every QML file of the surface with the set's import layout and exits 1 on any syntax,
type, property or import error (a file that does not compile is a layer that never builds: Settings once left the panel on a bare
wallpaper). `tools/build-ipk.sh` runs it and stops on a failure or a missing PyQt5. A hand deploy (sftp of `payload/compositor/lumatv`
into the shadow module) runs it first, then loads each changed surface in the offline host (`tests/qmlhost/qmlhost.py <file>`), which
compiles and runs it and prints any warning; `LS` must come from `import WebOSCompositorBase 1.0` (`tests/test_ls_import.py`).

## Measuring again

- Cold boot on demand: `snapshot-boot-manager --remove; reboot` (expect the
  "making" splash once). Normal reboot: `reboot`.
- Whether the first compositor is stock: its `/proc/<pid>/environ`
  `QML2_IMPORT_PATH`, and `awk '{print int($22/100)}' /proc/<pid>/stat` for
  its start second.
- Panel timeline during boot: a background loop calling `executeOneShot`
  every 2 s from the early shim, recording `writtenBytes`; frames in
  `evidence/boot/` came from that.
- Standby: `ls-monitor -f com.webos.service.tvpower > /var/lib/lumaglass/x.log`
  before `luna://com.webos.service.tvpower/power2/powerOff
  '{"reason":"remoteKey"}'`; wake with a WoL packet; compare `/proc/uptime`
  before and after. SSH survives active standby.

## Research

`research/home-launcher-replacements.md` surveys other rooted-webOS home
replacements and the input-hook tooling around them; it informed the choice
to mod the compositor and Home in place rather than launch over them.

Prior art worth reading before designing anything new:

- `webosbrew/custom-screensaver` and its fork
  `aabytt/custom-screensaver-aerial` — the single-file bind over
  `qml/main.qml`, and the cleanest example of modding a stock app without
  touching what SAM scanned. Its compatibility stopping at webOS 23 is the
  tell that LG changed the app's runtime.
- `webosbrew/webos-homebrew-channel` — the root elevation, the `init.d`
  runner this mod hooks, the failsafe flag, and the `/etc/hosts` update
  block.
- `openlgtv` — firmware archaeology rather than runtime modding:
  `epk2extract` unpacks LG firmware images (useful for reading a stock app or
  binary from a newer release without owning that set), plus buildroot trees
  and `nsu_emu`, an update-server emulator. Nothing needed for this project,
  but it is where to look when a question is "what changed in firmware X".
