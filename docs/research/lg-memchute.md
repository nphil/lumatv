# LG's memory manager (`memchute`, `com.webos.memorymanager`): interfaces, effects and a replacement plan

Research and plan, 2026-11-20. Nothing here is built or installed. TV facts were read on the set (OLED65C4PUA, webOS 10.2.1,
32-bit ARM userland on an aarch64 kernel) without changing it, from its firmware files, or from the luna captures in
`docs/research/arch/x4/` (2026-10-24); inferences are marked [inference]. The replacement folds into foreground priority
(`helper/app/src/app_priority/`), not beside it.

## Verdict

- memchute is LG's whole app-memory policy: launch admission, per-app cgroups, swappiness, freezing, reclaim, closing
  background apps, the low-memory level that WAM and others read, the preload gate, and LG-only extras (EFS, CRIU and flash
  hibernation, systrim, fault logging). The kernel only reports; nothing in the kernel closes apps before the OOM killer.
- Memory is not the gain: memchute holds 2 MB RSS and 644 kB of swap. The gain is one policy instead of two. Foreground
  priority already has to step around memchute's freezes, swappiness and keep lists, and memchute's closes are coarse: for one
  browser launch at 575 MB usable it closed Netflix, Amazon, Live TV, AirPlay and YouTube (x4, 27846.5), it closes LG Home
  (part of the Home-return gaps, PLAN Phase 1 item 3), and on 2026-11-21 04:16 UTC it closed Plex on a swap-free event.
- SAM never refuses a launch because of memchute's answer, but it waits for the answer with no timeout. A replacement must
  answer `setRequiredMemory` at once, always; everything else can be slower.
- It can be swapped without writing firmware, on the pattern of `compositor/sm/lumatv-sm.c`: a 32-bit shim bind-mounted over
  `/usr/sbin/memchute` owns the bus name and bridges to the helper, and falls back by unmounting itself and exec'ing LG's binary
  in the same process.

## 1. Process and units

`/usr/sbin/memchute`: 665,248 bytes, mode 0700, 32-bit ARM PIE, sha256
`446af3fef4da17b8e98cfeab09170eeceda1b14c2f04da0aacdd347919ef2912`. Links libluna-service2, glib, pbnjson, libcriu, libfm
(fault manager), libdile_*, PmLogLib. Seen live: VmRSS 2 MB, VmSwap 644 kB, `oom_score_adj` -1000; threads `memchute`, one
unnamed, `FM-watchdog`, `MemNotifier` and `mri_helper` (epoll loops), `tSystrim`; about 50 eventfds (cgroup v1 memory event
notifications [inference]) and a `NETLINK_CONNECTOR` socket on the proc connector group (fork/exit events).

| Unit | What it does |
|---|---|
| `memchute.service` | `DefaultDependencies=no`; `Requires=`/`After=ls-hubd.service mount-readonly-volatile.service`; `Type=simple`, `OOMScoreAdjust=-1000`, `EnvironmentFile=-/var/systemd/system/env/memchute.env` (absent), `ExecStart=/usr/sbin/memchute`, `Restart=on-failure`. Wanted by `multi-user.target`. |
| `memchute-ready.service` | `Requires=`/`After=memchute.service`, oneshot, `RemainAfterExit`, wanted by `webos-ibd.target`. Runs `scripts/memchute-ready.sh` (below), then `-/home/root/memchute_debug.sh` (absent). |
| `preload-manager.service` | `Requires=`/`After=ls-hubd surface-manager memchute`, `Type=idle`, `/usr/bin/webos-preload-manager`. |
| `snapshot-boot-components.service` | `systemctl start memchute.service` on a snapshot boot (`snapshot criu_enabled` on the kernel command line). |
| `cgroup-mem-perf.service` | No effect on this set: `memory.memsw.critical_perf` does not exist. |

Bus files: role `roles.d/com.webos.mm.role.json` (`exeName /usr/sbin/memchute`, names `com.webos.memorymanager` and
`com.webos.service.faultmanager.client*`), static service `services.d/com.webos.mm.service`, manifest
`manifests.d/memchute.manifest.json`, API groups `applications.internal` and `system` (`com.webos.mm.api.json`), `private` in
`compat.perm`. Outbound allowed to SAM, media, notification, service.bus, config, eim, faultmanager, tv.systemproperty,
tvpower, settingsservice, usercontextmanager, bootManager, connectionmanager, sdx, preloadmanager, pbsw, pmlogd, emmcd,
utp.dvr, utp.channel.

