# Round 26 on the set (forge main a988b80a, includes 8b025405)

Installed with grant cf1e58be (1 use). Panel 59.9 Hz at 60, 119.9 Hz at 120 on every surface; pop-ups off, receiver muted.
The toast surface is not measured: the gate posts it through LG's createToast, which stalled for 20 minutes (the call was killed); it was left out of the second run.

Dropped (the compositor's own count), `burst_over20` in brackets:

| surface | 60 Hz | 120 Hz | round 23 (60 / 120) |
|---|---|---|---|
| home-dpad | 0 (0) | 1 (0) | 0 / 0 |
| quick | 0 (0) | 0 (1) | 1 / 7 |
| picker | 0 (0) | 0 (0) | 0 / 5 |
| settings-scroll | 3 (2) | 5 (8) | 0 / 41 |
| apps-grid | 2 (1) | 13 (3) | 0 / 3 |
| home-widgets | 6 (3) | 24 (17) | 2 / 3 |

apps-grid and home-widgets at 120 Hz regressed against round 23. `LUMATV_FLOOR=0` gave 8 and 22, `LUMATV_PAGE_COPY=0` gave 18 and 23, so neither the floor nor the page copies cause it. Settings-scroll at 60 Hz shows frames of 150-217 ms at the end of the walk (+7.2 s). Further analysis is with Frames120b.

Screenshots of Home, the quick panel and the picker were taken but their key presses used names the compositor does not take (it wants evdev codes), so they show Home only; no visual check was made.

## Camera full view over YouTube: not verified

YouTube (Big Buck Bunny by deep link) played full screen with `vwin apps` and `cameras.testEvents` on. The script that sent the camera event (Cat Feeder, animal) and OK lost its connection: the TV had rebooted (uptime 0 min at 14:19, boot about 14:18). Nothing from before the reboot survived (/tmp, compositor log), so the cause is unknown; the event may or may not be the trigger. After the reboot YouTube stayed on its splash screen for over a minute, so the check was not repeated.

End state: LumaTV compositor persistent, Home in front, 120 Hz set with `ctl refresh 120` (not through the gate), popups.on on, testEvents off, receiver 0.52 unmuted. /tmp/lumatv is gone with the reboot, so the next round needs a reinstall.

## Reboot cause: not recoverable

Read after the reboot: there is no /sys/fs/pstore, no ramoops, and /var/log (messages, bootd.log, resv_log) holds only the current boot (its oldest message is at 12 s uptime of this boot; the rotated files were all written during it). `/var/lib/lumatv/crash` has nothing after 2026-10-29 21:25, and the kernel fault archives in /var/log/faultmanager/kernel are from 24-25 Oct. The boot ran the normal path (bootd `normal-boot-done` at 70 s, LumaTV's apply ok at 72 s), with no recorded reset reason. Whether the camera event or a power cycle caused it cannot be told from the set. Popups were switched off afterwards (popups.on off).
