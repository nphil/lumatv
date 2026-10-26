# Round 9: the overnight TV run (2026-10-26, TvRound9)

## Round 10/11

### Resume here

Round 11 resumed at 03:47 EDT. Clean package `3b7cceb` (includes `5d3267c`) is on
the set: 120 QML files compiled, 0 failed; package SHA-256
`da345b3bc410320d02768ae5ac4f13e984ed7b56d1e3ec94565d27996e9af59d`. Install
and apply completed at 03:52. Evidence: `round9/r11-2026-10-26/`; historical
diagnosis: `round9/r10-2026-10-26/log.txt`. The old scratch worktree is preserved
at `/tmp/lumatv-slot-r9`; the clean deployment worktree is `/tmp/lumatv-slot`.

Receiver read-back at 03:47: on, 65.5, unmuted, DOLBY DIGITAL, TV Audio. Round 10
restored the TV-speaker profile to 24 and selected external ARC again. Mute before
playback; close the playback app and bring Home forward before restoring unmute.
HDMI switching can change the receiver input and cancel mute.

Next: finish frame-rate probes, Live TV and Search checks, quick/picker gate,
GFN checks, and peer checks. Main confirmed the existing direct TV deployment
authorization; no infrastructure host changes are involved. No SAM restart,
rootfs writes, DRM or PIN probes, or light streams. Layout hash before deployment:
`e194fda89ba7219fb08c389b1c92573db64de9178272cd948c3a41ada336a6a2`. Finish with
Home, Active Standby, LG’s compositor, free decoder ports, and the receiver restored.

HDMI switching cancels mute: volume 0 was used during the HDMI regression. HA’s
`volume_set(0.655)` restored only 65; one TV Volume+ key on Home restored exactly
65.5. The receiver was unmuted on Home, then muted again for the next probes.

