# Round 8: the TV run and the slot script

## Resume here (TV round 8, 2026-10-25 22:55-23:55 EDT, TvRound8)

Stopped at 23:55 on the request budget, the set in Active Standby on LG's compositor, Home in front, the receiver as found (on,
65.5, not muted, DOLBY DIGITAL, TV Audio), Nitin's layout.json back byte for byte, theme as found (screensaver clock, video off),
the video-plane player removed, WOL on, backlight 80, nothing left in /tmp. On the set: the ipk of `d4b7ca7` (clean worktree
build; compile gate 117 QML, 0 failed), with two files put in by hand after it: `payload/vplane/role.json` and `vplane.sh` of
`8f3575c`, and the `lumatv-jpeg` of `591c720` staged in `/var/lib/lumatv/bin` (the next `apply` restages the same).
Evidence: `docs/research/round8/tv-2026-10-25/` (`log.txt` is the timeline; captures as JPEG; `gate1.json` the frame gate;
`light-test.txt`; `vplane-*.json`). Results table at the end ("Round 8 results").

Not done, in this order for the next slot (each needs the set):

1. Redeploy from HEAD (after `c422b7a`): my fixes `4c4a828` (volume text), `8f3575c` (vplane role and runtime dir), `591c720`
   (jpeg PmLog) and VolumeFix's `76ed1a3` are not in the installed ipk. `sh /tmp/r8/build.sh <sha>` is the clean build
   (worktree at /tmp/lumatv-slot, cargo caches copied, jpeg and vplane rebuilt), `python3 /tmp/r8/install.py` the install.
2. The compositor's eighth gate: `python3 compositor/tools/tv_next_slot.py --no-standby` from the worktree. The Denon cancels its
   mute on a volume key (seen 23:03), so set its volume low (`media_player.volume_set` 0.10) before the gate's volume check and
   put 0.655 back after; Nitin's decision: leave LumaTV's compositor live only if every check passes.
3. Autoinstall: one standby and wake (`grep AUTO_APPUPDATE\|APP_INSTALL_REQ /var/log/messages`), then one reboot (approved):
   Home stub, helper, padguard, volume patch, `declutter.sh status` (autoinstall 8/8), the boot logo.
4. The Dolby format text on the bar (needs the new helper and one muted Netflix stream), the voice-app QML copy LiveTvRestyle
   asked for (`/usr/palm/applications/com.webos.app.voice/qml`, read-only tar to `voice-qml/`), E3 (a static arm64 ffmpeg is at
   `/tmp/r8/e3/ffmpeg-7.0.2-arm64-static/ffmpeg` with `clip_1080p60_444.hevc` beside it), the frame-rate matching table (lowest).
5. The Python suite of the clean worktree hit its 3600 s limit without a summary (`/tmp/r8/pytests.log`, at least five `F` marks;
   the QML-host tests ran serially for the whole hour) and the helper's cargo tests never started: run them in the morning on a quiet
   workstation (`cd /tmp/lumatv-slot && python3 -m unittest discover -s tests -p 'test_*.py'`; `cd helper && cargo test --workspace`).

## The slot script (written 2026-10-25 afternoon, offline)

`docs/research/round8/tv_round8.py` is written and tested offline (`tests/test_round8.py`, 43 tests against a simulated set); it has not
run on the set. It needs Main's go for the set and Nitin's yes for the test plugin.

```
python3 docs/research/round8/tv_round8.py --plan                 # the steps; touches nothing
python3 docs/research/round8/tv_round8.py --go --approve-test-plugin
python3 docs/research/round8/tv_round8.py --go --skip x4,logo    # any of plugin,x4,maliit,logo
```

Other flags: `--no-standby`, `--keep-logo-off`, `--plex-keys "OK s4|OK s5"` (kb.py tokens, stages split by `|`), `--plex-seconds 20`,
`--out DIR`. About 20 minutes; paramiko and the homelab's Home Assistant token as in `tv_next_slot.py`. Evidence goes to
`docs/research/round8/<date>-<time>/` (`summary.txt` first); the summary lines are PASS, FAIL, INFO or SKIP and exit status 1 means a FAIL or a
PROBLEM (something that could not be put back, printed last).

Before the slot: the set's package must be the current tree, applied (`lumatv apply`): LumaHome live, the helper with the plugin broker and
`/v1/device`, `payload/systemui/MasterBrand` with the `logo-events` record. The script checks the first and the last and says so; a
helper without the broker shows as "the helper switched it on" failing.

## What the script does

