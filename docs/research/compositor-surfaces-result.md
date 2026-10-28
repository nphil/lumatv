# Surfaces under LumaTV's compositor: the shell client

Build-order items 2 to 4 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`), the interim route: LumaTV's
existing QML (LumaHome with its Settings layer; the volume display, toasts, alerts and the PIN prompt; the quick panel
and the input picker; the screensaver) in a standalone 32-bit Qt client on LG's own Qt 5.12, as two windows of LumaTV's
compositor. Under LG's compositor nothing changes: the shadow module keeps running inside it.

## Resume here

State (2026-10-27): built, committed and host-tested, not yet run on the set (ee17c73 the compositor's side, 83e9611 the
client, the tools and the gate checks). Next, in order:

1. A read-only slot: `/usr/palm/applications/com.webos.exampleapp.qmlapp.search/main.qml` (LG's own QML app: is its root
   a `WebOSWindow`, how does it get an alpha window) and `strings /usr/lib/qt5/qml/Eos/Window/libeoswindowplugin.so`
   for `windowType`/`appId`. If `WebOSWindow` is not what the plan assumed, `Shell.qml` falls back to `QtQuick.Window`'s
   `Window` with the titles the compositor also classifies by.
2. The deploy, through HomeLabber (one approval plan): the ipk (`sh /tmp/r8/build.sh 83e9611`, then `install.py`: the
   module gained `panels/Notify.qml` and the backdrop rule), `sh compositor/tools/tv-first-light.sh deploy`, then
   `cutover install` (the bundle now carries `shell/`).
3. The TV runs: `python3 compositor/tools/tv_gate.py --only shell` first (one session, ~5 minutes), reading `ctl shell`,
   `compositor.log` (`shell:` lines) and `/var/lib/lumatv/compositor/shell.log` (qml-runner's output) on a failure; then
   `--only keys,ime,shell,frames` (the keys check follows the shell-first routing); then the full gate.
4. Write the numbers here ("TV runs"), the gate lines go to `docs/research/compositor/gate-<date>.txt` as before.

Likeliest failures on the set, in order, and what to read: qml-runner refusing to run without SAM (`shell.log`:
`registerNativeApp`, `Got launch on closing`; the journal: `shell: client exited`), the `Eos.Window` import or the
`WebOSWindow` root (`shell.log`: a QML error; then `--shell` at `ShellPlain.qml`, see below), a system layer that is
opaque (black over an app while the volume display shows: the Qt window got no alpha config; `QT_WAYLAND_...` has no
switch for it, so the client would need a window type LG's plugin gives alpha to, read from `inputcommon`'s windows),
keys not arriving (`ctl shell` `ui.keys` stays 0 while `ctl keys` counts them delivered: the focus went elsewhere,
`ctl windows` and the journal's `focus ->` lines say where).

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
  panel joins the scene before the tap. `showQuickSettings`, `showInputPicker` and `systemUIManager/launch` of the two
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

None yet.

## Open

- The PIN prompt on the set needs Nitin (a locked input); the gate has no step for it.
- The never-filtered alerts (EAS, thermal) cannot be raised on the set from here; `tests/test_notify_panel.py` draws a
  `warning` alert without buttons.
- LG's own Quick Settings button in the panel (`lgQuick`) asks `systemUIManager/launch com.webos.app.quicksettings`,
  which under LumaTV's compositor opens LumaTV's panel again: the button is a no-op there until LG's Settings app is
  offered in its place.
- The screensaver vote (`panels/SaverVote.qml`) now registers with tvpower in the compositor's name through the bridge;
  whether tvpower accepts the registration from `com.webos.surfacemanager` with LumaTV's client name is a TV question.
