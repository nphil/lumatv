# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Volume and mute: a slim bar at the bottom centre is the only volume display, over Home,
  every app and every input; LG's popup is patched out of the shadow module (`lumatv revert`
  brings it back). It answers on the key before the receiver does, shows the Denon's level
  or the TV's own, the receiver's audio format (Dolby, DTS, PCM and the surround mode), and
  a slashed speaker while muted.
- Home Assistant on the TV: LumaShow's plugin runs through the helper as a user of its
  own, switched on and linked only by the owner's own commands (`plugin-consent`,
  `plugin-enable`, `ha-link`). Its cards fill the At-home lane, its tiles the quick panel,
  its notices the drawer, and its page is Settings > Home & look > Home Assistant. Locks,
  alarms and cameras are never touched from the TV; the placeholder chips are gone.
- Settings: minimal text on every page. A row shows its name and value; explanations sit
  behind one info mark, opened by holding OK. The rail shows one short fact per section,
  Remote & pointer has tabs, the Settings bar shows the link that is really up, and held
  Left/Right speeds up long sliders.
- Settings: native pages for LG's Accessibility (Closed Caption, Audio Guidance, audio
  options), Time & power (clock, power timers, auto-off, Screen Off), Bluetooth speakers
  and controllers, Wi-Fi join and IP settings, the HDMI rows and every Picture tab, on LG's
  own keys and value lists. The remaining LG menus open from a "More in LG settings" row.
- Helper: Bluetooth proxy for Home Assistant. The TV answers as an ESPHome Bluetooth
  proxy (passive scanning, raw advertisements, encrypted API, mDNS) fed by LG's bluetooth2
  `le/startScan`; off until the owner runs `lumatv-helper ble-link`, scans in windows of
  5-30 s with an equal or longer rest, and never while the panel is dark, Bluetooth audio
  is up or the adapter is off.
- Declutter group `autoinstall`: LG's silent app installs stopped (`option.appUpdateMode`
  manual, `support.automaticUpdate` off), the factory stubs LG's updater turns into full
  apps removed, and LG's not-installed country apps kept on the deleted list. Store
  installs and updates stay.
- Voice search: the mic key listens, Home Assistant's Assist pipeline turns the speech
  into text, and the text fills Search. Off until the owner links Home Assistant, chooses a
  microphone and switches it on; the helper's `voice-link` saves the address and token, and
  nothing the words say is run.
- LumaTV's own alert, input pop-up and PIN prompt over LG's notification view;
  toasts keep an app's own icon unless it is LG's generic file.
- Quick panel plugin tiles and an All settings button; the input picker draws
  its tiles and device icons from the TV's input list.
- Ambient light settings page, art-mode and logo options, and the screensaver:
  a drifting, dimming clock or a crossfade of six public-domain paintings, with
  no timers running while hidden.
- The notification drawer (bell on Home), plugin cards on Home, and weather,
  news and clock as first-party plugins on the plugin broker.
- The app library from the Apps tile (search, hide, rename, move, uninstall),
  search, the Live TV banner, and openers for the context menu, confirmation and
  on-screen remote.
- `tools/tokens.py web`: Lucent tokens as CSS for LumaTV's web apps.
- The UI engine draws into the compositor's scan-out pages (`--features ui`).
- The helper's `GET /v1/device` and device facts in every plugin welcome.
- The light-sync probe slot (`ambient/capture/tv_probe_slot.py`): LG's
  `vt_devmode` flag exists only while a capture runs.
- The card app (`org.nphil.lumatv.card`): a 64-bit app in its own card window,
  drawn with the Rust toolkit. SAM starts a 32-bit launcher that registers the
  app and passes SAM's relaunch and close on. It ships as an ipk inside the
  main one with its own 64-bit runtime (31 MB larger package). Under SAM's
  jail, whose `/dev` has only `dma_buf_unified` to allocate from, the heap shim
  takes the window buffers from that device.
- Hue Entertainment streaming as a second ambient output: `lumatv-helper
  hue-link` saves the bridge's keys privately with its certificate pinned, and
  the stream stops when the screen goes off.
- LumaTV's own banners over video (`LumaOverlay.qml`, switch `banners.own`,
  off by default), holding the gear key opens the notification drawer
  (`remote.gearHold`), number keys open dock tiles (`remote.numberKeys`), and
  the remote's app keys open the app chosen per key in Settings.
- Settings: Glass depth (Flat, Normal, Deep), a sleep timer on System and in
  the quick panel, and panel hours on OLED care.
- LG's input banners in the Lucent look (`lumatv inputcommon on`, off by
  default) and the on-screen keyboard's open and close in the theme's layer
  motion, by anchored patches that refuse when LG's code changes.
- Cutover safety for LumaTV's compositor: a firmware check against a list of
  verified builds, a watchdog that falls back to LG's compositor on a frozen or
  empty panel or an unsettled run, the hold-Home escape from the first moment of
  boot, and a gate that allows no gap over 20 ms.
- Declutter group `autoinstall` (profile `ads`): LG's silent app installs
  stopped at their two switches (`option.appUpdateMode` manual,
  `support.automaticUpdate` off), the factory stubs that LG's app updater turns
  into full apps in standby removed (DAZN came that way on 2026-10-25), and
  every not-installed id of LG's country list on SAM's deleted list so the
  wake-time sync adds nothing. The store's own installs and updates stay. Host
  tests run the tool against a made-up set (`DECLUTTER_ROOT`), and switching
  one group off re-applies the list edits of the others still on.

### Changed

- `lumatv preload on` keeps only Netflix, the Home stub and the HDMI input apps
  on LG's preload list: LG loads every enabled entry whenever memory allows, so
  Live TV, LG Channels, the voice app, Miracast, YouTube, the browser and Prime
  Video (30-70 MB each) are switched off. The HDMI inputs stay ready so a switch
  to the PC is instant. The live policy is accepted only while the screen is on;
  in standby the helper's next screen-on applies it.
