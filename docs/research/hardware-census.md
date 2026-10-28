# C4 hardware census for LumaTV

## Resume here

Done: one read-only slot on the set (2026-10-25 02:52-03:00 EDT; two 10.6 s counter windows, /dev and /proc listings, about 45 libraries and tools copied off for `nm`/`strings`), the static source reads, the ranking below. The set was returned in Active Standby.

Open: the five first experiments (each needs its own TV slot; none has been run) and the two offline signature-recovery steps they depend on (`libvtcapture` histogram calls from `research/zones/huesync-service`; `DILE_JPEG_Decode` from `libdile_photo.so.1.0.1`, done: `hwjpeg-result.md`). Raw captures were scratch (`/tmp/hwcensus/tv`); everything the ranking rests on is inlined here. Do not walk `/proc/lg` blindly: a Python reader segfaulted on it (userspace only), and `/proc/lg/irb/temperature` refuses root reads.

## Evidence key

`TV:` measured on this set (OLED65C4PUA, webOS 10.2.1 build 5202, kernel 5.4.268-329.ptl4tv.2, `chip=O22A3`, DT model "LG Electronics, DTV SoC LG1213 (AArch64)"). `SRC:` LG's published O22 GPL tree (webOS 25, newer than this set; kdriver ioctl headers and the 5.4 tree). `PLAN:` earlier LumaTV measurement. `[INFERENCE]` needs the probe named next to it.

## Ranking

Order is (benefit x feasibility) / (risk x effort), under the 60 fps rule and Principle 0. None of the five needs a kernel module.

| # | Block | Why | First experiment |
|---|---|---|---|
| 1 | Display-engine video tap and picture-engine histogram (`/dev/video60 vt-capture-dev`, `libvtcapture`) | Per-zone colour statistics straight from the video hardware: Hue/WLED sync at under 1 % CPU. Nobody holds the node now. Already in PLAN (ambient light); this census adds a 24 MB DE buffer pool that looks like the tap's (`g_de_vt_m1` [INFERENCE: name only]) and `video70 gps-capture-dev`, which offers graphics-plane capture too. | Recover the histogram signatures offline, then a 32-bit `dlopen` probe: 60 s at Home, 60 s on HDMI, 60 s of Netflix. |
| 2 | The TV's own audio mix (ALSA card 1 `LGSIC`: `MixerCapture`, `SoundEngineCapture`) | Audio-reactive lights and a loudness meter with no microphone. Five capture streams exist, no process holds any of them. PLAN has video sync but no audio sync. | Five seconds from `hw:1,2` and `hw:1,3` while LG plays a tone, receiver muted through Home Assistant; check for non-zero changing PCM and an unchanged DSP reset count. |
| 3 | Hardware JPEG block (`/dev/lg/mjpeg0`, `JpgDec` IRQ) with JPEG as a display-window source | Decode and encode, scaled decode 1/2-1/8, rotation, 32k x 32k. IRQ count 0 since boot. Takes wallpaper and poster decode off the CPU, and a 4K still can sit on a video window with no GPU (`DILE_VO_DisplayPicture`; the DE lists `jpeg` as a window source) [INFERENCE until run]. Encode gives cheap screenshots for the gated MCP/HA view. | Decode one 1080p and one 4K JPEG through `DILE_JPEG_Decode` from a 32-bit tool at Home; ms per image, CPU, `JpgDec` IRQ delta, bytes against the CPU decoder. |
| 4 | Hardware video decode (`video21 lgsoc-vdec-decoder`, `video22 lgsoc-vdec-sink`, `/dev/lg/vdec0-7`) | A looping 1080p wallpaper or screen saver on a video plane. The as-found Screen Saver state costs 54.5 % Mali and 60 flips/s on this set [INFERENCE: the animation is LG's screen saver app]; Home with 10 fps wallpaper motion costs 10.0 % and 12.4 flips/s. A video-plane wallpaper would cost the GPU nothing. Limits: the GPU cannot sample a video plane, so blur-behind over it is not possible; CMA is tight during playback. | 60 s of a looping 1080p HEVC clip through LG's own pipeline in a test web app: Mali busy, CPU, `CmaFree`, `CmaDeviceAlloc`, flips/s against the two baselines above. Status: the art saver on the video plane is built and host-tested, not run on the set (`videoplane-saver-result.md`, TV steps); the wallpaper is a design there. |
| 5 | Bluetooth LE radio through LG's `com.webos.service.bluetooth2` | Presence and BLE sensors for Home Assistant. Raw HCI is not available: no `hci0`, the controller is `/dev/stpbt`, owned by `webos-bluetooth`. Only LG's luna service can scan. | `adapter/getStatus` (read only), then one 15 s passive scan if the method exists. Needs Main's OK because it changes the state of LG's Bluetooth stack. |

**The NPU is not top 5.** `/dev/npu0` is idle and unclaimed (below), but the dual-core DNE runs only programs compiled offline into LG's `.lgnpu` package format, no public compiler exists (web search found nothing; no `.lgnpu` file is shipped on the set, and `lgnpu_nn_hal_test` expects `./dne_custom_op.lgnpu` from LG's build tree), and every small model LumaTV wants runs on the CPU, which is 89 % idle at Home. Rank 6, reopen only if a compiler or a package appears. Detail in the NPU section.

