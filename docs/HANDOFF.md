# LumaGlass — engineering handoff

Everything a person or model needs to pick this project up cold: how the mod
works, what was measured on the set, which approaches are dead and why, and
how to measure again. Facts here were observed on one set unless marked
[INFERENCE]. Credentials and the set's address are deliberately absent; they
live with the owner.

## Resume here (LumaTV, 2026-10-24 22:20 EDT)

Written so any model can continue cold. The plan is `docs/PLAN.md` (Status,
Phases, Decided); this file holds the proven mechanics.

- State: research done, compositor replacement decided (PLAN.md track C).
  The owner said go (2026-10-24); the build is under way. Phase 1 step 1,
  the rename to LumaTV with every name from `brand.json`
  (`docs/research/rename-result.md`), is deployed and proven on the set:
  migration moved `/var/lib/lumaglass` to `/var/lib/lumatv`, rewrote the env
  line, replaced the init.d link with `50-lumatv`, removed both old apps;
  `apply` restored LumaHome, and a reboot came up live through the new boot
  hooks with no failsafe. A copy of the old data dir is kept off the set in
  `/data/home/lumatv-backups/lumaglass-data-2026-10-24.tgz`.
- Repositories renamed 2026-10-24: forge `nphilip89/lumatv` (remote `forge`)
  and GitHub `nphil/lumatv`; `brand.json` `repo` matches.
- Next, phase 1 in order: T9 plus disk-cache env lines (tool already
  carries them; enable and measure), shared 100 ms clock, Home stub, heap
  shim, Rust helper.
- Remote tests: done 2026-10-24 for Netflix 4K Dolby Vision playback (X4
  pass 3, contiguous-buffer hold) and the SSAP key device; results in
  PLAN.md. Still open: X4 during PlxNative playback
  (`docs/research/arch/x4/x4-monitor.sh`, `x4-analyse.py`); the histogram
  probe, which first needs the `libvtcapture` histogram call signatures
  (LG's Hue Sync service is the reference caller). Mute the receiver first
  (`media_player.living_room_avr`) and restore it after. Netflix deep link
  for the 4K Dolby Vision test film: launch `netflix` with `contentTarget`
  `m=https://www.netflix.com/watch/80141336&source_type=4`, then ENTER.
  Protected video captures black; the tests record compositor behaviour.
  During Dolby Vision playback Picture Mode reads `dolbyHdrCinema` (the
  set's own Dolby Vision mode); it returns to `filmMaker` on Home.
- Tests that need the owner: PIN prompt (a lock is a security setting he
  sets), pointer cost (Magic Remote moved by hand), CEC keys from other
  remotes, receiver volume drift.
- Set rules: confirm it is idle first (`getPowerState`,
  `getForegroundAppInfo`); Picture Mode stays `filmMaker`; never restart
  SAM; write only under `/var/lib/lumatv`, `/tmp`, configd, bind mounts
  and `/var/systemd/system/env`; revert every experiment; leave it in
  Active Standby unless the owner is about to watch; no DRM bypass.
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
  unseen underneath.
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

Theme: `/var/lib/lumaglass/theme/` (`theme.json`, `layout.json`, assets)
polled every 2 s and applied live. `layout.grid.rows` is a count or an array
of pixel heights; the shipped layout pins the mock's 222/238/392. `stage_theme`
overwrites a theme file only while it still equals the last shipped copy
(`.shipped-<file>` md5), so user edits survive updates.

Key codes: the remote's Back arrives as evdev KEY_PREVIOUS (412), not
KEY_BACK (158); injecting 158 does nothing on Home. Qt 5.12's QML
XMLHttpRequest can PUT to file://, which is how the Home popovers save
theme.json (the compositor is not allowed to call Homebrew's exec service).

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

## Tried and dead

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

## Compositor restart side effects still worth knowing

A bare `systemctl restart surface-manager-daemon` on stock firmware, no mod,
makes stock apps segfault on teardown. `faultmanager`'s policy is in
`/etc/faultmanager/faultconf.json`; `getFaultStatus` lists what it has seen.
The mod removes only the reports its own restart produced.

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
