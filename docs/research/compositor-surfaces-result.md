# Surfaces under LumaTV's compositor: the shell client

Build-order items 2 to 4 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`), the interim route: LumaTV's
existing QML (LumaHome with its Settings layer; the volume display, toasts, alerts and the PIN prompt; the quick panel
and the input picker; the screensaver) in a standalone 32-bit Qt client on LG's own Qt 5.12, as two windows of LumaTV's
compositor. Under LG's compositor nothing changes: the shadow module keeps running inside it.

## Resume here

State (2026-10-27, after round 4 on f0d3cb4, `compositor/slot-2026-10-27-1900/`): the picker's and the toast's frame-gate
failures have a cause and a fix ("The picker and the toast of the frame gate" below: f903648, bc25983), host-tested and not yet
on the set. `gate.sh shell` passed 11 of 11 alone in round 4, and keys, ime and shell passed in `--only keys,ime,shell,frames`.
Next, in order:

1. One TV round of `tv_gate.py --only keys,ime,shell,frames` with the bundle of HEAD installed. The frame gate must show:
   - picker: `first_frame_ms` about 31 ms (limit 50), `burst_end_ms` about 148 (limit 180), `windows` 6/6, and `show_ms` under
     1 ms on the opens (the compositor asks the shell at the press). `burst_over20` is the open question: round 4 had 5
     intervals of 33.4 ms in its six windows (opens 1, 2 and 0, closes 1, 0 and 1). Where any remain, read the GPU busy time
     of those intervals in `frames.json` (12.1-13.5 ms in round 4, against 11.0 ms for a plain frame of Home) and cut the
     picker's GPU work per frame (its glass quad is 1.0 Mpx, the quick panel's 0.7) before touching the pacing.
   - toast: `windows` 3/3, and per window `sent_ms`, `delivered_ms` and `first_frame_ms` (now from the compositor's
     `Q:toast-delivered`): the shell's own time from being handed the toast (limit 50). If it is over, read the journal's
     `shell: system layer window mapped` line against `delivered_ms`: the system window's first showing after its last hide is
     the candidate.
   - the keys gate's `input-shell` line (journal `Input tap: ... LumaTV's input picker`) now comes at the press.
2. Input on Nitin's remote, once: a tap shows the picker at the press; a hold of 1 s opens the Home Hub and the picker goes
   with it.
3. The held digit: its press also reaches SAM, which launches the Quick Access slot's app (`com.webos.app.browser`, 0.6 s
   into the hold) beside the compositor's own Quick Access (`com.webos.app.magicnum`, at 1.0 s). The gate now closes
   the browser; whether `keys.rs` should keep the press from SAM is open.
4. The full gate (not run in round 4), then idle Home ("Open" below): the tick held while nothing visible changes.

