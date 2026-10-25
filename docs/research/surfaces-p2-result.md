# Surfaces, phase 2 onward (LumaTV)

Warm-up and the frame gate, the preload trim, the helper wiring, boot and power-off logos, then notifications,
quick panel, Settings, AI access, input picker, receiver OSD, splash, restyles, apps grid, drawer and screensaver.
Plan: `docs/PLAN.md` (Rules every surface meets, Phases). Previous owner's notes: `docs/research/surfaces-p1-result.md`.

## Resume here

State at 2026-10-25 06:30 EDT (SurfacesP2c, the restart after the credit cut-off). Milestones 1-6 are built in part, tested offline
and pushed; what the set has run so far is the warm-up A/B and the Home part of the gate (slot 1). Slot 2 is booked 06:55-07:45 after
CompositorC2c (Main: "go SurfacesP2c"); skip the startup A/B and the reboot if time is short.

Slot 2 (the ipk is `/tmp/sp2c-slot2.ipk`, built from a clean `git archive` of `3f8cd14` so none of the unfinished work goes with it:
`sh tools/build-ipk.sh` in an extracted archive with `helper/dist/lumatv-helper` copied in; it carries the SAM fix). Before the old
tool runs on the set, `ls /var/lib/lumatv/tile-backup | wc -l` must be 0 (the installed tool restarts SAM when it restores a
tile icon): install the new ipk first, so the old tool never runs. After every compositor restart check `webapp-mgr` is active and
LumaHome is in front (recovery in the HANDOFF). End state: Active Standby, receiver unmuted, preload trim and logo option 1 ON if
they pass. Helpers for it are in `/tmp/sp2c/slot.py` (outside the repo: preflight, install, health, the logo capture).

1. Preflight, install the ipk, `lumatv apply`, `lumatv helper status`, `lumatv surfaces`; Home's weather and news from the helper.
2. `gate.py --surfaces quick-settings,kit-gallery`, then `--surfaces toast,alert` with LG's own views, `lumatv notifications on`,
   the same again, then `notifications off` unless Main wants it kept.
3. `preload_ab.py --rounds 4` (keep `lumatv preload on` if the Home-return gaps are no worse and memory is better).
4. Logos: `lumatv logo on`; one power-off through `tvpower/power/powerOff {"reason":"remoteKey"}` with a capture loop running (the
   MasterBrand window, about 3 s), which leaves the set in Active Standby. BootLogo needs a cold boot: only with time to spare.
5. If time is left: `startup.py --runs 2 --sweep-at 2,4`.

Order of the milestones (changed 2026-10-25 02:27 by the owner: the boot and power-off logos move up to follow milestone 3):

1. Startup warm-up and the per-surface frame gate. Built, on the set once; see Log.
2. Preload trim and the Home-return re-measure. Tool and A/B driver built (`lumatv preload on|off|status`,
   `docs/research/perf/preload_ab.py`); the measurement waits for slot 2.
3. Helper wiring. Built (`lumatv helper start|stop|status`, start in apply, stop in revert, Home's cards from
   `kit/Helper.qml`); the set has not run it yet.
4. Boot and power-off logos. Three options of each rendered to `docs/mocks/logo`; the windows for LG's `BootLogo` and
   `MasterBrand` are built (`lumatv logo on|off|status`); the set has not run them yet.
5. Toasts, alerts, input popups, PIN prompt. Built offline for the toast and the alert (Log); the set has not run them.
   Not done: the input popup in the mock's look (`18-device-connected`: its LG cards are angled and cost a live clip each),
   a title and an app icon on the alert, and the PIN prompt (kept LG's black full-screen layout: it is a security screen and
   its test needs Nitin).
6. Quick panel, Settings, Remote and pointer page, AI access. Built offline: Settings as a layer of Home (rail, page, the gear's
   popover has "All settings"), the Home & look and AI access pages, the AI prompt over Home, all against a fake helper. Not
   started: the Picture, Sound, Inputs, Network, Ambient light, Remote & pointer and System pages, the Quick panel redesign, and a
   window of its own so that Settings and the prompt open over apps.
7. Input picker, receiver-fed volume and mute OSD, receiver page and tile. Not started.
8. Launch splash, spinners, context menu, on-screen remote. Not started.
9. Live TV banner, keyboard, search and voice bar, shared web-app theme. Not started.
10. Apps tile to the own grid, Notification Centre to the own drawer. Not started.
11. Own screensaver and art mode. Not started.

Also built, outside the list (asked for by CompositorC2b): the enable path for LumaTV's own compositor in the tool
(`tools/compositor.sh`, `lumatv compositor on|off|status`, `compositor-boot` in the boot worker, the live-compositor guards).

