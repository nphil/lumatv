# LumaTV

LumaTV provides a native liquid-glass home screen for rooted LG webOS TVs.

EXPERIMENTAL - TRY AT YOUR OWN RISK!

## Features

- **Liquid glass theme**: Native Home, panels and settings share a layered glass design
- **Abstract wallpaper**: Generated wallpaper for the native Home screen
- **Home layout**: Configurable cards, widgets and app dock
- **Survives reboots**: an opt-in Homebrew Channel boot hook re-applies the mod after every restart. It does **not** survive a firmware update - an update replaces the system partitions, after which you re-apply from the app (one button)
- **Reversible**: compositor changes use bind mounts and private copies. The optional wake policy changes configd separately; `wake off` restores its previous settings.

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

Helper updates require the matching `tools/lumatv`: apply, boot and `helper start` atomically remove the obsolete
`helper.json` field `log.maxBytes` before running the new helper. Other configuration bytes, permissions and ownership stay
intact; invalid JSON or a detected configuration change stops startup without overwriting the file. Product configuration
writers share a lock on the data directory; arbitrary external editors do not participate in that lock.

## Usage

Launch **LumaTV** from the Homebrew Channel. The app provides:

- **Apply**: Apply LumaTV and start an installed native compositor bundle
- **Revert**: Restore stock LG Home and remove LumaTV overlays
- **Persist** (on/off): Keep the mod applied across reboots (persists state to `/var/lib/lumatv/`, and installs the early boot hook described below)
- **Instant on** (on/off): Stay in RAM while off so power-on is instant with the mod already up, or power fully down (see Standby below)
- **FPS** (on/off): Enable compositor FPS debug overlay (for development)
- **Notify**: Configure optional ntfy.sh integration for status/alerts (requires internet + token)
- **Status**: Show current mod state, version, applied mounts, and configuration
- **Log**: View debug logs from recent applies/reverts

## How It Works

The native compositor uses the Rust UI for all 22 shell kinds. The compositor
bundle is installed separately from the IPK. Startup and runtime failures
return to stock LG Home. Project QML frontends and restyles are not shipped;
ownership-checked restoration removes previous LumaTV overlays.

### Theme

`/var/lib/lumatv/theme/` holds the theme and Home layout configuration.

The owner's `layout.json` holds only what differs from the shipped one, and is written by Settings › Home & look › Home widgets, by the
layout editor on Home (hold OK on a card) and by `lumatv layout get|check|put|set|reset`. A card has `id`, `type`, `col`, `row`, `span`
and `rows` on the grid's cells (`grid.rows` is a count or a list of pixel heights), and `hidden: true` to leave it off Home without
losing its place. `lumatv layout reset` is the way back to the shipped layout.

Nitin and Isabel have a profile each (`docs/research/profiles-design.md`): the helper keeps each person's look, cards, dock and quick panel in
`profiles/<id>/` and puts the active person's part into these files when the profile is switched (`/v1/profiles/switch`). Which keys are a person's
and which the household's is `compositor/assets/ui/profile-scope.json`.

The stock LG Home app remains the fallback.

The mod state and configuration live in `/var/lib/lumatv/`, which persists across reboots if persistence is enabled.
An interrupted maintenance transaction blocks boot activation and configuration migration through
`/var/lib/lumatv/maintenance`. Status reads and human off, revert and Home-hold escape remain available;
the interruption marker and root-private rollback copies remain until reviewed recovery completes.


### LG service-host repair

`lumatv sysbus on` stages a private repair for the known webOS 10.2.1 Luna binding and changes only the unified JavaScript host’s launcher. `lumatv sysbus restart` applies it to the running host; Home Assistant’s TV connections briefly disconnect while the hosted services restart. Other Node processes keep LG’s original binding.

`lumatv sysbus status` distinguishes the saved setting, launcher binding and running process. Only `loaded: patched` confirms that the running host uses the repaired module. The persistence hook reapplies the launcher at boot when enabled, but may run after the host has started; a restart can still be needed. Unknown firmware files and foreign launcher mounts are refused.