Likeliest failures on the set now: the picker's 33.4 ms frame intervals (above), the toast's first frame from the compositor's
marker, an app left open by a key (`gate.sh shell` starts with Home not in front: `connected` fails with `drawn false`, and
the frame gate's session then covers Home), the idle cost over the threshold (a client CPU over a third of a core at nice 10
would be the Qt render of the full scene per tick), keys not arriving (`ctl shell` `ui.keys` stays 0 while `ctl keys` counts
them delivered: the focus went elsewhere, `ctl windows` and the journal's `focus ->` lines say where).

## What was built

### The compositor's side (`compositor/crates/lumatv-compositor/src/shell.rs`)

- The supervisor: starts `/usr/bin/qml-runner` (SAM's QML runner: LG's Qt, QML and Wayland client stack in one 32-bit
  process) on `<bundle>/shell/Shell.qml` with LG's Qt apps' environment (`QT_QPA_PLATFORM=wayland`,
  `QT_WAYLAND_SHELL_INTEGRATION=webos`, `QT_WAYLAND_HARDWARE_INTEGRATION=wayland-egl`, `QT_IM_MODULE=wayland`,
  `QML2_IMPORT_PATH=<bundle>/shell/imports:/usr/lib/qt5/qml`, as read off `ai-inference-manager` on the set), its first
  argument the launch JSON SAM gives the runner (`appId`, `main`, `params`), stdout and stderr in
  `<data dir>/compositor/shell.log`, `PR_SET_PDEATHSIG` so it dies with the compositor. Restarted after an exit with a
  backoff (2 s after a long run, 2 to 32 s after a quick death); given up after 5 starts in 10 minutes (`ctl shell
  restart` tries again). `--shell off|auto|PATH`: `auto` (the default) runs it with `--input` when the bundle beside the
  executable has `shell/Shell.qml`.
- The UI channel: an HTTP server on the loopback (`--shell-port`, default 18750), on a thread of its own, that the client
  long-polls the way it polls LumaTV's helper (`kit/Helper.qml`): QML has no socket of its own and the set has no
  QtWebSockets, so the plan's JSON lines on a Unix socket became JSON over loopback HTTP. Every request carries the token
  of `<runtime dir>/lumatv-shell.token` (written 0600 at the start; another local process cannot drive the channel).
  `GET /v1/state` (the topics), `GET /v1/events?after=SEQ&timeout=25` (held until an event or the timeout; a client
  further behind than the 512 events kept is told to read the state again), `POST /v1/ui` (`can`, `wants`, `state`),
  `POST /v1/luna` (`tag`, `uri`, `payload`, `subscribe`), `POST /v1/cancel`. Topics: `foreground` {app, home: whether
  LumaTV's Home is drawn}, `screen` {on}, `keyboard` {up}, `cursor` {visible}, `audio` (the audio service's volume
  state, the compositor's own subscription); events `show` {kind, warm, n}, `homekey`, `luna` {tag, payload}.
- Luna for the client: the bridge makes its calls in the compositor's name from an allowlist of bus names (the services
  LumaTV's QML calls under LG's compositor and the compositor's own); qml-runner's role reaches the app manager only. A
  one-shot answer frees its tag, a subscription's answers keep coming until `cancel`; a client that is gone (exited,
  or no poll for 40 s) has every subscription cancelled at the bridge.
- `window.rs`, `scene.rs`: `_WEBOS_WINDOW_TYPE_LUMATV_HOME` (`ShellHome`, layer `Home`, under the cards) and
  `_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM` (`ShellSystem`, layer `Shell`, above the keyboard); a window titled `LumaTV shell:
  home` or `LumaTV shell: system` (wl_shell or the webos shell's `title`) is the same, and keeps that through a later
  default type. LumaTV's Home is drawn in the card slot, in place of the Home stub's card (which draws nothing), while
  that card is the foreground one or there is no card at all; under an app's card it is left out of the stack. SAM's
  foreground stays the stub. The system layer takes the keyboard focus and the pointer only while the client says it
  wants them (`takes_keys`); otherwise both go through it to what is under it. Both windows are exposed whole while
  mapped (LG's Qt plugin does not render before `exposed`).
- Keys stay on `wl_keyboard` (lower latency than the channel, and the compositor's routing runs first either way): a
  shell window with the focus gets every key but Home, Volume and Mute, Back and Exit included, and `back_pressed`
  leaves the shell's windows alone. The gear's tap, Input and the gear's hold ask the shell first (`system_ui.rs`, by
  its `can`); the interim app launches of step 1 stay the fallback. The gear's press sends `show {warm: true}` so the
  panel joins the scene before the tap; Input's tap is acted on at its press while `can` has the picker (at the release
  otherwise). `showQuickSettings`, `showInputPicker` and `systemUIManager/launch` of the two
  panels reach the shell too. A Home tap while LumaTV's Home has the keys sends `homekey` (the client goes back to the
  top of Home).
- `ctl shell [restart|show KIND]`: connected, pid, `can`, `wants`, the windows' mapping, the client's `state` report
  (what is open, the OSD, toasts, alerts, the PIN prompt, the saver, keys seen, the link), the topics pushed, the open
  luna calls, the counters. `ctl luna`'s tokens: one counter for every outbound call now (`luna_call` shared the
  bridge's token space with the subscriptions before).

Host tests: unit tests in `shell.rs` (allowlist, HTTP parsing, the outbox window, the token check, the server thread
holding a long poll over TCP), `tests/shell.rs` (Home in the card slot with the stub in front and its keys; the system
layer's focus and pointer; the gear, Input and the luna methods answered by the shell; luna through the bridge with
tags, cancels and the client going).

### The client (`compositor/shell/`)

- `Shell.qml`: a `WebOSWindow` (`Eos.Window`, LG's) for Home with LumaHome loaded from the module `tools/lumatv`
  stages (`<data dir>/qml/WebOSCompositor/<module>`, found by the compositor and passed in the launch parameters);
  `hostActive` and `homeCurrent` from the `foreground` topic as the fullscreen container sets them under LG's
  compositor; mapped once Home has loaded and not while the Home stub's `lg` mode has LumaHome step aside. Nested, the
  system window: LumaOverlay (`appFront` from the topic), `panels/Notify.qml` and `panels/Screensaver.qml`
  (`hostActive` while an app is in front: on Home, LumaHome's own saver draws), mapped only while something of it
  shows. Keys reach LumaBus.js through the module's own `LumaKeyBridge.qml` (`dispatch` for Home, `dispatchOverlay`
  for the layer; the notifications and the saver take theirs first), a press while the key is down counted as the
  remote's repeat; `imports/LumaShell/Keymap.js` turns the evdev codes the compositor's keymap delivers into the Qt
  keys the QML was written against. Every second and on each change the client reports `can` (`panelReady("quick")`,
  `panelReady("picker")`, `gearReady()`), `wants` and its state.
- `imports/LumaShell/Link.qml`: the channel client (the helper's pattern: state, long poll, backoff, a 35 s watchdog).
- `imports/WebOSCompositorBase/LS.qml`, `imports/WebOSServices/Service.qml`: `LS` and `Service` as the QML knows them
  from LG's compositor, over the channel: `LS.adhoc.call` one-shot, `LS.applicationManager.launchPointsList` from a
  `listLaunchPoints` subscription (a `change` push has the list read again), `LS.audioService` with LG's property
  names from the `audio` topic (`volumeUpdated` is what the volume display listens for), `Service.call`,
  `callService`, `cancel` and the `response(method, payload, token)` signal. They shadow LG's modules through
  `QML2_IMPORT_PATH`.
- `payload/compositor/lumatv/panels/Notify.qml` (new, in the module; never built under LG's compositor): the toast
  (a pill at the top right, gone after its timeout, never a key), the alert (a card over a scrim; `buttons[].action`,
  `onCloseAction`; a `warning` is full screen; nothing is filtered), the input popup (Switch per input, Stay here, its
  timeout) and the PIN prompt (a 3x4 pad and OK, digits typed from the remote too, relayed through
  `closePincodePrompt` as LG's view relays them, cancelled on Back, closed after the third wrong try), from the four
  feeds LG's models read. `tests/test_notify_panel.py`.
- Glass across windows: a texture cannot be sampled across windows, so Home names its window (`Lu.homeWindow`), the
  overlay its flat colour (`Lu.flatBackdrop`), and a pane outside Home's window samples that (`kit/LuPane.qml`; the
  overlay no longer repoints `Lu.backdrop` when it has a window of its own). Under LG's compositor everything is one
  window and nothing changes.

Host tests: `tests/qmlhost/shellhost.py` runs the client against a stand-in of the channel (the module staged as on the
set, the shell's imports shadowing the host's stubs, `Eos.Window` stubbed, keys sent by evdev code to either window);
`tests/test_shell_client.py`: the state read and the polls, `can` once Home is in front, the quick panel on Home from a
`show` and closed by Back, the overlay's panel over an app with the layer wanting the keys and letting go, the volume
display from the audio topic, a toast and an alert from the feeds (answered by OK), the feeds subscribed through the
channel. `python3 tests/qmlhost/compile_all.py`: 134 files, 0 failed.

### Idle cost (9514551)

Home as a client pays a whole-window render and a commit for every change, where LG's compositor redrew a few pixels of
its scene: CompFrames' round 2 read ~10 commits a second from it while idle under the X1 probe, and the probe's wake-ups
lost the CPU to qml-runner. Four changes: the `foreground` topic says when Home is `covered` (under a window that covers
the whole output: an app's card, LG's Settings, the X1 overlay) and LumaHome holds its idle tick then (`idleHold`), so
nothing is drawn; the client runs at nice 10 (apps at nice 0 win the CPU); it is started once SAM's Home has mapped, or 15
s in, never interleaved with WebAppMgr's first frames (the trial's one missed refresh); and Home's buffer counts as
opaque in the scene, so Home alone on the screen goes to the plane whole and its tick costs the compositor a flip, not a
composition. On Home at rest the tick stays as the owner's theme has it (the live wallpaper at 10 fps, the clock's hands
with it). `gate.sh shell` measures both: commits a second and qml-runner's CPU over 10 s on Home at rest (at most 12 a
second and a third of a core) and with the Home Hub in front (no commit, at most 3 %); `ctl shell` carries the per-window
commit counts. The volume display cues on a change only (a `cause`, or the level or mute differing from the last report),
not on the state read at the start.

### Tools and the gate

- `build-tv.sh` copies `compositor/shell/` into the bundle; `cutover.sh install` puts it at
  `/var/lib/lumatv/compositor/shell` (the compositor finds `shell/Shell.qml` beside its `bin/`).
- `gate.sh shell` (`tv_gate.py`: `shell`, after `ime`): connected and Home drawn in the stub's place; Home loaded,
  `hostActive`, beating; the D-pad (Up from the dock reaches the widgets, Down comes back, the client counted the
  keys); the quick panel and the picker on Home on the gear and Input, Back closing each; the volume display on
  Volume+; a toast through `createToast`; the quick panel over the Home Hub with the layer wanting the keys and
  letting go on Back; the screensaver after `tvpower/power2/turnOnScreenSaver`, ended by a key. Files:
  `shell-start.json`, `shell-end.json`, `shell-quick*.txt`, `shell-picker.txt`, `shell-result.txt`.
- `gate.sh keys`: the gear and Input steps follow the routing: with the shell able to show the panel (`ctl shell`'s
  `can`), the journal line `-> LumaTV's quick panel` / `-> LumaTV's input picker` and the client's layer; without, the
  interim launches as before.
- `tv_gate.py`: `frames` (after `evidence`, in a 20-minute session of its own): `docs/research/perf/gate.py --surfaces
  home-dpad,quick,picker,toast --input-key 241 --cycles 3` under LumaTV's compositor (the gate already holds LumaTV's
  compositor to zero dropped frames; the PLAN's thresholds: animated p90 under 3 ms, none over 20 ms, first frame
  within 50 ms, burst end within 180 ms); `GATE frames:` carries each surface's failed checks and its p90, first frame
  and burst end. `--frames-cycles N`.

