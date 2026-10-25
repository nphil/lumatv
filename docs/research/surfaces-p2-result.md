# Surfaces, phase 2 onward (LumaTV)

Warm-up and the frame gate, the preload trim, the helper wiring, boot and power-off logos, then notifications,
quick panel, Settings, AI access, input picker, receiver OSD, splash, restyles, apps grid, drawer and screensaver.
Plan: `docs/PLAN.md` (Rules every surface meets, Phases). Previous owner's notes: `docs/research/surfaces-p1-result.md`.

## Resume here

State at 2026-10-25 02:40 EDT. Nothing is built yet; the first slot on the set has not run.

Order (changed 2026-10-25 02:27 by the owner: the boot and power-off logos move up to follow milestone 3):

1. Startup warm-up and the per-surface frame gate.
2. Preload trim and the Home-return re-measure.
3. Helper wiring (start and stop in `tools/lumatv`, Home's weather and news and the receiver from the helper).
4. Boot and power-off logos (LG's logo replaced on power-on and power-off; three options for the owner to pick from).
5. Toasts, alerts, PIN prompt, input popups.
6. Quick panel, Settings, Remote and pointer page, AI access.
7. Input picker, receiver-fed volume and mute OSD, receiver page and tile.
8. Launch splash, spinners, context menu, on-screen remote.
9. Live TV banner, keyboard, search and voice bar, shared web-app theme.
10. Apps tile to the own grid, Notification Centre to the own drawer.
11. Own screensaver and art mode.

Working rules for this run:

- The set is shared: ask Main for a slot ("TV slot for SurfacesP2: N min, read-only|deploy|measure"), wait for "go SurfacesP2",
  send "TV back" when done.
- Offline QML checks run on the workstation (`tests/qmlhost`, PyQt5 5.15 on Xvfb with software GL): load errors and
  screenshots, no timing. Timing only counts from the set.
- Commit and push (`git push -q forge main`) after each milestone; stage only owned paths.

## Log

Nothing yet.