Design for milestone 5, worked out against LG's source (`/data/home/lgtv/uiinv/qml/qml`):

- The hook is `sysuicompmgr/changeSystemUIView {viewName: "notificationView", viewSource: file://...}`; LG's container
  (`StarfishNotificationView.qml`) loads the file as an item and falls back to LG's own source when it fails to load
  (error 102). The choice is persisted by LG in `/var/preferences/starfish-luna-surface-manager/systemUiViews.json`; the
  way back is the same call with LG's source.
- The loaded item's contract is in `SystemUI/Notification/NotificationView.qml`: it is a `SystemUIView` filling its parent and
  given `style`, `systemSettings`, `access`, `inputPopups`, `cursorVisible`, `allowScrim`; it exposes `alertActive`,
  `modalAlertActive`, `advancedAlertActive`, `pincodePromptActive`, the four `...HasActiveFocus`, `toastCount`, the signals
  `requestFocus(component, type)`, `releaseFocus`, `notificationReceived`, `opened`, `closed`, `clearMouseFocus`,
  `updateCursorFocus`, and the functions `closeAlert`, `closeModalAlert`, `closeAdvancedAlert`, `closePincodePrompt`,
  `closeToast`, `closeNotifications(obj)`. Its data layer is `Services.qml` (826 lines: the luna subscriptions and the
  four models) and `NotificationModel.qml`.
- Plan, as built: keep LG's data layer and contract; stage a fresh copy of `SystemUI/Notification` on the set at every
  `notifications on` (nothing of LG's is shipped), patch it with `tools/notification_patch.py` and lay LumaTV's own files over it
  (`payload/notification`). The patch is anchored text: Manrope everywhere, LG's red focus colour is the accent, the alert's
  and its buttons' colours and radius are the palette's, the scrim is the palette's, the alert is centred, the live DropShadow of
  `SimpleButton` and the PIN pad's keys is switched off (the alert card's own shadow is nine static images and stays), and
  ToastView's three placement lines put the toast at the top right. The toast is replaced outright by `payload/notification/Toast.qml`
  (the same interface ToastView reads: properties, three signals, the open/close `state`, `rotateToastView()`), with the palette
  filled into its `@@name@@` markers at stage time. Every anchor must match exactly once or the whole stage fails and LG's view stays.
  Gate every card with `gate.py` (`--surfaces toast,alert`); the PIN prompt (734 + 171 lines) answers only to the remote and is
  tested with Nitin present, so it keeps LG's layout.
- Messages that are never filtered or delayed (PLAN, Surfaces): EAS, thermal shutdown, Magic Remote battery, LG Remote
  Service sessions, permission and pairing prompts. They are restyled only where the restyle cannot fail.

Working rules for this run:

- The set is shared: ask Main for a slot ("TV slot for SurfacesP2c: N min, read-only|deploy|measure"), wait for "go SurfacesP2c",
  send "TV back" when done.
- Offline QML checks run on the workstation (PyQt5 5.15 from the wheel on Xvfb with software GL; `pip install PyQt5` and an X
  display; `tests/xdisplay.py` starts one): `tests/qmlhost/qmlhost.py` loads a compositor surface with stand-ins for LG's modules
  (`tests/qmlhost/stubs`) and a fake helper (`fakehelper.py`): load errors, screenshots, scripted life cycle (`--fire`), probes
  (`--eval`); `tests/qmlhost/notify.py` runs LG's real notification view (the census copy of its tree, `/data/home/lgtv/uiinv`) with
  LG's own data layer behind stand-ins for what is native (`notify_stubs`): `--patched` applies the patch tool first, `--toast`,
  `--alert`, `--input`, `--pin MS[:JSON]` publish a notification through `Services.qml`'s own subscription handler, `--shot
  MS:PATH` takes a picture (`--backdrop` a screenshot of Home from `qmlhost.py LumaHome.qml --shot`). No timing: Qt 5.15 on
  software GL says nothing about 60 fps; the gate on the set is the only timing. `python3 -m unittest discover -s tests -p
  'test_*.py'` (about 110 tests, about 70 s with the host ones) and `TZ=America/New_York bun test tests/` (36).
- Deploy loop: `sh tools/build-ipk.sh` (needs `helper/dist/lumatv-helper`), push to `/tmp/lumatv.ipk`, `luna-send -n 4 -f
  luna://com.webos.appInstallService/dev/install '{"id":"org.nphil.lumatv","ipkUrl":"/tmp/lumatv.ipk","subscribe":true}'`, wait
  for the app directory to match (the install is asynchronous and returns before the files are in place), `lumatv apply` (17 s).
  The paramiko wrapper is `/tmp/lt/tv.py`; `docs/research/perf/startup.py`, `gate.py` and `preload_ab.py` use `run.py`'s `Set`.
