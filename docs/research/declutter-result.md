# LG declutter: groups, tool and results

Owner: LGDeclutter3 (restart of LGDeclutter, LGDeclutter2). Own paths:
`tools/declutter.sh`, `payload/declutter/**`, this note. Facts are from the set
(LG OLED65C4PUA, webOS 10.2.1, firmware 33.22.52) unless marked [INFERENCE].
Raw census and measurements: workstation `/tmp/lt/dc/` (`census*.tgz`, `live/`,
`meas/`).

## Resume here (2026-10-25 05:35 EDT)

- On the set, now: profile `lean` on and persisted (9 groups, `ai`, `livetv`,
  `cast` off), rebooted clean at 04:08; all verification below passed. The tool
  copy on the set is `/var/lib/lumatv/dc-dev/` (the version before this note's
  mask fix); state in `/var/lib/lumatv/declutter/`; interim boot hook
  `/var/lib/webosbrew/init.d/52-declutter`.
- Done in the repository, **not yet on the set**: masks moved to
  `/run/systemd/system.control` (see "nudge and user-context-manager"), and the
  `early` verb. Next slot: `python3 /tmp/lt/tv.py put` the three files
  (`tools/declutter.sh`, `payload/declutter/items.txt`) into `dc-dev`, run
  `declutter.sh on lean` (re-masks in the new place, one `daemon-reload`, no
  reboot), check `systemctl show nudge.service -p LoadState` is `masked`, then
  one reboot to confirm nudge and user-context-manager stay down (log line
  `early:` appears once the hook below is wired).
- Open: the hook in `tools/lumatv` (request at the end; SurfacesP2c wires it);
  the clean off/on A/B (needs a reboot for the stock side, no slot was left);
  group `ai` never switched on.

## Decision

Nitin's decision of 2026-10-25: telemetry, ads, promotions and app push are
removed, and webOS is trimmed to its essentials, through two reversible
profiles. `ads` is the first five groups; `lean` is `ads` plus four more. The
`ai`, `livetv` and `cast` groups exist but are in neither profile.

## What each group removed

| group | profile | removed | cost |
|---|---|---|---|
| uploaders | ads | `rdxd`, `remotelogger`, `crashd`, `service-logger`, `faultmanager`, `uploadd` | none; local logging stays. `faultmanager` had written 69 MB to flash in 2.4 h |
| ads | ads | `contentminer`, `admanager`, `adoverlay-service`, `acr2`, `sportsalarm`, `pbs` | LG promotional tiles and sports alerts stop. `pbs` had written 190 MB in 2.4 h; `acr2` ran at every boot although consent is refused |
| push | ads | `pushclient` (AWS IoT MQTT), `nudge`, `user-context-manager`, `iot-client`, `sdx`, `sdp-server-notice` | no LG nudges, no server-pushed app list or promotions |
| settings | ads | 16 switches held off in LG's settings store (Live Plus, live and home promotion, promotion video, viewing-history collection, third-party cookies, debug-log upload, usage statistics, ad cookie, personalised ads, screen-saver ads, sports alarm, AI nudges, content recommendation) | none; `off` restores every value as recorded |
| apps | ads | 8 Ball Pool and LG Movies & TV, the server-pushed stubs | both gone from the launcher |
| voice | lean | `voiceconductor`, `voiceinput`, `google-home-controller`, `nlpmanager`, Alexa and ThinQ triggers and adapters, `tts`, the voice performer, the Google Home runtime | Magic Remote microphone button, LG/Alexa/ThinQ voice and Google Home stop |
| connectivity | lean | `iconnectivity`, the rule engine, the ThinQ buddy connector, `iotjs` | LG ThinQ app control of the set stops |
| share | lean | Miracast, AirPlay, DLNA/UPnP, `appcasting`, TV Link, My Car, `ftms`, `mfi` | casting from phones through those protocols stops; DIAL (Netflix and YouTube cast) and Chromecast stay |
| misc | lean | CUPS printing sockets (ports 631 and 515) | none |
| ai (opt) | none | NPU inference manager, `nais`, `nn-proxy` | AI picture and sound stop; kept out of `lean` because its effect on the picture pipeline is unmeasured |
| livetv (opt) | none | the broadcast stack | Live TV and the guide stop |
| cast (opt) | none | Chromecast built-in | Home Assistant's Cast session and phone Cast stop |