- Settings places initial focus when a page is entered, after service values
  have arrived, rather than selecting an unavailable page's last link during
  construction. The remote page describes the 650 ms Home escape.
- Receiver readings show only while the helper is up, the topic is fresh and
  its stamp is under six seconds old, through one gate for the volume display,
  the quick-panel tile and the receiver page. Polls that change nothing move
  the stamp without raising an event, so they do not wake Home's long poll.
- Settings rows write LG's own keys and values, rows LG marks inactive are
  dimmed, and deep links open the LG screen they name.
- The compositor takes direct scan-out while LumaTV's UI shows nothing over a
  full-screen window, the UI app's socket wakes the event loop, the frame latch
  learns the GPU's finish from the in-fence, and a covered card is minimized and
  returns on fullscreen.
- The helper takes the receiver's volume, mute, input and power over one held
  telnet connection, so changes reach the volume display as the receiver makes
  them; the HTTP status read is a fallback while telnet is down.
- The colour reader polls 10 times a second with an integer NEON reduce
  (output bit-identical, about 1.9 M instructions a poll down to 15 k).
- Compositor: foreground pushes leave the frame path, a frame is held while a
  driver buffer's first GPU job runs, texture frees are spread over flips, a
  system layer can sit on the overlay plane above a directly scanned-out
  window, the pacer follows the panel's measured refresh, each input device has
  its own `wl_seat`, and a keyboard made after focus is entered at once.
- The 64-bit card app binds every `wl_seat`, so the remote's keys reach it under
  LG's compositor, which sends a key on the pressing device's seat only.
- Boot: a set that boots into standby no longer gives LG's Home back, the
  preload policy waits for the screen, the compositor restart comes right after
  boot-bind so Home is up sooner, a set configured to start on its last input
  keeps it, and the logo windows keep a stamped record in `logo-events`.
- The compositor starts with `QT_QPA_UPDATE_IDLE_TIME=0`, removing Qt's fixed
  5 ms wait before each update; dropped animation frames fell from 8-9 % to
  3.4-4 % on a D-pad sweep and from 46-67 % to 2.5-4 % opening Quick Settings.
- Home's wallpaper motion and clock hands step on one shared tick (default
  10 a second) instead of separate timers, and the second hand steps without
  its overshoot animation, so idle Home no longer presents an uneven stream of
  frames with a periodic hitch.
- The project is renamed LumaTV. Every name (display name, app id, data dir,
  QML module directory, tool and boot-hook names) comes from `brand.json`;
  the build fills the `{{displayName}}`, `{{shortId}}`, `{{appId}}`, `{{dataDir}}`
  and `{{qmlModule}}` placeholders in the staged copy and names the ipk, the
  frame app and the installed files from it. The app id is now
  `org.nphil.lumatv`, the data dir `/var/lib/lumatv`, the tool `tools/lumatv`
  and the boot hook `50-lumatv`.
- The first verb run by the new tool on a set that has the LumaGlass layout
  moves `/var/lib/lumaglass` to `/var/lib/lumatv`, points the compositor env
  file, the early-boot hook, the init.d link, the configd key filter and the
  Quick Settings entry at it, and removes the apps `org.nphil.lumaglass` and
  `org.nphil.lumaglass.frame`. Revert and off work on a set that has only the
  old layout.
- `repo.json` lists no package until the first LumaTV release regenerates it.
  Its `brand.json` `repo` entry (`nphil/lumatv`) names the GitHub repository
  that release URLs point at.
- LumaHome is built when the compositor starts, not at the first visit to
  Home, so its heartbeat and a load error are visible at once. The Home
  surface's opacity is set to 0 or 1 outright: Qt 5.12's `Binding` left LG's
  Home at 0 when LumaHome stepped aside.
- Revert, the boot fallbacks and a failed apply remove the Home stub before
  the compositor restarts. `apply` leaves Home's assets bound under it.
- After a compositor restart with the Home stub mounted, the Home launch waits
  for the new WebAppMgr: at boot the old one closed 0.5 s after the launch,
  taking the stub with it, and the panel stayed black. A black panel on an
  Active set at boot is no longer taken for a boot into standby, so the launch
  is repeated. The guard gives LG's Home back for a missing module only after
  3 s, since a process read between fork and exec shows no module.
- `theme.json` is the one token source: colour roles for both materials in
  Lucent's names, type roles, radii, spacing, focus, state washes, motion and
  the font, in the shipped `payload/theme/theme.json`. Home merges the user's
  file over it key by key (the shipped files are staged as `theme.default.json`
  and `layout.default.json`, replacing the literal defaults in the QML), and
  `lumatv apply` cuts the user's files down to what differs from the shipped
  copy, so what was never changed follows an update. `radius` and `tileRadius`
  became `radii.card` and `radii.tile`; a material's `scrim` list became
  `wallScrim`, `scrim` now being the modal scrim colour.
- `tools/tokens.py` replaces `palette.py` and resolves the same tokens for the
  consumers that are not the compositor's QML: the web CSS (`--lu-*` as in
  Lucent's tokens.css, plus the Sandstone variables, generated per app), the
  settings-sheet palette Quick Settings is patched with (byte-identical to
  before) and `theme/Tokens.js`, which QML in other processes imports by file
  URL. `lumatv.css` carries no colour of its own.
- Manrope is the one typeface: four static faces (400, 500, 600, 700) cut from
  the pinned variable font by `generators/gen-fonts.py`, rendering identically
  to the faces they replace, loaded from `theme.font.files`. The web theme and
  Quick Settings use it (Quick Settings' tile titles are drawn at 0.9 of LG's
  size, Manrope being wider); Inter is gone from the package (1.7 MB).
