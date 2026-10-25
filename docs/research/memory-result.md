# Memory: what uses the 2 GB, and the preload trim (2026-10-25)

Owner: MemorySlot-2. Own paths: `payload/preload.json`, `tools/preload.py` and the
preload comment in `tools/lumatv`, this note, `docs/research/memory/` (the raw
snapshots: `meminfo.txt`, `procs.tsv` with PSS, RSS, swap and VmSwap per process,
`slab.txt`, `swaps.txt`, `summary.txt`; `after-on/f2io.txt` has the swap device's
sysfs counters for the zram slot). Facts are from the set (LG OLED65C4PUA, webOS
10.2.1) unless marked [INFERENCE].

## Resume here (2026-10-25 11:25 EDT)

- On the set: `lumatv preload on` is on with the Netflix-only list (18 ids off,
  `/var/lib/lumatv/preload.json`; LG's list saved in `preload.orig`). The app dir
  carries this repository's `payload/preload.json` and `tools/lumatv` (comment
  change only); the files replaced are in `/var/lib/lumatv/exp/memslot-backup/`.
  Declutter is unchanged (`lean`, `livetv`, `ai`, `cast` off). Swap, boot
  arguments and `/usr` untouched.
- Way back: `lumatv preload off` restores LG's list and removes the state file;
  the apps load again at the next switch-on.
- End state at 11:21: Active Standby, `filmMaker`, `external_arc`, Home stub on
  and beating, helper running, LumaTV applied and live, the receiver off again at
  -44.5 dB (35.5) as found. `declutter.sh status` reads `settings 15/16` (16/16 at
  05:19); not touched here, one value drifted or was reset by LG.
- Open: Live TV cannot be made on demand with the declutter mechanism (below);
  the `iot-client` and `iotjs` stubs and the `ai` group are candidates for a
  later slot.

## Before and after

Both in Active Standby, Home stub resident, apps idle; "before" 10:59 as found,
"after" 11:19, 75 s after the power-off that ended the slot. `MemAvailable` is
the kernel's estimate of what apps can have without swapping.

| | before | after |
|---|---:|---:|
| MemAvailable | 675,352 kB | 732,868 kB |
| MemFree | 44,500 kB | 164,372 kB |
| swap used (of 614,396) | 349,572 kB | 301,748 kB |
| AnonPages (app heap) | 499,452 kB | 395,064 kB |
| Buffers + Cached | 612,860 kB | 551,136 kB |
| CmaFree (of 692,224) | 3,468 kB | 118,616 kB |
| Slab (unreclaimable) | 172,576 (88,200) kB | 168,224 (88,236) kB |
| processes with memory maps | 158 | 155 |
| PSS sum / swap sum | 647,884 / 335,080 kB | 548,695 / 260,820 kB |

With the screen on and Home in front the same trim read higher: 857 MB right
after the four dropped apps were closed (11:03), 914 MB once the Live TV, voice
and HDMI 2 apps were gone too (11:08), 720 MB at the second wake (11:21). The
standby "after" is lower than those because Netflix had played the test film in
between and LG keeps it warm at 129 MB (58 MB before), and the page cache was
churned by the app launches.

Per process, PSS / swap in kB:

| process | before | after |
|---|---:|---:|
| Prime Video `ignition` (preload) | 70,135 / 7,712 | gone |
| YouTube `cobalt` (preload) | 58,453 / 32,112 | gone |
| browser `chrome` (preload) | 39,299 / 4,860 | gone |
| LG Channels (WebAppMgr renderer, preload) | 12,374 / 21,468 | gone |
| HDMI 2 input app `com.webos.app.inputcommon` (preload) | 34,084 / 1,228 | gone |
| Live TV app `inputcommon` (LG loaded it at the 11:04 switch-on) | 41,765 / 40 | gone |
| voice app `com.webos.app.voice` (loaded at the same switch-on) | 39,025 / 40 | gone |
| `netflix` | 58,404 / 25,648 | 129,123 / 14,488 (after playback) |
| `surface-manager` (LG's compositor with LumaHome) | 72,500 / 11,980 | 70,208 / 17,328 |
| `eim` | 35,186 / 34,340 | 24,017 / 50,080 |
| WebAppMgr, all processes | 54,687 / 51,192 (5) | 65,398 / 30,620 (4) |
| `ss.apiadapter` | 22,775 / 6,328 | 22,312 / 6,860 |
| `node /var/lib/tvweb/tvweb.js` (Nitin's adblock) | 17,778 / 2,496 | 17,097 / 2,924 |
| `systemd` | 12,419 / 864 | 12,242 / 1,028 |
| the broadcast stack (`legacy-broadcast-atsc`, `utp-atsc`, `iepg`, `dmost`, `bam`, `multiviewcontroller`, `tv-reservation-agent`, epgdb, conf downloader, broadcast UI manager) | 19,800 / 19,040 | 13,756 / 6,180 `legacy-broadcast-atsc`, 8,179 / 36 `utp-atsc`, 4,059 / 1,600 `iepg`, rest under 2 MB |

## Why 660 MB at 10:55 against 1,085 MB at 05:20

The 05:20 figure (`declutter-result.md`) was taken 70 minutes after the 04:08
reboot with the set idle since: the PSS sum was 188 MB with the same 332 MB of
swap, so LG's apps had been frozen and their pages dropped from RAM. Between
06:00 and 10:55 the set was used with the screen on (compositor trial, Rust UI
runs, the web Home stub), the preload manager was restarted (pid 11102, after
the compositor and WebAppMgr restarts) and every switch-on loads the enabled
entries again: at 10:59 Prime Video, YouTube, the browser, LG Channels, Netflix
and the HDMI 2 input app held 273 MB PSS and 93 MB of swap between them, and
AnonPages stood at 499 MB. Nothing leaked; LG's warm apps were back. Keeping
them off the list is the only way to keep that memory free.

## The preload trim (kept)

LG's list (`com.webos.service.preloadmanager.supportAppList`, 22 entries) is
loaded at the preload manager's start, at every switch-on and again when an app
closes ("Retry preloading ... (onRunning)"): every enabled entry is loaded while
memory allows, in list order. Only four were running at 10:59 because memory
was short; after they were closed, the first screen-off/on cycle (11:04) loaded
Live TV (42 MB) and the voice app (39 MB), neither of which had been running.
"Only Netflix stays preloaded" (Nitin, PLAN.md Decided) therefore means the whole
list off except `netflix` and `com.webos.app.home` (the Home stub, criu mode):

| off | why |
|---|---|
| `youtube.leanback.v4`, `com.webos.app.lgchannels`, `com.webos.app.browser`, `amazon` | Nitin's decision |
| `com.webos.app.livetv`, `com.webos.app.livemenu` | Live TV is used rarely; it opens on demand to a tuned channel in about 12 s (checked twice); the live menu only runs inside it |
| `com.webos.app.voice`, `amazon.alexapr` | the `voice` group of `lean` stops every voice service, so the apps are dead weight |
| `com.webos.app.miracast`, `com.webos.app.miracast-overlay` | the `share` group stops Miracast |
| `com.webos.app.hdmi1` to `hdmi4` | LG preloads the input app of every port with a device on it ("Not allowed" for the empty ports): the receiver on HDMI 2 is an audio sink, never a watched source; the HDMI 2 app opens on demand in about 8 s (checked); the empty ports are listed so a device plugged in later is not preloaded either |
| `vudu`, `pooq`, `globoplaywebos`, `yandex.alice` | not installed; LG's list is generic |

`com.webos.app.inputcommon` is not LG's input picker (that is
`com.webos.app.quickinputpicker`, drawn by the compositor): it is the one binary
behind the HDMI and Live TV apps, so the 34 MB it held at 10:59 was the
preloaded HDMI 2 app and is gone with the list.

Tool facts: `setPreloadPolicy` is answered only while the screen is on; in
Active Standby the preload manager logs `Pending-setPreloadPolicy` and the call
runs into luna-send's timeout, so `preload on` in standby writes configd but
reports every id failed, and the helper's next screen-on applies the policy
(`log-helper`: "preload: 18 apps kept from preloading"). `amazon.alexapr`'s live
call was refused (an error reply, not a timeout) with the screen on; configd
holds it off and LG never loaded it. The voice app answers `closeByAppId` with
`returnValue true` and stays; SIGTERM ends it, and with the policy off it was not
loaded at the next switch-on nor by the retries LG runs after each app close.
After the trim the preload manager logs "disabled" for each listed id and "All
apps are preloaded" with only Home and Netflix running (11:05, 11:20).

Verified with the screen on, after the trim: Netflix plays the Dolby Vision test
film (`dolbyHdrCinema`, decoder pipeline active, 200 MB PSS while playing),
`filmMaker` back on Home; YouTube, Prime Video and Plex (`com.beb.plxnative`)
reach the foreground within 14 s; the store (`com.webos.app.discovery`), the
browser and LG Channels within 18 s; Apple TV took 48 to 54 s on a cold start
(about 20 s at 05:15; the earlier launch had been ended after 36 s, before it
drew); Home returned after each by the HOME key; sound output `external_arc`
throughout; the receiver muted and unmuted through Home Assistant, whose entity
stayed stale at "muted" after the unmute until the AVR went off (the helper's
direct reading and the Denon's status page showed mute off at -44.5 dB).

## Live TV on demand: not possible with the declutter mechanism

The `livetv` group (0/9, off) stops and masks `utp`, `iepg`, `dmost`,
`hybridtv-manager`, `broadcast-systemui-manager`, `broadcast-conf-downloader`,
`multiviewcontroller`, `com.webos.app.multiview` and stubs
`legacy-broadcast-atsc`. In standby the stack holds about 20 MB PSS and 19 MB of
swap. `utp-atsc` is a `Type=static` luna service (ls-hubd never launches it;
only its systemd unit does) and the Live TV app talks to it first: with
`utp.service` stopped, Live TV reached the foreground with no channel state, no
signal and no picture, and `tvpowerd`'s warm-off query and the broadcast UI
manager logged the missing service; nothing started the unit. With the unit
started again Live TV tuned channel `1_32_2_1_0_0_0` with `videoSignalState
good` within 12 s. Only `legacy-broadcast-atsc` is `Type=dynamic` (launched by
ls-hubd on call), and `utp-atsc` polls it, so killing it alone gains nothing.
Masking the group means Live TV stops; the group stays off.

## The other users

| user | size | purpose | verdict |
|---|---:|---|---|
| `eim` | 35 MB + 34 MB swap (24 + 50 after) | LG's external input manager: HDMI devices, CEC, eARC with the receiver | keep; on the declutter deny list |
| `surface-manager` | 72 MB + 12 MB swap | LG's compositor, with LumaHome's QML inside it | replaced by LumaTV's compositor (track C) |
| WebAppMgr | 55 to 65 MB over 4 processes | main, zygote, network service, the Home stub's renderer (10 MB); the LG Channels renderer (12 + 21 MB swap) is gone with the trim | keep; the stub needs it until LumaTV's compositor draws Home, web apps (browser, GeForce NOW) need it anyway |
| `ss.apiadapter` | 23 MB + 6 MB swap | SSAP (LG Connect Apps): Home Assistant's webostv integration and the key device the tests use | keep |
| `node tvweb.js` | 18 MB + 2 MB swap | Nitin's adblock with the /etc/hosts bind | report only |
| `systemd` | 12 MB | PID 1 with LG's unit tree (309 processes, 1,135 threads) | untouchable |
| `sdx` | 8 MB | LG's server config client | kept for the app store (declutter note) |
| `SettingsService` | 7 to 12 MB | the settings store | keep |
| Live TV stack | 20 MB + 19 MB swap | above | candidate: `livetv` group, costs Live TV |
| `iot-client`, `iotjs` (family care) | 4.6 + 2 MB swap, 1.6 MB | ThinQ IoT client and the family-care service; both run again although `lean` masks `iot-client.service` and kills `iotjs`, because ls-hubd starts them from their `services.d` entries when something calls them (the caller was not captured; the log had rotated) | candidate: `stub` items in `connectivity` for `/usr/palm/services/com.webos.service.iotclient/iot-client` and the family-care launcher; needs a slot to see who calls them; risk ThinQ only |
| `ai` group (`nn-proxy`, `nais`, `ai-inference-manager`) | 3 MB + 5.5 MB swap | NPU inference; `filmMaker` has AI picture off, `external_arc` has AI sound off | candidate; small gain, its effect on the picture pipeline unmeasured |
| Chromecast cell | 3.6 MB | the `cast` group | keep; Home Assistant holds a Cast session |

Swap (341 to 350 MB used, `/dev/f2io-0`): a compressed swap device (`zacc`,
xxhash64) written to the eMMC partition `/dev/mmcblk0p53`, not RAM; `zram0`
exists with size 0. At 10:59 the processes' swap summed to 335 MB: `eim` 34,
YouTube 32, Netflix 26, the LG Channels renderer 21, `surface-manager` 12,
WebAppMgr main 12, `legacy-broadcast-atsc` 10, db8 maindb 10, `SettingsService`
8, WebAppMgr network 8, Prime Video 8, Homebrew Channel 7, `ss.apiadapter` 6,
`ssg-ssdp-manager` 6, the Home stub renderer 5, the zygote 5, `keymanager3` 5,
the browser 5, the media pipeline 4, `gal_renderd` 4, and about 83 MB over a
hundred daemons under 3 MB each. The trim removed 66 MB of that. Per-process
VmSwap is in `procs.tsv`; the device counters are in `after-on/f2io.txt`; swap
settings were left for the zram slot.

Unreclaimable slab, 88 MB, unchanged by the trim: `kmalloc-4k` 27.5 MB (6,881
objects of 4 KB), `kmalloc-128` 7.8 MB, `vm_area_struct` 5.6 MB (31,000
mappings), `task_struct` 4.7 MB (1,395 tasks), `kernfs_node` 4.6 MB,
`signal_cache` 3.7 MB, `mnt_cache` 3.4 MB (10,875 mounts, the app jails), `filp`
3.4 MB (13,440 open files), `sock_inode_cache` 2.8 MB, `kmalloc-2k` 2.3 MB. The
kernel runs with `cgroup.memory=nokmem`, so none of it is attributed to a
process; the `kmalloc` pools belong to drivers [INFERENCE: the GPU, video and
network stacks, from their size and the absence of userspace owners]. It only
shrinks with fewer processes, threads, mounts and files. The reclaimable 84 MB
(`ext4_inode_cache` 16, `dentry` 16, `squashfs_inode_cache` 12, `radix_tree_node`
9, `inode_cache` 7, `ovl_inode` 7, `buffer_head` 5.5) is already counted in
MemAvailable.

CMA, 676 MB (`CmaTotal` 692,224 kB, `cma_on` on the kernel command line): the
contiguous reserve the video decoder, the HDR and Dolby Vision pipeline and the
GPU allocate from. It is part of MemTotal and lent to ordinary movable pages
(page cache, app heap) while idle: `CmaFree` read 3.4 MB at 10:59 (lent in full)
and 119 MB after the trim. When a decoder needs a contiguous buffer the kernel
migrates the borrowed pages out first (the X4 contiguous-buffer hold measured
on 2026-10-24). It is why `MemFree` reads 44 MB while `MemAvailable` reads 675
MB; no memory is missing and no boot argument was touched.

Page cache, 613 MB at 10:59: `Buffers` 206 MB is the block-level cache of the
dm-verity and squashfs reads behind `/usr`, `Cached` 407 MB the file cache; both
reclaimable and inside MemAvailable.

## Changes on the set and their way back

| change | way back |
|---|---|
| `payload/preload.json` (18 ids) and `tools/lumatv` copied into the app dir; originals in `/var/lib/lumatv/exp/memslot-backup/` | copy them back, or the next ipk |
| `lumatv preload on`: configd list rewritten (LG's copy in `preload.orig`), live policy sent, state file for the helper | `lumatv preload off` |
| the running YouTube, Prime Video, browser, LG Channels, Live TV, `livedmost` and HDMI 2 apps closed with `closeByAppId`; the voice app ended with SIGTERM | nothing; each opens again when launched (all were launched and closed again in the slot) |
| `systemctl stop utp.service` for the Live TV test | `systemctl start utp.service`, done at 11:11 (`utp-atsc` pid 2885); Live TV re-verified tuned |
| receiver muted through Home Assistant at 11:02 | unmuted at 11:17, volume unchanged at -44.5 dB; off again with the set at 11:20 |
