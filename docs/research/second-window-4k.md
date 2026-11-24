# Camera pop-up over 4K HDR playback: does the second video fit? (offline, 2026-11-22)

Evidence tags as in `multiview-result.md`: `TV:` read on the set (file named), `SRC:` LG's kernel tree (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/`) or webOS Open Source Edition's `umediaserver` (https://github.com/webosose/umediaserver, `src/resource_manager/{ResourceManager,AcquireQueue}.cpp`, read 2026-11-22; same lineage as the set's binary), `[INFERENCE]` not measured. No TV access, nothing installed.

## Verdict: unknown as a blanket answer; the rule decides per app

- **Netflix 4K (Dolby Vision HEVC): still.** Its load takes all four `VDEC` and all three `DM`. A second decoder cannot fit, so the media server preempts the main stream: black frame, audio cut.
- **YouTube 4K60 SDR: predicted live.** Its load always asks for `VDEC` 2 of 4, at 1080p and at 4K alike. A 1080p H.264 sub stream fits beside it. Proven at 1080p (M3); the 4K case has no sub-window capture.
- **Any other 4K HDR HEVC app (Plex, others): unknown** until its own acquire is read (not on disk). The advance check below answers it per app.
- **A 4K stream over 60 fps (the 120 Hz HEVC the codec file allows) never has a sub window**: it holds both scalers.

The texture path (CPU decode, no decoder or scaler touched) never disturbs the main stream, so a pop-up always has a safe fallback; only the hardware-window path needs the rule.

## What each stream costs

Totals (`TV: multiview/tv-2026-10-30/config/_etc_umediaserver_umediaserver_resource_config.txt`): `VDEC` 4, `VDEC_BUS` 6, `G1_H264` 8, `HEVCP1_HEVC` 4, `HEVCP1_VP9` 4, `AV1_HANDLE` 2, `DM` 3, `SVP_CPB` 2, `MAIN_SCALER` 1, `SUB_SCALER` 1, `MAIN_SOUND` 1, `ADEC` 2. The display engine has two video windows (`NUM_WIN_MAX 2`, `hw-planes-result.md:50`): `MAIN` and `SUB0`.

| Stream | Units acquired | Evidence |
|---|---|---|
| Netflix 4K HEVC (3840x2160 at 59.94, Dolby Vision picture mode) | `VDEC` 4, `DM` 3, `SVP_CPB` 1, `DVR_HANDLE` 1, `ADEC` 1, `ADEC_BANDWIDTH` 3; connector `MAIN_SCALER` 1, `MAIN_SOUND` 1. `subScalerOccupied false` | `round9/r10-2026-10-26/nf10-messages.log` (`umediaserver ACQUIRE_REQUEST`, six identical sets over three captures) |
| YouTube (Cobalt), 1080p24 and 4K60 | `VDEC` 2, `VDEC_BUS` 4, `HEVCP1_HEVC` 2, `AV1_HANDLE` 2, `ADEC` 1, `ADEC_BANDWIDTH` 2; connector `MAIN_SCALER`, `MAIN_SOUND`. The same set at both sizes: the app declares an envelope, not the stream | `multiview/tv-2026-10-30/luna-yt/` (3840x2160 at 60, `hdrType none`), `m3-front/ums-pipelines.json` (1080p) |
| Sub stream, starfish pipeline, H.264 1080p or 720p, no audio | `VDEC` 1, `G1_H264` 1; connector `SUB_SCALER` 1. No `VDEC_BUS` | `m3-front/messages.txt` (acquire completed beside YouTube, no policy action) |
| Sub stream, LumaTV's NDL session | `VDEC` 2, `G1_H264` 4, `VDEC_BUS` 2; connector `SUB_SCALER`, and `videooutputd` reads it as `subScalerOccupied true` | `luna-e2/messages/messages:915-940`; fits beside YouTube only because 2+2 of 4 and 4+2 of 6 are exactly free |
| Live TV | `VDEC` 2, `VDEC_BUS` 2, `G1_MSVC` 2, `G1_H264` 2 | `luna-lgmv-yt-no-video/ums-pipelines.json` |

- H.264 sub and HEVC main use different cores (`G1_H264` against `HEVCP1_HEVC`), so only the shared counters `VDEC` and `VDEC_BUS` can contend.
- Net: beside Netflix 4K, free `VDEC` is 0 and a one-unit request is short. Beside YouTube, free `VDEC` is 2 and the starfish sub stream fits with one spare.
- The numbers come from `libmedia-resource-calculator.so.1`, loaded inside each app's own jail (`arch/app-libs-01.txt:21,50,82`) from the app's load (codec, size, rate, HDR, DRM). Its strings dump is empty and the library is not on disk, so the formula is not derived; the captured outputs above are the evidence, and each new app has to be read once.
- Memory: the decoder picture pool (`vpb`) is sized by the first decoder that opens (`SRC: kdriver/core/vdec/ipc/local/hma.c vpb_open`): secure or non-HEVC 4K takes the full pool, 4K HEVC the 164 MB partial pool, and a later opener only shares it ("vpb_open called twice", no resize). A 1080p H.264 stream opened after the main one needs no pool of its own; its bitstream buffers come from the shared `cpb` pool (66 MB, `hw-transcode.md`) and the CMA is thin (49 MB free at Home). [INFERENCE] fits; the test records it.
- Scalers: a source above 2K at 61 fps or more turns on both paths (`DE_IPC_SET_HIGH_FRAME_RATE`, `V4L2_CID_EXT_VSC_OCCUPATION_SUB_SCALER`; `framerate-result.md:155-165`), and then `SUB_SCALER` is not free for anyone. 4K at 59.94 or 60 (Netflix, YouTube) uses `MAIN` only: `subScalerOccupied false` in `luna-yt/videooutput-status.json` and every Netflix log.
- Sub window limits (`multiview_sub` in the codec file): 1920x1088 at 30 for H.264 and H.265. The camera clips at 25 to 30 fit; a 1080p60 stream does not.

## Preempt or refuse: preempt

`SRC: ResourceManager::selectPolicyCandidates`, `AcquireQueue::processRequest`:

1. The request is tried; the shortage (units not free) goes to policy.
2. Holders are ranked by pipeline-type priority, then foreground, focus, visible, then last use (oldest lowest). Every type in the set's config has `priority = 4`, so the order is decided by the rest.
3. Every acquire stamps the requester's last-use time now, so the new pipeline outranks every existing holder. Holders are walked lowest first until the shortage is covered; each is sent a policy action (`released by policy action`, error 600; Cobalt logs `avc.proxy policyActionCallback`).
4. The request fails only if no set of holders covers the shortage, or a holder answered DENIED. The requester is never the one refused when it is newest.

Observed once: in M2 the feeder's pipeline asked for `MAIN_SCALER` and YouTube's connector got `policy action by avconnector`, its video disconnected (`multiview/tv-2026-10-30/m2/messages.txt:94`; `multiview-result.md:37`). Round 20's error-600 failures read the same way.

The set's binary adds `levels` (overlay, screen saver, multiview, card, floating, background; `getActivePipelines` shows `level` 4 `card` for apps and the feeder opened for the app, 3 `multiview`), so ranking may compare level first [INFERENCE]; a feeder opened for the front app's id is level 4 like the app, so the tie falls to the order above.

So a short acquire does not fail safe: the main stream pays. The check has to run before the load.

## Advance check (read-only, before opening the sub window)

1. `luna://com.webos.media/getActivePipelines {}` (the helper already calls it: `helper/app/src/app_priority/lg.rs`). For each pipeline that is not LumaTV's own, count its `resource` entries per name: `held[name]`.
2. `free[name] = TOTAL[name] - held[name]`, with TOTAL read once from `/etc/umediaserver/umediaserver_resource_config.txt` on the set (the helper runs as root).
3. `luna://com.webos.service.videooutput/getStatus {}`: `MAIN` `videoInfo.subScalerOccupied` is false, `SUB0` is not connected, and `MAIN` frame rate is under 61.
4. Go for the hardware sub window only if `free.VDEC >= 1`, `free.G1_H264 >= 1`, `free.SUB_SCALER >= 1`, and steps 3 hold. For an NDL sub stream the need is `VDEC` 2, `G1_H264` 4, `VDEC_BUS` 2 and both scalers; it fits beside YouTube only exactly, so the starfish stream is the one to use.
5. Otherwise grant the texture (or stills). Do not open the pipeline and hope.

Corroboration, not part of the rule: the Multi View controller's `pipelineInfoUpdate` carries `extraInfo` `videoHalfOccupied` and `maxDisplayManager` for the app's pipeline; all 32 samples on disk (YouTube) read false, and the meaning of `maxDisplayManager` (likely the Dolby `DM` units all held) is [INFERENCE].

## Open: HDR on the sub window

Nothing on disk shows a sub window beside an HDR or Dolby Vision main. The set reports `multiview.supportSubHDR true` (`luna-e2/` MVC status), but whether connecting `SUB0` makes `pqcontroller` redo the Dolby or HDR mode (a black frame or picture-mode flash) is unmeasured. The test must watch it.

## The TV test that settles it

Two parts; the first changes nothing. Receiver muted; each app plays from Home's launcher, then LumaTV is brought to the front only as in `subwindow.sh`.

**A. Read-only (per app: Netflix 4K, Plex 4K HDR10 HEVC, YouTube 4K HDR if offered; Nitin starts playback).**
While the film plays: `getActivePipelines`, `videooutput/getStatus` (size, rate, `hdrType`, `subScalerOccupied`), and `grep ACQUIRE_REQUEST /var/log/messages` for the app's own set (`mvread.sh luna <tag>`). Fill the table above for each app and compute `free` with the rule. Predictions to confirm: Netflix `free.VDEC` 0, YouTube 2. This also reads whatever Plex declares.

**B. The sub window over each app whose `free.VDEC >= 1`** (YouTube 4K first, then Plex if it passes A): `sh /tmp/lumatv/subwindow.sh run m3` (feeder `--pipe smp --sink sub_video` for the front app's id, H.264 1280x720 at 30, or the 1080p25 camera), 60 s. Log `/proc/hma/status` and `CmaFree` before and after. Pass, all of:
- `videooutput/getStatus`: `MAIN` still the app's 3840x2160, same rate and `hdrType` throughout; `SUB0` connected at the claim's size.
- No `policy action`, `released by policy action` or error 600 in `/var/log/messages` and `getActivePipelines` still lists the app's same `VDEC` indices.
- No `_pq_setDolbyHDRCtrl` or `PQ` mode-change lines from `pqcontroller` at the connect (the HDR question).
- Three DISPLAY captures (`executeOneShot` DISPLAY, 960x540) over the minute, none black in the main area; `osd_irq` and vblank counters continuous (no gap); `audiod` shows `MAIN` source and volume unchanged, no mute.
- The app's own frame or playback counter (Plex position, YouTube stats) advances without a stutter.

**C. Netflix 4K over-ask (only with Nitin's word; it is expected to fail).** The same run over Netflix with the check bypassed (`ctl vwin` forced hardware). Expected: `policy action` for Netflix's connector within 2 s, its picture or audio drops. This confirms the preemption rule; with the check on, the pop-up must come as a texture and Netflix must not blink. Stop at the first glitch (`ctl vwin idle`).

A pass in B for a 4K HDR HEVC app moves its row to live; a fail names the unit or the HDR step that decided it. Until then LumaTV's rule is: 4K HDR HEVC main with `free.VDEC` 0, or any of step 3 failing, gets a texture; with room, the hardware sub window.

## Netflix below 4K, read during playback (2026-11-24 18:47)

A Netflix title at 1920x1080, AV1, 25 fps, SDR (secure path, `svp 1`; panel at 100 Hz) holds `VDEC` 2 of 4, `VDEC_BUS` 2, `HEVCP1_HEVC` 2, `SVP_CPB` 1, `ADEC` 1, `ADEC_BANDWIDTH` 2, with `MAIN_SCALER` and `MAIN_SOUND` on its connector; no `DM`, and `win1` is idle. So Netflix does not always take every decoder: the four-`VDEC`, three-`DM` load is its 4K Dolby Vision case, and beside a title like this one the starfish sub stream (`VDEC` 1, `G1_H264` 1, `SUB_SCALER` 1) fits with one `VDEC` spare. The advance check stays the rule: the pop-up reads `getActivePipelines` at the moment it opens. Read only; no sub window was opened. Raw: `second-window-2026-11-24.txt`.