## TV runs

Two `tv_gate.py --only shell` runs on 2026-10-27 (sessions 21:05 on 2791f7f and 21:11 on fae341e, each ended by `cutover
disable`: LG's compositor back, the Home stub on, the receiver unmuted; `docs/research/compositor/gate-2026-10-27.txt`):

- Run 1 (2791f7f): `live` PASS; `shell` 10 of 11: connected, Home loaded and drawn in the stub's place (the client started
  3 ms after SAM's Home mapped, Home's window mapped 1.3 s later, `can show quick, picker, drawer` at 2.4 s), idle on Home
  10 commits/s at 3 % CPU, D-pad (Up to the widgets, Down back), quick panel and picker on Home by the gear and Input with
  Back closing each, the volume display, a toast, 0 commits and 0 % CPU with the Home Hub in front, the screensaver on
  `turnOnScreenSaver` and ended by a key. FAIL: the quick panel over the Home Hub: open in the client, the layer wanting
  the keys, but the system window never mapped.
- Run 2 (fae341e, configure for transient wl_shell surfaces): 10 of 11 again, now with the over-app quick panel passing
  (the system window maps 15 ms after the client shows it) and the toast's window mapped; FAIL: the volume display, the
  system window's FIRST showing of the session, which never maps (a manual probe in a third session: `osd` true in the
  client for 2 s, `systemCommits` 0). The compositor's log shows LG's `WebOSWindow` making the nested window's platform
  window with the client (a bufferless commit at start) and the client re-sending its properties at every showing; Qt's
  own expose is missing at the first one. Fixed in 5dd1785 (`exposed` sent when a shell window's properties arrive, which
  LG's plugin turns into Qt's expose), built, not yet deployed: it goes into the next combined deploy with CompFrames'.

