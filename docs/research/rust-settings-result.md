# Rust Settings: result

The Settings kind of the Rust shell (`rustui/ui/src/settings/`, kind `settings`, `ctl shell rust +settings`). It is opt-in: the QML Settings
stays the default until the TV gate has passed twice; no QML was touched.

## Resume here

Every page of the QML Settings now has a Rust page (below), so the `settings` kind can be switched on as a whole on the TV. What is left is
TV-side, not code:

1. Text entry (Wi-Fi password, TV name, profile name, PIN) goes through RustPanels' `entry` kind: run with `ctl shell rust +settings +entry`
   (the request is cancelled at once without it). Layout of pop-ups' editor is SystemResume's `layout` kind (`Shell::open_layout`).
2. TV gate for the whole kind (twice): rail, every page opened with the Magic Remote and with the D-pad, 100 % and 200 % text, both looks.
3. AI access is deliberately read-and-reduce only (see "AI access: what the backend allows" below); Nitin should know before the kind is
   switched on that widening an assistant's access is not possible from the Rust screen, only from the QML one's remote prompt.

Open in the code: none for Settings. Not exercised on the TV: everything (host renders and tests only).

## Pages

Framework: `settings/{model,view,row,page,ctx,src,host}.rs` (pure model: rail, tabs, rows, held-OK help, confirm, small picker, owner locks,
debounced writes `Op::Later`, theme patches, page hooks), `surface/` (flowing focus on every row, windowed list, pointer hover = cue / click =
select, glyph hints, text size 100-200 %), `host/sio.rs` (LG's settings service). `pages/plugin_rows.rs` is `PluginRows.js`: the rows of a
plugin's own settings page from the schema the helper serves (switch, slider, up to three answers as segments, longer lists and the
plugin's own lists as rows with a check, read-only facts); Home Assistant and Cameras use it.

A page hears `owner:<id>` (an owner's document was taken) and `topic:<name>` (a helper topic changed) through its `answered` function; that
is how a plugin page asks its lists again when the room or the link changes.

Rail: Privacy, Profiles, Picture, Sound, Inputs & devices, Network, Ambient light, Home & look, Remote & pointer, AI access, System. Under
them: Time & Date, Power On/Off, Language & location, Safety, Audio Guidance, Audio options, Emergency, Closed Caption, Support, App
Permissions, Delete My Personal Information, Screenshots & recording, Fine Tune, White Balance, HDMI Signalling, Equalizer, HDMI audio, USB
sound, Receiver, Channels, Bluetooth speakers and devices, Wi-Fi, IP Settings, Layout of pop-ups, Home widgets, Home Assistant, Cameras (and
one camera's kinds), During the screensaver, App key.

## AI access: what the backend allows (for a security follow-up)

The page (`pages/ai.rs`) reads the helper's `ai` topic and `GET /v1/ai/audit`. It sends only what gives an assistant LESS, at once:
`POST /v1/ai/listener {enabled:false}`, `POST /v1/ai/assistants/{id}/pause`, `DELETE /v1/ai/assistants/{id}`, `POST /v1/ai/assistants/{id}/levels`
with a level that is lower (never when it is higher), `POST /v1/ai/limits` with a lower volume ceiling, `POST /v1/ai/lockout`, `DELETE /v1/ai/pairing`.
Whatever gives MORE (access on, a pairing code, a wider level, resuming an assistant, a higher limit) opens a dialog "Not available here" that
says the phone's Face ID approval is not connected to this screen, and sends nothing. The page has no way to answer a prompt, and nothing
else in the shell does (the `ai.prompt` topic is only shown).

What the backend does (`helper/app/src/ai/{gate,routes}.rs`), recorded as found:
- The helper's HTTP API is on loopback, `127.0.0.1`, and takes `POST /v1/ai/listener {enabled:true}`, `/v1/ai/pairing`, `/v1/ai/assistants/{id}/resume`,
  `/v1/ai/assistants/{id}/levels` (higher) and `/v1/ai/limits` (higher) from any local caller; each opens a prompt on the TV rather than
  acting. Any process on the TV, and anything that can reach the helper's API, can therefore ask.
- The prompt is approved by a key press (OK, evdev 28 or 352, released after a 300 ms display) from the devices named `LGE RCU` or
  `LGE M-RCU*`, read by the helper from the input devices itself. Keys from `Smart Remote RCU Input` and `LGE Network Input` (Home Assistant's
  `webostv.button`, SSAP) do not count. A person with the remote can approve; whether a device that injects events under one of those names
  could is not established here. This is "a physical remote", not proof of a person in the sense of Principle 0, and it is not the phone gate.
- There is no route that answers a prompt, and none that creates a token (`routes.rs`).
- Gap: the phone gate (Face ID, HomeLabber; `/data/home/Homelabber/design-notes/human-control-gate.md`) has no endpoint the TV or the helper calls, so
  nothing here could route widening to it. When it exists, the controls that now open "Not available here" are the ones to connect, and the
  helper's remote-key approval is the one to retire.

## Cameras

`pages/cameras.rs`, "Settings, Home & look, Cameras" of `cameras-ui-spec.md`: tabs Scrypted (connection chip, device name, verifier in groups of
eight, Forget link with a question), Cameras (which to show), Pop-ups, Notify for (a row per camera with its kinds as the value, opening that
camera's page of switches in the order picked; defaults person and doorbell) and Video (segments). The Cameras app's grid `Settings` button
opens it (`Shell::cameras_asks`, page id `cameras`).

## Tests and renders

`cargo test -p lumatv-ui` (settings: 171 pages and model; includes `every_icon_a_settings_page_names_is_in_the_icon_set`), `clippy -D warnings`,
`tests/test_rust_settings.py` (AI: reductions are sent, widening sends nothing; Home Assistant lists asked again on a room change and a pick
written to the plugin's page; one camera's switch written in the order picked; every new page lint-clean in both looks at both text sizes),
`tests/render_rust_settings.py` (scenes: `--list`; `play()` returns the helper's requests). Lean renders (1280-wide JPEG) are in
`docs/mocks/host-rust/settings/`: pages one by one in the dark at 100 %, the saver page with its picker, and light and 200 % samples. Every
scene was also run through the host's layout and contrast lint at dark and light, 100 % and 200 %: no findings.

Known, outside Settings: `payload/.../lucide` has no `move` glyph, which `home/editor.rs` and `panels/menu*.rs` name.
At 200 % text the five tabs of Home Assistant and Cameras do not all fit the tab strip; the strip follows the focus.
