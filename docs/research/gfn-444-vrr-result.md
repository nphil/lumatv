# GeForce NOW on the C4: 4:4:4, VRR and HDR, every route

Follow-up to `opennow-result.md` (commit d65d391). Set: OLED65C4PUA, webOS 10.2.1, chip O22A3 (kernel cmdline), kernel 5.4.268, 144 Hz panel option. Read-only on the set; nothing changed.

## Resume here

Done (2026-10-25): every route below has a verdict backed by LG's kernel source (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, `SRC:` paths relative to it), by read-only probes of this set, and by public sources. No experiment ran. Experiments that need a write or a picture change are designed in "Experiments for approval" and wait for Nitin.

Evidence tags: `TV:` read on this set, `SRC:` LG's GPL kernel tree, `PUB:` public source, `[INFERENCE]` not proven, probe named.

## Verdict

| Route | Verdict | One-line reason |
|---|---|---|
| (a) DRM/KMS VRR properties from the own compositor | Impossible | LG's `lg1k` DRM driver attaches no properties at all and its mode list is cosmetic; the CRTC never touches panel timing |
| (b) Display engine / TCON VRR entry points driven by a non-HDMI source | Impossible without LG firmware changes | The panel's VRR mode exists (`BE_TOP_SET_DISP_MODE` 3 = "VRR(120Hz)") but it follows the HDMI receiver's sync; a decoded stream's timing comes from a vsync timer the firmware will only slew by 0.5 % |
| (c) luna / media-pipeline game mode (low delay, ALLM-like, picture mode) | Works, and NDL already sets it | NDL's load payload carries `lowDelayMode:true`, `isGameMode:true`, `maxFrameRate:120`; the kernel has a "direct video" mode written for "nvidia, netflix game" |
| (d) Fixed 120 Hz with frames shown as they arrive, instead of VRR | Works, probe to confirm the rate | HFR mode pins the video vsync at 120 Hz; direct-video mode shows each frame on the next vsync and discards backlog. One 8.3 ms slot of jitter is the whole cost |
| (e) HEVC / AV1 / H.264 4:4:4 through the hardware decoder | Impossible | The decoder firmware logs "This decoder cannot support other color domain" for `chroma_format_idc != 1`; the kernel hard-codes 4:2:0 for every decoded frame it hands to the display |
| (f) CPU decode of 4:4:4 (no GPU decoder exists) | Not viable for play | Roughly 1080p at 25-40 fps on the four A76 cores at 1.4 GHz before the network stack is counted [INFERENCE]; the display side would accept the frames |
| (g) GFN sending 4:4:4 to this client | Only to native Windows/macOS clients | NVIDIA gates 4:4:4 by client; OpenNOW asks as a native client and falls back to 4:2:0 when the decoder lacks it |
| (h) A device on HDMI | Needs hardware Nitin does not have | The HDMI inputs do 4:4:4, VRR and HDR; nothing is connected to them except the Denon/Apple TV on HDMI 2 |
| "Pretend to be an HDMI input" | Impossible | Memory frames enter the display engine through a different door (VDO) than live inputs (CVI); the only internal generator on the HDMI side is a register test pattern; the SoC has no HDMI transmitter to loop back |
| HDR10 | Works (one probe from certain) | Static metadata from the stream and from `NDL_DirectVideoSetHDRInfo` both reach the display engine; GFN sends HDR10 only, never Dolby Vision or HDR10+ |

Recommended: build the port on NDL as planned, at 4K120 (fallback 1440p120 or 4K60) HEVC Main10 HDR10 4:2:0, fixed 120 Hz, no VRR, no 4:4:4. That is what LG's own GeForce NOW app gets on this model (PUB: LG lists OLED65C4PUA under "2024: 4K streaming in HDR, 120 Hz on select OLED models", https://www.lg.com/us/support/help-library/compatible-lg-tvs-with-nvidia-geforce-now-CT10000020-20152438861512). 4:4:4 and VRR are off the table on any on-TV path; they need a PC or console on an HDMI port.

## How a picture reaches the panel (the answer to "pretend to be HDMI")

Two doors into the display engine (DE):

