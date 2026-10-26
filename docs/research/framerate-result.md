# Frame-rate matching on the C4: how LG picks the output rate, what switches blank, and what LumaTV does

Set: OLED65C4PUA, webOS 10.2.1, chip O22A3, kernel 5.4.268, 144 Hz panel option, internal FRC. Read from LG's GPL kernel tree
(`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, `SRC:` paths relative to it), the set's recorded
boot line and settings (`TV:`), the X4 bus capture (`docs/research/arch/x4`), and the notes named. No TV time was used; nothing
on the set changed.

Evidence tags: `TV:` read on this set, `SRC:` LG's kernel tree, `PUB:` public, `[INFERENCE]` not proven, the probe below names it.

## Resume here

Done (2026-10-26, offline): every question answered from source as far as source goes ("Verdicts" and the sections after);
the probe `docs/research/framerate/probe.py` written with its parser tests (`tests/test_framerate_probe.py`); nothing run on the
set. Open for the TV round: the probe's four runs ("TV probe steps"), which settle every `[INFERENCE]` below, chiefly whether the
one real switch (120 to 100 Hz, the 25/50 family) blanks and for how long, and whether the output stays at 100 Hz on Home after
a 50 fps video.

Scope (Nitin, 2026-10-25 23:48): the TV's own decoder apps come first (Netflix, Plex, YouTube, Crunchyroll, Prime, the GFN app);
HDMI is secondary.

## Verdicts

The display engine's output rate for decoded video is a function of the source rate alone, picked in the kernel (`_vsc_check_fr`)
and doubled for this 120 Hz-class panel (`DE_IO_SetFrRate`): 24, 30, 60 and 120 fps families give 120 Hz, the 25, 50 and 100 fps
family gives 100 Hz, and the 48 Hz case is compiled out on this set (below). The output is phase-locked to the decoder's frame
clock (DCO tracking), so 23.976 fps is shown at 119.88 Hz, exactly five panel refreshes per frame. Every transition in the table
is read from source; the "blank" column is what the probe measures.

| Transition (output rate) | Cadence on the panel | Rate change | Expected blank | Evidence |
|---|---|---|---|---|
| Home (120) -> 23.976 / 24 fps | 5:5 | none (DCO slews 0.1 % for 23.976) | none [INFERENCE: dco_lock during the slew] | SRC `v4l2_vsc_event.c:2694-2707`, `de_io.c:988-998`, `io_video.c:1046-1059` |
| Home -> 29.97 / 30 fps | 4:4 | none | none | same |
| Home -> 59.94 / 60 fps | 2:2 | none | none | same |
| Home -> 4K 120 fps (GFN, HFR) | 1:1 | none; DE HFR mode on | none | SRC `vp_kwrap.c:3700-3712`, `vdisp_core.c:1078-1091` |
| Home -> 25 / 50 fps | 4:4 / 2:2 at 100 Hz | **120 -> 100 Hz** | the one switch to measure; it happens <= 40 ms after the decoder's first frame reaches the DE, while videooutputd still holds its own video mute (about 210 ms at a YouTube start) [INFERENCE: hidden by that mute] | SRC `v4l2_vsc.c:2436-2463` (40 ms poll), `v4l2_vsc_event.c:2411-2418`; TV X4 27874.491 mute on, .704 off |
| 25/50 fps -> Home, then a 24/30/60 title | 100 -> 120 | one switch at the next title's first frame | as above | SRC: no call resets the rate on disconnect (`vp_kwrap.c:3749-3773` turns only HFR off) |
| 30 <-> 60, 24 <-> 60, 25 <-> 50 (adaptive streams, next episode) | | none within a family | none | `_vsc_check_fr` maps both to the same output |
| 24 <-> 25, 60 <-> 50 (next episode, YouTube autoplay) | | 120 <-> 100, mid-play from the viewer's side | same as the start case, but after the previous picture, not under a loading screen | the only unavoidable mid-play switch; LumaTV's own players pre-switch (Design) |
| Dolby Vision / HDR10 / SDR | | none from the dynamic range | none from timing; the picture-mode change is pqcontroller's | SRC `vdisp_core.c:1290`, `deif.c:333-335` (buffer count only) |
| HDMI 24/30/60/120 Hz source | as for decoded | 120 | none at the TV; the source's own resync blanks | SRC `_vsc_check_fr` branch for CVI |
| HDMI 25/50/100 Hz source | | 120 -> 100 | the source's resync covers it | same |
| 1080p 120 fps stream (no DE HFR) | 1:1 at 120 | none | needs the pipeline's HFR flag, else the VDO handshake throttles [INFERENCE] | SRC `vo.c:579-606`, `driver.c:1732-1752`, `deif.c:156-165` |

What this means for Nitin's design direction: the panel already stays at 120 Hz for everything but the 25/50 family, and that
family already gets exactly one switch at the first frame. LumaTV has nothing to build for the common case. It has two things to
add: (1) its own players (Plex plugin, GFN) pre-switch to 100 Hz before their first frame when the item is 25/50 fps, and
(2) the shell puts the output back to 120 Hz when a 25/50 playback ends and nothing else holds a video window, so the UI does
not run at 50 Hz afterwards. Both use the one lever LG left open (`/proc/lg/de/fr_rate`, Steer points), and both wait for the
probe to say the switch is clean and the hold-over is real.

## 1. How LG picks the rate for decoded video

The chain, decoded video to panel (`vdec -> DE -> BE/OVI -> TCON`):

1. **Pipeline to decoder.** `starfish-media-pipeline` opens `/dev/vo` and sets the container and ES frame rates
   (`VO_SET_FRAMERATE_CONTAINER`, `VO_SET_FRAMERATE_ES`, SRC `vo.c:1061-1074`), the flags (`vo_flag_hfr`, `vo_flag_dolby`,
   `vo_flag_svp`, SRC `vo_kapi.h:77-82`), low delay and the direct-video drop threshold (game mode). `videooutputd` tells the
   pipeline `hfrEnabled` and `lowDelayMode` in `setVideoData.videoInfo` (TV X4: YouTube 24 fps `hfrEnabled:false`, Netflix
   4K Dolby Vision 59.94 `hfrEnabled:false`); it does not set any display rate itself.
2. **The decoder's rate (vrate).** `vrate` decides the frame rate from the codec, the container or the PTS intervals
   (`RATE_SRC_CODEC/CONF/PTS`, SRC `vrate.c:456-583`); rates under 10 Hz are multiplied up (`:427-454`); direct video fixes 60
   (`vdisp_core.c:1254-1258`); HFR fixes 120 or 100, whichever divides the content rate (`:1078-1091`).
3. **The decoder's vsync timer.** Each display instance owns a hardware vsync timer programmed to that rate
   (`vsync_set_next_rate`, SRC `vdisp_core.c:663-692`; `set_dev_rate`, `vsync.c:414-450`): 24000/1001 for 23.976, not an
   approximation. Every frame is announced to the DE with its `framerate_res/div` in the DE IPC register block (SRC
   `deif.c:256-262`, `de_ipc.c:286-315`), plus the HFR and direct-media bits.
4. **The DE kernel reads it back.** A kernel thread (`vsc_v4l2_thread`) polls every 40 ms (SRC `v4l2_vsc.c:2427-2463`); for a
   VDEC window it reads the vdec register (`DE_REG_O22_GetMviInfo`, SRC `de_reg_o22.c:3412-3421`: `res*100/div`), converts to
   0.1 Hz (`vp_kwrap.c:1382-1394`) and, when it differs from the last timing (`timing_changed = 3`, `v4l2_vsc_event.c:2230-2235`),
   runs `_vsc_check_fr` for window 0 and issues `DE_IOW_SET_FR_RATE` (`:2411-2418`).
5. **The mapping** (`_vsc_check_fr`, SRC `v4l2_vsc_event.c:2676-2718`): over 90 Hz the rate is halved first; 50 +-0.5 and 25 +-0.5
   give 50; 60 +-0.5 and 30 +-0.5 give 60; everything else gives 60, except 24 +-0.5 with `_g_prev_pulldown` set, which gives 48.
   `DE_IO_SetFrRate` (SRC `de_io.c:972-1097`) doubles that on a `LX_PANEL_TYPE_3840_120P` panel (this set: loader type 0,
   `de_reg_o22.c:1950-1952`): 96, 100 or 120, default 120; sends `DE_IPC_SET_FR_RATE` to the DE firmware; and sets the backlight
   PWM sync to 48/50/60.
6. **The DE firmware** (closed; reference source `io_video.c:1046-1059`): `VIDEO_CTRL_TRACE_DCO_FCW` on (the display clock
   oscillator tracks the source's vsync unless `isForceFreeRun`, which only the `disp=hdmi60` boot options set; this set's boot
   line has no `disp=`, TV `/proc/cmdline`), then `VIDEO_SetSyncOfDisplay(48P|50P|60P)`: the display sync generator is reprogrammed,
   the panel type is not. The firmware's status register reports `frate_in`, `frate_out`, `dco_lock`, `frc` and `low_delay`
   (SRC `de_kapi.h:3688-3712`); the kernel exposes none of it in `/proc` (only the capture driver reads `frate_out`,
   `v4l2_vt.c:2516-2582`), so the probe counts the DE vsync interrupt instead.
7. **OVI/TCON.** Nothing in the OVI driver reacts to a frame-rate change (grep of `core/ovi`, `chip/o22/ovi`: no rate field). The
   panel runs from the DE's output timing over the Vx1 link; `/proc/lg/ovi/status` reports `panelFramerate 3 (144)` and
   `dispFramerate 3` from `hwopt[9]='2'` (TV boot line `hwopt=502210020205102001`; SRC `ovi_hw_o22.c:876-884`). The DE's vsync
   at Home runs 120/s (TV `hardware-census.md`), so the link carries 120 Hz into a 144-capable TCON. [INFERENCE: the TCON follows
   the link rate within its VRR range, 40-144 Hz; a 120 -> 100 change is inside it.]

**24p is 5:5 at 120 Hz, never 48 or 60.** The 48 Hz branch needs `_g_prev_pulldown`, which `v4l2_vsc_update_pulldown` sets only
when boot `hwopt[4] == '0'` ("60hz output", the no-FRC models; SRC `v4l2_vsc_event.c:2130-2145`, `pe_dnt_hw_o18.c:475`). This
set's `hwopt[4]` is `1`, so the pulldown flag is always 0 and 24 fps maps to 60 -> 120 Hz. The 60 Hz output mode exists only
through `V4L2_CID_EXT_VBE_DISPLAYOUTPUT_5060HZ` (SRC `v4l2_vbe.c:856-881`, doc `v4l2-ext-vbe.h:3665-3733`: "called when HDMI
input video timing is 50hz or 60hz with 120hz panel", for input lag), whose OVI half is a no-op on this 144 Hz-option panel
(`ovi_hw_o22.c:4244-4246` acts only on a 120 Hz panel) and whose BE half sets the BE firmware's display mode to 60. It is HDMI
game-mode machinery and never touches the decoder path.

**What Real Cinema and TruMotion change.** `realCinema` and `truMotionMode` (picture category) go through `pqcontroller` to
`V4L2_CID_EXT_MEMC_MOTION_COMP` (SRC `v4l2-ext-pq.h:1033-1177`): off, cinemaClear (3/3), natural (5/5), smooth (7/7), user
(`truMotionBlur`/`truMotionJudder`), and `PULLDOWN_55` ("On 5:5 Pull-down for 24Hz Film source", blur 0, judder 0). The kernel
passes it to the MEMC block and, on no-FRC models only, raises the 48 Hz pulldown flag. On this set Real Cinema therefore decides
the FRC's cadence treatment (5:5 repeat against motion interpolation) and never the panel rate. Nitin's set: `realCinema on`,
`truMotionJudder 0`, `truMotionBlur 10`, `motionPro off`, picture mode `filmMaker` (TV settings census). pqcontroller also sets
`memcDelayLevel` per playback (`mid` at YouTube start, `max` 0.5 s later, TV X4 27874.709, 27875.209) and disables the TruMotion
row for the app (`setSystemSettingDesc truMotionMode active:false`).

**What LG's two HFR switches do.**

- Decoder HFR (`VDISP_MODE_HFR`): on when the pipeline flags it (`lx_vdec_flags_hfr`, `vo_flag_hfr`, `hfr_mode 2PID`) or
  automatically for streams over 1920x1440 whose ES or container rate is 61 fps or more (SRC `driver.c:1732-1752`,
  `vo.c:579-606`). It pins the vsync timer at 120 (or 100) and fixes it (`vsync_set_frame_rate_fixed`), skips the VDO
  read-done handshake (`deif.c:156-165`) and uses two clear buffers (`:333-335`).
- DE HFR (`DE_IPC_SET_HIGH_FRAME_RATE`, "4k 120Hz"): on when window 0's VDO source reports 61.0 Hz or more and is wider than
  1920 or taller than 1440 (SRC `vp_kwrap.c:3700-3712`), off at VDO disconnect (`:3755-3760`); the userspace twin is
  `V4L2_CID_EXT_VSC_OCCUPATION_SUB_SCALER` ("use 2 video path for displaying HFR source over 2K", `v4l2-ext-vsc.h:1938-2011`,
  videooutputd's `subScalerOccupied`). It is the 4K120 path through both scalers; it does not change the panel rate (120 either
  way).

So for the GFN app's 4K120 both are on; for a 1080p120 or 1440p120 stream neither is automatic, the pipeline must flag HFR
(`hfrEnabled`), and without it the vsync runs 120 unfixed with the handshake on [INFERENCE: measured as dropped frames in `vo`
status `frames_dropped`].

## 2. Does a switch blank the panel

What the source says:

- The kernel issues no mute around `DE_IOW_SET_FR_RATE` (the whole path `v4l2_vsc_event.c:2411-2418` -> `de_io.c:972-1097`
  contains no `DE_IOW_SET_WIN_BLANK`). Mutes come from three places: videooutputd's own `setAvMute videoMute` through
  avmutemanager at connect and after `setDisplayWindow` (TV X4: on at 27874.491, off at 27874.704 for YouTube; the audio mute
  0.663 to 0.792), the DE firmware's internal mute (reason codes in a shared register, read by the 40 ms thread and exposed as
  `localmute=N` in `/proc/lg/de/win0/status`, SRC `v4l2_vsc.c:2440-2449`, `de_proc.c:1637`), and `V4L2_CID_EXT_VBE_MUTE`
  (whole screen, panelcontroller's).
- A switch is detected and sent within one 40 ms poll of the first frame that carries the new rate. Pre-roll frames carry the
  rate too (`display_info.preroll_data`, SRC `de_ipc.c:294-296`), so for apps that pre-roll the switch lands during loading.
- The firmware's reaction (how many frames the sync generator takes, whether it mutes while the DCO relocks) is closed. The
  probe reads `localmute`, the DE vsync count and the screen every 50 ms across the switch. [INFERENCE: a sub-100 ms relock
  with no mute, as on LG's own 25/50 content every day; a mute would show as `localmute != 0` for one or more samples.]
- Seamless by construction: 24 <-> 30 <-> 60 <-> 120 (all 120 Hz: the FRC changes its repeat count; `DE_IOW_SET_FR_RATE` is
  still sent at every timing change, with the same value, and the firmware's reference path then re-sets the same sync type,
  `io_video.c:1583` [INFERENCE: a no-op, read as "no `localmute`, no vsync gap" in the 24 -> 60 run]); 23.976 <-> 24 and
  59.94 <-> 60 (a 0.1 % DCO slew; the decoder side changes the timer at once, `vsync.c:562-578`, the DCO follows).
- Not seamless by construction: 120 <-> 100, which is the real sync-generator change. On the decoder side the timer is simply
  reprogrammed; on the display side the DCO must re-lock to a new multiple.
- The compositor plane: the OSD's vblank is the DE output divided by the module parameter `fbdev_vsync_cnt_on_flip` (2: "value 2
  makes 60Hz handler (120Hz/2=60Hz)", SRC `fbdev_drv.c:180-181`, `:543-546`, `:2939-2947`), so the UI runs 60 Hz at 120 and 50 Hz
  at 100 (that is the 50 Hz "behind a browser page" of `perf-tweaks.md`), and 24p film gives the UI 60 Hz, not 24.

## 3. Steer points for a root userspace that does not touch `/usr`

| Lever | What it does | Safe? | Use |
|---|---|---|---|
| `echo <freerun> <50|60> > /proc/lg/de/fr_rate` | The same `DE_IOW_SET_FR_RATE` ioctl the kernel thread issues (SRC `de_proc.c:1231-1240`, usage line `:575`): 50 -> 100 Hz, 60 -> 120 Hz on this panel. Overwritten by the kernel at the next timing change of window 0. | Low: a path LG's own kernel takes at every playback start; no panel-type change; no `/usr` write. [INFERENCE: no blank, same as LG's switch] | Pre-switch to 100 Hz before a 25/50 item's first frame; back to 120 Hz when playback ends and no video window remains |
| `luna://com.webos.service.videooutput` `setVideoData.frameRate` / `hfrEnabled` | Informs pqcontroller (`setPQ.frameRate`) and the pipeline (HFR flag); no display-rate effect | Safe, but not a rate lever | Only through the media pipeline's own load options (`hfrEnabled`) for 1080p120 |
| settingsservice `picture.realCinema`, `truMotionMode`, `truMotionJudder/Blur` | MEMC cadence (5:5 against interpolation); no rate effect on this set | Safe (user-visible setting) | Keep `realCinema on`, judder 0 for film cadence; LumaTV's Settings already own these keys |
| `V4L2_CID_EXT_VBE_DISPLAYOUTPUT_5060HZ` on the vbe node | Forces 60 Hz output (HDMI game lag mode); OVI half a no-op here, BE half changes the BE firmware's display mode | Medium risk (BE mode change under a live picture), and the wrong direction | Never |
| `BE_IOW_SET_DISP_MODE` VRR / `DE_IOW_SET_HIGH_FRAME_RATE` ioctls | Panel VRR mode / DE 4K120 path | High / pointless outside the kernel's own conditions | Never (see `gfn-444-vrr-result.md` E4) |
| `/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip` | OSD vsync divider (2 -> 60 Hz UI at 120 output; 1 would give 120 Hz UI flips) | Writable by root at run time; untested; halves the UI frame budget | Not now; a later experiment if a 120 Hz UI is ever wanted |
| `/proc/lxvo/vdisp/debug` (`echo 4 id res div`: a fixed decoder rate) | Debug override of the decoder's vsync rate | Medium: breaks lip-sync, meant for LG's bench | Never in a product path; the probe reads `vdisp`, `vrate`, `vsync` only |
| DRM modes from the compositor | Cosmetic; the CRTC never touches panel timing (`gfn-444-vrr-result.md` (a)) | Harmless, useless | The compositor commits with `ALLOW_MODESET` once and never again (already so) |

Hold 120 Hz as base: nothing to do while LG's apps play 24/30/60 content; the DE comes up at 60 -> 120 at resume
(`resource_mgr.c:543-544`). The hold-over after 25/50 content is the gap, and the `fr_rate` write closes it.

## 4. LumaTV's compositor (C3)

What the compositor must and must not do, against what it does:

- Never force a mode change: it commits `ALLOW_MODESET` only before `modeset_done` (`kms/mod.rs:525-534`) and the `lg1k` CRTC
  ignores the mode anyway. Nothing else to add. It must never write `/proc/lg/de/fr_rate` itself: the shell (helper) does that
  on playback end, so the rule lives in one place.
- Its UI clock follows the panel: `kms/timing.rs` keys the pacer to the period the flips measure, re-keyed after four agreeing
  flips, with animations on wall time. That is the right design; two details to put straight: the doc comment names 24 Hz as a
  case ("24 Hz film is the other case"), but on this set 24p film gives a 60 Hz vblank (120 / 2), and the only UI periods that
  occur are 16.67 ms (120 and 119.88 Hz output: 60 / 59.94 Hz vblank) and 20 ms (100 Hz output). The `RATES_HZ` snap list covers
  both; the 4-flip agreement means a 120 -> 100 switch is recognised about 80 ms after it happens, during which up to four frames
  count as `odd` stamps. Fine for the gate; the comment is corrected with the first compositor change after the probe.
- It cannot disturb the video cadence: the OSD plane is blended at the DE's output rate from its own halved vsync; the decoder's
  frame clock is a hardware timer and the DCO tracks it in firmware. Only CPU starvation of the vdec's frame announcement could
  (every interrupt sits on CPU0, `perf-tweaks.md`); the compositor is not real-time and the gate's x1 run shows no video
  stalls under it.
- Video windows: placement goes through videooutputd as today (the compositor reports the rectangle; PLAN "Rendering stack").
  A LumaTV player that pre-switches must do it before `connect`, i.e. before the pipeline's first frame, during its own loading
  screen.

## 5. HDMI sources (secondary)

The TV: an HDMI timing goes through the same `_vsc_check_fr` (24/30/60/120 -> 120 Hz, 25/50/100 -> 100 Hz; a 144 Hz input
falls to the default 120 unless the VRR/game path is in charge) and the same `DE_IOW_SET_FR_RATE`. Nothing on the TV can soften
a source's mode change: the link retrains and HDCP re-authenticates, the receiver reports signal loss, the DE mutes
(`localmute` reasons; `v4l2_vsc_event.c:3247-3283` names the HDMI Dolby Vision metadata and invalid-signal cases) until the
new timing is stable. The source must either output the native rate per title (Apple TV "Match Frame Rate", with its own
1-3 s black at each change) or stay at one rate and accept 3:2 cadence inside the source. With the Apple TV retired or
reduced to a hub this is moot; a future HDMI source should match rate, and the TV's part is already right.

## Design

1. **Measure first** (the probe, next TV round). Four runs settle the `[INFERENCE]` marks: a 23.976 title (Netflix) for the
   no-switch case and the DCO slew; a 50 fps YouTube clip for the 120 -> 100 switch at start and the hold-over on Home after
   it; a 25 fps clip after a 24 fps one in Plex for the mid-play switch; the 4K120 GFN stream for HFR. Each run logs every 50 ms
   (`probe.py --sample` on the set): the DE and OSD vsync counts, `win0/status` (`vFreq`, `localmute`, `hdr_type`,
   `lowdelaymode`), `scaler/main.p0` (`mute`, `adaptive`, `connect_src`), `vdisp/{vsync,vrate,vdisp}` (the decoder's timer
   rate, fixed flag, rate source, film mode, frame count), the first `vo` instance's flags and frame counters; beside it a
   videooutput `getStatus` subscription, the bus monitor filtered to videooutput/avmutemanager/pqcontroller, screenshots in a
   loop (320x180 PNG, `writtenBytes` and luma: black and frozen frames), and the kernel log lines of the switch (`set_frate`,
   `timing_changed`, `4k HFR mode`, `mute-on/off`, `FrameDuration90k`, `[VSYNC`).
2. **If the switch is clean** (no `localmute`, no vsync gap over one refresh, no black screenshot outside videooutputd's own
   mute): LG's behaviour is the design for LG's apps; LumaTV adds only the hold-over fix (helper: on the foreground app leaving
   playback with no video window connected, `echo 0 60 > /proc/lg/de/fr_rate` if the measured rate is 100) and the pre-switch
   in its own players (Plex plugin: the item's `videoFrameRate` 25/50 -> `echo 0 50` before `connect`; GFN: 120, nothing).
3. **If the switch blanks** (`localmute` set or screenshots black for more than one sample after videooutputd unmutes): the
   pre-switch moves earlier for every app LumaTV launches with a known rate (Plex), and for the rest the switch is still hidden
   under the app's loading screen when the app pre-rolls; the mid-play case (next episode across the 100/120 boundary) stays
   LG's and is documented as such. No path exists to make the firmware's relock itself faster.
4. **Settings.** LumaTV's Picture page keeps Real Cinema on and TruMotion off/judder 0 as the film default (already Nitin's
   values); no new user setting for the rate: there is nothing the user should pick.

## Not found

The DE firmware's relock time and whether it mutes; whether the output stays at 100 Hz on Home after a 50 fps title; whether a
rate re-set with the same value (24 -> 60 content) is a no-op in the firmware; the DCO lock bit during a 23.976 <-> 24 slew; what
LG's pipeline does for 1080p120 without `hfrEnabled`. All four runs of the probe.

## TV probe steps

One slot of about 12 minutes, from the repository root, each run on its own: `probe.py --go` mutes the receiver through Home
Assistant before anything plays and puts it back only after Home is in front again, as the other slots do. `--go` is read-only on
the set apart from the app launch, the key presses (kb.py) and the files under `/tmp/fr-probe-<stamp>/`; the `fr_rate` write is a
separate experiment and the script refuses `--allow-fr-rate-write`. Each run ends with the app closed, Home in front and seen,
the receiver put back, the set in Active Standby (`--no-standby` leaves it on); its files land in
`docs/research/framerate/<run>-<stamp>/` with `summary.txt` and `verdict.json` written, and the screenshots pruned to every
20th plus those within 1.5 s of a switch, a mute or a mark.

1. `python3 docs/research/framerate/probe.py --plan --run youtube50` (nothing touched; prints the steps).
2. `python3 docs/research/framerate/probe.py --go --run youtube50`: a 50 fps YouTube clip (the default launch is a search for
   "4k 50fps test" and OK on the first result; `--launch '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/watch?v=..."}}'`
   names a known clip), 45 s, then Home for 10 s with the sampler running: the 120 -> 100 Hz switch and the hold-over.
3. `python3 docs/research/framerate/probe.py --go --run netflix24`: Netflix, a 23.976 title (the id in `RUNS` is a guess; `vFreq`
   in the summary says what played; `netflix60` is Meridian at 59.94): the no-switch case and the DCO slew.
4. `python3 docs/research/framerate/probe.py --go --run plex25after24 [--keys ... --second-keys ...]`: Plex, a 24 fps item then
   a 25 fps item without Home in between; the key paths depend on the library's order (`OK s4 OK s5 OK` then
   `BACK s2 DOWN s1 OK s4 OK` by default): the mid-play switch.
5. `python3 docs/research/framerate/probe.py --go --run gfn120`: the GFN app at 4K120, a session started by hand once the app is up
   (skip when no session is possible): both HFR switches on, 120 Hz 1:1.
6. Read each `summary.txt` (`--report DIR` recomputes it): the DE rate at start and end, the switch moments against the app's
   own video mute, the firmware mute spans, the vsync gaps, the black and frozen screen spans, `VERDICT`. Put the measured rows
   into "Verdicts" and settle the `[INFERENCE]` marks; the `kernel.txt` lines (`set_frate`, `timing_changed`, `4k HFR mode`,
   `mute-on/off`) date the switch to the 40 ms poll.
