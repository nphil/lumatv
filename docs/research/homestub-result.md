# Home stub on the set (LumaTV phase 1, step 3)

Measured on the LG C4 (webOS 10.2.1), 2026-10-24 and 2026-10-25. The mechanism
is in `docs/HANDOFF.md` ("The Home stub") and the plan in `docs/PLAN.md`
(Glasshouse, "Retiring LG's Home"). Code: `tools/lumatv` (`homestub`,
`homestub-boot`, `homestub-watch`), `tools/autostart.sh`, `payload/homestub`,
`payload/keyfilter/lumatv.js`, `payload/compositor/StarfishFullscreenContainer.qml`,
`payload/compositor/lumatv/{LumaHome,LumaKeyBridge}.qml`.

## Resume here

Step 3 is finished: the stub is on and confirmed, proven on the set, pushed.
State at 2026-10-25 00:50 EDT; the code on the set is the last commit of
`tools/lumatv` and `payload/`, and the docs are in the same commit.

**The set:** left in Active Standby (`tvpower powerOff {"reason":"remoteKey"}`;
no crash reports in `/tmp/var/log/reports/librdx`, which would turn that into
a reboot). Stub ON and confirmed (`config.json` `"homestub":true`), `persist`
on with the early boot hook installed (`kdump.env`, `earlybin/grep`; an
earlier `revert` test had removed them and `persist on` restored them), guard
running, `blockedSystemAppList/USA.json` md5 `26e1a4ce33512106d327aa77414ffd7c`
unchanged, AVR volume untouched (-44.5 dB), Picture Mode untouched. Last
`homestub status`:
`{"config":true,"state":"stub","mounted":true,"sam":"web","flutter":false,"guard":true,"beat":true,"faults":0}`.
To wake it: `tvpower powerOn {"reason":"remoteKey"}` then `turnOnScreen`.

**What passed:** everything under "What passed" below, on the final code, and
the last two reboots (stub at 15 s, Home in front at 40.4 s and 40.2 s, no
`flutterHome` log lines, no crash reports, no faults).

**What failed on the way, now fixed:** reboot 5 left a black panel. After the
boot apply's compositor restart, the tool's launch (37.4 s) went into a
WebAppMgr (WAM) that closed with the compositor (old WAM closed at 38.0 s, the
new one served at 39.1 s), taking the stub with it. `relaunch_home` then waited
30 s for pixels and took "Home running on the boot path" (the preload manager
had loaded Home at 68 s) for a boot into standby. A hand `launch
com.webos.app.home` fixed it. Fix in `tools/lumatv`: after a compositor restart
with the stub mounted the launch waits until `webapp-mgr.service` has a new
start time and `com.palm.webappmanager/listRunningApps` answers; a black panel
on an Active set is no longer taken for a standby boot. A manual `systemctl
restart surface-manager-daemon` also made the guard give LG's Home back
("without LumaTV's module") although the new compositor had the module; the
check now has to fail for 3 s. Both checked after the fix: two reboots (above)
and one manual restart with no fault. SAM's own `last_input_handler` still
launches Home at about 38.4 s into the closing WAM and loses it; the tool's
launch at 40.6-40.9 s is the one that lands.

**Not proven (open):** a power-on that opens Home starts Flutter Home about 1 s
before the stub is mounted at 15 s (bootd launches Home at 13.9 s; `reboot`
opens the last input, so it was not testable); the hold-Home escape was tested
with injected key events, not the physical remote; the page-in gaps of the
Home return are unchanged (not a goal of the stub). Next in the plan: heap
shim, then the Rust helper.

**Undo:** see "Undo" below. A black screen: `luna-send -n 1
luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}'`.

## What passed

- SAM takes the stub as Home within 0.1 s of `blockSystemApp` and
  `releaseBlockSystemApp`, with no SAM restart. `blockedSystemAppList/USA.json`
  stayed at md5 `26e1a4ce33512106d327aa77414ffd7c` through dozens of cycles.
- Trial: reverted at +120 s exactly, left an app in front alone; OK on the toast
  confirmed it (`config.homestub` true); `confirm` refuses when the stub is not
  mounted or LumaHome is not beating.
- Home key from YouTube and the browser reaches LumaHome (stub median 0.32 s,
  0.28-0.34; Flutter median 0.33 s, 0.30-0.34). Closing the last app lands on
  Home. Standby 20 s and wake: bootd launches the stub, beat continues. Back key
  causes no exit popup.
- Hold Home 650 ms: LG's Home in front 2.1 s after the press from YouTube; from
  LumaHome held 3.2 s LG's Home shows and Screen Zoom does not open. Injected
  evdev events, not the physical remote.