- Home's key filter fails open and pairs presses with releases. A key is taken
  only while Home is in front, LumaHome's heartbeat is under 3 s old and its
  handler takes it; an error anywhere in the filter or the handler passes the
  key to LG untouched and logs one line per 5 s. A release goes where its press
  went, so a held OK that opens an app leaves no stray release for it. Unit
  tests: `bun test tests/`.
- OK acts on release. Held 700 ms (`motion.holdMs`) it picks a dock tile up;
  anywhere else a long press now acts like a short one. A held OK and a
  picked-up tile are cleared when Home is hidden or shown.
- Focus rest: the highlight follows each key press at once, while lift, scale,
  shadow depth and the name pill start only after the focus has rested 150 ms
  (`motion.restMs`) and drop at once when it leaves; the dim of unfocused cards
  and the dock's accent glow follow the highlight. A D-pad sweep draws one
  frame per key press: 150-157 frames and 1195-1271 ms of GPU in a 5.2 s
  sweep, against 220-224 frames and about 1770 ms.

### Fixed

- Luna calls from the compositor QML work on the set: `LS` lives in `WebOSCompositorBase`, not
  `WebOSServices`, and eleven files that imported only the latter failed with "LS is not defined"
  (the Settings launch, the notification log, the app list). The offline host now exports `LS`
  from `WebOSCompositorBase` only, and a test fails when a file uses `LS` without the import.
- `tests/qmlhost/compile_all.py` compiles every QML file of the surface with the set's import
  layout; `tools/build-ipk.sh` stops on any syntax, type or import error, so a surface that
  cannot build never reaches the TV.
- A restart from standby no longer hands LG's Home back. The Home stub guard treats a missing
  heartbeat with the panel off as not yet and checks again from the first screen-on; a stopped
  beat on a set that is on is still a fault. The boot apply no longer waits 140 s on the preload
  manager (it holds `setPreloadPolicy` while the screen is off; the helper sends the policy at
  the first screen-on) nor 30 s for pixels on a panel that stays off.
- The ipk now ships the tools `systemui on` and `webglass on` call:
  `tokens.py`, `systemui.py` and `systemui_patch.py`.
- The "auto" light or dark glass choice read a canvas that stayed empty on
  this firmware, so it always chose dark. It now reads the wallpaper.
- A plugin could be cut off when the helper stopped without its Shutdown message: the connection
  ended on the stopping flag before Shutdown was queued, and was closed before the queue was
  written. The outbox byte count could also wrap when the writer took a frame before it was counted.

### Added