## 2. Luna API

`ls-monitor -i` lists 12 methods on `/` and the signal category `/com/webos/memory`. Callers come from the x4 captures (seen),
`ls-monitor -s` on 2026-11-20 (live subscription), and strings in the set's binaries (static).

| Method | Callers | Request → LG's reply | Replacement |
|---|---|---|---|
| `setRequiredMemory` | SAM before every launch, relaunch and preload (seen); `multiviewcontroller`, with its own wait timeout (static) | `{appId, requiredMemory (0; 160 for the browser), appType, defaultWindowType, containerBasedApp, relaunch, windowGroup, windowGroupOwner(Id), preload?}` → `{"returnValue":true}` in 0.4–4.4 ms; the freeing happens afterwards | Reply at once; the launch goes to the engine |
| `getCurrentMemState` | WAM every 5 s (seen); `gibbs`/`gibbips`, the ATSC 3.0 NRT controllers (static) | `{}` → `{currentLevel, usable, prevLevel}` (`normal`, 837 MB) | Level from the kernel's usable pages |
| `getCustomThreshold` | preloadmanager before each preload (seen) | `{appId, threshold:"100"\|"40", subscribe:true}` → `{subscribed, threshold, current}` (`current` = usable MB) or errorCode 9 "get custom threshold blocked" (Live TV preload) | Same shape; blocked while the preload status is `limit` (an `EfsTarget` app in front) |
| `getEFS` | emmcd (live subscription), the browser and Chromium `libcbe.so` (seen, static) | `{subscribe:true}` → `{EfsInfo:{activate:false, type:"dynamic", target:true}, HibernationInfo:{activate:true}}` | Fixed reply, both off |
| `getCloseAppId` | appcasting app `{appType:"web", subscribe:true}`, `gibbs`, `multiviewcontroller` (static) | `{}` → errorCode 4 "Invalid Parameters Error"; pushes `{id}` of an app being closed [inference] | Push before each close |
| `getUnitList` | `libSDL2` (`usable_memory`), the HbbTV WAM plugin, `make_oominfo.sh` and `re` (`{"procList":N}`, diagnostics) | `{usable_memory, F2IO_Stat, swapUsed, vmallocInfo, unitList:[…]}` | `usable_memory`, `swapUsed`, apps |
| `getGroupInfo`, `getPolicy` | none seen | groups with freeze/swap state; level boundaries and app lists | Same fields from the engine |
| `startMemNotifier` | `memchute-ready.sh` at boot | `{}` → `{"returnValue":true}`; starts the MRI/notifier threads | Reply true |
| `setMriHelper` | none seen | toggles the MRI helper | Reply true, ignored |
| `sendLowMemPopupTest` | none (test hook for the low-memory toast) | – | errorCode "not supported" |
| `getConfigs` | none; served on `/` with `provides: all` [inference: from its config client library] | – | Not registered |

