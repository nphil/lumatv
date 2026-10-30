# Camera pop-up: position and the layout editor

**Position (Nitin, 2026-10-30: "closer to its corner, much less padding").** The pop-up and the notification cards share one column zone (`ZoneKind::Column`, `placement/src/model.rs`): half of Home's larger safe margin from the screen's edges, 32 px (never under the toasts' own 24), down to the bottom quarter's top. The card ends at x 1888 and starts at y 32; it was 64 px from the right edge and 134 from the top. Over Home it covers the status bar's right end while it is up. `theme.json` `overlays.top` and `overlays.right` are 32 for the QML card (`Cameras.qml` reads both), and `placement/tests/shipped.rs` holds them to the model. The zone is not held to the lint's safe area (64, 54), as the toasts' corner is not; the floor is the one number to change if the card must stay inside it (`Frame::column_inset`).

**Placeable in Settings › Home & look › Layout of pop-ups.** The camera pop-up was already an item of the editor; it is now called "Camera pop-up". "Edit in preview" opens the editor, one row per item opens it on that item, and the owner picks the card up with OK and moves it with the arrows (corners, thirds, middle; Reset this, Reset all). What the owner places is `layout.overlays.popup` (an anchor and offsets), and the card, the cards under it, and the live video window claimed for its picture box all follow it (`shell.rs` test: the claim's rectangle is the card's picture box wherever the card is).

**Every overlay, checked.**

| Overlay | In the editor | Where it is otherwise |
|---|---|---|
| Volume bar | yes (`volume`: left, bottom or right edge) | |
| Mute mark | yes (`mute`) | |
| Camera pop-up | yes (`popup`) | |
| Notification cards | yes (`cards`) | |
| Toasts | yes (`toast`) | |
| Input pop-up (alert) | with the toasts: it sits at the toasts' place | `alert_surface.rs` |
| Notification drawer, quick panel | no: right-edge sheets | a sheet is the screen's edge by design, with its own width and keys |
| Input picker | no: a glass sheet rising from the bottom edge | |
| App switcher, menus, confirm dialogs, Settings, the PIN prompt | no: modal screens | centred or beside what opened them |
| Input pill and channel banner | no | the foot of the screen, 64 from the left (a decision of 2026-10-29: banners are not items) |

The sheets and dialogs are not pop-ups that can sit anywhere: moving a sheet means choosing the edge it slides from, which each panel's layout and its slide would have to be mirrored for. They are listed here so that the question is answered once; none was added.