- Guard: stale beat gives LG's Home at about 5-6 s; a compositor restarted
  without the module gives it within 4 s; a LumaHome that fails to load (broken
  QML) within 1.5-2 s; `revert` removes the stub and the stock compositor shows
  LG's Home. Two faults in a row switch the stub off; a boot after an unfinished
  one skips it; 60 s of beats clear the strikes.
- Boot (reboots 2, 3): stub mounted at uptime 15 s from the early hook, Flutter
  Home never ran (0 `flutterHome` log lines), Home on screen at 39.7 s and
  40.5 s. Reboot 1 (first after an install) had a slow WAM and the stub came up
  at 65 s; reboot 4 (late hook only, after a revert test) mounted at 30 s.
- The AVR-S760H is still the HDMI 2 tile and `eim` input; volume steps of
  0.5 dB through HA moved -44.5 to -44.0 and back.
- Heartbeat transport: with `XMLHttpRequest` PUTs the guard twice saw the beat
  unchanged for 5 s with the timer ticking on time (one fault, one near miss
  caught by the file-age check). LumaHome and the key bridge now use LG's
  synchronous `TextFile`; beat, mode, escape, confirm and the load-error marker
  were re-tested on it (beat 1 Hz with 2-byte files, escape in 3 s, trial OK,
  load error to LG's Home in 3 s, key to Home 0.31-0.43 s).

## Memory and speed

| | Flutter Home | Stub |
|---|---|---|
| VmRSS | 71.96 MB background, 80.5 MB in front | 32.4 MB |
| PSS | 35.6 MB background, 45.6 MB in front | 15.3 MB (RssAnon 11.1, RssFile 21.3) |

Compositor and GPU after the Home key (harness `docs/research/perf/run.py`, two
runs each, 4 s window):

| Workload | Stub | Compositor run (ms) | GPU busy (ms) | Dropped/animated frames |
|---|---|---|---|---|
| Home key from an app | off | 312.6, 296.2 | 506.8, 472.9 | 3/5, 2/33 |
| Home key from an app | on | 191.4, 253.7 | 332.5, 542.0 | 4/5, 1/28 |
| Home key from the browser | off | 373.7, 390.1 | 492.4, 468.9 | 4/13, 2/14 |
| Home key from the browser | on | 239.1, 233.6 | 364.6, 363.7 | 3/4, 7/8 |

With a 450 MB memory hog the longest gap was 116.8 and 266.9 ms with the stub
off against 100.1 and 467.3 ms on; the compositor run time 418/354 against
382/357 ms. The stub stays in SAM's running list under the hog. Idle (10 s):
compositor 443/447 ms off, 446/435 ms on, GPU about 10 %.

## Did not move or failed

- The Home-return page-in gaps (the compositor paging itself back in after a
  big app) are the same with the stub: the saving is memory, not that stall.
  PLAN Phase 1 item 3 stays open.
- A boot cover (LumaHome drawn over a late stub launch) cannot work: the
  fullscreen view and its container are 0x0 and invisible while no surface
  exists. Removed; LumaHome is built at compositor start instead so the beat
  and load errors reach the guard at once.
- Holding `homeAutoLaunch` off was not needed: nothing brought Flutter Home
  back in any test.
- One guard fault 66 s after a guard start (23:58:12) with LumaHome's counter
  showing no stall, and one caught near miss (00:08:37). Both were beat writes
  held up in the network queue; fixed by `TextFile`, not reproduced since.
- A power-on that opens Home starts Flutter Home about 1 s before the stub
  mounts at 15 s (bootd launches Home at 13.9 s). `reboot` opens the last
  input instead, so this could not be tested.
- Hold-Home escape was tested with injected events, not the physical remote.

## Undo

- `lumatv homestub off`: stub removed, LG's Home back under LumaHome as before.
- `lumatv revert`: everything stock, early hook included; `lumatv persist on`
  puts the boot hook back. With `config.homestub` true and `persist` on, a boot
  mounts the stub again unless `homestub off` was run first.
- Hold Home 650 ms anywhere: LG's Home until the next `apply` or reboot.
- By hand: `umount /usr/palm/applications/com.webos.app.home`, then
  `luna-send -n 1 luna://com.webos.applicationManager/blockSystemApp '{"id":"com.webos.app.home"}'`
  and `.../releaseBlockSystemApp '{"id":"com.webos.app.home"}'`, then
  `luna-send -n 1 luna://com.palm.webappmanager/killApp '{"appId":"com.webos.app.home"}'`.
- Black screen: `luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}'`.
