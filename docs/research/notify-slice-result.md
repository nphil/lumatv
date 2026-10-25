# Notifications and logos (NotifySlice)

## Resume here

Built and tested offline: the toast, the alert, the input popup and the PIN prompt in the Luma look on top of LG's
notification view, their scenes (dark and light), and the boot and power-off logo choice. On the set: one restart for
the power-on logo (2026-10-25 16:34, "Restart on the set"), which answered nothing about the logo: the set was
restarted from Active Standby, booted back into Active Standby with the panel off, and no `[Logo]` line was written.
The two faults that restart found are fixed offline (below, "Boot from standby (BootPathFix)"), **not verified on the set**.
Next, in order:

1. `lumatv notifications on` on the set (stages LG's tree, patches it, overlays `payload/notification`), then
   `python3 docs/research/perf/gate.py --surfaces toast,alert` with Home on screen (see "Set steps").
2. A PIN prompt on the set with Nitin present (a locked input), see "Set steps".
3. The power-on logo from a set that is ON: "Set steps for the boot path and the logo", which also re-checks the
   stub guard and the boot apply's time on the way.

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