`lumatv sysbus off --restart` restores the stock launcher and restarts the host with LG’s original module. The full stock-UI restoration also removes the launcher binding and disables its boot reapplication; an already-running host keeps its mapped module until it exits.

### Standby

`lumatv wake on` stages an experimental, opt-in policy to restore the last
foreground app on ordinary wake and power-on; `wake off` restores the previous
reason list and policy, and `wake status` reports the configured state. The
existing detached boot worker reloads the owned policy after boot bindings,
then uses the existing destination landing and apply path before final checks.
Fresh worker and activation tokens bind acceptance to the actual foreground,
not a potentially stale native `firstAppId`. Native explicit input/app choices
retain priority. The lifecycle commands `stage`, `trial`, `persist`, the phased
`boot-check` and `commit` are internal.

The runtime trial accepts native log truncation only when the latest bootd
start timestamp falls within the trial's monotonic-clock window and both
policy loaders name the owned policy. Failures retain per-check observations
and requested/observed configuration values before restoring stock settings.
Factory settings override temporary values on this firmware. The trial removes
only stock-valued Factory overrides after checking that the underlying defaults
remain stock after power loss. Recovery restores Factory before temporary values.

Acceptance uses fresh native policy-load records, actual foreground and activation-specific
boot reports. The staged cold-start check must pass before `wake commit`; renderer health alone does not establish a destination pass.
`wake off` is separate from `lumatv revert`. When the original policy path was
absent, rollback sets
the explicit stock path instead: its behavior is stock, but the setting's
metadata differs from an absent value. A future firmware change may require
removing that override and rebasing the owned policy. Unsupported native
policies are rejected rather than replaced.

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

The mod is developed and tested on webOS 10.2.1 (build 5202, Rockhopper chassis). Other firmware releases may require changes.

To re-apply after a firmware update:
1. Launch LumaTV in the Homebrew Channel
2. Tap **Apply** (all files are still present in the app's persistent directory)

## Troubleshooting

### Mod not appearing on screen after apply

- Check **Status** and **Log** in the app for errors.
- Use **Revert** to return to stock LG Home.

### TV appears stuck/frozen after apply

- Revert the mod via the Homebrew Channel → LumaTV → **Revert**.

## Building

### Prerequisites

- Node.js 20+
- ar, tar, sha256sum (standard Linux utilities)
- This repo cloned with git

### Build the IPK

```bash
sh tools/build-ipk.sh
```

The builder names the IPK from the app version and validates its structure.

### Generate Manifest

```bash
node tools/gen-manifest.js org.nphil.lumatv_0.1.0_all.ipk manifest.json
```

Creates a `manifest.json` with package metadata for the Homebrew Channel.

### Branding

Names come from `brand.json`. The IPK builder fills branding placeholders in
the staged copy and ships the branding file for the on-TV tools.
Renaming the project means editing `brand.json` and, if the repository moved,
its `repo` entry.

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

`docs/AGENTS.md` is the entry point for an agent or new contributor: the component
map, build quirks, install and rollback path, TV access, the frame gate and the
operational rules learned on the set.

`docs/HANDOFF.md` is the engineering record: the measured boot and standby
mechanisms on the reference set, what was tried and why it is dead, and how
to measure again. `docs/research/` holds the survey of other rooted-webOS
home replacements, and `docs/evidence/` the frame captures the measurements
rest on.

## Reverting/Uninstalling

The mod is completely non-destructive:

- **Revert**: Launch LumaTV → **Revert** to restore stock LG Home.
- **Uninstall**: Revert first, then remove the app from Homebrew Channel (or via `ipk-uninstall` on SSH)

After revert, the TV returns to stock appearance and behavior; no system files were modified.

## License

MIT License. See [LICENSE](LICENSE) for details.

## Acknowledgments

Developed for rooted LG webOS TVs. Tested on webOS 10.2.1. Thanks to the webOS community and Homebrew Channel project.