How each kind of item is switched is in the header of `tools/declutter.sh`.
Units are stopped and masked in `/run/systemd/system.control`; on-demand
ls-hubd services are killed and a do-nothing script is bind-mounted over the
executable (the bind propagates into app jails; jailed services are only
killed); settings are held through the settings service; apps go through the
install service, their package kept in `declutter/backup`. Nothing under
`/usr`, `/etc` or the rootfs is written. The tool refuses to start if
`items.txt` names a unit on its deny list (SAM, WebAppMgr, both compositor
units, `tvpowerd`, `ls-hubd`, `configd`, `activitymanager`, audio, video, eARC,
CEC, input, network, power, databases, the install services, `alwaysready`,
`homelaunchpoints`, the SSAP front, the certificate timer).

## The pushed apps, and why they stay gone

At every boot SAM runs a customization sync: it waits for the network, the
install service and `sdx_auth`, `contentminer` calls `RequestServerInfo`, SAM
takes the country app list (`applications_dosci` in
`/var/palm/customization/launchpoints/USA/applist.json`) and the install
service adds every id that is missing and not in
`/var/preferences/com.webos.applicationManager/deletedSystemAppList.json`. 8 Ball
Pool was installed that way on 2026-10-13, LG Movies & TV on 2026-09-22. The
`apps` group removes both through `appInstallService/remove`, adds the ids to
the deleted list and strips them from every `applist.json` (then made
immutable); `push` and `ads` stop `sdx`, `contentminer` and the push client.
Proof: with the groups paused by the failsafe skip, a reboot ran the whole sync
(`new_applist: true`, `FinishInstall` 23-31 s after power-on) and neither app
came back.

## nudge and user-context-manager

They were stopped at every boot by the first pass of the hook and came back
within seconds. Cause, read from the set at 05:20:

- systemd's search path is `/etc/systemd/system` before `/run/systemd/system`,
  so a mask symlink in `/run/systemd/system` is ignored for units defined in
  `/etc`. `systemctl show nudge.service` read `LoadState=loaded`,
  `FragmentPath=/etc/systemd/system/nudge.service`. (Units defined under
  `/lib`, such as `cups.socket`, did read `masked`.)
- `nudge` ignores SIGTERM, so the tool killed it after 15 s; it has
  `Restart=on-failure` and `Requires=user-context-manager`, so the kill (result
  `signal`) restarted both (log: `killed: nudge.service user-context-manager.service`
  at 04:09:48, ActiveEnter 04:09:49, killed again 04:10:37, `NRestarts=1`).
- Independently, `webos-rbd.target` (04:09:28, user-context-manager) and
  `webos-bd.target` (04:09:36, nudge, `uploadd`, `ftms`, TV Link) are reached
  after the hook's first pass, and pull their `Wants`.

Fix, in `tools/declutter.sh`: masks go to `/run/systemd/system.control`, which
systemd searches before `/etc`; old masks in `/run/systemd/system` are removed
on contact. Tested on the set (05:20, `nudge.service` already stopped): one
symlink there plus one `daemon-reload` gave `LoadState=masked`,
`FragmentPath=/run/systemd/system.control/nudge.service`; sam, webapp-mgr and
surface-manager-daemon stayed active; the symlink was removed again and
reloaded. A masked running unit is not restarted by `Restart=`, so the kill no
longer loops [INFERENCE: not yet seen on a boot]. New verb `declutter.sh early`
only lays the masks down and reloads once (no stops, no luna, no settings); it
exists so the masks are in place before those two targets (+22 s, +30 s), which
needs the early boot hook (request below).

## Kept, and why

Every profile keeps what the apps and the setup need:

| need | kept | evidence |
|---|---|---|
| Netflix | `netflix.service` (jailed), DIAL (`com.webos.service.dial`, `ssdpmanager`) for casting from phones | `LSHUB_SRV_LNCH netflix.service` requested by `netflix`; DIAL started by the app's clients |
| YouTube | DIAL as above | same |
| Prime Video | `amazon.service` (jailed); the app starts it itself (`contentminer` used to) | reaches the foreground with `contentminer` masked |
| PlxNative, Moonlight | `umediaserver`, `videooutputd`, `tvrm` (decoder and video plane) | both launch with every group on |
| Apple TV, Crunchyroll | WebAppMgr only | both launch |
| GeForce NOW | the system browser (`com.webos.app.browser`, `browser-service`) | foreground app after launch is the browser, as at a stock boot |
| HyperHDR | the loader service, `hyperhdr`, `piccapautostart`, `captureservice` | loader launches; process and ports checked |
| Hue Sync | `com.lighting.huesync.service` (jailed), `color-info-miner` | `LSHUB_SRV_LNCH` requester `colorInfoMinerSync` |
| Homebrew Channel | its service, dropbear, `/var/lib/webosbrew/init.d/*` | the only way in |
| Home Assistant | SSAP (`xinetd` 3000/3001, `ss.apiadapter`, `network-input-service`, `second-screen-gateway`), `mdnsd`, `avahi`, `ssdp-discovery-lgtv`, Chromecast on 8009 | two SSAP sessions and the Cast connection from 192.168.1.146 |
| sound and CEC | `arccontroller`, `eim`, `audiod`, `audiooutputd`, `alwaysready`, `microphone` | AVR follows the TV over CEC, eARC active |