Numbers read on the set: the client's idle cost on Home 10 commits a second (the live wallpaper's tick) at 3 % of a core
at nice 10; under an app 0 and 0 %; the luna proxy made 158-163 calls per session with none refused; the system window
maps within 15 ms of a showing (after the first); keys counted by the client matched the gate's taps.

### Round 4 (f0d3cb4, 2026-10-27, `compositor/slot-2026-10-27-1900/`)

Ipk, bundle and `cutover install` of f0d3cb4 (one HomeLabber plan; the installed set reports firmware `unverified`, so
sessions only). `--only shell`: 11 of 11 (`connected` through `saver`; idle Home 10 commits/s at 3 %, covered 0 %; 159
luna calls, 0 refused), the volume display included.

`--only keys,ime,shell,frames`, first run: keys PASS (gear, Input, Netflix button, digit tap, digit hold, Exit), ime PASS
(an input method bound, context made, keyboard at the bottom, 3 keys typed, closed by Back), shell FAIL and frames FAIL
with `com.webos.app.browser` in front of Home. The held digit of `keys` is the cause: its press also reaches SAM, which
launches the Quick Access slot's app (`com.webos.app.browser`) 0.63 s into the hold, before the compositor's own Quick
Access (`com.webos.app.magicnum`) at 1.0 s; the gate closed `magicnum` only. `tv-gate.sh` now closes the browser too.

