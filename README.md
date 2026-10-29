# LumaTV

Liquid glass home screen for rooted LG webOS TVs. Reskins the stock LG webOS 10 Home screen with a compositor-level "liquid glass" material design, generated abstract wallpaper, and a decluttered home layout.

EXPERIMENTAL - TRY AT YOUR OWN RISK!

## Features

- **Liquid glass theme**: Layered glass aesthetic with Rose Pine color palette, applied at the compositor level for seamless integration
- **Abstract wallpaper**: Generated dynamic wallpaper displayed through a transparent Home screen
- **Decluttered layout**: LG's network-loaded recommendation shelf and Q-card row are removed, which is also the single biggest responsiveness win; the hero strings are blanked so the clock and weather widgets own that space
- **Survives reboots**: an opt-in Homebrew Channel boot hook re-applies the mod after every restart. It does **not** survive a firmware update - an update replaces the system partitions, after which you re-apply from the app (one button)
- **Non-destructive**: every change is a bind mount over a read-only file plus a private copy under `/var/lib/lumatv`. No system file is ever written, and `Revert to stock` removes all of it

## Requirements

- **LG webOS 10.2.1** (tested on 32-bit ARM userspace, aarch64 kernel; compatibility with other webOS 10 releases likely)
- **Rooted TV** with Homebrew Channel installed
- **SSH access** to the TV (enabled by default on rooted TVs)

## Installation

### Via Homebrew Channel (Custom Repository)

1. Open the **Homebrew Channel** app and press the **gear** icon to open Settings
2. Add this repository URL:
   ```
   https://raw.githubusercontent.com/nphil/lumatv/main/repo.json
   ```
3. Go back to the app list and refresh; **LumaTV** appears alongside the official packages
4. Select it and press **Install**. Updates land the same way: bump appears when a new release is tagged

### Manual Installation (IPK)