Left out of `lean` on purpose: `ai` (the NPU reads 0 % in Filmmaker mode, but
nothing measured that AI picture features are not in the path), `livetv`, `cast`
(HA holds a Cast session). `certupdmgr`, `memchute`, `preload-manager`,
`bthidmanager`, `webos-bluetooth-service` and `homelaunchpoints` are on the
deny list.

## Verification (set, 2026-10-25)

- One reboot per group (`uploaders`, `ads`, `push`, `settings`, `apps`,
  `voice`, `connectivity`, `share`+`misc`): set in Active Standby after a plain
  `reboot`, `sam` and `webapp-mgr` active, Home stub beating, `arcActivation:
  eARC`, HA entity answering, clock skew 0-1 s.
- Rebooted clean with `lean` at 04:08, checked by the main agent at 05:15:
  Netflix, YouTube, PlxNative, Apple TV (cold start, about 20 s), Crunchyroll
  and Prime Video launch; GeForce NOW opens the browser as before; the AVR is
  connected on HDMI 2 with sound on `external_arc`; HA sees the TV; LumaHome in
  front. Also checked after the last group: HyperHDR loader, Hue Sync and
  Homebrew Channel reach the foreground within 6 s; Moonlight on the second try.
- `declutter.sh status` at 05:19: `lean`, persist true, skip false;
  uploaders 7/7, ads 6/6, push 6/6, settings 16/16, apps 2/2, voice 16/16,
  connectivity 3/4, share 11/11, misc 3/3 in target. Which connectivity item
  is not counted was not looked at [INFERENCE: an on-demand service that is not
  running, which counts as not in target].
- Failsafe: a boot that never settled makes the next boot skip declutter once
  (seen: a reboot 70 s after power-on left the marker and the next boot
  skipped, as designed). `touch /var/lib/lumatv/declutter/skip` skips every
  boot until removed.

## Measured

Same procedure, Home in front, MemAvailable, swap and PSS from `declutter.sh
measure 30`, 100 s and 200 s after a plain reboot.

| | MemAvailable kB | swap used kB | processes | PSS sum kB | CPU busy % |
|---|---:|---:|---:|---:|---:|
| stock, 100 s | 683,372 | 120,324 | 380 | 887,950 | 28.0 |
| stock, 200 s | 703,500 | 210,688 | 376 | 799,812 | 15.5 |
| lean, 200 s | 720,632 | 13,596 | 338 | 877,582 | 10.0 |

Lean frees 38-42 processes and keeps swap near empty (13.6 MB against 211 MB
at 200 s) and 17 MB more MemAvailable; the PSS sum is within noise because LG's
own apps and the compositor dominate it. Per-boot flash writes of
`faultmanager` and `pbs` (about 2.6 GB a day together) stop. Measurement at
05:20 in Active Standby, lean, apps frozen by LG: MemAvailable 1,085,636 kB,
swap used 331,964 kB, 317 processes, PSS 188 MB. The CPU figures of that run
(68 % busy, `systemd` 65 %) are skewed by another job on the set and not
reported as a result; load average 14 is 12 kernel threads parked in
uninterruptible sleep (`msleep`) plus WebAppMgr frozen, not work. No
`declutter off` / `on` A/B was run (a stock side without a reboot is not
possible: the pushed apps and the jailed services only come back at reboot).

## Memory tuning

Run at 04:12-04:15 after the lean boot with `docs/research/perf/run.py
--workloads apphome --repeat 3` (LG Settings launched, HOME, 3 times), settings
reverted by the script (now read back: `vm.swappiness` 100, `vm.page-cluster`
3). Means of three runs:

| setting | app fps | home fps | frames over 33 ms (home) |
|---|---:|---:|---:|
| lean, as is (swappiness 100, page-cluster 3) | 39.3 | 23.8 | 27.3 |
| `vm.page-cluster 0` | 38.3 | 21.5 | 28.7 |
| `vm.swappiness 60` | 40.4 | 20.3 | 29.7 |