- **CVI** (captured video input): live sources with their own sync. The source table is fixed in silicon: CVD, ADC, HDMI (SRC `core/de/v4l2_vsc_event.c:226-236`, `cvi_common_config`). HDMI 2.1 feeds it in "PVO" mode, the receiver's sync drives the capture (SRC `core/hdmi20/hdmi21_engine/hdmi21_hal_driver.c:3315`). Game-mode flags (FreeSync, G-Sync, VRR, FVA, MConst, QMS) are copied from the HDMI timing block into the CVI source descriptor and nowhere else (SRC `core/de/vp_kwrap.c:1293-1303`, `core/de/v4l2_vsc_event.c:2623-2628`, `include/de_kapi.h:947-959`).
- **VDO** (video from memory): decoded frames, JPEG stills, camera and casting. The decoder block owns six vsync timers wired 1:1 to the DE's input channels ("vsync N --- pqe N", SRC `core/vdec/hal/vsync.c:177-185`); each frame is announced to the DE through a register block with map type, colour format, HFR bit and a "direct media" bit (SRC `core/vdec/hal/deipc_reg.h:20-55`).

The HDMI receiver has a hardware test-pattern generator with its own sync generator (SRC `core/hdmi20/hdmi20_engine/hdmi20_hal_driver.c:4896-4927`): registers only, a built-in pattern, no frame-buffer input. The DE's HDMI-TX hook is `NULL` on every chip (SRC `core/de/hal/de_hal.c:330-865`), and the O22 device tree has no HDMI node (SRC `dts/o22.dts`), so there is nothing to loop a transmitter back into a receiver. A compile-time FPGA fake (`USE_VIDEO_FOR_FPGA`, `core/de/vp_kwrap.c:1205-1247`) only forges timing numbers. No kernel API, debug node or firmware command moves memory frames into CVI.

## VRR

### (a) DRM

SRC `core/drm/drm.c:298-358`: the connector advertises four static modes (720p, 1080p, 4K, 8K, all "60", the 4K mode even carries the 1080p pixel clock), `drm_set_preferred_mode(1920,1080)`, and no `drm_property`, `vrr_capable` or `VRR_ENABLED` is created anywhere in `core/drm/` (grep: zero hits). `lg1k_crtc_enable` only turns vblank on (`:411-419`). TV `/sys/kernel/debug/dri/0/state`: one CRTC "1920x1080 60 148500", connector TV-1, modes 1920x1080, 3840x2160, 1280x720. The kernel's DRM core (5.4.268) has the VRR plumbing (`linux-rockhopper/drivers/gpu/drm/drm_connector.c:1535-1564`); LG's driver never uses it. Verdict: impossible.

### (b) Display engine and TCON

What exists:

- The backend (BE) display mode enum has `LX_BE_DISP_FRAME_RATE_VRR` (SRC `include/be_kapi.h:429-442`, ioctl `BE_IOW_SET_DISP_MODE` `:73`), sent to the BE firmware as `uFrameRate` 3, commented "VRR(120Hz)" (SRC `chip/o22/be/fwi/ipc_def_o22.h:196`, `chip/o22/be/top/be_top_hw_o22.c:1551-1575`). LG's VBE control that drives it only ever picks 60 or 120 (`V4L2_CID_EXT_VBE_DISPLAYOUTPUT_5060HZ`, SRC `core/be/v4l2_vbe.c:856-882`, doc `include/webosext/.../v4l2-ext-vbe.h:3707-3733`: "0: normal output of 4K100/120 Hz to the panel, 1: 4K50/60 Hz").
- The OVI (panel link) output-mode switch `LX_OVI_DISP_OUTPUT_MODE_4KVRR` is a no-op on this set: `OVI_HW_O22_SetDispOutputMode` acts only on a 120 Hz panel, and this panel reports the 144 Hz option (SRC `chip/o22/ovi/ovi_hw_o22.c:4239-4299`; TV `/proc/lg/ovi/status`: `panelFramerate 3 (0:60,1:120,3:144)`, `dispFramerate 3`).
- The DE firmware reports VRR state and the measured VRR input rate in two status registers (`vrr` bit 23, `frate_vrr`, SRC `include/de_kapi.h:3686-3712`), read by `vsc_check_vrr_freq` for the HDMI driver (`core/de/v4l2_vsc_event.c:3285-3292`). VRR de-flicker and brightness compensation LUTs are keyed to "the framerate of the VRR signal" (SRC `v4l2-ext-vbe.h:4576-4590`, `v4l2-ext-pq.h:6702-6714`).

