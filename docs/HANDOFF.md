# LumaGlass — engineering handoff

Everything a person or model needs to pick this project up cold: how the mod
works, what was measured on the set, which approaches are dead and why, and
how to measure again. Facts here were observed on one set unless marked
[INFERENCE]. Credentials and the set's address are deliberately absent; they
live with the owner.

Start with [AGENTS.md](AGENTS.md) for the component map, build and install path
and the standing rules; this file is the dated record behind it.

## Native Home stub on the set, 2026-11-11 (main 6414c3ed)

`tools/homestub-native-tv.sh` (stub `1a9e47e2`, compositor as installed): `launch-over-app` PASS. SAM ends and restarts the unregistered stub for a launch of Home, and skips that for a stub under about 3 s old (`skip_by_launching_time`, measured at 2.1, 2.7, 3.1, 3.6, 4, 6, 8 and 10 s); the earlier failure was the probe's own restart 2.7 s before, and the probe now waits that out. A later run failed `restart` and `app-again` on a race: SAM's launch event for the restarted stub arrived after the probe's launch of HDMI 2 and the compositor raised Home over it. Saver and wake were not run. The web stub is back on the set. Receipts: `/data/home/tmp/lumatv-homestub-20261111/`.

## LG unified service CPU repair, 2026-11-10 (main 88cf849e)

The private `webos-sysbus.node` repair is installed and enabled for the unified JavaScript service host only; the global LG module is unchanged. Installed repaired module SHA-256: `e8b2a6719af992df973ab243da11e4a0d49e6dd861e9a61677cacb45ac750717`. Original tools are backed up at `/var/lib/lumatv/backups/sysbus-88cf849e`. `lumatv sysbus off --restart` restores the stock launcher and running module. After the 2026-11-11 05:38 reboot, `lumatv sysbus status` reported the boot hook ran and the host (`ss.gateway`, 9 services: jsserver, igallery.preview, homeconnect, secondscreen gateway and client, universalqcard, billing, sportsalert, magicnum) loaded the repaired module with no restart needed. None of those services is in the video path.

Three 30-second screensaver samples with Home Assistant connected measured 0.0666%, 0.0666% and 0.0666% of one core, versus 21.446%, 21.245% and 21.380% before repair. All repaired samples retained the same process and active, owned screensaver. Home Assistant mute and restore reached the native audio service; volume 57 and ARC output were preserved. The conditional Rust replacement is unnecessary at this measured load.

Receipts: `/tmp/cs/18172b987bff/` (`sysbus-install-receipt.json`, `apiadapter-before-3x30.json`, `apiadapter-after-3x30.json`, `sysbus-ha-muted.json`, `sysbus-ha-restored.json`). For a broken client the repaired loop reads the error once and stops polling it; LG's bus library still closes that connection only after the JavaScript garbage collector frees the request objects, so the descriptor can outlive the request. One closed after 1.1 s; two others outlived a 3-s window and were gone within 5 min in the same process. The stock module has the same release path. Release latency beyond these three cases is unmeasured.

## LG Channels control, 2026-11-10 (main 6cabee76)

Installed UI `8aaa411479f226ccedd767c8379e826c3d37e358862f3c89142b6e15e6d6b827` and helper `2865cb902fd08034b6097e4ab563b0e45c2f87508478a1bc06fc63e2bc3b7500`. The TV UI switched LG Channels off and on; off reported no running LG Channels process and persisted through a helper stop and start. The prior native close trial removed a running LG Channels process. Antenna Live TV displayed moving broadcast pictures. Network tuner 2.1 returned 1280×720 MPEG-2 video and AC-3 audio; the lineup and XMLTV guide responded. Reboot and early-boot behavior remain untested.

Left on Home with LG Channels enabled, receiver on and unmuted at -30 dB. Receipt: `/data/home/tmp/lumatv-lgchannels-20261110/`.

## Bounded remote logging

`base.logging` owns `enabled` (false), `host` (empty), and `port` (514). The About > Logging page edits those schema-backed values. Addresses are bare IPs or DNS names, not URLs or host:port pairs. Invalid updates leave the stored configuration unchanged. `GET /v1/logging` uses the standard topic envelope; `active` means the UDP worker is configured, never that a collector received a packet.

`GET /v1/logging` reports remote-forwarding status, not application capture consent. `base.logging.enabled` gates UDP output only; bounded local collection is independent. No supported external-app capture or opt-in API exists. Optional application capture requires its own explicit consent; Stats or overlay visibility is unrelated.

Ordinary producers send nonblocking Unix datagrams to `/tmp/lumatv-log.sock`: one `<PRI>tag: message` record, at most 4096 bytes. The helper owns `/tmp/lumatv-logs/current.log` and `previous.log`, each at most 512 KiB including newlines. The aggregate retained ordinary log payload is at most 1 MiB. The collector refuses non-RAM filesystems, oversized existing files, symlink files, and a second owner; there is no flash fallback. Producers drop records rather than block when unavailable or full. Socket buffers, record buffers, native readers and bounded Details tails are separately fixed overhead, not part of the retained-payload limit. Legacy user log files are not deleted or maintained as aliases.
The two logging workers reserve 128 KiB stacks each. Every helper-owned Unix and UDP logging socket requests 8 KiB send and receive buffers (Linux doubles these values for bookkeeping); no application-side record queue grows with collector downtime.
The collector accepts only a private directory owned by its effective uid, and descriptor-verified RAM regular files with mode0600, one hard link, and a collector ownership header. Directory-relative opens/rotation are pinned to the verified directory descriptor. Each retained file starts with LUMATV-LOG-V1 and a newline, counted inside512KiB and ignored by exact-tag readers; unrelated existing files are refused without adoption or truncation. The private128-byte owner.lock also records the bound socket identity. Startup removes only a matching, inactive stale collector socket; an unrelated or active socket is preserved. The socket parent must be trusted private/sticky RAM storage, and shutdown leaves any inode/type/owner replacement untouched.



The passive native source tails only LG’s existing RAM-backed `/var/log/messages`, starting at the end and handling truncation and replacement. It does not replay compressed rotations or override LG’s filtering. Critical kernel records (severity 0–4) use an independent, nonblocking `/dev/kmsg` descriptor positioned at the end; `/proc/kmsg` and the global kernel log cursor are never consumed. Native source availability, errors, gaps and dropped submissions appear under `value.sources`; unavailable sources are not claimed as covered. Kernel records can also appear in LG’s stdlog, so duplicates are possible. Neither LG daemon nor its whitelist/configuration is replaced or reloaded.
The independent critical-kernel source is outside LG’s accepted stdlog filter; its only selection is severity 0–4. It leaves LG’s readers and policy untouched rather than claiming LG approved those additional records.


The 1 MiB limit does not cover LG’s pre-existing native logs, the separately retained 768 KiB compositor trace, or the existing eight bounded crash diagnostic archives. Crash Details continues reading failure snapshots, not the current log window. These intentional diagnostics are not ordinary logger fallback storage.

The gated installer must stage the helper, matching `tools/lumatv`, and matching producer components together. The canonical apply, boot and `helper start` paths remove only obsolete `helper.json` `log.maxBytes` atomically before helper execution, preserving `log.level`, every other configuration byte, permissions and ownership. All product `helper.json` writers take an exclusive flock on the stable containing data-directory inode before opening/replacing the file; currently the canonical migration is the only production writer, while the helper itself only reads this process configuration and keeps runtime settings elsewhere. Invalid JSON or a detected external content/metadata change aborts migration without replacing the file. This serializes cooperating product writers; it does not claim compare-and-swap protection against an arbitrary external root editor in the final check/replace window. A helper-only binary replacement bypasses this mandatory migration. The product never ships a home collector address as a default. An owner-specific initial enabled/address/port change is a separate gated action. No native service configuration, firewall, port, DNS or collector changes are required.

The Netflix-button gate decodes `app:<id>` and `nothing` choices and checks mapped/foreground results, not journal phrasing. Other configured action types are reported as unsupported without invoking the button.

## Home Assistant weather sun times and UV, 2026-11-10 (main 82f1a3cc, installed 17:08 EST)

Helper `eead0c88` replaced `7dce7f81` in the package and state directory; ui `9933fdd9` replaced `0f24fb79`. Compositor `eeb157e7`, sm `65f6ca69`, `tools/lumatv` `0473b152` and `helper.json` are unchanged. The helper and UI restarted without a reboot.

Accepted on the TV with Pirate Weather still selected: today's sunrise 07:05 and sunset 17:38, overview UV PEAK 5 Moderate, Details UV TODAY 5 with Peak · Moderate, and a separate Cloud cover 0% row below the humidity gauge (34%). Light and dark at normal and 200% text, current UV 0 taking precedence over a daily peak, and missing UV without a marker were checked on the host only. Other TVs and other Home Assistant weather providers are untested.

Receipts: `/tmp/cs/18172b987bff/weather-tv/` (`installed.json`, `weather-details-state.json`, `acceptance.json`, `as-left.json`, `session.log`; rollback binaries in `backup/`). Left Active on weather Details, receiver settings untouched, lease released.

## Exact half-step volume to phones, 2026-11-09 (main f5afe326, installed 20:27 EST)

The phone remote's state now carries the receiver's `level` and `max` unrounded (41.5, 72.5), for LumaRemoteKit 1.0.3 / Marquee 0.1.29, which the one paired iPhone runs (`devices.json` app "Marquee 0.1.29"). Helper-only install with the TV in Active Standby: `lumatv-helper` 7dce7f81 replaced 658d0a76; ui 0f24fb79, compositor eeb157e7, sm 65f6ca69 and `tools/lumatv` 0473b152 unchanged. The plugin is on and listening on 18733. Not tested: the phone showing a half step against the receiver (receiver in standby at install).

## Saver after Home's dim: camera pop-ups no longer restart LG's timer, 2026-11-09 (main c86426e9, installed 20:11 EST)

LG's screen manager restarts its ScreenSaver timer every 30 s (`startScreenSaverResetTimer`) while `getForegroundVideoWindow` lists a `fullScreen` video. The compositor listed each camera pop-up's element at the feeder's whole-screen rectangle, so every pop-up counted as a full-screen video: LG's dbg-log of the 18:36 idle run has 8 `removeNopTimerHandler ... ScreenSaver:180(option:reset)` by `screenmanager` between 18:36:26 and 18:41:57 EST, each after a `full screen playing` verdict for `org.nphil.lumatv.cameras` at a `vwin: claim camera-popup granted` (1348,145 520x292). That confirms the [INFERENCE] in the idle timers entry below; the panel dim's 61 backlight writes in that run were followed by no restart. Each element is now listed where its claim placed it, `fullScreen` only when it covers the output. Home's dim level moved only at the 30 s beats (first step at 3:33, floor at 5:12) and now follows the stillness at every step. The weather view's UV gauge has no knob when the UV index is unknown.

Standard cutover with the TV Active on Home, a toast first: ui `0f24fb79` (replaced `a4d0a797`), compositor `eeb157e7` (replaced `fba0b770`); sm `65f6ca69`, helper `658d0a76`, `tools/lumatv` `0473b152` and `helper.json` unchanged.

- Idle, accepted, one run, Home left alone after one key at 20:11:50, sampled every 2.5 s: the dim began at 3:00 (want 0.99 at 3:05, then 0.01 every 8 s) and reached 0.85 at 4:58; LG's timer counted down from 298 s without a restart and the saver was up at 5:01 (4:58 the sample before). LG's dbg-log over the run has no ScreenSaver restart and no `full screen playing`. One key took the saver down and Home back to full within 3 s.
- Weather Details with the feed's UV index null: a dash and the bare scale, no knob (`checks/uv-details.png`).

