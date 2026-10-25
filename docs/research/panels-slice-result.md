# Panels slice: quick panel, input picker, receiver OSD and page

Surfaces: `panels/QuickPanel.qml`, `panels/InputPicker.qml`, `panels/VolumeOsd.qml`, `settings/PageReceiver.qml`,
`settings/PluginTiles.qml`, `kit/Inputs.qml`, `kit/ReceiverGate.qml`; helper side `helper/app/src/{receiver,hub}.rs`.
Scenes: `tests/qmlhost/scenes_panels.py` (08, 10, 11 x4, 24 x5). Tests: `tests/test_panels.py`, `tests/test_volume_osd.py`,
`helper/app/tests/receiver.rs`. Nothing here has run on the set.

## Resume here

State at 2026-10-25, offline only.

- Quick panel (mock 10): resident, as tall as its tiles; Picture mode, the receiver tile (label from the model, "Denon
  receiver"), Wi-Fi, plugin tiles, AI access, then the All settings button one Down past the last tile. Plugin tiles come
  from `PluginHub.tileRows()` through `settings/PluginTiles.qml`: rebuilt only when the set of tiles changes, focus kept by
  tile id, OK on a tile is `POST /v1/plugins/<id>/actions/<action>`, a tile that is unavailable or observed never takes the
  focus. The Ambient light tile arrives as a plugin tile (PluginsSlice publishes `plugin.ambient.tiles`). No sleep timer
  tile: the settings key for it is not known offline.
- Input picker (mock 08): tile data is `Inputs.tiles(theme.inputs)` (Live TV, each HDMI port by Auto / Always / Hidden, device
  icon from the CEC name, receiver marked eARC and Sound out, sound line "Sound: Denon on HDMI 2"). Home's Inputs row (PLAN,
  "Inputs on Home") is ShellSlice's to place: it draws the same list. Not done on Home: the mock 08 backdrop has no such row.
- Receiver readings (OSD, tile, page): one gate, `kit/ReceiverGate.qml`. A value is shown while `Helper.up`, the topic says
  `fresh` and `reachable`, and its `updatedAt` is under 6 s old. The helper moves that stamp quietly (`Hub::set_ignoring`:
  a poll that changes nothing raises no event), so the copy a surface holds ages out between real changes; a surface that is on
  screen then asks `GET /v1/state/receiver` and takes the newer stamp, otherwise it shows nothing (the OSD asks only when a
  volume key is pressed). The helper itself publishes `fresh: false` 6 s after the last good read.
- Receiver page (mock 24): draws only what the topic has. The helper publishes power, input, volume and mute (HTTP 8080
  Lite status; the full status page answers 403) and has no `/v1/receiver/set`, so the page says it cannot send commands and
  dims every control. Scenes `24-settings-receiver*` publish the full contract (sound modes, Audyssey, levels, Quick Select,
  Zone 2) as a fixture so the design is checkable; `24-settings-receiver-today` is the page against what the helper publishes.

Needs the TV or a decision: command route (`SI`, `MS`, `PS...` over the HTTP goform path) and a read-back for sound mode,
Audyssey, levels and zones (no readback on port 8080; Telnet is Home Assistant's, a second client has not been tried);
Quick Settings and picker first-frame and p90 numbers (`gate.py --surfaces quick,picker`); the CEC power state of an input
("Connected · standby" in the mock) has no source in `eim`.

## Contracts

- Tile descriptors: `PluginHub.tileRows()` rows (`kind` toggle or value, `id` `plugin.<pluginId>.<tileId>`, `icon`, `label`,
  `detail()`, `enabled()`, `on()`, `act()`), unchanged by this slice.
- `Inputs.tiles(choices)`: `[{appId, key, port, icon, name, state, on, connected, front, sound}]`.
- `ReceiverGate { active, confirmAged, staleMs }`: `fresh`, `doc` (the newest copy while in date, else null), `rc`, `stale`,
  `disabled`, `reading(doc)`, `confirm()`.

## Log

- Hosts serialise on the fixed helper port (flock in `tests/qmlhost/qmlhost.py`); the fake helper records other POSTs at
  `/__posts`, serves `GET /v1/state/<topic>` and takes `POST /__quiet/<topic>` (a retained value changes with no event).
- Helper: the per-poll receiver event the recovery had introduced is gone; `set_ignoring` is back with its test, and
  `polls_that_change_nothing_move_the_stamp_in_the_snapshot_and_raise_no_event` replaces the heartbeat test (10 receiver tests,
  283 library tests pass).