1. Download the latest `org.nphil.lumatv_<version>_all.ipk` from [Releases](https://github.com/nphil/lumatv/releases)
2. Copy the IPK to the TV via SSH:
   ```bash
   scp org.nphil.lumatv_*.ipk root@<tv-ip>:/media/developer/
   ```
3. SSH into the TV and install:
   ```bash
   ssh root@<tv-ip>
   ipk-install /media/developer/org.nphil.lumatv_*.ipk
   ```
4. The app appears in the Homebrew Channel; launch it to apply the mod

## Usage

Launch **LumaTV** from the Homebrew Channel. The app provides:

- **Apply**: Install the mod (mounts compositor QML and Home app assets, restarts services)
- **Revert**: Remove the mod (unmounts everything, unregisters the key filter, restarts services)
- **Persist** (on/off): Keep the mod applied across reboots (persists state to `/var/lib/lumatv/`, and installs the early boot hook described below)
- **Instant on** (on/off): Stay in RAM while off so power-on is instant with the mod already up, or power fully down (see Standby below)
- **FPS** (on/off): Enable compositor FPS debug overlay (for development)
- **Notify**: Configure optional ntfy.sh integration for status/alerts (requires internet + token)
- **Status**: Show current mod state, version, applied mounts, and configuration
- **Log**: View debug logs from recent applies/reverts

## How It Works

The compositor draws the Home screen; the stock Home app stays resident
underneath as a placeholder.

### 1. Compositor layer

`surface-manager` resolves QML modules through `QML2_IMPORT_PATH`. The tool
keeps a shadow copy of the `WebOSCompositor` module under `/var/lib/lumatv/qml`
with two changes and points the compositor at it through its optional
environment file:

- `views/fullscreen/StarfishFullscreenContainer.qml` is stock plus one
  `Loader` that instantiates `lumatv/LumaHome.qml` while Home is the
  container's app.
- `lumatv/` holds the Home itself: wallpaper, the glass material (one
  cached blur of the wallpaper sampled by single-quad shaders for cards,
  tiles and the status bar), the status bar, the widget grid (clock, Home
  Assistant, news, weather) and the app dock. The layer animates with
  transforms and shader uniforms only; nothing runs per frame while idle.

Remote keys arrive through `payload/keyfilter/lumatv.js`, which the tool
registers in configd's `com.webos.surfacemanager.keyFilters` list; the stock
key filter loads every entry of that list and picks up changes live. The
Magic Remote pointer arrives as ordinary QtQuick mouse events.

### 2. Theme

`/var/lib/lumatv/theme/` is a self-contained package: `theme.json`
(material tokens, accent, radii, blur, motion, tile rules, widget settings),
`layout.json` (safe area, column grid with row heights, status bar items,
widget entries with col/row/span/rows) and the assets they reference
(wallpaper, fonts, icons, widget QML). Changes reload within 2 s without a
compositor restart. Widget types resolve to a built-in, a `.qml` file in the
theme directory, or an installed pack under `/var/lib/lumatv/widgets/`.

The owner's `layout.json` holds only what differs from the shipped one, and is written by Settings › Home & look › Home widgets, by the
layout editor on Home (hold OK on a card) and by `lumatv layout get|check|put|set|reset`. A card has `id`, `type`, `col`, `row`, `span`
and `rows` on the grid's cells (`grid.rows` is a count or a list of pixel heights), and `hidden: true` to leave it off Home without
losing its place. `lumatv layout reset` is the way back to the shipped layout.

Nitin and Isabel have a profile each (`docs/research/profiles-design.md`): the helper keeps each person's look, cards, dock and quick panel in
`profiles/<id>/` and puts the active person's part into these files when the profile is switched (`/v1/profiles/switch`). Which keys are a person's
and which the household's is `payload/compositor/lumatv/profile-scope.json`.

### 3. Home app assets (`/usr/palm/applications/com.webos.app.home/data/flutter_assets/assets`)

A bind mount of a merged copy of the stock assets with a decluttered
`home.xml`, blank hero banners and adjusted strings, so the placeholder Home
paints nothing that could show through.

The mod state and configuration live in `/var/lib/lumatv/`, which persists across reboots if persistence is enabled.

### 4. Home stub (optional, over SSH)

`lumatv homestub on` replaces LG's Flutter Home with a blank web app of the
same id, bound over its directory, so the compositor-drawn Home is the only
Home (one 32 MB renderer instead of a 72-80 MB process). It starts a 120 s
trial: press OK on the toast, or run `lumatv homestub confirm`, to keep it,
otherwise it reverts itself. `off` and `revert` remove it. A guard process
gives LG's Home back within about 6 s if the Home layer stops drawing, and
holding Home for 650 ms anywhere does the same by hand until the next apply
or reboot. `lumatv homestub status` reports its state.

### Boot timing

The set resumes a hibernation image rather than cold-booting, and the
compositor inside it starts at 2.63s. It cannot load the modded QML: `/var`
does not show the writable filesystem until `mount-readwrite.service` at
12.96s, so the compositor unit's optional `EnvironmentFile` under `/var` is
not a path that exists yet. One compositor restart per boot is therefore
structural, and ~13s is the floor for applying anything.

With persistence on, the apply runs from a PATH shim in front of `grep` for
`kdump.service` (starts 14.2s) rather than waiting for Homebrew Channel's
`init.d` run-parts at ~33s. The modded home reaches the panel at ~33s, and
stock Home never paints first. Homebrew Channel's hook remains as the
fallback if the early apply fails.

### Standby

A full boot only happens when the set has gone cold. After power-off it
enters active standby with RAM alive, and a power-on from there is instant,
with the mod still on screen. What used to force it cold was faultmanager: any
crash report under `/tmp/var/log/reports/librdx` at power-off makes it answer
the suspend request with `rebootToSuspend`, and the set reboots about 6s
after entering standby. The compositor restart provokes exactly one such
report (inputcommon's teardown), which the apply now removes.

The other report seen is not this mod's, and not LG's fault either.
`webos-sddp`, LG's Control4 discovery service, segfaults in `SDDPSetDevice`
whenever it reads the interface list and finds a point-to-point interface
carrying the multicast flag - a Tailscale `tun` on the reference set. Proven
by stopping tailscaled: without the tunnel it starts and stays up. A stock
set has no such interface. At boot it starts before Tailscale's hook creates
the tunnel, then re-reads interfaces on the first wake and dies there; its
report arms the same reboot at the next power-off. The **Instant on** toggle
(`standby warm`) turns Quick Start+ on and `enableSDDP` off, remembering the
previous value; turning it off (`standby cold`) turns Quick Start+ off and
restores SDDP. Warm: instant power-on with the mod already up, higher
standby draw, no Control4 discovery. Cold: lowest standby draw, a ~35s boot
at every power-on. `status` reports the live mode as `standby`. A set with
no VPN interface and no Control4 could leave SDDP on; the toggle turns it
off regardless because the crash is silent and costs a cold boot.

## Firmware Compatibility

The mod is **developed and tested on webOS 10.2.1** (build 5202, Rockhopper chassis). Compatibility with other webOS 10 releases depends on the layout of the compositor QML and Home app structure; a firmware update may break the mod (the TV will still boot and work normally without the mod applied).

To re-apply after a firmware update:
1. Launch LumaTV in the Homebrew Channel
2. Tap **Apply** (all files are still present in the app's persistent directory)

## Troubleshooting

### Mod not appearing on screen after apply

- Check **Status** in the app: ensure "Applied: yes" and all three mounts are present
- Restart the Home app: swipe up on the home screen or open another app then return
- If compositing still looks wrong, check **Log** for errors; try **Revert** then **Apply** again

### TV appears stuck/frozen after apply

- Revert the mod via the Homebrew Channel → LumaTV → **Revert** (this unmounts everything)
- Or SSH in and manually: `pkill -f surface-manager-daemon; pkill -f com.webos.app.home; systemctl restart surface-manager-daemon`

### Black/corrupt wallpaper on screen

- The wallpaper must be copied to `/tmp/hx/wall_1080.png` before the compositor starts; check app **Log** for copy errors
- If persisting across reboots, verify `/var/lib/webosbrew/init.d/50-lumatv` exists and is executable

## Building

### Prerequisites

- Node.js 20+
- ar, tar, sha256sum (standard Linux utilities)
- This repo cloned with git

### Build the IPK

```bash
sh tools/build-ipk.sh
```

This outputs `org.nphil.lumatv_0.1.0_all.ipk` and validates its structure.

### Generate Manifest

```bash
node tools/gen-manifest.js org.nphil.lumatv_0.1.0_all.ipk manifest.json
```

Creates a `manifest.json` with package metadata for the Homebrew Channel.

### Branding

Every name comes from `brand.json`: display name, short id, app id, data dir and
QML module directory. Sources carry `{{displayName}}`, `{{shortId}}`, `{{appId}}`,
`{{dataDir}}` and `{{qmlModule}}` placeholders; `tools/build-ipk.sh` fills them in on the
staged copy, names the ipk, the tool, the QML module directory, the key filter
and the stylesheet from the file, and ships it so the on-TV tool and boot hook
read the same names. Renaming the project means editing `brand.json` and, if
the repository moved, its `repo` entry.

Sets that ran the project as LumaGlass (data in `/var/lib/lumaglass`) are moved
over by the first run of the new tool: data, boot hooks and env files follow, and
the old apps are removed.

## Development

The mod is packaged as a self-updating homebrew app. Each release (git tag `v*`) triggers:
1. Build of the IPK
2. Generation of `manifest.json` and updated `repo.json`
3. Creation of a GitHub Release with artifacts
4. Commit back to main (with `[skip ci]` to avoid loops)

To release a new version:
1. Update `appinfo.json` version field (e.g., `0.2.0`)
2. Update `CHANGELOG.md`
3. Commit and tag: `git tag v0.2.0 && git push --tags`
4. The release workflow creates the GitHub Release and updates `repo.json` automatically

`docs/HANDOFF.md` is the engineering record: the measured boot and standby
mechanisms on the reference set, what was tried and why it is dead, and how
to measure again. `docs/research/` holds the survey of other rooted-webOS
home replacements, and `docs/evidence/` the frame captures the measurements
rest on.

## Reverting/Uninstalling

The mod is completely non-destructive:

- **Revert**: Launch LumaTV → **Revert** (removes all mounts, unregisters the key filter, restarts compositor)
- **Uninstall**: Revert first, then remove the app from Homebrew Channel (or via `ipk-uninstall` on SSH)

After revert, the TV returns to stock appearance and behavior; no system files were modified.

## License

MIT License. See [LICENSE](LICENSE) for details.

## Acknowledgments

Developed for rooted LG webOS TVs. Tested on webOS 10.2.1. Thanks to the webOS community and Homebrew Channel project.