Second run, same checks: keys, ime and shell PASS (shell 11 of 11: 10 commits/s, 3 % on Home, 0 % covered, 180 luna calls,
0 refused); frames FAIL on two of four surfaces, 3 open/close cycles each, no growth in RSS:

- `home-dpad` ok (first frame 25.1 ms, p90 render 0.5 ms), `quick` ok (first frame 21.3 ms, burst end 138.8 ms, 0
  frames over 20 ms in the burst).
- `picker` NOT: the open's first frame is 82.1 and 80.2 ms (limit 50) in two cycles, with no render burst at the key in
  those two (burst end none); the third cycle's open is 47.7 ms with its burst ending at 197.8 ms (limit 180) and 3
  frames over 20 ms; the closes are 30.7-31.3 ms to first frame, burst end 130.8-131.4 ms. `burst_over20` 5.
- `toast` NOT: `windows` 2 of 3: two of the three opens have no frame after the key (the display's OSD interrupt, which the
  analysis reports as `osd_wake_ms`, was silent at the key and came back 281 and 582 ms into those windows), so no first
  frame; the closes' first frames are 421.9, 55.5 and 980.3 ms.

## The picker and the toast of the frame gate

Cause and fix, read offline from `gate-keys-ime-shell-frames/frames.json` (the window numbers and the gaps between frames)
and the code; round 4's trace files are not kept, so what a gap leaves out is marked [INFERENCE].

