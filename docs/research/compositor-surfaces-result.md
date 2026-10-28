# Surfaces under LumaTV's compositor: the shell client

Build-order items 2 to 4 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`), the interim route: LumaTV's
existing QML (LumaHome with its Settings layer; the volume display, toasts, alerts and the PIN prompt; the quick panel
and the input picker; the screensaver) in a standalone 32-bit Qt client on LG's own Qt 5.12, as two windows of LumaTV's
compositor. Under LG's compositor nothing changes: the shadow module keeps running inside it.

## Resume here

Round 6 (2026-10-28, `1fadfcb`, `compositor/slot-2026-10-28-0200/`; details and classification in `compositor-gate9-result.md`,
"Round 6"): the shell client runs again after a fix (a per-start spawn thread's PR_SET_PDEATHSIG killed it 0 s after every
start on 4198076). With the shell up: keys, ime PASS; shell 9 of 11 (`saver`: tvpower refuses `turnOnScreenSaver` with -100,
also under LG's compositor; `osd` once in three runs); frames: home-dpad, quick ok, picker ok in the full gate (burst_over20 3
in the earlier run), toast first frame 61-118 ms (round 5: 28-52 ms), so the toast item below is still open and worse.

State (2026-10-27, after round 5 on 12a1d0b, `compositor/slot-2026-10-27-2100/`): the picker passes the frame gate (first frame
27-34 ms, `burst_over20` 0, callbacks ahead 3522 of 4272); keys, ime and shell pass; the full gate passed every check
except x1 (`compositor-gate9-result.md`, "Round 5") and, in that run only, standby and homekey (a harness timer, fixed;
both passed in a run of their own). The toast's frame gate is on its limit: first frame 51.5 ms in one run, 50.0 ms (worst
of three opens) in the other, limit 50; with the shell's start-up in the trial (the other open item) it is what stands
between this build and go-live. Next, in order:

1. The toast's first frame from `Q:toast-delivered` (37.5, 51.5, 28.0 and 44.4, 50.0, 24.2 ms): read the journal's `shell:
   system layer window mapped` line against `delivered_ms`; the system window's first showing after its last hide is the
   candidate. Whether 50 ms is the right limit for the toast's first frame (the stock toast's baseline is in `gate.py`) is
   Main's.
2. Input on Nitin's remote, once: a tap shows the picker at the press; a hold of 1 s opens the Home Hub and the picker goes
   with it.
3. The held digit: its press also reaches SAM, which launches the Quick Access slot's app (`com.webos.app.browser`, 0.6 s
   into the hold) beside the compositor's own Quick Access (`com.webos.app.magicnum`, at 1.0 s). The gate closes the
   browser; whether `keys.rs` should keep the press from SAM is open.
4. Idle Home ("Open" below): the tick held while nothing visible changes.
5. The keyboard's place (section "The keyboard's place"): open LG's Search 5 times (a fresh MaliitServer on the first, a warm one after);
   the keyboard must be at the bottom each time. Per open read `ime-journal.txt`: the `the keyboard is on the screen` line (the panel
   should be 1920x438 at (0, 642) whatever `its window` and `input region` say) and `ime-up.json`'s `panel`; the gate's `ime` now fails a
   panel 540 rows or more high. A run with a full-screen window and no input region is not covered by the fix: the journal line says.

Likeliest failures on the set now: the toast's first frame (above), an app left open by a key (`gate.sh shell` starts with Home
not in front: `connected` fails with `drawn false`, and the frame gate's session then covers Home), the idle cost over the
threshold (a client CPU over a third of a core at nice 10 would be the Qt render of the full scene per tick), keys not
arriving (`ctl shell` `ui.keys` stays 0 while `ctl keys` counts them delivered: the focus went elsewhere, `ctl windows`
and the journal's `focus ->` lines say where).

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
- Frame callbacks: every client's go out when its frame's flip lands (`State::frames_in_flight`); a shell window's go out when its
  frame is the oldest on its way, and the buffers its commits replaced are given back when the frame lands (`Owed`), so the
  shell makes its next frame while the last waits for its refresh. `ctl callbacks shell early|landing` switches it; `ctl
  status` `callbacksAhead` counts it.

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

### Round 5 (12a1d0b, 2026-10-27, `compositor/slot-2026-10-27-2100/`)

Ipk, bundle and `cutover install` of 12a1d0b through one HomeLabber plan (the plan before it expired unanswered; the set was
untouched). `tv_gate.py --only keys,ime,shell,frames`: keys, ime and shell PASS (shell 11 of 11: idle Home 10 commits/s at
3 %, covered 0 %, 176 luna calls, 0 refused; the held digit's browser is closed by the gate). Frames FAIL on the toast only:

- `home-dpad` ok (first frame 28.0 ms), `quick` ok (28.1 ms, burst end 162.0 ms, `burst_over20` 0).
- `picker` ok, 6 of 6 windows: first frame 27.4 ms (the compositor asks the shell 0.17 ms after the key), burst end 171.8 ms
  (limit 180; the other opens 142-161 ms), `burst_over20` 0 (round 4: 5), `show_ms` 0.14-0.17 on the opens. The callbacks
  were in play: `shell callbacks: 3522 of 4272 sent ahead of their flip`. No picker trace needed copying.
- `toast` NOT: `windows` 3 of 3; the opens' `first_frame_ms` (from `Q:toast-delivered`) are 37.5, 51.5 and 28.0 ms (limit
  50), `delivered_ms` 116.1, 31.2 and 91.7; `burst_end_ms` none in the middle open and `burst_over20` 1 in the third. The
  failed value is 51.5 ms against 50; the full gate's frame gate (below) passed the toast with its worst open at 50.0 ms
  (44.4, 50.0 and 24.2 ms), so the toast sits on the limit. The OSD wake of round 4 is gone.

The full gate (`tv_gate.py --skip volume`, same bundle): frames PASS (home-dpad 28.8 ms; quick 28.0 ms, burst end 161.5;
picker 33.8 ms, burst end 166.7, `burst_over20` 0; toast 50.0 ms, burst end 83.3, windows 3 of 3; `shell callbacks: 3566 of
8956`), firmware, homeheld, live, netflix (Dolby Vision, picture mode back to filmMaker), hdmi, keys, ime, shell,
wd-hang, wd-crash and escape PASS; x1 FAIL (below); standby and homekey FAIL for a harness reason: the frame gate's
`enable --session` finds the first session live and does not re-arm its 12 minute timer, which ended the session before
the standby step (`mounted no`). `tv_gate.py` now sizes the first session for the frame gate too. `--only standby,homekey`
then PASSED both (PreparePowerOff 1, PreparePowerOn 1, no fallback; homekey: unmounted after 10 s, escaped, SAM unchanged).
The volume check, by hand in a session because the driver's receiver reads were stubbed: the receiver went 0.55 to 0.555
on Volume+ and back to 0.55 on Volume-: PASS.


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

**The picker's 33.4 ms intervals: the shell's frame starts at the flip, and its CPU and GPU time leave the flip nothing to
spare.** The slides have 5 of them in about 34 intervals: opens 1, 2 and 0, closes 1, 0 and 1 (the gate's `burst_over20` 5 also
counts the interval after open 2's stray frame). Each has the GPU busy 13.0-13.5 ms in 4 jobs, qml-runner 2.2-3.5 ms of CPU on
one thread, the compositor's thread under 1.1 ms (the client's buffer goes to the plane whole), and the GPU's first power-up
1.8-2.9 ms after the flip that began the interval. The shell's frame starts at the flip (the compositor fires a client's frame
callbacks when its flip lands, and Qt makes a frame on its callback), so it is 15.4-16.4 ms of CPU and GPU away from a flip that
needs the GPU done before its refresh (`kms/timing.rs`: a page the GPU finishes after the driver's cutoff slips a refresh,
16 gaps of 20-33 ms in the second gate's X1). One plain frame of Home in the D-pad's windows (11.0 ms of GPU, 3.2 ms of CPU)
missed the same way, and the quick panel's frames, a millisecond or two lighter, had none in about 48 intervals. Under LG's
compositor the picker had 0, 1 and 0 (rounds 12-14): its QML ran in the compositor's own process, with no client's callback
between a frame and the next. A slide's first frame is late the same way: in the second open the show reached the compositor
at about 50 ms, the GPU job started at 54 ms and the frame showed at 81 ms, a refresh after the one at 64 ms.

Fix (b193b28): a shell window's frame callback goes out when its frame is the oldest on its way (at its draw when none is ahead
of it, else when the one ahead lands) and not when it lands, so the shell makes frame k+1 while frame k waits for its refresh, and
the buffers its commits replaced are given back when the frame lands (a client let run ahead takes the first buffer it is given
back, and the one on the screen must not be drawn into). Other clients' callbacks stay at the flip (X1 paces to the refresh).
The shell's 15.4-16.4 ms then run from the draw of the frame before, with the GPU done before the compositor's latch (7 ms
before the refresh) instead of at the refresh. Expected [INFERENCE: not run on the set]: no 33.4 ms interval in a slide; the
burst ends 5-17 ms later than it would, each frame being made a refresh ahead of its display (150-165 ms from the key, limit
180); first frames unchanged. `ctl callbacks shell landing` and `tv_gate.py --frames-callbacks landing` turn it off for an A/B.
Host tests: `flips.rs` (a shell window's callback at its draw, behind a frame on its way, its replaced buffer held to the
landing, the switch off, a card's behaviour unchanged).

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

## The keyboard's place

Under LumaTV's compositor LG's keyboard (maliit, through `input_panel`) came at the top of the screen once and at the bottom once in
Search. The gate's `panel y .. h ..` lines over six runs are 1920x438 at y 642 twice (the first TV run, `compositor-step1-result.md`, and
`slot-2026-10-27-2100/gate-full`) and
1920x1080 at y 0 four times (`gate-2026-10-26`, `slot-2026-10-27-1900`, `slot-2026-10-27-2100/gate-keys-ime-shell-frames`), the keyboard
drawn in the top 438 rows of a transparent full-screen window (Main's read of `ime.png`). The gate called both "at the bottom": it only
checked that y plus h is 1080, which a window 1080 high at y 0 meets.

Cause: the compositor placed the keyboard by the size of maliit's window (bottom edge of the output minus the window's height, as LG's
`KeyboardView.qml` does with its item), and the window's size is maliit's: the QML's states set it with `PluginProxy.setPanelSize`
(`view-global/main.qml`, 1920x438 for the normal keyboard), and the root item sits at y 0 of the window. When the window stays at the
screen's size the whole window is "the keyboard" and its content is at the top. What decides 1080 over 438 on a run is on the set's side
of the wire: nothing in the compositor's events, text-model fields or input-panel geometry differs between the runs (the app's
`set_input_panel_rect` was not journaled; LG's `WaylandInputPanel` sends maliit no geometry and `set_toplevel`'s position is "not
supported" there too). Maliit's own wayland plugin is open-source upstream and makes a full-screen window whose input region is the
keyboard's area; KWin reads that region as the window's geometry for the same quirk. [INFERENCE: the webOS fork sets the same region; the
journal's new line says.]

Fix: the keyboard is the bounding box of the window's input region (the whole window when it sets none, or one that misses it), and the
window is placed, and the field told (`input_panel_rect`, `ctl ime` `panel`), by that box: full-width centre, bottom edge of the output,
or the corner of the rectangle the app asked for (`scene::keyboard_geometry`, `keyboard_origin`; `ime::panel_rect`). A window of the
keyboard's own size is placed as before. The panel is re-read at every commit of a keyboard window, so a region changing alone moves it.
The journal's `the keyboard is on the screen` line now gives the window's size, its input region, the part taken as the keyboard and the
field's asked-for rectangle, and `set_input_panel_rect` is journaled, so a run that still comes out wrong shows why. The gate's `ime`
needs the panel under 540 rows as well as ending at 1080. Host test `tests/ime.rs`
`the_keyboard_is_at_the_bottom_whatever_size_maliit_makes_its_window` (five window shapes; before the change the full-screen window at y 0
with the keyboard in its top rows came out as a 1920x1080 panel at (0, 0)).

## Open

- The picker's 33.4 ms frame intervals with the shell's callbacks ahead, and the toast's first frame from the compositor's marker, on the set (Resume); the full gate.
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
