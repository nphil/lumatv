# OLED panel maintenance: what LumaTV can read to know a run is going and when it ends

Offline study (no TV access) for deferring restarts and updates while LG's panel maintenance runs on the C4 (webOS 10.2.1, O22). Everything below is a read; nothing here calls a method that starts, schedules, cancels or powers anything.

## Verdict

- **The end of a run is observable, and readable in standby without waking.** `offrsCount` (LG's counter of finished off-RS compensations) goes up by one and `offrsLastTime` moves to the panel hour when a run completes. Measured on the set: standby began 00:34:20, count 609 at +362 s, 610 at +407 s (`docs/research/compositor/slot-2026-10-29-r17/refresh-standby-watch.txt`), so a run took about 6 to 7 minutes; the same pattern again as run 612 to 613 in r20 (`slot-2026-10-29-r20/gate/standby.out`). The reads were `luna-send` every 45 s through the whole standby; the set stayed in Active Standby, and the run finished, so the read neither wakes the set nor aborts the run.
- **The start of a run is not observable.** No luna event, tvpower state, sysfs node or V4L2 control marks it. What exists is a rule: the run was *due* when the standby began, and the count has not gone up since. That is an inference, not a status flag (it is the rule already in `helper/app/src/refreshguard.rs` and `compositor/tools/refresh_guard.py`).
- **tvpower does not show it.** In r17 LG's power state read "Active Standby" at every sample from standby to the wake (the run ended inside that window), and the compositor's journal shows one `PreparePowerOff (active standby)`, one `PreparePowerOff (always ready)` and one `PreparePowerOn`, nothing for the run (`slot-2026-10-29-r17/gate-run/woke-journal.json`, `woke-result.txt`: rounds 1/1). The reason lists in `com.webos.service.tvpower` (`compensationStart/End/Fail`, `pnwashStart/DoneOff/FailOff/DoneOn/FailOn`, `oslbSaving/Fail`; `lgtv/tmp-research/census1/dc-census/x/etc/configd/layers/base/com.webos.service.tvpower.json`) are power-off and power-on request reasons, relevant when the SoC is powered off and the micom runs the job, which is not the Always Ready standby this set uses.
- **The kernel offers nothing.** In LG's GPL kdriver (`lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`) the only trace of the pixel refresh is the micom command `CP_WRITE_FORCED_PNWASH` (0xEE, a write: forbidden) in `chip/o22/ucom/ucom_o22_func.c`; the micom reads that exist (`CP_READ_POWER_STATUS`, `CP_READ_MICOM_PANEL_ONOFF`) have no node, and `/proc/lg/ucom/status` only prints cold or hot. `V4L2_CID_EXT_VBE_TPC_RECOVERY_STATUS` (`core/be/v4l2_vbe.c`, `include/webosext/.../v4l2-ext-vbe.h`) is the TPC dimming recovery calculation, which `eplmanager` exposes as "Temporal Peak Control" (`getTemporalPeakControl`, `lgtv/server/tvweb.js`), not the off-RS run; `GSR_STRESS_COUNT` is the stress accumulator. The compensation itself runs in the micom and T-CON, outside what the kernel exposes.

## The exact read (what LumaTV should poll)

Two luna calls, both proven on the set, both answered in standby:

```
luna-send -n 1 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList '{}'
luna-send -n 1 -f luna://com.webos.service.panelcontroller/getPanelUsageTime '{"subscribe":false}'
```

Reply shapes (slot r16 `features/refresh/raw/luna-replies.json`, r18 `readonly/step3-refresh-guard.txt`):

```
{"returnValue":true,"jbLastTime":4142,"jbCount":11,"jbInterval":500,"pnwashKeyLock":true,
 "offrsInterval":4,"offrsLastTime":4339,"offrsCount":611,"subscribed":false}
{"returnValue":true,"panelUsageTimeStore":0,"panelUsageTime":26037}
```

`panelUsageTime` is in ten-minute ticks (6 to the hour); `getPanelUsageTime` with `{}` fails schema validation (errorCode 3), it needs `"subscribe":false`. Off-RS numbers are whole panel hours, interval 4.

Decision rule for "maintenance may be running" (unchanged from the guard, now with its evidence):

1. At the moment standby begins (tvpower `getPowerState` leaves `ACTIVE`/screen saver; the helper already tracks this in `standby.since`), read both calls. `dueAtStart = usage_hours >= offrsLastTime + offrsInterval`, `countAtStart = offrsCount`.
2. While in standby and `dueAtStart`: maintenance is assumed running until `offrsCount > countAtStart` (finished; run succeeded) or the standby reaches 15 minutes (LG's own text says about 10 minutes; observed 6 to 7; a failed or never-started run must not hold updates forever) or the set wakes.
3. Poll every 60 s while holding (the test tooling already does this; r17 used 45 s). Two local luna calls cost nothing measurable. The helper currently reads only with the screen on; a deferral needs this standby poll, held only while `dueAtStart` is true and the 15-minute window is open.
4. Not due at standby start: nothing runs, nothing to defer.

Observed cadence for context: counts 609 at 4330 h, 610 at 4335, 611 at 4339, 612 at 4343 (r16 to r20), so a run is due about every 4 to 5 panel hours of use, and each standby over that threshold starts one.

## Other signals, graded

| Signal | Read | Standby | Start | End | Status |
|---|---|---|---|---|---|
| `oledepl/getPixelRefresherInfoList` `offrsCount`, `offrsLastTime` | luna | yes (r17) | no | yes | proven |
| `getPixelRefresherInfoList` with `"subscribe":true` (the reply already carries `subscribed`) | luna | unknown | unknown | probably | untested; may push on the counter change and save the poll |
| `tvpower/power/getPowerState` (subscribe) | luna | yes | no | no | proven blind to it (r17) |
| `/mnt/lg/cmn_data/pnwash/{state,jobScope,completedOffRsCount,autoOffRsLastTime,failAlertCount,autoJbLastTime,completedJbCount}` | file read, eplmanager writes them | files persist (protected list in `tv.config.json`) | maybe | maybe | `lgtv/server/tvweb.js` treats `state` starting with `2` or `jobScope` = `1` as running; no capture in either repo shows those values, so unverified |
| `/mnt/lg/cmn_data/var/spool/rdxd/pnwashLog*` | file read | n/a | maybe | maybe | named in the protected list (`tv.config.json`); contents never seen |
| `micomservice/listPowerOnOffHistory {"maxCount":10,"startIndex":0}` | luna, micom's own last-10 history | yes | no | after the fact | seen once with `OFF_BY_PNWASHDONE` and `OFF_BY_OSLB_SAVING` entries (`docs/research/arch/x4/all.jsonl` line 86); useful to confirm a run completed, not to see one in progress |
| `eplmanager` lines in `/var/log/messages` (tag `eplmanager`, class `EPL_CTRL`) | log read | yes | maybe | maybe | the awake capture shows the format (`docs/research/multiview/tv-2026-10-30/*/messages/messages`); no standby capture exists |
| `ls-monitor -j -e` passive bus capture | bus monitor, as `docs/research/arch/x4/x4-monitor.sh` | yes | would show any luna traffic of the run | would show | the way to find out what the stock software says |

## The stock behaviour, as far as known

- `eplmanager` (`com.webos.service.oledepl`, also the 3-second `capture` caller at 640x360 and the writer of the pnwash files) and `panelcontroller` run as systemd services; the maintenance is `eplmanager`'s (`eplmanager.service`, `eplmanager-oneshot.service` in the census).
- LG's own OLED Care page (`lgtv/research/lgsettings/main.js`, "Pixel Cleaning") starts a manual run with `oledepl/setPixelRefresherMode {"mode":"start_now"}` or `{"mode":"light"}` (Quick Check), and queues one with the picture setting `scheduledPixelCleaning` (`on` runs once at the next power-off; read as `getSystemSettings {"category":"picture","keys":["scheduledPixelCleaning"]}`, `off` on the set at the X4 capture). Its text says about 10 minutes on an OSLB panel (`panelcontroller/isSupportOSLB` returns `isOSLB`), about an hour otherwise (90 minutes on 8K), the TV must stay off, and turning it on stops the run and it starts over.
- The two writers (`setPixelRefresherMode`, `CP_WRITE_FORCED_PNWASH`) are never called by LumaTV; the schedule flag is only read. A queued manual cleaning (`scheduledPixelCleaning` = `on` at standby) is the one case the rule above does not cover: it can run 10 to 90 minutes, and its end is `jbCount` (or `offrsCount`, which of the two is untested) going up and the flag returning to `off`, both unverified.
- Wake aborts the run; LG retries at the next standby (`docs/research/refresh-guard-result.md`: 58 of 86 standbys from 10-25 to 10-28 were under 15 minutes, because the test rounds woke the set).

## TV test to confirm (all read-only; no wake, no write)

Needs one standby of 8 to 10 minutes with a run due, using the usual SSH path (Active Standby keeps SSH; Always Ready on). The existing `python3 compositor/tools/refresh_guard.py status` shows whether a run is due. Do not use `--force-wake`; do not touch the remote.

1. Before standby, record: both luna reads; `ls -l --full-time /mnt/lg/cmn_data/pnwash/` and `cat` of `state jobScope completedOffRsCount autoOffRsLastTime autoOffRsInterval failAlertCount completedJbCount autoJbLastTime`; `ls -l --full-time /mnt/lg/cmn_data/var/spool/rdxd/ | grep -i pnwash`; `luna-send -n 1 -f luna://com.webos.service.micomservice/listPowerOnOffHistory '{"maxCount":10,"startIndex":0}'`; `luna-send -n 1 -f luna://com.webos.service.panelcontroller/isSupportOSLB '{}'`; the picture setting `scheduledPixelCleaning`.
2. Start `ls-monitor -j -e > $TMP/standby.jsonl` and `luna-send -n 100 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList '{"subscribe":true}'` (`-n 100` keeps it open for 100 replies; `subscribed` in the reply says whether it took) writing timestamps to a file, plus `tail -F /var/log/messages | grep -i -E 'eplmanager|pnwash|offrs|compens|oslb|tvpowerd'`. Passive; stop all three after.
3. Press the remote's power key once (the set goes to Active Standby). From a second SSH session, every 5 s for 10 minutes: the two luna reads and the `cat` of `state` and `jobScope` with `date +%s`.
4. After 10 minutes (the count should have gone up by then; if not, give it to 15), stop the captures and read everything again; do not wake the set before the count has risen.

What each outcome decides:

- `offrsCount` rises about 6 to 7 minutes after standby, `subscribe:true` pushed at that moment: use the subscription as the end signal (no polling), keep the poll as the fallback.
- `state` or `jobScope` changes at a time that brackets the run (and the files' mtimes agree): they are a real start/end signal; adopt them, since the start would then be known and the 15-minute cap could be dropped for "until it says finished". If they stay the same across the run, drop the `tvweb.js` assumption.
- `eplmanager` or `ls-monitor` lines at the start of the run: they name the stock start event; record it here and consider a log tail as the start signal.
- Nothing but the counter: the rule above stays as the design, which is what the guard already assumes. Measure the run length (start inferred from standby) over several runs to set the cap; panel hours at the run (does `panelUsageTime` tick during it) also settle whether `offrsLastTime` is floor of the usage at standby or at completion (r17: last 4335 against usage 4335.5 after the run).
- After the run, `listPowerOnOffHistory` should gain an entry (the earlier capture suggests `OFF_BY_PNWASHDONE`); the order of the list (newest first, as inferred, or oldest first) is settled by comparing before and after.

Untested on the set: any run while Always Ready is off, a manual Pixel Cleaning, a failed run (`failAlertCount`), a standby shorter than the run with the guard off, and whether `getPixelRefresherInfoList` accepts `subscribe:true` at all.