## Live counters

Both windows 10.6 s, read through `/proc/interrupts`, `/proc/stat`, `mali0/dvfs_utilization`, `/proc/lg/fbdev/fb0/flip_cnt`. "Idle" is the set as found (power state `Screen Saver`, LumaHome behind it); "Home" is after `powerOn` + `turnOnScreen`, foreground `com.webos.app.home`.

| | Idle (Screen Saver) | Home |
|---|---|---|
| fb0 flips/s | 60.0 | 12.4 |
| Mali busy | 54.5 % | 10.0 % |
| CPU busy, online cores | 20.1 % of 4 (CPU3 online for part of the window) | 10.8 % of 3 (CPU3 offline, mask 0-2) |
| `surface-manager` | 23.6 % of a core | 5.3 % |
| `pqcontroller` | 8.4 % | 8.2 % |
| `lginput2` | 2.2 % | 1.7 % |
| IRQ/s: `osd_irq` | 120 | 120 |
| IRQ/s: `DE2VSYNC` | 0 | 121 |
| IRQ/s: `eth0` | 43 | 69 |
| IRQ/s: `gfx_irq` (2D blitter) | 0.4 | 0.4 |
| IRQ/s: `TE_MCU[0]`, `[1]` | 0.9 each | 0.9 each |
| IRQ/s: `hdmicec` | 0.9 | 1.0 |
| IRQ/s: `mmc0` | 0 | 0.6 |
| SoC temperature (`/proc/lg/pm/temperature`) | 57 C | 56 C |
| `MemAvailable` / `CmaFree` | 684 / 48 MB | 735 / 68 MB |

Nothing else counted: no JPEG, TE hardware, I2C, USB or other peripheral interrupt moved in either window. Since boot (2 h 25 min): `JpgDec` (IRQ 111) 0, `TE_HW` (114) 0, `gfx_irq` 3,058. NPU, VDEC, VENC, ZACC, audio DSP, PDM/WoV and the three Mali lines are absent from `/proc/interrupts` altogether, which proves nothing about use (Mali is plainly in use): they are requested elsewhere or lazily. Pressure at Home: cpu some avg10 3.94, memory some 0.60, io 0.

CPU3 is hotplugged out at Home by LG's PM governor (`/proc/lg/pm/status`: `cpu_num: 3`, load 9/16/100 %). `pqcontroller` burns about 8 % of a core in both states and `lginput2` about 2 % (7 RT threads), so the quiet floor on this set is roughly 10-12 % of one core before any LumaTV work.

## Who holds what (TV: `ls -l /proc/*/fd`, root, jailed apps shown with their jail path)

Held: `/dev/lg/gfx0` by `pqcontroller`, `eplmanager`, `captureservice` (two fds each); `/dev/video40`,`50` by `pqcontroller`/`panelcontroller`/`eplmanager`; `video30`,`31` by `videooutputd`/`avmutemanager`; `video240` by `kadpd`/`gal_renderd`; `video241` by `nn-proxy`; `video12`,`13` by `eim`/`arccontroller`/`extinput-integration`; `video2` by `legacy-broadcast`; `/dev/lg/crypto0` by `legacy-broadcast`; `/dev/stpbt` by `webos-bluetooth`; `/dev/lg/se0` by more than a dozen services and apps, `se1` by `rmt-uta`; `/dev/snd/controlC0` by `audiooutputd`, `legacy-broadcast`, `wowplay`; `/dev/mali0` by `surface-manager`, `WebAppMgr`, `gal_renderd`, `nn-proxy`, jailed Cobalt.

Not held by any process: `/dev/npu0`, `/dev/lg/mjpeg0`, `png0`, `venc0`, `vdec0-7`, `te0`, `irb0`, `/dev/vdec`, `/dev/venc`, `/dev/vo`, `/dev/hma`, `/dev/ion`, `video20,21,22,24,60,61,70`, every `/dev/snd/pcmC*` (card 0 playback and card 1 capture), `hidraw0`. `voiceinput_sound`, `_hidraw`, `_preprocessor` hold no audio or hidraw node: voice is dormant with the screen on. `/proc/npu/npu_process_list` is empty.