- The component kit, `payload/compositor/lumatv/kit/`: the token layer `Lu`
  (a QML singleton fed by LumaHome), `LuText`, `LuRow`, `LuToggle`,
  `LuSlider`, `LuButton`, `LuSegmented`, `LuChip`, `LuHalo` (the dock's accent
  glow, now shared) and `FocusLift`. Every component follows the focus rule
  and Nitin's cue: no line under the focused item. `theme set
  '{"kitGallery":true}'` shows them all on one sheet (Back closes it).
- Tests: `bun test tests/` (key filter, saved-first cache, tint, kit) and
  `python3 -m unittest discover -s tests -p 'test_*.py'` (tokens, theme file).
- Home's weather and news cards draw what the native helper serves (`kit/Helper.qml`:
  one state read, then a long poll, so the cards fetch nothing themselves) and, until
  the helper has answered, the document it saved last time in
  `/var/lib/lumatv/cache/weather.json` and `news.json`. A reading is never drawn after
  6 hours (or when stamped over 5 minutes in the future) and an older reading never
  replaces a newer one. The ZIP, units and news edition are the helper's settings
  (`base.weather`, `base.news`) instead of keys in `theme.json`.
- `lumatv helper start|stop|status`: the native helper is staged under
  `/var/lib/lumatv/bin` (an update cannot overwrite a running executable) and started
  by every apply and boot, stopped by revert. The ipk carries `helper/dist/lumatv-helper`
  and the build refuses to run without it.
- `lumatv preload on|off|status` keeps only Netflix and Prime Video warm: YouTube, LG
  Channels and the browser are dropped from the preload list in configd and in the running
  preload manager, LG's list is saved once and restored by `off` and `revert`, and the
  helper sends the policy again at every switch-on, which webOS 10 needs. Measured on the
  set: no change in free memory or Home-return gaps, as it closes nothing that is running
  (it changes what a cold start preloads), so it ships off.
- `lumatv logo on|off|status` replaces LG's power-on and power-off logos with LumaTV's:
  windows for `BootLogo` and `MasterBrand` that load one of three animations each
  (`theme.json` `logo.on`, `logo.off`, default 1; stills in `docs/mocks/logo`). The
  power-off window answers `tvpowerd` once, whatever happens, so a missing or broken logo
  never holds a power-off.
- The startup warm-up draws every text style, image and shader component the surfaces use
  once, hidden, after Home has presented, so a first open compiles and rasterises nothing;
  `theme.json` `warmup` switches it off.
- `docs/research/perf/gate.py` is the per-surface frame gate (p90 render under 3 ms over all
  the surface's animated frames, no frame over 20 ms inside a running animation, first frame
  within 50 ms, open by 180 ms, RSS flat over the cycles, no QML error in the log) and
  `startup.py` and `preload_ab.py` are the A/B drivers; the gate holds LG's compositor to its
  measured floor of 17 % late frames and LumaTV's own to none.
- `tests/qmlhost` loads a compositor surface on the workstation (PyQt5 on Xvfb) with stand-ins
  for LG's modules and a fake helper: load errors, screenshots, scripted window life cycles.
- `lumatv compositor on|off|status` and the boot worker's `compositor-boot` put LumaTV's own
  compositor over LG's `surface-manager` when the bundle is installed and enabled, check that
  LG's binary is the one the bundle was installed against, and fall back to LG's compositor
  by revert, by `off` or by the supervisor. While it is live, the Home stub's guard does not
  read LumaHome's heartbeat, apply does not restart the compositor, and nothing restarts it
  while a Home recovery holds `/tmp/lumatv-recover.lock`.
- `lumatv notifications on|off|status` points the compositor's notification view
  (toasts, alerts, input popups, the PIN prompt) at a copy of LG's own tree that is
  made and patched on the set: Manrope, the toast in the theme's colours, and the
  live shadow layer of each toast, alert and button turned off. LG's data layer and
  item contract are untouched, so a notification of any kind arrives as before; a
  stage or a compositor that refuses the file leaves LG's view in place, and revert
  puts LG's view back.
- `lumatv surfaces` lists every switch (Home, Quick Settings, the settings theme,
  notifications, logos, helper, preload, compositor) with whether it is on, read
  from the flag or state its own verb uses, and the way back to LG's version.
- The gate has entries for LG's own toast and alert (`gate.py --surfaces
  toast,alert`), opened by a luna call, as the baseline the rebuilt ones must beat.
- The notification view's toast is LumaTV's own (`payload/notification/Toast.qml`, laid over LG's
  tree when `notifications on` stages it): a glass pill at the top right under the status bar with
  an icon tile, a title over a dimmer subtitle, and an accent link, where LG draws a grey bar at the
  bottom. The alert is LG's, centred over the palette's scrim and in the palette's colours, its buttons
  a lighter glass with a light edge when focused; LG's red focus colour is the accent.
- LumaTV's Settings (`Settings.qml`, `settings/`) is a layer of Home, opened from the gear's popover
  ("All settings"), with Home's widgets and bar stepping aside while it is up. A rail of sections, each
  with a line on its current value, and a page: Home & look (dynamic tint, appearance, clock, wallpaper
  motion: the theme's own knobs, written to the user's `theme.json`) and AI access (listener, pairing,
  assistants, what each may do, recent activity, lock-out). The last rail row opens LG's Settings app.
- The AI prompt (`AiPrompt.qml`) draws the helper's `ai.prompt` over Home and keeps the remote's
  direction, OK and Back keys from everything behind it. It has no button and calls nothing: the helper
  reads the answer from the physical remote's input device itself. The AI page only ever asks; whatever
  gives an assistant more waits for that answer, and giving less (off, pause, remove, a lower level,
  lock-out) is done at once.
- `kit/Helper.qml` follows the `ai` and `ai.prompt` topics, sends DELETE with the same headers as POST
  and returns the document of 202 and 409 answers.
- The offline host's fake helper models the `/v1/ai` routes (a prompt opens for what gives more and is
  answered only through the test's own door), and the host parks the pointer in the corner, so a
  hover cannot move a test's focus.
- `lumatv revert` and `apply` no longer restart SAM or `homelaunchpoints` when they put back the icon
  edits of an older version: the files are restored and the launcher reads them at its next start.
  SAM is never restarted; a test fails if any shell tool in `tools/` restarts, stops or kills it.
- Wallpaper tint: LumaHome samples the wallpaper at 32x18, cuts each channel
  into 8 levels, skips near-black pixels and takes the most common bin; a
  newer wallpaper cancels a sample still running. The result is kept in
  `theme/tint.json`. A dock tile with no plate colour glows in the tint's
  accent (`accent: "wallpaper"` does so for every tile and for the web menus
  that `tokens.py` themes), and `tint.mix` moves the focused glass toward it.
- `lumatv homestub on|off|confirm|status` retires LG's Flutter Home behind a
  web stub bound over its directory (`payload/homestub`), so LumaHome is the
  only Home. The stub is one WebAppMgr renderer at 32 MB resident against the
  Flutter process's 72-80 MB. SAM is made to read it with `blockSystemApp` and
  `releaseBlockSystemApp`, never restarted, and the block list is
  byte-identical afterwards; LG's process is stopped; Home's assets bind stays
  under the stub for the way back. `status` reports it.
- `homestub on` starts a 120 s trial that reverts itself unless `confirm`
  or OK on a toast in LumaHome arrives. Only a confirmed switch is kept in
  `config.json` and mounted at boot, by the boot worker's first act, ahead of
  anything that could start LG's Home; a boot after one that never finished
  skips it.
- A guard process (`homestub-watch`, 0.05 % of a core) gives LG's Home back
  about 6 s after LumaHome's once-a-second heartbeat stops, at once when the
  compositor restarts without the LumaTV module or LumaHome fails to load, and
  when Home is held for 650 ms anywhere, until the next apply or reboot. Two
  faults in a row switch the stub off. The 650 ms hold also cancels LG's own
  2.5 s Home hold (Screen Zoom). The heartbeat, mode, escape and confirm files
  go through LG's synchronous `TextFile` type: `XMLHttpRequest` PUTs twice
  reached the file 5 s late with the timer on time.
- `lumatv systemui on|off|status` runs Quick Settings from a copy of LG's
  QML tree under `/var/lib/lumatv/systemui`, by pointing the panel's entry
  in the configd object `com.webos.surfacemanager.systemUIManager` at it.
  Nothing is written to `/usr`, and `off` restores the stock path in one
  configd write. The copy is rebuilt whenever the stock tree's fingerprint
  moves, so a firmware update cannot leave old QML running against new
  libraries.
- `payload/systemui/QuickSettings/Component/CommonBoxShadow.qml` replaces LG's
  drop shadow, which rendered every shadowed item into an offscreen texture and
  blurred it in two passes on each repaint, with kernels the call sites size up
  to 51 samples. The replacement evaluates the shadow analytically from the
  signed distance to a rounded box: one pass, no texture read, and no repaint
  when the item it sits behind animates.
- The settings menus wear the LumaTV material. Quick Settings is a floating
  sheet, inset from the screen edges and rounded on every corner, with glass
  tiles and header buttons; focus lifts a surface rather than inverting it to a
  light fill. `tools/systemui_patch.py` applies this to the staged copy, and
  fails loudly on a missing anchor so a firmware update cannot half-patch it.
- `lumatv webglass on|off|status` gives the same treatment to the two web
  settings surfaces, Settings and Notification Centre. WebAppMgr injects
  `webOSUserScripts/userScript.js` from an app's own directory, so each app
  gets an overlay mount whose upper layer holds just that script, the
  stylesheet and the font - no copy of a 23MB bundle, and unmounting restores
  the stock app exactly.
- `tools/tokens.py` derives the menu palette from `theme.json`, so the menus are
  tinted by the wallpaper and theme in use. A contrast guard darkens or
  lightens the derived sheet until its body text clears 7:1 against it, which
  is what keeps a pale theme from producing an unreadable menu.
- Quick Settings stays built between opens, the way the volume OSD does.
  Stock rebuilds the whole panel from 165 QML files on every press: 420ms from
  the key to the open transition, which then waits another 180ms and runs
  stretched under the build, fully open at 860ms. A close the panel makes
  itself (back, the settings key again) now stands it down to the view
  container's `background` state instead of exiting, which keeps the
  lifecycle at launch, so the next press relaunches the same instance: open
  transition 10ms after the launch, fully open at 170ms. A close from outside
  (Home, power, another app) still exits as stock, because the manager
  refuses to relaunch an app whose last event was a close; the first press
  after one is a normal cold open. Compositor RSS is flat across 40 relaunches.
- `/var/lib/lumatv/env.extra` passes extra variables to the compositor,
  one `KEY=VALUE` per line, for tuning and measurement. Only
  `QT_QPA_UPDATE_IDLE_TIME`, `QML_FORCE_DISK_CACHE`, `QSG_*` and `QV4_*` keys
  with plain values are copied into its env file; anything else is ignored.
  `revert` and the boot fail-safe delete the file, so an experiment does not
  outlive a compositor that failed to start with it.

### Changed

- LumaHome is built once and kept. It was destroyed whenever an app came to
  the front and rebuilt on every return to Home, which froze the panel for one
  frame of 187-320 ms and then 3-6 more of 43-116 ms; a return now shows at
  most two frames of 57-87 ms. The kept copy holds about 52 MB of compositor
  memory while an app is in front.
- While Home is hidden its clock, theme check, news and weather stop; showing
  it again ticks at once and refreshes feeds older than their interval.
- `apply` relaunches Home when the restart leaves only a background (preload)
  instance of it, which had left the panel black after a wake.
- The key filter is written to configd only when the chain would change.
  Each write makes the compositor reload every filter and rerun LG's init(),
  which in Game mode opens the Game Dashboard.
- The frame probe reports pacing rather than a rate: worst interval and missed
  60Hz deadlines per second, plus one line per gap over 20 ms. A single dropped
  frame is invisible in a per-second average.
- The compositor keeps its compiled QML between starts: `QML_FORCE_DISK_CACHE=1`
  in its env file overrides LG's `QML_DISABLE_DISK_CACHE=1`, which cut
  restart-to-Home from 5.8 s to 4.6-5.2 s. The cache
  (`/home/root/.cache/surface-manager/qmlcache`) is cleared whenever the
  staged compositor set changes, and at revert.
- `apply` restarts the compositor when its env file changes, not only when
  its QML does; the file is read only at start.

## [0.4.0] - 2026-09-15

### Changed

- The Home screen is drawn by the compositor. `payload/compositor/lumaglass`
  (LumaHome.qml, status bar, widget cards, dock) renders the wallpaper,
  glass chrome, clock, Home Assistant, news and weather widgets and the app
  dock on top of the stock Home, which stays resident underneath as an
  invisible placeholder. Cards, tiles and the status bar are single shader
  quads that sample one cached blur of the wallpaper, so nothing runs per
  frame while idle and focus and layer changes animate at 60 fps.
- Navigation has two layers: the dock (default) and the widgets. Up from
  the top dock row or Back moves to the cards; Up from a top card reaches
  the status bar; Down from a bottom card returns to the dock. Back from
  the widget layer is left to Home. The Magic Remote pointer focuses by
  hover.
- Remote keys reach the layer through `payload/keyfilter/lumaglass.js`,
  registered by the tool in configd's `com.webos.surfacemanager.keyFilters`
  (picked up live, no restart). A shadowed `controllers/StarfishKeyFilter
  .qml` is never loaded on this firmware: module types resolve to
  `/usr/lib/qt5/qml` whatever `QML2_IMPORT_PATH` says, only relative-file
  references from an already-shadowed view follow the shadow.
- Theme and layout live in `/var/lib/lumaglass/theme/{theme.json,
  layout.json}` and reload live within 2 s of a change. Layout is a column
  grid with explicit row heights (the reference pins 222/238/392 px);
  every element, the dock included, is a widget entry with col/row/span/
  rows. Widget types resolve to a built-in, a QML file in the theme
  directory, or an installed pack under `/var/lib/lumaglass/widgets/
  <packId>/pack.json`. Theme files the user has edited are never
  overwritten by an update; untouched ones follow the payload.
- Tile plates are derived in memory from each icon's edges (full-canvas
  artwork gets a plate of its own edge colour drawn edge to edge, floating
  artwork sits inset on a plate in its dominant hue); `theme.tiles.plates`
  and `theme.tiles.icons` override per app id. The per-app appinfo/icon
  rewrite and its `sam` restarts are gone; `apply` restores any files it
  had touched.
- Dock tiles can be rearranged: hold OK (or press-and-hold with the pointer)
  on a tile to pick it up, Left/Right/Up/Down swap it with the neighbour
  (which slides across), OK drops it and Back puts it back. The order is
  written through `homelaunchpoints/moveLaunchPoint`, the list stock Home
  reads, so it persists and both homes agree. LG's Apps tile is unmovable.
- The profile button shows the LG account (nickname, initial and avatar
  colour from `accountmanager/getLoginUserData`) and opens a popover with
  the account and a "Manage LG account" row that launches the LG Account
  app. Popovers take Up/Down/OK, and Back closes them.
- The wallpaper is alive: the still image is drawn through a slow domain
  warp (two sine fields, a few pixels of drift, the warp also lighting the
  waves for depth), evaluated per vertex on a 48x27 mesh so the fragment is
  one texture read, ticked at 30 fps by a timer rather than the animation
  clock. `theme.wallpaperMotion` sets amplitude, speed, depth, fps or turns
  it off; `motion.reduced` disables it. The glass blur stays baked from the
  still image so nothing above changes.
- Wallpaper motion is adjustable in two places: a Wallpaper panel in the
  companion app (`lumaglass theme get|set` behind it) and a popover under
  the gear on Home, where Left/Right change amount, speed, depth, wave size,
  detail and frame rate with the widgets and dock faded out so the wallpaper
  is seen bare; Back saves to theme.json. Other popovers dim the page behind
  them by 30% (22% on the light material).
- Wallpaper look toggles, all free per frame: vignette and specular
  (per-vertex, with the wave lighting), bloom (a screen blend of the
  already-baked blur, fetched per vertex), saturation and contrast (baked
  once into a graded copy of the wallpaper that the live pass, the still
  path and the glass blur all read). Measured 58-60 fps with everything on.
- Wallpaper grain is 0.4% (was 1%): with the TPDF dither that still breaks
  banding on the dark gradients without reading as texture.
- Mini apps. OK on a widget card hands it the keys (the edge brightens);
  in the news card Left/Right step through stories (the slideshow holds for
  30 s after a manual step) and OK opens the article: a glass frame grows
  out of the card to a centred 1520x860 window, shows a spinner, and the
  article renders inside it. Back shrinks the frame back into the card.
  Web content is rendered by a second, overlay-type web app
  (`org.nphil.lumaglass.frame`, transparent, composited above Home) so
  Chromium's work never touches the compositor's frame budget; it embeds
  the page in an iframe where the site allows framing and otherwise builds
  a reader view (headline, hero image, body) from the fetched HTML, since
  news sites send `x-frame-options: deny`. Up/Down scroll. The frame app
  ships as an ipk inside the payload and `apply` installs it through
  appInstallService when its version changes. Any HTML5 app URL can be
  opened the same way.
- The news card cross-fades between stories (hero images mixed in the
  card's own shader, text dipping out and in) instead of cutting.
- Frame budget: a sustained animation with the full layout and the live
  wallpaper measures 60 fps. Card and tile quads shade only the shadow term
  outside their rect, use tighter margins and mediump colour math, the
  scrim is folded into the wallpaper pass, and the covered stock Home
  surface is skipped at opacity 0.
- The clock has an analog style (face drawn once, hands as rotation
  transforms, ticking second hand, weekday/date/week strip beside it) next
  to the digital one; `theme.clock.style` selects it.
- Tile focus is depth only (lift, scale, deeper shadow, rim, sheen); `theme.focus.tileGlow` and `theme.focus.ambient` bring the accent halo and dock wash back.
- Side safe margin is 64 px (was 96); the dock centres its tile grid and
  shows the bottom fade only when it scrolls.

### Fixed

- Back closes the notifications popover and, from the dock, moves to the
  widgets. The key filter is registered first in configd's list: the stock
  `handleSystemKeys` answers Back with NotAccepted while Home is up, which
  ends the chain before a later entry.
- `apply` no longer restarts the compositor on every run: the staged Home
  QML tree is hashed by paths relative to the tree, so `luma.next` and
  `luma` compare equal when their contents do.
- `apply` restarts the compositor when the QML, wallpaper or key-filter
  script changed. It only restarted a compositor that lacked the shadow
  module, so a set with the mod already live kept running the previous
  QML until the next boot.

## [0.3.6] - 2026-09-14

### Changed

- The cause given in 0.3.5 for the `webos-sddp` crash is corrected. It does
  not crash "on the first wake on this firmware"; it segfaults in
  `SDDPSetDevice` whenever it reads the interface list and finds a
  point-to-point interface with the multicast flag - the Tailscale `tun` on
  the reference set. With `tailscaled` stopped it starts and stays up; with
  the tunnel present it dies within 3s of starting. It only looked like a
  wake crash because at boot it starts before Tailscale's hook creates the
  tunnel and re-reads interfaces on the first wake. A stock set has no such
  interface. The toggle's behaviour is unchanged; README, the handoff and
  the code comments now say why.

## [0.3.5] - 2026-09-14

### Fixed

- The set now stays in active standby after power-off, so a later power-on
  is instant with the mod already on screen. It never did before: every
  power-off was a hidden reboot, because the crash report the compositor
  restart provokes (inputcommon, `QFontDatabase::removeAllApplicationFonts`)
  was never actually removed. `disarm_restart_crashes` read the names back
  from `ls`, which word-split them on spaces and printed the 0x02 bytes rdxd
  uses in place of `/` as `?`; `rm -f` on those names was a no-op that still
  counted as a success, so "disarmed 9" meant nothing. faultmanager then
  answered the suspend request with `rebootToSuspend`, and tvpowerd rebooted
  the set ~6s after it had entered active standby. The sweep now uses
  pathname expansion, which returns the raw bytes, and runs a second time
  once Home is back on the panel, since the crash lands ~3s after the
  restart and can miss the first pass.

  Measured: three power-off / wake cycles of 150s each with no reboot,
  uptime carrying across, and the same compositor still running.

### Added

- An "Instant on" toggle in the app, `standby warm|cold` in the CLI. Warm
  turns Quick Start+ on and `enableSDDP` off, remembering the previous SDDP
  value; cold turns Quick Start+ off and restores it. The tradeoff is the
  user's: warm means an instant power-on with the mod already up, at the cost
  of higher standby draw and no Control4 discovery; cold is the lowest
  standby draw with a ~35s boot at every power-on. `enableSDDP` matters
  because `webos-sddp` crashes on the first wake from active standby on this
  firmware, and its crash report arms the same reboot at the next power-off.
  `status` reports the live mode as `standby`: `warm`, `cold`, or `system`
  (Quick Start+ on with SDDP still enabled - the stock state, which is not
  instant-on in practice).

## [0.3.4] - 2026-09-14

### Changed

- The mod now reaches the screen at boot+33s instead of boot+55-60s, and the
  boot no longer shows stock Home before switching. Two changes get there.

  The apply used to run from Homebrew Channel's `init.d` run-parts at ~33s,
  by which time Home has long since painted. It now triggers at ~12-14s from
  a PATH shim in front of `grep` for `kdump.service`, which starts at 14.2s,
  reads an `EnvironmentFile` under `/var`, and on a set without `crashkernel`
  on the kernel command line is a no-op that exits 0. The shim always execs
  the real grep, so it cannot stop that unit, and run-parts stays as the net
  in case the early apply fails.

  The restart path also slept 14s before asking for Home and 8s after.
  Measured, asking for Home immediately still put pixels on the panel at
  +12s, so both sleeps are gone. Readiness is now taken from the panel: a
  captured frame is 835 bytes while it is still blank and over 79000 once
  Home is composited, whereas the Home process, `NL_HOME_SHOWN` and
  `getForegroundAppInfo` all go true about 8s before anything is drawn.

  Restarting the compositor that early also provokes none of the stock-app
  segfaults a later restart does, so there are no crash reports to disarm.

### Fixed

- The cause of the compositor's first start ignoring the override, withdrawn
  as unexplained in 0.3.3, is that `/var` does not show the writable ext4
  subtree until `mount-readwrite.service` relocates it at 12.96s.
  `surface-manager-daemon` starts at 2.63s, so `/var/systemd/system/env` is
  not a path that exists yet and the optional read is a silent no-op - the
  file's atime is untouched across a boot, and LG's own env files for
  `ls-hubd`, `configd`, `memchute`, `tvpowerd`, `bootd` and `sam` are equally
  inert that early. One compositor restart per boot is therefore structural,
  and 13s is the floor for anything this mod can do.

  Baking the mod into the hibernation image the set resumes from
  (`snapshot resume=/dev/mmcblk0p54`) was tried and does not work: the image
  is captured at ~15s, before the 13s floor plus a compositor restart can
  complete, so it always captures the stock compositor.

- The late hook no longer applies a second time on top of the early one. It
  stands down while an early worker is running or has succeeded, and only
  takes over if that worker died with the boot still incomplete.

## [0.3.3] - 2026-09-14

### Fixed

- A boot could leave the modded Home layout with the stock compositor: no
  wallpaper, no clock, weather or news. `apply` only restarted the compositor
  when the panel already reported Active, and on a boot where the panel had not
  finished waking it skipped the restart and nothing retried, so the set ran the
  whole session on stock QML. The restart now happens whenever the running
  compositor lacks the shadow module, whatever the panel is doing; a restart
  while the panel is dark is invisible, never restarting is not

### Changed

- The claim that the compositor picks the override up on its first start is
  withdrawn. Measured across consecutive boots it takes the global
  `DefaultEnvironment` from `/etc/systemd/system.conf.d/30-webos-global.conf`
  instead of the unit's `EnvironmentFile`, although the same file applies to
  every later restart, and `var.mount` completes at 1.946s while
  `surface-manager-daemon` does not start until 2.591s - so the file was
  readable. Cause not established, and nothing depends on it any more. One
  restart per boot remains the norm; what keeps power-off immediate is clearing
  the crash reports that restart produces, not avoiding the restart

## [0.3.2] - 2026-09-14

### Fixed

- A power cut while the shadow compositor module was being written left the TV
  with a black screen on the next boot. `/var` is ext4 `data=ordered`, so the
  directory entries were committed and the file contents were not: the module
  had all 318 files present holding 212KB instead of 2030741 bytes. The
  compositor started on it, resolved nothing and presented nothing, and because
  it was running and healthy no liveness check noticed. `build_shadow` now
  syncs the contents to disk before the rename publishes them
- Shadow integrity is checked against a fingerprint of the whole tree, not just
  the one file we inject. The truncated files were never looked at, so the
  damaged module was reported as current
- `boot-bind` restarts the compositor after repairing a module the compositor
  had already loaded. Rebuilding on disk does nothing for a process that read
  the bad copy at startup

## [0.3.1] - 2026-09-14

### Fixed

- The firmware-staleness guard never fired. `stock_module_fp` used
  `find -printf`, which busybox 1.35 does not implement, and the failure was
  silent: the pipeline hashed empty input, so every fingerprint compared equal
  to every other and a replaced compositor module would have gone unnoticed.
  It now fingerprints the recursive listing

## [0.3.0] - 2026-09-14

### Changed

- The compositor no longer gets its QML from a bind mount over `/usr`. A full
  shadow copy of the `WebOSCompositor` module lives in `/var/lib/lumaglass/qml`
  with the modded file swapped in, and `QML2_IMPORT_PATH` points at it through
  `/var/systemd/system/env/surface-manager.env`, an optional environment file
  the compositor's own unit already reads. The compositor therefore starts on
  the modded QML at boot+2s. Only Home's assets are still bound

### Fixed

- Cold boot no longer shows stock Home for ~25s before the mod appears, and no
  longer restarts the compositor. A restart was previously unavoidable: the
  compositor reads QML once at startup, it starts at boot+2s, and Homebrew
  Channel's hooks cannot run before boot+20s
- Power-off is immediate again. A compositor restart makes LG's igallery
  preview segfault in `QFontDatabase::removeAllApplicationFonts` - reproducible
  on stock firmware with no mod present - and faultmanager then reports the
  crash with recovery `rebootToSuspend`, so tvpowerd answered the next power
  press with a full reboot into standby instead of switching off. Not
  restarting the compositor avoids the crash; when an interactive apply does
  have to restart it, the reports that restart produces are removed

### Added

- `boot-bind` rebuilds the shadow module when the stock one changes, so a
  firmware update cannot leave stale QML bound to new libraries, and drops the
  environment override entirely if the rebuild fails or if it finds no
  compositor running - a persistent override could otherwise repeat a failure
  on every boot
- `status` reports `live`, whether the running compositor actually has the mod,
  separately from `applied`

## [0.2.4] - 2026-09-14

### Fixed

- The boot hook no longer forces the panel on. `apply` calls `turnOnScreen`
  before relaunching Home because a compositor restart can leave the panel
  blanked; on the boot path that turned a set back on after a crash-recovery
  reboot-to-suspend. The hook now marks itself as the boot path and the panel
  is left to whatever the system decided

## [0.2.3] - 2026-09-14

### Fixed

- 0.2.2 could leave the compositor on stock QML under the modded layout: a
  black Home with only the sidebar and dock. It judged whether the compositor
  predated the binds by `/proc/<pid>`'s mtime, which on this kernel is only the
  time the `/proc` entry was last instantiated, not the process start. The
  decision now uses the kernel's own record (`/proc/<pid>/stat` field 22), in
  seconds since boot, and the bind time is recorded the same way and scoped to
  the boot id, so it cannot be confused by an unsynchronised wall clock or a
  record left by a previous boot
- The shader-failure check is anchored on a log line count taken before the
  restart again, never a timestamp: the log spans boots and its early lines
  carry an unsynchronised clock

### Changed

- 0.2.2 claimed cold boot no longer restarts the compositor. That was wrong,
  and the measurement behind it was the same bogus timestamp. The compositor
  starts at boot+2 s and Homebrew Channel's hooks run at boot+20 s, and `/etc`
  is a read-only overlay with no way to order a unit ahead of it, so a cold
  boot always takes exactly one restart. `boot-bind` stays: it makes that
  restart happen as early as the hook allows, and any later compositor start
  picks the modded QML up on its own

## [0.2.2] - 2026-09-14

### Changed

- Cold boot no longer restarts the compositor. The compositor starts about two
  seconds after Homebrew Channel's hooks, and a full apply spent ~5 s staging
  before its binds landed, so it read stock QML and had to be restarted: a
  fifteen-second black screen on every boot. The boot hook now binds the
  previous boot's verified set first, in well under a second, so the compositor
  loads the modded QML on its own first start
- `apply` is idempotent. A staged set identical to what is already live is left
  mounted rather than drained and re-bound, and the compositor is judged against
  the time that content actually went live. A no-op apply now takes about a
  second and makes no restart
- `fps` goes through `apply`. The inline version bound both QML paths, which
  are one inode, and stacked two mounts

### Added

- `boot-bind` and `boot-ok` verbs for the boot hook. `boot-bind` only mounts a
  set that a full apply has verified on this firmware, and skips itself once
  after a boot that never reported ok

## [0.2.1] - 2026-09-14

### Fixed

- Boot hook no longer holds Homebrew Channel's failsafe window open. `startup.sh`
  arms a failsafe flag, runs `run-parts`, then clears the flag ten seconds later;
  applying inline took around 45 seconds, so a set powered off in that window came
  back in failsafe mode with every root customization disabled. The hook now
  returns immediately and applies in a detached child
- `apply` only restarts the compositor when it has to. If the compositor started
  after the binds went live it already loaded the modded QML, and if the panel is
  not active the binds are left staged for its next start. Both cases previously
  cost a needless restart, one of them on the boot path
- `apply` and `revert` take a lock. A detached boot apply can now overlap a user
  launching the app, and two concurrent runs could leave the wrong number of
  mounts live

## [0.2.0] - 2026-09-14

### Added

- News card in the bottom-left corner: photo, headline and as much of the summary
  as fits, rotating every 10 seconds. Top world and US stories come from keyless
  public RSS (BBC News, falling back to The New York Times), so the widget ships
  with no API key and no quota

## [0.1.0] - 2026-09-14

### Added

- Initial release: LumaGlass homebrew app for rooted LG webOS 10 TVs
- Liquid glass compositor theme with Rose Pine color palette
- Abstract wallpaper displayed through transparent Home screen
- Decluttered home layout with refactored UI strings
- `apply` command: Install mod via bind mounts, restart services
- `revert` command: Remove mod, restore original assets, restart compositor
- `persist` command: Enable/disable state persistence across reboots
- `fps` command: Toggle compositor FPS debug overlay
- `ntfy` command: Configure ntfy.sh integration for notifications
- `status` command: Display current mod state and configuration
- `log` command: View debug logs from recent operations
- Web UI app built with Homebrew Channel integration
- Self-updating via GitHub Releases
- Custom Homebrew Channel repository support
- Non-destructive install: all changes are mount-based, easily reverted
- Tested on LG webOS 10.2.1 (32-bit ARM userspace, Rockhopper chassis)

### Technical

- Build tooling: POSIX shell scripts, no webOS SDK required
- Deployment: ar-based IPK packaging, deterministic builds
- CI/CD: GitHub Actions + Gitea mirror workflows
- Compositor QML: Custom materials and shaders for glass effect
- Home app modifications: XML layout, image overlays, localization strings
- Persistent state: `/var/lib/lumaglass/` with auto-backup of original assets