Signals on `/com/webos/memory`: `thresholdChanged` on each level change (WAM, which closes web apps under `LOW_MEMORY_POLICY`
[inference], `gibbs`, Chromium); `clearContainers` `{"currentLevel":"normal"}` with each batch of closes (received by
`googlehomecontroller` in x4; WAM's container apps [inference]).

### SAM when the service is missing or slow

From SAM's `MemoryCheckerTV` (disassembled, sha256 `9202653e…6669`):

- `run` takes the next queued launch and calls `check_memory_status`, which sends `setRequiredMemory` with `LSCallOneReply`
  and no timeout. No timeout is registered anywhere on this path; SAM's only launch timeouts are the splash and last-loading
  timers.
- If `LSCallOneReply` itself fails ("lscall fail"), and when the reply has `returnValue:false`, SAM logs
  `fail_set_required_memory` (APP_LAUNCH_ERR) and `finish_memory_checking` goes on with the launch. The answer never cancels
  a launch.
- With no reply, the launch waits [inference: and so does every launch queued behind it].
- An unregistered name answers at once ("Service does not exist", 10 ms, probed with a made-up name). A name declared in
  `services.d` as static but not registered gets no answer: a call to `com.webos.db8.test.lunaservice` was held for the whole
  3 s timeout and started nothing. `com.webos.memorymanager` is declared static, so while memchute is down SAM's launches hold
  until it registers again [inference: the hub delivers the held call then].

Other callers without an answer: preloadmanager's timer cancels and skips that preload ("GetCustomThreshold Return fail. Not
preload this time."); WAM logs "Failed to request to memorymanager"; multiviewcontroller has its own wait timeout.

### What memchute calls and follows

- Calls: SAM `closeByAppId {id}` (SAM reports reason `memoryReclaim`, configured for caller `com.webos.memorymanager` in
  SAM's configd layer); `com.webos.media/closeMediaPipeline {pid}`; preloadmanager `setPreloadStatus {status:"limit"}` when an
  EFS target app comes to the front and `"start"` when Home does; `com.webos.service.pbsw/debug {removeEPG:false}` (pbsw is not
  running on this set: an error each time); `notification/createToast` (the "This app will now restart to free up more
  memory." toast, SURFACES: Replace); `faultmanager/registerAndNotifyFault`; `pmlogd/forcerotate`; `config/getConfigs`;
  `utp.dvr`/`utp.channel getLastChannelId`; `service.bus` registerServerStatus/addmatch; emmcd `checkLifetime`.
- Live subscriptions (`ls-monitor -s`): SAM `getAppLifeStatus` and `foregroundAppInfoEx` (both `extraInfo`), `running`;
  media `trackAppProcesses` (`procUpdate`, the media processes per app); tvpower `getState`; eim `getAllInputStatus`;
  connectionmanager `getstatus`; sdx `sdp_get_auth`; config `tv.model.sysType`, `system.debugstatus`; settingsservice
  `localeInfo`. At start also bootManager `getBootStatus` (waits for `rest-boot-done`) and usercontextmanager
  `getModeledRank` (ranks apps for closing).

## 3. What it writes

Cgroups (v1). memchute creates `/sys/fs/cgroup/memory/<appId>` and `/sys/fs/cgroup/freezer/<appId>` on launch ("create new
cgroup dir", "add group(netflix)") and writes the app's processes into `cgroup.procs`, from SAM's pids and media's
`procUpdate`. Per app it sets `memory.swappiness` (10 for the front app, 100 behind), `freezer.state` (`FROZEN`/`THAWED`),
LG's `memory.reclaim`/`reclaim_control` and `memory.memsw.dev_distance` (the swap device for flash hibernation), and
`memory.force_empty` when there is no hibernation device. Foreground priority's `Os::app_procs` reads these groups, so they
must keep existing after the swap.

Kernel and files:

| Path | Written by | Use |
|---|---|---|
| `/proc/sys/vm/swappiness` 100, `min_free_kbytes` 8192, `page-cluster` 3, `watermark_boost_factor` 0, `lowmem_reserve_ratio` first value 32 | `memchute-ready.sh` | Boot tuning, skipped when `/tmp/memchute_ready` exists |
| `/sys/kernel/mm/low_mem_notify/{freeze,ratio,workingset}` 1, 0, 80M; `DEFAULT_WORKINGSET` in systemd's environment | `memchute-ready.sh` | Kernel notifier set-up |
| `/sys/kernel/mm/low_mem_notify/{efs_shrink,force,freeze,reset,xreclaimable_ctrl}` | memchute | EFS shrink, forced events, reset (`freeze` reads 0 now) |
| `/proc/mri/{event_ctrl,freeze,reset}` | memchute | MRI event control |
| `/proc/sys/vm/compact_memory`, `drop_caches` | memchute | "DropCaches while app launching", compaction |
| `/sys/block/mmcblk0/queue/read_ahead_kb`, `/sys/block/dm-0/cache_flush` | memchute | I/O tuning around hibernation |
| `/tmp/memchute_ready` (`1923 1486 123`) | memchute | Marks a running instance for the ready script |
| `/tmp/systrim/`, `/tmp/systrim/<pid>/trim` | memchute | Asks processes linked to `libsystrim.so.3` (they create `/tmp/systrim/<pid>/end_watch`) to trim their heap at level medium/low, for the 25 daemons in `/etc/palm/systrim-conf.json`; 50 pid directories exist |
| `/tmp/faultmanager/general/{memory-log,unevictables}`, `/tmp/mri_stats` | memchute | Fault reports (empty now) |
| `/mnt/lg/cmn_data/efs`, `/home/root/efs.*`, `lgch_boost.service_switch` | memchute (reads switches) | EFS |
| `killall -s 9 ft_` | memchute | Kills `ft_*` processes under emergency [inference] |

It reads `/sys/kernel/mm/low_mem_notify/usable_pages` (its "usable" figure), zram0 stats, `/proc/gpu/`, `/proc/um/`,
`/dev/f2io-0`, `/proc/pressure/*`, `/etc/palm/{memorymanager,systrim}-conf.json` (the first is absent) and overrides in
`/home/root/`.

Kernel side. `low_mem_notify` and MRI are built into the kernel; no module is loaded. `/sys/kernel/mm/low_mem_notify/`
computes `usable_pages` (218,002 = 851 MB), keeps the level against `thresholds` (`0 2978 11706 17033 51200 56320 64000 *74453
661197` pages, the configured enter/leave values) and raises events; `/proc/mri/` (`mri_kthread`) raises events on level, PSI,
swap free, zsmalloc pages, unevictable pages, zram compression ratio and DMA32 usable. memchute is the only reader. The kernel
signals and never closes an app [inference: no kill path among the exported symbols]. PSI is at `/proc/pressure/`.

## 4. Startup order

1. `ls-hubd`, `mount-readonly-volatile`, then `memchute` (no default dependencies, so very early). It registers the name,
   subscribes, reads config and waits for bootManager's `rest-boot-done` before its full policy [inference: from the
   strings].
2. `memchute-ready` (in `webos-ibd.target`): kernel tuning, then `startMemNotifier`. memchute writes `/tmp/memchute_ready`, so
   a respawned memchute is not tuned twice.
3. `preload-manager` after it; preloads ask `getCustomThreshold` first.
4. SAM's first launch calls `setRequiredMemory`; a SAM that starts before memchute registers has its first launches held
   (section 2).

## 5. Policy and configuration

Effective configuration (configd `getConfigs`, the `distro_hd2160-normal` layer wins). Levels, in usable MB:

| Level | Enter below | Leave above |
|---|---|---|
| medium | 250 | 290 |
| low | 200 | 220 |
| critical | 45 | 66 |
| reboot | 0 | 11 |

Launch: `DefaultRequiredMemory` and `LaunchThreshold` 120 MB, `LaunchPolicy` 0, `clearBackgroundApp` true; timers
`LaunchTimeout` 500, `LowTimeout` 1300, `CriticalTimeout` 300, `CriticalDelay` 2000 (×`CriticalCount` 3), `GroupKillTimeout`
4500, `MaxTimeout` 7000, `RestartTimeout` 15000 ms; `EmergencyActionType` 3; `MriHelper` true; MRI event steps for PSI memory
30/35/40 and CPU 25/40/50, swap free, zsmalloc pages, unevictable, zram ratio and DMA32. Lists: `KeepAlive` (livehbbtv,
livedmost, liveibb), `KeepOnLaunchEx` (home, seniorhome, google.assistant, amazon.alexapr, familycare), `HomeGroup`,
`QuickLaunch`, `KeepAppHib` (lgchannels), `EfsTarget` (YouTube, Netflix, Amazon, Max, Chromecast and five more),
`BoostTargetApp`, `ExtraMemUnits` (videothumbnailer). Close order follows the priority ladder BASE/APPHIB < UMS < APP <
MODELEDAPP < INPUT < HOME < DIST < CRIUDUMP < EFS < FG < KA, with `getModeledRank` ranking apps inside a rung.

Observed actions: "request close by launch" (one batch when a launch needs more than is free), "request close by emergency"
and "Close BG apps (profile based) by Swap Free" (MRI swap-free events, 2026-11-21 04:16 UTC: Plex closed in 1,194 ms),
freezing on low memory, `setPreloadStatus limit/start` and `removeEPG` on every front change, "Try to make snapshot" (CRIU) on
close.

## 6. Replacement plan

### Shape

- `lumatv-mm`, a 32-bit C shim linked against the set's `libluna-service2` like `lumatv-sm`, bind-mounted over
  `/usr/sbin/memchute` so the role's `exeName` matches and `memchute.service` stays LG's unit. It registers
  `com.webos.memorymanager` with the 11 methods and two signals, keeps the subscriptions (`getCustomThreshold`, `getEFS`,
  `getCloseAppId`) and bridges JSON lines to the helper over a unix socket, with the kinds `lumatv-sm` uses (inbound, reply,
  event, ping/pong, fallback). It carries no LumaTV dependency beyond the socket protocol.
- The policy is foreground priority's engine. The helper side is a bridge server in `app_priority` that turns shim messages
  into engine events and engine state into replies and signals.
- The shim never makes SAM wait: `setRequiredMemory` is answered in the shim before it is forwarded, and the shim keeps the
  last level so `getCurrentMemState` and `getCustomThreshold` are answered even while the helper is away (computed in the shim
  from `usable_pages` and the thresholds above when it has nothing newer).

### Jobs taken into `app_priority`

| memchute job | In the engine |
|---|---|
| Per-app memory and freezer groups, `cgroup.procs` | `os.rs` creates them at `launching` and attaches SAM's `processid` and media's pipelines (children inherit the group) |
| `memory.swappiness` 10 front / 100 behind | Written by state; `memchute.rs`'s rule of not writing it applies only in stock mode |
| Freeze on low memory | Already there: freeze cached apps after 10 s |
| Hibernation reclaim | Already there: `memory.force_empty` on the oldest frozen app at medium pressure |
| Launch admission (`setRequiredMemory`) | A `launching` event with the required MB; if usable is below required + `LaunchThreshold`, close cached apps one at a time, oldest first, until it is not; launch closes are not held to the three-a-minute cap |
| Levels, `thresholdChanged`, `getCurrentMemState` | From `usable_pages` with the hysteresis table; PSI stays the close trigger in `pressure.rs` |
| Closes | Through SAM as now, plus `getCloseAppId` push before, `closeMediaPipeline` for the app's pipelines and one `clearContainers` after each batch; LumaTV's toast replaces LG's |
| Keep lists | `KeepAlive`, `KeepOnLaunchEx`, `HomeGroup`, `QuickLaunch` from the config layers (`memchute.rs` already reads them) are never closed or frozen |
| Preload gate | `setPreloadStatus limit` while an `EfsTarget` app is in front, `start` otherwise; `getCustomThreshold` answered from usable MB |
| systrim | At level medium and below, create `/tmp/systrim/<pid>/trim` for the whitelisted daemons that have a pid directory, at most once per level change |
| LG apps in the background (Live TV, AirPlay, input apps) | Closable last, after every third-party cached app, never frozen; Home never [decision for Nitin: the engine today never touches LG apps, and with memchute gone nothing else would close them] |

`memchute.rs` stays as stock mode: whenever LG's memchute serves the name (switch off, fallback), the engine runs exactly as
today beside it. The bridge's state (`ours`/`stock`) picks the mode; going to stock first restores everything from the journal.
At takeover the engine adopts memchute's groups: groups it finds `FROZEN` are thawed unless the app is cached, then handled as
its own.

### Dropped on purpose

- EFS (flash-backed foreground swap, gated on emmcd lifetime, device auth, network and an LG server switch): `getEFS` reports
  both off; `removeEPG` goes with it.
- CRIU snapshots and relaunch, flash hibernation (`memsw.dev_distance`, `cache_flush`, read-ahead): they write the eMMC; apps
  relaunch cold instead [inference: SAM's CRIU path needs memchute's snapshot].
- Fault manager reports, `memory-log`, `pmlogd forcerotate`, the low-memory popup test.
- MRI and `low_mem_notify` event control (`force`, `reset`, `efs_shrink`, `/proc/mri/event_ctrl`): PSI and `usable_pages` are
  read instead; LG's ready script still does the boot tuning.
- `drop_caches` and `compact_memory` during launches.
- Modeled-rank (`usercontextmanager`) ordering: recency, as Android's cached list.
- `killall -s 9 ft_`, `reboot` level actions [decision: the kernel OOM killer with the engine's `oom_score_adj` order is the
  last resort].

### Swap, fallback and switching back

- `tools/mm.sh on|off|status|early`, as `tools/ssgw.sh`: state in `/var/lib/lumatv/mm/` with an `enabled` marker. `on`
  checks the pinned sha256 of `/usr/sbin/memchute` and of the role file, bind-mounts the shim over the binary, and restarts
  `memchute.service` while Home is in front. `Requires=` makes systemd restart `memchute-ready` and `preload-manager` with it
  [inference]; the ready script skips its tuning because `/tmp/memchute_ready` exists, and preloaded apps are lost once.
  `off` removes `enabled` first, unmounts and restarts the unit. `early` from `tools/autostart.sh` swaps again at boot while
  `enabled` exists and always exits 0; memchute starts before it, so the first seconds of each boot are LG's.
- Fallback, in the shim as in `lumatv-sm`: unmount the bind (`MNT_DETACH`) and `execv("/usr/sbin/memchute")` in the same
  process, so the pid, unit and role stay LG's and the name is back within one memchute start. Triggers: no helper within 20 s
  of start, the bridge gone for 10 s, two missed heartbeats (5 s apart), or a name it cannot register. The shim's own crash
  restarts it through `Restart=on-failure`; an unfinished-run marker counts it, and after three failures in 10 minutes the shim
  writes `gave-up` with the boot id and execs LG's at every start of that boot, until the next boot or `lumatv mm on` (Try again).
- The helper watches the shim: a shim that stops answering pings for 15 s is killed (SIGKILL) and comes back through systemd.
  While the switch is on and LG's serves after a fallback, the helper runs `mm.sh on --auto` once Home is in front and the link
  has been gone 30 s, at most every minute doubling to ten, never after a give-up of this boot.
- The helper's stop sends `bye`: the shim then waits 60 s for it instead of 10 and hands back without counting a failure, so
  installs and helper restarts are not crashes.
- Escape: `lumatv mm off`, which also works with the helper stopped; the escape's stock-UI restoration runs it too.
- The switch is the `enabled` marker (`lumatv mm on|off`), off until accepted on the set; no Settings row yet. Its state, the
  last record and the give-up are in `GET /v1/state/app_priority` under `memoryManager`.

### Crash records and alert

As `ssgw`: each crash or fallback writes `/var/lib/lumatv/mm/crash/<time>-<n>/` with `reason.txt` (exit status, signal,
"stopped answering", or the fallback trigger), the shim's last 300 log lines, the level and usable MB at the time, and the last
20 calls (method and caller, no payloads); the newest 8 are kept and `pending` marks an unread one. The helper sends one Home
Assistant notification per crash through the saved link and helper.json's `appPriority.notify` service (ssgw's phone by
default), naming the reason, that LG's memchute is serving, and whether LumaTV's tries again or gave up until the next boot.

### Host tests (each under a second, fake clocks)

- Engine, in `app_priority/tests.rs`'s rig with the fake `Os` and `Lg`: launch admission closes the oldest cached apps until
  usable clears the need and leaves kept apps and Home; levels follow the enter/leave table with hysteresis; swappiness 10/100
  by state; groups are created and pids attached at launch; LG apps close after every third-party app; stock mode restores
  everything and returns to today's coexistence rules; takeover adopts frozen groups.
- Shim, built for x86_64 against a fake `libluna-service2` that records registrations and delivers calls (the same stub
  approach as `compositor/sm/build.sh`): replays the x4 callers (SAM's `setRequiredMemory` for launch, relaunch and preload;
  WAM's 5 s `getCurrentMemState`; preloadmanager's `getCustomThreshold` including the Live TV block; `getEFS` subscribe;
  `getCloseAppId {appType:"web"}`; `getUnitList {"procList":200}`) and compares each reply's fields with LG's recorded reply.
  `setRequiredMemory` is answered within 5 ms with the helper present, absent and hung. Fallback execs a stub at the bound path
  on each trigger; three failures exec at start.
- `tests/test_mm.py` for `tools/mm.sh`, as `tests/test_sysbus.py`: pinned hashes, refusal on another firmware, `early` exits 0,
  `off` removes `enabled` first.

### TV acceptance (with Nitin at the set; receipts recorded)

1. Swap with Home in front: `ls-monitor -i com.webos.memorymanager` shows the shim's methods; `memchute-ready` and
   `preload-manager` active; SAM's `setRequiredMemory` round trip no slower than stock (ls-monitor).
2. Launch each app type: web (browser, YouTube), native (Netflix, Plex), LumaTV and Marquee, LG apps (Settings, Live TV, an HDMI
   input), and preloadmanager's YouTube and Netflix preloads.
3. Netflix and YouTube warm launches back and forth: no closes while usable stays above the medium level.
4. The 450 MB hog, then a browser launch at low usable: cached apps close oldest first with LumaTV's toast, Home stays, and the
   Home-return gap is measured against the stock baseline (PLAN Phase 1 item 3).
5. WAM receives `thresholdChanged` at medium and low (dbg-log).
6. Active Standby and wake; reboot (LG's first, then `early`); a snapshot boot.
7. Crash injection: SIGKILL and SIGSTOP of the shim, the helper killed (shim answers alone, then falls back after 10 s), three
   crashes in 10 minutes, `lumatv mm off` with the helper stopped. Each checked for the fallback, no held launch, the record and
   the phone message.
8. Seven-day soak: closes per day, no launch held (SAM's `APP_LAUNCH` log), shim memory flat.

### Size [inference]

Shim about 800 lines of C and 300 of fake bus and harness; engine additions about 900 lines of Rust (bridge server 250,
methods and levels 200, launch admission 150, groups and swappiness 150, preload gate, systrim and pipeline closes 150) and 600
of tests; `tools/mm.sh` 200 and 150 of tests; supervision, records and notification about 300, mostly reused from `ssgw` and
`beszel`. The shim is about 1 MB resident against memchute's 2.6 MB.

### Risks

- Every launch depends on the shim answering; a bug that stops replies stalls the UI's launches until the 10 s fallback.
- Firmware updates can change memchute, the role or SAM's call; the pinned hashes stop the bind and leave LG's.
- Dropping EFS and CRIU changes nothing measured on this set (EFS is inactive) but is untested for its effect on Chromium and
  emmcd.
- Closing LG apps under pressure is new behaviour for the engine and needs Nitin's decision.

## Untested

- The hub delivering a held `setRequiredMemory` once the name registers again.
- `systemctl restart memchute.service` taking `memchute-ready` and `preload-manager` with it.
- Bind-mounting over `/usr/sbin/memchute` and registering the name from the shim with LG's role.
- `getCloseAppId` pushes and what WAM does on `thresholdChanged` and `clearContainers`.
- SAM's behaviour with a `setRequiredMemory` reply that takes seconds (only the code path was read).

## Sources

- Copied from the set and disassembled: `/usr/sbin/memchute`, `/usr/sbin/sam` (`MemoryCheckerTV::run`,
  `check_memory_status`, `handle_memory_status`, `finish_memory_checking`), strings of `webos-preload-manager`,
  `libWebAppMgr.so.1.0.0`, `libwebappmgr-hbbtv3-plugin.so`, `libSDL2-2.0.so.0.14.0`, `emmcd`, `gibbs`, `gibbips`,
  `multiviewcontroller`, `re`, `libcbe.so` (read on the set), the appcasting app's `main.js`, `/etc/faultmanager/make_oominfo.sh`.
- Units and scripts in `/etc/systemd/system/`, `/usr/share/luna-service2/`, configd layers in `/etc/configd/layers/`,
  `/etc/palm/systrim-conf.json`, `/var/log/dbg-log*` (memchute lines), `/sys/kernel/mm/low_mem_notify/`, `/proc/mri/`,
  `/sys/fs/cgroup/`.
- Read-only luna calls: `getCurrentMemState`, `getPolicy`, `getGroupInfo`, `getUnitList`, `getEFS`, `getCloseAppId {}`, configd
  `getConfigs`; `ls-monitor -i` and `-s`.
- `docs/research/arch/x4/{all.jsonl,pass2.jsonl.gz,pass3.jsonl.gz}`; `docs/research/perf-tweaks.md`; `compositor/sm/lumatv-sm.c`;
  `docs/research/ss-gateway-rust-plan.md`.