| Slice | Result | Evidence |
|---|---|---|
| TV-speaker profile | PASS, Round 10 | 01:39: volume 0 → 24 on `tv_speaker`; output restored to `external_arc`; receiver remained 65.5. |
| Netflix failure diagnosis | CONFIRMED, Round 10 | Old restart with HDMI 2 foreground left `adec.p1 Open=1`, `HDMI_PORT2`, while Home was foreground. Netflix reported `tvq-pb-101`; the decoder was not released with the input app. |
| Input release recovery | PASS, Round 10 | 01:58: `inputs release` launched and closed HDMI 2 through SAM; both decoder ports became `Open=0` in 3 s. |
| Fixed restart | PASS, packaged Round 11 | 03:53: HDMI 2 foreground, `adec.p1 Open=1 Start=1` → restart in 5 s → Home, both ports `Open=0`, no running input app. |
| Netflix after fixed restart | PASS, packaged Round 11 | 03:54–03:55: media service reported `playState=playing` for 20 s, Dolby Vision cinema mode, receiver muted throughout. Netflix closed, Home restored, both ports free. |
| Receiver restoration | PASS, Round 10 and Round 11 read-back | 02:11 restore and 03:47 HA read-back: on, 65.5, unmuted, DOLBY DIGITAL, TV Audio. The two 06:11 log timestamps are UTC and corrected immediately afterward. |
| Dolby signal and text | PASS telemetry and offline render; audible TV check deferred | Actual receiver data: `signal=Dolby Atmos - DD+`, `soundMode=DOLBY ATMOS`, saved as `netflix-receiver.json`. Dark/light offline renders of that reading (only mute disabled in the host) both show the full `Dolby Atmos` text, no warnings. Actual TV format text stays hidden while muted; unmuted TV check remains for the morning. |
| Persistent mute mark | FAIL subtitle clearance; motion works | `netflix-playing.png` and `netflix-mute-hop.png`: the mark moves horizontally after 10 s but covers the Netflix subtitle rows. Reported to Main for the volume owner. |
| Clean package | PASS | 120 QML, 0 failed; installer matched the helper hash after 8 s; apply took 18 s. LG compositor, Home stub, helper and lean profile intact; helper reports 14 apps kept from preloading. |
| Frame-rate probe, first run | INVALID, fixed by Main in `03b6f31` | The sampler stopped on a non-UTF-8 byte in `/var/log/messages`; the old report still called zero samples a pass. That run is not counted. |
| Live TV restyle | PARTIAL; left off | Before: GlobalStyle MD5 and 273 QML files matched. `on` mounted 17 files and set one dump aside; no new failed unit, report, or DSP reset. Input pill, no-signal pill, channel OK banner, channel INFO, and banner editor rendered with Lucent styling; banner setting stayed `none` after Back. Fails: input options focus capture was blank after Right, and the restored-input 1.6 s capture did not show the stock pill. `off` restored the dump and mount. |
| Search restyle | FAIL; left off | `voice on` mounted, but `com.webos.app.voice` pid 8321 survived `closeByAppId`; SAM used `RelaunchAsCommon`, so the patched QML did not load. `voice off` restored the mount. Stock Search reappeared after a later relaunch. |
| Keyboard animation | PASS by capture; left off | `keyboard on` patched one marker and restarted LG’s compositor. The keyboard stood flush at the bottom; after Back it was gone by the next 0.3 s capture. `keyboard off` returned to unpatched. |
| LG keyboard source | PASS | `/usr/lib/maliit/plugins/qml` has 85 QML files. A read-only copy is in `/data/home/lgtv/uiinv/maliit/qml`; the archive is `maliit-qml.tgz`. |
| Frame rate: Netflix 23.976 | PASS | `netflix24-20261026-0409`: source 23.976, decoder 23.97, firmware mute cleared at 14.4 s, no vsync gap; re-reported with `osd_irq` as 120 Hz for the whole run (no switch). |
| Frame rate: 50 fps | FAIL, LG mute on the return switch | `youtube50-20261026-0424` (`v=xX0wOkuLKZM`): a 50 fps segment switched 120 → 100 Hz under the app's own mute, then the 23.976 video switched 100 → 120 Hz with firmware mute while the app's video was unmuted. Earlier YouTube runs were the app home (29.97) or 23.976 content and are not counted. |
| Frame rate: Plex 24 → 25 | OPEN | `plex25after24-20261026-0414` selected two 23.976 items; no 25 fps item was reached by the scripted path. |
| Quick and input picker gate | FAIL | Home and HDMI mostly pass. Failures: quick p90 render 3.70 ms; picker RSS +1.67 MB in 8 cycles; over Netflix quick first frame 1479 ms with no key burst and one >20 ms frame; picker over Netflix lost 2 of 6 markers; both Netflix rows logged LG's `SystemUI/Notification/Services.qml:513` TypeError. |
| GFN read-only and feeder probe | PASS | Media capability files copied; `libNDL_directmedia.so.1`, features 31. No game controller was connected (input names saved). |
| GFN app launch through SAM (`4757348`) | FAIL, fixed by GfnApp4 in `1255efe` | SAM accepted `registerApp`, then the feeder exited 0 about 105 ms later. By hand, the UI lacked the display environment; with it, the UI exported `_Window_Id_15` and failed on wrong core/engine paths. |
| GFN video plane | PASS | 1080p60 H.264 test card: playing in 1 s, 60 fed/s, 0 dropped. Nine HEVC tiers including 4K120 HDR 50/80/100 Mbit/s and declared level 5.1: 120 fed/s, 0 dropped, p99 queue ≤3, no full buffer. 4K120 80 Mbit/s: source 120 Hz, low-delay mode, HDR, `hdrGame`, OSD 119.8/s, feeder CPU 6.4 %. |
| GFN endurance and edge probes | PASS except shared window | G1 4K120 80 Mbit/s for 300 s: 36001 fed, 0 dropped, p99 queue 2, no full buffer. Drop thresholds 0/2/4 and millisecond PTS: no drops. 4:4:4 negative: black, firmware mute 777. AV1 4K plays at 60 Hz. Opus with the receiver muted: 501 packets, 0 dropped; audible check remains. A second feeder playing into another process's exported window ran 60 fed/s but the panel stayed black. |
| GFN SAM lifecycle (`1255efe`, `a2124cb`) | PASS lifecycle; sign-in route FAIL | Registered with SAM; Home minimizes; relaunch restores; close ends the UI in 0.15 s with nothing left. `a2124cb` finds the streamer and chooses writable `/media/developer/temp/org.nphil.lumatv.gfn`, but a signed-out launch opens an `authentication_required` error card instead of device-code sign-in. Reported to GfnApp4. |
| Batched package `6afef85` | PASS after second apply | 126 QML, 0 failed. The first `apply` ran while the installer was still unpacking and kept the 04:07 shadow; the second rebuilt it and restarted LG’s compositor. The round tool now waits until the installed compositor tree matches the package. Restart with HDMI 3 holding `adec.p1` ended HDMI 3 through SAM in 1 s and freed both ports. |
| Mute mark and format text (`00dbca3`) | PASS for Netflix; Home placement FAIL | Netflix: mark top-right, hops from (880,67) to (833,114), clear of subtitles; receiver data `Dolby Atmos - DD+` displays `Dolby Atmos` with the receiver unmuted at volume 0. Still overlapping: the volume bar covers bottom subtitles, and the Home mark touches the clock widget/saver clock. |
| LumaTV saver ownership (`bc28398`) | PASS steps 1–4, 7, 8; pointer open | Both tvpower lists include `org.nphil.lumatv`; questions came 175 s after the last key and every 180 s while idle, each answered no; power stayed `ACTIVE`; no LG saver; one key returned Home by 0.3 s and restarted the idle count. Power key from the saver reached Active Standby; wake returned Home. `own:false` restored LG's saver; `own:true` restored both registrations. The remote pointer cannot be moved remotely. |
| Home mute mark (`404e9b8`) | Placement PASS; startup FAIL | After a mute change it hops along the status-bar row, clear of the widgets. After the 05:10 compositor restart with the receiver already muted, no mark appeared for 30 s until mute was toggled. |
| Search restyle (`81a809d`) | PASS | `voice on` ended resident pid 13642 in 1.9 s; next launch used new pid 14126 with the Lucent pill field and chips; reopen kept the patch; `voice off` ended it in 1.4 s and new pid 14543 showed LG’s look. No new failed unit, crash report, or DSP reset. Left off. |
| Keyboard keys (`5f4704c`, package `1aaf632`) | PASS; left on | Main authorized the keyboard-service restart. Patched keys rendered; D-pad focus and typing worked; `keys off` restored LG's keys in 0.8 s; `keys on` restyled them again in 7.6 s. The overlay appears in 43 app-jail mount paths and `keys off` removed all. |
| GFN signed-out start (`d402b6c`) | PASS | Signed-out launch shows the sign-in code, URL and QR, then polls; no regions error. Each reopen issues a new device challenge. Code captures were not committed. Sign-in and the first stream are for Nitin. |
| Panel re-gate `r12-qp` (`b009de3`) | FAIL | Quick p90 4.23 ms and one >20 ms frame; picker RSS +1.85 MB, HDMI picker RSS +1.82 MB; over Netflix, picker burst end 1541 ms. LG's own Quick Settings over the film: first frame 80.7 ms with no key burst. The film's toast is audio service's Dolby popup; LG's `Services.qml:513` TypeError follows it 20–30 ms later. Picker dev-door RSS rose 624 KB in 12 cycles. |
| Settings read-only replies | Saved | `settings-rest4-reads.txt`; page walk not run. `ppm/queryPermissions '{}'` returned invalid JSON; Bluetooth balancing reported no device; sound cards list is empty; connection status includes `isInternetConnectionAvailable:true`. |

