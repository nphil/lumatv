# The cold boot of 2026-10-29 15:13:40 EDT

Read-only evidence taken from the set at 15:14 to 15:17. The previous boot's logs were in tmpfs and are gone, so the cause of the
restart itself is not recorded; what the new boot recorded is.

- The kernel restarted at 15:13:40 (`/proc/uptime` 33.96 s at 15:14:14); the last reading of the old boot was 62347 s at 15:13:06, power state
  `Active Standby`. SSH was refused from 15:13:49 to 15:14:14.
- Power-on reason `remoteKey`; the micom's history of the previous power-off reads `OFF_BY_KEYTIMEOUT`, `utt_time` 4343 (`boot-excerpts.txt`).
  `/sys/kernel/oops_count` and `warn_count` 0, no pstore, no new LumaTV crash bundle after 14:33. `/sys/power/mode` is `resume` (instant-boot snapshot).
- LumaTV's compositor came up by itself: `enabled` `persistent`, `failures` 0, compositor started at 30 s of uptime, shell and helper running.
- Firmware unchanged: 33.22.52, webOS 10.2.1-5202; root shell intact (`update-state.txt`).
- Update: `com.webos.service.update` is `idle` with `nsuImageSize` 1769900424 (found version 33.31.69, 1687 MB, not downloaded). At 13 s of uptime the
  size was 0; the update service learned of the image at 29.7 s and posted the "update now?" alert at 31.8 s. The network came up at 22.7 s.
- The block (flag file plus the four LG update names to loopback in `/etc/hosts`) reads `blocked` now. Its `/etc/hosts` bind mount is made by the
  Homebrew boot hooks, whose service started at 27 s of uptime, so the update check at the network's arrival (22.7 s) most likely ran before the
  block existed [INFERENCE: the bind-mount time itself is not logged]. `checkUpdate` now fails with a certificate error, i.e. the block holds.
- Privacy: early pass `startSec` 13.73, `stubbedSec` 17.56, `maskedSec` 19.73; stubs over `pbs`, `acr2`, `admanager`, `adoverlay-service`,
  `sportsalarm`, `uploadd`; no log line of any of them in this boot. `iot-client` runs although its unit is masked: `ls-hubd` launched it as a
  Luna service at 36.8 s on request of `com.webos.service.homeconnect` and it talks to `aic.api.lgtviot.com` (`privacy-boot-timeline.txt`).