What is missing: any way for a memory source to drive that timing. The DE's output frame rate for memory sources is a fixed number (`DE_IOW_SET_FR_RATE`, `LX_DE_FR_RATE_T {isForceFreeRun, fr_rate}`, SRC `include/de_kapi.h:565-572`), chosen as 50, 60 or 48 by `_vsc_check_fr` (SRC `core/de/v4l2_vsc_event.c:2676-2715`); 100/120 Hz is a separate "high frame rate" switch (`DE_IPC_SET_HIGH_FRAME_RATE`, SRC `core/de/mcu/de_ipc_def.h:214`). The DE firmware command list has frame rate, low delay, frame delay, "low delay by source sync" and "network sync", and no VRR command (SRC `core/de/mcu/de_ipc_def.h:136-222`). The decoder-side vsync timer can change its period at run time, but LG caps the slew at 5 ‰ of the base period (`VSYNC_SHIFT_LIMIT 5 kppm`) in steps of 300 ticks of 27 MHz (11 µs) per vsync, because "changing vsync interval too much makes display (DE, FRC) wrong" (SRC `core/vdec/vds/vsync.c:30-34, 415-450, 545-560`). That is a 59.94/60 lock, not a 48-120 Hz window.

So the three pieces (panel VRR mode, DE low-delay-by-source-sync, a tunable source vsync) exist separately, and the part that would couple them for a memory source lives in the closed DE/BE firmware (`g_fw_mcu*`, TV `/proc/lg/de/status`). Forcing the BE into VRR mode under a decoded stream is possible to try (experiment E4) and is expected to change nothing useful, at real risk to the panel timing. Verdict: impossible without LG.

### (c) Game mode through luna and the media pipeline

This is LG's own cloud-gaming path and NDL rides it:

- NDL DirectMedia on this firmware builds a Starfish load with `option.lowDelayMode`, `option.adaptiveStreaming.maxFrameRate` (from the capability file, 120 for H.265) and `option.videoInfo.isGameMode` (TV `strings /usr/lib/libNDL_directmedia_impl.so.1.0.0`, keys in that order; `libndl-media 1.0.0-44.webos4tv.17`). On webOS 10 the values are `lowDelayMode:true`, `isGameMode:true`, `maxFrameRate:120` (PUB jellyfin-webos-native `docs/ndl.md`, payload captured by interposing `StarfishMediaAPIs::Load`).
- The pipeline turns these into sink properties: `directmedia-frame-drop-threshold` ("Google Cloud Game) frame drop threshold"), `low-delay`, `Use HFR` (TV `strings /usr/lib/gstreamer-1.0/libgstlxvideosink.so`; `libplayerAPIs.so` logs `lowDelayMode[%d]`, `isGameMode[%s]`, `hfrEnabled`).
- In the kernel a drop threshold above 0 selects `VDISP_MATCH_DIRECT_VIDEO`, commented "game mode (nvidia, netflix game)" (SRC `core/vdec/vo.c:545-546`, `core/vdec/vds/vdisp_types.h:29`). Direct video shows every frame at the next vsync and discards backlog once the queue stays at the threshold (SRC `core/vdec/vds/vpts.c:871-895`); it sets the "direct media" bit the DE firmware reads as its low-delay mode "(camera, widi, miracast, cloud game)" (SRC `core/vdec/hal/deipc_reg.h:53`, `include/de_kapi.h:3574`). Picture-side low delay is a separate control (`V4L2_CID_EXT_VPQ_LOW_DELAY_MODE`, `bGameMode`, SRC `v4l2-ext-pq.h:4345-4390`) that `pqcontroller` drives (`pq_setGameMode`, `mw_pq_setLowDelayMode`, `isGameMode`, TV strings).
- `videooutputd` tracks per sink `contentType` (values include `game`), `hdrType`, `hfrEnabled`, `allmEnabled` and logs "The Instant Game Response is launched." (TV strings `/usr/sbin/videooutputd`); its luna surface is `com.webos.service.videooutput` (TV `ls-monitor -i`: `connect`, `setVideoData`, `display/setDisplayWindow`, `setDisplayWindowProperty`, `requestVideoDelay`, `setFrameDelay`).

