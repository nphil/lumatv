# Rust Settings: result

The Settings kind of the Rust shell (`rustui/ui/src/settings/`, kind `settings`, `ctl shell rust +settings`). It is opt-in: the QML Settings
stays the default until the TV gate has passed twice; no QML was touched.

## Resume here

Done (parity with the QML Settings, by screen):

- Framework: `settings/{model,view,row,page,ctx,src,host}.rs` (pure model: rail, tabs, rows, held-OK help, confirm, small picker, owner
  locks, debounced writes `Op::Later`, theme patches), `surface/` (`mod, chrome, list, rows, sheets, figure, layout, paint`: flowing focus on
  every row, windowed list, pointer hover = cue / click = select, glyph hints, text size 100-200 %), `host/sio.rs` (LG's settings service, one
  shared port; the quick panel uses it too, `text()` is a String, `opt_text()` an Option).
- Pages (all with unit tests): Privacy, Profiles (+ person, picture), Home & look (+ Layout of pop-ups, During the screensaver), System (+ Time &
  Date, Power On/Off timers, Language & location, Safety, Audio Guidance, Audio options, Emergency, Closed Caption, Support, App Permissions,
  Delete My Personal Information, Screenshots & recording), Picture (+ Fine Tune, White Balance, HDMI Signalling).
- Compositor: `show {kind:"settings", page}`, `ctl shell show settings [PAGE]`, gear held; Home's Settings request opens it.

Next, in order (each is a `Page` in `settings/pages/`, registered in `pages/mod.rs::all()`; QML source named):

1. Sound (`PageSound.qml`, `PageEqualizer`, `PageHdmiAudio`, `PageUsbSound`, `PageBtAudio`, `SoundOut.js`, `PageBluetooth`, `PageBtDevice`, `PageBtHid`).
2. Inputs & devices (`PageInputs.qml`, `PageReceiver`, `PageAppKey`), Network (`PageNetwork`, `PageWifi`, `PageIp`), Ambient light (`PageAmbient`),
   Remote & pointer (`PageRemote`), AI access (`PageAi`, 331 lines), Home widgets (`PageWidgets`), Home Assistant (`PageHomeAssistant`), `PageChannels`.
3. Rail order in `pages/mod.rs`: picture, sound, inputs, network, ambient, look, remote, ai, system; Privacy and Profiles are first.
4. Text entry (Wi-Fi password, TV name, profile name, PIN): `Op::Entry` is modelled and tested; the shell only logs `ShellRequest::Text`
   until RustPanels' keyboard kind lands: call `SettingsHost::entered / entry_cancelled` with its answer (`shell.rs settings_requests`).
5. Layout editor: `ShellRequest::Layout{item}` is only logged in `shell.rs settings_requests`; call RustSystem's `open_layout(item)`.
6. Not yet written: `tests/test_rust_settings.py` (use `tests/rust_settings_host.py`, `tests/qmlhost/lgsettings.py`), `tests/render_rust_settings.py`
   (dark/light, 100/200 % into `docs/mocks/host-rust/settings/`), lint at 100/200 % (`--eval lint` is clean on the saver page), a test that every
   `.icon("..")` literal exists in `payload/compositor/lumatv/icons/lucide` (known missing: `move`, `user-round`, `chevron-up`), `docs` section in
   `rust-shell-kit.md`.
7. Open decisions taken: the kind is all-or-nothing (the compositor switches per kind), so it stays off until every section is ported; the
   saver page follows RustSystem's `/v1/saver/rules` contract; text size is the theme key `type.scale` (`Shell::reload_theme`).

Host renders of the screensaver page (dark, light, 200 %) were checked by eye and with the layout lint; they are in `/tmp/rs-shots/` on the
build machine and should be re-rendered into `docs/mocks/host-rust/settings/` by the render script above.
