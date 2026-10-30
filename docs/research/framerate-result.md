# Frame-rate matching on the C4: how LG picks the output rate, what switches blank, and what LumaTV does

Set: OLED65C4PUA, webOS 10.2.1, chip O22A3, kernel 5.4.268, 144 Hz panel option, internal FRC. Read from LG's GPL kernel tree
(`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, `SRC:` paths relative to it), the set's recorded
boot line and settings (`TV:`), the X4 bus capture (`docs/research/arch/x4`), round 11's probe runs (`framerate/*-20261026-*`,
run 2026-10-26 04:09-04:25) and the notes named.

Evidence tags: `TV:` read on this set, `SRC:` LG's kernel tree, `PUB:` public, `[INFERENCE]` not proven, the probe below names it.

## Resume here

Done (2026-10-26 morning, offline, no TV): round 11's 50 fps run read sample by sample ("Round 11"): the return to 120 Hz was
LG's switch at the start of another video (YouTube's Browse-page masthead ad), inside the firmware's start mute that every start
has, 0.33 s before that video's window was shown. Built on it: the helper puts the panel back to 120 Hz on Home after a 25/50 fps
video (`helper/app/src/framerate.rs`, `898e638`); the probe places each switch against the video window instead of counting the
start mute as a blank, sees 0.2 s stays, keeps the helper's log lines, and has the `youtube50home` run (`10aa929`). Open: the
TV check ("TV check"), with Nitin at the set for the one thing no capture shows (whether a switch blinks the panel itself); the
GFN 4K120 run; a 25 fps Plex item (Plex's library API answers 401 without a token, and none has been handed).

Scope (Nitin, 2026-10-25 23:48): the TV's own decoder apps come first (Netflix, Plex, YouTube, Crunchyroll, Prime, the GFN app);
HDMI is secondary.

## Verdicts

The display engine's output rate for decoded video is a function of the source rate alone, picked in the kernel (`_vsc_check_fr`)
and doubled for this 120 Hz-class panel (`DE_IO_SetFrRate`): 24, 30, 60 and 120 fps families give 120 Hz, the 25, 50 and 100 fps
family gives 100 Hz, and the 48 Hz case is compiled out on this set (below). The output is phase-locked to the decoder's frame
clock (DCO tracking), so 23.976 fps is shown at 119.88 Hz, exactly five panel refreshes per frame. Rows with `TV` evidence were
measured in round 11 (2026-10-26); the rest are read from source.

| Transition (output rate) | Cadence on the panel | Rate change | Expected blank | Evidence |
|---|---|---|---|---|
| Home (120) -> 23.976 / 24 fps | 5:5 | none (DCO slews 0.1 % for 23.976) | none seen: 120 Hz band throughout, no vsync gap | TV `netflix24-20261026-0409`, `plex25after24-20261026-0414`; SRC `v4l2_vsc_event.c:2694-2707`, `de_io.c:988-998`, `io_video.c:1046-1059` |
| Home -> 29.97 / 30 fps | 4:4 | none | none seen | TV `youtube50-20261026-0411` (the app's own 29.97 preview); SRC as above |
| Home -> 59.94 / 60 fps | 2:2 | none | none | same |
| Home -> 4K 120 fps (GFN, HFR) | 1:1 | none; DE HFR mode on | none | SRC `vp_kwrap.c:3700-3712`, `vdisp_core.c:1078-1091` |
| Home -> 25 / 50 fps | 4:4 / 2:2 at 100 Hz | **120 -> 100 Hz** at the first frame | none seen: the switch lands before the window is shown (round 11: at 6.90 s, window shown at 6.97 s, firmware start mute 0.15 s after videooutputd's unmute) | TV `youtube50-20261026-0424`; SRC `v4l2_vsc.c:2436-2463` (40 ms poll), `v4l2_vsc_event.c:2411-2418` |
| 25/50 fps window closes | stays at 100 Hz, UI at 50 Hz | none | the hold-over: 2.6 s in round 11, until the next video's first frame; on Home the helper puts 120 Hz back | TV round 11 (16.80-19.66 s); SRC `v4l2_vsc_event.c:2220-2221` (`default: return` with no source) |
| 25/50 fps, then a 24/30/60 video in the same app | | **100 -> 120 Hz** at the new video's first frame | none seen: inside the firmware's start mute (round 11: switch at 19.69 s, window shown at 20.02 s) | TV `youtube50-20261026-0424` |
| 30 <-> 60, 24 <-> 60, 25 <-> 50 (adaptive streams, next episode) | | none within a family | none | `_vsc_check_fr` maps both to the same output |
| Any start after the output was moved by hand (`fr_rate`) | | the previous stream's rate at the connect, the new one at the first frame (about 0.2 s apart) | under videooutputd's mute and the start mute [INFERENCE] | TV: the DE reads the decoder register's last value at a connect (0411: 23.97 then 29.97; 0412: 29.97 then 23.97; 0424: 50.0 then 23.97) |
| Dolby Vision / HDR10 / SDR | | none from the dynamic range | none from timing; the picture-mode change is pqcontroller's | SRC `vdisp_core.c:1290`, `deif.c:333-335` (buffer count only) |
| HDMI 24/30/60/120 Hz source | as for decoded | 120 | none at the TV; the source's own resync blanks | SRC `_vsc_check_fr` branch for CVI |
| HDMI 25/50/100 Hz source | | 120 -> 100 | the source's resync covers it | same |
| 1080p 120 fps stream (no DE HFR) | 1:1 at 120 | none | needs the pipeline's HFR flag, else the VDO handshake throttles [INFERENCE] | SRC `vo.c:579-606`, `driver.c:1732-1752`, `deif.c:156-165` |

What this means for Nitin's design direction: the panel already stays at 120 Hz for everything but the 25/50 family, and LG's
switches into and out of that family land at a video's start, before its window is shown (round 11). LumaTV has one thing to add:
the helper puts the output back to 120 Hz once Home is in front with no video window, so LumaTV's own UI never runs at 50 Hz
after a 25/50 video ("Design"). A player that writes `fr_rate` before its first frame gains nothing: the kernel applies the decoder
register's previous rate at the connect and the item's own at its first frame, whatever was written before.

## Round 11: when the return to 120 Hz happened

From `framerate/youtube50-20261026-0424` (samples every 50 ms, the bus monitor, videooutput's subscription, screenshots). Seconds
from the run's first sample (uptime 14815.52).

| s | What |
|---|---|
| 2.90 | YouTube launched with `contentTarget` `v=xX0wOkuLKZM` (Camera Jabber, "Sony A1 4K Full frame vs Super 35": 151 s, 3840x2160, 50 fps) |
| 6.71 | The clip's window connects; the DE reads 59.9 then 60.0 Hz (the decoder's first rate), 50.0 Hz from 6.92 |
| 6.82 / 6.90 / 6.97 | videooutputd unmutes; output 120 -> 100 Hz (OSD vsync 6 -> 5 per 50 ms); the firmware's mute (34) clears and the clip shows |
| 6.97-16.67 | 50 fps at 100 Hz: no vsync gap, no frame dropped |
| 16.44 | YouTube's own Browse page (its loading skeleton) is on screen while the clip still decodes; no key was pressed |
| 16.67-16.80 | YouTube ends its player: ADEC and VDEC disconnected, the pipeline destroyed, videooutputd's mute on, the scaler `NONE`. The output stays at 100 Hz |
| 16.80-19.36 | No video; YouTube's Browse page drawn at 100 Hz (UI at 50 Hz) |
| 19.37 | A new pipeline connects: the Browse page's masthead, a Walmart ad, 1920x1080 at 23.976 fps in a 3380x1902 window |
| 19.41 | The DE's connect-time read is 50.0 Hz (`vFreq` 500): the decoder register still holds the clip's rate, and the kernel sets 50 from it (SRC `v4l2_vsc_event.c:2401-2418`: the timing `win0/status` shows is the one the rate is set from), no change |
| 19.51 | videooutputd unmutes the window; the firmware's start mute follows (267, then 34) |
| 19.61 / 19.69 | The DE reads 23.97 Hz; `_vsc_check_fr` gives 60; output 100 -> 120 Hz |
| 20.02 | The firmware's mute clears: the ad's first visible frame, at 120 Hz |

The return was the kernel's `_vsc_check_fr` on the first frame of a different video, 2.6 s after YouTube itself abandoned the clip
for its Browse page, 10 s into a 151 s clip: the app's decision opened the gap, the next video's rate closed it. Not the clip's
end, an ad inside the clip, a seek, or a rate change inside one stream; the video window was not on screen during the switch.

The run's "firmware mute yes" was the firmware's start mute, which every start has, switch or not: after videooutputd's unmute
the window was shown 0.41, 0.41, 0.41, 0.31, 0.41 and 0.52 s later at that night's six starts without a switch (netflix24,
plex25after24 twice, youtube50 0411, youtube50 0412 twice), 0.51 s at this one and 0.15 s at the clip's own start. The probe now
fails a switch only when the window had been on screen before it (`probe.py` `switch_context`); `--report` reads this run as
two switches at starts, `VERDICT: PASS` (its committed `summary.txt` is the original reading).

What no capture shows: whether a 100 <-> 120 Hz change blinks the panel itself, the UI around a muted window included. The capture
service reads the planes, not the panel, and the DE's vsync ran on without a gap through both switches. That needs an eye on the
set ("TV check").

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
- The firmware's reaction is closed in source; round 11 measured it (TV `youtube50-20261026-0424`): both switches took effect
  within one 50 ms sample (OSD vsync 6 -> 5 and 5 -> 6 interrupts per 50 ms) with no vsync gap, and the only `localmute` around
  them was the start mute every start has (codes 34 and 267; "Round 11"). Whether the panel itself blinks is not visible to any
  capture.
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
| `echo <freerun> <50|60> > /proc/lg/de/fr_rate` | The same `DE_IOW_SET_FR_RATE` ioctl the kernel thread issues (SRC `de_proc.c:1231-1240`, usage line `:575`): 50 -> 100 Hz, 60 -> 120 Hz on this panel. Overwritten by the kernel at the next timing change of window 0. | Low: a path LG's own kernel takes at every playback start; no panel-type change; no `/usr` write. [INFERENCE: no blank, same as LG's switch] | Back to 120 Hz on Home once no source is connected (the helper, "Design"). Not before a first frame: the connect applies the decoder register's previous rate whatever was written |
| `luna://com.webos.service.videooutput` `setVideoData.frameRate` / `hfrEnabled` | Informs pqcontroller (`setPQ.frameRate`) and the pipeline (HFR flag); no display-rate effect | Safe, but not a rate lever | Only through the media pipeline's own load options (`hfrEnabled`) for 1080p120 |
| settingsservice `picture.realCinema`, `truMotionMode`, `truMotionJudder/Blur` | MEMC cadence (5:5 against interpolation); no rate effect on this set | Safe (user-visible setting) | Keep `realCinema on`, judder 0 for film cadence; LumaTV's Settings already own these keys |
| `V4L2_CID_EXT_VBE_DISPLAYOUTPUT_5060HZ` on the vbe node | Forces 60 Hz output (HDMI game lag mode); OVI half a no-op here, BE half changes the BE firmware's display mode | Medium risk (BE mode change under a live picture), and the wrong direction | Never |
| `BE_IOW_SET_DISP_MODE` VRR / `DE_IOW_SET_HIGH_FRAME_RATE` ioctls | Panel VRR mode / DE 4K120 path | High / pointless outside the kernel's own conditions | Never (see `gfn-444-vrr-result.md` E4) |
| `/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip` | OSD vsync divider (2 -> 60 Hz UI at 120 output; 1 would give 120 Hz UI flips) | Writable by root at run time; untested; halves the UI frame budget | Not now; a later experiment if a 120 Hz UI is ever wanted |
| `/proc/lxvo/vdisp/debug` (`echo 4 id res div`: a fixed decoder rate) | Debug override of the decoder's vsync rate | Medium: breaks lip-sync, meant for LG's bench | Never in a product path; the probe reads `vdisp`, `vrate`, `vsync` only |
| DRM modes from the compositor | Cosmetic; the CRTC never touches panel timing (`gfn-444-vrr-result.md` (a)) | Harmless, useless | The compositor commits with `ALLOW_MODESET` once and never again (already so) |

Hold 120 Hz as base: nothing to do while LG's apps play 24/30/60 content; the DE comes up at 60 -> 120 at resume
(`resource_mgr.c:543-544`). The hold-over after 25/50 content is the gap; the helper's write closes it on Home.

## 4. LumaTV's compositor (C3)

What the compositor must and must not do, against what it does:

- Never force a mode change: it commits `ALLOW_MODESET` only before `modeset_done` (`kms/mod.rs:525-534`) and the `lg1k` CRTC
  ignores the mode anyway. Nothing else to add. It must never write `/proc/lg/de/fr_rate` itself: the helper does that on Home,
  so the rule lives in one place.
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
  No player pre-switches the rate ("Verdicts": a connect applies the decoder register's previous rate).

## 5. HDMI sources (secondary)

The TV: an HDMI timing goes through the same `_vsc_check_fr` (24/30/60/120 -> 120 Hz, 25/50/100 -> 100 Hz; a 144 Hz input
falls to the default 120 unless the VRR/game path is in charge) and the same `DE_IOW_SET_FR_RATE`. Nothing on the TV can soften
a source's mode change: the link retrains and HDCP re-authenticates, the receiver reports signal loss, the DE mutes
(`localmute` reasons; `v4l2_vsc_event.c:3247-3283` names the HDMI Dolby Vision metadata and invalid-signal cases) until the
new timing is stable. The source must either output the native rate per title (Apple TV "Match Frame Rate", with its own
1-3 s black at each change) or stay at one rate and accept 3:2 cadence inside the source. With the Apple TV retired or
reduced to a hub this is moot; a future HDMI source should match rate, and the TV's part is already right.

## Design (2026-10-26, after round 11)

1. **Inside an app: LG's.** The next video's start carries the kernel's own switch, before its window is shown (round 11: 0.33 s
   before). Holding 100 Hz through it is impossible (the kernel sets the rate at every timing change of window 0) and wrong
   (23.976 fps at 100 Hz shows frames for four and five refreshes unevenly). LumaTV writes nothing while an app is in front.
2. **On Home: the helper** (`helper/app/src/framerate.rs`, `898e638`). SAM's `getForegroundAppInfo` subscription says when Home
   comes to the front; the screen coming on with Home in front counts too. One check then runs: the output is counted (`osd_irq`
   over 0.5 s); at 100 Hz the helper waits until the main window has no source (`connect_src=NONE` in
   `/proc/lgtv-driver/scaler/main.p0`, every 0.25 s for 15 s, then every 2 s), counts again with the window closed, looks at the
   window once more, writes `1 60` (free run; `0 60` left the output at 99.9 Hz on the set, `vwin-refresh-result.md`) to `/proc/lg/de/fr_rate` and counts a third time. The log line is
   `frame rate: Home was left at 100.0 Hz with no video window open; set back to 120 Hz (119.6 Hz measured)`. Never a write while
   a source is connected; nothing at all while the subscription is down. `helper.json` `"frameRate": {"returnOnHome": false}`
   turns it off.
   Camera video in the main window plays as a direct video (`cameras.directVideo`), whose rate the kernel fixes at 60, so a 25 fps camera
   leaves the panel at 120 Hz (`vwin-refresh-result.md`).
3. **The price.** The next start after a return applies the 25/50 video's rate at its connect (the decoder register) and its own
   at the first frame: a 120 -> 100 -> 120 pair about 0.2 s long, under videooutputd's mute and the start mute [INFERENCE until
   the TV check].
4. **Settings.** LumaTV's Picture page keeps Real Cinema on and TruMotion off/judder 0 as the film default (already Nitin's
   values); no new user setting for the rate: there is nothing the user should pick.

## Not found

Whether a 100 <-> 120 Hz change blinks the panel itself (the planes are captured, not the panel; the DE's vsync never stopped);
the DE firmware's relock time; whether a re-set with the same value (24 -> 60 content) is a no-op in the firmware; the DCO lock
bit during a 23.976 <-> 24 slew; LG's pipeline for 1080p120 without `hfrEnabled`; a 25 fps Plex item.

## TV check (next slot, Nitin at the set)

The helper of `898e638` or later on the set (the next package). From the repository root; each run mutes the receiver through
Home Assistant, puts it back once Home is in front, and saves the helper's `frame rate:` lines (`helper.txt`); `--no-standby`
keeps the set on between runs. About 6 minutes.

1. `python3 docs/research/framerate/probe.py --go --run youtube50home --no-standby`: the known 50 fps clip, Home 8 s after the
   launch, 15 s of Home. Nitin watches the whole screen at the clip's start and for 2 s after Home appears.
   Pass: `switches:` has `120 -> 100 Hz at … (start)`, then `100 -> 120 Hz at … (no window)` within 1.5 s of the clip's window
   closing (the `0 at` source rate after the `home` mark); `helper:` has `Home was left at 10x.x Hz … set back to 120 Hz`;
   `VERDICT: PASS`; OSD vsync 119-120 Hz at the end; Nitin saw no blink at either moment. If the clip's window closed before the
   `home` mark (YouTube left it 11 s after the launch at the earliest in round 11), run it again.
2. At once, from Home: `python3 docs/research/framerate/probe.py --go --run netflix24 --no-standby`: the first start after the
   return. Pass: `VERDICT: PASS`; `switches:` none, or `120 -> 100` and `100 -> 120` about 0.2 s apart, both `(start)`; Nitin saw
   no blink on Netflix's loading screen. The pair, or its absence, goes into "Verdicts" (the `fr_rate` row).
3. Round 11's in-app return, watched: `python3 docs/research/framerate/probe.py --go --run youtube50home --secs 25 --no-standby`
   (Home 30 s after the launch). When YouTube leaves the clip for its Browse page and starts a preview there, `switches:` has
   `100 -> 120 Hz … (start)`; Nitin watches YouTube's Browse page at that moment. Pass: `VERDICT: PASS`, no blink seen.
4. The set back in Active Standby (the last run without `--no-standby`, or the power key).

A blink at step 1 alone is Nitin's choice: Home at 120 Hz after 25/50 video, or no write (`"frameRate": {"returnOnHome": false}`
in `/var/lib/lumatv/helper.json`, then `lumatv helper stop` and `lumatv helper start`). A blink at step 2 or 3 is LG's own
switch and stays either way.

Still open after that: `gfn120` (a GFN session started by hand: both HFR switches on, 120 Hz 1:1), and a 25 fps Plex item:
`plex25after24` picked two 23.976 items in round 11, and the Plex library API (`http://192.168.1.69:32400`) answers 401 without a
token. With a token from Nitin, `/library/sections/<id>/all?type=4` lists each episode's `Media videoFrameRate` (`PAL` is 25 fps)
and the run gets that item's key path; without one, a title he knows to be 25 fps (a UK or Australian series).

## Home refresh rate read, 2026-10-27 14:55 UTC (read only)

The helper's log has four lines `frame rate: the output measures 95.9 Hz on Home, neither 100 nor 120; left as it is` (2026-10-27 03:42:07, 03:43:58, 03:44:52 and 03:48:20 UTC), all after the helper restarted at 03:28:28; they fall inside round 17's frame gate (about 03:38-03:45, "Round 17" in `shell-host-result.md`) and the input-banner check (03:45-03:49). 95.9 Hz is 48 `osd_irq` interrupts in the helper's 0.5 s window each time. No other `frame rate` line is in the log.

The same read now, with Home in front (Netflix closed a minute before): `osd_irq` in `/proc/interrupts` over 2.0 s, three times, 241, 241 and 240 interrupts = 119.9, 119.9 and 120.0 Hz. `/proc/lgtv-driver/scaler/main.p0` says `connect_src=NONE`; `/proc/lg/ovi/status` `panelFramerate 3 (144)`, `dispFramerate 3`; `picture.pictureMode` is `filmMaker`. `/proc/lg/de/fr_rate` is write-only (it prints its usage text); nothing was written.

So Home is at 120 Hz now and nothing left it off 120 in this read: an app (Netflix browse), the picture mode and the helper's own check do not. Not read: the VRR / Game Optimizer settings, and the rate during a gate run (the gate and the TV check were stopped at the owner's request before they started). Round 17's windows with 144-157 vblanks against 180-198 match a rate about 20 % lower than 120 Hz [INFERENCE: the same cause as the 95.9 Hz reads; not isolated]. To isolate: sample `osd_irq` every 0.2 s on the set through a gate run and line the counts up with the surfaces.