- Commit and push (`git push -q forge main`) after each milestone; stage only owned paths.
- Python bytecode is ignored everywhere (`.gitignore`); nothing generated is committed.

## Log

### Reviewed from the cut-off run (d548e1e)

`Warmup.qml` and `kit/Warm.qml` were sound and are kept as written: a hidden 1 %-opacity pass, one step at a time, after Home
has presented (1.5 s), over the text styles found by walking Home's item tree plus the type roles, then images and shader
components that surfaces ask for through `Warm.request()`. On the set it logged "28 text styles ... in 3202 ms" and
"26 ... in 2800 ms" in the host. `gate.py` needed three corrections (below). The logo files were complete for options 1 and 2;
option 3 (the name alone, `LogoLetters.qml`, `LogoOn3.qml`, `LogoOff3.qml`) was added.

### Slot 1 (2026-10-25 03:00-03:20, deploy and measure): warm-up and the Home gate

Installed the HEAD build and applied (17 s; helper not yet part of it). Startup A/B, `startup.py --runs 2 --sweep-at 10`, order
off, on, on, off, compositor restarted by `lumatv restart` each time:

| | blank to first present after the restart call | D-pad sweep at 10 s: animated frames over 20 ms inside the animation | animated render p90 |
|---|---|---|---|
| warm-up off | 5.06 and 5.00 s | 6 and 0 | 3.38 and 2.80 ms |
| warm-up on | 4.95 and 5.04 s | 3 and 0 | 2.88 and 3.16 ms |

No measurable difference at 10 s: the warm-up runs after first present (so it cannot move that) and finished at about 4.7 s,
well before the sweep. Whether it helps a key pressed earlier (2 and 4 s after first present, where it is still running) is
what `startup.py --sweep-at 2,4,10` answers; not yet measured. Not claimed: any gain from the warm-up.

Gate on Home (`gate.py`, 3 full-trace cycles per surface, then 10 basic cycles), numbers re-derived from the saved traces with
the corrected rules (the first run's rules counted wrongly, below):

| surface | first frame | pooled p90 render | worst window p90 | drops inside the animation | RSS over 8 cycles |
|---|---|---|---|---|---|
| Home idle | not checked | 2.57 ms (69 frames) | 2.57 | 0 | not soaked |
| Home D-pad | 27.8 ms | 2.79 ms (533 frames) | 3.02 | 6 of 533 animated frames (0, 1 and 5 in the three cycles) | -0.00 MB |
| Home sweep | 25.7 ms | 2.91 ms (250 frames) | 3.25 | 0 | +0.64 MB (allowed 1.08) |

Under LG's compositor, 17 % of frames take two vblanks for a trivial client (X1), so the D-pad's 6 of 533 (1.1 %) is under
that floor; the gate holds LG's compositor to it and LumaTV's own to zero (`LG_COMPOSITOR_DROP_SHARE`). The gate was stopped
at Quick Settings to return the set for another agent's slot; Quick Settings and the kit gallery have no number from the new
gate yet.

What the first gate run got wrong, all fixed in `gate.py` and `analyse.py` with unit tests (`tests/test_gate.py`, 34):

- It took the p90 of each window and compared the worst to 3 ms; windows of 6-15 frames made that noise (a 3.02 ms window failed
  a surface whose 533 frames had p90 2.79). It now pools every animated frame of the surface (`burst_anim_samples`) and keeps a
  4 ms per-window limit so one bad cycle is still caught.
- `dropped` counted the first animated frame after an idle gap, which is late by construction (the update that starts it fires
  when the key does): 60 of them on the D-pad over 3 cycles. `analyse.py` now reports `dropped_in_motion` (a frame over 20 ms
  whose previous presented frame was animated too) and the gate uses it.
- Home idle's windows are timer-driven (10 updates a second), so "first frame" is not a response to a key; an empty burst there
  is no longer a dead key. The idle cycle is 3 s long (the 5 s one overflowed the full trace's 8 MB ring on this loaded set:
  41,000 events a second, and the start marker was overwritten in 1 of 3 captures).
- Left unchanged: the window's `worst_ms` of 100 ms is the wallpaper's 10 Hz tick on idle Home, not a stall.

Left on the set by this slot: nothing running (capture and `kb.py` stopped, `/tmp/perf` emptied), theme `warmup.enabled` true,
set in Active Standby.

### Offline, 2026-10-25 05:10-05:55: the notification view

Docs folded first (PLAN Status, Phases, Native helper, AI access, Hardware census; HANDOFF; CHANGELOG): lean declutter applied and
checked on the set, the helper's AI smoke 42 of 42 and its plugin broker, the census's top five, the owner's decisions. Python
bytecode that had been committed was untracked.

`tests/qmlhost/notify.py` runs LG's actual `SystemUI/Notification` tree offline, so a notification can be looked at before the set
is asked. It found that this firmware's `Services.qml` takes its data from the `com.webos.notification` subscription (the
`getToastNotification`, `getAlertNotification`, `getInputAlertNotification`, `getPincodePromptNotification` replies), not from
the `WebOSServices` models, so the stand-in for luna answers those replies and everything above them is LG's. What it showed:

- LG's toast is a grey bar (#787F86) centred at the bottom, 900 px wide, one line of 30 px text; its alert is a light grey card
  (#CACBCC at 95 %) 180 px above the bottom with the buttons stacked on the right and no title unless it is a full alert; both hard-
  code the colours of `SimpleButton` in code, outside the style object; the style's `variations.defaultView` repeats the base
  values and is bound over them, so the patch has to change both. The PIN prompt is a black full-screen sheet.
- After the patch: the toast is a glass pill at the top right under the status bar (right edge level with it, as in
  `docs/mocks/07-toast` and `35-toast-advanced`), with an icon tile, a title over a dimmer subtitle when the message has two lines,
  an accent chevron and a label (`actionLabel`, optional) for an advanced toast's link; the alert is a centred dark-glass card with
  a hairline edge, ink text, button fills from the palette, a focused button with a light 2 px edge, over the palette's scrim.
  Verified here: 25 tests (`tests/test_notification_patch.py`) including a toast, an alert, an input popup and a PIN prompt through
  the patched view with no warning from the tree (LG's own PIN pad writes through a null `KeyNavigation.left` and is filtered).
- A modal alert refuses an input popup and a PIN prompt (`acceptAdvancedAlerts`, `acceptPincodePrompts` in LG's view), which is LG's
  rule and kept; the test says so.
- Not done and why: the input popup's LG layout (angled cards, each drawn through a live `ShaderEffectSource` clip) needs its own
  rebuild for the mock's card with two buttons and a countdown line (`docs/mocks/18-device-connected`), the preview cannot show LG's
  native angled shapes (they are stubs here), and the alert has no title or app icon (LG shows the title only for full alerts).
  None of it has run on the set: `lumatv notifications on` and the toast/alert gate wait for slot 2.

### Offline, 2026-10-25 05:55-06:30: the SAM fix, Settings, the AI page and the prompt

Main's safety fix first: `revert_tile_icons` restarted `sam.service` and `homelaunchpoints.service` whenever it put back an icon edit
of an older version (and `apply` calls it every time). It now logs and restarts nothing, and `tests/test_tool_rules.py` fails if any
shell tool restarts, stops or kills SAM. The tool installed on the set still has the old code: it restarts SAM only if
`/var/lib/lumatv/tile-backup` holds files (`tileicons.js revert` deletes each backup it restores).

Settings (`Settings.qml`, `settings/`) is a modal layer of Home, built on the first open and kept; Home passes the D-pad as
`key(dir)` and `closeRequested` closes it; a page asks the host through `themePatch`, `themePersist` and `launchApp`, so it owns no
file. Home's widgets, dock and bar fade out behind it (the wallpaper settings popover's `previewMode`). Pages: Home & look (dynamic
tint = `accent: "wallpaper"`, appearance = `material`, clock style, wallpaper motion; the rows `LuRow` with `LuToggle` or
`LuSegmented`) and AI access. The rail lists only pages that exist and ends with the way to LG's Settings.

AI access (`settings/PageAi.qml`) follows the helper's `ai` topic: the listener switch, pairing, each assistant (pause or resume,
remove with a second press), what the chosen assistant may do (four groups, Always, Ask on TV, Never, the whole levels object sent),
recent activity from `/v1/ai/audit`, lock-out (second press). It can ask for anything and give less at once; whatever gives more
returns 202 and a prompt opens. `AiPrompt.qml` draws `ai.prompt` and Home returns before any handling of the direction, OK and
Back keys while it is up; the Home key still works. It shows over Home only: over an app it needs the notification view or a
window of its own.

Checked here: 12 tests in `tests/test_settings.py` through the offline host with a fake helper that models `/v1/ai` (a request for
more opens a prompt that only the test's own `/__approve` answers, as only the physical remote does): the switch writes the theme and
the user's file; the keys never reach Home behind Settings; turning it on only asks and an open prompt swallows every key; off is
sent at once; remove and lock-out need a second press; a lower level is sent whole and a higher one only asks; approving turns it on
and the page follows. The full offline suite is 126 tests plus the 36 of the JavaScript side, all passing. The host also now parks
the pointer in the corner (Xvfb's mid-screen pointer hovered a row and moved the test's focus) and lets XHR write local files as
Qt 5.12 on the set does.