Preflight (read-only apart from waking the set): it answers and is Active, nothing of another slot runs (`ls-monitor`, X1/X2 clients, the
compositor supervisor, `slot.sh`, `kb.py`, the card app, plugin scripts; the ambient probe's lock or `vt_devmode`), Home is in front. A set it
woke is put back to sleep if the run is refused. `/tmp/round8.lock` on the set keeps two runs apart. Then the receiver is muted through Home
Assistant and read back from the helper's direct reading (Home Assistant's entity is stale after an unmute, so it is only the fallback); if
that cannot be shown, nothing that plays or cycles power runs.

| Step | What it does | PASS means |
|---|---|---|
| plugin | `00-home` baseline. Test plugin (below) on. Lane screenshot, then UP/OK/OK from the dock's first tile (Home's focus path into the At-home lane) to run the chip's action. A note posted, toast screenshot. Quick panel by the door `quick` (screenshot, tile action through `POST /v1/plugins/round8/actions/ping`, screenshot). Settings by the door `settings`, UP x10 then DOWN x4 to the fifth rail section (Ambient light), screenshot, the helper's `base.ambient` and `/v1/ambient`. Drawer by the door `drawer`, screenshot. Each surface has the compositor's log since it was asked for, checked with `gate.py`'s failure patterns | each surface differs from Home at rest by more than its floor (3 %, 10 % for Settings) and is not one flat colour; the plugin's card, tiles and both notes are helper topics; the plugin was told of both actions; no failure line in any log window |
| maliit | `find`, `server.conf`, `ls -R`, the process's maps, strings of `libplugin-global.so`; every QML/JS/qmldir/json file under a path with `maliit` tarred and copied to `maliit/tree/` with md5sums | the tree is copied; with no QML on disk it is an INFO that the keys are compiled in (a rebuilt resource is Nitin's decision, `lg-restyles-result.md`), not a FAIL |
| x4 | Each segment under its own `ls-monitor -j -e` (the repo's `x4-monitor.sh`, its capture moved to `/tmp/round8/x4`), gzipped, with `x4-analyse.py`'s census beside it. Picker: `showInputPicker`, screenshot, `systemUIManager/closeByAppId`. PlxNative: launch, the keys, up to three stages, playback held, closed, Home. Standby/wake: `powerOff`, Active Standby, 20 s, wake. No PIN, no Netflix | both picker calls in the capture and Home after; PlxNative in front, a `setDisplayWindow`/video window by it, closed and Home seen before the capture ends; the standby rounds `request power off`, `prepare power off`, `prepare power on` pushed to the compositor's subscription |
| logo | `lumatv logo on` first if it was off (before any power-off of the run; put back at the end), then the standby cycle's `logo-events` lines | `power-off logo N launch` and `power-off answered tvpowerd` are new lines; INFO gives the gap and the `[Logo]` lines. It reuses the x4 standby cycle; alone it runs its own |

End, always, in this order: monitor stopped, test plugin off and the roster put back byte for byte, PlxNative closed, Home in front and seen, PlxNative
not in SAM's running list, logo switch back, the receiver unmuted (not if Home or the quiet is not confirmed: that is a PROBLEM and it
stays muted), `/tmp/round8` removed, Active Standby. Picture Mode is read before and after, never set.

## The test plugin (Principle 0)

Switching a plugin on in the helper's roster is installing software the TV runs, so it is the owner's act: without `--approve-test-plugin` the
step draws only what the helper's own plugins give (the ambient tile) and the lane and drawer show their empty state. With it, the script adds
`round8` to `<data>/plugins/enabled.json` (the file as it was is kept and put back; a roster that did not exist is removed again), starts
`plugin/plugin.py` as plugin user 20000+n (the first one the roster does not use) with `os.setuid`, and takes both away at the end. The plugin has
cards, tiles and notify permissions only, no network, no leases; it speaks the real wire (every message it sends parses as the plugin-api crate's
own types, checked once with a throwaway Rust program, and `tests/test_round8.py` runs it against a stand-in broker).

The approval is the SHA-256 of the manifest's consent document, rebuilt in Python (`consent_document`, permissions in the library's enum order,
not the author's) and pinned in the test to what LumaShow's crate gives: `8bb95c0e...97d5`. If `plugin.json` changes, recompute it with
`plugin_api::Manifest::parse(bytes)?.consent_document()` and update `RUST_CONSENT` and `RUST_APPROVAL` in the test.

## Not known until the first run

- **PlxNative's way to playback.** Nothing recorded says what its first screen offers. The default keys are OK, wait, OK, wait, OK (three
  stages, each checked for the video window); `after-keys-N.png` shows what each opened, `--plex-keys` changes them. A FAIL here is the keys,
  not the capture. Playback is judged by the live `getForegroundVideoWindow` naming the app or by a placement call from it in the capture.
