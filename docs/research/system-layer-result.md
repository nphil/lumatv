# System layer in Rust: status

Module `rustui/ui/src/system/`. Kinds in the shell: `osd`, `toast` (core), `alert` (this slice, wired). The rest of the layer is partly built.

| Piece | State |
|---|---|
| Alerts, input popup, PIN (`alert.rs`, `alert_surface.rs`) | Done, wired as kind `alert`; QML cases ported, lint clean at 100 % and 200 % text, dark and light |
| Overlay queue (`cards.rs`, from `Overlays.js`) and plugin hub (`plugins.rs`, from `PluginHub`) | Models done with the QML tests ported (class order, dwell, wait, merge, doorbell, score, keys after the arm time); no surface and not wired: kind `cards` is not answered yet |
| Saver numbers, clock raster, model (`saver/look.rs`, `clock.rs`, `model.rs`) | Done and tested (cap, dim, drift, spots, vote on tvpower's lists, wake, standby); the surface (date, weather, notices, slots) and the wiring are not built, so kind `saver` is not answered yet and no saver render exists |
| Saver rules and slots (helper, crate `lumatv-saver`) | Done: `/v1/saver/*`, the `saver` topic, enforcement at the camera pop-up and the plugin post (`docs/research/saver-complications.md`) |
| Camera pop-up, capture mark, layout editor, mute-mark drift | Not started in Rust; the QML ones stay in force |

QML changes: the interim relief of the saver (`clockLevel` 0.4, `dimTo` 0.4 over 20 min), Home counts as busy while its saver is up (a camera
pop-up no longer covers it), and the compositor's `status` carries `oledcare.saver`.

## TV checks (next round)

1. Saver on Home: clock at 0.4 and dimmer, over 20 minutes the dim reaches 0.4 of that; wake on a key, the pointer and Back.
2. A camera event while the saver is up: no video over it (default `camera.popup`: hide); the event is a line in `lines` of the `saver` topic.
3. A doorbell while the saver is up (QML saver owned by LumaTV): it does not wake by itself (the next key does); with LG's saver (state `Screen Saver`) it wakes the screen.
4. `GET /v1/saver/rules` on the TV lists the shipped classes and one row per installed plugin; `POST` changes one and `saver-rules.json` appears in the data directory.
5. `ctl status` shows `oledcare.saver` true while the saver is up.
