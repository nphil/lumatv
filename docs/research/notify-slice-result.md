# Notifications and logos (NotifySlice)

## Resume here

Built and tested offline: the toast, the alert, the input popup and the PIN prompt in the Luma look on top of LG's
notification view, their scenes (dark and light), and the boot and power-off logo choice. Nothing here has run on the
set. Next, in order:

1. `lumatv notifications on` on the set (stages LG's tree, patches it, overlays `payload/notification`), then
   `python3 docs/research/perf/gate.py --surfaces toast,alert` with Home on screen (see "Set steps").
2. A PIN prompt on the set with Nitin present (a locked input), see "Set steps".
3. A cold boot for BootLogo, see "Set steps".

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