- **The lane's focus path** (UP, OK, OK from the dock's first tile) is read from `LumaHome.qml`'s key code; `01b-lane-focus.png` shows where the focus went.
- **The door**: a layer named in `/tmp/lumatv-layer` is picked up by a 2 s poll (3.2 s waited). The `settings` door has to be emptied to close (Back
  leaves the door's own state); the script does it. A reboot clears the file.
- **Screenshot floors** (3 % and 10 %) are chosen, not measured; the numbers are in each line so the first run calibrates them.
- Toasts for plugin notes, the lane's look and `01-lane.png` against the mock are by eye (INFO lines give the pixel share).
- The compositor's own sender side during standby (pass 2 saw none of its pushes after a restart) is counted and reported, not asserted.

## Files

`docs/research/round8/{tv_round8.py, plugin/plugin.py, plugin/plugin.json}`, `tests/test_round8.py`.

CHANGELOG line for the main agent: *Round 8's TV slot script (`docs/research/round8/tv_round8.py`): plugin drawing (lane, quick-panel tiles, the ambient page, the drawer) through a test plugin on the helper's plugin API and Home's development door, the X4 remainder (input picker, PlxNative playback, standby and wake), the maliit tree copied read-only, and the power-off logo record, with the receiver muted before anything plays and put back after Home is; tested against a simulated set.*

## Round 8 results (2026-10-25 22:55-23:55, the ipk of `d4b7ca7` on LG's compositor; evidence in `round8/tv-2026-10-25/`)

| Slice | Result | Evidence and numbers |
|---|---|---|
| Install and apply | PASS | dev/install 7 s, `apply` 24 s, one compositor restart; `[Resident] settings built`, apps, overlay banner/drawer/volume/quick panel/input picker built; no `is not defined`, no syntax error; one `TypeError` is LG's own `InputPicker.qml:35` at its preload-then-close on every start (`01-home-after-apply`) |
| (a) Volume bar | PASS, one defect fixed | Bottom-centre pill over Home, Plex and HDMI 2; 65.5 → 66 → 65.5; one `volume view is not available` and one `[VolumeOsd] cue` per press; key to cue 4-11 ms (`02`-`14-*`); hold 1 s: 69.5 by the keys, settles on the Denon's 68.5; mute: slashed speaker, bubble hops within 14 s, unmute removes it (`06`-`10`); LG's popup never. Format text elided (`PCM · Surrou…`, `PC…` on HDMI 2): fixed in `4c4a828`. The mute key's report has no `cause`; the Denon cancels its mute on a volume key: VolumeFix's `76ed1a3`. Format text with a Dolby stream: not run |
| (b) Settings | PASS, notes | Every rail page draws (`20-*`); Picture tabs hold LG's values (service read: contrast 85, brightness 50, backlight 80, sharpness 10, colour 50, tint 0, temp −50, gamma high2 = BT.1886, all `off` switches off, realCinema on); held OK opens the help sheet, Back closes only it (`22-*`); slider hold: 80 → 95 in 1.5 s (acceleration), put back to 80; Sound Mode and Advanced, Home & look tabs, Home Assistant page (`30-*`), System tabs, Closed Caption (4 tabs), Time & Date (rows dim "Set Automatically is on"), HDMI tab, Bluetooth Devices, Join Network ("No answer from the Wi-Fi", no crash), IP Settings (`31`-`38-*`). Defects: the four `4:4:4 Pass Through` rows carry no port number; Bluetooth lists three devices named "Not Paired"; the Time & Date row's value is the page's build time (11:15 PM at 11:34); the Vision group label scrolls off; Back on the rail does not close Settings while the door file is set (by design). A slider row takes Left/Right as its value (Wake from the network was toggled by an OK that landed on it and put back) |
| (c) Visual QA | PASS | Mode cards whole, four across, the strip moves by whole cards; words do not move on focus (`21-*`); rail reads "Picture / Filmmaker"; no "Not available on this TV". Open: the Apps library's rail wraps its second lines into the next row and its bar clock stays at the build time (10:57 PM at 11:28) (`45-apps-grid`) |
| (d) Gear tap and Input key | PASS on screen, gate FAIL | Gear tap opens LumaTV's quick panel left-docked (`40-*`), Back closes; Input key is evdev **241** (`Key_webOS_InputSource` in the log) and opens the picker along the bottom (`41-*`). `gate.py --surfaces quick,picker --input-key 241` (`gate1.json`): quick first frame 42/125/25 ms, burst end 192-242 ms, p90 3.1-3.7 ms, 2 bursts with a >20 ms gap; picker first frame 7 ms, close burst 375 ms, p90 2.6-3.4, 3 gaps. Bars: 50 / 180 / 3 ms / 0 |
| (e) Apps grid | PASS | OK on the Apps tile opens the library (23 installed), focus moves, Back returns to the tile (`45-*`); no `LS is not defined` |
| (f) HA lane | PASS | One chip "Home Assistant / Not switched on"; OK opens Settings at the page (address Not set, token Not saved); `plugin-status` enabled [], consent `bd302afc…`, program 995,848 B mode 755 |
| (g) Screensaver | GPU: not visible under LG; video plane: plays, panel black | LG's `com.webos.app.screensaver` (flutter) draws its clock over LumaTV's saver layer in the Screen Saver state (`50`, `54`); `turnOnScreenSaver` + measure: Mali 21.9 %, 60 flips/s. Player: hub refused its nameless handle and the AV connector needed `XDG_RUNTIME_DIR` (fixed, `8f3575c`); `--once great-wave 20` in the Active state shows the print, caption and clock, Home stays foreground (`57-*`; the window is transparent outside the 1856×1024 canvas, the clock reads 23:39); the saver path plays (status playing, 60 units/s, LG's saver evicted, Mali 1.9 %, flips 12.3/s, vsync 121.7/s, player 3.6 % CPU, 25 MB) but DISPLAY and GRAPHIC captures are black while the VIDEO capture has the print (`58-*`): [INFERENCE] LG blanks the output in its Screen Saver power state. A key wakes to Home in about 1 s; MemAvailable 772 → 739 MB. Removed, switch off, style clock |
| (h) Hardware JPEG | PASS after one fix | `libdile_photo` needs PmLog loaded first (`591c720`). Block: 1080p 13.1 ms decode + 16.9 ms NEON convert, 4K 47 + 68 ms, 4K 4:4:4 64 + 70 ms (pixfmt 8), 4K at 1/2 35.6 ms; refused: 1003×707 and the progressive file (status 5). Helper route `/v1/image/images/a.jpg?w=1920&h=1080`: `X-Luma-Decoder: hardware`, 32.6 ms decode, 82-88 ms a request against 158 ms on the CPU; art prints (1280×860) stay on the CPU |
| (i) BLE passive scan | PASS | `le/startScan {"subscribe":true}` answers `subscribed`; 15 s: 30 reply lines, 28 `scanRecord`s, 17 addresses (file not kept); ping during the scan 60/60, 0.218 ms average (0.175 before; the set is wired); no `luna-send` left; no "listener dropped" line found; Magic Remote still connected |
| (j) Light sync | Run, side effect | 500 frames at 20.0 fps and 100 at 10 fps through HA `mqtt.publish` with the helper's exact frames; Z2M reports nothing for them; the IR camera sees no change; the bulb back as found (OFF, 1, 366) each time. First run saturated the coordinator mid-OTA: the leak sensor's OTA aborted (`z2m-direct-result.md`, "One stream"); Nitin restarts it |
| (k) Widget editor | PASS, one defect | Hold OK on Weather opens the grid (`65-*`); move left refused with "Clock is in the way" (Nitin's layout); a column narrower then the bar's Save writes `grid.rows 16` and the cards (`69-*`); **OK on "Save" in the "Save changes?" prompt saves nothing and keeps editing** (`68-*`); `layout reset` gives `{}`; Nitin's file put back, equal to the copy |
| (m) Home gate | PASS but one gap | home-idle p90 0.16 ms; home-dpad first frame 23 ms, p90 2.77, dropped 1 of 82 (1.2 %); home-sweep first 25 ms, p90 2.51, dropped 0, one burst gap > 20 ms (FAIL on that check). Morning baseline 3.9-4.4 % drops |
| LG's Settings app | PASS | Not preloaded (no process before); a web app of 65 MB RSS while open; Back closes it and it leaves SAM's running list (`63-*`) |
| Compositor gate, autoinstall standby/reboot, second deploy, GFN E1-E3, frame-rate table | NOT RUN | E1/E2 need the NDL probe app, which does not exist; the rest fell to the budget (see Resume here) |

Fixes made tonight: `4c4a828` (volume text width), `717e935` (Zigbee2MQTT finding), `8f3575c` (vplane role and runtime dir),
`591c720` (jpeg PmLog). Open failures and owners: the quick panel's and picker's frame bars (ShellHost), the editor prompt's Save
(WidgetEditor), the Apps library's rail wrap and stale clock (VisualQA/Apps.qml), the Settings row names and the stale Time & Date
(SettingsCoverage), the player's margins and the Screen Saver power state (VideoPlaneSaver), the mute key's cue (VolumeFix,
`76ed1a3`, unverified on the set).

CHANGELOG lines: *Volume display: the pill measures its words as drawn (the format text was cut on the set).* *Video-plane saver:
the player's role carries every name LG's media library registers and the supervisor hands it the compositor's runtime dir.*
*Hardware JPEG: PmLog is loaded before LG's photo library; the helper's picture route decodes on the block (4K to 1080p in 33 ms).*