Not tested: a camera pop-up during the idle wait on the set (no camera event arrived in the run; the vwin test covers the listing), other dim and saver waits. Left: Active Standby by the power key, receiver off and unmuted at -19.5 dB, `helper.json` as found, lease released. Receipts, not in git: `/data/home/tmp/lumatv-saverdim-20261109/` (`session.log`, `as-found.json`, `found2.json`, `rel/installed-c86426e9.json`, `rel/backup/`, `checks/idle7.json`, `checks/idle7-logs.txt`, `checks/idle7-saver.png`, `checks/uv.json`, `as-left.json`, `tvlogs/` with the 18:36 run's dbg-log).

## Backyard on TCP in Rebroadcast, 2026-11-09 (Scrypted setting only; nothing installed on the TV)

Backyard (camera 88) went grey on Home for 2 to 6 s after a return from an app because Rebroadcast read its two streams with `FFmpeg (UDP)` and lost the key picture. Its RTSP parser (`prebuffer:rtspParser-channel0` for Stream 1, `-channel1` for Stream 2) is now `Scrypted (TCP)`, as on the other cameras, set at 18:56 through the Scrypted client (`putSetting`). To revert, put both back to `FFmpeg (UDP)`; the settings before are in `backyard-prebuffer-before.json`.

- Tile, five rounds each (20 s in Marquee or YouTube, alternating, then Home; the helper's picture for the tile, 429x209, asked every 0.5 s for 15 s; flat = sd under 4, matched by panel captures): before 4 of 5 rounds grey, 0 / 6.7 / 2.0 / 6.3 / 2.7 s (17.6 s in all, longest span 6.7 s); after 0 of 5, 0 s. The first answer for the tile still takes 1 to 2 s in some rounds (before 2 of 5, after 3 of 5), which is the feeder's start and not the parser.
- Pop-up and full view after: Backyard pop-up picture at 0.08 s, live at 0.56 s; Back Door 0.17 and 0.48 s; Backyard full view live at 1920x1080 h265 (capture 19:02:39).
- NAS, 30 s windows with the TV on Home: the scrypted container used 1.53 and 2.27 cores before, 1.23, 2.09 and 2.23 after, which varies with the NVR and detection more than with this change. The Rebroadcast process (`@scrypted/prebuffer-mixin`) used 0.24 to 0.25 cores after against 0.247 over its whole 5.7 days; Backyard's two ffmpeg used 0.01 before. Camera traffic is 2.2 Mbit/s over TCP after (`ss` bytes received); the same streams are declared 2.25 Mbit/s before (a per-flow count of UDP is not available, conntrack accounting is off).

Not tested: a camera or Scrypted restart with the TCP session, a run longer than five returns, Stream 1 or 2 alone. Receipts, not in git: `/data/home/tmp/lumatv-backyardtcp-20261109/` (`before.json`, `after.json`, `before-r*`, `after-r*`, `pop-after.json`, `full-open.png`, `nas-*.txt`, `as-left.json`). TV left as found: Active Standby, receiver off and unmuted, `helper.json` as found, lease released.

## Idle timers, status bar, perf, Apple TV and feeder install, 2026-11-09 (main 872f9363, installed 17:53 EST, rebooted 18:04)

Standard cutover from Active Standby: app package files (`items.txt` f6685b2a, `theme.json` 28cb6c8a, `layout.json` ebdc79e3), ui `a4d0a797` (replaced `0e7230c6`), compositor `fba0b770` (replaced `6cded4b2`), then helper `658d0a76` (replaced `30f067a6`); sm `65f6ca69` and `tools/lumatv` `0473b152` unchanged. The builds carry cbaa4dbc (idle timers), 346f367d and 4b1e9885 (perf), 3f6e3759 (status bar; Nitin approved the Full width designs 9 Nov 17:40), 7eb35094 (feeder kept 30 min), d57e1a46 (Apple MFi kept) and the test and clippy fixes 44dad141 and 872f9363. Rebooted from Active Standby at 18:04 so the `mfi` bind goes.

- Wake, before and after the reboot, accepted: Active in 3.9 and 3.8 s, the first RIGHT moved the focus, Quick Settings full, no `at most` or `scene is full`. The log's `room` is what is left of 512/256/64. Used: Home 122 layers, 44 groups, 12 tracks; Quick Settings open 170/68/13, the same over five opens and closes; Settings 171/35/15, back to Home's numbers after. The 13:11 peak was 176/68/13.
- Apple TV after the reboot, accepted: `/usr/sbin/mfi` not mounted, window 1.3 s after start, in front at 2.3 s, on the profile picker.
- Gate at 120 Hz, 3 cycles, dropped / bursts over 20 ms: home-widgets, quick, switcher and keyboard pass at 0/0; apps-grid 2/2, settings-scroll 1/0 and home-dpad 1/0 fail, home-dpad also on one first frame of 478 ms (the others 6 and 20 ms). The afternoon's tvc6 run passed 2 of 7 (home-dpad first frame 346 ms; 195 ms after the focus-dim removal).
- Idle, accepted: `homeDim` 180 s over 120 s, LG's ScreenSaver timer 300 s, both Settings pickers offer 1, 3, 5, 10, 15, 30 min, 1 h, 3 h and Off, LG's values unchanged. Home left alone dimmed in four steps to 0.85, starting between 3:23 and 3:39 and done by 5:13, and one key brought it back. The saver did not come in 8 min: LG's timer restarted seven times, each when the panel dim let go, while the Plant Room Cat Feeder camera popped up for a cat [INFERENCE: the pop-ups restart it].
- Status bar Full width and the ticker switch in the layout editor, accepted; there were no unread notifications. Sound shows the receiver. Weather shows dashes for UV and sunrise and its labels fit; the UV gauge's knob still sits at 0 under the dash.
- Cameras: Front Door pop-up live in 898 ms on a new feeder, Back Door in 485 ms; OK held on a pop-up opens its snooze menu; Office full view main h265 3840x2160, live in 168 ms. Backyard after 20 s in Marquee had a picture at 1.1 s, then was flat grey from 5.1 to 8.6 s (the lost key picture above).

Left as found: Active Standby, receiver off and unmuted at -22.5 dB, `helper.json` as found (a test event answers 403), lease released. Not tested: the status bar's glyphs and ticker with unread notifications, the saver after 5 min of idle without pop-ups. Receipts and the rollback copies: `/data/home/tmp/lumatv-integrate-20261109/` (`session.log`, `rel/installed-872f9363.json`, `rel/helper-installed.json`, `pkg-installed.json`, `reboot.json`, `w1.json`, `w2.json`, `appletv.json`, `gate/`, `qs-cycles.json`, `checks/`, `pop-int.json`, `okhold.json`, `rel/backup/`).

## Camera fixes: Office 4K, door pop-ups, Backyard tile, 2026-11-09 (ScryptedBridge 0.2.7 deployed 17:13 EST; helper default only, no TV install)

Office full view came up h264 896x512, not 4K: Rebroadcast's default prebuffer session for the camera ran on the first stream it listed, because its start-up call for the stream options (the Reolink plugin, 5 s timeout) had failed. The bridge's address was right; the stream behind it was not. Re-saving the camera's Rebroadcast default stream (`prebuffer:defaultStream`, camera 128) restarted the session as main h265 3840x2160 at 25 fps, seen on the TV at 16:37 (`/data/home/tmp/lumatv-camfix-20261109/item1/office-full.png`). If a camera comes up small again, compare its streams' `prebuffer:detectedCodec` and `prebuffer:detectedResolution` in Rebroadcast's settings with the declared sizes and re-save the default stream. The bridge does not detect it.

Door pop-ups took 705 ms (Front) and 784 ms (Back Door) against 427 ms warm. Two parts vary: the bridge's answer and the feeder's start. The answer is 5 ms warm but took over 100 ms in 27% of 924 live starts between 2 and 9 Nov (up to 926 ms): once its 60 s copies had expired the bridge read the camera's stream description (Reolink, 150-185 ms) and its Rebroadcast settings (150 ms) one after the other. A first picture came 150-200 ms after the start on a kept feeder and 330-520 ms on a new one (median by idle gap: under 30 s 161 ms, 30-120 s 204, 120-600 s 332, 600-1800 s 416). Bridge 0.2.7 answers from the last read however old and reads again behind the call, reads both at start and when the ticked cameras change, and reads the two together for a camera never read. The helper's `cameras.feederIdleSeconds` default is now 1800 (was 120): a parked feeder holds no pipeline and 18.0 MB (VmRSS, MemAvailable 830 MB of 2.0 GB). On the set with `feederIdleSeconds` 1800 in `helper.json`: Back Door 150 s after the last stream, the same feeder pid kept, the bridge answered in 5 ms, first picture 154 ms, live video 431 ms (784 before). Front Door on a new feeder right after a helper restart: bridge 5 ms, first picture 573 ms, video 849 ms. The installed helper (30f067a6) still has 120 s until the next helper install.

Backyard's tile went grey after a return from an app. The bridge's stills ffmpeg ran with `-fflags nobuffer`, which discards what is read while probing, and the first of that is the key picture Rebroadcast replays at once: every start decoded P pictures with nothing to refer to and made 16 flat-grey pictures (3.2 s at 5 a second) on Stream 1 and 9 on Stream 2, in 3 of 3 runs each; without the flag 0 in 6 of 6 and 3 of 3, first picture 0.2-0.8 s. The flag is gone, and a camera whose streams carry no sizes (H.265: Rebroadcast cannot read them) feeds from its low-resolution stream (Backyard Stream 2, 768x432, not Stream 1, 1920x1080 with 634 KB key pictures). On the set after 20 s in Marquee and back: the tile had a picture 1.1 s after the Home launch (before: flat grey for the first 2.7 s and at 3 of 10 later samples up to 89 s), and one 0.85 s flat span at 3.5 s. What remains is a damaged key picture: Rebroadcast reads this camera over UDP, and a lost packet leaves grey until the next key picture. Switching Backyard's Rebroadcast parser to TCP (a Scrypted setting) is the untried fix.

Not tested: a gap over 30 min (the feeder ends and the next pop-up starts a new one), other apps' video while a feeder is parked (a session ends with its pipeline unloaded), the first pop-up of a battery camera, the 1800 s default as an installed build, Backyard over a longer run. Left as found: Active Standby, receiver off and unmuted, `helper.json` as found (a test event answers 403), lease released. Receipts: `/data/home/tmp/lumatv-camfix-20261109/final/` (`pop.json`, `feeder.txt`, `grey.json`, `backup/helper.json.as-found`), `item3b/`.

## Home focus dim removed, 2026-11-09 (main 7b442d73, installed 16:54 EST)

Home no longer dims the other cards and the dock when the focus enters the cards (measured on the set: cards 0 to 2 %, dock 12 %). The selected card keeps its lift and highlight; the dock's glass still lights while the focus is on it. The idle panel dim is untouched.

Standard cutover from standby: ui `0e7230c6` (replaced `55c1832c`); compositor `6cded4b2`, sm `65f6ca69` and helper `30f067a6` unchanged. The ui build also carries the other ui commits that reached main since `55c1832c` (camera pop-up OK, weather view, Sound route). Left as found: Active Standby, receiver off and unmuted.

Frame gate at 120 Hz, 3 cycles each, old build then new: Home widgets 3 dropped / 1 burst over 20 ms, then 1 / 0; most layers in a frame 107, then 84; Home d-pad 0 / 0 on both, layers 93, then 76, first frame 242 ms, then 195 (both fail the 50 ms bar). One run each, so the drop counts sit inside the earlier run-to-run spread (0 to 2). Screenshot with focus on the Smart home card, nothing faded elsewhere: `/data/home/tmp/lumatv-nofocusdim-20261109/home-focus-smart-home-card.png`. Receipts, `gate/`, `rel/installed-7b442d73.json` and the rollback copy of `55c1832c` in `rel/backup/` are in the same folder. Not tested: standby wake after the install.

## Apple TV opens at once: Apple MFi kept, 2026-11-09 (not installed)

The lean profile's `share` group stubbed `/usr/sbin/mfi` (`com.webos.service.mfi`). Apple TV's LG provisioning verifies its MFi key through that service at every start: with the stub it retried every 5 s, mapped its window 51.7 s after connecting and opened on a "ProvisionRebootRequired" alert, which the 7 and 9 Nov checks (20 to 35 s waits) took for no window. `items.txt` now keeps the service; it is on demand and runs only when called. With the stub lifted for one launch the window mapped 1.3 s after the app connected, on the profile picker, and the stub went back. Receipts: `/data/home/tmp/lumatv-appletv-20261109/` (not in git).

A running set keeps the old bind until it reboots: `off` walks only the rows `items.txt` still has. To accept after the install: reboot, then `/usr/sbin/mfi` has no mount in `/proc/self/mounts`, and Apple TV maps its window within about 2 s with no "ProvisionRebootRequired" alert.

## TV checks, 2026-11-09 afternoon (no install)

Check-only session, 14:44 to 15:45 EST, on the installed ui 55c1832c, compositor 6cded4b2, sm 65f6ca69 and helper 30f067a6. Left in Active Standby, receiver off and unmuted at -22.5 dB, test events off, Do Not Disturb as found, lease released. Receipts: `/data/home/tmp/lumatv-tvcheck-20261109/` (not in git).

- Wake from Active Standby, twice, accepted: Active in 3.91 and 3.61 s, Quick Settings full at 5.45 and 5.2 s, Phone remote focusable and opening, first key drawn in 8.3 and 6.1 ms; no `at most` or `scene is full` lines. `w1*.json`, `w2*.json`.
- Camera pop-ups, asked to live: Front Door 705 ms, Back Door 784 ms (morning 0.42 to 0.49 s), Plant Room 473 ms. Cameras view: Bird 163 ms, Backyard 297 ms, both filling the frame; a fast switch logged a feeder failure and recovered. Office full view came up h264 896x512 at 20 fps, not 4K (the helper asked for h265 3840x2160; 4K on 7 Nov). `camera/cycle.json`, `camera/office-full.png`.
- Apple TV: the process runs (sleeping, 4 threads, 15 MB) but makes no window and Home stays in front, as on 7 Nov. `appletv/result.json`.
- Visual, accepted: dock rollover, Settings toggles without replay, the Temperature row, category fly-in (54 to 55 frames over 445 ms, largest gap 10 ms), Settings with the profile chip, the Home widgets focus-dim (`visual/home-widgets-focus-dim*.png`). Sound's summary says "TV speakers" while the route is `external_arc` (`visual/sound-summary-mismatch.json`). Idle Home: LG's 180 s timer brings the LumaTV saver first (panel 75 to 66), so Home's own 5 minute dim never shows. The Backyard camera tile is grey for the first seconds after Home returns (`gate/home-after-marquee.png`).
- Do Not Disturb accepted: the Quick Settings tile and its card, the pop-up's held OK and Snooze sheet by keys, the doorbell switch read-only. An OK held within 600 ms of a pop-up's arrival reaches Home and opens Edit Home. `dnd/`.
- 120 Hz gate, 3 cycles, panel 119.9 Hz: settings-scroll and keyboard pass; apps-grid (1 dropped, 1 over 20 ms), home-widgets (2, 1), quick (1, 1) and switcher (1, 1) fail, and home-dpad fails on one first frame of 346 ms (the others 6.9 and 12.9 ms). Marquee in front gets 120 Hz from the automatic policy (YouTube 60, Home 120). `gate/tvc6-*.json`, `gate/refresh-auto.json`.
- Smart home accepted: OK on the Back Door Light turned it on ("On · 100%" within 1.0 s) and off again; the colour sheet (held OK on a floor lamp) and the thermostat sheet open and close with nothing changed. `smart/`.
- Weather (Home Assistant, Pirate Weather): card and view in °F and mph with local hour labels. With no UV reading the view's first page shows "UV INDEX" over an empty value (the Details page shows a dash); the Details page's sunrise tile is an empty arc and the Feels-like tile's "Actual 72°" runs past its edge. `weather/`.
- Saver probes T1 and T6 pass (backlight 66 to 75 in 51 ms after a key). `saver/`.
- Untested: sensor history (no sensor is shared to the TV), 200 % text, the saver's weather row, storm and snow moments (no hook on the set), saver probes A to F, T2 to T5 and T7, the picker gate, the 15-app soak with its video, recording and recovery checks, the audio-format glyphs, the offline camera retry.

## Phone remote connects after pairing, 2026-11-09 (main dc5474e4, installed 14:04 EST)

The first real iPhone (Marquee 0.1.25, LumaRemoteKit 1.0.2) paired, then showed the TV as "off or out of reach": the TV welcomed each session, and the kit failed to decode the `welcome` because the receiver's `volumeMax` is 72.5 and the kit's `VolumeState.level`/`max` are `Int`. dc5474e4 rounds both to whole numbers (the protocol now says so). Helper-only install while the TV was on: `lumatv-helper` ea8e659b replaced 92dbac9c; compositor 6cded4b2, ui 55c1832c, sm 65f6ca69 and `tools/lumatv` 0473b152 unchanged. Twenty seconds later the phone's session was live (`live:true`, ESTABLISHED from 192.168.1.151). Receipts and rollback copy: `/data/home/tmp/lumatv-remotefix-20261109/`. `lumaremote`'s own `log::info!` lines (paired, connected, left) do not reach the TV's log; the helper does not install a `log` backend.

31aa3773, installed 14:30 EST (helper 30f067a6 replaced ea8e659b, the rest unchanged): the phone's `volume` action takes the receiver's half steps (41.5), as the receiver and the TV's volume bar show them. The state still rounds `level`/`max` for kit 1.0.2. Sending the exact half steps waits for a kit that decodes them as numbers (asked of the Marquee session, 9 Nov 14:33).

## Scene ceilings raised, 2026-11-09 (main 890cc3e8, installed 13:11 EST)

A client may now hold 512 layers, 256 groups and 64 motion tracks (were 256, 96 and 32), so Settings over Quick Settings over Home no longer runs out of groups. A frame still walks only the layers and groups there are, and the compositor's maps stay reserved for 112 groups and 56 tracks.

Installed by the standard cutover with Home in front and the receiver as found (on, TV input, -30.0 dB, unmuted): ui `55c1832c` (replaced `fd666b15`), compositor `6cded4b2` (replaced `63a10300`); sm `65f6ca69` and helper `92dbac9c` unchanged. On the set Quick Settings draws whole, Phone remote takes focus and opens on OK, the pop-up layout editor draws whole, and no `at most`, `scene is full` or `more than N` line was logged from 13:10:30 to 13:43:52. The most a walk through Home, Quick Settings, Settings > Remote & pointer > Phone remote and the layout editor held: 176 layers, 68 groups and 13 tracks, with Quick Settings up; Home at rest 97, 44 and 10; no refusals.

Frame gate at 120 Hz, the old build then the new, same scenarios, dropped frames / bursts over 20 ms. No load: Home d-pad 0/0 twice on both; Home widgets 0/0 twice, then 0/0 and 1/1. Under plugin load: Home d-pad 0/0 on both; Home widgets 0/0 and 2/1, then 2/1; a second load run 0/0 on both; Apps grid in three runs 2/1, 2/0 and 0/0, then 1/1, 1/1 and 3/1 (first frame about 1450 ms on both, follow-up (b)). Input picker three runs with no drops on both; the new build's third failed on burst end (183 ms against 180). p90 render per run 1.64-2.87 ms, mean 2.07 (old 1.71-3.09, mean 2.41). The drops are within the old build's spread between identical runs (0-2) and render did not rise, so the build stays.

Anonymous memory after the gates: compositor 43.2 MiB (old 41.8), ui 33.5-34.5 MiB (old 36.6-36.7); the two together hold 1.0-1.6 MiB less by Pss. Before the gates the compositor held 36.7 MiB (old 36.6). Not tested on the set: a wake from standby. Receipts and the rollback copies of `fd666b15` and `63a10300`: `/data/home/tmp/lumatv-sceneroom-20261109/` (`session.log`, `gate/`, `walk-*.json`, `rel/installed-890cc3e8.json`, `rel/backup/`, `A-quick.png`, `A-phone-opened.png`, `A-layout.png`).

## Quick Settings and Settings with room after a wake, 2026-11-09 (main 5ef54774, installed 11:28 EST)

After the 09:40 wake Quick Settings opened empty and Settings > Remote & pointer could not focus Phone remote; both logged `at most 96 groups`. Home's 16-tile dock kept a lit group per tile at rest (a sixth of the client's 96), and with the smart-home card's lifts and the wake's picker too few groups were left for a panel. The dock now lights only the focused tile; under Settings, the cameras' viewer or any full kind the dock and the smart-home card let their groups go until the kind leaves, and a panel short of room is drawn whole once they do, never as an empty sheet.

Installed by the standard cutover with Home in front and the receiver as found (external ARC, volume 57, unmuted): ui `fd666b15` (replaced `862b4cbf`); compositor `63a10300`, sm `65f6ca69` and helper `92dbac9c` unchanged. On the set Quick Settings draws whole, Phone remote takes focus and opens on OK, Home comes back with both cameras live, and no `at most 96 groups` was logged from 11:27:45 to 11:32. Not tested on the set: a wake from standby, the cameras' viewer and other full kinds. Receipts and the rollback copy of `862b4cbf`: `/data/home/tmp/lumatv-scenecap-20261109/` (`session.log`, `installed-5ef54774.json`, `qs-after-install.png`, `settings-remote-phone-focused.png`, `settings-phone-remote-opened.png`, `backup/`).

## Camera streams: one set, no duplicates, 2026-11-09 (ScryptedBridge dbacd9e, deployed 09:13 EST, no TV install)

The Reolink doors' RTMP streams declare no codec, so a pop-up's codec list skipped their prebuffered `sub.bcs` and the bridge dialled the unbuffered RTSP preview (3.7-7 s). The bridge now fills a missing codec from Rebroadcast's `prebuffer:detectedCodec` and, between streams of one size, prefers the prebuffered one (ScryptedBridge 0.2.6, 744 tests). The morning's `RTSP h264Preview_01_sub` in `prebuffer:enabledStreams` on Front Door 104 and Back Door 103 is out again; the settings before are in `/data/home/tmp/lumatv-cleanstreams-20261109/scrypted-before.json`.

Pop-up (light) is the smallest prebuffered stream and full view (native) the largest the set decodes, with no transcode: doors `sub.bcs` 640x480 and `main.bcs` 2560x1920 / 1920x2560, Plant Room 1280x720 and 2688x1520, Bird 640x360 and 1080p, Tool Room 640x360 and 1440p, Office `sub.bcs` and 4K H.265, Gym Stream 2 and Stream 1, Cat Feeder 720p and 1080p, Backyard Stream 2 and Stream 1 (both H.265; Stream 1 is held open by NVR recording). Downstairs Door (Ring) has no live prebuffer and shows stills. The doors' `ext.bcs` stays prebuffered: it is the Medium quality of the full view and Back Door's HomeKit remote stream. Pop-up to live: Front 0.71, Back 0.42, Plant Room 0.44, Backyard 0.43 s, Bird, Tool Room, Gym and Cat Feeder 0.42-0.45, Office 0.54 s (morning: doors 0.48-0.99, Plant Room 0.46). Back Door full view: 2560x1920 H.264 live 0.16 s after OK; Backyard full view H.265 live 0.17 s. Not tested: Office 4K full view, Ring live.

## Phone remote helper installed, plugin off, 2026-11-09 (main da90a3e8, installed 05:12 EST)

Helper-only install of main da90a3e8: `lumatv-helper` 92dbac9c (replaced 7cf3523c; installed at `payload/helper` and `/var/lib/lumatv/bin`). Compositor 63a10300, ui 862b4cbf, sm 65f6ca69 and `tools/lumatv` 0473b152 are unchanged. Two stale tests were fixed on the way: the HA fixture lacked the phone-remote entry (036205986) and the `ipc` endpoint count lacked its routes (da90a3e8); `helper/build.sh test` and the release build pass at da90a3e8. The plugin runner's root test needs a world-traversable `TMPDIR`, not a 0700 scratch dir.

On the set: `/v1/phone-remote` answers `on:false`, the `plugins` topic lists `phone-remote` as `off`, nothing listens on 18733, and Settings > Remote & pointer > Phone remote opens (receipt `/data/home/tmp/lumatv-smooth5-20261109/phoneremote-installed.png`). Not tested: enabling the plugin and pairing. Left as found: Home in front, receiver unmuted and in standby, Active Standby, lease released. Rollback copy: `/data/home/tmp/lumatv-smooth5-20261109/rm/backup/`.

## Unchanged first frame of a motion kept, 2026-11-09 (main 6f9df990, installed 04:17 EST)

Installed with the TV Active and the receiver muted (owner-approved, overnight), by the standard compositor cutover, in three steps: ui `114db6bb` (8dedc9a6, replaced `ee06b4fe`) at 03:39; compositor `0970cd28` (99ee6a4c, replaced `d3389603`) and ui `862b4cbf` (99ee6a4c, which also carries the phone remote 26099d23; replaced `114db6bb`) at 03:57; compositor `63a10300` (6f9df990, replaced `0970cd28`) at 04:17. Helper `7cf3523c` and sm `65f6ca69` unchanged. Receipts: `/data/home/tmp/lumatv-smooth5-20261109/` (`session.log`, `gate/`, `pop-after.json`, `rel/installed-*.json`, `phoneremote-plugin-off.png`).

Camera pop-up on 8dedc9a6, one test event per camera, seconds to the picture and to live video in the pop-up (752ab5f9 before): Front Door 0.12 and 4.15 (0.86, 2.48), Back Door 0.18 and 3.65 (0.80, 5.31), Plant Room 0.09 and 0.44 (0.27, 0.67), Backyard 0.07 and 0.46 (0.07, 0.73). Front and Back Door sit on "Connecting" for 2.05 s before their video. The input picker passed three runs (first frames 31, 33, 30 ms). The Apps grid under load, three runs, still fails follow-up (b): first frame up to 1442, 1256 and 290 ms (752ab5f9: 524, 230, 112), 0 drops.

Follow-up (a). The ring now has a `U` line per UI commit taken in (99ee6a4c): its arrival, ops and motions, whether a frame was wanted, the time since the last flip, and the queued and flowing state. With it, the d-pad's motion-start drop is the UI's first commit after a key: it starts its motions at progress 0, so the frame drawn from it owes 0 px yet was flipped as a frame of the motion, and the first moving frame, which misses the next refresh, counted as a drop. 6f9df990 keeps such an unchanged frame on screen without a flip when nothing moved before it (`kms/frames.rs`, `keeps_unchanged`). The picture and the time of the first moving frame are the same as before; the gate no longer counts an identical picture's flip as a motion frame.

Frame gate at 120 Hz, nine cycles per surface, dropped frames. No load: Home d-pad 2 on 99ee6a4c, then 1, 0 and 0 in three runs on 6f9df990; Home widgets 0 and 0. The one d-pad drop came on the first key after the install, whose commit carried 654 ops and took a 12 ms tick to plan; that cycle's first frame took 813 ms, and the third run's first cycle 65 ms (limit 50), so two of the three d-pad runs fail on first frame. Under plugin load (the tuner streaming about 10.5 Mbit/s, camera test pop-ups, a guide refresh): Home d-pad 1 (752ab5f9: 3), Home widgets 2 with one burst over 20 ms, then a pass (752ab5f9: 1). The widgets drops fall where a camera pop-up claims or releases the video window, with frames taking 13 ms to GPU done.

Settings > Remote & pointer > Phone remote opens with the plugin off and reads "Not available": the installed helper predates the phone remote. Nothing was enabled or paired.

Left in Active Standby, receiver in standby and unmuted, test events off, lease released.

## Home motion without frames drawn over waiting ones, 2026-11-09 (main 752ab5f9, installed 02:06 EST)

Installed with the TV Active and the receiver muted (owner-approved, overnight), by the standard compositor cutover and the helper path. Installed sha256 prefixes: compositor `d3389603` (replaced `301583e6`), ui `ee06b4fe` (replaced `e516e5dd`), helper `7cf3523c` (replaced `5848b62d`); sm `65f6ca69` and `tools/lumatv` `0473b152` unchanged. The ui and helper also carry `6938bc32` (the camera pop-up opens on a kept picture). Receipts: `/data/home/tmp/lumatv-smooth4-20261109/` (`session.log`, `gate/`, `rings/`, `rel/installed-752ab5f9.json`).

Frame gate at 120 Hz, nine cycles per surface, dropped frames / bursts over 20 ms, b6f72b77 then 752ab5f9 in two runs. No load: Home d-pad 87/39 then 2/1 and 2/1, Home widgets 7/5 then 2/1 and a pass (0/0), Apps grid 3/1 then 1/1 and 1/1. Under plugin load, one run (the tuner streaming 10.7 Mbit/s, camera test pop-ups, a guide refresh): Home d-pad 63/33 then 3/1, Home widgets 10/8 then 1/0, Apps grid first frame up to 1444 ms then up to 524 ms. Keyboard passed on both builds; the input picker passed on b6f72b77 and failed its one run on 752ab5f9 (burst end 189 ms against 180), not rerun. Nothing visual changed. The one or two drops left on Home widgets and the Apps grid with no load fall mid-motion (+0.14 s, +0.51 s) or on a burst's last frame and are not analysed.

Follow-up (a), the first frame after a still screen, in the pacer (`compositor/crates/lumatv-compositor/src/kms/mod.rs`). The d-pad's drops in A1 and AL1 include the first interval of the motion. In `rings/A1-home-dpad-m0-r2.ring.json` (compositor clock): after 442 ms of still screen, frame 2816 (25600 px) flips at 78398.094; the UI's next commit owes 0 px (frame 2817, drawn on damage at .105) and flips at .111; the first moving frame (2818: 820992 px, 109 draws) starts at .112, right after that flip lands, reaches GPU done 8.6 ms after its tick against an 8.3 ms refresh, misses .119 and flips at .128, 16.7 ms after the flip before. To make .119 it had to start by about .110, before the 0 px frame's flip landed. Until the pacer counts the frames as flowing, nothing starts it earlier: `next_frame_at` returns `None` while `flowing()` is false (not in a run, no pacer phase, or nothing presented for 1.5 refreshes), `draws_on_damage` is false while `pending_flip`, and the ticker falls back to the idle grid tick, whose budget (`min(typical, IDLE_TYPICAL_CAP_NS) + IDLE_LEAD_NS`) comes from the cheap frames before the motion. The ring does not record when the moving frame's commit arrived, so the first step is a ring line per UI commit (arrival time, draws, owed px) and one d-pad run. If the commit was waiting by .110, a UI commit with damage that arrives while not `flowing()` is drawn at once and queued behind the flip in flight (the existing `queued` path, which `after_waiting_frame` already protects), and only then, so the loop never spins with nothing owed. If it arrived only after the 0 px frame's flip, the UI's first commit after a key has to carry the first moved frame. Accepted when Home d-pad and Home widgets drop 0 with no load in two runs and nothing gets worse under load or on the other surfaces.

Follow-up (b), the Apps grid's first frame under load: in AL1 nothing flips from the digit key until a camera pop-up is released (+518 ms, +104 ms) or arrives (+215 ms); the cycle with no pop-up change drew its first frame at +10 ms (`rings/AL1-apps-grid-*.ring.json`). Taken with the camera pop-up work.

Left in Active Standby, receiver in standby, test events off, lease released.

## Network tuner with another app in front, 2026-11-08 (main b6f72b77, installed 23:33 EST)

Helper `9287d135` (replaced `d40ed35e`) installed at 23:33 EST with the TV Active and Marquee playing in front, by the helper path alone: compositor `06313632`, ui `c3f161dd`, sm `65f6ca69` and `tools/lumatv` `0473b152` unchanged, Marquee in front and playing throughout. Before it, with Netflix in front, a tune of 2.1 returned the "did not respond" slate (`repro.ts`, `repro-slate.png`, `repro-helper.log`).

Accepted on the TV at 00:08 EST over Marquee playing: a 20 s tune of 2.1 from BeastNAS carried MPEG-2 video at 1280×720 with AC-3 5.1 and stereo (mean -33.6 dB), its picture 1.6 s after the request (`verify/host-2.1-ffprobe.txt`, `verify/host-2.1-frame.png`); `/status.json` named the client while it ran and no one after; the tuner's pipeline held only `ADTU` and `SDEC`, Marquee's kept its decoders, main scaler and main sound (`session.log`, `verify/host-*.jpg`). The iPhone's Marquee tuned 5.6, 11.2 and 11.11 over the same app with pictures in 1.4 to 3.1 s (`verify/helper-tuner-lines-2335.txt`), and the TV's last channel stayed `1_32_2_1`. DockDown's bundle from the same commit then put its own build of the helper, `5848b62d`, in place; with it, in Active Standby at 00:42 EST, 2.1 again carried MPEG-2 1280×720 and AC-3, its picture 1.8 s after the request (`verify/5848b62d/`). Untested: Live TV taking the tuner mid-stream, Marquee's sound by ear. Receipts: `/data/home/tmp/tuner-awake/`.

Accepted with Netflix in front on helper `5848b62d` (2026-11-09 00:50 EST): Jurassic World: Dominion playing 3840×2160 23.976 Dolby Vision on the main decoder, a 20 s tune of 2.1 from the host answered in 0.03–0.06 s with MPEG-2 1280×720 and AC-3 5.1 and stereo and a decoded picture; Netflix stayed in front and kept playing; `/status.json` named the client during and no one after. Receipts: `/data/home/tmp/tuner-awake/netflix-4k/`. Left in Active Standby, receiver off.

## Dock Down and Quick Settings without scrolling, 2026-11-09 (main b6f72b77 with e791d5fb, installed 00:38 EST)

Installed with the TV Active and Marquee in front (owner-approved), by the standard compositor cutover and the helper path; Marquee was in front when the lease was released. Installed sha256 prefixes: compositor `301583e6` (replaced `06313632`), ui `e516e5dd` (replaced `c3f161dd`), sm `65f6ca69` and `tools/lumatv` `0473b152` unchanged, helper `5848b62d` (replaced `9287d135`; the same source as b6f72b77, a different build). Bundle `/data/home/tmp/lumatv-ready-b6f72b77`, receipt `installed-b6f72b77.json` in the bundle.

Accepted on the TV: Home dock focus on top-row tile 8, Down lands on tile 15, the last of the bottom row (`visual/dock-down-before.png`, `visual/dock-down-fix.png`); Quick Settings over Marquee shows the signal chip and clock on one row, 1.5-times avatars and every tile with no scrolling (`visual/qs-profiles-noscroll.png`), all under `/data/home/tmp/lumatv-smooth3-20261107/`. Untested on the TV: the receiver tile (the receiver was off), 200 % text (the list scrolls by whole rows there; the tile set needs about 2000 px).

## Quick Settings flush right and signal chips install, 2026-11-08 (main bcda7804, installed 22:29 EST)

Installed with the TV Active and Netflix in front (owner-approved interruption), by the standard compositor cutover and helper path; Netflix relaunched after, lease released. Installed sha256 prefixes: compositor `06313632` (replaced `108414af`), ui `c3f161dd` (replaced `8ba48ab2`), sm `65f6ca69` (unchanged), helper `d40ed35e` (replaced `a0d0c3d1`), `tools/lumatv` `0473b152` (unchanged). Gates at bcda7804: uitest, comptest, helpertest, release. Receipts: `/data/home/tmp/lumatv-ready-bcda7804/`.

Accepted on the TV (`visual/qs-over-netflix-bcda7804.png` in `/data/home/tmp/lumatv-smooth3-20261107/`): over Netflix's profile picker the sheet stands 40 px from the top, right and bottom for the screen's height, and the signal line is one sync chip (`96 Hz`) under the title. Untested on the TV: the size, frame-rate and range chips during playback, an HDMI input's Game Optimizer readings and VRR chip, the Home dropdown after this change.

## Do Not Disturb and camera snoozes install, 2026-11-08 (main 9f6414cc, installed 21:06 EST)

Installed with the TV Active and Netflix in front (owner-approved interruption), by the standard compositor cutover and helper path; Netflix relaunched after, receiver left on at -23.5 dB, DND off, no snoozes, test events off, lease released. Installed sha256 prefixes: compositor `108414af` (replaced `5150771a`), ui `8ba48ab2` (replaced `abb8d680`), sm `65f6ca69` (unchanged), helper `a0d0c3d1` (replaced `3f14761a`). Gates at 9f6414cc: uitest, helpertest, comptest (816 passed), release.

Accepted on the TV (`dnd-check.json`, `dnd-check-run2.json`, `visual/dnd-*.png` in `/data/home/tmp/lumatv-smooth3-20261107/`): a Front Door test event with DND on is held (`held: dnd`) and recorded as missed; with DND off it shows (over the screensaver as its event line); a camera snooze holds the next event from that camera; the status bar shows the circle-minus mark with `1h`, and the bell-off mark for a snooze only. An event held by DND still arms that camera's per-kind cooldown, so a repeat of the same kind within the cooldown after DND ends shows nothing. Untested on the TV: the Quick Settings tile and duration card by remote, the pop-up's Snooze key and sheet by remote, held LG-feed toasts, the doorbell switch, receiver Tests B and C.

## Receiver kept on after LG saves its speakers, 2026-11-08 (main 81ab0161, installed 20:05 EST)

Helper `3f14761a` (replaced `3d57f9ba`) installed at 20:05 EST with the TV Active and Netflix in front; helper restart only, no compositor cutover, Netflix stayed in front, receiver left on at -19.0 dB. Compositor `5150771a`, ui `abb8d680`, sm `65f6ca69` unchanged. The keeper logged at start: sound output `external_arc`, receiver kept on with the screen. Tests A/B/C not yet run. Receipts: `/data/home/tmp/lumatv-ready-20261108/`.

LG saves `soundOutput` `tv_speaker` about a second after the receiver leaves ARC while the screen is on (the TV's syslog, 2026-11-07 22:07:28 and 22:15:09). The plugin took that saved value for the owner's choice, so no wake, start or keeper sent a power command and nothing was logged. LG's own CEC wakes the receiver only while its saved output is ARC (`Last ARC(1)` at 12:28 woke it, `Last ARC(0)` at 09:51 did not). 81ab0161 remembers the receiver output in `receiver-output.json` through LG's own switch, treats a start with the screen on as a wake, and keeps retrying every `receiver.powerOnMs` (5 s), doubling to `maxBackoffMs` (30 s), while the screen is on.

The receiver's settings, read over HTTP and left unchanged: Network Control Always On, HDMI Control On, ARC On, TV Audio Switching On, Power Off Control All, Auto Standby, ECO and Power Saving off.

Not installed: the TV was in use (Active) from 13:38 to 15:10. The prebuilt helper `3f14761a` (replaces `3d57f9ba`; compositor, ui, sm and tools/lumatv stay f4c0ffa8's), its install and the wake, standby-while-on and cold-start checks are in `/data/home/tmp/lumatv-ready-20261108/`. Untested on the TV: all of it, and a real power cut.

## Watch-bug fixes install, 2026-11-07 (main f4c0ffa8)

Main f4c0ffa8 installed at 22:05 EST from the prebuilt bundle in Active Standby by the standard compositor cutover and helper path; left in Active Standby. Installed sha256 prefixes: compositor `5150771a` (replaced `6b18d2dd`), ui `abb8d680` (replaced `e9cc2bb2`), sm `65f6ca69` (unchanged), helper `3d57f9ba` (replaced `a977eee7`). Brings the Settings-over-quick-panel fix (96797d97), the receiver keeper (3ad4ab98) and pop-ups without a hardware video window over a foreground app (f4c0ffa8). Untested on the TV: all three. Receipts: `/data/home/tmp/lumatv-smooth3-20261107/session.log`, bundle and rollback binaries `/data/home/tmp/lumatv-ready-20261107/`.

## Tuner busy slate install, 2026-11-07 (main f5f3a2b4)

Main f5f3a2b4 installed at 15:43 EST by the standard compositor cutover and helper path, with the owner watching: a LumaTV toast before each step, the receiver and input left alone, Home in front before and after. Installed sha256: compositor `6b18d2ddbf25f8267438525f458aeb250b2afd81c513a0354e9107444ed4a23b` (unchanged), ui `e9cc2bb27541ff5b0907062d967bd78c0fc287a0f071cd2961a34dafd74e347e` (replaced `8eb3c7f1`), sm `65f6ca69` (unchanged), helper `a977eee76c42013602bb45e766e7e951bd2f55c7e04842ec8e721a4e0775efe2` (replaced `f08140a1`). Left on, at Home, receiver off and unmuted at -37.0 dB, test events off. Receipts: `/data/home/tmp/lumatv-smooth3-20261107/`.

- Dock titles accepted: the top row's above its icon, the bottom row's below. `visual/dock-*.png`.
- Camera pop-ups accepted, one test event each with test events switched off again after: Front Door portrait at 330x440, Back Door landscape at 520x390, both on the main video window. `visual/popup-*.png`, `session.log` (camaspect lines).
- Tuner busy slate accepted: with BeastNAS holding 2.1, a request from another address got 200 and 6.02 s of MPEG-2 1280x720 with two AC-3 tracks reading "Tuner busy / Plex is recording College Football: Alabama Crimson Tide at LSU Tigers on 2.1 WSB-HD", then the end. `/status.json` named the holder (`192.168.1.69`, `Plex`, 2.1 WSB-HD) while held and `{"holder": null}` before and after. A second request from the holder's own address takes the tuner over instead, as before. `tuner/`.

## Batch2 install, 2026-11-07 (main ca5da42c)

Main ca5da42c's helper installed at 10:30 EST by the standard helper path; the set was taken from Active Standby, put back into Netflix and Active Standby at 10:47, the receiver left off, unmuted, at -37.0 dB. Installed sha256: helper `1971c3e01ba8dc378abaa959d09e20932c4ef24a335cf37ff0770c9f469698a5` (replaced `5232eb4f`); compositor `e4a5f6dc`, ui `9e491f5d`, sm `65f6ca69` unchanged. Receipts: `/data/home/tmp/lumatv-batch2-20261107/`.

- Tuner slate, ffmpeg from the server: accepted. With the next two slate pictures sent behind the first, the first picture comes 0.010 to 0.022 s after the first byte on 2.1 and 5.1, three runs each (0.46 to 0.58 s before). `tuner/`.
- Visual: dock focus rollover, the Home & look Temperature row and Settings toggles (no replay after scrolling) accepted; idle Home went to the dimmed clock saver after the 5 minute timeout, panel 75 to 67. Settings category fly-in untested: a 1.3 s screenshot cannot catch the motion. `visual/`.
- 120 Hz frame gate: fails 6 of 7. Quick settings passes; Home d-pad first frame 773 ms, Settings scroll and the Apps grid 2 frames over 20 ms, keyboard 112 over, picker burst end 194 ms (limit 180), Home widgets 15 over. `gate/b2-*.json`.
- Weather card (HA source, units, hour labels), the full camera viewer (Backyard, Bird and Office live in 0.08 to 0.5 s, all filling the frame, no failure lines), and the thermostat and colour sheets accepted. Sensor history untested: no sensor tile on the card. `visual/`, `camera/`.
- Soak: 14 of 15 dock apps came to the front in 0.5 to 3.4 s and closed; Apple TV ran but made no window. Compositor failures 0, helper pid unchanged, no helper failure lines. `soak/result.json`.
- Self-heal: ui.refusals 0, no `compositor refused:` or `scene is full` lines. Home widgets focus-dim screenshot: `visual/home-widgets-focus-dim-inside.png`.


## Morning install, 2026-11-07 (main 1dd11322)

Main 1dd11322's helper installed at 09:40 EST by the standard helper path, the set woken from Active Standby into Netflix and put back there at 09:56, the receiver muted while on and left off, unmuted, at -37.0 dB. Installed sha256: helper `5232eb4fbd8340039f10d14cf56e3c00028b7c834102a44850cc399fbb1ca0bb` (replaced `568e66d6`); compositor `e4a5f6dc`, ui `9e491f5d`, sm `65f6ca69` and lumatv-jpeg unchanged. Receipts: `/data/home/tmp/lumatv-morning-20261107/`.

- Tuner slate, ffmpeg from the server on 2.1 and 5.1: not faster. The video PES now carries its length (30034 and 31038 bytes) and is whole 3 ms after the first byte, but the first picture still comes 0.55 to 0.81 s after it (0.59 to 0.66 s before). FFmpeg's MPEG video parser ends a picture only at the next picture's start code, and the helper sends the slate's second picture 0.54 to 0.80 s after the first. `tuner/`.
- Tuner refusals: six asks for 2.1 from a second peer while the first held 5.1 logged "all tuners in use" once and then "asked for 2.1 5 more times in 0.8 s"; 5.1 kept streaming. `tuner/refusal.json`.
- Camera resume, Backyard, 3 rounds of 20 s: the picture stood 2.30, 0.71 and 0.70 s at the hand-off to live. The end is seen 250 ms after the recording's last picture; live's first picture follows about 0.45 s later. In round 1 the end reached the full view 2.0 s after it was seen [INFERENCE: `read_stream` waits for the bridge's answer to `/v1/playback/stop` before it publishes "stopped"]. Backyard's live stream also froze 3.8 to 4.0 s at a time, one picture per key picture, while its RTP lost packets: 257 to 478 lost and 12 to 17 pictures a second per view, against 0 lost and 24.9 a second overnight; the live path is unchanged since then. `camera/`.
- Self-heal: `ui.refusals` 0, no "compositor refused:" or "scene is full" lines, compositor failures 0.
- Untested: the visual checks (dock focus rollover, Settings toggles, Home & look Temperature row, category fly-in, idle Home dim, audio glyphs beside the volume bar), the 120 Hz frame gate, the weather card, the cameras cycle, the Smart home sheets, the 15-app soak and the focus-dim screenshot. The weather card's storm and snow moments have no ctl hook.


## Overnight install, 2026-11-07 (main 6fd2a63d)

Main 6fd2a63d installed at 00:50 EST by the standard path, the set woken from Active Standby with Netflix in front. It carries the format badge waiting for the real size and a recording ending where its window does (6fd2a63d). Installed sha256: compositor `e4a5f6dcc192c0324cd3de74a91b2767f4df5358460755f25fad6e62706d4365`, ui `9e491f5dd6f40be1511b51f3f83e4a9167caa233f0cfea33e7983c403c2d5e31`, helper `568e66d64fea027c13863c05a0a9ef8afa3e8c64b751b578acb6d6c6294eb4d9`; sm `65f6ca69`, lumatv-jpeg `8b6d65a8` and `tools/lumatv` `0473b152` unchanged. Replaced: compositor `66c65cc6`, ui `35eb2e98`, helper `e26b0377`. Receipts: `/data/home/tmp/lumatv-overnight-20261107/`.

- Live TV, 69-6 (704x480 interlaced): the badge shows SDR alone, for 3.0 s.
- Cameras, Backyard (88), two rounds of OK hold, about 3 s, then Play, watched 20 s: both played the recording and went on into live about 7 s after Play, 0 failures. Round 2 ran the recording to 7.1 s and was live 0.5 s later; in round 1 the recording ran dry 4.6 s after Play and the picture stood about 2.3 s (between 1.9 and 2.8 s) before live.
- Settings: the focus plate on the rail and the profile chip beside the title.
- Self-heal: `ui.refusals` 0; no `compositor refused:` or `scene is full` lines.
- Left as found: Active Standby with Netflix in front, the receiver off at -37.0 dB, unmuted, compositor failures 0.

Open: the early hand-off to live does not fire when the bridge runs out of recording before the window's end. Live starts about 7 s after Play in both rounds whatever the recording did, so a recording that runs dry earlier leaves the picture standing until then.

The receiver ignored `MUON` on the HTTP command page (200, still unmuted); telnet on port 23 muted it.

## Follow-up install, 2026-11-06 (main c80e8555)

Main c80e8555 installed at 23:35 EST by the standard path, the set woken from Active Standby with Netflix in front, which was relaunched afterwards. It carries the format badge's glyph marks (595cd907), OK holding live on its last whole picture (c155115d), Home widgets saying a width of its own (7c0acb07) and the map tests (c80e8555). Installed sha256: compositor `66c65cc64ddc476f823bd8cc310c5b652b486644001e52c95bded859df3a7e03`, ui `35eb2e98fc2ba111094baa36222e5b2724de7670a0d242956e7954acb62cc1c8`, helper `e26b037794a2d4a73f0e9e8703238a70a1b2171712838fb7aa40bc14ee432b9d`; sm `65f6ca69`, lumatv-jpeg `8b6d65a8` and `tools/lumatv` `0473b152` unchanged. Replaced: compositor `d60bff98`, ui `440a800c`, helper `f042de19`. Receipts: `/data/home/tmp/lumatv-followup-20261106/` (`session.log`, `installed-hashes.json`, `as-found.json`, `as-left.json`, `format/`, `toast/`, `camera/`, `widgets/`, `selfheal.json`, `gate/`; rollback copies in `backup/`).

- Format badge over Home, `show toast format:` with `uhd,dolby-vision,atmos`, `fhd,sdr,pcm`, `hd,hdr10,dts,5.1` and `uhd,120hz,hdr10plus,dolby-truehd,7.1`: one pill 44 px high, 40 px in from the top and the right (278, 210, 274 and 418 px wide), glyph marks and the Dolby logos, up for 2.97 to 3.02 s. Live TV (69-6) showed FHD and SDR in the same place for 3.1 s.
- Toast `Recording saved|Front door`: a 320 by 100 pill at the bottom centre with a Home card's glass (fill 55,50,70 against the clock card's 52,47,69) and text as sharp as the card's (99th-percentile edge step 0.967 against 0.958).
- Cameras, Backyard (88), five rounds: OK held within 0.09 s on a clean picture each time (no green, unchanged 3 s later); OK again played on as the recording (H.265) after 0.47 to 1.25 s with 0 pictures lost.
- Settings › Home & look › Home widgets › Show: the Show and Order labels draw; Clock and Smart home read "7 of 12 columns" and Weather "5 of 12 columns", with no size lit.
- Self-heal: `ui.refusals` 0 throughout; no `compositor refused:` or `scene is full` lines in the RAM log, `compositor.log` or `shell-rust.log`.
- `gate.sh rust` format: PASS (first 90 ms, drawn +43, GL errors 0). 120 Hz frames gate, format, 3 cycles: PASS (panel 119.9 Hz, first frame 11.8 ms, burst end 237 ms, p90 render 1.67 ms, none dropped).
- Untested: an HDMI input's sound line (no HDMI source is connected; HDMI 2 is the receiver).
- Left as found: Active Standby with Netflix in front, every plugin on, the receiver off at -37.0 dB, unmuted, input TV, the theme and profile files byte-identical, compositor failures 0.

Findings:

- Live TV's 69-6 is 704x480 interlaced, yet the badge said FHD. videooutputd's first status for the new video reports 1920x1080 with frame rate 0 and no video info, and `size.or(self.size)` in `badge.rs` keeps that FHD when the real size arrives, which below 720 lines has no mark.
- Played on from a hold, the recording stops near the moment Play dialled it. Round 5 (held 6.1 s): the position stopped 7.7 s in, at 23:45:02.26 on the bridge's clock, against the dial at 23:45:02.51 on the set's; the picture stood for about 9 s, the helper ended the session ("the stream went quiet", 18.1 s after the dial) and the view went live 19.5 s after Play. Round 4 ended the same way, 16.3 s after the dial; rounds 1 to 3 were closed within 10 s of it. [INFERENCE] The bridge's recording ends at the time it is asked for rather than running on into live.

## Combined install, 2026-11-06 (main 43e68615)

Main 43e68615 installed at 22:15 EST by the standard path over Live TV, which was relaunched in front afterwards. It carries the volume fix (90229a84), the highlight self-heal (191cbbe0), the switched-off controller (f606415f, below), the camera OK pause (a98bf5df) and the format badge (3dfcfb8a, 43e68615). Installed sha256: compositor `d60bff985665f1b03b3bcb1a193868ddc9db6e5a42f19d29eafaaa49ab0f16ca`, ui `440a800c9c2647e2f87a52365291a4d9d6ce04b692d246e295d66e09155cfa26`, helper `f042de1921cdf35a142ab61b92bc098f1e7258c206b39c29fff84ba1c9e0a66b`; sm `65f6ca69` and lumatv-jpeg `8b6d65a8` unchanged. Replaced: compositor `87ef3b24`, ui `f6c19d64`, helper `e74688a8`. The app's leftover `payload/padguard/` was removed; `50-padguard` is absent. Receipts: `/data/home/tmp/lumatv-combined-20261106/` (`session.log`, `installed-hashes.json`, `as-found.json`, `as-left.json`, `selfheal/`, `vc-fix/`, `vc-lat/`, `pad/`, `camera/`, `format/`, `gate/`; rollback copies in `backup/`).

- Self-heal: Home, then Settings › Home & look › Home widgets (Show and Order) and back with remote keys: `ui.refusals` 0 throughout and no `compositor refused:` or `scene is full` lines; the profile chip, focus plates and row plates drew on every page.
- Volume, receiver plugin on: three taps down, two up, a 2 s hold and mute twice moved the receiver with no overshoot; the bar matched all six, CEC sent no volume steps (two mute), and the bar showed mute and its release. Key to bar 9.2 ms median, 12.6 ms max (n=10), against 12.6 ms (n=20, f802ddd2) and 10.0 ms (n=12) before the fix.
- Controller, pad off: the compositor logged `event9` "sticks at the ends of their ranges, no report yet: centred", and `EVIOCGABS` reads 128 on ABS_X, ABS_Y, ABS_RX and ABS_RY (0 before the install). With Netflix in front for 60 s the node gave no reports and nothing scrolled.
- Cameras: OK holds the full view with the pause badge centred and "Paused" in the pill; OK again plays on (Backyard, which records: from the hold, "Recorded"; Bird, which does not: back to live). Down opens all cameras on the camera shown and Back closes; when the controls have hidden, the first Down only brings them back. No `HW jpg fail` in the log or dmesg.
- Backyard's first hold showed a broken picture, its lower half green, then blocks on playing on; the helper logged that recording as H.264 with 50 pictures lost. A second hold got H.265, lost none and drew clean.
- Format badge: `show toast format:uhd,dolby-vision,atmos` over Home drew "4K · Dolby Vision · Dolby Atmos" at the top right and left within 6 s; Netflix's Meridian page (its preview in Dolby Vision, 3840×2160) showed "4K · Dolby Vision" 10 s after launch; Live TV showed "FHD · SDR". `gate.sh rust` format: PASS (first 110 ms, drawn +61, GL errors 0). 120 Hz frames gate, format, 3 cycles: PASS (first frame 11.2 ms, burst end 236 ms, p90 render 1.87 ms, none dropped).
- After the Dolby Vision page `DE2VSYNC` ran at 240 a second while `osd_irq` stayed at 120, so gate.py's panel check refused to start; a few seconds of Live TV brought `DE2VSYNC` back to 120.
- Untested: the pad switched on (Netflix navigation with it, GeForce NOW taking it); a Dolby Vision film played, beyond its preview.
- Left as found: Live TV in front, every plugin on, the receiver on at -37.0 dB, unmuted, input TV, the theme and profile files byte-identical, compositor failures 0.

## Switched-off controller centred, 2026-11-06 (main f606415f, installed with 43e68615)

Compositor change only (`input.rs`): a game controller whose four stick axes all sit at an end of their ranges when the compositor opens it gets a centred report written into its node. Read on the set at 21:13 to 21:23, before the change: the receiver (054c:09cc, `usb 8-1`, device 035) with its pad off since 18:56, `event9` at LX=LY=RX=RY=0 of 0..255, no reports on `event9` or `hidraw0`; `event9` held by `surface-manager` and the compositor only; `/var/lib/webosbrew` made anew 2026-10-30 21:00, so `50-padguard` and `padguard.py` are gone and no `/tmp/padguard.log` exists. TV check: the combined install above.

## Plugin isolation, the Receiver link 2026-11-06 (main 5acdf16a)

Main 5acdf16a installed at 18:30 EST by the standard path; the set was woken from Active Standby for the session and returned to it. Installed sha256: compositor `87ef3b2466b7b25ba2df64610bd288bf7224379dc20a3008ed9e9af338be27f8`, ui `f6c19d64347d8feceaf2466770a9904ad84b73eccee15f6a1fd69e45d3127e3a`, helper `e74688a88d94382784a32669a954d159be177d25b792d2b271f8ef42a9ab356e`; sm `65f6ca69` unchanged. Replaced: compositor `1ef99f70`, ui `366a165c`, helper `4e660ad6`. Receipts: `/data/home/tmp/lumatv-plugin-session-20261106/` (`session.log`, `installed-hashes.json`, `checks.json`, `cycle-*.json`, `idle-*.txt`, `latency*.json`, screenshots).

- Each plugin's thread runs under `plugins::guard`: a panic stops that plugin alone, its switch stays on with "Stopped by an internal error: …" under it, and it retries after 30 s, doubling to 30 min. Not covered: threads a plugin spawns itself, a stack overflow or a panic while unwinding (both still end the helper), and the 32-thread HTTP pool.
- Every plugin switch (denon-receiver, scrypted-cameras, home-assistant, live-tv-tuner, guide-data, ble-proxy, tv-sensors) was switched off, reached again and switched on with remote keys from Home: gear, All settings, rail, tab, link, switch, then the question's second answer. While the receiver plugin is off, the Sound page shows the Receiver link as "Off" and "Receiver plugin is off" under Receiver volume.
- Off: the camera tiles and status icon leave Home and `/v1/cameras/event` answers 409 `off` with no pop-up; Home Assistant's card leaves Home and the weather falls back to the ZIP; the tuner's discovery port refuses; the guide settings answer 404; with only the BLE proxy off the TV sensors stay in HA (one 13 ms reconnect at the switch); with TV sensors off their entities have no state in HA. On again, each comes back, with the same HA entity ids.
- With the receiver plugin off, the volume keys step the receiver over ARC (-42.0 to -41.5 dB and back) and show the bar. With it on, key to bar is 20.8 ms median (n=2, poll bounds 11–21 ms) against 12.6 ms (n=20, f802ddd2). One tap read +0.5, +1.0, then +0.5 dB at the receiver within 0.4 s, ending on the bar's level.
- Idle Home for 60 s (liquid glass, the IdleFollowups method), before → after: compositor 5.82 → 4.96 % and 182 → 207 MB, ui 3.89 → 3.53 % and 56 → 52 MB, helper 1.21 → 1.05 % and 45 → 34 MB, flips 20.1 → 18.9 a second.
- No LumaTV crash records or LG crash reports since the first plugin install (13:35); compositor failures 0; helper and compositor not restarted since the install.
- Left as found: Active Standby with Home in front, every plugin on, theme files byte-identical, the receiver off at -42.0 dB, unmuted, input TV.

## Denon receiver plugin 2026-11-06 (main 6e902567)

Main 6e902567 installed at 17:46 EST, the set in Active Standby (`tv-first-light.sh deploy`, cutover disable/install/probe/enable, helper staged in `payload/helper/`, `lumatv helper start`). Installed sha256: helper `4e660ad60eab330477ec0930eeea234d9cf56f4b4618c4491d55664e050e92da`, ui `366a165c5ade44a7ed1dcc0e8f0639c80fb4e9ba27205420903d63a5642d5558`; compositor `1ef99f70` and sm `65f6ca69` unchanged. Replaced: helper `14994b99`, ui `6ec9b73b`. Receipts: `/data/home/tmp/lumatv-plugin-denon-20261106/` (`session.log`, `installed-hashes.json`, `migration.txt`, `checks.json`).

- Migration: `denon-receiver` is approved in `plugins/enabled.json`, its settings hold 192.168.1.93:8080, `base.receiver.json` is gone; the plugin reads `connected` and the receiver topic `enabled`.
- Switched off: plugin `off`, receiver topic `enabled:false`, the settings page answers 404, no socket to the receiver. Switched on again: `connected`, same address.
- The receiver was off (-42.0 dB, unmuted, input TV) throughout and was left that way, the plugin on.
- Untested on the TV: receiver volume steps and key-to-receiver latency (receiver off; not powered on), and the volume keys changing the TV's own volume while off (panel off; host test only).

## Saver sources follow their plugins 2026-11-06 (main ce52bdf4)

The helper from ce52bdf4 installed at 16:53 EST, the set in Active Standby (staged in `payload/helper/`, `lumatv helper start`); compositor, ui and sm unchanged. Installed sha256: helper `14994b9912877ce9bd635b58265a746b4860952e5f4b755746c4fa70700916bc`. Replaced: helper `203e514b`. Receipts: `/data/home/tmp/lumatv-plugin-saver-20261106/` (`session.log`, `installed-hashes.json`, `*-state.json`, `*-crop.png`).

- Saver with every plugin on (`b-saver-all-on`): the date and Home Assistant's alert "Login attempt failed" as a notice; the weather from `weather.pirate_weather`.
- Home Assistant switched off while the saver was up (`c-saver-ha-off`): the notice is gone within 10 s, the weather's place is the built-in ZIP, no Home Assistant cards or notes are read.
- Switched on again under the saver (`d-saver-ha-on`): the notice and the entity's weather are back.
- Left in Active Standby with Home in front, every plugin on, the receiver off at -42.0 dB and unmuted (muted while the panel was on).
- Untested on the TV: the cameras' saver lines leaving at the Cameras switch (host test only), and the BLE proxy's switch (host tests).

## Home Assistant plugin fixes 2026-11-06 (main a6c0acf9)

Fixes the three findings below. Installed by the standard path, the set in Active Standby: the bundle from ce38b9c6 at 16:25 EST (cutover `disable`, `install`, `probe`, `enable`), then the helper from a6c0acf9 at 16:30 (staged in `payload/helper/`, `lumatv helper start`); a6c0acf9 changes only the helper and the changelog. Installed sha256: compositor `1ef99f702661155ac844a8679cdee94bba215217082b55811ee6e9d0d28d2406`, ui `6ec9b73b13e1ddbadf983d9fea8aabea1c36684a7b58a2199f41c8b691115d4f`, sm `65f6ca69` (unchanged), helper `203e514bd7f680a5acaa63b5ee8ba560e71163c3f268c8b7c3ae23fd23f5d414`. Replaced: compositor `281a2469`, ui `e1b629f9`, helper `9e400b16`. Receipts: `/data/home/tmp/lumatv-plugin-ha-20261106/fixes/` (`session.log`, `installed-hashes.json`, `old-keys-log.txt`, `i-*`, `i-check.png`).

- Black strip: the plugin switched off while the saver was up left the strip black after the saver went (`g-*`, before the fix). The floor draws stack 0 on its own, and an image layer the frame as a whole did not show had no texture yet. With the fix (`i-off-display.png`) the built-in Atlanta strip shows its icons and temperatures after the saver; `ctl pageprobe off` changes nothing.
- Badge: on again (`i-onagain-display.png`) Pirate Weather is back and the bell shows 1, `ui.home.unread` 1 with the topic holding 1 alert; off, 0 and 0.
- Warnings: after the helper start the log has `helper.json: cameras.enabled …` and `helper.json: onhome.enabled is ignored …` once each.
- Left in Active Standby with Home in front, the plugin on, the receiver off at -42.0 dB and unmuted (muted while the panel was on).
- Untested: the plugin switched on while the saver is up, and the off switch while awake on this build.

## Home Assistant plugin 2026-11-06 (main 44458c3d)

The `home-assistant` switch (e22fb982) and the Plex account without its Photos hand-off (44458c3d). Installed 15:23 EST from 44458c3d (origin and forge main) by the standard path: the bundle with cutover `disable`, `install`, `probe`, `enable`, the helper staged in the app's `payload/helper/` and `lumatv helper start`; `tools/lumatv` (`0473b152`) and the packaged Home Assistant program (`56a01d79`) unchanged. Installed sha256: compositor `281a2469e0754d77aa0afc14073541cbe325b281a20319e8bd21cf3f2c00ba99`, ui `e1b629f9334882492640295ccb7c83d4e731e391f9e880d924752db597f584ba`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da` (unchanged), helper `9e400b16dea6a059581a6e97f8da63559025b7256ef445f164278deb0ada7eb3`. Replaced: compositor `0dff2ac1`, ui `c9628a95`, helper `f0413e04`. Receipts: `/data/home/tmp/lumatv-plugin-ha-20261106/` (`session.log` from 15:21, the PNGs named below).

- Migration: the first start approved `home-assistant` in `plugins/enabled.json` and wrote `ha/plugin-migrated`; helper.json (still `onhome.enabled: true`) and `ha/link.json` are unchanged.
- On (`on-home.png`): the Smart home card with its camera tiles and the Back lamp, Pirate Weather (`ha:weather.pirate_weather`). On Home connected with 46 controllable, the program connected, `ha.link` connected with 1 holder, HA's `binary_sensor.lumatv_connected` on.
- Off through `POST /v1/plugins/home-assistant/switch` (`off-home-awake.png`): within 4 s nothing listens on 6054, the program has ended, `ha.link` is off with 0 holders and the TV holds no connection to Home Assistant; the weather is the built-in Atlanta (30311); Home has no Smart home card and no alert badge. HA's `binary_sensor.lumatv_connected` went off at the switch. The ESPHome device (BLE proxy and TV sensors) kept its 6053 connection and reported at 15:26:54, after the 15:26:06 switch.
- On again with the entry's `approve` (`on-again-home.png`): the program connected at once, On Home after 35 s (the integration's redial) with 46 controllable, `ha.link` 1 holder, Pirate Weather and the Smart home card back, `plugin.home-assistant.notifications` holding its 1 alert.
- Plex (`plex-page.png`): the page says Signed in as nphilip89 with BeastNAS and has no Photos row; `plex.account` is `signed-in`.
- Left in Active Standby with Home in front, the plugin on, helper.json, `settings/*.json` and the theme files byte-identical to before (enabled.json gained only the `home-assistant` switch), the receiver off at -42.0 dB and unmuted as found (main zone and HEOS muted through HA while the panel was on).
- Untested: the Settings switch pressed with the remote, the `homeassistant-off` render scene, a doorbell ring, lamp control.

Findings:

- Off, the weather card's hourly strip drew as a black box without icons, in two shots 30 s apart (`off-home-awake.png`, `off-home-awake2.png`: 94 % of the strip black); on again it drew normally. Fixed in ce38b9c6 (above).
- The bell's alert badge (1 in `on-home.png`) was not drawn after On again, though the topic holds the alert. Fixed in ce38b9c6.
- The `helper.json: onhome.enabled is ignored` warning never reaches the log: `log::log` drops lines before `log::init`, and `Config::load` runs first (`cli.rs`). The Cameras keys' warning is lost the same way. Fixed in a6c0acf9: records sent before the service's own log collector runs are dropped too, so the lines are said once `Service::start` returns.

## Cameras plugin ends its idle feeder when switched off 2026-11-06 (main 099bfc7e)

Helper-only install from 099bfc7e at 13:59 EST, staged in the app's `payload/helper/` with `lumatv helper stop` and `start`: helper sha256 `f0413e041641d3e03c3db7e9650b183b813c5fba81fc76f5a83cd6426b2f48cd` (was `604f071c`). Receipts: `/data/home/tmp/lumatv-plugin-cameras-20261106/` (`feeder-off.txt`, `session.log` from 13:59).

A camera-103 test pop-up left the idle feeder (pid 15291). The switch off was answered 0.03 s after it was sent, and the feeder was gone 0.03 s after that, at the first look (uptime 21140.30 to 21140.36); it used to stay for `feederIdleSeconds` (120 s). Switched on again: ready with 10 cameras. Left in Active Standby with Home in front, the plugin on, helper.json and the theme files as found, the receiver off at -42.0 dB and unmuted (muted through HA while the panel was on).

## Cameras plugin 2026-11-06 (main 4f965451)

Installed 13:35 EST from 4f965451 (origin and forge main) by the standard path: the bundle with cutover `disable`, `install`, `probe`, `enable`, the helper staged in the app's `payload/helper/` and `lumatv helper start`; `tools/lumatv` unchanged (`0473b152`). Installed sha256: compositor `0dff2ac17f1f4722211fd718c845efbf91cbad23557b191a5aedcf40bfcbb70f`, ui `c9628a95420670db2eb9b0b8fede2a2a7b2eec4a24fcc4901abaf227f979ce93`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da` (unchanged), helper `604f071cacd09d597e14b5adff6fb327d8bfae86742d20240d20e52b4dfd7ac5`. Replaced: compositor `3dd284cf`, ui `ba13803e`, helper `77544f13`. Receipts: `/data/home/tmp/lumatv-plugin-cameras-20261106/`.

- Migration: the first start approved `scrypted-cameras` in `plugins/enabled.json` and wrote `scrypted/plugin-migrated`; `link.json` and `settings/scrypted-cameras.json` are unchanged.
- On: the status bar's camera button and the Smart home card's two camera tiles show. A test pop-up of camera 103 was granted the main window at 1348,145 520x292 (15.1 a second); its stream was silent for 2 s and was dialled again (first picture at 3754 ms, 0 lost). The full view of camera 88, opened with `ctl shell show cameras`, was granted the main window (26.2 a second, 0 lost).
- Off through the Settings page's route: the button and the tiles are gone, `GET /v1/cameras` says `off`, the test event answers 409, the bridge's events stop and no connection to it is left. The feeder kept for the next session ends only when `feederIdleSeconds` (120 s) runs out.
- On again: ready with 10 cameras, the button and both tiles are back, and a 103 pop-up played on the main window (15.2 a second). An event that came while LumaTV's saver was up was kept out by the saver's rules (`saver::route_camera`); the route still answers `popup: true`.
- Left in Active Standby with Home in front, the plugin on, helper.json and the theme files byte-identical to before, the receiver off with its main zone at -42.0 dB and unmuted as found (muted through HA while the panel was on). HEOS has reported level 0 since that mute; the main zone's own status page is the reading to trust.
- Untested: the camera button and the Settings switch pressed with the remote, a Cameras card on Home (the layout has none), Ring's doorbell event, the `cameras-off` render scene.

## Backyard's silent starts and the first pop-up after a helper restart 2026-11-06 (main 0e9ee7bc)

Helper-only install from 0e9ee7bc at 09:18 EST: helper sha256 `77544f131ed8e3cf26b222637539b60b80e4485ae4d0731ae3265120c69e42ba` (was `087d4ece`). Receipts: `/data/home/tmp/lumatv-camplane-20261106/` (`session.log` from 09:18, `logs-followup-0918-0941.txt`).

- A live stream with no data 2 s after its PLAY, before any picture, is dialled once more; the reader is shared by the plane and the texture, so the silence is not the plane's. Backyard stays on the main window.
- Camera 88 on the set: full view on a claimed main window 10 of 10 (first picture 151 to 164 ms, 10.3 to 13.8 a second); texture 10 of 10 (178 to 330 ms, 10.0 to 13.5); shell pop-ups on the main window 10 of 10 (151 to 190 ms, 10.1 to 13.1). No start went silent in this run, so the redial is proved by its host test only.
- The 08:56 pop-up with no claim: pop-up ids restarted at 1 with the helper, and the shell, which outlived it, had shown a pop-up 3 before. Ids now start at the helper's start time; the first pop-up after this install showed and played.
- A live start with no window claim has its feeder closed by the compositor ("no claim was granted for it"): TV checks claim with `vwin claim` first.
- Left in Active Standby, helper.json as found (test events off).

## Sizeless cameras' pop-ups on the main window 2026-11-06 (main 87d83650)

Helper-only install from 87d83650 at 08:55 EST by the standard helper start: helper sha256 `087d4ece8998b98d2604790e9a6eff539c7c7ac1f0921e87da6788a0f5dc0ec5` (was `0000c475`), compositor, ui and sm unchanged. Receipts: `/data/home/tmp/lumatv-camplane-20261106/` (`session.log`, `event-popup-*`, `logs-popup-*`, `full-*`, `popup-88-retry.png`).

- The helper read both sizeless streams at its start (88: h265 768x432, 168: h264 640x360) and the cameras topic says `popupVideo: window` for 88, 103 and 168, `stills` for 196.
- Test pop-ups on Home, each granted the main window at 1348,145 520x292: 88 at 11.0 and 11.4 pictures a second (first after 153 ms), 168 at 11.4 (208 ms), 103 at 15.0 (2.18 s). The panel stayed at 119.5 to 120.2 Hz; the compositor flipped 10 to 17 times a second during each.
- Full view on the main window, 88's light stream: 8 of 10 starts at 11.3 to 13.8 pictures a second; in 2 the stream sent no data for 8 s after it opened ("went quiet"), so the decoder never had a picture. The H.265 decoder is not the limit.
- The first test pop-up of 88 (08:56, Home on the profile picker after the saver) was posted but the shell made no window claim and no live session started; the two after it played. Not explained.
- Left in Active Standby, helper.json as found (test events off).

## Camera pop-ups over Home, the UI engine's memory at a restart 2026-11-06 (night, main 75acf6f9)

Installed by the standard path from b6a00593 at 00:14 EST: compositor `3dd284cf`, ui `ba13803e`, sm `65f6ca69` (unchanged); the helper from 75acf6f9 at 00:30, `46895c9c`. Full sha256s in `session.log` of the receipts, `/data/home/tmp/lumatv-idlefollowups-20261106/` (`idle-post-*`, `event-post-*`, `mem-*`).

- Pop-ups over Home claim the main window. Camera 103: granted it, h264 640x480, first picture after 1.06 s, 15.1 pictures a second, the panel at 120 Hz; LumaTV took 15 to 17 % of a core during the 12 s pop-up against 11.9 % before, back within 1 % at +29.9 s. Cameras 88 and 168 list no stream size: with the size read from the stream (b6a00593) they went to the main window, where 88's H.265 stream showed no picture in 8 s and 168's four in 11 s, so 75acf6f9 takes that out and they stay textures. Camera 196 (battery) shows stills.
- Glass at rest on one build, 60 s with no other client drawing: Liquid 10.2 % of a core (16.4 flips a second), Glazed 9.8 % (14.1), Frosted 9.6 % (15.5). WidgetsBisect's Glazed window (11.1 %, 18.8 flips) had a camera pop-up drawn as a texture in it: the feeder at 4.8 % and 253 client commits. `tv-idle-probe.py` fails such a window now.
- A toast on a rested Home is back within 1 % of the 12.3 % before at +24.9 s, the time it shows; nothing stays after it.
- Each UI client restart left 63 MB of GPU memory in the compositor (f802ddd2: VmRSS 265 to 333 MB, Mali native 186 to 248 MB). The engine now gives back 31 to 47 MB at each restart: after three, VmRSS 164 to 173 MB and Mali native 94 to 95 MB (193 and 116 MB fresh). Each start removes about 70 files the client before left in `/tmp/xdg/ui-shm` (580 files, 63 MB after a day). The compositor wakes 253 to 287 times a second at rest, its loop about 55 of them, the rest Mali's GPU threads.
- Left in Active Standby with the theme files byte-identical (cardGlass Liquid, as found at 23:28), pop-ups on, helper.json as found, the receiver unmuted at 38.0.
- Untested: a real camera event over an app; a pop-up of camera 88 or 168 on the texture path with this build.

## HDMI 2's audio decoder after a compositor restart, 2026-11-05 (night, main 7575e89b)

Done 22:53-22:58 in the volume bar session below, the TV woken from standby for it: after the compositor restart `lumatv inputs status` held no port with no input app running, and a Marquee film played. The receiver was on at 38.0 after the wake and went to standby with the set at the end.

`tools/lumatv` `03ddc6a0` installed 22:35 EST (replaced `56899321`); no restart, nothing launched. Receipts: `/data/home/tmp/lumatv-hdmi2dec-20261105/`.

Cause, from SAM's log and LG's CEC log: `cutover disable` restarted to LG's compositor without ending the input apps, so HDMI 2 in the background died with it (exit 11 at 21:35:40 and 21:43:21) and left `HDMI_PORT2` open. `inputs release` then launched HDMI 2, whose CEC Set Stream Path woke the Apple TV; it answered Active Source and `com.webos.service.eim` launched HDMI 2 again 5.5-5.8 s after the release closed it (21:36:50, 21:44:30). Every restart now ends the input apps first, and `inputs release` is removed; install scripts drop that step.

## Volume bar holds its number after a held key 2026-11-05 (night, main f802ddd2, installed 22:53)

Recorded on the set at 22:06 with ed45a97c installed, nothing installed or restarted, over Marquee's menu with no media playing: four 1 s holds (repeats every 100 and 40 ms) and six taps, with the keys, the bar's `ctl shell` level, the helper's posts and MV commands (strace of the helper) and the `receiver` topic on one clock. The receiver was put back to 39.5, unmuted, as found. Receipts: `/data/home/tmp/lumatv-denonvolume-20261105/` (`rec1/timeline.txt`).

- Cause: LG's audio service passes every volume key on to the Denon over CEC too. The helper had the bar's level 8 to 11 ms after the key and the receiver reported it about 40 ms later, but the Denon took the key's own CEC step 120 to 300 ms after the key, usually after that report. The bar gave up its target at the first report of it after the 200 ms hold gap, so three of the four holds then showed the late step and the helper's read-back correction 400 ms after its last command: 36 then 36.5 after a hold down, 40 then 39.5 after a hold up, 35.5 then 36. Each tap overshot one step until that read-back, which could resend a level the bar had already moved past. The first press after 6 s without a change was not counted (the bar's copy of the topic had aged), so only its CEC step moved the receiver.
- f802ddd2: the bar keeps the target until the receiver has reported it after the last post with nothing else for 500 ms, or 1.5 s after the last key; the helper hears every reading, sends the level back when a step moves the receiver after it got there once no level has been asked for 400 ms (within 1.5 s, at most 3 times), resends a failed command and always sends the newest level; a key on an aged copy counts and goes out when the newest copy confirms it. Host replays of the recording: `rustui/ui/src/system/volume.rs` (the four holds: none steps back now, each ends on the receiver's level) and `helper/app/src/receiver_volume.rs` (the hold up: one correction at 1309 ms, none while the keys come).
- Installed 22:53 EST from f802ddd2 (origin and forge main; 55de44be adds only docs and `tools/volume_check.py`) by the standard path, the set woken from standby once LG's panel refresh had run (run 621 to 622): the bundle with cutover `disable`, `install`, `probe`, `enable`, the helper staged in the app's `payload/helper/` and `lumatv helper start`. Installed sha256: compositor `f9e65297a89a27fd9c25f96de21857ed13d7d45d4b47cead3b6910be21447cf3`, ui `3df65190f8c9bfb0917bcae950f87d13de2d120c5a1bed37e074158075100eb4`, sm `65f6ca69` (unchanged), helper `758ce955cdf1d47a42ec68e052a6095c5a0f38881c51e145a4e65371747681ba`, `tools/lumatv` `03ddc6a0` (unchanged). Replaced: compositor `c676f650`, ui `67a45a21`, helper `cbd18df8` (kept in the receipts' `backup/`).
- Checks, `tools/volume_check.py` over Home with nothing playing, receiver 38.0 unmuted (cap 41.0), keys on the SSAP key device: 10 of 10 holds pass (after each release the number did not move, it equalled the receiver's level and no target was left); a change made at the receiver showed 4 of 4 times, 44 to 107 ms after it; on taps the receiver's CEC overshoot (36.0 between two reports of 36.5) never showed and the helper put it back 0.47 s after the key. Key to the bar's new number (`ctl shell` read every 4 to 10 ms): median 12.6 ms, 18 of 20 under 20 ms. The first press, 18 s after the helper's restart, took 135 ms: the bar's copy of the topic had aged, so it counted the key at once but showed it only with the receiver's next report (116 ms after the key); one tap took 141 ms with no report in between (not traced). Receiver back at 38.0 unmuted. Receipts: `vc/summary.json`.
- After the restart: `lumatv inputs status` held "" with no input app running, before and after the film; Marquee resumed Memento (a `media` pipeline with the audio and video decoders, the picture captured at +5 s) with the receiver at 28.0 for 23 s, Back stopped it, the receiver went back to 38.0. Left in Active Standby with Marquee in front.
- Untested: the real remote (the keys came through the SSAP key device), holds longer than a second, a muted receiver.

## Glazed glass at 120 Hz, the calm commit hold back 2026-11-05 (night, main ed45a97c)

Installed 21:44 EST from ed45a97c (origin and forge main) by the standard path: the compositor bundle only, `lumatv inputs release` after the compositor restart (released: true). Installed sha256: compositor `c676f650ea9d7e82b5c87ccb88587ffc55a68b76a57844570537eb7058993ee5`, ui `67a45a21d18b1d36c3dffd0e92190775a42ce3af5411f7baa80b5539d05380f1`, sm `65f6ca69` (unchanged); helper `cbd18df8` and `tools/lumatv` `56899321` untouched. Replaced: compositor `e9b1702d`, ui `7a81895c`. Receipts: `/data/home/tmp/lumatv-widgets-20261105/` (`session.log`, `gate/` with the rings in `gate/rings/`, `idle-wb-*`).

Checks, owner's theme with the Glazed card glass, 120 Hz, 3 cycles each:
- home-widgets 0 dropped and 0 intervals over 20 ms after a frame of a motion (f7226459 under Glazed: 58 and 34; de53f401 under Liquid: 0 and 0); quick 0 and 0 (2 and 2); settings-scroll 2 and 0 (18 and 0), within the re-gate's allowance of 3. The calm commit hold is back and dropped nothing under Glazed.
- Idle Home, 60 s, same build: Glazed compositor 7.65 % and UI 3.46 % of a core (11.11 %, 18.8 flips a second), Liquid 3.84 % and 2.26 % (6.10 %, 13.9 flips a second; the glass switched with `theme set` and the theme files put back byte for byte). Under Glazed the idle frames were mostly the moving wallpaper's steps, each the whole screen with 94 layers, about 10 a second (the ring at 22:05); the backdrops' own time per frame fell from 813 µs (82996a22 under Glazed) to 1 µs.
- The release launched HDMI2, which woke the Apple TV over CEC and came to the front; HDMI2 was closed with `closeByAppId` before the gate and Plex put back in front at the end. A gate run cut off at 21:47 left the pop-ups off, the refresh at 120 and the wallpaper's motion off: all put back, the theme files byte-identical.

## Live TV over the tuner, BER n/a, Hide broken pictures removed 2026-11-05 (night, main 8454edf4)

Installed 21:36 EST from 8454edf4 (origin and forge main) by the standard path, with `lumatv inputs release` after the compositor restart (released: true). Installed sha256: compositor `e9b1702da714e3abcb1a64e77f2416d95a693f7c4f5a16c319b61f476c4682ff`, ui `7a81895c951fab09fb9563f7f34ba5ed7654b7801d0d29537c364954e1ae5c97`, sm `65f6ca69` (unchanged), helper `cbd18df839b26fa5ecdd33982ea186bce0d2ac79b366646dab7f09e1ea4a305e`, `tools/lumatv` `56899321` (unchanged). Replaced: compositor `898ccab5`, ui `8b3eadd7`, helper `2c537463`. Receipts: `/data/home/tmp/lumatv-tuneryield-20261105/`.

Checks:
- The cause, with luna alone and a SCAN pipeline held on 16.1: LG unrealized the pipeline 1.0-1.1 s after Live TV's launch; a `changeChannel` on it then made it `RENDERING` again and Live TV's video never connected (LG's broadcast client left after 419 ms). `stop` and `close`, `close` alone, or leaving the pipeline alone kept Live TV playing.
- Live TV over the meter: PASS on RF 32 (picture) and RF 29 (Live TV up with LG's own "No Signal" at 17.8 dB); the meter read "the TV took the tuner", then "Live TV is in front"; main video connected to Live TV for the 12.5 s watched.
- Live TV over a stream of 2.1: PASS, picture within 2.3 s of the launch and for the 14 s watched; the stream ended with "The TV is using its tuner" and `Preempted("Live TV is in front")`.
- Plex on 2.1 through this helper: 42 segments in 45 s, the first at 3.1 s, the programme 1.0 s after the request.
- Hide broken pictures through Plex on 16.1, a Plex session per run: off and on 52 segments each with identical output, as 16.1's tables never came (31-255 packet errors) and every other RF channel read none; the earlier 0 segments with it on came from one Plex session id reused across runs. Removed.
- The meter on RF 29 shows `BER n/a` for LG's full count; the page has no Hide broken pictures row.
- After the release `com.webos.app.hdmi2` came back in the background and holds `HDMI_PORT2` (as it did before the install).
- Untested: Live TV over a reception check, and Live TV opened with the panel off.

## TV session 2026-11-05 (night, main 65df7ada: screensaver)

Installed 21:00 EST through `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`. Installed sha256: compositor `898ccab5a1bfa855cf86aba826bd44823789082f288362f9ec58448e56961a84`, ui `8b3eadd74eb437e6c91ff3fbc22db8f9de648334b510ab11d48fbbfd4cfabe83`, sm `65f6ca69` (unchanged); helper and `tools/lumatv` untouched. Receipts: `/data/home/tmp/lumatv-saver-veto-20261105/`.

Checks:
- tvpower's request (`power2/turnOnScreenSaver` by hand) over Marquee playing a letterboxed film (1920x818 at y 131): both lists asked, LumaTV answered no, power state stayed `Active`, LG's saver app not launched, nothing drawn. Over Marquee's Home with no video: LumaTV's clock up, Up ended it.
- LG's screen manager now receives Marquee's video window (`fullScreen: false`); it still does not hold the screen-saver timer back for it (165 to 115 s with no key), so outside LumaTV LG's saver still starts over a letterboxed Marquee film unless Marquee answers tvpower itself.
- Each compositor restart left `HDMI_PORT2`'s audio decoder held and Marquee's playback failed ("could not start the video stream") until `lumatv inputs release`.
- Untested: the real 3-minute idle timer running out (only the hand-made request), the hold-Home escape restoring LG's saver (tvpower was seen dropping a client whose connection ended).

## TV session 2026-11-04 (main 0c1d3ae5)

Installed 08:32 EST with `compositor/tools/build-tv.sh`, `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`; `tools/lumatv` and the helper went in as copied files (`lumatv helper start` swaps the helper). `install.py` and `tv-install.sh` (`/data/home/tmp/lumatv-install/`) are stale: they run `lumatv apply`, which no longer exists, and `cutover install` without `disable`; the set's app directory already equalled the 0c1d3ae5 package apart from those two files. Boot-direct stays off (`flag false, api true`).

Installed sha256: compositor `d79501e9fe07631e2ad9c263804a3e07941152d7d28e132d0d73ef1a48e975b0`, ui `03a019d511d9fb7d68360dfff36b5d10ea9bc954da03d82baf21291ffe07d2e7`, sm `f9625a8a57d9a226647c94065cc06c4ee11072bbd428935e6ba01cc0c7b3134d` (unchanged), helper `ab4bd984998717530811a9210b50e2e614961d427a725181d6fb4956b85e7610`, `tools/lumatv` `d95b6fe7474bab00aaddee53bb71a269a8dd34e552d005e4d793f3694d9b6c9c`. Replaced: compositor `bc5a069b`, ui `0a8467e6`, helper `94ebaded`, `tools/lumatv` `bc28257a`. Receipts, backups and the installed binaries: `/data/home/tmp/lumatv-tv-20261104/` (`results.json` lists every receipt path). The staging worktree `/tmp/lumatv-r28` (target of `/data/home/tmp/lumatv-next`) is at 0c1d3ae5 with its build. End state: TV on, LumaTV's compositor live and persistent, failures 0.

Checks, with keys injected through `ctl remote` (the real remote and the wheel were not used):
- Install health PASS. Keymap PASS: Live TV logged 18874401 and 18874402 for CH+ and CH-, Back 18874371 and the digits; OK, Back and arrows worked in Marquee, YouTube and Netflix. Back-to-Home PASS with Homebrew Channel and Punktfunk. Live TV banner PASS: LumaTV's only, gone after 5 s, INFO card and a second INFO ends it.
- Channel guide FAIL. `ctl shell show guide`, OK, Back and CH± after it work, but the Guide key opens LG's guide: `com.webos.app.livemenu` is an Overlay window and the `foreground` topic reports only the card (`input.rs` `foreground_app`, `guide/host.rs` `update`), so the guide's trigger never fires. LG's guide needs another trigger (the overlay in the topic, or the key).
- Smart home card: focus look PASS in both looks. Lamp control is off in the helper (`controllable 0`), so OK on a lamp opens its item menu and sends nothing. No sensors are shared (sensor sheet untested). The card scrolls both ways on `ctl wheel`.
- Weather card PASS on an overcast sky at 120 Hz: 120 fps, worst interval 8.4 ms, 0 dropped. Storm and snow moments untested.
- Cameras PASS: grid, Front Door full view with the blurred fill, Downstairs Door note with Try again and the helper's reason line. Offline retry untested (all cameras online).
- 120 Hz gate (home-idle, home-dpad, quick, settings-scroll, home-widgets) FAIL: panel 119.9 Hz and 0 dropped frames everywhere, but the first frame after a key took 69-81 ms (quick 819 ms, OSD off at the key) against a 50 ms limit, and home-dpad, quick and home-widgets had 1, 1 and 4 intervals over 20 ms. The 30 Oct run had first frames of 1.2-9.8 ms. Explained: not a latency. lginput2 made the SSAP key device (`event10`) and `event12` anew at the 08:50 wake, the compositor had read only the devices found at its start, so the gate's keys never arrived; the idle timer ran out at 09:05:57 (`ctl keys` counted 250 keys for the session, the panel dim from 09:06:15 lasted until the owner's Power key at 09:17:12) and LumaTV's saver was up, drawing its second hand every 100 ms. The "first frame" was the wait for the next sweep, uniform over 0-100 ms. Fixed on main (the compositor opens devices made after its start; the gate refuses to run when the compositor has not opened the key device). The numbers need a new run on the fixed build.
- Untested: real CH+ and CH- keys, the Magic Remote wheel direction, LG's banner returning when the UI ends, the sensor sheet, the storm moment, the offline retry.

## TV session 2026-11-04 (afternoon, main 2f2559c1)

Installed 14:54 EST with the set in standby (the panel stayed dark until the 14:59 wake) through `compositor/tools/build-tv.sh`, `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`; no reboot, boot-direct off. `tools/lumatv` (hash equal to the morning's) and the helper (sources unchanged since 0c1d3ae5) were not replaced. A helper built in another checkout is not byte-identical (`6b2eb3e2`, 8979832 bytes, against `ab4bd984`, 8985136) because the layout follows the checkout path, so a same-source rebuild is no reason to swap it. `tools/build-ipk.sh` needs the staging tree's capture, jpeg, vplane, gfn and card builds and does not run in a fresh worktree.

Installed sha256: compositor `01c79f54ffa16bf087a410ffef27997c79411c5b47ea499662c49e084a08388f`, ui `ff561261e170cbfa63b5db3377717e97513a1749f83b89c8fdd268ea5f170f55`, sm `f9625a8a57d9a226647c94065cc06c4ee11072bbd428935e6ba01cc0c7b3134d` (unchanged), helper `ab4bd984998717530811a9210b50e2e614961d427a725181d6fb4956b85e7610` (unchanged), `tools/lumatv` `d95b6fe7474bab00aaddee53bb71a269a8dd34e552d005e4d793f3694d9b6c9c` (unchanged). Replaced: compositor `d79501e9`, ui `03a019d5`. Receipts, backups, binaries and the deployed bundle: `/data/home/tmp/lumatv-tv-20261104b/` (`results.json` lists every receipt path). End state: TV on at Home on Nitin's profile (Isabel's was active at the start; Nitin switched at 15:02 and stayed), compositor live and persistent with failures 0, boot-direct off, refresh policy auto, camera pop-ups on, theme as found.

Checks (keys injected into the SSAP key device by `kb.py`; Nitin used the real remote for the physical steps):
- Install health PASS. Input hotplug PASS: the compositor opened `event1` (Smart Remote RCU Input) and `event12` (LGE Network Input), which the old build never had, and watches `/dev/input`; a Bluetooth controller's nodes (`event8` to `event10`) that came back at each wake were noticed within 3 s and left alone. lginput2 did not re-create its own devices at either wake, so a uinput node (`/tmp/lumatv/uinput-key.py`) stood in: opened in 1.6 s, its keys delivered, made anew a second time and opened again. Not repeated: the real remote right after the 15:08 wake.
- Guide PASS on every route. The Guide key (injected, and Nitin's real one) opens LumaTV's guide and Live TV sees no key. LIST and the banner's Guide button go through SAM: the journal has `LG's guide launched (SAM launched it)`, `closeByAppId` answered and `window N of LG's guide is mapped and is not drawn`, and no capture shows LG's guide. OK tunes (5-4 to 2-2) and closes it, Back closes it without Live TV seeing the key, CH+ works after (2-2 to 2-3). The layout of 2f2559c1 reads clean on the set: the now-dot shows under the ruler, titles are whole or absent, no line runs through a word, the sheet hides the page.
- Camera labels in the light look PASS: each label sits on a visible plate and reads over the foliage.
- Ring downstairs door FAIL against "2 to 5 s": the bridge answers in 3.5 s every time; the first start played at 10.9 s (7.4 s after the bridge, 24.2 fps for 37 s, 0 lost), the next two (a fresh open, then Try again) got "Scrypted sent no picture in time" after 8.5 s. The grid tile showed "No picture" and then Scrypted's own "Snapshot Timed Out" picture.
- 120 Hz gate FAIL: its key-device check passed, but the TV re-timed the panel to 100 Hz (10.00 ms, at about 15:17 and again at the gate's start) and kept it: 99.9 Hz on every surface against 119.9 this morning. First frame after a key: home-dpad 928 ms, quick 1323 ms (no burst), settings-scroll 474 ms, home-widgets 9.0 ms (limit 50; 30 Oct 7.6, 9.8, 1.2 and 8.0), with the OSD off at the key on the three slow ones. Compositor-counted drops: 6, 14, 3, 17 and 13 over home-idle, home-dpad, quick, settings-scroll and home-widgets. The causes are below. The gate also writes `"weather.animated": true` into theme.json when it puts the setting back, so the file stops being byte-identical; restored by hand.
- Open on Live TV: LG's own Guide button on the launch banner stays drawn. The claim covers the bar only (`x 0, y 556, w 1506, h 168`; `option.bannerPosition` is `none`) and the button (x 27-251, y 743-796) lies below it, drawn by Live TV's own surface (`ctl windows` lists no other window). LumaTV's banner (x 64-1518, y 870-1023) lets the picture show through its plate.
- Untested: the real remote right after the second wake, a doorbell start while the doorbell is awake.

### Causes of the afternoon's 120 Hz gate failure

- The panel ran at 99.9 Hz because the Ring doorbell's full view (a 25 fps stream in the main window) made the TV re-time it at 15:17:16; the compositor's log has `the panel refreshes every 10.00 ms now (8.33 before)`. Only a 30 or 60 fps video restores 120 Hz (`docs/research/vwin-refresh-result.md`). `gate.sh refresh 120` printed `DE2VSYNC/s 100.1` and the gate ran on; it now measures the panel first and stops.
- The OSD was not off. In each of the 26 captures (of 41) that lost CPU 0's head, CPU 0 has no event of any kind (timer, USB, network, scheduler, OSD) from the capture's start until the OSD interrupt appears, while CPUs 1 to 3 are normal, and the dump's header counts lost events (`entries-in-buffer/entries-written` 498175/550637 for quick-m0-r2): the ring overran on CPU 0, which takes every device interrupt, at 20.8k to 33k events a second (11.4k on 30 Oct). The load is a USB game-controller dongle polled at 1 kHz (`ohci_hcd:usb8` 1000/s against 62), `eth0` 550/s (95) and `ipc_vdec` 330/s (0). The compositor's own flip ring shows a flip every 10 ms through the same stretch and the first flip 32 ms after the quick key; key to first flip over 51 open windows is median 16 ms and p90 28 (30 Oct: 12 and 22 over 72). On the idle set `osd_irq` ran at 99.9/s for 150 s with no gap over 80 ms, `lg1k-fb` cannot runtime-suspend (`power/control` is `on`) and all five PQE-PM clients are active. This morning's 120 Hz run shows the same (17 of 36 captures), 30 Oct 9 of 129, and so does round 11's Dolby Vision finding.
- The compositor's two-second look at `/dev/input` (9527ab15, installed 14:54) held its loop for 115 to 178 ms in every capture, against 0 or 1 on 30 Oct and this morning; these are the 6 to 17 drops a surface. The look now skips nodes with no device behind them and nodes already judged.
- The run took 1387 s against 873 because LumaTV's own GPU jobs counted as an app's drawing and every capture of a key was repeated twice.
- Open: Home is not at rest. Idle flips a second before the first key: 2 on 30 Oct, 14 on Isabel's profile this morning (`bc5a069b`), 96 on Nitin's profile (`2f2559c1`); empty frames (`owed 0 px`), the UI engine asking for every refresh, the compositor at 17 % and the UI at 15 % of a core on an idle set. Which animation of the Smart home, weather or camera cards holds it is not found.
- Untested on the set: the input fix (host tests only), the 16 MB capture ring's memory (32 MB more while a capture runs; 643 MB was available), the gate's new checks in a real run.
- To re-measure: play a 30 or 60 fps video for a few seconds with no camera pop-up open, `gate.sh refresh 120` and check `DE2VSYNC/s` is near 120, install the build from main, run the 120 Hz gate. Expected: no `warn loop: 1xx ms in one pass` every 2.0 s in `ctl ring`, `trace_complete` passing, key to first flip 10 to 30 ms.

## TV session 2026-11-04 (evening, main 749156b0)

Installed 17:10 EST with the panel on at Home (Nitin's profile) through `compositor/tools/build-tv.sh`, `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`; no reboot, boot-direct off. `tools/lumatv` and the helper (sources unchanged since 2f2559c1) were not replaced.

Installed sha256: compositor `51858d6422f28ad3be3735592ba6ed7e8bc19bf94a519814f999e06917000a55`, ui `3834e7743f4e6419291ee80cf47adfa0f06ce2f08f67117e223d5a37d4228ff2`, sm `f9625a8a57d9a226647c94065cc06c4ee11072bbd428935e6ba01cc0c7b3134d` (unchanged), helper `ab4bd984998717530811a9210b50e2e614961d427a725181d6fb4956b85e7610` (unchanged), `tools/lumatv` `d95b6fe7474bab00aaddee53bb71a269a8dd34e552d005e4d793f3694d9b6c9c` (unchanged). Replaced: compositor `01c79f54`, ui `ff561261`. Receipts, backups, binaries and the deployed bundle: `/data/home/tmp/lumatv-tv-20261104c/` (`results.json` lists every receipt path). End state: TV on at Home on Nitin's profile, compositor live and persistent with failures 0, boot-direct off, refresh auto, pop-ups on, theme files byte-identical to the start.

Checks (keys injected into the SSAP key device; Nitin could not get to the TV):
- Install health PASS: live, mounted, persistent, failures 0, native Home visible. `check-boot.py --now` still ends FAIL on its boot-line criterion alone (the 10:52 boot is older than the RAM log; no reboot).
- Input look (67ecf63c) PASS: no `warn loop` line in 178 s at rest (2f2559c1: one of 122 to 143 ms about every 2 s). Since the install 17 loop warnings of 8 to 17 ms, none periodic (the gate's captures, a Home key, the switcher opening). The compositor holds event0 to 7, 11 and 12 open as before; one kb.py Up raised `ctl keys` delivered from 2880 to 2882; a uinput node made twice was opened as event14 within 2 s and its Right and Left were delivered.
- Slim banner (413c7e85) and Guide-chip mask (8d865013) PASS on a Live TV launch at 1.5, 2.9, 5.0 and 5.6 s: no LG Guide button (the mask claims the bar and a 237 by 66 chip at 21, 737), the strip centred with its foot at about y 890 and gone at 8 s, its plate opaque (two different pictures differ by at most 3 levels inside it). CH+ puts the new channel in the strip in 0.4 s, INFO opens the expanded card, Back removes it.
- Idle Home, read-only, Nitin's profile, without IdleHome's fix: at 120 Hz with the screensaver off (60 s, no key) 119.9 flips a second, every flip animating, 0 empty frames, the compositor at 30 % and the UI at 75 % of one core; at 100 Hz with the screensaver up 99.9 flips, 33 % and 75 %. `ctl` has no Demand field; the count of animating flips stands for it.
- 120 Hz gate, provisional (the idle fix is not installed): five surfaces, 3 cycles, 17:39 to 17:58, panel 119.9 Hz in every window, every trace complete, log errors 0, p90 render 1.6 to 2.8 ms. First frame after a key: home-dpad 5.6, quick 2.5, settings-scroll 4.8, home-widgets 7.5 ms (limit 50; afternoon 928, 1323, 474, 9.0). Compositor drops over home-idle, home-dpad, quick, settings-scroll, home-widgets: 0, 3, 3, 9, 8 (limit 0; afternoon 6, 14, 3, 17, 13). `burst_over20` 3, 13, 10, 10 on the four keyed surfaces (limit 0; afternoon 0, 1, 0, 1), and quick's burst ends 1795 ms after the key (limit 180). Only home-idle passes.
- Untested: the real remote (the three physical steps), OK pressed 7 s after a launch (LG's banner with its chip), the strip over a bright ad and in the light look, a device made anew by lginput2 at a real wake, the Ring.

Findings:
- The panel stayed at 100 Hz after the afternoon's Ring test until a Live TV launch at 17:25 brought it to 120.5 Hz; gate.py's panel check passes only after such a video.
- Back on Home with an app held opens the app switcher for 45 s; Up is a harmless key to end the screensaver.
- The install regenerates `/var/lib/lumatv/theme/Tokens.js` (accent `#A79BF5` becomes `#9898E1` from the same theme.json); restored to the bytes found. The gate still rewrites theme.json and the profiles index while it runs; restored byte-identical.
- gate.py needs 3 cycles or more and five surfaces take 19 minutes. The compositor's startup `info trace:` lines fill the 512 KB RAM log in about 4 minutes.

## TV session 2026-11-04 (night, main f98fb940)

Installed 19:25 EST with the panel on at Home (Nitin's profile) through `compositor/tools/build-tv.sh`, `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`; no reboot, boot-direct off. The helper (clippy-only source changes since 749156b0) went in as a copied file followed by `lumatv helper start`; `tools/lumatv` (unchanged) was not replaced. Main's b876d523 (Settings switches) came after the build and is not installed.

Installed sha256: compositor `90f02541def04d054ab9326d49245d88bd25531a274890c0714597b32436a6c6`, ui `a34981996fa8e6a2cbfdfd7766704b6533e5c4cff171f9558897ec55fe6fbf3a`, sm `f9625a8a57d9a226647c94065cc06c4ee11072bbd428935e6ba01cc0c7b3134d` (unchanged), helper `48d10c50fcede369eadd39e506f346389536964b9dd09863e925619abece5e68` (built in `/data/home/wt-tv4`; the layout follows the checkout path), `tools/lumatv` `d95b6fe7474bab00aaddee53bb71a269a8dd34e552d005e4d793f3694d9b6c9c` (unchanged). Replaced: compositor `51858d64`, ui `3834e774`, helper `ab4bd984`. Receipts, backups, binaries and the deployed bundle: `/data/home/tmp/lumatv-tv-20261104d/` (`results.json` lists every receipt path). End state: TV on at Home on Nitin's profile, compositor live and persistent with failures 0, boot-direct off, refresh auto at 120 Hz, pop-ups on, theme files byte-identical to those found, lease released.

Checks:
- Install health PASS: live, mounted, persistent, failures 0, helper answering, native Home visible. `check-boot.py --now` still ends FAIL on its boot-line criterion alone (no reboot).
- Idle Home, Nitin's profile, screensaver off, 60 s without a key, the sky moving as Nitin chose: 119.9 flips a second, every one animating, UI frames 119.9 a second with 0 empty; compositor 24.8 % and UI 6.8 % of one core (749156b0: 30.2 % and 74.8 %). With the sky switched off in the theme, as the gate does for its still surfaces: 8.5 flips a second, none animating; UI frames 8.5 a second, 4.5 of them empty; compositor 3.1 % and UI 1.9 %.
- 120 Hz gate, five surfaces, 3 cycles, 19:33 to 19:50: panel 119.9 Hz in every window, every trace complete, log errors 0, p90 render 2.1 to 2.6 ms. First frame after a key: home-dpad 10.2, quick 8.0, settings-scroll 7.7, home-widgets 4.4 ms (limit 50; 749156b0: 5.6, 2.5, 4.8, 7.5; later now because Home no longer flips at every refresh, so a key waits for the next tick). Compositor drops over home-idle, home-dpad, quick, settings-scroll, home-widgets: 0, 1, 1, 10, 7 (749156b0: 0, 3, 3, 9, 8; limit 0). `burst_over20` on the four keyed surfaces: 0, 1, 16, 7 (749156b0: 3, 13, 10, 10), and quick's burst ends 132 ms after the key (749156b0: 1795; limit 180). Home-idle and quick's burst end pass; the rest still fail. The idle fix explains quick's burst tail and most of home-dpad's and quick's drops; settings-scroll and home-widgets did not change. Their late flips sit at the same moments of every window (settings-scroll at 0.07, 3.5 and 5.9 s with the compositor's pass at 9 to 12.6 ms against the 8.33 ms refresh; home-widgets at 4.4 to 5.9 s), so they come from those surfaces' own frames.
- 120 Hz restore (7e9982f5) FAIL: after the Ring's full view re-timed the panel to 100 Hz (`kms: the panel refreshes every 10.00 ms now`, 3.8 s after OK) and Back closed it, the panel stayed at 99.5 to 100.8 Hz with `connect_src=NONE`; no `the panel refreshes every 8.33 ms now` line, osd_irq about 100 a second. The compositor's request did go out, but 121.5 s after Back (see the findings), and the firmware ignored it.
- Nitin on the real remote: Home feels instant; Live TV shows the new bar with no grey Guide button (`shots/N-livetv-a.png`, `N-livetv-b.png`); the Info, Back, Home sequence ran (Home in front again at 19:33:09), no verdict reported for it.
- Boot direct was not turned on: Nitin decided at 19:40 that the design changes first (no 5 s hold). The flag is off.

Findings:
- Closing the Ring's full view leaves the cameras app's hidden VideoWindow (`org.nphil.lumatv.cameras`, 16 by 16, not exposed) in the scene for 120 s. `panel_rate_clear` counts any VideoWindow, so `refresh: the panel was left at 100 Hz by a video; 120 Hz asked for, no video window left` came at +121.5 s, not +1.5 s.
- That request is ignored with no source connected: 16 s later the panel was still at 100 Hz. The helper's own `1 60` write during the view (`frame rate: wrote 1 60 to fr_rate ... the output measures 99.9 Hz`) was ignored too. A Live TV launch (59.94 fps) put the panel back at 120.0 Hz.
- The Ring's stream showed no picture in the first of two tries (`Scrypted sent no picture in time`, 50 s, the panel stayed at 120 Hz) and played in the second.
- Killing the install client between `probe` and `enable` does not stop `cutover enable` on the set; a second `enable` answers "the compositor is already mounted".
- The install regenerates `/var/lib/lumatv/theme/Tokens.js` again (`2a7ffbce` becomes `0d8c012d`); restored to the bytes found. The gate rewrote theme.json and the profiles index again; both restored byte-identical.

## TV session 2026-11-04 (late night, main c7704853)

Installed 21:53 EST with the panel on at Home (Nitin's profile) from c7704853, built in `/data/home/wt-tv5` once the commit was on origin and forge main, through `compositor/tools/build-tv.sh`, `tv-first-light.sh deploy` and cutover `disable`, `install`, `probe`, `enable`; then `tools/lumatv`, `index.html`, `launch.js` and `app.js` as copied files and `lumatv helper start`. The three app files in the repo are templates (`{{displayName}}` and the like); they were rendered the way `tools/build-ipk.sh` does, `appinfo.json` and `style.css` were left alone. `helper.json` had no `frameRate` key before the helper started. Main's later commits (23470fba up to 7c3c8d36) are not installed. This build starts every compositor with the retime clip on (e888426a turns it off by default), so `ctl retime off` was run after each start and it reads off now; any restart before the next install turns it on again.

Installed sha256: compositor `2005f0ad24ae22c0f4c6d624d952599d3a9a8584f6135a0915411fee023fd2d1`, ui `2f4fa0e9e15c0db05662d72a3bf6dca52668355046eb06297b5af5de9a3988bb`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `4eaf20b1545861bc83534659defec21701e36fa7138df1652551e972e16289ec` (built in `/data/home/wt-tv5`), `tools/lumatv` `19e131b656e4e07b42faff9394a39195194aee4a6ab71e2e7bbcebe68d16670e`, `index.html` `e180ece6b8f5b7aa38d88de83a40fa4a6e0373ac932ea4a959245bf1c427969b`, `launch.js` `dab85ec16ecdeef476c073ba014117e0e50f5ca27dcd4f6a3be760e301cb030e`, `app.js` `c0e60f5f7d48d46d07fc41be90bc47e490f249f0f743be6ffac0ade6e632cbe4`. Replaced: compositor `90f02541`, ui `a3498199`, sm `f9625a8a`, helper `48d10c50`, `tools/lumatv` `d95b6fe7`, `index.html` `41656e4e`, `app.js` `467de598` (`launch.js` was absent). Receipts, backups, binaries and the deployed bundle: `/data/home/tmp/lumatv-tv-20261104e/` (`results.json` lists every receipt path). End state: TV on at Home on Nitin's profile, compositor live and persistent, failures 0, boot direct on, retime off, lease released.

Checks:
- Install health PASS: live, mounted, persistent, failures 0, helper answering. `check-boot.py --now` ends FAIL on its boot-line criterion alone (no reboot).
- Idle Home, Nitin's profile, saver off, 60 s without a key, the sky moving: 119.88 flips a second, all animating; UI frames 119.9 a second, 0 empty; compositor 25.2 % and UI 7.6 % of one core (f98fb940: 24.8 % and 6.8 %).
- 120 Hz gate, five surfaces, 3 cycles, 22:14: panel 119.9 Hz in every window, every trace complete. home-idle and home-dpad PASS; quick, settings-scroll and home-widgets FAIL (f98fb940: only home-idle passed). First frame after a key on home-dpad, quick, settings-scroll, home-widgets: 4.1, 9.6, 8.3, 6.3 ms (f98fb940: 10.2, 8.0, 7.7, 4.4). Compositor drops over the five surfaces: 0, 0, 0, 6, 4 (f98fb940: 0, 1, 1, 10, 7); `burst_over20` on the four keyed surfaces: 0, 3, 12, 9 (f98fb940: 0, 1, 16, 7); quick's burst ends 176 ms after the key (limit 180; f98fb940: 132).
- Retime FAIL (visible). `tv-retime-probe.py restore --clip25` (a 640x360, 25 fps, 710-byte clip made with ffmpeg) provoked 100 Hz at 22:42:32.9; the panel was back at 120 Hz at 22:42:38.0, 0.4 s after the retime clip began and 2.3 s after the 25 fps claim ended, stayed there, and everything was cleaned up. Nitin saw no blink and no square top left, but the picture left Filmmaker Mode while the black box stood bottom right and came back after it. LG's logs hold no pqcontroller, videooutputd or picture-mode line for 22:42:30 to 22:42:50, and which of the two videos does it is not isolated. The Ring's full view retimed once as well (22:15, back at 120 Hz 2.3 s after Back).
- Saver probes PASS: the ScreenSaver timer is 180 s and its 5 subscriptions answer (T1); the pad, touchpad and smart-remote touchpad nodes are opened "(activity only)" (A); the video saver came up after 181.6 s and a key restored the dim in 0.053 s and removed the saver in 0.062 s (T6); a virtual pad and a mouse removed it in 0.056 and 0.053 s and restarted the timer (B); LG's saver over `org.webosbrew.hbchannel` ended 0.358 s after OK with the app still in front (T3); Power with LG's saver up gave Active Standby 243 ms later and `powerOn` with reason `remoteKey` woke the set to Home (T5). The dim had reached 74 of 75 when T6's key came, so the restore from the ramp's 30 is untested. Receipts `saver/*.json`.
- Nitin on the real remote, 22:57: smart home card, camera button, dock and Settings all OK.
- Boot direct (`bootdirect/`): `lumatv boot-direct on` PASS (flag, stub, sam web, no Flutter Home). Home held 3 s stayed in LumaTV. Home held 10 s at 23:01:45: the supervisor's escape, every LumaTV process gone, LG's Flutter Home up, stub off, flag on, no failure counted, crash bundle `kind: escape`. LumaTV app opened 23:02:50: stub in at 23:02:51, compositor on at 23:03:00, no Flutter Home, escape marks gone (the tmpfs one stays), no LG Home flash seen. Reboot 23:04:13: SSH after 14 s, stub in at 17.3 s, SAM's first Home launch at 29.4 s, native Home at 33.9 s (2026-11-03: 33.5 s), no Flutter Home process, client or NL_HOME_SHOWN, no strike. `check-boot-direct.py --json` FAIL on two criteria: failures 1 (the reboot came 73 s after the app's `compositor on`; the supervisor counts a run that never settled, two healthy minutes, and clears the count after 10 healthy minutes, here at 23:14:46) and the last ("LumaTV is in front"), which reads `homeDrawn` and `bootDirect` from `ctl shell` (see findings). By hand both hold: the front window is the stub's card from WebAppMgr, no flutter-client among the clients, the UI ready, `ui.home.home` true on Nitin's profile. The flag stays on. The 30 s unplug cold start and a reboot after an escape (the "escape of an earlier boot has lapsed" path) were not run.
- Resources at idle Home, 30 s, 4 minutes after the reboot (`bootdirect/resources-idle-home-after-reboot.json`): compositor 171 MB RSS and 23.7 % of one core, UI 54 MB and 7.1 %, helper 38 MB and 1.6 %, the rest under 3 MB each; the stub 32 MB; no Flutter Home (72 to 80 MB when it ran under LumaTV). MemAvailable 879 MB of 1.94 GB, CmaFree 75 MB of 676 MB.
- Untested: the cold start after a 30 s unplug, the boot after an escape, T7, the saver C/D matrix, CEC, the hidden-cursor pointer wake, the wallpaper sink's appId (no wallpaper video is installed: `lumatv vplane status` installed false), the retime clip's picture effect on its own.

Findings:
- `tv-retime-probe.py play` ends in `AttributeError: 'str' object has no attribute 'get'` (line 317): `ctl status` `summary` is a string and the verdict reads `.get("summary", {}).get("holes")`. Judged by hand from the run's evidence, the decoder connected, the kernel read 30 fps, the rate stayed 120 Hz and the sink was hidden and cleaned up; `no_hole` was not computed.
- The Ring's live view: `live session 2 on camera 196 failed: Scrypted sent no picture in time` at 22:37:47 (the bridge answered in 3.5 s of its 8.5 s allowance; no picture for 46.7 s); `no picture shown after 7.9 s` at 22:55:11. It played once, at 22:15.
- Front Door (camera 104), full view: the first start after another camera's view ended fails with "can't play camera: an earlier write to the feeder failed" (three times, each after 0.0 s) and the retry plays, small. Scrypted offers the camera at 1920x2560 (portrait), 896x672 and 640x480, none with codec or rate; the view asks for `h264 640x480 at 0 a second declared` and shows about 10 pictures a second. Back Door: "the feeder went: Broken pipe (os error 32)" at 22:55:56. `evidence/frontdoor/FINDINGS.md`.
- `ctl shell` in c7704853 has no `bootDirect` and `homeDrawn` keys (control.rs answered with `shell_rust_report()` since 8f6c04ab), so `check-boot-direct.py`'s last criterion cannot pass; 7c3c8d36 restores them and is not installed.
- A reboot less than two minutes after a compositor start counts as a failure (`the previous run never settled or ended cleanly`) and the second in a row makes the supervisor refuse LumaTV; a test that follows `compositor on` with a reboot has to wait.
- `ctl journal` keeps 200 lines and does not carry the input-node lines ("(activity only)"); the RAM log does.

## TV session 2026-11-05 (Live TV Tuner, main 7e8b8f43)

Installed 15:37 EST from 7e8b8f43 (origin and forge main) by the standard path; the set stayed in Always Ready, off, throughout. Installed sha256: compositor `6c5ae2575734d606e0c09966967c2aa539658ab4abe35ab866111431d063a0fe`, ui `0db85b2b8c0f5acde8439a60c56349fb2627fb4f00b2b373589129a1d4809413`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `817bcb1178c13ef56ec603e1c6e250b0f5e9f12aaecb70ee747c1b0421fe83a1`. Failures 0. Before it the same afternoon: ddfe579c (15:00) and 4ffaba19 (15:29), whose read of the demodulator's report aborted the helper on each refusal.

- Plex's Error 16 on 16.9: its PMT names a picture and a sound that carry no packets, so the tuner sent only tables, Plex's transcoder reconnected for 25 s and the next channel (17.3) was refused as busy behind it. 16.9 is now refused with 503 and 807 in 6.3 to 13.5 s, "not on air" when the demodulator counts no packet errors and "weak signal" otherwise.
- Plex's API, one client 2.1, 16.9, 17.3, 5.1 (tune request to the client's first HLS segment): 5.9 s, failed at 25.1 s, failed (busy), 9.6 s on 429039de; 3.3 s, refused at 7.1 s, 4.3 s, 4.0 s on 7e8b8f43 (3.9, 6.8, 4.3 and 3.6 s with a 720p video transcode). Plex's log has Error 16 for 16.9 only and no "Invalid frame dimensions" (78 in the 14:03 session from an Android client); `/livetv/sessions` was empty after.
- Sweep of the 59 channels: 52 good, first byte median 1.81 s (3.27 s on 429039de), every stream PAT then PMT first. RF 29 was weak (145 to 255 packet errors): 16.7, 16.8 and 66.1 refused for tables not whole in 8 s, 16.9 for no picture. RF 35 does not lock.
- 16.3, 16.4 and 16.9 five times each: 0/5 each on ddfe579c (its 30-minute cache), 4/5, 5/5 and 0/5 on 7e8b8f43. Plex's transcoder at debug level found the stream info 1.5 to 2.5 s after opening 2.1, 17.3, 5.1 and 16.4, with no "Invalid frame dimensions" (1 to 3 a probe on ddfe579c).
- Not done: the Guide data plugin's switch-on in Settings (30311, Plex then the free listings). Receipts: `/data/home/tmp/lumatv-tv-20261105g/` (not in git).

## Home Assistant device: Bluetooth proxy fix, TV sensors 2026-11-05 (night, main ea8f2f49)

Home Assistant's ESPHome entry "LumaTV Bluetooth" held the key from before the TV's key was made again on 11-02, and the proxy's Noise hello carried a 17-character MAC that aioesphomeapi cuts to 16, so a failed handshake read as another device and Home Assistant stopped reconnecting. e79794ae (LumaShow `esphome-api`) sends ESPHome's 12-hex form; Home Assistant then raised its reauth, which took the TV's key (Nitin's OK). `ble-link --standby` is on. f1cf93b1 makes the proxy and TV sensors plugins (`docs/plugins/ha-device.md`).

Helper-only installs by the standard helper start, compositor `84a7091b`, ui `4109df20` and sm `65f6ca69` unchanged: a6ccf051 at 19:30 EST (helper `4fe35234a3804c274c78df679aa9fd960e35f7fe118873c5761787826d148d77`), then ea8f2f49 at 20:38 EST (helper `2c5374638c33dd2e9510ca2e2f8de919d8e34eb7e1058abfd512001461d8103a`, was `aedf49c2`). The migration switched the Bluetooth proxy plugin on; TV sensors was switched on through the Settings route.

- Home Assistant lists `lumatv-ble (84:A3:29:B9:4D:43)` as a passive ESPHomeScanner: 17 to 28 devices, the freshest under a minute old, with the screen on and in Always Ready (3 windows in 2 min 45 s; advertisements 81 to 168 on a6ccf051, 254 to 292 on ea8f2f49).
- The ten `*.lumatv_bluetooth_*` entities exist and update: load and memory every 30 s, temperature every 60 s in standby, and power, screen, app, input (HDMI 2 reads "AVR-S760H") and picture mode within a second of a change.
- Idle cost over 301 s with Home Assistant subscribed: the helper's worker 0.56 % of one core in all; the `ble` thread 11 ticks (0.04 %), `tvsensors` and its two `luna-send` subscriptions 0.
- Untested on the set: the tuner's signal while a channel streams, and the Settings page (ships with the ui of any main from f1cf93b1). `cargo test --test scrypted` fails 2 live tests on main without this work. Receipts: `/data/home/tmp/lumatv-tvtoha-20261105/` (not in git).

## Plex QR sign-in 2026-11-05 (night, main bf40856b)

Installed 20:11-20:14 EST from bf40856b (origin and forge main) by the standard path, with Nitin's app (`com.beb.plxnative`) in front; it was relaunched after. Installed sha256: compositor `28f9286c9035ddeaf52264a519d72974b4d4635b913ad9bcb4ae904b36fcfb24`, ui `e879bdfad03b098dab4ab83bc2ebba4db37bea567475f00d1b8d3c34dc01181f`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `2ed2d64527695c91c63bf85a9ffac09cee02549bc494af2b78c529695869e811`. Failures 0; check-boot's activation line is absent without a reboot. f7226459's helper (aedf49c2) replaced this helper later the same evening.

- Guide data's Plex address and token became the Plex account's on the first start: `plex/account.json` 0600 (keys server, servers, token, user), `guide/secrets.json` empty, no `plex.` key left in `guide-data.json`; the client identifier is PlexConnect's `lumatv-LGwebOSTV`. plex.tv named the account (nphilip89) and its server BeastNAS at 192.168.1.69:32400; Plex also lists the server's Docker bridges (172.17-21.0.1) as local connections, so the address nearest the TV's own is taken. Guide data's page says "BeastNAS (192.168.1.69:32400)".
- Settings › Inputs & devices › Plex: "Signed in as nphilip89" and the server; Sign in showed code DFTS with a QR code that decodes from the panel capture to `https://plex.tv/link/?pin=DFTS` (Plex's own QR for a PIN carries the same address). The helper's curl children were seen 2 s apart while the code showed; Cancel went back to signed in. No link was completed and Sign out was not run (it would revoke the token).
- ea8f2f49 keeps the account's server in use while a new code shows (bf40856b dropped Guide data's Plex source for that time).
- Untested: a phone completing a link, Sign out, the Photos handoff (no Photos plugin on the TV). Receipts: `/data/home/tmp/lumatv-plexqr-20261105/` (not in git).

## Antenna meter, Hide broken pictures, H.264 2026-11-05 (night, main f7226459)

Installed 20:15-20:17 EST from f7226459 (origin and forge main; carries 4d01650b) by the standard path, with Nitin's Plex app in front; Plex was relaunched after. Installed sha256: compositor `84a7091bab9a1d56737ff3b601004cfe021f096011e9a9a57bca179d56bedcd6`, ui `4109df20d27000c59b3bb09ac38b44dfe639fe04448c013e90f70c6816351fc3`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `aedf49c2d2ab38e2599f373a01621c474bb92257dffb991bfbe1fb8a3b30d949`. Failures 0; check-boot's activation line is absent without a reboot. The helper restart cleared `cameras.testEvents`.

- Antenna meter PASS on the Live TV Tuner page: RF 29 Poor, 17.9 dB, MSE 351, 31 packet errors; RF 27 Good, 34.3 dB, MSE 8, 0 errors (RF 18 27.5 dB and RF 32 31.6 dB also Good). The bar, the word and the best-so-far tick follow the picked channel. On RF 29 the BER reads 16777215, the demodulator's saturated raw value, not a rate.
- Live TV preemption FAIL, twice: with the meter on RF 29, launching LG's Live TV makes the meter yield ("the TV took the tuner", then "Live TV is in front"), but Live TV stays black: `legacy-broadcast-atsc` disconnects after 275-406 ms and videooutput's MAIN sink is not connected. Live TV launched with the meter stopped plays (VDEC connected).
- 34.4 PASS: H.264 640x480 59.94 with AC-3 (stream type 0x1b), first byte 1.54 s; the helper logs "34.4's picture is not MPEG-2: it streams without slates".
- Hide broken pictures stays off by default. Plex's Live TV on 16.1 (RF 29), 60 s each, off/on/off/on: off gave 52 HLS segments, first at 8.5-8.6 s, both times; on gave 0 both times: Plex's grabber left the stream 6.8-8 s after the tune and its transcoder failed to start the session. A direct 15 s stream of 16.1 with it on began at 4.68 s and carried 1.23 Mbit/s.
- 120 Hz gate, 3 cycles, panel 119.9 Hz: home-widgets FAIL 58 dropped, 34 over 20 ms (82996a22: 64 and 38); quick 2 and 2; settings-scroll 18 and 0. First frame 10.7, 12.5, 6.8 ms. Reverting 82996a22's commit hold did not bring home-widgets back.
- Receipts: `/data/home/tmp/lumatv-tunersignal-20261105/` (not in git).

## Tuner page, slate shape, guide fall-through 2026-11-05 (night, main 65976a62)

fc6cf188 went on the set with IdleRegress's install of de53f401 (17:47: compositor `0f805697`, ui `25ec25dd`, sm `65f6ca69`); 65976a62 replaced only the helper at 18:23 by the standard helper start: helper sha256 `f4ffd934e1d6d2141b9e1dc80dd58fbc4cae2c0ea5d8705d33a8e526053b1674` (was `d254cf9b`). Failures 0, settled; check-boot's activation line is absent after a helper-only restart.

- Live TV Tuner page: the channel list (RF heads, reception line with dB, share lost, dropouts, the lineup switch) and *Check every channel* draw; the check ran over all 59 channels in about 3 min from the button. It calls 34.4-34.7 "No signal" at 27 dB: the check listens only to MPEG-2 pictures, so a channel whose picture is not MPEG-2 reads as no signal.
- Slates through Plex's API on channels not seen before: 5.1 (720p60) and 2.2 (480p) open with the tuning slate in their own size, 2.1 (seen) too; no "Error reinitializing filters", Error 16 or "Invalid frame dimensions" in Plex's log (two of those errors at 16:39 and 16:49, 2.1 and 16.7 first tunes). Grabs and client sessions stopped and deleted.
- Guide data with Plex picked first and not set up: the Plex line says it is skipped, the free listings filled 53 of 59. While that fetch ran (25 s) the status showed the last words published, which 65976a62 fixes (covered by the plugin's test; not seen again on the set, as Plex was set up through the API at 18:19 by someone else and left so).
- Receipts: `/data/home/tmp/lumatv-tunerpolish-20261105/` (not in git).

## TV session 2026-11-05 (evening, main 55598bc0)

Installed 16:39 EST from 55598bc0 (origin and forge main; d3d50812 above it is a research note) by the standard path; only the helper changed, compositor, ui and sm are the 7e8b8f43 bytes. Installed sha256: compositor `6c5ae2575734d606e0c09966967c2aa539658ab4abe35ab866111431d063a0fe`, ui `0db85b2b8c0f5acde8439a60c56349fb2627fb4f00b2b373589129a1d4809413`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `9b5c486b54c1df8ce8dbcb35075dda310810fe679b2e73a8bd57f8324d185244`. Failures 0, settled; the set ended in Always Ready, off, with the Live TV Tuner and Guide data plugins on.

- Slates through Plex's API (one client): 2.1 shows the tuning slate, then the picture, in 2.0 to 3.3 s. 16.3, 16.7 and 16.8 show the tuning slate, then the weak-signal slate with its wait and dB (16.7's picture came after 32.7 s, 16.3 and 16.8 had none in 35 s). 16.9 shows tuning, weak signal, then "No signal" at 45 s and ends cleanly; Plex asks again and gets a new tuning slate. Plex's log has no Error 16 and no "Invalid frame dimensions" in these runs. Where the slate and the picture differ in size or format (2.1 on its first tune after an install, a 1080 slate to a 720p picture; 16.7's picture), Plex's transcoder logs "Error reinitializing filters" and restarts its session at that point; the client stream goes on.
- RF 29's channels are not stable in LG's own list: it flips between 16-1..16-11 and 66-1 (WYGA-CD) and 29-1..29-12 with empty names (ids `1_29_29_N_0_0_0`). The list read 29.x at 16:39, so Plex got "no such channel" for 16.x, and 16.x again at 16:45 after a tune of 29.1. LG's channel database log (`/var/db/main`, copied read-only) shows 4 flips to 29.x and 4 back since 02:41; four RF 29 tunes at 16:47 to 16:51 (16.3, 16.7, 16.8, 16.9) wrote nothing. Nothing writes the list but LG; whether the tuner's SCAN pipeline or LG's own background tuning triggers a flip is not shown.
- Settings › Live TV Tuner shows the messages switch but not the channel list or "Check every channel": `settings/pages/tuner.rs` lists only the `tuner` and `addresses` sections, so the helper's `channels` section has no page. Not run.
- Guide data on through Settings (zip 30311). Plex is not configured: there is no Plex connection on the TV (no Photos plugin) and the plugin fetches nothing while Plex is picked without an address, so the sources are the free listings only. Status "53 of 59 channels have online listings"; `xmltv.xml` has 10922 programmes over 7 days (107 before). The LumaTV Live TV guide and banner were not opened (the Guide key does nothing on Home and Live TV would take the tuner).
- 120 Hz gate, 3 cycles, pop-ups off: home-idle and home-dpad pass (0 dropped, 0 over 20 ms). quick 1 dropped and 1 over 20 ms, home-widgets 1 dropped and 1 over 20 ms (7 with the still screens), settings-scroll 1 dropped and 0 over 20 ms. By eye, Settings opened from the quick panel goes from the panel alone to Settings alone with no frame of both (captures 0.25 s apart), and Home's dock goes from luma 81.9 to 71.3 in one step between 0.09 and 0.34 s after the key that moves focus off it.
- Idle Home, 60 s, 120 Hz: 18.04 % (compositor 9.5, UI 8.54) after a shell restart and 14.44 % in a repeat, against 7.92 %; 36.3 and 32.2 flips a second against 23.5. The cause is not found.
- Receipts: `/data/home/tmp/lumatv-tv-20261105i/` (not in git).

## Plex Live TV 2026-11-05 (a559ccb3 installed)

`base.livetv` switched on at 11:08 EST through the helper's settings API, the write Settings' "Share the antenna tuner" makes; the helper is still the installed `3c4259b902888c2ea2d30e7ba44e4e62f6ee0f35bd747b8fc7549540b7e0b4b4`. From BeastNAS, `discover.json` (DeviceID `77CF7C46`) and `lineup.json` (57 channels) answer on the TV's port 5004.

- Plex 1.43.5 on BeastNAS (Plex Pass), set up through its local API: device 4 `device://tv.plex.grabbers.hdhomerun/77CF7C46` with Transcoder Quality "Original format" (`transcodeProfile=none`; the facade sends the broadcast as is); DVR 5 (`b1a08178-995f-496b-813c-8afaa223ab77`) on "Local Broadcast Listings (156 channels)" for 30311. Plex's channel map matched 53 of the 57 channels; those are enabled and all have listings for the next 24 h. 35.2, 35.3, 36.4 and 66.1 have no guide match and are off. Undo: `DELETE /livetv/dvrs/5`, then `DELETE /media/grabbers/devices/4`. Device 1 (a DIRECTV tuner, dead since 2025-10-13) predates this and is untouched.
- Stream: Plex's tune of 2.4 answered in 4.6 s and streamed 82 s (11:19:01 to 11:20:22). 2.50 MB left the TV in 10 s; Plex cut one HLS segment a second, MPEG-2 720x480 29.97 and AC-3 stereo at 2.4 Mb/s. Cancelling the grab ended it. After it: Active, Home in front, no `tv` pipeline, last channel `1_32_2_4_0_0_0` (its value before the probes), the lineup unchanged. The stream was of the last channel itself, so the restore to another channel is still untested.
- Plex's transcoder could not create a session, so no Live TV could start: `/tmp/Transcode` on BeastNAS was root-owned (the same error since at least 2026-10-25). Duplicacy's `init.sh` runs `chown -R 0:0 /cache`, its `/cache` is the host's `/tmp`, and it starts 31 s after Plex makes the folder. Given back with `chown -R 99:100 /tmp/Transcode` at 11:18; Duplicacy takes it again at its next start until its cache is a folder of its own.
- Untested: watching in a Plex app (the test drove Plex's API; the server's remote address answers), a recording, a stream with the TV in standby or Live TV open. Receipts: `/data/home/tmp/lumatv-tv-20261105-plex/` (not in git).

## TV session 2026-11-05 (afternoon, main 429039de)

Installed 13:49 EST from 429039de (origin and forge main; b902b4bc above it is docs only) by the standard path, the panel already woken. Installed sha256: compositor `198db06a66805db75482f9f018f53bf21dac0d3204fefb1ccdf634b3c7f4e09a`, ui `e956df12a54ec2b002bb713dc1092b333676853b9992cda6750ed5f77c2a78c6`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `5a1a58b341a6820241e3f33284b61deae456e843bb4225296da8b8889e1da65f`. Failures 0, settled. Before it, a helper-only install of f0889a3d (helper `55d5d3fd...`, 13:21) carried 3176af1b.

- Tuner takeover, one client (this host) on /auto/v2.1, then v5.1 while it streamed, then v2.4 right after closing v5.1: all 200, no 503. `streams X after N s` read 3.3, 3.4 and 3.5 s on f0889a3d and 3.4, 3.2 and 3.4 s on 429039de; the 2.1 stream ended the moment 5.1 was asked (10.4 MB in 8 s). Plex's DVR tuned 2.4 and changed to 2.5 on both builds (`streams` after 3.2 and 3.4 s, then 3.0 and 3.4 s) with no "all tuners in use". Device 4 stayed alive.
- Migration: the log says `base.livetv carried over to the Live TV Tuner plugin, on`; `plugins/enabled.json` lists `live-tv-tuner` under `firstParty`; discover.json, lineup.json (57), lineup.m3u and xmltv.xml answer. Settings › Inputs & devices › Devices › Live TV Tuner shows the switch on, status, name, answered networks and the three addresses. Its Always Ready row appears only with Always Ready off, so it is not on screen.
- Seen on the set: a toast is one compact pill at the bottom centre and rises from the foot; the Themes gallery previews on focus, OK applies and Back returns; a category switch draws about 57 frames; the Smart home card holds 48 items (2 cameras and 46 things); the weather card and view show units and hour labels from Home Assistant's Pirate Weather. Idle Home: LumaTV's saver took the screen at 2 min 40 s, before Home's own 5 min dim, and the panel ramp ran 75 to 64 over the next 5 min. The volume bar's format glyph needs a volume key, which writes to the receiver, so it was not tried.
- `check-boot --now` says FAIL on "worker activated the compositor" as on the last install (a manual install runs no boot worker). Theme, tokens, layout and profiles are byte-identical to before. The set was left in Always Ready, off. Receipts: `/data/home/tmp/lumatv-tv-20261105f/` (not in git).

## TV session 2026-11-05 (midday, main 543ba636)

Installed 12:41 EST from 543ba636 (origin and forge main; 4ba6a327's no-flip change and the Plex handoff) by the standard path, with the panel woken first (Nitin had powered the set off by remote at 12:16; Active Standby). Installed sha256: compositor `d8c70cf7a937ed760a3e81e050036369f72fc0f2ae4a0c54bb3f36758b4f3955`, ui `9a60717e253576a06aff5422ff87e1a8d034eb05db4a369315e891475f6afdb0`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `04ce2af6dd1c28b41c53267893c25baeb86419081169b232af2758ce53603246` (a559ccb3's was `3c4259b9...`; the helper source did not change, so the build is not byte-reproducible), feeder `0f99db547431a44ba558b76d32d63d018d6c299c4d671874e9f7f6710bdb1904`. Failures 0, settled. `base.livetv` stayed on; `discover.json` and `lineup.json` (57 channels) answer on port 5004 with the panel on and in standby. No Plex stream was active at install or after.

- 120 Hz gate, 3 cycles, pop-ups off, panel 119.9 Hz (session 10 in brackets): home-idle PASS; home-dpad PASS, 0 dropped, 0 over 20 ms (1); quick PASS, 0 and 0 (1 and 2); settings-scroll FAIL, 8 dropped and 1 over 20 ms (3 and 2); home-widgets FAIL, 5 dropped (0 in the trace) and 2 over 20 ms (0 and 6). First frame 8.0 ms on home-dpad, 11.2 quick, 15.0 settings-scroll, 9.2 home-widgets.
- `LUMATV_KEEP_UNCHANGED=0` on settings-scroll and home-widgets (a line in `surface-manager.env`, one cutover disable/enable): worse, 19 dropped and 30 over 20 ms, and 10 and 7. The file and the compositor were put back as found.
- Idle Home, 60 s, 120 Hz: compositor 5.69 % and UI 2.23 % of a core, 7.92 % together (session 10: 7.84 %), 23.5 flips a second, 38.1 ticks, GPU 12.4 % busy.
- Home screenshot after the run shows nothing wrong. The set was powered off again at the end, as found. Receipts: `/data/home/tmp/lumatv-tv-20261105e/` (not in git).

## TV session 2026-11-05 (idle and 120 Hz gate, main a559ccb3)

Installed 10:02 EST from a559ccb3 (origin and forge main), panel on at Home, Nitin's profile, `base.livetv` untouched (off). Installed sha256: compositor `de6fdce02524bad5df123634c4874466c828c66fa9c83cf52c61c3679063c9ed`, ui `6b0b0be04de5564817cc467914ae9fa477679f7bbfc80d62dcbb6c0e5af9df89`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `3c4259b902888c2ea2d30e7ba44e4e62f6ee0f35bd747b8fc7549540b7e0b4b4`, feeder `0f99db547431a44ba558b76d32d63d018d6c299c4d671874e9f7f6710bdb1904` (staged by the install). Failures 0, settled.

- Idle Home, 60 s, 120 Hz: compositor 5.34 % and UI 2.50 % of a core (7.84 % together, target 8; 7.17 + 4.36 = 11.53 % before), 22.8 flips a second, 36.8 ticks a second (1.6 per flip; 5.1 before), 0 flips at every refresh, GPU 11.9 % busy. The weather reading was not available to the probe, so its flips and animating checks read "?".
- 120 Hz gate, 5 surfaces, 3 cycles: home-idle PASS; home-dpad FAIL (1 burst over 20 ms), quick FAIL (1 dropped, 2 over 20 ms), settings-scroll FAIL (3 dropped, 2 over 20 ms), home-widgets FAIL (0 dropped, 6 over 20 ms). Before: settings-scroll 10 dropped, home-widgets 1, quick failing, home-dpad passing.
- The first gate start was refused: a 3 s panel reading of 178.7 Hz; a re-reading gave 120.3 Hz and the gate ran.
- Home screenshot after the run shows nothing wrong. Receipts: `/data/home/tmp/lumatv-tv-20261105d/` (not in git).

## TV session 2026-11-05 (after midnight, main 1fdaab34)

Installed 23:57 EST from 1fdaab34 (on origin and forge main), panel on at Home, Nitin's profile, through `build-tv.sh`, `tv-first-light.sh deploy`, cutover disable/install/probe/enable, `tools/lumatv`, the launcher files and the helper. Installed sha256: compositor `3ab32dc1c4aeec4af3893258739f170192e04a81829277bf74d1586be6f0eb29`, ui `d22be363708cb134fc643c08497576c1648d3c20c1def2d77fb07b4a8856566a`, sm `65f6ca69eb6da66cbfb139817803d0c1e01b03931fe373894b123c3f64df13da`, helper `9b76a93fdc8df4f9e24ea071508182f52362fb552a687b9db48dddaf873d1eed`, tools/lumatv `56899321dcf6b4f7583ab015a0182a61b5a9d45dc70c3e63187a3dc1d9c347b4`; native stub program `f2820100e25ae3e93049647e339b5b8c4baa81403ce31c5232e56e5db5a8809e`, copied to the set but not kept. Receipts: `/data/home/tmp/lumatv-tv-20261104f/`.

Checks:
- Install health PASS: live, mounted, persistent, failures 0, settled; `check-boot.py --now` PASS.
- Native Home stub FAIL (16 PASS, 1 FAIL, 1 not run, 32 s): RSS 52 kB and PSS 40 kB against 2 MB (web stub 32.4 MB and 15.3 MB); swap, SAM type, one process, mapped card, Home in front, three launches, preload, the Home key over HDMI 2, the saver and its end, and no Flutter Home all PASS. `launch-over-app` FAILED (a luna-send launch of Home did not bring Home in front within 6 s over HDMI 2). The wake was not run: LG's panel refresh was due. The web stub is back; boot direct stays on (kind web).
- Retime FAIL (visible). Nitin watched the `restore --clip25` run at 00:10:30: the colour changed, the screen flickered, the colour changed again, then Filmmaker returned. Picture settings polled at about 12 Hz across it: `colorGamut` stayed `auto` and `localDimming` `low` throughout; `pictureMode` went `filmMaker` to `game` twice (the 25 fps claim, then the retime clip; `dynamicRange` `sdr` to `sdrALLM`), with colorTemperature -50 to 0, gamma high2 to medium, backlight 75 to 90, color 50 to 55, and back. A whole-screen clip and a 256x144 clip write the same set of picture rows. The probe's own `pq_no_pig` verdicts do not see this, and its positive control (`detector_works`) came out false. The retime is off.
- Retime rate: 100 Hz held, then 120 Hz 2.8 to 3.5 s after the release; `play` and `control` kept 120 Hz.
- Front Door (camera 104) full view right after Back Door's: PASS. The claim was granted at 00:12:50, the first picture came after 159 ms (helper) and 176 ms (UI), main stream 1920x2560 at 25 a second in a 810x1080 window at 555,0, 397 pictures in 16.1 s with 0 lost.
- Idle Home, Nitin's profile, saver off, 60 s without a key, sky moving (no sky fix in this build): 119.91 flips a second, UI frames 119.91 a second with 0 empty; compositor 27.8 % and UI 8.3 % of one core, load average 14.7.
- Untested: the native stub's wake, the picture colours of the retime with a different picture mode, the top bar's centring and popup placement, the Smart home card without its heading.

Findings:
- The helper's `lumatv-gfn-feeder --serve` stays up while a camera pop-up plays, and `tv-retime-probe.py play` then answers "not ready"; pop-ups off does not remove one already up.
- Back from a camera's full view returns to Home, not to the Cameras grid.

## TV session 2026-11-05 (overnight, main b1858b27)

Installed 02:42 EST from b1858b27 (on origin and forge main) through `build-tv.sh`, `tv-first-light.sh deploy`, cutover disable/install/probe/enable, `tools/lumatv`, the launcher files and the helper. Installed sha256 (first 8): compositor `df040047`, ui `65b703e4`, sm `65f6ca69`, helper `d5de74e2`, `tools/lumatv` `56899321`. Native stub program built from the tree: `1a9e47e2`. Receipts: `/data/home/tmp/lumatv-tv-20261105a/`. `ctl retime` no longer exists. The TV ends in Active Standby with boot direct on and the web Home stub.

Checks:
- Install health PASS: live, mounted, persistent, failures 0, settled. `check-boot.py --now` ends FAIL on its boot-line criterion alone (no reboot since the boot began at 23:04).
- Native Home stub FAIL twice (17 PASS, 1 FAIL, 1 not run, 33 s): `launch-over-app` (a launch of Home over HDMI 2 is not heard by the compositor, front stays HDMI 2), wake NOT RUN (LG's panel refresh due). RSS 52 kB, PSS 40 kB. The web stub is back; no reboot with native was tried.
- Camera rate PASS on the clips: 25 declared as 30 holds 120 Hz (decoder 30), the unchanged 25 clip goes to 100 Hz and holds it, 30 and 60 fps clips hold 120 Hz (decoder 30 and 60). The real camera (196, 1280x720) was in family in the second run: re-declared, decoder as declared, 120 Hz during and after, 15.1 pictures a second.
- `isGameMode`: picture mode (read-only poll, about 12 Hz) is filmMaker before and after, and `game` for the whole of every clip and of the camera's full view (colour temperature -50 to 0, gamma high2 to medium, backlight 75 to 90, colour 50 to 55). Gamut stayed `auto` and local dimming `low`.
- Weather memory probe PASS with `LATE=1` (the verdict is on the row 10 s after close: UI Pss +0.9 MiB, layer files +0, helper worker +0.9 MiB, compositor RSS below baseline). A run without `LATE` failed on compositor RSS alone (203 MB against 186 MB + 8 MB, 5 s after close); the compositor's baseline moved between 147 and 273 MB across runs. GPU KiB not readable.
- Idle Home, Nitin's profile, 60 s, saver off: compositor 7.17 % and UI 4.36 % of one core (11.5 % together, target 8, FAIL); session 6 baseline 27.8 % and 8.3 %. 24.2 flips a second, panel 119.91 Hz, GPU 13.6 % busy.
- 120 Hz gate, 3 cycles: home-idle and home-dpad PASS, quick, settings-scroll and home-widgets FAIL as in session 5. Dropped: settings-scroll 10 (5: 6), home-widgets 1 (5: 4); burst over 20 ms: quick 2 (3), settings-scroll 11 (12), home-widgets 15 (9); quick burst end 192.9 ms (176.4).
- Prime Video (`amazon`) is in the dock. The dock also lists HDMI 2 twice.
- Standby 03:04, held 16 min, `powerOn` 03:20: Home (LumaTV's) in front, failures 0, `homeDrawn` true. `power/turnOnScreen` answered -102 in Active sub state; the wake did not need it.

Findings (probes on main that do not run on the set as written):
- `tv-camera-rate-probe.py` imports `statistics`, which the set's python lacks; only `mode()` is used.
- `wx-memory-probe.sh` looks for a process named `lumatv-rustui` (it is `lumatv-ui`), reads `--shm` from the command line (the UI gets `LUMATV_UI_SHM` in its environment), calls the compositor binary without its loader, and measures the first `lumatv-helper` process, which is the 220 kB supervisor and not the worker.
- Picking the weather card needs Home's focus on the status bar; after a run the focus stays on the card and the next run opens another card.

Recovery: `lumatv homestub-kind web`, `lumatv boot-direct off`, hold Home 10 s.


## TV session 2026-11-05 (morning, main 29bd61a9)

Installed 04:53 EST from 29bd61a9 (on origin and forge main), by the same steps as the overnight session. Installed sha256 (first 8): compositor `13f13dea`, ui `e3d02d16`, sm `65f6ca69`, helper `03bc584f`, `tools/lumatv` `56899321`, camera feeder `0f99db54` (was `0f720bff`). Receipts: `/data/home/tmp/lumatv-tv-20261105b/`. The web stub stays; the TV ended in Active Standby, boot-direct on, failures 0.

The install steps did not carry the feeder, so the ES pipeline was missing on the set: the first `picture` run played nothing through the ES player (the old feeder prints its usage for `--pipe es`) and still reported the mode kept. The feeder built from main (`gfn/feeder/build.sh`) was staged with `feeder.sh update` into the app payload and `/var/lib/lumatv/bin`; its libraries were left as installed. The feeder is only updated by a package install or the boot-time update, so any install from main needs this step when `gfn/feeder` changed.

Checks, after the feeder update:
- `tv-camera-rate-probe.py picture`: NDL (control) takes `game`; the ES player, with and without low delay, keeps `filmMaker` (about 10 readings a second) and returns to it, display low delay on, 118 to 122 Hz (120 band) while playing.
- `camera`: camera 88 (`--downs 0`), 418 pictures in 15.2 s (27.5 a second), 0 refused, 0 lost, `filmMaker` for every one of 304 readings, 120 Hz during and after. Camera 196 (the default `--downs 2`) sent no picture in time in both attempts, so it proved nothing. Before the feeder update, camera 88 took `game` for the whole view with display low delay on.
- `wx-memory-probe.sh`, as in the repo and without `LATE`: VERDICT PASS (closed UI Pss 29.6 MiB against 31.1 baseline, helper RSS 27.6 MiB against 24.1, compositor RSS 161 MiB against 196).
- Dock: Prime Video (`amazon`) once, HDMI 2 once (`shots/02-home-dock.png`).
- Standby, held 91 s, `powerOn`: LumaTV Home in front, `homeDrawn` true, failures 0.

`tv-camera-rate-probe.py` still imports `statistics`, which the set's python lacks; a one-function shim in `PYTHONPATH` stood in.

## TV session 2026-11-05 (6be749ee installed)

Installed 06:12 EST from 6be749ee (on origin and forge main; includes 8a0bb604's NDL fallback and f82907e6's feeder staging) by the standard path: `tv-first-light.sh deploy`, then `cutover.sh` disable, install (which now updates the feeder), probe, enable, plus tools/lumatv, the launcher files and the helper. Installed sha256 (first 8): compositor `c2ccb7ab`, ui `71f00561`, sm `65f6ca69`, helper `0f9efe04`, camera feeder `0f99db54` (the same bytes as the previous install; the set's copy matches the bundle's `HASHES`), `tools/lumatv` `56899321`. Receipts: `/data/home/tmp/lumatv-tv-20261105c/`. The web stub stays; the TV ended in Active Standby, boot-direct on (marker 2), failures 0.

- Ring (`tv-camera-rate-probe.py ring`, one try): camera 196 played; the bridge answered in 3.5 s and the first picture came 5.7 s after OK (the helper's own count: 2067 ms after the bridge), 811 pictures in 54.2 s (15 a second), 0 refused, 0 lost. Panel 120 Hz during and after, pictureMode `filmMaker` for all 1721 readings and after Back, display low delay on. The helper logged the view ended after Back and did not dial again. Back during "Connecting..." (before the first picture) was not exercised.
- Full view of Back Door (103, 2560x1920) and Front Door (104, 1920x2560): first picture 318 and 237 ms, 21.9 and 20.8 pictures a second, 0 refused, `filmMaker` throughout and after, 120 Hz. Both loads were `transportType 283` container ES (the ES player); no NDL fallback line was logged.
- Camera list popup scrolled to its last row (Downstairs Door Camera, "No picture") and the Smart home card opened and walked: the helper logged no line for camera 196 after the probe's own view.
- Standby, held 91 s, `powerOn`: LumaTV Home in front, `homeDrawn` true, failures 0, then standby again.

`check-boot.py --now` fails its "worker activated the compositor" line after an install without a reboot (no boot worker line yet); the compositor was live, mounted, persistent with 0 failures.

## Current maintenance state (2026-10-31 EDT)

The proposals reuse existing Lucent CSS, Manrope and audio assets. Seven 1920×1080 scale-1 renders were inspected in Cody’s visible shared Chromium: `shots/review-toast-dark.png`, `shots/review-toast-light.png`, `shots/review-audio-atmos-dark.png`, `shots/review-audio-dts-dark.png`, `shots/review-audio-dtsx-bright.png`, `shots/review-audio-dolby-light.png` and `shots/review-audio-stereo-dark.png`. Toast proposal is approximately 320×65 px with 24 px text and one bare check, versus the old 520–760×96 px and 27 px title with a 64 px tile. Audio uses original PNG glyphs left of the number and track in one floating group at the existing bottom position. Light/dark and 200% text showed no overlap or overflow; fonts loaded, browser errors were absent and conservative ink-contrast bounds exceeded 7. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/review-render-proof.json`. These are illustrations, not TV acceptance or behavior changes.


Both keyboard tasks are complete. The compact-toast and audio proposals were approved and implemented in source, with host-native evidence in `/data/home/tmp/lumatv-ui-proof.68u1vel0`; they are not installed or accepted on the TV. The toast defaults to bottom centre with padding and uses existing Settings position controls. Transient LG feed 28737 reaches the toast only, not NoteLog subscription, history or unread count; genuine TV, plugin and AI entries remain. The newest toast replaces the current one and resets its deadline; older toasts do not resurface. Original audio PNG glyphs appear left of the number and bar, with no invented glyph for PCM, unknown formats or mute.

R4 is frozen and locally reviewed/exercised at /data/home/tmp/lumatv-f5-renderer-packet-r4. Entry SHA3e5fd3953c3c67aae80d849b0ab6187f7c3df9300d1336104b0d86614a41cd5b; scope-receipt-r4.json SHAb006de5f91f6f88dfa5cfc06af322ffee5141ffdbfdd4296c6f0e10b16537c37. execute-command.txt SHAc4596739ce95df9eca054f4df94ac85c0f9152536b09a7aa7b461d78be977735 hashes then compiles the same entry bytes. Entry/receipt are regular single-link0400 files. Direct Fix it offline authorized preparation only, not installation or transferred permission. No TV action or request is pending here.

Coordination03:20 EST November1: Marquee lease is RELEASED after reported stable0.10.0 runtimefce9888a deployment,54 matched files and native smoke. Final observed state was Home, Stats hidden, video_planes=0. No fresh refresh/divider read followed the earlier bounded capture;120/divider1 and8.3ms output remain historical evidence only. Zero video planes are not empty-media proof or interruption consent. Preserve subtitle sidecar ABI6 with epoch-tagged readiness publication in later alignment.

The UI-feature artifact /data/home/tmp/lumatv-foreground-pacing-20261031/lumatv-compositor, SHA256904c8e93a985aa25feb07afac8fdde2f0796b67317ac431ef86277e4edbcb6c0, is SUPERSEDED/NOT READY and ineligible for installation. It is not an authorized substitution for pinned legacy f5/R4. Toast/audio artifacts remain separate and uninstalled. Native callbacks and LG video-family ownership remain unchanged: divider1 follows100/119.88/120, not independent120-over100. No panel, Home-global120 or optical-seamlessness claim follows.

The existing final f5 replacement already corrects the old904 review findings. Independent narrow review of foreground-pacing-source-20261031-final.tgz confirms webos_shell.rs:114–130 recomputes app/property/window-type changes and :80–86,100–110 recomputes changed nongroup card state, including minimized→Maximized/Default. Archived tests/refresh.rs:229–260 explicitly covers these transitions. Existing legacy-tests.log records9 refresh,11 callback and2 control/capture tests passing; cross-build-legacy.log records a successful release build. These are historical records, not new runs.

Fresh SHA256SUMS-legacy verification passes for existing lumatv-compositor-legacy f5c864ae1e869da3e2f60af14faca77ef51664254c8043652dff84a2fbf3463a, the same hash pinned at frozen R4 scope-receipt-r4.json:18–20. No second implementation, rebuild, artifact substitution or packet edit is needed for these findings. Archived source/build records are an association, not independent reproducible source-to-binary proof. R4 remains separately unapproved and not hardware-accepted. Native Qt host gating supports no production shell change; it does not establish TV GPU-slot causality or video-wallpaper/all-config behavior.

Native120Pacing produced no analysis: its latest wake failed with HTTP400 before an answer, so no slow analysis is running. It remains the sole authorized deep-task; no replacement worker or role change was made. The Marquee caller reports no pending HomeLabber request; that does not transfer unused R3 permission or authorize R4/retained-HDMI interruption. Any future TV access requires fresh joint coordination and exact authorization. Current RESUME/HANDOFF status supersedes archived coordination prose; preserved source/binary archives and frozen packets remain unchanged.

The current source menu-pacing map and graphics evidence index are recorded in `docs/research/framerate-result.md`, Current foreground pacing policy. Ordinary overlays inherit the underlying eligible card, so foreground Marquee gets nominal120 while Home/system overlays retain startup base60; selected Qt animations are frame-driven but not universally120. The existing local refresh protocol smoke passed all9 cases. This does not establish current TV mode, unique120 pictures, global Home performance or direct-scanout device acceptance.

Local material/theme extension is separate from frozen R4 and all old artifacts: preserve Clear/Frosted, derive Liquid from the actual native Marquee topbar and Glazed from LumaShow native card/sheet rendering, add independent Strength beside Depth, and add20 light/20 dark palettes. The native Themes gallery belongs under Home & look. Nitin selected immediate focus preview, OK confirmation and Back restoration of the last confirmed theme. No Echo Show screenshot/device access is needed: Nitin replaced that request with a source audit. Shared Lucent adoption remains conditional on Nitin approving actual LumaTV renders; no shared-language edit is authorized yet.

The gallery uses the effective native text scale for its column count, not theme JSON alone. Theme thumbnails composite each palette’s translucent roles over its own canvas and sheet. Catalog text roles, including Ink3 headings, target7:1 on those resolved surfaces; arbitrary video still needs the existing reading fallback. Protocol14 adds outward edge-lens and tint-gradient fields, so the client and renderer must be built as a matching pair; no new UI binary substitutes for a frozen protocol/payload packet. Native Mesa captures are local rendering evidence, not TV timing or hardware acceptance.

Native startup uses /usr/bin/surface-manager -platform eglfs_webos with captured inherited LD settings. R4 requires both native-alias and installed-supervisor executable identity, the guarded private loader, all-LD/duplicate checks and a fresh stable ARM ELF32 shared-object header/hash proof for libsystrim. Pinned ELF64 loader was exercised locally in existing qemu: explicit private library path overrides LD_LIBRARY_PATH and an ELF32 preload is ignored. Actual TV preload/proc evidence remains unobserved; header/class compatibility is not vendor provenance, full supervisor trust or complete loader isolation.

The exact saved Home media profile has four retained HDMI2 rows and six occupied input/audio resources; it is not empty or proven idle. R4 rejects app/foreground/unknown/malformed state and full-record drift before disable; postrestart IDs may change within the same permitted kind. Future scope must explicitly allow possible background input/audio interruption and a fresh exclusive lease; snapshot checks are not atomic stream exclusion. Actual inspect and capture-based checks passed with sockets forbidden. Independent startup/media reviews found no remaining narrow blocker after full-header/inode corrections; guarded settlement/rollback transitions were exercised with explicit local stand-ins, not hardware. Renderer/R8/payloads are unchanged, and f5 remains NOT installed.

Picture-mode selection sends settings.pictureMode through LG setSystemSettings without a dimension selector, matching the recorded r19w immediate-change path. Dimensioned writes remain appropriate for per-mode controls, not mode selection. The generic compositor/native Luna bridge forwards that payload unchanged; it does not require or inject a dimension. The shared Sio writer discards stale context and refreshes remembered values/descriptions after acknowledgement or the bounded two-second guard. Dependent controls require fresh native mode, input and property availability.

Quick Settings and Picture use LG's visible/active choices; the legacy fallback is restricted to confirmed SDR with an absent descriptor. Popup selections retain their displayed native value through descriptor reordering. The shared borrowed caption formatter retains existing names and LG's proven dolbyHdrCinemaBright → Cinema Home; conditional Dolby captions are not guessed. Source is verified on the actual Rust/Mesa host, not installed or included in frozen R4.

The current saver source is rustui/ui/src/system/saver, with system/layer.rs and shell.rs owning presentation/input dispatch. The start deadline is LumaTV's screensaver.afterMin (5 min by default): the compositor runs LG's screen-saver nop timer at it, and LG's option.screenSaverTimer (3/10/20/30 min) is no longer shown or written. Native Screen Saver state is approved by LG's idle/media policy; a tvpower vote question alone is not approval. Home-owned negative voting stays Home-only; an app saver must wait for LG-confirmed Screen Saver so native playback/game vetoes remain authoritative.

LG's idle settings as read on the set on 2026-11-09, in Active Standby (/data/home/tmp/lumatv-idletimers-20261109/lg-idle-settings.json): option.screenSaverTimer 3 (checkChangedScreenSaverTimerByUser default), general.screenSaverAd off, time.automaticStandBy 0, time.noOperationAutoOff off, time.autoOff15Min on, time.autoOff2HourOnTimer on. The idle-timer change writes none of them; after its install each reads the same.

Native UI checks reuse tests/rust_settings_host.py and tests/rust_home_host.py with the real lumatv-ui binary and Mesa. Helper screen-state replay proves UI behavior, not elapsed hardware timeout or individual app vetoes. The current ARM compile commands are compositor/tools/build-tv.sh lines26 and33; the full script also rebuilds deploy staging, while rustui/tools/build-tv.sh targets the older lumatv-rustui client. New UI/platform changes remain separate from the immutable R4 packet.

Current UI corrections pass 1094 affected UI tests (two ignored; 162 unchanged Home tests filtered), all 406 compositor library tests, and both production ARM builds. Native-host evidence covers SDR/Dolby arrows, explicit mode selection, confirmed readback on reopening and app-native saver key/pointer wake. Real mapped-app regressions cover wake isolation and pre-existing held-input ownership. Proof is retained at /data/home/tmp/lumatv-ui-proof.68u1vel0; private native-ui-fixes source/ARM archives supplement the resume handoff, not an installer or publication. Actual-TV movie mode switching, elapsed idle timeout and playback/game veto acceptance remain outstanding.

Parent local trace report: /tmp/marquee-readonly-20261101-{sched.gz,ring.json,capture.log}, monotonic18658.566782..18664.569939,374216 events,0 overrun; 718 draws/717 flips and two16.7ms intervals amid8.3ms output. late/lost/missed counters were0, so those counters do not establish zero stutter. Main-thread runnable/blocked timing is correlation, not sole cause; overlapping Mali slot events invalidate naive summed GPU attribution. No app/callback/governor/panel change follows from this sample.

The preserved source has no previously successful standalone renderer-only host CLI. Proven boundary is renderer_cutover(ctx) with the R8 transport: /tmp/lumatv-historical-stage.TyUJ9M4D/renderer-fragment.py:89-116; wake-maintenance-evidence/events.jsonl:23-54, attempt-923dafccaf6845f8927a9f5b434d39d2. Three commands exited0; two health records and renderer-cutover=healthy identify installed compositor6fe8bae..., not f5c864. Historical staging was deleted at event55. wake-maintenance.py:1328-1348 offers no renderer-only mode, and execute():971 onward continues into tools/wake/trial/coldboot; its enclosing entry must not be replayed for renderer-only work.

Historical commands below are provenance, not current execution instructions; their temporary stage was removed:
```sh
LUMATV_D=/tmp/lumatv-renderer-XXCdjMIc/renderer sh /tmp/lumatv-renderer-XXCdjMIc/renderer/cutover.sh disable
LUMATV_D=/tmp/lumatv-renderer-XXCdjMIc/renderer sh /tmp/lumatv-renderer-XXCdjMIc/renderer/cutover.sh install
/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv compositor on
```

R2 request `179715b6-e14a-4347-af5f-08b3ccfbcb0b` was APPROVED then FAILED before mutation, change `fb6f79b1-4d44-46d0-8e1b-637f70652148`, exit1 after1.7s. Local attempt-1606d622abce49a5b01f15b43c2b209c/events.jsonl contains only four read-only file/hash checks: brand matched; tools/lumatv76310050 differed from expected d22a3ced. No upload/disable/restart occurred. The original permission is consumed, not denied.

At 01:27 EDT November1 the parent explicitly released its TV lease after reporting native Marquee37d8d0f4 deployment, 54 matching payload hashes, successful rk18700 dialogue-enhanced Opus2-channel playback and seek-preview cancellation, followed by video[]. IPK SHA c9e95defb396b62062fd7405551606c135d4da318a694fb28ddfe420162b7812. Temporary UI120/tick8.333333 is parent-reported; automatic pacing and zero-stutter acceptance are not claimed. Lease release is not permission for platform execution. No TV calls or new approvals occurred here; frozen packets remain unchanged.

Foreground Marquee f5c864 remains NOT installed or TV-accepted. Parent’s ONE corrected-request permission remains UNSENT/UNUSED, not standing or transferable. No prompt or TV action during human use. This token retains no-new-requests; consumed R2 cannot be replayed. Nitin subsequently selected Fix it offline here, permitting a separate narrowly corrected R4 candidate only; it does not authorize R4 execution or transfer the parent’s request permission. R2/R3, receipts, payloads and the historical renderer/R8 functions remain frozen.

R2 remains frozen for audit, not retry: `/data/home/tmp/lumatv-f5-renderer-packet/renderer-entry-r2.py`, SHA256 `7fbb2ed738bb1a16e772240193bd49caf6440751569edecf8f5a21515d25f5c7`. Separate R3 is `/data/home/tmp/lumatv-f5-renderer-packet-r3/renderer-entry-r3.py`, SHA256 `e3f1642c10f513f53e0e05406af2cb3b8dad41b82a2c2578b18e87de2f42e666`; its frozen scope-receipt-r3.json SHA256 is `974eae34150180a0ddd01e2372f4b9d856d5b297b1246a67b12ad760ad3b2164`. R3 changes only the exact tools/lumatv pin d22a3ced→76310050. Payloads, guards, dependencies, callbacks and rollback behavior are unchanged; no additional accepted hashes.

Final reviewer `review-recognized-dispatch-763100` found no blocker in the exact e3f1642c entry’s single-pin diff, independent payload copies, reference and provenance. Actual local --inspect returned ok:true. This is a local correction review, not a full hardware/security review or execution approval. The frozen receipt retains its earlier review-requested wording; this handoff records the delivered final verdict without changing that receipt.

Dispatcher7631005064da542f26f644ae1b636597e729884c4288d996c027b60185f366cf matches the installed observation, preserved source and accepted wrapper-package.ipk SHA8b1aba0a629fab7413285249bf2768712f1e9e886471cd05d32d2641e5f55f1c, corroborated by `/data/home/tmp/lumatv-wake-install.ms6eh5iv/wrapper-package-verification.json`. Relevant compositor status/on initialization and transitive calls are unchanged; differences concern wake/boot paths. Pin the accepted installed artifact, not a version label or temporary trial payload: d22 was an earlier trial that failed and rolled back. The other five reference dependencies match the accepted package, but their live checks were not reached; full tree/startup guards remain execution-time requirements. Unobserved extra files remain refusals, not new accepted states.

Staging both archives plus extraction needs 222553876 bytes (212.24 MiB). Initial free `/tmp` must be at least 384 MiB; post-stage `/tmp` and `/var` require 128 MiB. Known temporary 60/120 OSD state is accepted before cutover with unchanged commit callbacks and canonical startup; candidate and rollback must settle Home at 60, with candidate auto/default60. Unrelated tree/flags/process/dispatch drift or unreadable state vetoes rollback rather than overwriting an uncertain state.

Existing display recovery restarts SM/WAM/IME/booster, returns Home, may close stale input/Netflix preload, and clears all temporary librdx reports, including reports arriving after the last check. Pre-existing regular reports are copied off-device and checked before disable; they are not restored to the TV. These effects belong in the owning Face ID plan. No standby/reboot, native-app/storage, panel/fr_rate, native video-path or wake-policy writes are included.

Marquee owns a separately reported Home hero art/text swap hitch; `FixHomeBannerHitch` owns its local investigation. The hidden-shell host proof neither diagnosed nor fixed it. Separate native-app authorization is not compositor/platform approval. Source and evidence checkpoints predate these coordination-only notes; compositor artifact `f5c864` remains the final built, reviewed artifact and is not installed.


Final Bazarr connection showed `https://bazarr.nateshome.net`, the saved key masked, Connected, Bazarr 1.6.2, Sonarr 4.0.20.3014 and Radarr 6.4.4.10685. No API key was read, logged or changed. Evidence includes `keyboard-bazarr-final-connection.png`, `keyboard-address-restored.png`, `keyboard-bazarr-key-two-bullets.png`, `keyboard-bazarr-key-one-bullet.png`, `keyboard-bazarr-key-reopened-empty.png`, `keyboard-lg-search-two-g.png`, `keyboard-lg-search-one-g.png` and sanitized key JSON files.

For Marquee, virtual Back hides the keyboard but leaves the edit active; app Back cancels it. Require IME null before reopening with OK, which otherwise commits the draft. API-key drafts reopen empty: a masked two-character draft, one Backspace, virtual Back and app Back reopened empty; empty Done does not write. Nonempty Done persists: never test it on changed real credentials. Normal keyboard navigation uses existing `ctl remote CODE tap`; `ctl key` targets the app and bypasses IME. Actual RIGHT moved focus 0→2 and g→h. LG Search ignored app Back/Home for closing its overlay; existing `closeByAppId` for `com.webos.app.voice` closed it. `own_hide` already emits input_panel_state 0 and intentionally leaves the field active; SDL dismissal is not an established defect. Parse control JSON stdout separately from the known wrong-ELF warning on stderr; never dump raw shell, IME surrounding text or API keys.


Wake root task is complete: activation `c413b8707ca44e47ac788a3296573221` passed cold Home invocation `b370dc321fda4bff9227a16b2d043d82`; the two-minute healthy window cleared naturally, the existing wake commit succeeded with enabled true and pending null, and postcommit cold invocation `6ecd5f05acbb47898f067be9aafbe2e2` also passed Home without a manual postboot Home launch. Six warm cases passed: `warm-home-wol`, `warm-livetv-wol`, `warm-hdmi2-wol`, `warm-home-remotekey`, `warm-livetv-remotekey`, and `warm-plex-wol` (Marquee, native legacy ID `com.beb.plxnative`). Each used real native standby for 25 seconds, fresh HA AVR-off evidence and isolated WOL or software powerOn with reason remoteKey; native bootd/LumaResumeApp, actual foreground and DISPLAY, closed CoverUI, persistent/live/settled compositor with zero failures, external_arc and fresh HA AVR-on/input were observed. Software remoteKey is not physical-button proof; the LiveTV screenshot was black and does not prove visible channel content. Evidence and screenshots: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`. Source publication remains on hold.

Keyboard acceptance passed on the actual TV without source changes or a build. Existing `lumatv keyboard luma` enabled the persistent `/var/lib/lumatv/.keyboard-luma` flag; Rust keyboard capability was already active. Marquee (`com.beb.plxnative`, its legacy ID) Bazarr address and API-key fields passed typing, Backspace, hide, cancel, reopen and Done; LG Search (`com.webos.app.voice`, `launchMode:vkb`) independently passed typing and Backspace. This is not an FPS or latency benchmark. Final `keyboard-final-state.json` and `keyboard-final-home.png` show TV Active, native Home, IME null and keyboard luma; own counts activations 7, shown 7, texts 6, keys 6, nav 110, failed 0. The compositor was persistent, live, mounted and settled with zero failures; wake enabled/pending null, external_arc and fresh Denon on TV input. TV and AVR were left on. Evidence: `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`.

Separate CEC tests: sole HA `remote.turn_on` did not wake; sole Apple TV `remote.send_command(home)` woke Home standby to HDMI2 with actual native `hdmiCec` and AVR AppleTV/MPLAY on, without TV powerOn or a Home override. Native receiver MPLAY is the configured Apple TV, corroborated by fresh HA Apple TV state and prior r10/r11 logs; retain the original false raw/display-name comparison and `receiver-source-corroboration.json`. The direct CoverUI fallback chain passed from LiveTV: firstApp `{}`, secondApp `{}`, fallback absent ghost ID and finalFallback Home resulted in closed status and actual Home/DISPLAY; inventory confirmed ghost absent and Marquee (`com.beb.plxnative`) present. This does not prove a missing-saved-app wake. Physical Netflix Hotkey/OTT wake remains unaccepted: the historical Netflix log was copied from a prior boot and its stimulus is unknown, with no proven safe software equivalent. Native input/Hotkey unit contracts were reviewed, not accepted as hardware-button tests. Never write native intent files.


At 17:29 EDT, opening live tuner settings reportedly crashed Rust Home and left the TV on the old compositor. The trigger and resulting screen are owner-reported; the fault is not diagnosed. Rust Home was subsequently restored by the renderer installation below. Full stock-screen migration and the tuner-settings crash remain separate deferred backlog items.

Audio, bounded RAM/syslog logging and the custom keyboard were installed at 14:36 EDT. Wallpaper controls passed physical-TV acceptance at 16:24. The approved wake-maintenance batch subsequently installed compositor `6fe8baeae18aa1aa039a6fc6103cc1559214d44bfd55db378c31d27db7db7052`; it was live, persistent and settled before the tools-copy failure. UI `d4313791`, supervisor `3635ed0c`, helper `c70c9e6f` and capture `00999f22` are unchanged. The later accepted wake state is recorded above. Source publication remains on hold.

Specular 0.10→0.15 and Bloom 0.65→0.70 independently changed the frozen wallpaper picture. Each restoration returned the wallpaper-only crop `(60,150,1250,1000)` pixel-exactly to its baseline. The complete control values, including Motion On, were restored and the TV returned to unobstructed Home with eARC selected. Speed changed/restored through the TV controls earlier; speed-ratio rendering is host-tested, not a physical-TV timing measurement. The compositor is live, persistent and settled with zero failures.

The renderer previously discarded lighting controls at zero speed. It now retains those controls with motion disabled. A separate glyph-free-scene damage omission is also corrected; it is not established as the cause on Home. The actual compositor-output regression failed before and passed after; production Home wallpaper and Silk-report integration tests passed. Renderer unit tests remain unbuildable in the synchronized tree because its test-only upstream files are omitted.

Final wallpaper evidence: `/tmp/lumatv-wallpaper-fix-tkil2ncd/evidence-8dd46fb2/results.jsonl` and `/tmp/lumatv-historical-stage.TyUJ9M4D/ui-evidence-20261031-162401-ab4c9a31/`. The frozen batch `install-and-check.py` has SHA-256 `a7ce0ec42114aae845ac24e8438d10ea9cf174576ec539effb4a2e262773dbdc`; bundle `b9417dad810837e7811b912321f4aec839a2f9027af41bedb940c2a5dd15ebf1` differs from the previous bundle only in `bin/lumatv-compositor`. The successful verification run skipped installation. Grant `4dd9e715-6d4e-4ef7-a14b-95db671b8ce2` has one use remaining until 03:20 EST on November 1. Uploaded archive and individual captures were removed from TV RAM. Final free space was 881084 KiB in `/tmp` and 617968 KiB in `/var`; reports and images stay on Cody.

Three TV-only standby/wake cycles passed: Denon followed off/on without direct AVR power commands. Selecting Apple TV on the TV also selected Apple TV on the AVR after wake. HA timestamps are observations, not hardware latency measurements; three cycles do not establish perfect reliability. Evidence: deployment `power-hdmi-evidence.json`.

Physical Denon recovery passed with SIMPLINK disabled: the helper powered the AVR on and returned to eARC after a helper restart. The loopback unavailable-receiver case attempted wake, fell back to TV speakers while retaining saved eARC, stayed on speakers while the fixture remained off, and recovered eARC when it reported on. Audio, eARC, SIMPLINK, receiver address/port, volume and input were restored; formerly default receiver address/port are now explicit equivalent values. Evidence: `/data/home/tmp/lumatv-restore-20261030/audio-acceptance-recorded-dt4esdx5/events.jsonl`.

Logging forwards to `192.168.1.69:514` over UDP with a 1 MiB RAM limit. Actual kernel and app markers reached Unraid; accepted stdlog is covered, compressed rotations are not replayed. The last detailed readback reported zero source drops/gaps and 852 KiB RAM logs. Evidence: deployment `syslog-receipt.txt` and `final-installed-readback.jsonl`.

The actual Home Search keyboard passed entry, on-screen Backspace and Back. System > Logging displayed the destination, forwarding status and RAM cap; its custom address keyboard opened and cancelled without changing the destination. Evidence: deployment `ui-evidence-20261031-155857-e4f7cb8f/`. This proves shell keyboard interaction, not third-party app IME coverage or measured 120 FPS pacing.

Historically, wake destination was unresolved. Native `bootd.log` proves Home was correctly saved before standby, but stock `wakeOnLan` excludes full Home booting and selects the Last Input App, Live TV. The runtime native-policy trial failed its readiness/log/foreground check; recovery restored the stock policy path and original full-Home reason list. No full TV reboot or new wake/source acceptance sequence occurred. The failed native trial is not proof that the policy itself was rejected.

The native bootd binary and actual rules are captured at deployment `native-wake-sao1j7vh/`; binary SHA-256 is `e701cafc51ae7faba89452f1cdc1efb314c9fccf3e70eb404bdca3d17e5fbc27`. Stock Last App Booting handles only `cpuAbnormal` and reads `last_foreground_app_id.json:appId`, not the normal saved foreground `bootd_last_foreground_app_id.json:lastAppId`. The candidate subsequently installed clones the native Cover App action, retaining hotkey/OTT ordering and other explicit-destination exclusions; previous-app resume covers WOL, remote/local power, `micomPower` and `lastPowerOn`.

The installed Rust CoverUI replacement launches real apps with four fixed candidate slots, including a final Home fallback. Native wake has 37 local behavioral tests and real-CLI trial/mismatch recovery smoke. The earlier native trial proved policy loading, not destination selection: its log truncated Home to `com.webos.app.hom`. Source now terminates selector and shell-fallback output with a newline for LG’s final-byte-stripping reader. Both paths passed actual-TV CLI smoke through r8 without installation; temporary source was removed. Later persistent activation and destination acceptance are recorded above.

Retail `configd` accepts top-level `volatile:true`, but its loaded service merges volatile values before Factory values; Factory wins whole-key conflicts. Nonvolatile API writes update Factory. The trial removes only the two owned stock-valued Factory overrides after checking stock main-database fallback; recovery restores Factory first. Main defaults remain unchanged. Captured bootd constructor/getter confirm an absent path retains `/etc/palm/action-condition.json`.

Historical: TV BusyBox 1.35 rejects GNU `chmod --reference`; preserve permissions with `stat -c %a` and explicit octal `chmod`. This copy incompatibility was corrected in the maintenance packet. Its resumed execution, request `c92e8ad3-7370-4950-a86a-0337bebfb1af`, failed at `native-trial` and completed verified safe rollback. The original autostart and CLI hashes are restored (`5aa2f6fb9006a36e6590bfd6750c223ff65543b09fcea538c19369801661fc5c`, `c98f340da1f36fcc0ca94e17d708d32ae2c15ecee089e12ffc0c0580069f96d9`); installed `tools/wake.py` is absent. Inactive owned policy/snapshot files remain at `/mnt/lg/cmn_data/lumatv/wake`. The formerly absent config key now explicitly names `/etc/palm/action-condition.json`; the original reason list is restored.

Historical: Work is paused at the owner’s 18:12 EDT request. Final evidence is deployment `wake-maintenance-evidence/events.jsonl`, attempt `2ebcae5af8a74f948a60f1b0cbdbde1a`: `failure-safe-rollback-complete` follows healthy, persistent, settled compositor status with zero failures, a physical DISPLAY capture and `finish-saver-confirmed`. The capture is `attempt-2ebcae5af8a74f948a60f1b0cbdbde1a/display.png`. Home was visually observed before the saver was activated. TV remained Active; Denon was last observed on with eARC selected before the native trial. No AVR power/source commands or TV standby/reboot were issued by this run. Nothing is scheduled for this chat. Both maintenance approvals executed once; neither is pending.

Resumed diagnosis establishes that native `bootd` opens its log with `O_RDWR|O_CREAT|O_TRUNC` (`0x242`, captured ARM binary address `0x6b28a`). The old inode/size witness therefore rejects ordinary restart-time truncation. The saved failed trial did not preserve its individual predicates, so this is a confirmed verifier defect, not proof of which predicate failed that run.

The corrected witness reads at most the final 1 MiB and requires the latest `PERFORMANCE`-stamped `BOOTD_START` within the trial's monotonic-clock window before accepting both owned-policy load records. The captured logger uses `CLOCK_MONOTONIC`; this set reports no hardware timer. A different clock origin fails closed. The actual captured-log replay accepts a fresh truncated log and rejects the same log as stale. Trial failure records retain each check's successful observations and exceptions before rollback overwrites native logs.

The native-only diagnostic temporarily migrates only the exact reviewed inactive selector, runs one volatile trial, restores stock config and prior selector/markers, and verifies unchanged installed tools and healthy renderer. It performs no renderer installation, persistent activation, reboot, standby, input/app launch or saver change. The obsolete BusyBox-resume command remains invalid.

Earlier diagnostic `06d095e48d7f48719b781f07b54deb33` failed configuration readback before restart. The actual Factory database contained the stock path and five original reasons, shadowing the trial. Read-only state evidence is `/data/home/tmp/lumatv-wake-readonly-2odth4qv/`. Its leftover RAM directory contained only a 20404-byte Python bytecode file; that file and the empty directories are removed. New diagnostic imports disable bytecode writes.

Corrected diagnostic `234ed66a71784b0e8c2133fbb39bebd8` passed on the TV: both owned-policy load witnesses were true and foreground remained Home. Stock settings, original selector/markers and original tools were restored; renderer remained healthy, persistent and settled with zero failures. Evidence: `/data/home/tmp/lumatv-wake-layer-trial.m8iisQ/wake-maintenance-evidence/`. The earlier frozen packet is unchanged.

The owner resumed the wake fix and explicitly prohibited further HomeLabber access/approval requests. HomeLabber is not used. Previously successful testing, deployment and recovery paths are required; no further wrappers or replacement frameworks. Both instructions are saved in global memory. The original 86-task inventory remains intact; one completed learning-record item, six render-first UI items and the immediate keyboard replacement bring the total to 94 (39 complete, 0 open, 55 blocked). No source publication is authorized.

Historical: Activation `6928bb03053940598f3ed1c93054cde3` failed the snapshot-start policy witness. Stock configuration was restored before a six-second bootd restart timeout; later reads proved active/running, success and no pending job. The existing `wake off` then succeeded, with stock config and no pending/trial markers confirmed. The existing backup restore returned the original autostart/CLI hashes and removed installed `tools/wake.py`; renderer remains persistent, settled and healthy with zero failures. The newer inactive selector, failed report and migration backup remain preserved because exact marker restoration rejects the historical timeout error. Do not replay activation or bypass that check.

Current source serializes prepare/reload, existing landing/apply and finalize with invocation/activation binding and scoped restoration after owned input teardown. It accepts LG’s padded early-start timestamps and normalizes only corroborated native Input App inputcommon wrappers to the matching input. All 61 wake tests, both shell syntax checks and captured-log delivery/selection smoke passed. Independent native review cleared the wrapper/action-pairing contract after missing-request boundaries were fixed; captured-log replay is not TV acceptance.

The TV has reviewed source `cfc2c1aac8c33654051184c758406980767ee5e0a3cb828ccef244b08656f233`, package SHA256 `8b1aba0a629fab7413285249bf2768712f1e9e886471cd05d32d2641e5f55f1c`. All 151 QML files compiled; three installed tools/helper/408-file compositor tree matched and apply took 16 seconds. Fresh Home activation `c413b8707ca44e47ac788a3296573221` passed runtime trial AND actual cold startup, invocation `b370dc321fda4bff9227a16b2d043d82`: both loaders, native readiness, destination and original scenario accepted, actual Home DISPLAY captured with no manual postboot Home launch. The existing two-minute supervisor healthy window then cleared, all persistent/live/verified/settled checks passed with zero failures and boot-ok marker absent. Existing wake commit succeeded; status enabled, pending null. A fresh post-setup cold test is running with prior invocation reports explicitly ignored; standby/intent/receiver/fallback acceptance still remains.

Historical inactive directories are preserved at `/mnt/lg/cmn_data/lumatv/wake-archive-46a404c6723042deb67505e53c1ec217`, `/mnt/lg/cmn_data/lumatv/wake-archive-4c6ad6ee0f2d49dfa2b1885c33afe751` and `/mnt/lg/cmn_data/lumatv/wake-archive-437a8319ad91488d9ac0ee48926b1d67`. The last preserves the disarmed failed activation7ad report and B51 selector; all files were checked before/after atomic rename under both locks with stock effective/Factory settings and no bootd job. Current evidence and exact packages are private under `/data/home/tmp/lumatv-wake-install.ms6eh5iv/`; full pre-update `installed-package-and-state.tgz` remains recovery evidence, not permission to overwrite mutable live state.

Verified constraints, evidence links and unresolved startup questions are recorded in [wake-learnings.md](research/wake-learnings.md). Append dated findings there; distinguish observation from inference and update this current-state section separately. Later new-policy acceptance and its limits are recorded above. A stock-policy HDMI-CEC wake preserved HDMI2/Apple TV despite saved Home and stale firstAppId Home; its native log and DISPLAY capture are in the learning record. Reuse the existing scripts and documented 25-second standby baseline. The owner waived the panel-refresh wait at 19:30 EDT for this work only. Preserve unowned `rustui/ui/src/system/saver/model.rs`; no commit, push, tag or release under the source-publication hold.

The compact toast and audio glyph changes are source-implemented and host-native tested; TV installation and acceptance remain pending (current maintenance state above).

The owner reported LG’s old keyboard in Marquee’s Bazarr address/API-key entry. The existing Rust keyboard passed both fields and LG Search; see [keyboard-result.md](research/keyboard-result.md). The original broader keyboard task is complete; actual visual remote input was accepted, without an FPS benchmark.

TV verification reuses `compositor/tools/tv-gate.sh`, the October 29 r19/r19w Rust controls/capture helpers and the Round 8 receiver baseline. Full gates also change sessions, refresh and final power state; select relevant operations. Read control JSON with stdout/stderr separated because the TV loader emits a preload warning. Home Search is focus 103, gear 102; key code 217 is only a test fixture. Wallpaper rows clamp rather than wrap: navigate toward the actual target index.

The supervisor deliberately keeps its `running` marker for the first 120 healthy seconds. A fresh successful installation can therefore report live, persistent, zero failures and `unsettled:true`; wait for settling before final acceptance rather than reinstalling. Preserve durable Cody-side results when MCP times out before its server command finishes; never repeat an unknown action. Cody’s Paramiko 5.1.0 lives in the persistent Python user site under `/data/home/.local/lib/python3.11/site-packages`; container replacement does not guarantee system-site packages survive.

## Earlier snapshot (LumaTV, 2026-10-30 EDT)

- The TV runs forge main (round 27 in progress); LumaTV's compositor is live persistently,
  LG's as the automatic fallback. Rust kinds are off by default.
- Camera pop-ups use SUB0 (the second video window) over apps and the CPU texture over
  Home. `popups.on` is OFF after an unexplained reboot during the camera full view over
  YouTube (round 26); turn it on only once that is explained.
- 120 Hz frame work: Frames120b, `docs/research/frames-120-result.md`.
- lumashow-ha 0.3.0 and LumaShow v1-20261030a are released; HA's LumaShow 0.3.0 runs with
  48 entities shared to LumaTV.
- All screens move to the 64-bit Rust shell; no new QML. Home, quick panel, picker, drawer,
  Cameras and part of Settings are opt-in. Remaining Settings and system surfaces are in
  progress. QML retires per kind only after two device parity passes. Recovery points are
  `rust-home-result.md`, `rust-panels-result.md`, `rust-settings-result.md`,
  `rust-cameras-result.md` and `system-layer-result.md` under `docs/research/`.
- The saver gains a dim analog clock, date, weather, notices and plugin items. The helper
  rules and camera-over-saver fix are implemented; surfaces and device checks are separate
  work. HA couch-occupancy wake is parked. Scrypted replaces HA cameras; bridge installation
  and pairing remain human actions (`scrypted-cameras-design.md`).
- Privacy is addressed on the TV, not by router blocks. The early hook stubs programs
  before masking units; the boot timeline must establish whether it beats LG's starts
  (`privacy-menu-result.md`). No data deletion or system-image changes are authorised.
- OSD0_1 held writes but did not latch its header or show an extra layer under LG's AFBC
  path. The linear case remains open; evidence is in `compositor/osd01-probe-2026-10-29/`.
  `display-chip-direct.md` ranks hardware work; `tools/regdecode.py` decodes captures.
  Host policy tests do not establish working cursor, AFBC or extra video windows on the TV.
- Interrupted work has snapshots and ownership notes at
  `/data/home/tmp/lumatv-recovery-2026-10-29/RESUME.md`. Read that and the live worker roster
  before resuming: replacements may already own the original worktrees. Preserve unfinished
  files. Rate limits and an exhausted API balance are separate failure modes.
- At most 12 helpers and one deep-task worker; no spawning by helpers. Only the verified
  human gate permits protected actions. No raw root infrastructure commands, secret reads,
  network changes, data deletion or protected installs outside that gate. A changed installer
  script is not covered by an earlier standing approval.
- Hourly check-ins and a 06:05 EDT recovery check are scheduled. Conservative defaults resolve
  routine engineering choices. Marquee remains parked until the Rust screens are complete.

## Earlier snapshot (LumaTV, 2026-10-26 19:05 PDT)

- Source of truth: `docs/PLAN.md` (phases, gates, decisions), `docs/SURFACES.md`
  (inventory), each `docs/research/*-result.md` (its worker's recovery point). An
  implemented file is not proof that its feature passed on the TV.
- TV installs: every LumaTV install on the set goes through Nitin's Face ID
  (HomeLabber `run_command` on beastnas running the installer in the Cody container,
  approved on his phone). A yes in chat does not count (PLAN, Decided). A request that
  expires pauses HomeLabber approvals until Nitin resumes them.
- Helpers: at most 12 at once; briefs forbid spawning. One TV owner at a time,
  deploying from a clean worktree; the compile gate (`tests/qmlhost/compile_all.py`)
  runs before every package or hand deploy. Helpers commit from their own worktrees.
- Installs: HomeLabber standing grant on `sh /mnt/nvme/appdata/cody/home/tmp/lumatv-install/tv-install.sh`
  (installs whatever `/data/home/tmp/lumatv-next` points at; one Face ID per 12 h).
- On the set at 11:00 PDT (10-28): package a1555df (round 14) on **LG's compositor**:
  YouTube (Cobalt) and Plex (plxnative) SIGSEGV under LumaTV's compositor (LG's SDL patch
  calls the webOS seat proxy, which is NULL; evidence `docs/research/compositor/slot-2026-10-28-r14/appcompat/`),
  so persistent go-live is paused until AppCompat's fix passes with app launches in the gate.
  Always Ready on (Nitin). On: `inputcommon`, `voice`, `store`, `remote.quickAccess`,
  `remote.quickPanelEdit`, keys restyle. Passed under LumaTV's compositor and waiting for
  go-live: keyboard luma, app switcher, transitions, overlays, OLED care orbit, capture 1-5.
  Video wallpaper fails (black margin, darker grade); 120 Hz holds idle but not the slides.
  venc's next TV probe needs Nitin's OK (the M2M node wedged the kernel once).
- Done 2026-10-26: compositor gate 9 (every safety check passes, frame timing fails:
  `compositor-gate9-result.md`); compositor step 1, the remote's keys and the keyboard
  under LumaTV's compositor, passed on the set (`compositor-step1-result.md`); quick
  panel and picker round 16 (`shell-host-result.md`): over Netflix and HDMI every panel
  opens in 146-159 ms, over Home p90 2.42/2.54 ms.
- Open, in order (also the todo list):
  1. Installed 3036de9 through Face ID (2026-10-26 23:28 EDT, during Netflix, which kept
     playing): c496611's still mute mark, the Zigbee2MQTT OTA guard, `revert` removing
     `vt_devmode`. Not yet re-measured: the one late frame per panel opening over Home.
  2. Light sync: the Zigbee2MQTT OTA guard and back-off are built (4dd5a5b, offline);
     `vt_devmode` is on for good (Nitin, 2026-10-26). There is no Hue light by the TV:
     Nitin picks which lights follow the picture; WLED strip not set up (PLAN, Ambient
     light, Status).
  3. TV checks: Live TV restyle, Settings look-only walk, frame-rate blink with Nitin
     watching.
  4. Parked by Nitin: compositor frame timing and moving Home and the panels to
     LumaTV's compositor; Music Assistant.
  5. Nitin's steps: GFN sign-in, HA token and plugin enable, Bluetooth proxy link, LG
     store check, leak sensor OTA retry, Denon OSD off, Apple TV CEC off.
- Netflix tvq-pb-101 came from an input app left holding the audio decoder port;
  restarts end the input apps first, `lumatv inputs status` shows a held port, and gate.py
  closes the HDMI app it opens.
- Backups of uncommitted work: `/data/home/lumatv-backups/wip-*`.

Written so any model can continue cold. The plan is `docs/PLAN.md` (Status,
Phases, Decided); this file holds the proven mechanics.

- State: the build is under way (PLAN.md track C and the surfaces). Proven on
  the set and in the repository:
  - The rename to LumaTV (`docs/research/rename-result.md`), the Home stub
    (`docs/research/homestub-result.md`), the performance work of phase 1
    (`docs/research/perf/summary-2026-10-24-phase1.txt`).
  - Surfaces phase 1 (`docs/research/surfaces-p1-result.md`): fail-open key
    filter, OK on release, the focus rule, wallpaper tint, one token source,
    Manrope, the component kit.
  - Surfaces phase 2 (`docs/research/surfaces-p2-result.md`, which has its own
    Resume here): the startup warm-up, the per-surface frame gate and the
    startup A/B in `docs/research/perf`, the offline QML host
    (`tests/qmlhost`), Home's weather and news from the native helper, the
    helper's start and stop in the tool, the preload trim, the boot and
    power-off logo windows, `lumatv surfaces` (the registry of every switch and
    its way back) and the notification view (`lumatv notifications on|off`: LG's
    tree copied on the set and patched there with LumaTV's own toast laid over
    it, nothing of LG's shipped; `tests/qmlhost/notify.py` runs LG's real view
    offline). LumaTV's Settings (`Settings.qml`, `settings/`: Home & look and AI
    access pages) is a layer of Home opened from the gear's popover, and the AI
    prompt (`AiPrompt.qml`) draws the helper's `ai.prompt` over Home; both are
    tested offline against a fake helper (`tests/qmlhost/fakehelper.py`, which
    models `/v1/ai`) and have not run on the set. The set has run the warm-up and
    the Home gate (slot 1); that note's Log has the numbers.
  - The native helper (`helper/`, `docs/research/native-helper-result.md`):
    proven on the set (smoke test 35 of 35; AI-access smoke 42 of 42, which skips
    the two checks that touch the TV and the receiver). The plugin broker
    (LumaShow's plugin API level 1) is built and host-tested, not yet run on the
    set. Its API is on `127.0.0.1:18731`; files under `/var/lib/lumatv`:
    `helper.lock`, `helper.json` (optional), `cache/weather.json`
    and `news.json`, `settings/<owner>.json`, `bin/lumatv-helper` (the staged copy
    the tool runs). Stop it with `lumatv helper stop` or `lumatv-helper stop
    --brand <app dir>/brand.json`; if TLS misbehaves set `"fetch":
    {"backend":"curl"}` in `helper.json`.
  - 64-bit drawing (`gfx64/`, `docs/research/gfx64-result.md`): heap shim,
    launcher, pinned blob fetch; smoke run on the set 60.63 fps over idle Home.
    The Rockchip blob bundle is fetched by `gfx64/fetch-blob.sh` (about 37 MB)
    and never committed; an ipk that ships it must run that at build time.
  - LumaTV's own compositor (`compositor/`, `docs/research/compositor-c2-result.md`):
    C2 passed on the set; C3 and C4 built offline, first window and bridge dry
    run on the set. The tool owns the enable path now (`tools/compositor.sh`,
    `lumatv compositor on|off|status`, `lumatv compositor-boot` in the boot
    worker): the bundle itself is put in `/var/lib/lumatv/compositor` by
    `compositor/tools/tv-cutover.sh install` until it ships in the ipk. LG's
    compositor is the fallback on any failure; `lumatv revert` unmounts and
    forgets the flag. Fragility: after LG's compositor was away, `webapp-mgr`
    can be left `failed` and the web Home stub cannot launch (black panel):
    `systemctl reset-failed webapp-mgr; systemctl start webapp-mgr`, then
    launch `com.webos.app.home` (`tools/tv-recover-home.sh` does it, and
    holds `/tmp/lumatv-recover.lock` while it runs; the tool never restarts the
    compositor's unit while that exists).
  - Hardware census (`docs/research/hardware-census.md`), the top five by benefit
    over risk, none run yet and none needing a kernel module: 1 the video tap and
    picture histogram (`libvtcapture`, for ambient light), 2 the TV's own audio
    mix (`hw:1,2`, `hw:1,3`), 3 the hardware JPEG block, 4 hardware video decode
    for a video-plane wallpaper, 5 Bluetooth LE through LG's `bluetooth2` service
    (PLAN.md, Hardware census). LumaShow lessons:
    `docs/research/lumashow-lessons.md` (nice -19 on surface-manager is LG's own,
    nothing to adopt).
  - Declutter (`tools/declutter.sh`, `docs/research/declutter-result.md`): the
    `lean` profile is on and persisted on the set (uploaders, ads, push,
    settings, apps, voice, connectivity, share, misc; `ai`, `livetv` and `cast`
    off). Rebooted clean at 04:08 and checked at 05:15: Netflix, YouTube,
    PlxNative, Apple TV (about 20 s), Crunchyroll and Prime Video launch;
    GeForce NOW, a web app, opens the browser as before; the receiver is
    connected on HDMI 2 with sound on `external_arc`; Home Assistant sees the TV.
- Nitin's decisions of 2026-10-25 (also in PLAN.md Decided): the approved
  mockups in `docs/mocks` are the design, Settings layout included; LumaTV's
  compositor goes live as soon as it passes the C4 gate, LG's staying as the
  automatic fallback; LG's telemetry, ads, promotion and app-push services are
  removed entirely and webOS is trimmed to essentials through two reversible
  profiles, `ads` and `lean` (`docs/research/declutter-result.md`); LG's power-on and
  power-off logos are replaced by LumaTV's, three options each in
  `docs/mocks/logo`, option 1 the default; Prime Video stays unless an LG
  surface clearly pushes it. `lumatv preload on` keeps Netflix and Prime Video
  warm and drops YouTube, LG Channels and the browser from the list; the A/B on
  the set (slot 2) showed no gain, since it closes nothing that runs, so it is
  off until a cold boot shows one.
- Remote tests: done 2026-10-24 for Netflix 4K Dolby Vision playback (X4
  pass 3, contiguous-buffer hold) and the SSAP key device; results in
  PLAN.md. Still open: X4 during PlxNative playback
  (`docs/research/arch/x4/x4-monitor.sh`, `x4-analyse.py`); the histogram
  probe (its ABI was recovered offline, commit dd3c57a). Mute
  the receiver first (`media_player.living_room_avr`) and restore it after.
  Netflix deep link for the 4K Dolby Vision test film: launch `netflix` with
  `contentTarget` `m=https://www.netflix.com/watch/80141336&source_type=4`,
  then ENTER. Protected video captures black; the tests record compositor
  behaviour. During Dolby Vision playback Picture Mode reads `dolbyHdrCinema`;
  it returns to `filmMaker` on Home.
- Tests that need the owner: PIN prompt (a lock is a security setting he
  sets), pointer cost (Magic Remote moved by hand), CEC keys from other
  remotes, receiver volume drift.
- Set rules: confirm it is idle first (`getPowerState`,
  `getForegroundAppInfo`); Picture Mode stays `filmMaker`; never restart
  SAM; write only under `/var/lib/lumatv`, `/tmp`, configd, bind mounts
  and `/var/systemd/system/env`; revert every experiment; leave it in
  Active Standby unless the owner is about to watch; no DRM bypass. Slots on
  the set are asked of the main agent and answered "go <name>".
- Tests: `python3 -m unittest discover -s tests -p 'test_*.py'` (101, about
  55 s; the ones that start the offline QML host need PyQt5 and an X display,
  `tests/xdisplay.py` starts Xvfb itself) and `TZ=America/New_York bun test
  tests/` (36). The frame gate on the set: `python3 docs/research/perf/gate.py`
  (every number and rule is in its docstring); startup and preload A/B:
  `startup.py`, `preload_ab.py` beside it.
- Working style: one step at a time, commit and push after each
  (`git push -q forge main`), results written to files with their own
  "Resume here" note, so a model switch mid-task loses nothing.

## The rule that must not be broken

https://rootmy.tv/warning — **never write to the kernel, rootfs or tvservice
partitions.** A set bricked that way is not recoverable in software.

Everything this project does stays inside that boundary, deliberately:

- Only bind mounts and files under `/var` (writable ext4) and
  `/media/developer`. Never a raw write to a block device.
- The single exception is `snapshot-boot-manager --remove`, which clears the
  hibernation image on `mmcblk0p54`. That is LG's own tool on its own
  scratch partition, the kernel rebuilds the image on the next boot, and the
  worst case is one slow cold boot. It is not a firmware partition.
- `/usr`, `/etc`, `/lib` and `/mnt` are read-only overlays with lowerdir only
  and cannot be written even as root, which is why the mod shadows the
  compositor module and bind-mounts over Home's assets instead of editing
  them in place.
- No firmware flashing, no `dd` to `mmcblk0*`, no epk handling.

If a future change needs a partition write, stop and reconsider the design.

## The set

- LG OLED65C4PUA, model code `o22n2`, webOS 10.2.1, firmware 33.22.52, MediaTek
  (`chip=O22A3`), 2 GB RAM, armv7 userspace (`GLIBC_2.28`).
- Rooted with Homebrew Channel. SSH root login via Homebrew Channel's sshd; no
  passwordless sudo, the login is root. `luna-send` from a non-PTY SSH exec
  hangs — allocate a PTY. `journalctl` hangs — read `/var/log/messages`
  (monotonic timestamps in brackets), which rotates within minutes.
- A plain `reboot` returns powered on and is safe to drive over SSH. The luna
  `machineReboot` drops to standby and does not power back on. Wake-on-LAN
  works with `wolwowlOnOff` on (built-in LAN port), so a set in standby or
  fully off can be woken with a magic packet.
- `/sys/power/mode` reads `resume` on a normal boot and `making` on a boot
  that rebuilds the hibernation image.
- Screen capture: `luna://com.webos.service.capture/executeOneShot` with
  `{"path":"/tmp/x.png","method":"DISPLAY","format":"PNG","width":960,"height":540}`,
  then sftp the file. A blank panel returns ~835 bytes at 320x180.

### Developer-app filesystem and maintenance

The established deployment entry is `/data/home/tmp/lumatv-install/tv-install.sh`: app install, `tv-first-light.sh deploy`, then `cutover install`. The earlier installer uses separate SSH connections per command and a dedicated SFTP connection per transfer; native control commands use a PTY. Installed Rust binary hashes are recorded in `docs/research/compositor/slot-2026-10-29-r19w/log.txt`. Reuse this transport before adding another deployment mechanism. Full historical `apply` and cutover scripts also change unrelated app state and restart the display, so selective updates must retain the current artifact scope and private configuration.

On the reference set, `/media/developer/apps/usr/palm/applications/org.nphil.lumatv` and its developer-app ancestors through `/media/developer` are root-owned mode0777 directories. Existing public app files can also be mode0777. The approved maintenance packet must pin the inspected ancestry identities and retain no-follow checks; this layout is not a reason to relax private `/var/lib/lumatv` or RAM-workspace permissions, or to change directory modes globally. Device and inode numbers must come from current inspection, not a previous run.

The established full-package and Rust-bundle installation remains the deployment path. The experimental combined-maintenance runtime validator is not an installation prerequisite. Preserve user-facing configuration through the existing migrations, retain rollback copies and storage checks, and verify installed hashes and running services; a wrapper exit code alone does not prove installation.

Normal work is allowed while the TV is in use; a screensaver is not a maintenance prerequisite. OLED idle protection and state-dependent screensaver tests are separate. Display interruptions require owner coordination. Installation while watching permits an agreed compositor interruption, not deliberate TV or AVR standby; physical power-cycle acceptance stays separate.

Large Python maintenance programs travel over bounded SSH stdin with a fixed command under1KiB and data chunks at most16KiB. Dropbear documents a32KiB default receive-payload limit; the former quoted maintenance request was49,986bytes and reproduced a connection reset against that limit. The TV-specific reset cause remains unconfirmed. Native Luna commands retain the existing PTY transport. The current install-while-watching scope permits a compositor interruption but no deliberate final TV or AVR standby; physical power-cycle acceptance stays separate.

`surface-manager-daemon.service` starts `/usr/bin/surface-manager.sh`. Maintenance owns the verified unit and its complete existing cgroup, with process-start and executable continuity checks; child names alone do not establish ownership. An existing bind-mounted `/usr/bin/surface-manager` can retain an unlinked executable inode. Any zero-link read exception is restricted to that canonical path with matching mount, inode and content evidence; other regular files retain the single-link requirement.

## What the mod is

Two halves, one owner each.

- Compositor: a full shadow copy of the `WebOSCompositor` QML module (318
  files) at `/var/lib/lumaglass/qml`, with `views/fullscreen/
  StarfishFullscreenContainer.qml` swapped for stock-plus-one-`Loader` and a
  `lumaglass/` directory added beside it (LumaHome.qml, Glass, StatusBar,
  Widget, Dock, Tile, the widgets, pre-rendered icons). `surface-manager-
  daemon.service` has `EnvironmentFile=-/var/systemd/system/env/surface-
  manager.env`; the mod writes `QML2_IMPORT_PATH=/var/lib/lumaglass/qml:/usr/
  lib/qt5/qml` there. `/usr` is a lowerdir-only overlay and can never be
  written. The Loader instantiates LumaHome above the Home surface while
  Home is the container's app; the stock Flutter Home stays resident and
  unseen underneath, or the web stub that `homestub on` puts in its place (see
  "The Home stub").
- Home: `mount --bind /var/lib/lumaglass/assets` over
  `/usr/palm/applications/com.webos.app.home/data/flutter_assets/assets`
  (decluttered `home.xml`, strings, blank hero banners), then relaunch Home.
  A bind does not survive a reboot.

Input: the compositor's `KeyFilter` sends remote keys straight to the focused
Wayland window, so a QML item never sees them through QtQuick focus. The
stock `StarfishKeyFilter.qml` builds its handler chain from the configd list
`com.webos.surfacemanager.keyFilters` (`[{file, handler}]`, loaded through
`StarfishKeyFilterLoader.qml` + `Qt.include`, re-read live on change). The
tool appends `{file: /var/lib/lumaglass/keyfilter/lumaglass.js, handler:
handleLumaHome}` at apply and boot-bind and removes it at revert. That script
runs inside the compositor's QML engine, instantiates `LumaKeyBridge.qml` from
the module and dispatches through the `LumaBus.js` pragma-library instance
LumaHome registered with; it accepts a key only while Home is the active
surface and LumaHome consumed it, everything else falls through to LG's
chain. Pointer input is ordinary QtQuick mouse events (LumaHome sits above
the surface item and swallows what no card or tile takes).

Rendering: one `GaussianBlur` + `HueSaturation` of the wallpaper at half
resolution, baked once into a `ShaderEffectSource`; every card, tile and the
status bar is a single `ShaderEffect` quad that samples that texture at its
own screen rect and draws tint, inset edge, sheen, rim and drop shadow from a
rounded-rect signed distance. No layers, no per-frame effects; focus is a
uniform plus a transform. Text is `NativeRendering` on integer positions and
never inside a scaled item (the glass under it scales, the content does not).
Icons are PNGs rendered by `generators/gen-icons.py` at display size (no
SVG plugin on the set).

Frame budget, measured with `focusMs: 5000` and the per-second `[FPSLOG]`
counter (middle seconds are exact): the set is fill-rate bound. Dock alone
60; each widget card's glass quad costs ~2 fps until the whole layout sat at
50, and a two-read live wallpaper took 10 more. What brought it back to 60:
shadow-only early return outside the card/tile rect, margins trimmed to the
shadow reach, mediump colour math, scrim folded into the wallpaper pass, the
covered stock Home surface at opacity 0, and the wallpaper warp moved into
the vertex shader on a GridMesh. Every new full-screen or card-sized quad
must be re-measured this way. Measured: idle 0 frames; the analog second hand's
160 ms tick renders ~10 frames/s; a 150 ms focus change renders 9 frames,
i.e. 60 fps.

Theme: `/var/lib/lumatv/theme/` (`theme.json`, `layout.json`, assets) polled
every 2 s and applied live. The files there hold only what the owner changed:
LumaHome merges them over the shipped `payload/theme` files, which `stage_luma`
stages beside the QML as `theme.default.json` and `layout.default.json`
(objects merge key by key, arrays and scalars replace), and `lumatv apply` runs
`tools/theme.py minimize` on them so what was never changed follows an update.
`theme.json` is the one token source (colour roles of both materials in
Lucent's names, type roles, radii, spacing, focus, states, motion, font);
`tools/tokens.py` resolves it for the web CSS (`theme.css` in the web apps'
overlay), for the settings-sheet palette Quick Settings is patched with, and
for `theme/Tokens.js`, which QML in other processes imports by file URL. The
compositor's QML reads it through `kit/Lu.qml`. `layout.grid.rows` is a count
or an array of pixel heights; the shipped layout pins the mock's 222/238/392.

Key codes: the remote's Back arrives as evdev KEY_PREVIOUS (412), not
KEY_BACK (158); injecting 158 does nothing on Home. Qt 5.12's QML
XMLHttpRequest can PUT to file://, which is how the Home popovers save
theme.json (the compositor is not allowed to call Homebrew's exec service).
It goes through the network access manager's queue, though: PUTs of the Home
stub's heartbeat twice landed 5 s late while the timer ticked on time. Where
timing matters LG's own `TextFile` (`import TextFile 1.0`; `name` as a
`file:///` URL, `exists`, `read()`, `write(text)`, all synchronous) is used
instead, as in the stub's heartbeat, mode, escape and confirm files.

Luna from the layer: `Service { appId: LS.appId }` (the compositor's own
identity; a made-up appId gets no reply, silently). Account data:
`accountmanager/getLoginUserData {serviceName:"LGE", subscribe:true}`
(`profileNick`, `iconNick`, `profileBg`, `id`). Launch-point order:
`homelaunchpoints/listLaunchPoints` (subscribe) carries `unmovable`;
`moveLaunchPoint {launchPointId, position}` writes it.

Mini apps: LumaHome's `openMini(spec, fromRect)` grows `MiniFrame.qml` (one
Glass quad) from a card to the centred frame; `kind: "web"` launches
`org.nphil.lumaglass.frame` (appinfo `defaultWindowType: "overlay"`,
`transparent: true`, `visible: false`) with `{url, title, source, ago, mode,
light, rect}`; the app positions its pane at `rect` and leaves the rest clear.
WAM honours `defaultWindowType` from appinfo; overlay surfaces are accepted by
StarfishOverlayView and the compositor's own `handleSystemKeys` closes them on
Back. LumaHome subscribes to `applicationManager/running` to see the app
appear (spinner off) and disappear (frame closes). The app reaches the root
service through `PalmServiceBridge` (`window.webOS` is not present in a WAM
app without the webOS.js library); fonts must live inside the app directory
(file:// access to other directories is blocked). `kind: "qml"` content goes
in `MiniFrame.contentSlot` for native mini apps (weather, HA, calendar).
`appInstallService/dev/install {id, ipkUrl, subscribe}` installs a local ipk
on the rooted set; the frame ipk is built by `tools/build-ipk.sh` into
`payload/frame/` and installed by `install_frame_app` at apply.

Not built yet: widget rearrange/resize (hold on a card, move/snap on the
grid with animated repositioning, span/rows changes written back to
layout.json). The dock's move mode is the pattern to extend: pick up on
hold, ListModel.move so existing delegates animate, persist on drop.

State lives in `/var/lib/lumaglass/`; the CLI is `tools/lumaglass`, the boot
worker is `tools/autostart.sh`. `status` is the source of truth for what is
live.

## Boot: the proven mechanism

The set does not cold-boot from standby. `/proc/cmdline` carries
`snapshot resume=/dev/mmcblk0p54`, boot status reports
`snapshot-resume-done: true`, and the compositor restored from that image
carries the pid and start time it had when the image was made. A "first
start" of the compositor is therefore a memory restore, and systemd
timestamps read on a resumed boot are values baked into the image, not this
boot's.

On a genuine cold boot (`mode=making`), measured:

| event | monotonic |
| :--- | ---: |
| `mount-dynamic-partition.service` | 1.07–1.32 s |
| `ls-hubd` ExecMainStart | 1.32 s |
| `var.mount` (p58 on /var) active | 1.98 s |
| `surface-manager-daemon` ExecMainStart | 2.63 s |
| `sam.service` | 5.28 s |
| `mount-readwrite.service` active | 10.8–13.1 s |
| `kdump.service` ExecMainStart | 12.1–14.3 s |
| image capture (making boot) | ~15 s |
| Home first paint | ~12–18 s |
| Homebrew Channel `init.d` run-parts | ~33 s |

The compositor's first start can never load the modded QML, and the reason is
the filesystem, not ordering. Until `mount-readwrite.service` relocates it,
`/var` does not show the writable ext4 subtree, so `/var/systemd/system/env`
is not a path that exists yet; the daemon's optional `EnvironmentFile` read at
2.63 s is a silent no-op. Proof:

- The env file's atime, set to 2020 before a cold boot, was untouched after it.
- Marker variables planted in `ls-hubd.env`, `configd.env`, `memchute.env`,
  `tvpowerd.env`, `bootd.env` and `sam.env` were absent from every one of
  those processes' `/proc/<pid>/environ`. LG's own env files are inert that
  early too.
- `/var` is a subtree mount (`mountinfo` root `/var` on p58).
  `/var/systemd/system/env/surface-manager.env` and
  `/mnt/lg/cmn_data/var/systemd/system/env/surface-manager.env` are the same
  inode; `/mnt/lg/cmn_data/systemd` does not exist. A copy staged at p58's
  root was not read either, so the early `/var` is the firmware overlay
  (which holds only `lib`), not any view of p58.

Consequences: one compositor restart per boot is structural, ~13 s is the
floor for anything a root mod can do, and every restart after ~13 s picks the
env file up reliably.

### The early hook

Homebrew Channel's run-parts fires at ~33 s, after Home has painted, which
produced the stock-Home-then-blink-then-mod sequence. The mod now triggers
from `kdump.service`: it starts at 12–14 s, reads
`/var/systemd/system/env/kdump.env`, and its script's first act is a bare
`grep`. `persist on` writes `PATH=/var/lib/lumaglass/earlybin:...` into that
env file and a shim named `grep` into `earlybin` that starts the worker once
per boot and then `exec /bin/grep "$@"`. On a set without `crashkernel` on
the kernel command line the script is a no-op that exits 0, and the unit is
`Restart=no`. The shim uses absolute paths throughout (a bare `grep` inside
it would re-enter itself) and launches the worker with a plain PATH. The late
hook stands down when `/tmp/.lumaglass-worker` names a live pid or the boot
completed.

The worker never lets the wake preparation decide whether LumaTV starts. `wake boot-check prepare` only
chooses the app to land on; when it fails (it rolls itself back) the compositor is activated with no
landing. Its `systemctl daemon-reload` is cut off at 6 s and ran past that in the power-loss cold boot of
2026-11-03 20:38, which stopped the worker before activation and left the set on LG's compositor with
LumaTV's mounted. A standby activation that fails because the set was switched on meanwhile gets one more
pass, as an active boot. A failed preparation leaves the wake policy off (`wake status`: `off`, with
`failure.json` kept) until it is staged again.

### Readiness

`systemctl is-active`, the Home process, `NL_HOME_SHOWN` and
`getForegroundAppInfo` all go true ~8 s before anything is composited. The
only honest gate is the panel: `executeOneShot` at 320x180 returns 835 bytes
while blank and >79000 once Home is up. Measured after a restart: content at
+12 s, and asking for Home at +4 s does not delay it. The old sequence slept
14 s before launching Home and 8 s after; both sleeps are gone. The gate is
bounded at 30 s, and on the boot path a timeout is not an error (a boot into
standby never presents).

### Result

Wake from cold, frame-captured every 2 s (`evidence/boot/`): live TV at 12 s
(the image's restored state), black from ~17 s (our restart pre-empts Home's
first paint), modded home fully formed at 33–36 s. Previously ~55–60 s with a
stock-Home phase in between. Whether Home paints before the restart lands is
a race that varies by a second or two per boot.

## Standby: the proven mechanism

After power-off the set enters active standby (RAM alive) within ~3.4 s. A
power-on from there is instant with the mod still on screen. What made it go
cold was `faultmanager`: at power-off it scans
`/tmp/var/log/reports/librdx`, and any report there makes it answer
`responseSuspendRequest` with `ack:false, reason:"rebootToSuspend",
rebootReason:"rebootByFault"`; tvpowerd then reboots ~6 s after entering
standby. Captured on the bus with
`ls-monitor -f com.webos.service.tvpower` written to `/var` (not `/tmp`,
which does not survive the reboot).

Two reports were arming it.

1. The compositor restart makes `com.webos.app.inputcommon` segfault on
   teardown (`QFontDatabase::removeAllApplicationFonts`), landing ~3 s after
   the restart. The sweep meant to remove it never worked: it read names back
   from `ls`, which word-splits on the spaces every name contains and prints
   the 0x02 bytes rdxd uses in place of `/` as `?`; `rm -f` on those names
   was a no-op counted as success. The sweep now uses pathname expansion
   (raw bytes) and runs again once Home is back on the panel.
2. `webos-sddp` (Control4 discovery) segfaults in `SDDPSetDevice` (NULL
   dereference, backtrace `main → start_sddp → SDDPStart → SDDPSetDevice`)
   whenever it reads the interface list and finds a point-to-point interface
   carrying the multicast flag: the Tailscale `tun` (`/32`, no broadcast) on
   the reference set. Proven both ways: with `tailscaled` stopped it starts
   and stays active; with the tunnel present it dies within 3 s of
   `systemctl start`. A stock set has no such interface. At boot it starts at
   ~42 s, before Tailscale's hook (which sleeps 30 s) creates the tunnel, so
   it survives until the first wake re-reads interfaces - which is why it
   first looked like a wake crash. Not this mod's and not LG's for normal
   customers. Its script is gated on the set's own `enableSDDP` setting, so
   the **Instant on** toggle (`standby warm`) turns that off and Quick
   Start+ on, remembering the previous value; `standby cold` restores it and
   turns Quick Start+ off. Removing Tailscale from the set would make the
   SDDP part unnecessary.

   Nothing else on the set is affected by the tunnel: `avahi-daemon`
   (AirPlay/HomeKit), `chromecast-provisioning`, `miracast`, `appcasting`,
   `connman` all active; no faultmanager detections; no crashd or rdxd
   history; the three failed units (`broadcast-configd`, `f2io-on`, `gibbs`)
   are tuner one-shots that fail at 12-18 s, before the tunnel exists.

   Tailscale was removed from the set on 2026-09-14 (node logged out, daemon,
   init.d hook and `/media/developer/tailscale` deleted). Remote terminal
   access goes through the Unraid node's subnet routes instead; nothing on
   the set depends on a tunnel any more, and `enableSDDP` is off only because
   the Instant on toggle sets it.

Measured after both: three power-off / wake cycles of 150 s each with no
reboot, uptime carrying across, the same compositor still running, and Home
from a warm wake painting in 8 s (`evidence/home-after-warm-wake.png`).

Related facts from `tvpowerd`: `com.webos.service.tvpower.instantBootRefreshPeriod`
is 24 (hours) via configd, after which a warm set is refreshed cold once
("Wait for 1 Minute at active standby" then "Go to Cold"). There is a
`warmOwner` / `registerActiveStandbyRequest` protocol
(`{"clientName","timestamp","ack"}` responses) and a learned power-off
history at `/mnt/lg/cmn_data/tvpower/webos_offhistory`; neither needed
touching.

## The Home stub: the proven mechanism

LG's Home (`com.webos.app.home`, a Flutter app) is retired behind a web stub:
`payload/homestub` (same id, `type: web`, `appCategory: ["home"]`,
`visible: false`, a blank page) staged at `/var/lib/lumatv/homestub` and
bind-mounted over `/usr/palm/applications/com.webos.app.home`, so LumaHome is
the only Home. `lumatv homestub on|off|confirm|status`; results and the way
back in `docs/research/homestub-result.md`.

- **Order.** The Home assets bind goes in first and stays under the stub (the
  mount over the parent hides it; unmounting shows it again). Then SAM:
  `blockSystemApp` followed at once by `releaseBlockSystemApp` makes it read
  the directory again in under 0.1 s, with no SAM restart, and
  `blockedSystemAppList/USA.json` is byte-identical afterwards (md5 `26e1a4ce...`
  after dozens of cycles). `getAppInfo` then reports `type: web`. The release is
  retried: an app left blocked cannot launch. Only then is LG's instance
  stopped (`pkill` on `flutter-client -i com.webos.app.home`); with Home in
  front, SAM's `last_input_handler` opens the stub about a second later. Going
  back: unmount, block and release (`type: flutter`), then WebAppMgr's `killApp`
  on the stub page (`closeByAppId` only backgrounds it); SAM opens LG's Home
  about 3 s later.
- **The manifest must not set `handlesRelaunch`.** WAM then logs "handlesRelanch
  : true; Do not call raise()" and the stub stays behind the app the Home key
  was pressed in. Home launches skip memchute's memory check ("launch without
  memory check").
- **`/usr` is a shared mount**, so the bind and the unmount propagate into every
  app jail's mirror of the directory.
- **WebAppMgr is the stub's clock.** A compositor restart kills WAM, systemd
  restarts it, and the stub can only launch once it is back: 3 s on two boots
  and on every restart on an idle set, 56 s (stub at 65 s) on the first boot
  after an install. The fullscreen view is hidden and the container 0x0 while no
  surface exists, so nothing of LumaHome can cover that gap: black until Home
  is up. LumaHome is therefore built when the compositor starts (the container
  loads it for the base container at once), which also gives the guard its beat
  and a load error immediately.
- **WebAppMgr dies with the compositor.** After `systemctl restart
  surface-manager-daemon` (boot: restart at 35-36 s) the old WAM closes about
  2.5-3 s later and systemd starts a new one 0.1-0.3 s after that
  (`webapp-mgr.service`, `Restart=on-failure`; serving at 39 s). A web Home
  launched in between goes with the old one. SAM's own `last_input_handler`
  launches Home at about +3.4 s, into that gap, and loses it too; the tool's
  launch (`relaunch_home`) therefore waits until `webapp-mgr.service`'s
  `ExecMainStartTimestampMonotonic` differs from the value taken before the
  restart and `com.palm.webappmanager/listRunningApps` answers. Before that
  gate a boot left the panel black (the boot-5 failure in the result file),
  because the old rule took "Home running, boot path" for a boot into
  standby; it now needs a power state other than Active.
- **Qt 5.12's `Binding` does not restore a property** when its `when` ends over
  a surface that stays. The container sets the Home surface's opacity to 0 or 1
  explicitly; before that, LG's Home returning under a LumaHome that stepped
  aside stayed at opacity 0, a black screen.
- **Boot.** `autostart.sh --worker` runs `homestub-boot` first: stub in place at
  uptime 15 s in three boots, before the compositor restart (36 s), so Flutter
  Home never ran. It skips the stub when the previous boot never reached
  `boot-ok`, and a boot-time fault or two in a row turns the switch off.
  bootd's own first app is launched at 13.9 s: on a `reboot` that is the last
  input (reason `micomPower` is not a full-home boot), but a power-on that
  opens Home would start Flutter Home about 1 s before the 15 s mount and be
  killed 1.5 s later. `homeAutoLaunch` was left as the owner set it: holding
  it off would make every wake land on the last input.
- **The guard** (`lumatv homestub-watch`, one detached shell, 0.05 % of a
  core: builtin reads, a fifo for the one-second wait, one `grep` every 10 s)
  watches `/tmp/lumatv-beat`, which LumaHome rewrites once a second for as long
  as it lives. A beat unchanged for 5 s (and the file 3 s old) is a fault; the
  guard also acts at once when the compositor restarts without the module (its
  environment lacks the shadow path) and when the container writes `error` in
  place of a beat because LumaHome failed to load. A fault drops the stub
  (mode `lg`), opens Home if the stub was in front, and counts a strike; two in
  a row, or an unfinished boot, switch the stub off. Strikes clear after a
  minute of beats.
- **LumaHome and the tool talk through `/tmp`:** `lumatv-beat` (LumaHome writes),
  `lumatv-homemode` (tool writes: `stub|lg|legacy <token> [trial=<s>]`;
  `lg` steps the layer aside, the token changes with every write), `lumatv-escape`
  (the Home key writes) and `lumatv-confirm` (OK on the trial toast writes).
  The compositor may not run commands, so every action goes through a file the
  guard reads. LumaHome and the key bridge use LG's `TextFile` for all of them,
  synchronously; `XMLHttpRequest` PUTs for the beat were seen to land 5 s late.
- **Home held 650 ms** is watched by the key filter's first handler (the press
  still reaches LG's chain untouched, so a short press behaves as before). At
  650 ms it writes the request, cancels LG's own 2.5 s hold (Screen Zoom) with
  `longPressTimer.setBlockCancel(false); cancel()`, and swallows the repeats
  and the release. With no stub the layer hides itself, since LG's Home is
  already underneath.
- **Trial.** `on` writes `/tmp/.lumatv-homestub-trial` (a deadline in uptime
  seconds); the guard drops the stub when it passes. Only `confirm`, or OK on
  LumaHome's toast, writes `"homestub":true` to `config.json`; a reboot in the
  middle of a trial ends it, since the file is in tmpfs.
- **Do not run `apply` over an SSH command that can time out**: the shell dying
  mid-restart leaves the compositor on whatever it had loaded (a stale module
  that `apply` then calls current). Detach it (`nohup`) and read its output.
- **Boot direct (built 2026-11-03 behind a flag, not run on the TV).** The stub again, under the Rust shell: SAM's Home is the stub from the
  boot worker's first act while LumaTV's compositor is live, so Flutter Home never starts. Home held 10 s is the one way to LG's Home (the
  supervisor's escape: it ends everything LumaTV started, drops the stub, launches LG's Home, leaves the flag on); a reboot or the LumaTV app in
  LG's launcher brings LumaTV back. `docs/research/boot-direct.md`.

## Tried and dead

- **A LumaHome boot cover for the stub's late launch.** Built and logged on the
  set: the cover state flipped on, and the screen stayed black. The base
  container is 0x0 and invisible, and its parent view invisible too, while no
  surface exists, so a layer inside it cannot draw. Removed. Covering that gap
  needs the layer above the view (track C's compositor).
- **Holding `homeAutoLaunch` off while the stub is in place.** Nothing brought
  Flutter Home back in any test, so there was nothing for it to fix, and it
  changes where every wake lands (bootd opens the last input instead of Home).
- **Baking the mod into the hibernation image.** LG's own remake path is
  `snapshot-boot-manager --remove` then `reboot`; the next boot comes up
  `mode=making` and the kernel writes a fresh image to p54 (250 MB
  partition). Safe — worst case one cold boot, as after a firmware update —
  and exercised several times. It cannot carry the mod: the image is captured
  at ~15 s, before the 13 s floor plus a restart can finish, so the resumed
  compositor is always stock and the asset bind is absent. `snap_list` has no
  Home/sam entry, and `-m` from a live desktop would not fit against ~1.2 GB
  in use.
- **Staging the env file on p58's root** so the early `/var` would see it:
  not read (see above).
- **A unit drop-in or a `/run/systemd/system` unit**: `/etc` and `/usr` are
  read-only overlays; `/run` is writable but volatile and nothing early can
  populate it.
- **Making the restart cheaper**: the 12–16 s from restart to pixels is the
  compositor's cold start plus Home's cold start (`surface-manager.sh` kills
  every client on restart), not padding.
- **Shadowing `controllers/StarfishKeyFilter.qml`** to hook `preProcess`:
  never loaded. A marker in the shadow copy stayed silent across restarts and
  the log's own "Created a StarfishKeyFilterLoader object for /usr/lib/qt5/
  qml/KeyFilters/..." line shows the stock file running every time. Types
  reached through the module's qmldir resolve to `/usr/lib/qt5/qml` whatever
  `QML2_IMPORT_PATH` says; only relative-file references from an already-
  shadowed view (`Qt.createComponent("fullscreen/...")`, a Loader's relative
  `source`) follow the shadow. The configd key-filter list is the hook.
- **Stealing QtQuick activeFocus for the Home layer**: keys never arrive;
  the KeyFilter delivers them to the Wayland window, not through the item
  tree.
- **A multi-row app grid from `home.xml`**: appList is a fixed single row and
  a duplicated item is the same list.
- **Rewriting each app's appinfo/icon for tile plates** (the `tileicons.js`
  hack): worked, but touched app files and needed `sam` restarts; replaced by
  in-memory plate derivation in `Dock.qml`.
- **A 4K UI plane**: KMS accepts 3840x2160 but the OSD stream stays 1080p
  (torn lines), +290 MB RAM, capture breaks; the UHD OSD mode is chosen from
  boot `hwopt`, unreachable without a partition write. The OSD scaler index
  (`/sys/module/lg1k/parameters/gfx_o22_scaler_index`) changes nothing
  visible (0 breaks output).

## The other mod on this set

Starfish (`nphil/starfish`, formerly `lg-webos-mqtt`; still installed on the
set under `/var/lib/tvweb`) bind-mounts its own adblock `/etc/hosts`. That
bind stacks on top of Homebrew Channel's, which is how Homebrew's OS-update
block (four LG hosts pointed at loopback) was silently defeated: the TV found
a 1.7 GB firmware update and put its NSU alert up at boot+34 s for 30 s on
every boot. Starfish 0.34.3 carries those hosts whenever
`/var/luna/preferences/webosbrew_block_updates` exists.

tvweb used to offer QML screen saver replacements. They are gone as of tvweb
0.35.0, and the reason is worth keeping because it applies to any webOS 10
set.

The community mechanism (`webosbrew/custom-screensaver` and forks such as
`aabytt/custom-screensaver-aerial`) is a **single-file** bind:

    mount --bind our.qml /usr/palm/applications/com.webos.app.screensaver/qml/main.qml

Nothing else is touched, so the app SAM scanned is unchanged: no type change,
no SAM restart, and a boot hook re-applying it is enough. It works because on
webOS 5 to 23 the stock screen saver is a QML app - exactly the compatibility
range those projects advertise.

webOS 10 rewrote it in Flutter. The app directory holds `appinfo.json` with
`"type": "flutter"`, `data/flutter_assets/` and `lib/libapp.so`, and there is
no `qml/` directory at all, so aerial's own `apply.sh` exits "Target file does
not exist" here. tvweb's screen savers descended from that lineage, which is
why they never ran: every launch failed "Unable to start engine without AOT
data" and exited 85 ms later, and after a few of those tvpower refused all
further requests (`InvalidStatechangeRequest`) until a power cycle - the set
showed no screen saver at all. That, not the compositor widgets, was the cause
of "the screen saver never kicks in".

Rejected alternatives, all tested on the set:

- Binding our own directory over the whole app with a QML `appinfo.json` does
  run, but only after SAM re-reads the app type, which it caches at its boot
  scan (5s) while any writable filesystem arrives at ~13s. `systemctl restart
  sam` forces it and works, at the cost of a dropped Home and an `eim` crash
  whose report makes faultmanager reboot the set at the next power-off. It
  would also need repeating every boot.
- A copy under `/media/developer/apps` or `/media/cryptofs/apps` with the same
  app id is ignored: SAM dedupes by id and the system copy wins. Confirmed on
  two cold boots.

With stock restored, LG's idle detector `com.webos.service.nop` calls
`power2/turnOnScreenSaver` exactly 180 s after the last input and the stock app
stays up. Idle time is 3 minutes; `screenSaverTime` is not a settingsservice
key on this firmware. A Flutter build of our own would be the only native route
and was not attempted.

LumaTV's own saver does not replace that app. While Home is in front it is on
tvpower's screen saver list and answers the idle question with no, so the app
is never started (`docs/research/saver-own-result.md`; on the set the list, the
no on both lists, the idle timer's question about 176 s after the last key and
every 180 s after that, the key that ends it and a power-key standby (the list
keeps the client and the question after the wake is answered no) work; the
Magic Remote's pointer and the video saver are not yet tried).

## Compositor restart side effects still worth knowing

A bare `systemctl restart surface-manager-daemon` on stock firmware, no mod,
makes stock apps segfault on teardown. `faultmanager`'s policy is in
`/etc/faultmanager/faultconf.json`; `getFaultStatus` lists what it has seen.
The mod removes only the reports its own restart produced.

## Before a package or a hand deploy

`python3 tests/qmlhost/compile_all.py` compiles every QML file of the surface with the set's import layout and exits 1 on any syntax,
type, property or import error (a file that does not compile is a layer that never builds: Settings once left the panel on a bare
wallpaper). `tools/build-ipk.sh` runs it and stops on a failure or a missing PyQt5. A hand deploy (sftp of `payload/compositor/lumatv`
into the shadow module) runs it first, then loads each changed surface in the offline host (`tests/qmlhost/qmlhost.py <file>`), which
compiles and runs it and prints any warning; `LS` must come from `import WebOSCompositorBase 1.0` (`tests/test_ls_import.py`).

## Measuring again

- Cold boot on demand: `snapshot-boot-manager --remove; reboot` (expect the
  "making" splash once). Normal reboot: `reboot`.
- Whether the first compositor is stock: its `/proc/<pid>/environ`
  `QML2_IMPORT_PATH`, and `awk '{print int($22/100)}' /proc/<pid>/stat` for
  its start second.
- Panel timeline during boot: a background loop calling `executeOneShot`
  every 2 s from the early shim, recording `writtenBytes`; frames in
  `evidence/boot/` came from that.
- Standby: `ls-monitor -f com.webos.service.tvpower > /var/lib/lumaglass/x.log`
  before `luna://com.webos.service.tvpower/power2/powerOff
  '{"reason":"remoteKey"}'`; wake with a WoL packet; compare `/proc/uptime`
  before and after. SSH survives active standby.

## Research

`research/home-launcher-replacements.md` surveys other rooted-webOS home
replacements and the input-hook tooling around them; it informed the choice
to mod the compositor and Home in place rather than launch over them.

Prior art worth reading before designing anything new:

- `webosbrew/custom-screensaver` and its fork
  `aabytt/custom-screensaver-aerial` — the single-file bind over
  `qml/main.qml`, and the cleanest example of modding a stock app without
  touching what SAM scanned. Its compatibility stopping at webOS 23 is the
  tell that LG changed the app's runtime.
- `webosbrew/webos-homebrew-channel` — the root elevation, the `init.d`
  runner this mod hooks, the failsafe flag, and the `/etc/hosts` update
  block.
- `openlgtv` — firmware archaeology rather than runtime modding:
  `epk2extract` unpacks LG firmware images (useful for reading a stock app or
  binary from a newer release without owning that set), plus buildroot trees
  and `nsu_emu`, an update-server emulator. Nothing needed for this project,
  but it is where to look when a question is "what changed in firmware X".
