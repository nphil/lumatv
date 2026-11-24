# Memory map of the C4 under LumaTV: who holds the RAM, what can go

Measured on the C4 (webOS 10.2.1, kernel 5.4, 4 cores, 2 GB) on 2026-11-24 with nothing changed on the set: reads of `/proc` and `/sys`, one
`luna-send` read of SAM, one Home key to bring Home forward (the lane was held by `LumaTV-MemoryMap`, with Main's go). Numbers are MB of 1,024
kB unless a unit is given. Video and contiguous memory are mapped in [`cma-holders.md`](cma-holders.md) and not redone here.
Earlier passes: [`memory-result.md`](memory-result.md) (2026-10-25), [`declutter-result.md`](declutter-result.md),
[`lg-memchute.md`](lg-memchute.md), [`lg-apiadapter-interfaces.md`](lg-apiadapter-interfaces.md).

## Answer

- At Home idle about 400 MB is available with LumaGame resident, about 585 MB without it (read at 14:47, before LumaGame's run).
  **LumaGame kept hidden holds 208 MB, more than every LG service together (192 MB).** It is the one large item that can be freed today.
- The LG system services cannot give much back: everything the owner's rules forbid touching (panel, power, tuner, HDMI/CEC, DRM, audio,
  input) is most of it. The stoppable remainder is the AI group at 3.9 MB, plus Bluetooth and Cast if their plugins are off.
- LumaTV's own processes hold 171 MB (the compositor and UI 128 MB, the helper 31 MB). The helper spends about 9 MB on 13 `luna-send`
  processes, 2 `openssl` keep-alives and an `arecord`.
- A 73 MB capture file from a perf run sits in `/tmp` (RAM-backed) right now.
- Rewrites (`ss.apiadapter`) would return about 11 MB. Not worth doing for memory (see the plan).

## State at the measurement

| | |
|---|---|
| Time, uptime | 2026-11-24 15:05:13-15:06:13 EST (60 s CPU window), uptime 28,774 s (8 h 0 min); a second pass that ran alongside (15:05:24-15:06:24) agrees within 1 % |
| Foreground | LG Home (`com.webos.app.home`, the Luma Home stub) for the whole window, checked before and after through SAM |
| Resident apps | Home stub and **LumaGame** (hidden, kept by its own prewarm). Netflix, Marquee, Prime Video, Apple TV, Live TV and the browser were not running. WebAppMgr had its main and zygote processes and no renderer |
| Lock | held by `LumaTV-MemoryMap` for the window. Before that `LumaGame-RecorderTv` held the set (a GFN recorder run, 14:47-15:04); the first two snapshots (14:48, 14:53) caught LumaGame in front and are not used |
| Not used for the baseline | 14:48 (LumaGame streaming: `MemAvailable` 334 MB, LumaGame 160 MB PSS) |

Procs: 316 (165 with memory maps, 151 kernel threads), 1,156 threads. Load average 14.3 is about a dozen kernel threads parked in
uninterruptible sleep, not work (same reading as `declutter-result.md`); memory pressure `some avg300` 0.00.

## 1. Totals

| Item | MB | Note |
|---|---:|---|
| MemTotal | 1,982.8 | |
| MemAvailable | 400.4 | with LumaGame resident (585 at 14:47 before its run) |
| MemFree | 216.1 | the rest of the free-looking memory is page cache and CMA lent to apps |
| AnonPages (app heaps) | 433.5 | |
| Cached / Buffers | 284.9 / 23.8 | file cache; reclaimable; `Shmem` 108.6 of Cached |
| Mlocked | 101.9 | pinned pages (GPU and video clients) |
| Slab | 146.0 | 65.1 reclaimable, 80.9 not; top: `kmalloc-4k` 24.6, `ext4_inode_cache` 12.8, `dentry` 10.8, `squashfs_inode_cache` 9.2, `proc_inode_cache` 7.6 |
| KernelStack, PageTables | 18.1, 14.7 | scale with the 1,156 threads and 316 processes |
| VmallocUsed | 310.9 | `__mem_ops_alloc` 258 (172 buffers of 1.5, `vmalloc user`: the driver allocations of `cma-holders.md`, not attributed to a process), `vmap_phys` 73 (the 8 MB display and decoder buffers), `paging_init` 38, kernel stacks 24 |
| CMA (`CmaTotal` 676) | free 3.5, `CmaDeviceAlloc` 269.9 | free was 123 at 14:47: LumaGame's heap is lent CMA, see below and `cma-holders.md` |
| GPU (`GPUTotal`) | 133.8 | 79.2 at 14:47; the Mali contexts of LumaTV's compositor (82) and LumaGame |
| Swap | `/dev/f2io-0` 600, used 216.8 | LG's zacc-compressed swap on eMMC `mmcblk0p53`, not RAM. `zram0` size 0, as found. The 201 MB of per-process swap-PSS is mostly cold daemon pages |
| tmpfs | `/tmp` 113 used of 991 | `/tmp/perf` **73** (a 73 MB `gate-rec3a-game-library-m0.trace`, another agent's capture, RAM-backed), `/tmp/xdg` 22 (`ui-shm`: two 8 MB UI layers, the compositor's trace ring 0.8), `/tmp/org.nphil.lumagame` 11 (LumaGame's layers), `/tmp/var` 9, `gamepad_log.txt` 3.6, `/tmp/cast` 1. `/run` 0.4, `/dev/shm` 0.6 |
| `/var` (flash, not RAM) | 541 of 966 | `/var/lib/lumatv` 390 (compositor build 217, `cache/wx` 65, `systemui` 29, `bin` 26, `assets` 26); the weather-map cache has no size cap seen [INFERENCE] but it costs flash, not RAM |

Reading: of the 2 GB, about 270 MB is driver allocations and 134 MB GPU memory that no user-space change touches, 146 MB is slab, and the
processes' own PSS is 587 MB of which LumaGame is 208.

## 2. Every process, grouped

Each figure comes from `/proc/<pid>/smaps_rollup` (PSS counts shared pages in proportion; USS is Private_Clean + Private_Dirty; swap is Swap-PSS)
and CPU is the share of one core over 60 s from `/proc/<pid>/stat`. Sorted by PSS + swap-PSS inside each group. Full per-process data is in the
raw snapshot, kept in the session scratch (not in git); the top of each group is below.

| Group | Processes | PSS MB | Swap-PSS MB | USS MB | CPU % of one core |
|---|---:|---:|---:|---:|---:|
| LG system services (daemons, luna services) | 129 | 191.9 | 166.0 | 175.0 | 18.2 |
| LG apps (WebAppMgr children, native apps) | 2 | 1.6 | 8.0 | 0.6 | 0.0 |
| Third-party kept apps | 0 | 0.0 | 0.0 | 0.0 | 0.0 |
| LumaTV's own | 25 | 171.3 | 18.8 | 163.9 | 18.2 |
| LumaGame (own app, kept resident, hidden) | 3 | 207.8 | 0.0 | 206.9 | 0.2 |
| Homebrew | 4 | 7.1 | 8.7 | 3.2 | 0.1 |
| Measurement tools (not part of the set) | 2 | 6.7 | 0.0 | 5.0 | 0.0 |
| **All processes with memory maps (165)** | 165 | 586.5 | 201.4 | 554.6 | 38.2 (incl. kernel threads) |

LG's `WebAppMgr` main process is counted under system services; the "LG apps" row is the zygote and the Cast log helper. Nothing from the
third-party list (Netflix, Prime Video, Marquee, Apple TV) was resident. The measurement tools row is this note's own sampler and another
agent's, excluded from every total below.

### LumaTV's own (PSS, USS, swap in MB; top 12 of 25)

| Process | pid | PSS | USS | Swap | Threads | CPU % |
|---|---:|---:|---:|---:|---:|---:|
| ld-linux-aarch6 | 8884 | 100.3 | 97.4 | 0.0 | 13 | 8.34 |
| ld-linux-aarch6 | 29474 | 27.7 | 24.8 | 18.7 | 15 | 5.44 |
| lumatv-helper | 13668 | 30.7 | 30.5 | 0.0 | 85 | 3.5 |
| openssl | 13747 | 1.7 | 1.3 | 0.0 | 1 | 0.0 |
| openssl | 4445 | 1.7 | 1.3 | 0.0 | 1 | 0.0 |
| lumatv-jpeg | 7841 | 1.1 | 1.1 | 0.0 | 1 | 0.65 |
| beszel-native | 13776 | 1.1 | 1.1 | 0.0 | 1 | 0.02 |
| home-assistant | 13730 | 0.9 | 0.9 | 0.0 | 1 | 0.0 |
| luna-send | 13708 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13719 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13725 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13674 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13687 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13724 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13788 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13811 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13870 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13696 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13782 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13802 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| luna-send | 13868 | 0.4 | 0.4 | 0.0 | 1 | 0.0 |
| arecord | 13729 | 0.4 | 0.3 | 0.0 | 1 | 0.23 |
| lumatv-helper | 13667 | 0.2 | 0.1 | 0.0 | 1 | 0.0 |
| lumatv-capture | 4923 | 0.1 | 0.0 | 0.1 | 1 | 0.0 |
| lumatv-homestub | 12177 | 0.0 | 0.0 | 0.0 | 1 | 0.0 |

The remaining 13 are `luna-send` subscriptions (0.4 each, 5.3 together), `arecord` 0.35, `lumatv-homestub` 0.04, `lumatv-capture` 0.05 (+0.1 swap) and
the helper's parent 0.2. The two big `ld-linux-aarch64.so.1` rows are the compositor (pid 8884, `lumatv-compositor`) and its UI child (pid 29474,
`lumatv-ui`): the dynamic loader shows as the process name because both 64-bit programs start through their own bundled loader and libraries. Plugins: `home-assistant` 0.9, `beszel-native` 1.1 (the Rust
rewrite of the 12.6 MB Beszel agent, now done). No camera feeder and no `lumatv-gfn-feeder` was running.

### LumaGame (own app), hidden

| Process | pid | PSS | USS | Swap | Threads | CPU % |
|---|---:|---:|---:|---:|---:|---:|
| `gfx64` bundle (`lumagame-ui`, GL/Mali) | 15465 | 197.5 | 197.5 | 0 | 23 | 0.13 |
| `lumagame-main` | 15443 | 10.0 | 9.0 | 0 | 2 | 0.05 |
| launcher `sh` | 15456 | 0.3 | 0.3 | 0 | 1 | 0 |

Inside the 197 MB (`smaps`, 15:08): 126 MB anonymous (1.2 GB of address space: the GL and GPU buffers), 39 MB heap, 15 MB `libmali`, 8 MB a UI layer
(`layer-507.rgba`), 4 MB `lumagame-ui`. It has no swap, so every page is RAM [INFERENCE: LG's memory manager does not manage this native app's pages].
Main's figures for the same app: `CmaFree` 170 MB at LumaTV Home with kept apps closed, 82 MB at LumaGame's Home (about 88 MB of contiguous
memory lent), 3.5-20 MB during a GFN 4K120 stream, `MemAvailable` 649 falling to 399-440 MB during the stream. Today's read agrees: `CmaFree` 3.5 and
`MemAvailable` 400 with LumaGame resident, LumaGame hidden and idle.

### LG system services: top 31 by PSS + swap

| Process | pid | PSS | USS | Swap | Threads | CPU % |
|---|---:|---:|---:|---:|---:|---:|
| eim | 3459 | 2.5 | 2.3 | 27.7 | 17 | 0.17 |
| legacy-broadcas | 2255 | 13.4 | 13.1 | 6.2 | 89 | 0.72 |
| ss.apiadapter | 24999 | 13.5 | 9.8 | 5.5 | 13 | 0.02 |
| SettingsService | 4611 | 10.8 | 10.7 | 4.0 | 4 | 0.13 |
| WebAppMgr | 9332 | 4.7 | 3.4 | 8.9 | 28 | 0.05 |
| mojodb-luna | 3869 | 6.0 | 5.8 | 4.3 | 4 | 0.08 |
| nn-proxy | 3541 | 3.1 | 2.7 | 6.5 | 10 | 0.13 |
| mojodb-luna | 3673 | 6.1 | 5.9 | 3.4 | 5 | 0.0 |
| systemd | 1 | 8.5 | 8.3 | 0.2 | 1 | 0.0 |
| sdx | 3255 | 7.4 | 7.1 | 0.6 | 12 | 0.47 |
| ls-hubd | 1865 | 6.1 | 6.0 | 1.2 | 5 | 0.85 |
| pqcontroller | 2695 | 4.6 | 4.4 | 2.7 | 37 | 7.77 |
| audiod | 2218 | 3.8 | 3.8 | 2.6 | 4 | 0.02 |
| keymanager3 | 4163 | 4.7 | 4.5 | 0.7 | 3 | 0.0 |
| eplmanager | 2900 | 4.7 | 4.6 | 0.5 | 8 | 0.17 |
| gal_renderd | 1994 | 0.5 | 0.1 | 4.5 | 8 | 0.02 |
| webos-bluetooth | 3765 | 4.1 | 4.0 | 0.9 | 18 | 0.15 |
| com.webos.chrom | 4566 | 4.6 | 4.4 | 0.4 | 10 | 0.2 |
| starfish-media- | 2797 | 0.9 | 0.4 | 4.0 | 5 | 0.0 |
| audiooutputd | 2215 | 1.8 | 1.7 | 3.0 | 7 | 0.03 |
| sam | 2500 | 4.4 | 4.3 | 0.3 | 4 | 0.0 |
| physical-device | 2931 | 1.4 | 1.2 | 3.2 | 93 | 0.07 |
| LunaSysService | 3478 | 1.5 | 1.1 | 2.8 | 3 | 0.0 |
| lginput2 | 3639 | 2.9 | 2.7 | 1.3 | 13 | 1.9 |
| activitymanager | 5924 | 1.9 | 1.8 | 2.0 | 1 | 0.0 |
| btaudiosnk | 3570 | 1.0 | 0.6 | 2.8 | 3 | 0.02 |
| iepg | 2901 | 1.6 | 1.5 | 2.2 | 1 | 0.0 |
| update | 3146 | 2.0 | 1.5 | 1.9 | 6 | 0.02 |
| securitymanager | 4605 | 1.0 | 0.8 | 2.3 | 7 | 0.0 |
| extinput-integr | 3464 | 2.1 | 2.0 | 1.1 | 34 | 0.3 |
| btaudiosrc | 3578 | 0.8 | 0.6 | 2.0 | 6 | 0.0 |
| factorymanager | 3045 | 0.9 | 0.7 | 1.9 | 2 | 0.0 |
| panelcontroller | 3067 | 1.6 | 1.5 | 1.0 | 15 | 0.0 |
| webos-connman-a | 3793 | 2.0 | 1.7 | 0.6 | 3 | 0.1 |
| nais | 9628 | 0.3 | 0.0 | 2.3 | 1 | 0.0 |
| wowplay | 4452 | 1.4 | 1.3 | 1.1 | 28 | 0.03 |
| configd | 1912 | 1.6 | 1.6 | 0.9 | 3 | 0.0 |
| kadpd | 1954 | 1.7 | 1.6 | 0.8 | 5 | 0.43 |
| mojodb-luna | 3806 | 1.5 | 1.3 | 0.9 | 4 | 0.02 |
| lowlevelstorage | 1862 | 0.6 | 0.6 | 1.8 | 3 | 0.0 |

Not in the list: 98 more services, together 59.6 MB PSS and 56.1 MB swap, none above 2.8 MB (`factorymanager`, `panelcontroller`, `webos-connman-adapter`, `nais`, `wowplay`, `configd`, `kadpd`, `mojodb` accountdb are the largest; 49 are under 1 MB). The five `mojodb-luna` rows
(`maindb`, `epgdb`, `accountdb`, `mediadb`, `tempdb`) are 15.4 PSS together.

### Homebrew

`org.webosbrew.hbchannel.service` 6.4 PSS (+8.6 swap), `dropbear` 0.3 each (3 sessions: one is this note's), total 7.1 PSS. `node tvweb.js` (the adblock of
10-25) was not running.

## 3. LG's top 25: what each does, who needs it, class

Classes: (a) stop or never start; (b) start on demand; (c) needed, trim by config; (d) Rust replacement candidate; (e) leave alone. Evidence is
`ls-monitor -l` (which service names each process owns), the static luna roles in `/usr/share/luna-service2/roles.d` and `manifests.d` (who may call
whom), and the repo's own notes. Callers not observed on the bus are marked [INFERENCE]. Panel, power, tuner, HDMI/CEC, DRM/Widevine/PlayReady and
audio are (e) by rule.

| Process (MB PSS + swap, CPU %) | What it does | Needed under LumaTV? | Class |
|---|---|---|---|
| `eim` (2.5 + 27.7, 0.17) | LG external input manager: HDMI devices, CEC, eARC with the receiver (`com.webos.service.eim`) | Yes: the AVR follows the TV over CEC and eARC; Home's input picker reads `eim/getAllInputStatus` (`SURFACES.md`) | (e) |
| `legacy-broadcast-atsc` (13.4 + 6.2, 0.72) with `utp-atsc`, `iepg`, `dmost`, `bam`, `tv-reservation-agent`, `multiviewcontroller`, `epgdb`: about 25.6 + 16 | The ATSC tuner stack and guide database | Yes: the Live TV Tuner plugin serves the antenna tuner and the guide from it (`docs/plugins/live-tv-tuner.md`); `memory-result.md` showed Live TV gets no signal with `utp` stopped | (e) while the plugin or Live TV is wanted. Stopping it is the `livetv` group, not in `lean` |
| `ss.apiadapter` (13.5 + 5.5, 0.02) | SSAP: LG Connect Apps websocket on 3000/3001 (`com.webos.service.apiadapter`, `jsserver`, `secondscreen.gateway`) | Yes: Home Assistant's webostv integration and the test tooling talk to it (`declutter-result.md`) | (d), see below; (e) today |
| `SettingsService` (10.8 + 4.0, 0.13) | LG settings store (`com.webos.settingsservice`) | Yes: picture, sound, power, and Luma's own reads | (e). It was 6.9 MB at 14:48 and is 10.8 now, a store that grows with use [INFERENCE] |
| `WebAppMgr` main (4.7 + 8.9) and zygote (1.3 + 7.9) | LG's web app runtime (Chromium) | Yes for web apps (Settings, browser, Crunchyroll, GFN launch) and Home's web fallback; no renderer is up at Home | (e); (b) is what it already is: renderers start per app |
| `mojodb maindb` (6.0 + 4.3) | LG's main database (`com.palm.db`) | Yes: SAM, settings and launch points use it | (e) |
| `nn-proxy` (3.1 + 6.5), `nais` (0.3 + 2.3), `ai-inference-manager` (0.5 + 0.2): 3.9 + 9.1 | NPU inference front for AI picture, AI sound and AI upscaling | No other service's luna role lists them as a destination (static, `roles.d`), `nais` appears only in `sdx`'s caller list; `filmMaker` has AI picture off and `external_arc` has AI sound off (`memory-result.md`) | **(a)**: the `ai` group, built and never switched on. Needs an A/B on the picture path first |
| `systemd` (8.5 + 0.2) | PID 1 and LG's 309-process unit tree | Yes | (e) |
| `sdx` (7.4 + 0.6, 0.47) | LG's server-config client (`com.webos.service.sdx`) | LG's app store shows "service unavailable" without it (`declutter-result.md`, bisected); callers per roles: `appinstalld`, `sam`, `sm`, `mm`, `nais`, `certupdmgr` | (e). Costs 7.4 MB of RAM for the LG store; the owner's call, not a measurement |
| `ls-hubd` (6.1 + 1.2, 0.85) | the luna bus | Everything | (e) |
| `pqcontroller` (4.6 + 2.7, **7.77**) | picture-quality controller (`com.webos.service.pqcontroller`), 37 threads | Yes: picture modes, HDR, Dolby Vision | (e). The largest LG CPU user at Home idle: 7.8 % of a core. Not examined further; whether Luma's 10 fps wallpaper animation provokes it is [INFERENCE] and could be tested with a static Home |
| `audiod` (3.8 + 2.6), `audiooutputd` (1.8 + 3.0) | audio routing and output (`com.webos.service.audio`) | Yes: sound, eARC, passthrough | (e) |
| `keymanager3` (4.7 + 0.7) | LG key store (`com.webos.service.keymanager3`) | DRM and app keys | (e) |
| `eplmanager` (4.7 + 0.5) | OLED pixel refresh (`com.webos.service.oledepl`) | Panel care | (e). Never touched |
| `gal_renderd` (0.5 + 4.5), `kadpd` | LG graphics abstraction for the display chip | Yes: `cma-holders.md` shows it holds 32 MB of device memory | (e) |
| `webos-bluetooth-service` (4.1 + 0.9), `btaudiosnk` (1.0 + 2.8), `btaudiosrc` (0.8 + 2.1), `bthidmanager` (0.6 + 1.7): 6.5 + 7.5 | Bluetooth stack, audio sink/source and HID | The BLE sensors/presence feature talks to `bluetooth2` (`research/ble-result.md`); `bthidmanager` and `btaudiosrc` are allowed callers of each other and of `mrcu` (the remote) | (e) with BLE and the remote in use. The two audio helpers (1.8 + 4.9) are a (b)/(a) candidate only if Bluetooth audio is never used: not measured |
| `com.webos.chromecast.cell` (4.6 + 0.4) and provisioning (0.5 + 0.7) | Chromecast built-in | Home Assistant holds a Cast session (`declutter-result.md`) | (e) with HA on; the `cast` group (a) otherwise |
| `starfish-media-pipeline` (0.9 + 4.0), `umediaserver`, `videooutputd` | LG media pipeline and decoder control | The converter, Live TV and every LG app | (e) |
| `sam` (4.4 + 0.3) | Application manager | Everything | (e) |
| `physical-device-manager` (1.4 + 3.2, 93 threads) | USB and HDMI device manager | Gamepad dongle, storage | (e) |
| `LunaSysService` (1.5 + 2.8), `activitymanager` (1.9 + 2.0), `update` (2.0 + 1.9), `securitymanager` (1.0 + 2.3), `extinput-integration` (2.1 + 1.1, 34 threads, 0.3 %) | system service, scheduler, firmware update, security, external-input glue | Update must keep working (LG app-update block is separate) | (e) |
| `lginput2` (2.9 + 1.3, 1.9) | remote and input handling | Yes: the second-highest LG CPU user, 1.9 % | (e) |
| `memchute` (1.4 + 0.5, 0.73) | LG memory policy, 4 MB total with swap | Admission and closing (`lg-memchute.md`) | (d) only for the policy reasons in that note; no memory gain |
| `wowplay` (1.4 + 1.1, 28 threads) | WOWCAST/WiSA speaker link | Allowed callers: `audio`, `eventmonitor`; audio rule | (e) |

Nothing else among the 129 services is a stop candidate with evidence: the remaining PSS is the always-on core. The sum of everything marked (a)
is **3.9 MB of RAM and 9.1 MB of swap (eMMC)**. Adding the Bluetooth audio pair and the Cast pair if the owner dropped those features gives 8.4 MB of
RAM more. The whole Live TV stack is another 25.6, kept for the tuner plugin.

## 4. LumaTV's own: the obvious wins

| Item | Measured | Why it is an easy win or not |
|---|---|---|
| LumaGame hidden | 207.8 PSS, all private | It stays resident to start fast. It is an own app: release the GL context and the UI process on hide, or after a dwell with no stream, and start it again on launch. The reopen cost is unmeasured |
| `/tmp/perf` trace | 73 MB of RAM | The capture of a perf run left in tmpfs; the harness should write to flash or delete on completion (`CLAUDE.md`: diagnostics are deleted once analysed) |
| Helper's `luna-send` subscriptions | 13 processes, 5.3 MB, the helper's own design (`helper/app/src/ai/luna.rs`: the 64-bit static helper cannot link the 32-bit libluna-service2, so every query and subscription is a child process) | One 32-bit bridge process (the pattern of `compositor/sm/lumatv-sm.c`) carrying all subscriptions would be 1 process of about 0.5 MB, saving about 4.8 MB |
| Helper's `openssl s_client` keep-alives | 2 processes, 3.4 MB (both to `192.168.1.71:443`; what is behind it [INFERENCE]: the weather/news fetch or the Scrypted connection) | In-process TLS in the helper would remove the child. Saves 3-3.4 MB, adds a TLS library to the helper |
| Helper `arecord` | 0.35 MB, 0.23 % CPU | The ambient sound capture, running although Home is idle. It should start only while a sound mode is set and the lights are lit (`helper/app/src/ambient.rs:41`: this is its documented condition), so it either is set now or the stop is missing. Not checked: the ambient mode |
| Compositor | 100.3 PSS (heap 44, `libmali` 17.6, anon 14, two 8 MB UI layers) | For comparison LG's compositor it replaced was 72.5 + 12 swap (`memory-result.md`, 10-25). The heap of 44 MB is not attributed. The UI process adds 27.7 + 18.7 swap (heap 16). Together 128 PSS against 72, so LumaTV's drawing costs about 56 MB more than LG's; what fills the heap needs an allocator count |
| Helper itself | 30.7 PSS, 85 threads (24 MB anonymous) | Not attributed; the plugins are separate processes |
| `lumatv-jpeg` | 1.1 MB, 0.65 % CPU at idle | An image decode service doing work while Home is idle; why was not looked at |
| Compositor and UI CPU | 8.3 % + 5.4 % of a core, the largest at Home idle | Home's 10 fps wallpaper motion (`memory-result.md`); a panel-care or power question, not memory |
| Plugins | `home-assistant` 0.9, `beszel-native` 1.1 | Already small; nothing to win |

## 5. Ranked plan

MB figures are measured unless marked estimated. "Free today" means a stop, an on-demand start or a config or app change; "rewrite" means new code.

### Free today

| # | Item | MB saved | CPU saved | Risk | Effort | Verified by |
|---|---|---:|---|---|---|---|
| 1 | LumaGame: close the UI process (or release the GL context) while hidden and idle | 208 measured (about 88 of it shows in `CmaFree`: 82 against 170 in Main's runs) | none | LumaGame opens slower than from a prewarm; the cost is unmeasured | S-M, own app | reopen-to-first-frame before and after, `MemAvailable` and `CmaFree` at Home |
| 2 | Delete `/tmp/perf/*.trace` after the run, and write captures to flash | 73 measured (transient) | none | none once analysed | S | `df /tmp` |
| 3 | Helper: one bridge for the 13 `luna-send` subscriptions | 4.8 estimated (13 x 0.4 measured, one 0.5 bridge) | tiny | a bridge bug loses events the UI depends on | M | same subscriptions answering, helper tests, PSS sum |
| 4 | Helper: `openssl s_client` keep-alives into the process, or close them when idle | 3.4 measured | tiny | TLS code in the helper | M | fetches still validate the chain, PSS |
| 5 | Compositor and UI heap audit | 10-25 estimated of 44 + 16 heap | tiny | none | M (needs allocator counts) | PSS of both after a day of use |
| 6 | `ai` group (`nn-proxy`, `nais`, `ai-inference-manager`) | 3.9 RAM measured (+9.1 swap) | 0.13 % | AI picture, sound or upscaling stops; effect unmeasured | S, plus an A/B | picture modes unchanged in the test pictures, PSS |
| 7 | Bluetooth audio pair (`btaudiosnk`, `btaudiosrc`) | 1.8 RAM measured (+4.9 swap) | none | Bluetooth headphones and speakers stop | S | only if the owner never uses them |
| 8 | `arecord` only when needed | 0.35 | 0.23 % | none | S | process absent at Home idle |

Realistic total of items 1-6: about **300 MB** if LumaGame is released and the trace is deleted, **about 22-37 MB of permanent** savings without those two.

### Rewrite (parked list in `PLAN.md`, After LumaTV)

| # | Item | MB saved | CPU saved | Risk | Effort | Verified by |
|---|---|---:|---|---|---|---|
| R1 | `ss.apiadapter` in Rust | about 11 of 13.5 (estimated from `beszel-native` at 1.1 against the 12.6 it replaced) plus 5.5 swap | none: 0.02 % today | SSAP is how Home Assistant and the tests reach the TV; 71 public paths in 15 interfaces, 52 answered inside the service and 19 forwarded (audio, `secondscreen.gateway`, `networkinput`) | L: about 2,000-3,500 lines [INFERENCE: 52 handlers at 30-50 lines, plus the websocket, pairing and forwarding] | HA integration and the test tooling against the interface table |
| R2 | `memchute` in Rust | 0 | 0.73 % | admission policy | L | its own note; one policy instead of two, not memory |

## 6. For Nitin

**Worth it? Mostly no, apart from LumaGame.** Of about 500 MB normally free, the LG programs give back almost nothing: what can safely be
switched off is about 4 MB, because the rest runs the panel, sound, HDMI, tuner and copy protection. The Rust rewrite of `ss.apiadapter` would give 11
MB, about 2 % of the free memory, for weeks of work.

What is big is our own stuff:

| What | Size now | What to do | Gain |
|---|---:|---|---:|
| LumaGame sitting hidden | 208 MB | close it when idle, open it on demand | 208 MB (about 88 MB of it is the contiguous memory the video converter needs) |
| A 73 MB test recording left in `/tmp` | 73 MB | delete it, and have the test tool clean up | 73 MB |
| LumaTV's drawing programs (compositor and UI) | 128 MB (LG's was 72) | find what fills their heap | 10-25 MB, a guess |
| The helper's many small helper processes | 9 MB | fold them into one | 8 MB |

Total if all are done: about 300 MB at today's numbers, but 208 + 73 of that is LumaGame and a leftover file. Without them it is 22-37 MB, a small gain.

**Next step for you:** decide whether LumaGame should close itself when idle (opens a little slower, frees 208 MB). Nothing else needs your decision.

## Not measured

- The reopen time of LumaGame after a close, and what fills the compositor and helper heaps (needs allocator counts, a debug build).
- Whether AI picture or sound features change when the `ai` group stops; whether Bluetooth audio is used.
- Which driver owns the 172 `__mem_ops_alloc` buffers (258 MB); `cma-holders.md` has the same gap.
- Why `pqcontroller` uses 7.8 % of a core at Home idle.
- Netflix, Prime Video, Marquee, Apple TV and Live TV were not resident; their sizes are in `cma-holders.md` and `memory-result.md`.