ALLM is an HDMI signal; the app-side equivalent is exactly these flags. Whether the picture mode flips to a game mode for an NDL app (settings keep `hdrFilmMaker` for apps in HDR and `filmMaker` in SDR on this set, TV `getSystemSettings`; the game modes are `game`, `hdrGame`, `dolbyHdrGame`, TV `pqcontroller` strings) is a one-line read during the probe (E1). Verdict: works, nothing to build beyond the NDL feeder.

### (d) Fixed 120 Hz as the VRR substitute

- HFR mode is set automatically for streams above 1920x1440 with an ES or container rate of 61 fps or more, or by the pipeline's HFR flag for anything else (SRC `core/vdec/vo.c:578-605`, `core/vdec/driver.c:1734-1745`, `core/vdec/vdec_internal_types.h:27`). HFR pins the vsync to 120 Hz (100 for 50-family rates) and fixes it (`core/vdec/vds/vdisp_core.c:1078-1089`); direct video otherwise fixes 60 (`:1254-1258`). For 1080p120 or 1440p120 the pipeline's flag must be on; `adaptiveStreaming.maxFrameRate:120` is how NDL asks [INFERENCE: read `vdisp` status during E1 at 1440p120].
- Cost of no VRR: a frame that misses its 8.33 ms slot is shown one slot late and the next one is dropped once the backlog crosses the drop threshold; that is the same stutter a PC shows on a fixed-rate monitor, bounded at 8.3 ms. With `cloudGsync:false` the GFN server renders at the fixed stream rate, so cadence is regular and the TV's 120 Hz matches it 1:1 (opennow-result, "OpenNOW's architecture"). For 60 fps tiers the HFR flag off gives a 60 Hz lock with the same property.
- The UI plane stays at its own rate (vblank 60 Hz while DE vsync runs 120, TV `perf-tweaks.md`); a HUD over the stream does not need 120.

Verdict: works; E1 records the measured flip rate.

## 4:4:4

### (e) Hardware decoder

- HEVC: the decoder firmware (`/lib/firmware/vdec.bin`, 6,656,504 B, loaded by `firmware_vdec.service`, TV) parses `[sps] chroma_format_idc` and carries the error pair `WARN : chroma_format_idc %d` / `This decoder cannot support other color domain.!!`; it names `PROFILE_MAINREXT` and `PROFILE_HIGHTHROUGHPUTREXT` only in a profile-name table (TV `strings`). The kernel's decoded-buffer struct has one colour format, `fb_color_format_420` (SRC `core/vdec/include/vdec_kapi.h:373-376`); its output maps are tiled HEVC 8/10-bit and AV1 FBC 8/10-bit (`:130-136`), and every decoded frame is announced to the DE as `VDISP_FRAME_FORMAT_420` unconditionally (SRC `core/vdec/ipc/local/v4l2_vdec.c:487`). So even a firmware that decoded 4:4:4 would be displayed as 4:2:0 garbage.
- AV1: the firmware rejects by subsampling ("Stream not supported WxH:%dx%d bit_depth:%d subsampling_x:%d subsampling_y:%d soc_model:%s", TV strings). GFN does not offer AV1 4:4:4 anyway (OpenNOW `docs/hdr.md`).
- H.264: the firmware's H.264 profile table is Baseline/Main/High (TV strings `H264_BASELINE_PROFILE`, `H264_MAIN_PROFILE`, `H264_HIGH_PROFILE`, "Unsupported profileId"); no High 4:4:4 Predictive.

Verdict: impossible. E2 confirms the negative on the set if wanted.

### (f) Software decode

