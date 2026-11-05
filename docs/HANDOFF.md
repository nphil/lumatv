# LumaGlass — engineering handoff

Everything a person or model needs to pick this project up cold: how the mod
works, what was measured on the set, which approaches are dead and why, and
how to measure again. Facts here were observed on one set unless marked
[INFERENCE]. Credentials and the set's address are deliberately absent; they
live with the owner.

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

The current saver source is rustui/ui/src/system/saver, with system/layer.rs and shell.rs owning presentation/input dispatch. LG option.screenSaverTimer remains the only start deadline. Native Screen Saver state is approved by LG's idle/media policy; a tvpower vote question alone is not approval. Home-owned negative voting stays Home-only; an app saver must wait for LG-confirmed Screen Saver so native playback/game vetoes remain authoritative.

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
  `lumatv inputs status|release`, and gate.py now closes the HDMI app it opens.
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
