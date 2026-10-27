# Surfaces under LumaTV's compositor: the shell client

Build-order items 2 to 4 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`), the interim route: LumaTV's
existing QML (LumaHome with its Settings layer; the volume display, toasts, alerts and the PIN prompt; the quick panel
and the input picker; the screensaver) in a standalone 32-bit Qt client on LG's own Qt 5.12, as two windows of LumaTV's
compositor.

## Resume here

State (2026-10-27): the compositor's side is built and host-tested (`shell.rs`, the shell window types in `window.rs`
and `scene.rs`, the hook in `system_ui.rs`; `tests/shell.rs`, unit tests in `shell.rs`). The client (`compositor/shell/`),
the tools wiring, the gate checks and the TV runs are next. Nothing has run on the set yet.

## What was built

- `shell.rs`: the shell client's supervisor (starts `/usr/bin/qml-runner` on `<bundle>/shell/Shell.qml` with LG's Qt
  apps' environment, restarts it with a backoff, gives up after 5 starts in 10 minutes; `ctl shell restart`) and the UI
  channel: an HTTP server on the loopback (`--shell-port`, default 18750) the client long-polls as it polls LumaTV's
  helper, since QML has no socket of its own and the set has no QtWebSockets. A root-only token file
  (`<runtime dir>/lumatv-shell.token`) gates every request. Topics to the UI: `foreground` (app, whether Home is drawn),
  `screen`, `cursor`, `keyboard`, `audio` (the compositor's own volume subscription), `homekey`; events `show`
  (quick, picker, drawer; `warm` at the gear's press) and `luna` (answers by tag). From the UI: `can`, `wants`, `state`,
  luna calls from an allowlist of bus names (made by the bridge in the compositor's name; qml-runner's role reaches the
  app manager only), `cancel`.
- `window.rs`, `scene.rs`: window types `_WEBOS_WINDOW_TYPE_LUMATV_HOME` (`ShellHome`, layer `Home`) and
  `_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM` (`ShellSystem`, layer `Shell`, above the keyboard), or the titles `LumaTV shell:
  home` / `LumaTV shell: system`. Home is drawn in the card slot, in place of the Home stub's card, while that card is the
  foreground one or there is no card; under an app it is left out. The system layer takes the focus and the pointer only
  while the UI says it wants them (`takes_keys`). Both are exposed whole while mapped.
- Keys: on `wl_keyboard`, after the compositor's routing. A shell window with the focus gets every key but Home, Volume
  and Mute (Back and Exit included, closing nothing: `back_pressed` leaves the shell's windows alone). The gear's tap,
  Input and the gear's hold ask the shell first (`system_ui.rs`, `can`), the interim app launches stay the fallback;
  `showQuickSettings`, `showInputPicker` and `systemUIManager/launch` of the two panels reach the shell too.
- One token counter for every outbound luna call (`luna_call` shared the bridge's token space with the subscriptions).