- The GPU (Mali-G52) has no video decoder; a shader decoder for HEVC does not exist in practice. CPU is the only option.
- The set ships FFmpeg 5.0 (`libavcodec.so.59.18.100`, decoders `hevc`, `h264`, `mpeg2video`, `opus`, `flac`; `libgstlibav.so`), 32-bit and compiled `-mcpu=cortex-a9` (TV strings). A 64-bit build with NEON would be needed for speed.
- Throughput: dav1d, the most optimised software decoder available, runs 1080p 4:2:0 at 108 fps and 4K at 23.7 fps on four A76 cores at 2.4 GHz (PUB https://openbenchmarking.org/result/2403018-NE-2401257NE67). This set's cores run 1.4 GHz with one core offline at Home (`hardware-census.md`), FFmpeg's HEVC decoder is slower than dav1d, 4:4:4 doubles the sample count, and game streams at 50-100 Mbit/s are far heavier than a nature clip. Expected: 1080p 4:4:4 somewhere around 25-40 fps on all cores, with nothing left for SRTP, FEC and Opus [INFERENCE: E3 measures it].
- The display side would take such frames: `/dev/vo` accepts 4:4:4 and RGB raster frames (`VO_SET_COLOR_SUBSAMPLE` 0x00010001 / 0xFFFFFFFF, SRC `core/vdec/vo.c:1041-1055`, `VO_WRITE_PICTURE` `core/vdec/include/vo_kapi.h:89-131`), the DE frame descriptor has a 2-bit colour format (SRC `core/vdec/hal/deipc_reg.h:40`, `include/de_kapi.h:3566` "for MJPEG"), and the JPEG path already shows 4:4:4 stills on a video window (`hardware-census.md`, `/proc/lg/de/usages` src 5). The graphics plane (RGB, full chroma) is the other output, SDR only and at the UI's 60 Hz [INFERENCE: OSD blending is SDR-composited by the BE's OSD level controls].

Verdict: not viable for play; the decode, not the display, is the wall.

### (g) What GFN sends