Permissions: `/dev/lg/{de0,gfx0,gfx1,mjpeg0,png0,pe0,ovi0,sys0,te0,vdec0,venc0,za0,zb0,hdp0}`, `/dev/vdec`, `/dev/vo`, `/dev/hma`, `/dev/mali0`, `/dev/dma_buf_te` and `/dev/spidev0.0`,`0.1` are mode 0666/0777; `/dev/lg/crypto0`, `irb0`, `se1`, `ucom0`, `/dev/npu0`, `/dev/video*` (root, group `composit`) and the audio nodes (group `audio`) are root only. LumaTV runs as root, so none of this blocks it; it matters for what an unprivileged AI-facing process could reach.

## Blocks

### NPU (DNE, `/dev/npu0`, module `npu_drv`)

- TV: `/proc/npu/npu_device_capability`: vendor LGE, name DNE, driver 1.0, cache size 256, `tops 0.4`, `aipq compatibility : false`. `npu_utilization` reads `0`; `npu_process_list` empty; `iommuinfo` absent. Module `npu_drv` 122,880 B, used by 0. Node `crw------- root 238:0`. No process maps `libnpu`, `libtflite-npu-delegate` or `libaibsp`; `nn-proxy` maps only `DNN_SQM1.tflite` (CPU TFLite), at 0.0-0.1 % CPU.
- SRC (`npu-drv/`): dual core only (`DUAL_CORE_IDX`, "DNE only support Dual Core Mode", `npu_hwc_dne.c:1201-1222`); clock register values for 600 MHz and 400 MHz (`:84-85,595-609`); buffers are physical, from HMA pools via `hma_alloc_user` (`npu_drv.c:473`), cache maintenance by hand (`:404-422`), per-open instances (`inst_id`) and a priority queue (`PRIORITY_SCHEDULER`, `npu_drv.c:74`). 0.4 TOPS = 0.2 TMAC/s; with 2 cores at 600 MHz that is about 167 MAC/core/cycle [INFERENCE: array size not stated in source]. The 13-entry ioctl list has no lease or exclusivity call [INFERENCE: nothing else in the kernel enforces one]. The TOPS budget lives in userspace: `ai-inference-manager` (`ARM::PriorityStrategy`, `AIModelTopsProvider`) and `/usr/share/aif/model_info/model_npu_tops.json`, which lists only `ai_pq` at 0.29 TOPS, leaving 0.11 TOPS [INFERENCE: that the provider reads this file].
- ioctls (`npu_api.h`, magic `'n'`): 0 RESET, 1 LOAD_BINARY (program or parameters), 2/3 SET_INPUT/OUTPUT_TENSOR_INFO, 4 SET_FEATURE_BUFFER_SIZE, 5 SET_PRIVATE_DATA (DNE: register-init list, strides, input/output offsets), 6 REQUEST_JOB, 7 CANCEL_JOB, 8 GET_JOB_RESULT, 9 CANCEL_ALL_JOBS, 10 SET_PERIOD, 11/12 SET_JOB_INPUT/OUTPUT_TENSORS, 13 SET_AFFINITY. No read-only query ioctl: every one mutates a context. Tensor types int8/int16/int32/fp16/fp32/uint8/bf16; memory types malloc, device mmap, caller fd, caller physical address.
- Userspace on the set: `libnpu.so.1.0.0` (`lgnpu_init/prepare/invoke/free/set_affinity/set_period`, a flatbuffer `NpuPackage` parser: program, parameters, tensor shapes, DNE or Linux ("LNX", CPU) private data), `libnnrt` (`Nnrt::LoadOpGraph/PrepareOpGraph(id, priority)/ExecuteOpGraph`), `libwebos_offline_nnhal` and `libwebos_nnhal` (an NNAPI 1.2 shaped HAL that only sleeps 1 ms, `npu-result.md`), `libtflite-npu-delegate` (options `use_custom_alloc`, `min_freq`; a custom-op delegate that executes an embedded package, which is why it applies to none of LG's shipped TFLite models), `run_lgnpu` (`-p .lgnpu -i input -o prefix -a affinity -c count -j tensor-meta.json`), `lgnpu_nn_hal_test`, `lgnpu_online_nnhal_test`, `airunner`, `benchmark_model`, `libtensorflow-lite.so` with `libXNNPACK.so`, `libopencv_dnn` 4.5.5 with `dnn_superres`, `tflite-auto-delegation` choosing among CPU, GPU, edge-TPU and NPU delegates. Shipped models: `/usr/share/aiservice/` DNN_GNET 138,592 B, DNN_SQM1-3 112,168 B each, sound_DNN 810,300 B (CPU, `default_models.json`); `/usr/share/aif/model/FitTV_Detector_Yolov3_QAT_SoC.tflite` 4,385,848 B; `/usr/share/aibsp/{GNMNET,SGNET}_o22_{param,text}.dat` (370,688 + 17,408 B and 694,272 + 14,336 B), which look like DNE program + parameter binaries for the picture engine [INFERENCE: read `libaibsp.so.1` strings]. No `.lgnpu` was found under `/usr/lib`, `/usr/bin`, `/usr/sbin`.
- LG's use: `ai_pq` only, and Filmmaker mode leaves it idle (utilisation 0, no holder).
- LumaTV uses, with arithmetic only (no measurement): MobileNetV2-224 is 300 MMAC = 1.5 ms at 100 % of 0.2 TMAC/s, EfficientNet-Lite0 390 MMAC = 2.0 ms; a realistic 25-30 % utilisation gives 5-8 ms. Keyword spotting (52-60 KB models) is sub-millisecond anywhere. LG's YOLOv3 int8 takes 421 ms and its sound model 66 ms on the four CPU cores (PLAN). Whisper-tiny/Moonshine-class speech-to-text does not fit: transformer ops (attention, softmax, layer norm) are outside what a CNN-style DNE is known to run [INFERENCE], and it would need about 16 GMAC per 30 s window.
- Effort L (reverse the package and DNE program format, or find LG's compiler), risk medium (a wrong program can hang the engine; the kernel module is LG's), kernel module no. Verdict: park. If it is reopened: step 1 `find / -name '*.lgnpu'` on every mount and read `libaibsp.so.1` strings; step 2 `run_lgnpu` on whatever package that finds, `-c 100`, utilisation read from `/proc/npu/npu_utilization` before, during, after, `ai_pq` untouched.

### Video encode (`/dev/video24 sic-encoder`, `/dev/venc`, `/dev/lg/venc0`)

- SRC (`venc_kapi.h`): H.264 and VP8 only (`LX_VENC_ENCODE_TYPE_H264=0`, `VP8=1`), inputs from the display engine (DVI), USB buffers, MJPEG, VDEC; 15-60 Hz; rate control CBR/VBR/fixed QP; GOP, IDR, scale ioctl (`ENC_SCALE`). TV: `libgstlxvideoenc.so` takes NV12, emits `video/x-h264`, properties `target-bitrate`, `dst_width`, `dst_height`. No holder; no `venc` interrupt line.
- LG's use: recording (PVR) and screen/camera paths [INFERENCE]; idle now.
- LumaTV: H.264 clips or a low-rate stream of the graphics or video plane for the gated "what is on screen" view and Home Assistant, with no CPU encode. Memory is the cost: SRC reserves 10 MB non-secure plus 12 MB secure for VENC (`platform_mem_map.h:86-105`), and CMA is 0-9 MB free during playback (PLAN). Effort M-L, risk medium, module no. Rank 7: do it after #1 and #3 prove the capture path; HW JPEG encode (#3) covers still screenshots at lower cost.

### Image decode: JPEG (`/dev/lg/mjpeg0`) and PNG (`/dev/lg/png0`)

- SRC (`mjpeg_kapi.h`): `LX_MJPEG_MAX_HSIZE/VSIZE` 32 k; decode and encode; formats 4:2:0/4:2:2/4:2:4/4:4:4/4:0:0; progressive (SOF2 marker handled); decode scaling 1/1, 1/2, 1/4, 1/8 per axis; rotation 0/90/180/270; ioctls `ENCODE_INIT/START/STOP`, `DECODE_INIT/START/STOP/SEQUENCE_INIT/PARTIAL/CONTINUE`, `GET_BUFFER_INFO`, `GET_CFG`. PNG (`png_kapi.h`): `DECODE` (full or partial), width/height 16-bit, bit depth, output format; the O22 IRQ table has a JPEG line (`O22_IRQ_INTR_JPEG`, GIC+79) and none for PNG [INFERENCE: PNG may poll or be software-assisted; probe before relying on it].
- TV: userspace wrappers `liblxjpeg.so.2.0.0`, `libhal_photo.so.2.0.1`/`libdile_photo.so.1.0.1` (`DILE_JPEG_Open/Decode/Encode/Close`, `DILE_VO_DisplayPicture/RedrawPicture`, `DILE_VO_FB_*`), `libgstlxmjpegdec.so` (+`_cmn`), `/usr/lib/camera/libdecoder-jpeg_hardware.so.1` (USB webcam MJPEG). `/proc/lg/de/usages` lists `jpeg` (src 5) as an input source of display windows w0/w1, next to hdmi and vdec: a decoded still can be shown on a video window. Kernel thread `MJPEG_WORKQ` exists at 0 CPU. IRQ 111 `JpgDec` 0 since boot.
- LG's use: the photo viewer and webcam path; unused in two and a half hours of this session.
- LumaTV: poster and wallpaper decode off the CPU, thumbnails by 1/2-1/8 scaled decode, 4K still art on a video window (screen saver, ambient art) with no GPU, JPEG encode for screenshots. CPU baseline [INFERENCE, to measure]: libjpeg-turbo on one A76 at 1.4 GHz around 70-120 MPix/s, so 1080p 17-30 ms and 4K 70-120 ms. Effort M (the libraries are 32-bit: a call from the 64-bit helper needs the small 32-bit helper or raw ioctls with the GPL structs; output buffers are driver-owned, so a copy to GPU texture remains), risk low (idle hardware; CMA is needed, so decode only when no video plays), module no.

### Video decode (`/dev/video21`, `video22`, `/dev/lg/vdec0-7`, `/dev/vdec`)

- SRC (`kdriver/core/vdec/ipc/local/v4l2_vdec.c`, `v4l2_sink.c`): V4L2 decoder named `lgsoc-vdec-decoder` plus a `lgsoc-vdec-sink` node; formats in the tree: H.264, HEVC, VP8, VP9, AV1, MPEG-2/4, H.263, VC-1, Xvid; outputs NV12/NV12M/YUV420M and LG's compressed `LFBC`. Eight `vdecN` nodes on the TV. Nothing holds any of them now.
- LG's use: all playback goes through `starfish-media-pipeline` onto the display engine's video windows, in 137 MB of secure memory (PLAN); `/proc/lg/de/status` shows `g_m0_pqe_svp` 54 MB and `g_m1_pqe_svp` 83 MB secure.
- LumaTV: #4 above, plus short intro and boot-logo animation on a video window. Frames in the clear path are NV12 in CMA, so a GPU texture upload costs the imported-memory penalty measured in PLAN (5-20x), which is why the plane route is the useful one. Effort L (window placement and punch-through belong to the own compositor; decoder instances and CPB/VPB memory come from the same CMA the UI buffers use), risk medium, module no.

### 2D blitter (`/dev/lg/gfx0`, `gfx1`; irq `gfx_irq`)

- SRC (`gfx_kapi.h`): surface alloc up to `GFX_MAX_SURFACE` 2000, manual blend, CSC table, batch run, sync, palette; pixel formats ARGB8888/4444/1555/6343, RGB565, AYCbCr variants, 422/420 planar and interleaved, index 1-8 bit, alpha-8. No AFBC. TV: pool base 0x87228000, 79 MB, 33 MB allocated in 3 surfaces, 45 MB free (`/proc/lg/gfx/mem_stat`), clock gating off, runtime-PM active (`pm_status`). Holders: `pqcontroller`, `eplmanager`, `captureservice`. Rate: 0.4 IRQ/s.
- LG's use: screenshot and panel-care paths. LumaTV: stretch/convert/blend one-offs (tint sampler downscale, screenshot format conversion, 4K logo blit), not per-frame composition: Mali's output is AFBC (`0x0800000000000062`) which the blitter cannot read, and 45 MB holds five 1080p ARGB surfaces or one 4K. Effort M, risk low, gain under 1 ms a frame. Rank 8.

### Display plane, PE and DE

- TV (`drmplanes-01.txt`, `drm-debugfs-01.txt` from PLAN's captures): one CRTC at 1920x1080@60, one connector, three planes: primary (LG's, `surface-manager` is DRM master), overlay (plane 34, idle, formats AR24 XR24 AB24 XB24 with the AFBC modifier on the two B-order ones), cursor (plane 36, AR24, 128 x 128). The overlay plane is only usable by the DRM master, so it belongs to track C (the own compositor), not to this census. OSD IRQ and vsync both run at 120/s; `ovi/status` reports a 144 Hz panel option (`panelFramerate 3`, `dispFramerate 3`: 0=60, 1=120, 3=144) next to the 60 Hz CRTC, which PLAN already treats as the UI rate.
- Picture engine: `/proc/lg/pe` (version 20210514), PE trace shows `lgsr` (LG super-resolution) database download at boot, 3D LUT, HDR, noise-reduction blocks; `de/status` firmware 7356 (2025-04-14), internal FRC. These act on the video path, not on the OSD plane, so they cannot upscale or interpolate the UI. The part that is reachable is the tap and histogram (#1): `de/status` shows a `g_de_vt_m1` pool of 24 MB (`0x80e28000`) that looks like the video-tap buffer [INFERENCE: name only], and the nodes `video60 vt-capture-dev`, `video61 vivtdev-dev`, `video70 gps-capture-dev`.
- `libvtcapture.so.1.0.0` exports `vtCapture_init/initEx/create/createEx/process/preprocess/postprocess/stop/release/finalize`, `vtCapture_capabilityInfo/planeInfo/videoWinInfo/currentCaptureBuffInfo` and `vtCapture_initHistogram/setPresetHistogram/getHistogram/finalizeHistogram`; a second `vtCapture_init` fails with code 11 (scaler busy) while another client owns it (PLAN). `libhal_vt.so.2.0.1` adds `HAL_VT_*` and `HAL_VTV_*` and a `GCP` capture path. LG's Hue Sync service calls the histogram API (PLAN). Effort S-M, risk low-medium (single owner; protected video may return black or empty statistics: that is what the experiment answers), module no.

### Audio: DSP, capture, PDM microphone, WoV

- TV: `/proc/asound/cards`: card 0 `lg115x` (10 playback, 6 capture streams, internal), card 1 `LGSIC` with capture streams `00 WoV PDM Mic`, `01 SpeakerFeedback`, `02 MixerCapture`, `03 SoundEngineCapture`, `04 BluetoothCapture` (`pcmC1D0c`-`D4c`, root:audio 0660). Four DSP trees `/proc/aaudio/dsp0-3`, `/dev/aud_blind`, `fwload_aud0-3`, `fwload_wov`. Audio runs through `audiod`/`audiooutputd` to external ARC speaker (a Denon receiver). No holder of any PCM node; nothing on the DSP is user-programmable (SRC `core/fwload/fwload_aud.c`: closed firmware images `aud0`-`aud3`, IPC only through LG's ADEC protocol).
- SRC detail (`core/aud/alsa/lgsic_mc.c`, `lgsic_pdm.c`): the mixer and sound-engine captures use 48 kHz, 2-8 channels, S16/S32, 2,048-byte periods (256 frames, 5.33 ms stereo S32); the PDM device is two-channel, 8-48 kHz, 256-sample periods, with VAD and noise-suppression registers; WoV holds a firmware-resident wake pipeline (`trigger`, `net`, `gram` images, DSP3) that runs in standby.
- LumaTV: #2 (mix capture) is a speaker-side signal, not a microphone. Risk: opening a capture inserts a capture peer into LG's audio graph, so it needs the DSP reset counter check; in ARC bitstream passthrough the capture may be silent [INFERENCE]; effort S-M, module no. **Microphone paths (PDM mic, WoV, remote voice through `voiceinput_hidraw`) are Nitin's decision**: nothing here opens them, and any voice feature must show capture state, stay off by default and never reach MCP without his action on the remote. DSP offload (own code on `aud3`, own wake model in WoV firmware): rejected, no ABI.

### Radios and USB

- TV: Wi-Fi and Bluetooth are one MediaTek USB module on xHCI bus 1 (`043e:310e`, "Wireless_Device"), drivers `wlan_mt7668_usb` (2,088,960 B) and `btmtk_usb`. Wi-Fi: `wlan0` (managed) and `p2p0`; `iw phy` lists modes managed, AP, P2P-client, P2P-GO; no monitor mode; the 2.4 GHz band shows HT20/40 MCS 0-7 (the 5 GHz block was not captured). Bluetooth: no `/sys/class/bluetooth`, no `hci0`, no `hciconfig`/`btmgmt`; HCI is the char device `/dev/stpbt` (+ `stpbt_sco`, `stpbtfwlog`), held by `webos-bluetooth` through an Android-style stack (`libbt-vendor_mtk.so`, `bluetooth-sils/libbtstdbluedroidsil.so`; Realtek vendor libraries are also in the image). Luna services present: `bluetooth2`, `btaudiosnk`, `btaudiosrc`, `bthidmanager`, `btleaudio`, `btgattwowcast`.
- Corrects the earlier source-only plan: a raw HCI socket scan (`board-exp/ble_passive_scan.py`) cannot work on this set; the only safe BLE route is LG's `bluetooth2` luna API (#5). No 802.15.4/Thread/Zigbee radio exists on board (DTS and drivers).
- USB (TV): three xHCI controllers (buses 1/2, 3/4, 5/6) and one EHCI/OHCI pair (7/8). Bus 1 is the internal radio link; bus 3 carries a `LiteEMF Hid Vendor` dongle (`hidraw0`); buses 5-6 are free; a Sony `054c:09cc` controller sits on OHCI bus 8. Kernel features that matter: `usbip-vudc` as a virtual UDC and `vhci_hcd` host controllers (USB/IP), `cdc-acm` and `usbserial` majors registered (`ttyACM` 166, `ttyUSB` 188), `tun`, `uinput`, `macvtap`. A real USB gadget controller does not exist. So a USB serial or CDC-ACM stick (a Zigbee/Thread dongle) should enumerate [INFERENCE: which USB-serial chip drivers are built in is not captured] and USB/IP could bring a remote microphone or camera in as a local device; neither is a census item to build now. Module no. Network-facing changes (AP mode, USB/IP listeners) are Principle 0 human-only steps: write steps for Nitin, do not apply.

### Security, crypto, compression

- TV: `/proc/crypto` has the ARMv8 Crypto Extensions drivers (`aes-ce`, `gcm-aes-ce` priority 300, `sha1-ce`, `sha256-ce`, `ghash-ce`) and no hardware AES driver; the CPU shows `aes pmull sha1 sha2 crc32`. AES-GCM on the extensions is around 1 GB/s per A76 core [INFERENCE]; no offload is worth it. `/proc/config.gz` is absent. No `/dev/hwrng`, no `hw_random` class; entropy_avail 256 from the kernel's `jitterentropy_rng` and DRBG.
- ZACC (`e0000000`, `e0001000`): kernel built-in (`drivers/crypto/zacc`), registered as crypto `zacc` type `compression` (not `scomp`), driven one 4 KiB page at a time by polling with a bounce copy under a spinlock (`zacc_enc.c:48-76`). Its only user is LG's compressed swap `f2io-0` on `mmcblk0p53` (capacity 629,145,600 B, `comp_alg zacc`, 614,396 kB swap with 365-463 MB in use). `/dev/zram0` exists with `lzo-rle`/`lz4` and size 0. No device node, and `AF_ALG` compression is not in 5.4, so userspace cannot reach ZACC; changing it is a kernel change (Nitin). Rejected for LumaTV caches.
- TrustZone: `/dev/lg/se0` (0770) and `se1` (0600) take `SE_IORW_RUN_TZ_OP` calls into the secure world (`se_kapi.h`); `libopteeKeymasterCA.so` (626,992 B) suggests an OP-TEE-derived TEE [INFERENCE]; DRM decryptors for Widevine, PlayReady, FairPlay and Netflix sit in `/usr/lib/drmdecryptor`; `/dev/lg/crypto0` is the transport-engine AES/DES/3DES block (`te_crypto_kapi.h`, ECB/CBC/CTR) for broadcast and PVR. There is no `/dev/tee*`, and trusted apps are signed by LG, so LumaTV cannot load its own. **Leave the whole group alone** (DRM, secure video, key ladders, `keymanager3`). Whether LG's `keymanager3` luna API can seal an AI-access secret outside AI reach is unknown; it is a key-custody decision for Nitin, not an experiment.

### IR, micom, sensors, board buses

- TV: `/dev/lg/ucom0` (micom mailbox, held by about a dozen services including `micomservice`, `tvpowerd`, `panelcontroller`), `irb0` (IR blaster driver; root, no holder; no evidence a physical emitter is fitted), `/proc/lg/pm/temperature` 56-57 C (SoC), `eplmanager` log temperature 24.4-24.6 C (panel or board, unit and location unknown). No `/sys/class/hwmon`, `thermal_zone`, IIO or backlight node; `tv.display/getLightSensorData` is not a listed service. Board buses: `gpiochip0-17`, `i2c-0..9`, `spidev0.0/0.1` (0777) and `spidev1.0`, `ttyAMA0-2` (kernel console on `ttyAMA0`), all LG's board wiring.
- LumaTV: the SoC temperature is a free read-only input for throttling optional work (S). Everything else (UCOM, IRB, GPIO, I2C, SPI, UART) rejected: panel power, watchdog and key locks sit behind UCOM, and unknown buses can lock the board.

### CPU, GPU, memory

- Four A76 at 1.4 GHz, governor `performance`, one OPP; LG's PM governor offlines CPU3 at Home. Spare at Home: about 2.7 of 3 online cores idle. A background job (artwork tagging, cache build) fits only as `SCHED_IDLE`, one core, no GPU, no CMA, paused on any frame activity; start unpinned because CPU3 comes and goes. Mali: 10 % busy at Home, 30 % during a 60 fps UI (PLAN), so GPU work other than the UI frame is rejected. Memory: `MemAvailable` 684-735 MB, `CmaFree` 48-68 MB, swap 60-75 % used: no allocation for a background feature.

## Rejected or left alone

IR blaster, UCOM, GPIO/I2C/SPI/UART, audio DSP and WoV firmware, TrustZone and DRM, ZACC, hardware AES, USB gadget, Wi-Fi AP or monitor mode, GPU or NPU background work. Reasons are in the sections above.

## Needs Nitin

1. Any microphone path (PDM, WoV, remote voice): decide whether it is ever opened, and the on-remote consent flow.
2. Key custody for the AI-access secret (TEE or `keymanager3`): feasibility unknown, his call.
3. Any kernel module or kernel config change: none of the top five needs one. The two that would unlock more are the dma-heap module already in PLAN and a ZACC userspace path (kernel patch; not recommended).
4. Network-facing radio use (AP mode, USB/IP, BLE scan policy): Principle 0 human-only steps.

## Experiment details

Common rules: mute `media_player.living_room_avr` through Home Assistant before any playback and unmute after; Picture Mode stays `filmMaker`; no writes outside `/tmp`; leave the set in Active Standby; abort on any new `failed` unit or a Mali frame rate below 59 fps.

1. **Video tap and histogram.** Offline: recover `vtCapture_initHistogram`, `setPresetHistogram`, `getHistogram` argument structs from `research/zones/huesync-service` and `libvtcapture.so.1.0.0`. Build a 32-bit probe (`zig cc -target arm-linux-gnueabihf`). Slot 12 min, read-only to LG's state: confirm no other `vtCapture` owner, `dlopen`, init the histogram for the 16 zones, poll 30 Hz for 60 s at Home, 60 s on an HDMI source, 60 s of Netflix; record zone values, probe CPU and the `/proc/interrupts` delta; finalise and release on exit. Pass: zones change with content and CPU under 1 %; on protected video, record whether the values are real or black.
2. **TV audio capture.** Offline: `ls /usr/bin/arecord`, `ls /usr/lib/libasound.so*` on the set (not captured; `libasound_module_*_pulse.so` exist, so the library is there). Slot 10 min: `cat /proc/asound/card1/pcm*c/info` (read only); mute the receiver; play LG's own sound; open `hw:1,3` then `hw:1,2` for 5 s each at S32_LE 48 kHz stereo into `/tmp`; check `od` for non-zero changing samples; read `/proc/interrupts` and the module parameter `aud_dsp_reset_count` before and after. Then repeat once in ARC passthrough to record the limitation. No microphone stream is touched (`hw:1,0` and `hw:1,4` stay closed).
3. **Hardware JPEG.** Offline part done and the slot written out in `hwjpeg-result.md` ("TV step list"). Original plan: recover the `DILE_JPEG_Decode` argument struct from `libdile_photo.so.1.0.1` and `liblxjpeg.so.2.0.0` (or build from the GPL `mjpeg_kapi.h` ioctls). Slot 10 min at Home, no playback (CMA): decode a 1080p and a 4K JPEG 20 times each; record ms per image, CPU, `JpgDec` IRQ delta, `CmaFree` before and after, and compare the output with libjpeg-turbo (`/usr/lib/libjpeg.so.62`) on the same files. Second run: `DILE_VO_DisplayPicture` of the 4K image on a video window for 30 s, Mali busy and flips/s against Home. Release the device on exit.
4. **Video-plane wallpaper.** Slot 12 min, set at Home: a test web app plays a 10 s 1080p HEVC loop for 60 s on LG's pipeline; sample Mali busy, CPU, `CmaFree`, `CmaDeviceAlloc`, `fb0` flips/s every 5 s against the Screen Saver (54.5 %, 60/s) and Home (10.0 %, 12.4/s) windows above. Watch whether the Home stub keeps its 32 MB and the UI frame rate stays at 60.
5. **BLE via LG's service.** Slot 6 min, Main's OK first: `luna-send -n 1 luna://com.webos.service.bluetooth2/adapter/getStatus '{}'`; if the service answers, one 15 s passive LE scan through its `le` methods, a second run with a known tag or phone present. Abort if the remote or Bluetooth audio drops; turn the scan off in all cases.

## Open questions

- Is the `.dat` pair in `/usr/share/aibsp` (GNMNET, SGNET) DNE program + parameters? Strings of `libaibsp.so.1` (1,058,796 B, not yet copied) decide it.
- What does the `aipq compatibility : false` line mean for sharing the engine while `ai_pq` is off? Source for `npu_device_capability` is not in the published tree (it prints from the proprietary part of `npu_drv`).
- What is the 24.4-24.6 C reading `eplmanager` logs, and which luna method serves it?
- 5 GHz capability of the Wi-Fi module (the `iw phy` capture stopped at the 2.4 GHz band).
