## Facts the replacements must keep

Behaviour of LG's code that the rows above depend on. Evidence per row is in
`docs/research/surfaces/*.json`.

Notifications (phase 2):

- Every service message funnels into `com.webos.notification`, served by
  `notificationmgr`: four subscriptions (`getToastNotification`,
  `getAlertNotification`, `getInputAlertNotification`,
  `getPincodePromptNotification`). Queueing, default timeouts and PIN
  validation live in that daemon, so a view swap keeps it.
- An alert button is a luna call made as the source app
  (`callForApplication(sourceId, uri, method, params)`), not as
  surfacemanager. Toast and input-alert actions and `onfail` and `onclose` are
  plain calls. `createAlert` returns an `alertId`; callers close with
  `closeAlert` and move a progress bar with `setAlertProgress`.
- PIN digits go back through `closePincodePrompt {closeType: relay, pincode,
  mode, appId}`; Back at zero digits, or the third wrong code, sends `close`.
  A PIN prompt closes a non-modal alert and the reverse; a modal alert closes
  input alerts.
- Items are dropped while the foreground window's access policy forbids them,
  while the set powers off and while the OSD is off; a dropped alert fires its
  `onfail` call.
- Timings: a toast stays 5 s (1-10 s), one is visible at a time, a queued one
  replaces it after 2 s, hard cap 15 s. Z-order: PIN 10001, alert 10002,
  input-alert scrim 10003, input alert 10004, voice agent 10005, toast 10006.
- Alert text is spoken through `tts` when Audio Guidance is on, and buttons
  register with `voiceconductor` (`ALERT_SELECT_n`, `PINCODE_SELECT_n`).

Keys (phase 1):

- Slot 5 of the chain (`handleScreenSaver`) restarts the compositor's 120 s
  activity timer (`userActivityTimeout`) on every key. When it expires with
  the pointer hidden, the compositor closes overlays, alerts, the PIN prompt,
  the keyboard, the spinner, the splash and the Home-type view; it does not
  start the screensaver, `tvpowerd` does. A key swallowed in slot 1 never
  restarts it, so LumaTV calls the exposed `restartUserActivityTimer`
  (service app id `LS.appId + '.screenSaver'`) for the keys it takes, or LG's
  pending alerts and overlays close after two minutes. Whether `tvpowerd`
  counts keys the compositor swallows is unverified [INFERENCE].
- Slot 4 (`handleLongPressKeys`) swallows the press of Back, Gear, Input,
  Guide, 123 and Red and delivers it on release; it also shows and hides the
  Magic Remote pointer for them. A slot-1 handler that takes such a key calls
  `setMRCUCursorVisibility` itself. Replayed keys re-enter at slot 1, so the
  handler must be idempotent, and its timers must not reuse LG's shared
  `longPressTimer`.
- Writing configd `keyFilters` unsubscribes every filter, reruns each file's
  `init()` and reopens Quick Settings in Game mode.

Power and boot (phases 1 and 5):

- `tvpowerd` waits for the compositor to answer `responsePreparePowerOff`,
  `responsePreparePowerOn` and `responsePowerOffLogoRequest`; a replacement
  power-off logo reports done or power-off stalls.
- CoverUI is part of boot: `bootd` opens it through `coverUI/open`, it
  launches the first app and emits `smart-service-ready` and
  `firstapp-launched`.
- After boot, `preloadSystemUI()` launches Quick Settings and the input picker
  with `reason: preload`. Their replacements handle `{params, reason:
  'preload'}`, `params.activateType` and the `localkey` flag the one-key TV
  button sends.

Inputs and Home:

- `com.webos.app.inputcommon` is one binary behind Live TV and every input
  app, preloaded for `livetv` and `hdmi2`. Its launch parameters
  (`showBanner`, `showBannerWithEdit`, `toggleInfo`, `sendKeyPress`,
  `uniqueId`) are how services reach it. Overlays sit in window-group layers
  (ACR -9, captions -19 to -21).
- The picker lists only inputs that `eim/getAllInputStatus` reports connected
  with `hdmiPlugIn`; the set has four HDMI ports and no other input.
- Recents are drawn by Home from `recent/listRecents`. Closing the last app
  relaunches the latest recent, then `lastInputApp`, then `profile.idleApp`
  (Home).
- Today LG's Flutter Home still runs behind LumaHome; once the Home stub
  replaces it (phase 1) the hero banner, sponsored shelves and `NL_*` usage
  events stop at the source.
- On this set the volume OSD is the ARC form (speaker, + and -, no number)
  because `external_arc` reports `volumeSyncable` false.