**The picker: the Input tap is decided at its release, and the gate's tap holds the key for 50 ms.** The three opens differ in
their first frame (82.07, 47.65, 80.19 ms) and not in their slide: the sheet's frames run from 82.1, 81.0 and 80.2 ms to
198.8, 197.8 and 196.9 ms after the key, 117 ms each (the 110 ms slide and a frame). The 47.65 ms frame of the second open is
a Home tick ahead of the slide (its next frames are at 81.05, 97.7, 114.4, 131.0, 164.4 and 197.8 ms); it alone starts a
burst, which chains from any frame within 50 ms of the key, so the other two have none. The compositor's router decides
Input's tap at the key's release (`keys.rs`: LG's chain swallows the press and decides at the release or at 1 s), `kb.py`'s
tap releases it 50 ms after the press, and the shell then needs 30-32 ms from the `show` event to the first flip (the quick
panel, shown at its release, needs 19-21 ms). Back, which reaches the client at its press, has its first frame 30.7-31.3 ms
after it. LumaTV's key filter under LG's compositor acted on Input's press ("as LG's does", `payload/keyfilter/lumatv.js`),
and the gate times the picker from the press.

Fix (f903648): while the shell's `can` holds the picker, Input's tap is acted on at its press (`keys::Ctx::picker_ready`).
Expected: first frame about 31 ms after the press and burst end about 148 ms (were 81 and 198). A press that found no picker
still waits for its release, since the interim Hub launch must not run at the press of a hold. A hold still opens the Home Hub
once at 1 s; the picker the press opened goes with the app switch (Home's layers close with `hostActive`; the system layer's
panels with `Shell.qml`'s new `fgApp` handler). The compositor writes `Q:<panel>-show` into the kernel's trace buffer when it
asks the shell for a panel (not for the gear's warm-up); the gate reads it as `show_ms`, the key to that instant, beside
`first_frame_ms`.

**The picker's 33.4 ms intervals, not changed.** The slides have 5 of them in about 34 intervals: opens 1, 2 and 0, closes 1, 0
and 1. The gate counted `burst_over20` 5 (open 2's burst, 3 with the interval after its stray frame, and one in each of two
closes); the opens with no burst counted none, and will count theirs once the first frame is in time. Each interval has the
GPU busy 13.0-13.5 ms in its 4 jobs, the compositor's thread 0.0-0.27 ms (the client's buffer goes to the plane whole) and
qml-runner 2.2-3.5 ms; one plain frame of Home in the D-pad's windows measured 11.0 ms. The quick panel's slides had none in
about 48 intervals in round 4 (1 in the first run). [INFERENCE: a frame of Home with the 1792 px sheet is at the pipeline's
limit (the client's CPU, about 13 ms of GPU, the flip within a refresh of the compositor's latch); the sheet's glass quad is
1.0 Mpx against the quick panel's 0.7.]

**The toast: the capture lost its first events, and the surface is not the volume display.** The gate's `toast` is LG's
`createToast` through the notification service (`kb.py` `!toast`). Its cycle was 16.3 s (8 s settle on each side of the close),
longer than a full capture's ring holds (about 13 s). The two windows that were found have no `Q:toast-sent` marker and the
display's OSD interrupt silent for their first 281 and 582 ms (930 and 892 interrupts in 8.03 and 8.02 s against 965 in the
closes' 8.05 s: 34 and 70 missing at 120 a second), and the third capture has no open window at all: the oldest events of
CPU 0's ring were gone, with luna-send's return and the first frames. The stock toast's gate of 2026-10-25 (the same cycle)
also found 2 of 3 windows, and the browser run of round 4 (a busier set) read the OSD back after 1979 ms. The numbers say
nothing of the shell. What the frames do show [INFERENCE]: besides the 1 Hz ticks (1.001 s apart) the first window has frames
at 0.915, 5.135 and 5.452 s and the second at 4.994 and 5.144 s, with its first frame at 1.007 s: a toast up for about 4 s,
handed over about 0.9-1.0 s after the gate's marker (luna-send's start, the hub and the service) and drawn in the frame after.

Fix (bc25983, f903648): a cycle of the toast is 7.8 s (`settle_s` 7, `close_settle_s` 0.5) and `validate()` refuses a cycle
over 12 s; each toast has its own text (a repeated text replaces the one on the screen and leaves nothing to time); the
compositor writes `Q:toast-delivered` (and `Q:alert-delivered`) when the service's push reaches it, and the gate times a
delivery surface from that marker, else from luna-send's return, and keeps both (`sent_ms`, `delivered_ms`). The service
hands a toast over at its own pace (2-4 s after luna-send's return in the gate of 2026-10-25 under LG's compositor), which no
change of the shell touches; the shell's part is what the marker measures.

## Open

- The picker's 33.4 ms frame intervals and the toast's first frame from the compositor's marker, on the set (Resume); the full gate.
- Idle Home commits at the live wallpaper's tick (10 a second, 3 % of a core at nice 10). Parity with LG's compositor:
  there too idle Home presents one frame per tick while `wallpaperMotion.enabled` is on (`LumaHome.qml`: the phase and
  the clock's hands advance on the one 10 Hz tick, "idle Home presents one frame per tick"; the gate's `home-idle`
  surface measures exactly that), and the clock's second hand rides the same tick (`widgets/ClockWidget.qml`: "the second
  hand moves on Home's shared tick and nothing else here runs per frame"), so it adds no frame of its own. The live
  wallpaper is a chosen feature: kept. With the motion off in the theme the tick still runs for the hands; a tick that
  stops when neither the wallpaper nor the hands change is the one improvement left here. Covered Home stays at 0.
- The quick panel's "LG" button (LG's own Quick Settings, a window of LG's compositor) is not offered under LumaTV's
  compositor (`Lu.ownCompositor`, set by the overlay when it finds itself in a window of its own; "All settings" fills
  the foot). Not yet run on the set (in the next ipk).
- The PIN prompt on the set needs Nitin (a locked input); the gate has no step for it.
- The never-filtered alerts (EAS, thermal) cannot be raised on the set from here; `tests/test_notify_panel.py` draws a
  `warning` alert without buttons.
- The screensaver vote (`panels/SaverVote.qml`) now registers with tvpower in the compositor's name through the bridge;
  whether tvpower accepts the registration from `com.webos.surfacemanager` with LumaTV's client name is a TV question.