## Round 9 handoff (historical)

Started 00:00 EDT. On the set: the ipk of `d57f2ce` (clean worktree `/tmp/lumatv-slot`, compile gate 117 QML, 0 failed;
install 4 s, `apply` 23 s, one compositor restart, every `[Resident] * built` line, Home in front at 00:01). Evidence in
`docs/research/round9/tv-2026-10-26/` (`log.txt` the timeline, captures as PNG). Receiver: found on, 65.5, not muted,
DOLBY DIGITAL, TV Audio; set to 0.10 for the compositor gate (the Denon cancels its mute on a volume key), 0.655 to put back.

Done: the compositor's eighth gate (FAIL on x1, netflix, wd-crash; LG's compositor live, flag off; results table).

Next, in order: autoinstall standby/wake then one reboot; Dolby format text and the mute cue; the frame-rate table; the
voice-app QML copy and E3; the peers' commits (RoundFixes, VideoPlaneSaver, LiveTvRestyle, SettingsRest2) in batches with a
quick/picker re-gate. End state per the rules: Home in front, Active Standby, helper + Home stub + lean + preload trim intact,
LumaTV's compositor live only if the full gate passed, receiver 0.655 unmuted.

## Round 9 results

(filled in as each slice ends)

| Slice | Result | Evidence and numbers |
|---|---|---|
| Deploy `d57f2ce` | PASS, two startup warnings | dev/install 4 s, `apply` 23 s; `[Resident]` splash, banner, voice, search, drawer, saver, picker, edit, settings, quick, menu, confirm, remote, overlay banner/drawer/volume, apps, overlay quick panel, overlay input picker all built; no `is not defined`. New at start: `panels/AppsGrid.qml:171: TypeError: Value is undefined and could not be converted to an object` (`page.rail[parent.at].sub` before the rail exists) and two `[Dock] launchPointsList parse failed: SyntaxError` (SAM not answering yet); LG's own `InputPicker.qml:35` and `StarfishViewAdjustments.qml:50` as before |
| Compositor gate 8 (`slot-2026-10-26-0003`, 00:03-00:13, `--enable-after` armed) | FAIL, LG's stays | firmware, homeheld, live, volume (0.10 → 0.105 → 0.10), hdmi, standby/wake (PreparePowerOff 1, PreparePowerOn 1, no PIN), homekey, wd-hang, escape PASS. **x1 FAIL**: the client's own swaps 0 over 20 ms (3599 frames, 59.97 fps, worst 19.7 ms, 70 in 17.5-20 ms; the best run yet) but the compositor's log has `frame gap 20.4 ms on surface 185 (x1) after 1516 steady frames` and a `page-flip stamp 33.37 ms` right after `tick: frame held while a driver buffer warms` (`gate/x1-compositor.txt`). **netflix FAIL**: the deep link opens to Netflix's error `tvq-pb-101 (5.2.8)` ("issue with your streaming device"), no video window, Picture Mode stays filmMaker (`gate/netflix.png`); the same screen as slot 1606 (163003 bytes) while slot 1730 played Dolby Vision: intermittent under LumaTV's compositor, never seen under LG's. **wd-crash FAIL on the reason string only**: fallback in 0.5 s, failures 0 → 1, LG's back, SAM unchanged, but `last-fallback` reads `compositor closed the bridge socket` instead of `killed by signal 9` (the supervisor sees the socket close before it reaps the child). Trial: 2 loop passes over 20 ms (`loop: 23.2 ms in one pass (tick)`) |
