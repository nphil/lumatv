# Notifications and logos (NotifySlice)

## Resume here

Built and tested offline: the toast, the alert, the input popup and the PIN prompt in the Luma look on top of LG's
notification view, their scenes (dark and light), and the boot and power-off logo choice. On the set: two restarts for
the power-on logo. The first (2026-10-25 16:34, "Restart on the set") was from Active Standby and answered nothing. The
second (17:50, "Restart from a set that is on", BootPathFix's tools deployed first) verified the boot path: the stub in
place at 15 s, the guard waiting, `apply` 38 s (188 s before), Home in front with `faults 0`, nothing given back to LG;
the logo itself is still unseen: the boot's panel capture starts at 14.6 s and the first lit frame (16.8 s) is already
the HDMI 2 input banner, and the log keeps nothing before 12.7 s, so whatever logo there was came and went before
either could see it, and no `[Logo]` line survives.
Next, in order:

1. `lumatv notifications on` on the set (stages LG's tree, patches it, overlays `payload/notification`), then
   `python3 docs/research/perf/gate.py --surfaces toast,alert` with Home on screen (see "Set steps").
2. A PIN prompt on the set with Nitin present (a locked input), see "Set steps".
3. The logo: a capture that starts before 12 s (the boot hook cannot: it runs at ~14 s), or a log that keeps the
   compositor's first seconds (`[Logo]` goes to the QML console; where that lands before PmLog starts at 12.7 s is
   unread), or a person in the room at a power-on. The power-off logo (`logo.off`) is untested the same way.
   Also to settle: the set restarted into the last input (HDMI 2, selected by the compositor gate before), not Home.

## What changed

LG's `SystemUI/Notification` tree stays the contract and the data layer; `tools/notification_patch.py` patches a staged
copy and lays `payload/notification/*.qml` over it (same-named files replace LG's; a missing original is fatal).

| Surface | Mock | Files | Notes |
|---|---|---|---|
| Toast | 07-toast | `Toast.qml` | glass pill at the top right; title over subtitle; accent link. The icon rule is now exact: an icon is replaced by LumaTV's mark only when empty or when the file name (no folder, no extension) is `info`, `notice` or `default`; an app icon in a folder or app id containing those words is kept. The three names are the old pattern's words and are **not verified against LG's real default icon file on the set**: capture the `iconPath` of a default advanced toast and put the real name in `Toast.qml` (`genericIcon`). |
| Alert | 06 (LG view's own) | `Alert.qml` | card in the middle: app icon tile, title, message, buttons in a row (LG stacks them in a column and shows no title); a button with `danger: true` is drawn in the danger colour; full alert (`type: warning`) is full screen; check box, progress, spinner and countdown kept. Focus, TTS, voice control, discard timer are LG's behaviour. No `layer.enabled`, DropShadow or ConicalGradient. |
| Input popup | 18-device-connected | `AdvancedAlertDelegate.qml` + patches to `NotificationView.qml` (placement, scrim) and `AdvancedAlertView.qml` (slides in from the top) | glass sheet at the toast corner: icon, "NAME connected", "Switch to it?", a Switch button per input, Stay here, a bar that runs out with the timer; no black scrim; closes by itself without switching. LG's data carries only the port name and icon, so the mock's "turned on" and "The Denon follows" are not drawn. |
| PIN prompt | 19-pin-prompt | `PincodePrompt.qml`, `PincodePromptNumButton.qml`, `PincodePromptPasswordButton.qml` + `PincodePromptView.qml` patch | card over the dimmed screen, 3x4 pad (1-9, Clear 0 Back) and an OK key that is live only with four digits and takes the focus then (LG's rule); number keys and Backspace/Back work as in LG; secret (arrow-only) mode kept; retry message in the danger colour; a fresh `password` per confirm (LG appended). The answer leaves only through LG's `closePincodePrompt`; nothing here listens to anything but keys and pointer clicks. |

Never filtered: nothing in the overlay drops or delays an alert or toast; EAS, thermal shutdown and Magic Remote battery
arrive as alerts/toasts like any other (`test_a_warning_alert_is_drawn_whole_and_never_filtered`).

`tools/notification_patch.py`: palette now needs `sheetTop`, `radius`, `accentInk`, `danger` as well (tokens.py carries them,
62b72ee). New anchored patches: input popup placement and scrim, popup slide direction, PIN page background and rise. Each
fails loudly when LG's source changes.

Logos: the six options (`payload/compositor/lumatv/logo/`, windows in `payload/systemui/{BootLogo,MasterBrand}`) were already
built; stills in `docs/mocks/logo` regenerate byte-identical from the current QML. Choice keys are `logo.on` and
`logo.off`, 1 to 3, anything else is 1 (SettingsSlice's rows write them; titles offered: Panes, Arc, Name only). Each window
now logs `[Logo] power-on logo N` / `[Logo] power-off logo N` when it starts, and a test pins the choice for 1, 2, 3 and invalid values.

## Scenes

`tests/qmlhost/scenes_notify.py` registers (runner scenes, dark and light): `07-toast`, `06-alert`, `06-alert-warning`,
`18-device-connected`, `19-pin-prompt`, `19-pin-retry`. Each runs the patched view through `tests/qmlhost/notify.py` over a
render of Home in the same material. "Light" stages the view with the light material as its palette; on the set the sheet is
always the dark material's (`tokens.py`), so light shows the same files in a theme with a light sheet.
`python3 tests/qmlhost/scenes.py --only 07-toast,06-alert,06-alert-warning,18-device-connected,19-pin-prompt,19-pin-retry`
writes `docs/mocks/host/<name>.png` and `<name>-light.png` (untracked).

`notify.py` gained `--light`, `--probe MS:OBJECT:PROP`, `--calls`, digit and Backspace keys; the stub service answers
`callForApplication`. Two LG removal-time warnings (stock tree too) are ignored by name.

## Tests

- `python3 -m unittest tests.test_notification_patch` (35): patch anchors, overlay interface, palette, and in the host: alert keys
  call each button's own action, a warning alert is drawn whole, the PIN goes out only with four digits and OK, Back on an empty PIN
  closes with no PIN, the pad's direction keys, the input popup switches on OK and stays on the other button, exact generic-icon rule.
- `python3 -m unittest tests.test_logos` (7).
- Changed existing tests, with reason: the alert-placement and shadow tests now run `overlay=None` (the overlay replaces those
  files; the patch on LG's own is still tested); "changes nothing but what it names" lists the new overlay files; the palette-refusal
  test names the missing key generically and the new `danger`.

## Set steps (not verified on the TV)

Toast and alert gate (LG's own was the baseline; the rebuilt must meet the PLAN numbers):
1. `lumatv notifications on`; `lumatv notifications status` shows on; `lumatv log` has no `notification_patch:` error (if it fell back to LG's view the log says so).
2. Home on screen. `python3 docs/research/perf/gate.py --surfaces toast,alert` (kb.py opens them with luna `createToast` / `createAlert`, the
   alert is closed with OK on its first button; the toast by its own timer). Pass: first frame within 50 ms of `luna-send` returning, p90 render
   under 3 ms, no ShaderEffectSource rebuild, RSS flat over the cycles. Keep the JSON beside the earlier LG baseline.
3. Eyes on the panel: toast top right under the bar; alert card centred with the title above the message.

PIN (Nitin present; a security screen): lock an input in LG settings, select it. Check: 0-9 on the remote, Back erases, Back on empty closes,
OK dead before four digits, OK sends, a wrong PIN shows the red message, three wrong closes (LG's rule). `lumatv notifications off` is the way back.

Input popup: plug or power a device on an HDMI port; Switch focused and OK launches that input, Stay here (or waiting 10 s) leaves it.

Cold-boot logo check: `lumatv logo on`, pick the styles in Settings > Look > Standby (or `lumatv theme set '{"logo":{"on":2,"off":3}}'`), `reboot`.
Watch the panel from power-on: the chosen power-on logo, then Home with no flash of LG's red logo. After boot, `grep '\[Logo\]'` in the compositor
log must show `power-on logo 2`; turn the TV off and the log shows `power-off logo 3` and the TV goes dark within the logo's time (about 0.5 s).
A logo that fails to load answers tvpowerd at once (tested), so the worst case is a black screen for the old logo's time, not a stuck boot.

## Restart on the set (2026-10-25 16:34-16:48, `docs/research/notify/boot-2026-10-25-1634/`)

`logo on` was in place (`lumatv surfaces`: `logos` on; `theme.json` has no `logo` key, so option 1 of each). A plain
`reboot` over SSH at 16:34:00 with the set in Active Standby. What happened, from `/var/lib/lumatv/log` and
`/var/log/messages*` (all rotated files of the boot kept their lines):

- The set booted into Active Standby (panel off) and stayed there: `getPowerState` `ActiveStandby` at uptime 729 s,
  `/sys/power/mode` `resume`. From standby a `reboot` comes back in standby; HANDOFF's "returns powered on" is from
  reboots of a set that was on. The one panel capture (uptime 122 s, 480x270) is black; the other 468 captures in the
  loop came back as empty files, each call answering in about 0.1 s (not read why).
- No `[Logo]` line in any log of this boot (`messages.4.gz` starts at uptime 13 s). With the panel off there was
  nothing for BootLogo to draw on, and LG's own `com.webos.app.bootlogo` was closed by Flutter Home at uptime 212 s
  (`closeAnimation bootlogo`), so whether LumaTV's QML logs to `/var/log/messages` at all is still unread.
- The boot path: `homestub: stub in place at uptime 15 s`, `compositor-boot {"compositor":"off"}` (LG's compositor,
  no `enabled`), `boot-bind` 1 mount, `apply` at 16:34:40 took 188 s and restarted LG's compositor at 16:37:05 to read
  the shadow module; then `home up after 0 relaunch attempt(s), panel not presenting` and the stub guard:
  `LumaHome's heartbeat stopped (30 s, file 1792960512 s old), giving LG's Home back`, `fault 1 of 2`. The heartbeat
  file was never written in this boot (its age is the epoch), so after the compositor restart the stub never beat with
  the panel off, the guard gave LG's Flutter Home back, and the set sat in standby with `homestub` `state lg, faults 1`.
  Woken at 16:47 (screen on, LG's Home in front), `lumatv homestub on` put the stub back in 6 s (`state stub, mounted,
  sam web, guard, beat true, faults 0`). Everything else came back by itself: the helper (pid 6933, healthy on 18731),
  `lean` (`nudge.service` masked), the preload trim (`preload status` on, backup present), Picture Mode `filmMaker`,
  LG's compositor live with no `enabled` flag, no `vt_devmode`. For the tool's owner: with the panel off the stub cannot
  beat, so the guard's 30 s rule after a boot-time compositor restart hands LG's Home back on every reboot into standby;
  the guard should wait for the panel (or `apply` should not restart the compositor on a boot whose shadow is current).
- The receiver turned off with the set and on again at the wake, as before.

## Boot from standby (BootPathFix, offline, not verified on the set)

Cause of the 188 s apply: `apply` runs `preload.py on`, which sent one `setPreloadPolicy` per app (14 ids). With the panel off
the preload manager holds each call until `luna-send` is killed at 10 s: 14 x 10 s = 140 s, and the compositor restart came
145 s after the apply started (16:34:40 to 16:37:05). The 30 s `READY_WAIT` for pixels after the restart and the Home launch,
on a panel that stays off, was the rest (188 - 145 = 43 s).

- `preload.py on` reads the power state; unless it is `Active` it writes the list and skips the live calls (`deferred` names
  the ids, `ok` stays true). The helper already sends the policy at every screen-on from `preload.json`. A failed list read
  or write is still an error.
- `wait_presenting` on the boot path (`LUMA_BOOT=1`) gives up after `STANDBY_WAIT` (5 s) when no pixels came and the power
  state is not `Active`; `relaunch_home` then logs `home up ..., panel not presenting` as before. With the panel on, or
  off the boot path, the wait is `READY_WAIT` as before.
- The guard (`homestub-watch`): when the beat is stale past the grace, it asks `panel_is_active` first. Panel off: no fault,
  no fallback, one log line (`no beat and the panel is off, waiting for the screen`), the beat is re-read every 5 s. At the
  first screen-on (`screen on, waiting for the beat`) the grace of 30 s starts again; if LumaHome still does not beat, the
  fault and LG's Home come as before (fail-open kept, fault 1 of 2). A beat clears the dark state. Unchanged: the
  `error` marker (LumaHome could not load) and the compositor restarted without the module are faults with the panel off too.
- The apply still restarts the compositor in standby (the shadow module has to be read; an abandoned restart once left a
  whole session on stock QML). Why LumaHome never wrote its beat with the panel off after that restart is not known (the
  1 s timer in `LumaHome.qml` has no panel condition; the container may not be built without a surface); the guard no
  longer needs the answer, but whether the stub beats in standby is a TV question below.
- Tests: `tests.test_preload` (standby defers, screen on sends, a failed list write still fails), `tests.test_tool_rules`
  (`GuardWaitsForThePanel`: the guard run against a stub: panel off no fault, fresh grace at the screen-on, beat keeps the stub,
  panel on a stopped beat is a fault, load error is a fault with the panel off; `WaitForPixelsOnTheBootPath`; `BootCapture`).

## Set steps for the boot path and the logo (not verified on the TV)

The logo is up about 14-30 s into a boot and sshd answers on the new system only at about 46 s, so the 16:34 loop could not
have seen it even from a set that was on. New: `tools/bootcap.sh`, started by the boot hook at ~14 s when
`/var/lib/lumatv/bootcap` holds a number of seconds (the flag is removed by that boot); frames and an uptime index land in
`/tmp/lumatv-bootcap`. `docs/research/notify/reboot_logo.py` sets the flag, reboots, retries SSH until the new system answers
(uptime below the old one), waits for the capture's `done`, fetches the frames and the `[Logo]` lines of every
`/var/log/messages*`, and prints the state of the stub, preload and apply.

1. Deploy `tools/{lumatv,preload.py,autostart.sh,bootcap.sh}` to the set's app directory (`bootcap.sh` executable; the ipk
   build copies it). `lumatv log` is clean; `lumatv homestub status` shows `state stub`, `faults 0`.
2. Set on, screen on, Home in front; `lumatv logo on`; pick the styles (`lumatv theme set '{"logo":{"on":2,"off":3}}'`) so that a
   `[Logo] power-on logo 2` line cannot be the default.
3. `python3 docs/research/notify/reboot_logo.py docs/research/notify/boot-<date>-<hhmm>` (60 s of frames). It refuses a set
   that is not Active. Check: frames of the logo before Home (look at them), `[Logo] power-on logo 2`, `hook: boot capture of 60s
   started`, `faults 0`, Home in front, no `giving LG's Home back`.
4. Standby case, after step 3: put the set in Active Standby, then by hand `echo 30 > /var/lib/lumatv/bootcap; reboot`. From a
   second SSH session, `lumatv log`: expected
   `apply ... took` well under 188 s (the 140 s preload hole and 25 s of the pixel wait are gone; the rest is not re-measured), `homestub: no beat and the panel is
   off, waiting for the screen`, no `fault 1 of 2`; then wake the set: `screen on, waiting for the beat`, `homestub status`
   `state stub, beat true, faults 0` within 40 s, `preload status` still on, Picture Mode unchanged. If the beat never comes
   after the wake, the fault and LG's Home arrive 35 s later as before; note the `homestub` and `compositor` lines and the
   `/tmp/<shortId>-beat` age during standby (does the stub beat at all with the panel off?).

## Restart from a set that is on (2026-10-25 17:50-17:54, `docs/research/notify/boot-2026-10-25-1751/`)

Steps 1 to 3 of "Set steps for the boot path and the logo" at `f3ed5a6` (the tools of `35a880e`, unchanged since):
`tools/{lumatv,preload.py,autostart.sh,bootcap.sh}` copied to the app directory by rename (the running `homestub-watch`
shell keeps its old inode), `homestub status` `state stub, beat true, faults 0`, `logo status` on with
`com.webos.app.bootlogo` and `com.webos.app.masterbrand` mapped to LumaTV's QML, `theme set '{"logo":{"on":2,"off":3}}'`;
receiver muted through Home Assistant before the reboot and unmuted once Home was in front after it (0.355 both ways).
The set had gone to `ScreenSaver` on Home (the script refuses anything but `Active`); `turnOnScreen` brought it back.
`reboot_logo.py` issued the reboot at 17:50:38 from power `Active`, uptime 4589.7 s; sshd on the new system answered at
uptime 29.0 s (46 s at 16:34). The script's own SSH session died at its first command after the frames (channel closed;
the collection was finished with `NO_REBOOT=1`). `theme.json` was put back without the `logo` key afterwards.

The boot path, from `/var/lib/lumatv/log`: `homestub: stub in place at uptime 15 s`, `boot: homestub-boot {"ok":true,
"stub":true}`, `guard started`, `compositor-boot: off`, `boot-bind: 0 bind(s) made, assets live, compositor already on
shadow=no`, `boot: boot capture of 60s started` (17:50:53), `apply: staged set unchanged, shadow current, assets bound,
not remounting`, `homestub: compositor restarted` (17:51:33, uptime 55 s), `apply: compositor restarted to read shadow
module`, `apply: success version=0.4.0 mounts=1 took=38s`. After the boot: power `Active`, `/sys/power/mode` `resume`,
Home in front, `homestub` `state stub, mounted, sam web, guard, beat true, faults 0`, `preload status` on with the backup,
no `giving LG's Home back`, no `fault`. Against the 16:34 boot: the apply 188 s → 38 s (the preload hole is gone), and
the guard kept the stub through the compositor restart.

The panel, from the boot capture (85 frames, 480x270, 14.6 s to 73.4 s of uptime; the 12 kept are the frames where the
picture changed, `frames-index.txt` lists all 85 with their sizes): black at 14.6-16 s; 16.8-19.5 s a black screen with
LG's "HDMI 2" input banner; black to 23.2 s; 23.8-54 s LG's "No Signal" page of the HDMI 2 input through its four art
slides (the receiver was off), with LG's "HID device is disconnected" toast at 31.5 s; black 54.9-60.5 s (the
compositor restart of the apply); LumaHome from 61.2 s (clock, news, weather, the app row). So the set came back on
the last selected input, HDMI 2 (the compositor gate's `hdmi` step at 17:35 had selected it; Home was in front when the
reboot was issued), and Home took the panel only after the apply's restart.

The logo: no frame shows one and no `[Logo]` line exists. The capture's first frame is at 14.6 s and the panel already
shows the input at 16.8 s, so the logo phase (LG's or LumaTV's) was over by then; `/var/log/messages*` of this boot
starts at uptime 12.7 s (PmLog's first line), the compositor's system UI launches are logged from 13.2 s
(`FRESH_LAUNCH_SYSTEMUI` for quicksettings and quickinputpicker only), and nothing names bootlogo, masterbrand or
`[Logo]` after that. Either LG's compositor put the boot logo up before 12.7 s (lost before PmLog) or it did not launch
`com.webos.app.bootlogo` on a `reboot` at all; the frames cannot tell the two apart. What would: a capture or a log
that reaches back to the compositor's start ("Resume here" 3).
