# Camera video windows and the panel rate; Back resuming the app

## Resume here

Done offline (2026-10-30, no TV), host tests green:

- **Cause.** The kernel sets the panel's output rate from the decoder's rate at every timing change of the main window (`_vsc_check_fr`: the
  25/50 family gives 100 Hz, everything else 120 Hz). A camera at 25 fps (Cat Feeder: 25.0 a second in the helper log) therefore re-timed the
  panel to 100 Hz at the pop-up's first frame; nothing puts the rate back when the window closes (`v4l2_vsc_event.c`, `default: return`).
  The helper's return-to-120 watcher never ran for it (a pop-up leaves Home in front, so SAM's foreground subscription says nothing), and its
  write, `0 60` to `/proc/lg/de/fr_rate`, did not take when it did run (Frames120, from 04:42: "wrote 0 60 ... output measures 99.9 Hz"; a
  compositor restart did not help either, only a playing YouTube video did). The compositor itself writes nothing.
- **Why the write failed** [INFERENCE, LG's source]: the request reaches the display firmware as `MCU_DE_IO_SetFrRate`, which switches the
  display clock oscillator's tracking of the source on (`VIDEO_CTRL_TRACE_DCO_FCW`) and sets the sync type. With a source the oscillator
  re-locks to the new rate; with none it holds the old one. The kernel's own request is the same `0 60`; it works only because a video is
  there. Free run (`1 60`, `isForceFreeRun`) switches the tracking off.
- **Fix 1, prevention** (helper `live/mod.rs`, `cameras.directVideo`, default true): a camera's main-window NDL load sets the frame-drop
  threshold 2 (the GeForce NOW feeder's value). A threshold above 0 makes the decoder a direct video (`vo.c` `drop_threshold > 0`,
  `VDISP_MATCH_DIRECT_VIDEO`), whose rate is fixed at 60 (`vdisp_core.c` `vrate_update_fixed_rate(60, 1)`), so the kernel reads 60 whatever the
  camera runs at and keeps 120 Hz. The sub window (starfish pipeline) and the texture path are untouched: only the main window's rate counts.
  A direct video's first frame is also a timing change, so it takes a panel that was left at 100 Hz back to 120.
- **Fix 2, restore** (`helper/app/src/framerate.rs`): the write is `1 60` (free run). The check also runs when a camera stream ends
  (`cameras.live` idle or failed) with Home in front, not only at a foreground change.
- **Fix 3, Back** (`compositor server/webos_shell.rs` `front_app`, `POST /v1/front {app}`, shell `ReturnToApp`): the full view's Back raised
  the app by `applicationManager/launch {id}`. YouTube's appinfo has `handlesRelaunch` false, so a launch of the running instance reloads
  it at its start page [INFERENCE: matches the report; LG's own relaunch of a preloaded app is the same call]. Back now raises the app's
  card as a `set_state(fullscreen)` would (Home minimized, the foreground pushed to SAM's subscribers) and launches only an app that has no
  card left. Journal line `front: APP (window N) raised to the front, not launched again`, or `front: APP has no card, launching it`.

## TV check (Round22)

1. Install the helper, compositor and shell. Settings as shipped (`cameras.directVideo` true). `T ctl refresh` / compositor log.
2. From the current 100 Hz state (idle Home): send a pop-up event. Pass: the compositor log has `kms: the panel refreshes every 10.00 ms now
   (20.00 before)` (the direct video's first frame, or `20.00 ms` to `16.68 ms` at 60 mode) and then stays; the feeder log's `loaded:` line shows
   `drop threshold 2`.
3. Cold, from 120 Hz: pop-up on Home. Pass: no `refreshes every` line at all during or after it; `T cat /proc/interrupts | grep osd_irq` over 2 s
   is 240 (120 Hz). Repeat over the full view and over YouTube's pop-up (texture path: no hardware window, no change expected).
4. If a pop-up still re-times: `cameras.directVideo` false in `helper.json` and the end of the stream should log
   `frame rate: Home was left at 100.0 Hz with no video window open; set back to 120 Hz` (the `1 60` write). If it logs `wrote 1 60 ... but the
   output measures 100 Hz`, free run does not take either; the lever left is a short 24/30/60 fps video (what YouTube did).
5. Full view over YouTube, Back: journal `front: youtube.leanback.v4 ... raised to the front, not launched again`; YouTube is in front and its
   video continues. If YouTube is blank or at its start page, note which (journal `foreground` and SAM's `getForegroundAppInfo`); the
   fallback is the old launch (revert `shell.rs` `ReturnToApp` to `applicationManager/launch`).

Not verified on the set: all of it. The inferred links are the firmware's reaction to free run and whether SAM treats a raised card as
foreground without a launch.