NVIDIA: "YUV 4:4:4 is currently available on the native Windows or macOS app" (Ultimate only); the LG TV app gets HDR10 and 10-bit, no 4:4:4; Cinematic Quality Streaming (4:4:4 + HDR10 + high-bitrate AV1) is "native apps on Windows, macOS and Shield TV" (PUB https://nvidia.custhelp.com/app/answers/detail/a_id/5390 and `a_id/5711`, search excerpts; the pages return 403 to fetchers). OpenNOW requests ten-bit 4:4:4 HEVC only when the platform backend proves a 4:4:4 decode fixture; otherwise preflight reduces chroma to 4:2:0 and records `stream_profile_fallback` (PUB OpenNOW `docs/hdr.md`, "10-bit 4:4:4"). A webOS backend reports H.265 Main/Main10 4:2:0 and the session starts at 4:2:0 without server-side drama.

### (h) HDMI

Both features exist on the C4's HDMI inputs (opennow-result, last section). Nothing of Nitin's is on those ports besides the Denon and the Apple TV, so this is a purchase, not a port.

## HDR

- **What GFN sends.** HDR10: PQ transfer, BT.2020 primaries, static mastering metadata, 10-bit, HEVC or AV1. CloudMatch gets `sdrHdrMode=1`; `displayData` carries the client's luminance (OpenNOW defaults 1000 nits max, 400 frame-average, 0 min) and optional primaries; the server's encoder writes the VUI and the MDCV/CLL SEI into the stream, and OpenNOW reads transfer, primaries, matrix and range back from the decoded stream (PUB OpenNOW `docs/hdr.md`, "Color and ownership contract"). There is no Dolby Vision or HDR10+ in the service (NVIDIA's HDR article is HDR10 only, KB 5390); the set's Dolby paths (`hevcpx_o18_dolby_mode`, `dolbyHdrCinema`) do not apply.
- **Hardware path (NDL and Starfish are the same path).** The decoder firmware parses VUI and the mastering-display and content-light SEI into `decoded_buffer_hdr` (SRC `core/vdec/include/vdec_kapi.h:717-736`); the kernel packs them into an extra-data block per frame and marks the frame HDR10 when transfer = 16 (SRC `core/vdec/driver.c:825-885`, `save_extradata`); the address rides with every frame into the DE (`core/vdec/vds/deif.c:272`, `core/vdec/hal/de_ipc.c:283-284`). `NDL_DirectVideoSetHDRInfo` builds `{hdrType:"HDR10", sei:{displayPrimaries, whitePoint, luminance, maxContentLightLevel, maxPicAverageLightLevel}, vui:{transferCharacteristics, colorPrimaries, matrixCoeffs}}` and hands it to `StarfishMediaAPIs::setHdrInfo` (TV strings `libNDL_directmedia_impl.so`), which the sink applies as `hdr-type`/`hdr_sei`/`hdr_header` (TV strings `libgstlxvideosink.so`, `libgstlxvideodec.so`). Both feeds end in the same DE extra-data block; a GFN stream carries its own SEI, so `SetHDRInfo` is belt and braces, not a requirement [INFERENCE: which one wins when they differ is unmeasured].
- **Panel processing.** The picture engine applies HDR10 (EOTF, tone map, 3D LUT; SRC `v4l2-ext-pq.h:1264-1619`) to the video window, not to the UI plane; `/proc/lg/de/win0/status` shows `hdr_type` per window (TV). The picture mode follows the dynamic range: this set keeps `filmMaker` (SDR), `hdrFilmMaker` (HDR10) and `dolbyHdrCinema` (Dolby Vision) for apps (TV `getSystemSettings`, dimension `input:default`), the last one observed live during Netflix (`ambient-result.md`). With `isGameMode:true` pqcontroller may choose `hdrGame` instead (TV strings) [INFERENCE: read during E1].
- **Combinations.** Hardware path: 4K120 + HDR10 + 4:2:0 at fixed 120 Hz is the ceiling (the capability file allows H.265 4096x2304 at 120 fps, 50 Mbit/s, TV `/etc/umediaserver/device_codec_capability_config.json`; LG's own app does 4K120 HDR on this model, PUB above); 4:4:4 and VRR never. Software 4:4:4 path: SDR on the graphics plane, or HDR via `/dev/vo` with the extra-data block attached, at whatever frame rate the CPU manages (too low). HDMI device: all four at once.

## Experiments for approval (writes or picture changes; none run)

| # | What | How | Expected | Risk | Revert |
|---|---|---|---|---|---|
| E1 | The planned NDL probe app, extended | Phase 1 of opennow-result plus: flip rate from `drm_vblank_event`/`DE2VSYNC` counts, `/proc/lg/de/win0/status` (`hdr_type`, `lowdelaymode`), `getSystemSettings` picture mode while a 4K120 HDR10 and a 1440p120 clip play, vdisp HFR state | 120 Hz vsync at 4K120; HDR10 engaged; game or filmmaker HDR mode named | Low: LG's public media path; app install is Nitin's | Close/uninstall the app |
| E2 | 4:4:4 negative test | Feed an x265 Main 4:4:4 10 clip through the same probe | Black or garbled window; decoder log line about colour domain | Low | Unload |
| E3 | CPU 4:4:4 decode benchmark | Static 64-bit FFmpeg in `/tmp`, `nice -n 19 ffmpeg -benchmark -threads 3 -i clip_1080p60_444.hevc -f null -`, 30 s | 25-40 fps at 1080p [INFERENCE] | Low: CPU load only, no picture change; LG services may stutter for 30 s | Process ends; delete `/tmp` file |
| E4 | Force the panel's VRR mode under a decoded stream (not recommended) | `BE_IOW_SET_DISP_MODE {3840, VRR}` on `/dev/lg/be0` during E1's 120 fps clip; read the DE status registers (`vrr`, `frate_vrr`) and the flip trace | No change in effective timing; possibly flicker or a black panel | Medium-high: TCON/DBC LUTs expect an HDMI VRR source; a wedged display until reboot is plausible | Same ioctl with `FRAME_RATE_120`, else power-cycle |

E4 answers a question whose answer the source already gives; it is listed so the choice is Nitin's.

## Not found

The exact picture mode LG assigns to an `isGameMode` app stream; whether `adaptiveStreaming.maxFrameRate` alone enables HFR below 1920x1440; the active TCON rate at Home (120 or 144; the DE vsync runs 120/s, `perf-tweaks.md`); the official app's media recipe (its 808 KB package is LG-encrypted, `NCGFILE_HEADER`, TV).
