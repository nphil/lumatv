# Camera video windows and the panel rate; Back resuming the app

## Resume here

**Back: done and passed on the set** (round 22, `888dc431`). The full view's Back used `applicationManager/launch {id}`; YouTube has
`handlesRelaunch` false, so the launch reloaded it at its start page. Back now posts `/v1/front {app}` and the compositor raises the app's own
card (`server/webos_shell.rs` `front_app`: Home minimized, the card fullscreen, SAM's subscribers told). Journal `front: youtube.leanback.v4
(window 10) raised to the front, not launched again`; YouTube was in front on the same page (before/after screenshots identical).

**Panel rate: cause known, prevention not found, restore in progress.**

- Cause: the kernel sets the output rate from the decoder's rate at a timing change of the main window (`_vsc_check_fr`: 25/50 gives 100 Hz).
  The camera stream is 25 fps: `/proc/lxvo/vdisp/vrate` reads `rate_src CODEC 25/1`, `use_fixed 0`, win0 `vFreq 250`, `connect_src VDEC` for
  the pop-up's 11 s (`framerate-r22/vr-A.txt`); the compositor logs `the panel refreshes every 20.00 ms now (16.67 before)` 0.8 s after the claim.
  The NDL load carries no frame rate (`gf_ndl.c`: width, height, codec); the 25 comes from the stream's SPS.
- A frame-drop threshold after the load (`NDL_DirectVideoSetFrameDropThreshold`, 2), which would make the decoder a direct video with a fixed
  rate of 60 (`vo.c`, `vdisp_core.c`), did NOT do that on the set: `use_fixed 0` with the threshold loaded (`888dc431`, since removed). Only
  `lowdelaymode 1` was set.
- Nothing written with no source connected restores 120 Hz: `1 60`, `0 50`, `0 60`, `1 50`, `1 60` to `/proc/lg/de/fr_rate`, each counted 1.2 s
  later, left `osd_irq` at 102 a second (`framerate-r22/vr-B.txt`). What restores it is a 30/60 fps video playing for a few seconds (YouTube),
  the kernel's own `0 60` at that video's first frame. The firmware therefore seems to take a rate request only while a video window is
  connected [INFERENCE; the next reading tests it].
- The helper's return-to-120 watcher never fired for a pop-up (Home stays in front, so SAM's foreground subscription says nothing).

**Now in the helper** (`framerate.rs`): the watcher also listens to `cameras.live`. When a camera stream starts to play with Home in front it
waits 1 s (the kernel re-times at the first frame), measures `osd_irq`, and at 100 Hz writes `1 60` (free run) with the source still connected,
then measures again. When a stream ends or fails, an ordinary check runs. Log lines: `frame rate: Home measures X Hz with a source connected to
the main window`, `frame rate: Home was left at X Hz; set back to 120 Hz (Y Hz measured)`. A 25 fps stream at 120 Hz is shown in a 5-5-5-5-4
cadence, fine for a camera. The check is limited to Home in front, so an app's own 25/50 fps video keeps LG's rate.

## TV check (Round22, after the frames gates)

1. From 120 Hz, a pop-up on Home. Pass: the helper log has `Home measures 10x Hz with a source connected` then `set back to 120 Hz`; the
   compositor log's `refreshes every` goes 20.00 to 16.67 ms (60 mode) about 1.5 s after the card appeared and stays; `osd_irq` 122 a second
   during and after; the card's picture is smooth.
2. If `wrote 1 60 ... output measures 100 Hz`: the firmware ignores the request even with a source; the manual reading while a card plays
   (`echo '1 60'` and `'0 60'` mid-playback) says which request, if any, works.
3. A second pop-up from 120 Hz: it goes to 100 Hz at the first frame and back within 2 s; the full view over Home likewise.
