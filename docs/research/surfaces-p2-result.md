# Surfaces, phase 2 onward (LumaTV)

Warm-up and the frame gate, the preload trim, the helper wiring, boot and power-off logos, then notifications,
quick panel, Settings, AI access, input picker, receiver OSD, splash, restyles, apps grid, drawer and screensaver.
Plan: `docs/PLAN.md` (Rules every surface meets, Phases). Previous owner's notes: `docs/research/surfaces-p1-result.md`.

## Resume here

State at 2026-10-25 04:35 EDT. Milestones 1-4 are built, tested offline and pushed (HEAD `458fc0b` and later); what the set
has run so far is the warm-up A/B and the Home part of the gate (slot 1). The second slot (install, gate on Quick Settings and
the gallery, preload A/B, logos on the set) is queued with Main; its numbers go under "Log" and the Status line of PLAN.md.

Order (changed 2026-10-25 02:27 by the owner: the boot and power-off logos move up to follow milestone 3):

1. Startup warm-up and the per-surface frame gate. Built, on the set once; see Log.
2. Preload trim and the Home-return re-measure. Tool and A/B driver built (`lumatv preload on|off|status`,
   `docs/research/perf/preload_ab.py`); the measurement waits for slot 2.
3. Helper wiring. Built (`lumatv helper start|stop|status`, start in apply, stop in revert, Home's cards from
   `kit/Helper.qml`); the set has not run it yet.
4. Boot and power-off logos. Three options of each rendered to `docs/mocks/logo`; the windows for LG's `BootLogo` and
   `MasterBrand` are built (`lumatv logo on|off|status`); the set has not run them yet.
5. Toasts, alerts, PIN prompt, input popups: NOT STARTED. Design below.
6. Quick panel, Settings, Remote and pointer page, AI access. Not started.
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
- Plan: keep LG's data layer and contract, restyle and re-cost the visuals. Stage a copy of `SystemUI/Notification` like
  Quick Settings (`systemui_stage`), patch `NotificationStyle.qml` (815 lines: every colour, size, radius and font of toast,
  alert, input popup and PIN prompt) with the tokens, drop the per-toast `layer.enabled` DropShadow (`Toast.qml`: a live blur
  per toast, the cost the gate forbids) for the kit's static shadow, set Manrope, and gate every card with `gate.py`
  (toast, alert, modal alert, input popup, PIN prompt each need a key or luna sequence that shows them: `luna://
  com.webos.notification/createToast`, `createAlert`). The PIN prompt (734 + 171 lines) answers only to the remote and is
  tested with Nitin present.
- Messages that are never filtered or delayed (PLAN, Surfaces): EAS, thermal shutdown, Magic Remote battery, LG Remote
  Service sessions, permission and pairing prompts. They are restyled only where the restyle cannot fail.

Working rules for this run:

- The set is shared: ask Main for a slot ("TV slot for SurfacesP2b: N min, read-only|deploy|measure"), wait for "go SurfacesP2b",
  send "TV back" when done.
- Offline QML checks run on the workstation: `tests/qmlhost/qmlhost.py` (PyQt5 5.15 from the wheel on Xvfb with software GL;
  `pip install PyQt5` and an X display; `tests/xdisplay.py` starts one) loads a surface with stand-ins for LG's modules
  (`tests/qmlhost/stubs`) and a fake helper (`fakehelper.py`): load errors, screenshots, scripted life cycle (`--fire`),
  probes (`--eval`), no timing. Timing only counts from the set. `python3 -m unittest discover -s tests -p 'test_*.py'` (68
  tests, about 55 s with the host tests) and `TZ=America/New_York bun test tests/` (36).
- Deploy loop: `sh tools/build-ipk.sh` (needs `helper/dist/lumatv-helper`), push to `/tmp/lumatv.ipk`, `luna-send -n 4 -f
  luna://com.webos.appInstallService/dev/install '{"id":"org.nphil.lumatv","ipkUrl":"/tmp/lumatv.ipk","subscribe":true}'`, wait
  for the app directory to match (the install is asynchronous and returns before the files are in place), `lumatv apply` (17 s).
  The paramiko wrapper is `/tmp/lt/tv.py`; `docs/research/perf/startup.py`, `gate.py` and `preload_ab.py` use `run.py`'s `Set`.
- Commit and push (`git push -q forge main`) after each milestone; stage only owned paths.

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
| Home D-pad | 27.8 ms | 2.79 ms (533 frames) | 3.02 | 6 of 172 animated frames in cycle 0 | -0.00 MB |
| Home sweep | 25.7 ms | 2.91 ms (250 frames) | 3.25 | 0 | +0.64 MB (allowed 1.08) |

Under LG's compositor, 17 % of frames take two vblanks for a trivial client (X1), so the D-pad's 6 of 172 (3.5 %) is under
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