Neither improves the home phase, runs vary more than the differences, so
neither is kept. LG's f2io + zacc swap stays; no zram, no swap files.

## Undo

- Everything: `/var/lib/lumatv/dc-dev/tools/declutter.sh off`, then
  `declutter.sh persist off`. Units unmask and restart, stubs unmount, settings
  return to the recorded values, the two apps return from `declutter/backup` at
  the next SAM start (reboot).
- One group: `declutter.sh off --group voice` (any name from `declutter.sh list`).
- Profile `ads` only: `off --group voice --group connectivity --group share
  --group misc`, or `off` then `on ads`.
- If the set misbehaves at boot: `touch /var/lib/lumatv/declutter/skip` over
  SSH or Homebrew Channel, reboot; or remove
  `/var/lib/webosbrew/init.d/52-declutter`. Masks live in `/run` and the stub
  binds are mounts, so a reboot without the hook returns those to stock.
- Settings values and app removals persist across reboots, which is why `off`
  restores them from `declutter/before` and `declutter/backup` rather than
  relying on a reboot.

## Router-level steps for Nitin (instructions only, not done)

With the groups on, the daemons that called these names are down. Blocking the
names in the router's DNS (UniFi: Settings, Security, Content Filtering or the
local DNS record `0.0.0.0`) stops anything else on the LAN that still asks:
`US.info.lgsmartad.com`, `US.nextlgsdp.com`, `US.ibs.nextlgsdp.com`,
`AIC.tv.wiselg.com`, `ngfts.tv.wiselg.com`, `aic-ngfts.tv.wiselg.com`,
`aic.lgchhomeapp.lgtvcommon.com`, `aic.cdplauncher.lgtvcommon.com`,
`aic.cdpsvc.lgtvcommon.com`, `US.lgeapi.com`, `US.emp.lgsmartplatform.com`,
`aic-connect-client.lgthinq.com`, `AIC.api.lgtviot.com`, `cdn77.uphor.com`.
Glasshouse's `/etc/hosts` already blackholes
`ad.lgsmartad.com`, `ibis/ibs.lgappstv.com`, `rdx.lgtvcommon.com`,
`aic.lgtvcommon.com` and four ad networks.

## Request for `tools/lumatv` (SurfacesP2c)

Wire declutter into the app's existing hooks. (1) `tools/build-ipk.sh`: add
`tools/declutter.sh` and `payload/declutter/{dc.py,groups.txt,items.txt,stub.sh}`
to the file list; `declutter.sh` finds its app dir from its own path and needs
`tools/brand.sh` and `brand.json` beside it. (2) `install_early_hook`'s shim:
after the `autostart.sh --worker` line, inside the same `ONCE` guard, add
`"$APPDIR/tools/declutter.sh" early </dev/null >/dev/null 2>&1`; it returns in
about a second and only lays the masks down, before `webos-rbd.target` (+22 s).
Do not call `declutter.sh boot` from the shim as well: `boot` keeps a marker that
a second call would read as a boot that never completed. (3) `persist_enable`:
run `"$APPDIR/tools/declutter.sh" persist on` when
`/var/lib/lumatv/declutter/active` is non-empty, which re-points the init.d
link `52-declutter` from the dev copy to the app's copy (`persist_disable` and
`verb_revert`: `declutter.sh off` and `persist off`, then remove
`/var/lib/lumatv/declutter`). (4) A pass-through verb `lumatv declutter
on ads|on lean|off|status|list|measure|verify`. Until the ipk carries it, the
set runs the tool from `/var/lib/lumatv/dc-dev/`.

## Set check 2026-10-25 05:20-06:10 (main agent)

- LG's app store showed "Service is temporarily unavailable" with lean on. Bisected on the set by group, then by item: stubbing `/usr/sbin/sdx` (push group) breaks the store; with every other lean item on and `sdx` kept, the store loads its catalogue. `sdx` moved to kept in `items.txt`.
- Netflix playback failed twice (tvq-pb-101, "audio codec not supported" on audio-app-es) in an instance of Netflix that had stayed preloaded through the runtime group changes. After the Netflix process was ended and started fresh, Meridian played (Dolby Vision, 59.94 Hz) with lean on. Cause: the warm Netflix instance outlived services it had bound at start; a reboot or a fresh Netflix start clears it. `declutter.sh on/off` at runtime should end preloaded media apps afterwards (open item).
- The `system.control` masks (62a65e1) are deployed: `nudge.service` LoadState=masked.
