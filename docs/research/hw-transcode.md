# Transcoding the antenna stream to H.264 on the TV's own hardware (phases 1 and 2)

Nitin asked on 2026-11-05 whether the set can turn its antenna stream (ATSC 1.0: MPEG-2 video at 480i, 720p or 1080i, AC-3 audio) into H.264 in real time on its own hardware, so Plex and phones could play it without Plex's transcoder. It is only worth doing with under ~300 ms of added latency, a first H.264 keyframe within ~0.5 s of the MPEG-2 keyframe, no effect on the TV's own viewing, and no wedge risk in the chosen path. This note is phase 1: routes, what each costs, what Plex gains, and a probe plan. Nothing in it touched the decoder or encoder nodes on the set.

Phase 1's verdict is about Plex and stands. Phase 2 (the last section) asks the same of the clients that read the tuner directly, the phone apps, and holds the chosen route, the tuner's API and the on-set plan that replaces phase 1's. Nothing in phase 2 ran on the set.

## Verdict

**No-go unless Plex can direct-stream the H.264, and it did not; the encoder on the set remains unproven (the test used x264 on the host).** In every run Plex transcoded the H.264 video exactly as it transcodes MPEG-2: in its API with the stock Android and Web profiles, on Nitin's iPhone (captions on: Plex burns them into the picture) and on his Android phone with quality Original and subtitles off (Plex: "Direct Streaming is disabled"). Tap to picture stayed about 5 s either way; the time goes to the tune, Plex's probe and segmenting and the player's buffer, and Plex's hardware transcode on BeastNAS starts as fast as a copy (MEAS, below). No encoder probe on the set follows from this.

**The encoder takes raw NV12 frames only.** A transcode on the set therefore needs the decoder's output routed into the encoder, which nothing on the set does for a broadcast today: LG's recorder encodes what a video scaler shows, and the broadcast decoder feeds the display. The one route that could meet the latency target is the V4L2 decoder writing NV12 into buffers the encoder reads in place (route b, about 100-170 ms added [INFERENCE]). It is unproven: no step of it has run on the set, and with the payoff above it is not worth its wedge risk for Plex.

## Evidence key

`TV:` read on this set (OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, `O22N2`) on 2026-11-05 with read-only commands (`/proc`, `/sys`, file listings, copies of LG binaries; no ioctl on a video node, no LG service started). `SRC:` LG's O22 GPL tree (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, paths relative to it; webOS 25, newer than the set). `BIN:` strings and dynamic sections of binaries copied off the set (kept outside git). `MEAS:` measured on BeastNAS (Plex 1.43.5, Intel UHD 630 for Plex's hardware transcoding). `[INFERENCE]` is untested.

## What the set has for this

| Item | Value | Evidence |
|---|---|---|
| Video decoders | UMS counts `VDEC` 4, `G1_MSVC` (the MPEG-2 core) 4, `VDEC_BUS` 6; MPEG-2 up to 1920x1088 at 30 frames, 40 Mb/s | TV: `/etc/umediaserver/umediaserver_resource_config.txt`, `device_codec_capability_config.json` (copies in `gfn/tv-2026-10-26/`) |
| Video encoder | one Hantro H1 behind the video MCU: H.264 (Baseline/Main/High, CAVLC/CABAC) and VP8, I and P frames only, 1920x1080 at most; UMS counts `VENC` 1 and `AENC` 1 | `venc-result.md`; TV: same config |
| Who holds decoders while the tuner streams | none for the helper's tuner path: its windowless pipeline (`playSignaling`, purpose `WATCH_SECTION_PARSE`) claims `ADTU` and `SDEC` only. LG's Live TV and `utp changeChannel` claim the whole watch chain (`ADTU`, `SDEC`, `VTP`, two `VDEC`, `G1_H264`, `G1_MSVC`), and `changeChannel` failed while Netflix 4K held all four `VDEC` | `live-tv-capture.md` (The path), `second-window-4k.md` |
| Memory pools | `cpb` 66 MB at 0x77200000 (the V4L2 decoder's bitstream and capture buffers, the encoder's 10 MB, `photofb`), `vpb` 300 MB (the decoders' picture stores), `venc1` 12 MB (secure encode), `de-vt-m0/m1` 24/12 MB; `CmaFree` 49 MB of 692 MB at Home | TV: `/proc/hma/status`, `/proc/meminfo` |
| Kernel pieces of route b on this firmware | `dma_start` and `gps_if_*` (the tiled-to-NV12 DMA); two local `__mem_ops_attach_dmabuf`, the name of the dma-buf import in `vb2_hma.c` and `vb2_cpb.c` | TV: `/proc/kallsyms` |
| Nodes | `video20` `v4l2_live` (LG's broadcast decode path), `video21` `lgsoc-vdec-decoder`, `video22` `lgsoc-vdec-sink`, `video24` `sic-encoder`, `video27-29` `v4l2_conn0-2`, `video30-33` `vsc-video-dev0-3`, `video60` `vt-capture-dev`, `video70` `gps-capture-dev`, `/dev/venc` | TV: `/sys/class/video4linux/*/name` |
| Recovery | `/dev/watchdog` petted by `watchdogd` and `wdt-detect`; `kernel.sysrq` 0, `panic` 0, `panic_on_oops` 0; command line `wdtlog=dump@1M mmcoops=dump`; `reboot` over SSH comes back powered on; Wake-on-LAN wakes it from standby | TV: `/proc/sys/kernel/*`, `/proc/cmdline`; `HANDOFF.md` (The set) |

## Routes

### a. LG's record service (`ts-record`, DVR_SVC_VENC / SENC)

- **What it is.** `/usr/sbin/ts-record` is `com.webos.service.tsrecord`, a dynamic service (not running), API group `media`: `record/initialize`, `load`, `start`, `stop`, `unload`, `destroy`, `updateMetaData`, `getRecStatus`, `getRecTime`, `getRecInfo`, `getMPD` (TV: `api-permissions.d/com.webos.service.tsrecord.api.json`). It is built from elements: `ElemVenc` (`HAL_VENC_Open/Connect/SetParam/Start/RegisterEncoderCallback/CopyData`, "`port:%d srcType:%d pid:0x%x`"), `ElemAenc`, `ElemSenc` (a software TS multiplexer: `SENC_MAIN_*`, `SENC_UTIL_InitPAT/PCR/EIT`, `API_SENC_StartForDVR`), a PVR download, and a DASH writer (`getMPD`, `timeShiftBufferDepth`, `segmentAlignment`). Request keys include `sourceType`, `recordMode`, `vencPort`, `aencPort`, `sencPort`, `decoderIndex`, `videoPid`, `sdecChannel` (BIN). Its only caller on the set is `libdvr-proxy` (TV).
- **What its encoder sees.** `HAL_VENC_Connect` puts the encoder in record mode, which encodes what a video scaler writes back (`V4L2_EXT_VSC_DEST_VENC`, "ex. Analog Recording", SRC `include/webosext/v4l2-ext-renderer-header/linux/v4l2-ext-vsc.h:1601`; record mode `venc/hxenc/h1encode.c:970-996`). A digital program is recorded as its own transport packets through the PVR download; the encoder path is for sources that have none (analog, external inputs) [INFERENCE from the element set].
- **MPEG-2 interlaced:** yes, indirectly: the display path decodes, deinterlaces and scales; record mode takes top/bottom fields (`h1encode.c:1956-2025`) and caps 1080p at 30 frames (`FHD_FRAMERATE_MAX`, `:59`, `:2180`).
- **Latency knobs:** GOP, rate control (VBR/CBR/QP), frame rate (`LX_VENC_IOW_RECORD_SET_*`); no B frames. The service's own output is DASH segments, seconds behind.
- **Contention:** a video scaler (the sub scaler or a VSC path under `videooutputd`), `VENC`, `AENC`; the DVR stack is off in this region (`tv.rmm.dvrReady` false, TV: `/etc/configd/data/rmm/US/USA/tv.rmm.json`).
- **Memory:** the display engine's feedback buffers and the encoder's 10 MB [INFERENCE].
- **Wedge risk:** unknown. The request format is undocumented, the service drives the scaler and the DVR state machine, and the region has the feature switched off; finding the format means trial requests against LG's recorder.
- **Verdict: no-go.** Not a broadcast transcoder, no known request, DASH latency, and it borrows the display path.

### b. V4L2 decoder into the encoder (`/dev/video21` to `/dev/venc`): the vdec-to-memory route

The decoder is a V4L2 stateful memory-to-memory node; the encoder is the `/dev/venc` interface LumaTV already built for the screen (`venc/`).

- **MPEG-2:** supported by the node: `V4L2_PIX_FMT_MPEG2` maps to `LX_VDEC_CODEC_MPEG2` (SRC `core/vdec/ipc/local/v4l2_vdec.c:216,292`). Capture is `NV12` or LG's compressed `LFBC` (`:267-276`). Each decoded picture is converted from the decoder's tiled store into the NV12 capture buffer by the graphic scaler's DMA (`fbc_to_yuv` → `dma_start`, `:538-570`; `core/de/v4l2_gps.c:2691`), with the picture's scan type passed on (`set_gps_source_info`, `:518-526`); capture buffers report `V4L2_FIELD_NONE` (`:1698`), so a 1080i picture arrives as one frame with both fields woven [INFERENCE: what the DMA does with interlaced pictures].
- **Zero copy into the encoder:** the capture queue accepts dma-bufs (`VB2_MMAP | VB2_DMABUF`, `:1861`) and takes the buffer's physical address from the dma-buf's scatterlist (`vb2_hma.c:410-440`). An HMA buffer exported with `HMA_IOCTL_GET_DMABUF` carries its physical address there (`core/hma/hma_dmabuf.c:81-110`). So the decoder can write each picture into a contiguous NV12 buffer whose address the process allocated and knows, and `LX_VENC_IOWR_ENC_ENCODE` reads it in place (`bus_luma`, `bus_chroma`). No CPU copy [INFERENCE until probed].
- **Interlaced output:** the H1 encodes fields when created with `interlaced` (each encode is one field, `preCfg.inputHeight = height / 2`, field order from `topFieldFirst`, `h1encode.c:2238-2283`), but its stride is fixed at the width (`config.stride = handle->width`), so the woven frame must be split per field (a copy of 1.5 MB per 1080 frame) or encoded as one progressive frame (combing on motion). Halving a field to 1920x540 at the decoder's DMA, if its scaler accepts it, would give 30 progressive frames without a copy [INFERENCE]. 720p broadcasts (2.1, 5.1 here) are progressive and need none of this. 480i is also anamorphic (SAR 32:27); Plex's stock Android profile refuses anamorphic video for direct stream (`NotMatch video.anamorphic`), so SD wants square pixels (852x480) from the same DMA scaling [INFERENCE].
- **Latency knobs:** encoder: GOP 1-300 frames and IDR on demand (`LX_VENC_IOWR_ENC_PARAM`, `h1encode.c:2806-2880`), VBR/CBR/fixed QP (`ENC_SetRateCtrl`, `:2757`), no B frames, no lookahead (one blocking `ENC_ENCODE` per frame), SPS/PPS in front of every I frame (`:2386-2399`). Decoder: no low-delay control exists (controls `:1260-1300`); MPEG-2 B frames hold each anchor until the next one arrives.
- **Expected latency [INFERENCE]:** MPEG-2 reorder 2-3 frames (50-100 ms), decode 5-10 ms, NV12 DMA up to one display period (17 ms), encode 10-25 ms at 1080 and 5-12 ms at 720p (H1 1080p30-60, `venc-result.md`), TS mux and send under 10 ms: **100-170 ms added**. First H.264 keyframe: the first decodable MPEG-2 I picture leaves the decoder with the next anchor, then one encode: **about 100-200 ms after the I picture has arrived**.
- **Audio:** AC-3 is copied from the tap with its own timestamps. The decoder carries each bitstream buffer's timestamp to the picture it produces (`write_buf` and `process_picture`, `v4l2_vdec.c:901-920, 705`; `V4L2_BUF_FLAG_TIMESTAMP_COPY`, `:1850, 1866`), so video keeps the broadcast's timestamps and lip sync needs no retiming [INFERENCE].
- **Contention:** one more decoder, outside UMS's count unless the process registers and acquires `VDEC` itself (how a plain root process does that is unproven). The helper's tuner path holds no `VDEC` (`ADTU` and `SDEC` only); Live TV holds two, YouTube two, Netflix 4K all four (`second-window-4k.md`). An uncounted decoder lets UMS hand an app decoders the MCU no longer has, and that app fails. Phase 2 therefore converts only while no app has a video pipeline and stops within 1 s when one appears. `VENC` is single: LG's Miracast transmit and camera encode cannot run beside it.
- **Memory:** decoder bitstream buffer 6 MB from `cpb` (`VDEC_CPB_SIZE`, `core/vdec/hal/o22/vdec_base.h:264`; `v4l2_vdec.c:841-847`), its picture store in `vpb`, four NV12 capture buffers (4 x 3.1 MB at 1080) and the encoder's 10 MB, all from `cpb`: about 29 MB of 66 MB. What LG's own playback takes from `cpb` at the same time is not measured.
- **Wedge risk: medium, and different from `/dev/video24`'s.** That node's mmap turns the buffer's address into a kernel page with `page_address(vmalloc_to_page(...))` and maps it writable (`PAGE_SHARED`) into the caller (SRC `v4l2_venc.c:1594-1631`): the first slot's test picture went into kernel memory and the task spun in `v4l_dqbuf` → `__dma_inv_area` → `do_page_fault` with SIGKILL pending (`venc-result.md`). Route b never opens that node. `video21`'s own mmap is a size-checked `remap_pfn_range` of the HMA address (`vb2_hma.c:51-84`), and with dma-buf import no driver memory is mapped at all. `/dev/venc`'s mmap maps the ES and AUI buffers the MCU allocated (`venc.c:484-514`). What remains is a call into the MCU that never returns (decoder or encoder IPC) or a bad sequence of decoder ioctls; both stop one process, and the slot's watchdog detects a process that survives SIGKILL. Calls route b must never make: `LX_VENC_IOWR_ENC_SCALE` (a DMA between two physical addresses the caller supplies, unchecked, `venc.c:386-410`), anything on `/dev/video24`, `/dev/lg/venc0` (mode 0777, maps any physical page) or `/dev/mem`.
- **Verdict:** the only route on the set that can meet the target; unproven at every step.

### c. LG's GStreamer elements

- **What exists (BIN):** GStreamer 1.24 (32-bit) with `lxvideodec` (input `DEMUX0-2` or `MEMORY`; output `DE0/DE1` or `MEMORY`; memory format `TILED`, or `RASTER` in `thumbnail-mode`; `mp2v`; a VDEC handle from UMS through `acquire-vdec-handle` and `resource-info`), `lxtiled2linear` (de-tiles on the CPU through `/dev/mem`), `lxvideoenc` (NV12 in, copied into an HMA `photofb` frame, `KADP_VENC_ENC_Encode`; `venc-result.md`), and stock `tsdemux`, `mpegvideoparse`, `ac3parse`, `mpegtsmux`, `transcodebin`. None of LG's record, camera or Miracast binaries names the three `lx` elements (TV: grep of `/usr/sbin/advr-*`, `starfish-camera-pipeline`, `miracast`, `libadvr*`, `librecord*`, `libdvr*`).
- **MPEG-2 interlaced:** decoded; the raster thumbnail output and its interlace handling (`lxvdec_get_interlace`) are undocumented.
- **Latency:** like route b plus one or two CPU copies per frame (3.1 MB at 1080), and whatever the thumbnail mode costs [INFERENCE].
- **Contention and memory:** as route b; the VDEC comes from UMS through LG's own signal.
- **Wedge risk:** medium-high: closed elements driven outside their pipeline, and `lxtiled2linear` writes through `/dev/mem`.
- **Verdict: no-go.** Same hardware as route b with more copies, a 32-bit GStreamer process and less control.

### d. Other routes

| Route | For | Against | Verdict |
|---|---|---|---|
| d1. Sub scaler into the encoder's record mode (`V4L2_EXT_VSC_INPUT_SRC_VDEC` → `V4L2_EXT_VSC_DEST_VENC`) | all hardware: deinterlace and scale in the display engine, no buffers | the sub scaler is the Multi View and picture-in-picture window LumaTV's camera pop-ups use (`multiview-result.md`); `video31` belongs to `videooutputd`; 1080p capped at 30 frames; display-timed | no-go |
| d2. The graphic scaler's capture (`video70`, `GP_SCALER` 4) of the decoder an LG `utp` watch pipeline runs (the helper's tuner path runs none), into `/dev/venc` | no second decode, NV12 dma-buf ready for the encoder | whether a windowless pipeline's decoder can be connected to it at all; the connection belongs to LG's GAV pipeline (`videooutputd`, VDO) | park |
| d3. MPEG-2 decoded on the CPU (FFmpeg's MPEG-2 decoder added to the build the feeder already ships for H.264/H.265) into an HMA buffer, into `/dev/venc` | no `VDEC` taken from LG; only the encoder node is new | about half a core for 1080i30 on one A76 [INFERENCE], plus the field split | fallback for b |
| d4. Off the set: BeastNAS transcodes (Plex itself on the Intel GPU, or a relay) | no risk on the TV; Plex's hardware transcode starts as fast as a copy (below) | it is what happens today | baseline |

## Plex: what H.264 from the tuner would change

A throwaway relay on the host (`/data/home/tmp/tx264`, stopped afterwards, not in git) took the facade's stream, re-encoded the video with x264 the way the H1 would (High profile, I and P frames only, an IDR every second with SPS/PPS in front, 720p60 kept, 1080i and 480i made progressive with square pixels) and copied AC-3 (or made stereo AAC). Plex saw it first as a second HDHomeRun-style tuner on its own DVR with the same guide, then, for the phones, as a second device on the main DVR carrying only 2.1; both were removed and Plex's DVR configuration compared equal to the one read before the test. Plex's live flow was driven through its API with its stock client profiles (`Android.xml` for "Plex for Android", `Web.xml` for Plex Web): tune, Plex's grab (a copy into 1 s segments), a client stream from the universal transcoder (HLS, `directStream=1`, captions off), its first segment. Seconds from the tune request (MEAS, 2026-11-05, the facade at 7e8b8f43):

| Source | Runs | Tune answered | Grab's first segment | Client's first segment | Client's video |
|---|---|---|---|---|---|
| Live from the set, MPEG-2 (2.1 720p, 2.2 480i, 11.11 1080i) | 6 | 1.7-3.8 | 2.9-4.7 | 3.1-5.0 | transcoded |
| Live through the relay, H.264 (same channels) | 7 | 2.5-5.0 | 2.9-5.0 | 3.6-5.6 | transcoded |
| Recorded 2.1 at its own pace, MPEG-2 | 2 | 1.9 | 2.4-2.5 | 2.7-2.8 | transcoded |
| The same recording as H.264 | 10 | 2.1-2.4 | 2.6-2.9 | 2.8-3.3 | transcoded |

- **Plex transcoded the H.264 video too**, in every run and both profiles, whether the relay was declared a plain tuner or an HDTC transcoding tuner (Transcoder Quality "heavy"), in High or Constrained Baseline profile. The decision call answers "Direct play not available; Conversion OK"; Plex's debug log for the decision has "Streaming Resource: Required bandwidth unknown (media requires deep analysis)" and "MDE: Cannot direct stream video stream due to profile or setting limitations" (the log cuts long lines; the full text is in the server binary). A live stream has no measured bitrate.
- **Captions force it anyway:** with the account's caption choice on, the EIA-608 stream is marked `burn`, which transcodes the video whatever its codec.
- **Plex for iOS on Nitin's iPhone** (16:06-16:21): the video was transcoded in every session, MPEG-2 and H.264 alike, and AC-3 was copied. Plex's reason was the same each time: "selected subtitle cannot be converted to a compatible format, burning into video stream", then "Direct Streaming is disabled, so video stream will be transcoded": the app had captions on. From the tune request to the first media segment served (Plex's log): 3.3-4.3 s for 2.1 through the relay, 4.0-5.8 s for 2.3 and 2.4 straight from the set. Tap to picture by hand: about 5 s for 2.1 as H.264, 5-6 s for 2.3 as MPEG-2.
- **Plex for Android on Nitin's phone, quality Original, subtitles off** (16:34-16:35), with the relay's output set for a copy (High@4.1, 8 Mb/s, progressive, captions stripped, AC-3): still transcoded, video and audio, with "Direct Streaming is disabled, so video stream will be transcoded" and the same for audio, and no caption line: the app's request asked for no direct stream. Tune request to first media segment: 4.45-4.54 s for 2.1 as H.264, 4.38 s for 2.2 as MPEG-2; no difference he could see.
- **One tuner behind two devices fails:** with the relay and the set on one DVR, Plex counts two tuners. Four times (16:12:34, 16:13:10, 16:13:59, 16:35:13 on Plex's clock) it opened 2.2 on the set within 0.1 s of closing 2.1 through the relay, while the relay still held the tuner; the facade answered "all tuners in use" (TV: helper log, 16:13:58.589 and 16:35:12.512 on the set's clock, about 0.9 s behind), the relay let go 15-113 ms later, and Plex did not retry, so 2.2 did not load. Once, with 1.7 s between the two, 2.2 played. A transcode inside the facade would be one device and have no such race.
- **Audio:** AC-3 became AAC (Android) or MP3 (Web) in the API runs and was copied to iOS; the relay's stereo AAC was copied to both profiles. The video was transcoded either way.
- **The codec costs Plex no start-up time:** its own transcoder, from a recording at its own pace into 1 s segments, had the first segment after 0.63 s copying H.264 1080p and after 0.64-0.65 s transcoding MPEG-2 1080i on the Intel GPU; 720p MPEG-2 took 0.91 s, while a copy of the 720p H.264 took 1.62 s because a copy can only cut at the source's keyframes.
- **Where the time goes:** the tune (the facade answers 1.1-3.0 s after the request, with the first whole picture) and Plex's probe of the stream before its grab writes (about 0.5-1 s). The live H.264 rows also carry the relay's own start-up (facade's first byte to the relay's first IDR, 0.9-1.3 s, mostly FFmpeg's probe); an encoder on the set would add about 0.1-0.2 s instead [INFERENCE], which brings H.264 level with MPEG-2, not ahead of it.

## Recommendation

| Route | Against the target (< 300 ms added, first keyframe < 0.5 s) | Go / no-go |
|---|---|---|
| a. LG's record service | DASH output, seconds; scaler-timed; request format unknown | no-go |
| b. V4L2 decoder into `/dev/venc` | 100-170 ms added, first IDR 0.1-0.2 s after the I picture [INFERENCE] | the only candidate; no-go for Plex, probes first for the phone apps (phase 2) |
| c. LG's GStreamer elements | route b plus CPU copies | no-go |
| d1. Sub scaler into record mode | display-timed, 1080p30 cap, takes the sub window | no-go |
| d2. Graphic scaler capture of LG's decode | unknown whether it can be connected | park |
| d3. CPU decode into `/dev/venc` | about route b plus 10-20 ms per picture, half a core [INFERENCE] | fallback for b |
| d4. Transcode on BeastNAS (today) | Plex's hardware transcode adds no start-up time | keep |

For Plex: keep the broadcast as MPEG-2 and let Plex transcode on BeastNAS. Plex transcodes live video for both phones whatever the codec, so H.264 from the set would not change what they play; the start-up time worth chasing is the tune (1.1-3.0 s from request to the facade's first byte here), Plex's probe and segmenting, and the player's buffer.

## Phase 2: direct clients, the tuner's API and the first on-set run (2026-11-23)

Added evidence labels: `PUB:` SiliconDust's HTTP API page (`info.hdhomerun.com/info/http_api`, read 2026-11-23); `HOST:` measured on the build host (i9-9900K, FFmpeg 5.1.9, CPU seconds of one thread over 20 s of video, the host loaded at 35-60).

### Who gains, and by how much

The Marquee app for webOS has no Live TV and never reads the tuner. The clients that do are Marquee for iPhone and Marquee for Android (`marquee-ios/docs/LIVE-TV.md`; the Android `DirectTuner.kt` finds LumaTV by `DeviceAuth` `lumatv`). Both decode the station's MPEG-2 and AC-3 in software and give H.264 to the phone's hardware decoder. The iPhone deinterlaces on the CPU (`bwdif`, or Metal `yadif`); the Android app has no deinterlacer, so 1080i shows combing [INFERENCE]. Plex gains nothing (phase 1). VLC, Jellyfin, Emby, Channels, TiviMate and Kodi are untested.

| Station picture | MPEG-2 decode | with `bwdif` | H.264 on the phone |
|---|---|---|---|
| 480i | 0.04 core | 0.07 | hardware, about 0 [INFERENCE] |
| 720p60 (2.1) | 0.13 | not needed | same |
| 1080i (11.11, 46.1, 69.1) | 0.16 | 0.31 | same |

`HOST:` AC-3 decode adds 0.003-0.006 core to each row; `bwdif` ran in frame mode (field mode costs 0.44 at 1080i). x86 with AVX2 is a proxy: phone cores decode MPEG-2 slower per clock and `bwdif` has no NEON path, so the phone pays more [INFERENCE]. The phone's own CPU and energy were never measured (slice L0 of `LIVE-TV.md` was not run). A 1080i station is 19.4 Mb/s of MPEG-2 on the wire against about 3-4 Mb/s for `mobile` H.264 [INFERENCE], which matters at the edge of the Wi-Fi and over the tailnet. The iPhone engine's soak on BeastNAS (30 min of 2.1: 29 of 107,011 pictures dropped) shows the software path works: the gain is battery, heat and bandwidth, not a broken picture. On the TV the pixels stay in hardware except where a profile needs a scale (Geometry); the pipes and the mux are small [INFERENCE].

### The route

Route b with the hardware in a child process. Route d3 (CPU decode into the encoder) is its fallback; a relay on BeastNAS serving the same request is the alternative with no wedge risk.

- **Child.** A child process (`lumatv-xcode`, name proposed) holds the decoder and the encoder; the helper never opens `video21` or `/dev/venc`. The engine talks to it through `Codec` and `Convert` (`helper/tuner/src/transcode.rs`: pictures in, H.264 units out, over pipes). Every blocking call runs under the `venc/` watchdog (exit 75), and a child stuck in the kernel is `Fault::Wedged`. It is written after the probes pass, against a fake `video21` with dma-buf import beside `venc/tests/fake_sys.c`.
- **Never called:** `LX_VENC_IOWR_ENC_SCALE` (unchecked physical addresses, SRC: `venc.c`), `video24`, `/dev/lg/venc0`, `/dev/mem`.
- **Buffer flow:** TS, then the engine (PES gather), a pipe, a bitstream buffer, the decoder, the graphic scaler's DMA into an NV12 buffer in `cpb`, the encoder by physical address, the ES ring, a pipe, the engine (PES, TS, PCR), the client. The CPU touches pixels only where a profile needs a scale or a field select.

Decoder calls, in order (`/dev/video21`, V4L2 multi-planar):

1. `open`: a context only, no hardware (SRC: `v4l2_vdec.c` `fops_open`).
2. `QUERYCAP`, `ENUM_FMT`; `S_FMT` on the bitstream queue: `MPG2`, one access unit per buffer.
3. `S_EXT_CTRLS` `RES_INFO` (1920x1088, 30 fps); `SUBSCRIBE_EVENT` `SOURCE_CHANGE`.
4. `REQBUFS` on the bitstream queue (MMAP, 4), then `STREAMON`: this creates the decoder and takes its 6 MB bitstream buffer from `cpb` (SRC: `start_vdec`, `VDEC_CPB_SIZE`).
5. `QBUF` each access unit with its PTS as the timestamp; the picture comes back with it (SRC: `V4L2_BUF_FLAG_TIMESTAMP_COPY`).
6. On `SOURCE_CHANGE`: `G_FMT` and `G_SELECTION` for the size; NV12 buffers from `/dev/hma` (`HMA_IOCTL_ALLOC` on `cpb`, `GET_DMABUF`); `REQBUFS` on the capture queue as `DMABUF`; `QBUF` all; `STREAMON`.
7. Per picture: `poll` (1 s cap), `DQBUF`, the encoder, `QBUF` the buffer back.
8. End: `DECODER_CMD` `STOP`, `STREAMOFF` on both queues, close (SRC: `stop_vdec` frees the instance and its buffer).

Encoder calls, in order (`/dev/venc`, `venc/venc_abi.h`):

1. The KADP gate (`"SIC"`, `"KADP"`, `"VENC"`), then `ENC_CREATE` (size, whole fps, kbit/s, a GOP of one second, NV12).
2. `GET_BUFFER_INFO`; map the ES ring.
3. Per picture `ENC_ENCODE`: the luma bus address of the capture buffer and the chroma at + width x height (the H1 reads it in place); an IDR for the first picture and each GOP; the Annex B unit is read from the ring.
4. `ENC_DESTROY`, close.

### Memory and decoders beside LG's own playback

- **`cpb`** is 66 MB; `CmaFree` was 49 MB at Home. At 1080 the chain needs about 30 MB: bitstream buffers 2 MB, the decoder's bitstream 6 MB, four NV12 buffers 12.5 MB (3.1 MB each), the encoder 10 MB. At 720p about 24 MB (1.4 MB per NV12 buffer); a CPU scale adds one or two output frames. What Live TV and Marquee's playback use of `cpb` has not been read: P0 reads `/proc/hma/status` at Home, with Live TV and with Marquee playing. The chain starts only with 16 MB more free than it needs.
- **UMS counts** (MEAS, `second-window-4k.md`): totals `VDEC` 4, `VDEC_BUS` 6, `VENC` 1. Held: Live TV `VDEC` 2, `VDEC_BUS` 2, `G1_MSVC` 2, `G1_H264` 2; YouTube `VDEC` 2, `VDEC_BUS` 4; Netflix 4K `VDEC` 4. The chain would need one `VDEC`, one `G1_MSVC` and the `VENC` [INFERENCE]; P0 reads the config's totals and P3 what an acquire counts. By `VDEC` it fits beside Live TV and YouTube and never beside Netflix 4K; a Marquee 1080p H.264 stream has not been counted.
- **The kernel node does not tell UMS.** UMS preempts the oldest holder when an acquire is short (error 600), but only among holders it knows, and a plain process on `video21` is not one; how such a process registers `VDEC` and `VENC` is unproven. An app opened later could be told the decoders are free when the MCU has no instance left, and fail to start.
- **Rule for the first release:** convert only while no app has a video pipeline (`luna://com.webos.media/getActivePipelines` shows nothing but LumaTV's own; `free = TOTAL - held` as in `second-window-4k.md`), and stop within 1 s when the foreground app changes or a pipeline appears, as the facade already stops for Live TV. Otherwise the request is served as the station sends it. Beside Live TV and Marquee the chain yields instead of sharing until P0 and P3 show what fits.

### Geometry

What the engine asks for (`Profile::want`, tested on the host). The first row needs no pixel work, so it is first light:

| Station | `heavy` | `mobile` | Pixel work |
|---|---|---|---|
| 720p60 | 1280x720 at 59.94, every picture | 1280x720 at 29.97: every other decoded picture is not encoded | none |
| 1080i | 1920x1080 interlaced (H1 field mode: a CPU field split, 1.5 MB per frame) | 1280x720 at 29.97 from one field | CPU scale of a 1920x540 field; hardware scaling only if P3 shows a smaller capture is possible [INFERENCE] |
| 480i | 720x480 interlaced | 640x480 (4:3) or 848x480 (16:9), square pixels | CPU deinterlace and scale; `heavy` needs the SPS aspect ratio written, the H1 wrapper has none |
| 34.4 (H.264) | not touched | not touched | `Unsupported`: served as sent |

A shape the backend was not taught answers `Unsupported` and the request goes out as MPEG-2, so the first build can ship 720p60 to `mobile` alone. Whether phones play field-coded H.264 for `heavy` 1080i is untested [INFERENCE]. The `internet` profiles are the same path with smaller boxes and lower rates; the kbit/s each asks for (`mobile` 3500, `heavy` 9000 at 1080) are starting points to tune on the set.

### The tuner's API (HDHomeRun EXTEND)

- **Request:** `GET /auto/v<channel>?transcode=<profile>`. The names are SiliconDust's (`PUB:`): `heavy` (H.264 at the source's size, rate and interlacing), `mobile` (progressive, at most 1280x720 and 30 fps), `internet540`, `internet480`, `internet360`, `internet240` (progressive, at most 30 fps, within 960x540, 848x480, 640x360, 432x240; 4:3 stations within 720x540, 640x480, 480x360, 320x240). Case is ignored and `duration=` is ignored (the client closes the connection). Only a request that asks is converted: Plex and every other client keep MPEG-2 byte for byte.
- **Errors** (`PUB:` numbers, as the tuner already answers 805): 802 Unknown Transcode Profile (HTTP 400) for a name that is none of these, before the tuner is touched; 806 Tune Failed (HTTP 502) when the converter fails while the stream runs. 801, 805 and 807 are as before.
- **Gate:** the query is acted on only when a converter is installed in the engine (`Tuner::transcode(Some(codec))`); without one it is ignored, even a name nobody knows, so a client that always asks gets MPEG-2 from a tuner that cannot. The plugin's setting (off by default) arrives with the backend: a toggle with nothing behind it would do nothing.
- **Output:** the station's PIDs; a PAT and a PMT whose video type is 0x1B (audio entries kept; the closed-caption descriptor dropped, since captions ride in the MPEG-2 user data and are lost); H.264 with a PTS and an access unit delimiter in front; nothing goes out before the first key unit (SPS, PPS, IDR); PCR regenerated from the pictures, with a 40 ms keepalive when they are slow. `discover.json` is unchanged: a player reads the PMT, and the transcode capability is not advertised since Plex gains nothing.

### Audio

The station's AC-3 goes through unchanged on its PID, held back by the time the first picture took through the converter (measured at the first key unit), so lip sync is the broadcast's. Both phone apps decode AC-3 in software at 0.003-0.006 core (`HOST:`). AAC would need an audio encoder (UMS counts `AENC` 1) or a CPU one, and no client without AC-3 is known; it is added only if one appears.

### Latency [INFERENCE until P4]

| Stage | Added |
|---|---|
| The engine waits for the next PES to know a picture is whole | one frame, 17-33 ms |
| MPEG-2 reordering in the decoder | 50-100 ms |
| Decode | 5-10 ms |
| NV12 DMA into the capture buffer | up to 17 ms |
| Encode | 5-12 ms at 720p, 10-25 ms at 1080 |
| Mux and send | under 10 ms |
| Total | about 100-200 ms; the target is under 300 ms |

The first H.264 key unit follows the first MPEG-2 I picture by 0.1-0.2 s, so a stream starts that much later than a raw one; the tune (1.1-3.0 s) is the same. The phone's jitter buffer (about 0.33 s on the iPhone) is larger than all of it.

### Failure and fallback

Built and tested on the host against a fake converter (`transcode.rs`, `stream.rs`; the crate's 107 tests run in under 2 s). A client never loses its stream to the converter:

1. Not installed, or the station is already H.264 (34.4): MPEG-2 as the station sends it, byte for byte as without the query.
2. The converter cannot take this picture (a shape it was not taught, an app has the decoders, `cpb` short): `Fault::Unsupported`, MPEG-2, no penalty; the next request asks again.
3. It fails to open or stalls before the first unit: MPEG-2, and no request asks it for 60 s.
4. It fails or stops answering mid-stream (no unit for 2 s, or a picture that took over 1.5 s): the stream ends cleanly (HTTP 502, 806) and the client's reconnect gets MPEG-2 until the rest is over. There is no switch inside one connection: the PMT would change under a running player.
5. A hardware call that never returns: `Fault::Wedged`; no request tries a converter again until the helper restarts, and the set needs the recovery below.
6. Conversion requests get no tuning slate (slates are MPEG-2); the "tuner busy" slate stays MPEG-2.

### Wedge risks and recovery

- MPEG-2 on LG's V4L2 decoder is outside what LG validates (`media-vda-vea.rst`: H.264 only); the format table lists it and it has never run on the set.
- The decoder's write can block (SRC: `vdec_api_write_data()`, `v4l2_vdec.c:914`): the child never queues faster than pictures return, and every `poll` is capped at 1 s.
- An uncounted decoder instance (above): an app opened meanwhile can fail; the rule and the 1 s stop contain it.
- The encoder's second slot has not run, and the first ended with a process stuck in the kernel (one core spinning, the rest of the set alive; `venc-result.md`). `ENC_ENCODE` is one blocking call per picture, under the watchdog. A wedge has ended in a reboot before: the ALSA control lock of 2026-11-22 (`lgtv/research/truehd-from-marquee/WEDGE.md`).

Recovery, in order:

1. A blocked call: the watchdog ends the child (exit 75).
2. A child stuck in the kernel with the set alive: `sync; reboot` over SSH, which comes back powered on. If shutdown hangs on it: `echo 1 > /proc/sys/kernel/sysrq; echo b > /proc/sysrq-trigger` (immediate restart, no sync) [INFERENCE: not tried on this set].
3. A hung kernel, no SSH: the hardware watchdog should restart the SoC [INFERENCE: its timeout is unknown and it has never been seen to fire; `panic` is 0]. Last resort: the plug, Nitin at the set.
4. Standby or off after a restart: Wake-on-LAN. Active Standby is no recovery: the same kernel keeps running.

Each needs Nitin's OK.

### On-set probe plan (P0 to P4 and P5a run on 2026-11-23; results below)

Each slot needs Nitin's OK and the TV lock (`/data/home/Marquee/tools/tv-lock.sh`). Start: Home in front, nothing playing, `/status.json` holder `null`, the receiver muted. Every step is its own process under the `venc/` watchdog (exit 75 names the call) and a budget; `venc/tv_slot.sh` kills at the budget (SIGKILL), and a process still there 5 s later is `WEDGED` (its `/proc/PID/stack` saved) and the slot stops. Nothing is written outside `/tmp`.

Health check, before the first step and after each: `/proc/hma/status` and `CmaFree`; holders of `video21`, `venc`, `video24` and `hma`; `getActivePipelines`; `systemctl --failed`; the `dmesg` tail; LumaTV's video check (a picture after 1 s). A step passes only if the holders end empty, `cpb` equals what it was, no unit has failed, `dmesg` has no new `vdec`, `venc`, `hma`, `oom` or `watchdog` line, `getActivePipelines` is unchanged and the video check passes. The slot stops at the first step that does not.

| Step | What runs | Passes when | Also stops on |
|---|---|---|---|
| P0 read-only, 10 s each: at Home, with Live TV, with Marquee playing 1080p (Nitin starts each) | the health check; UMS's resource config; `cpb` and `CmaFree` | no holder of the nodes; the chain's need plus 16 MB free in `cpb` at Home; the counts a Live TV and a Marquee stream hold are on record | a holder of `video21` or `venc` |
| P1 `vdec-caps`, 5 s per call | open `video21`, `QUERYCAP`, `ENUM_FMT` on both queues, `ENUM_FRAMESIZES`, `G_FMT`, `QUERY_EXT_CTRL`, close; no buffers, no `STREAMON` | driver `lgsoc-vdec-decoder`; `MPG2` on the bitstream queue; `NV12` (and `LFBC`) on capture; `RES_INFO` and `MIN_BUFFERS_FOR_CAPTURE` listed | any error but `EINVAL` |
| P2 encoder, 20 s per step (`tv_slot.sh`: `caps`, `gate`, `create-320`, `map-320`, `hma-320`, `one-320`; then `create-720`, `synth-720`) | the second encoder slot: one synthetic NV12 frame, then 60 at 720p | the host decodes the frame and the 60; 5-12 ms per picture at 720p | a non-zero exit; `WEDGED`; an encode over 500 ms; `ENC_CREATE` over 2 s |
| P3 `vdec-ts`, 30 s: 10 s of 720p60 and 10 s of 1080i, each recorded from the tuner to a file on the host first, FFmpeg's decode as the reference | `STREAMON`, access units in, pictures into driver MMAP buffers and then into HMA dma-bufs; `DECODER_CMD` `STOP`, `STREAMOFF`, close | pictures at 40 dB PSNR or better against the host decode; MMAP and HMA bytes equal; the `DQBUF` interval at 720p60; the DMA time per picture; how 1080i arrives (woven or fields, field order); the `VDEC_BUS` and `G1_MSVC` the acquire holds; every buffer returned | a `poll` over 1 s with a buffer queued; an unexpected event; a second pipeline in `getActivePipelines`; `CmaFree` under 20 MB |
| P4 `chain`, 40 s: P3's files through decoder and encoder (720p30 from 720p60 first, then 1080i with a CPU scale) | `DQBUF`, `ENC_ENCODE` by address, the Annex B unit out | the output decodes; bitstream in to unit out under 100 ms at the 95th percentile; the first IDR under 200 ms after the first I picture is queued; the child under 10 % of a core at 720p; no dropped picture | an encode over 500 ms; as P3 |
| P5 `live`, 60 s cap: the engine and the child, `?transcode=mobile` on 2.1 from a host client, then from Marquee for iPhone | (a) Live TV opened mid-run; (b) a Marquee H.264 stream opened mid-run; (c) the helper killed mid-run; (d) the same with the screen off | the client plays; the PMT says 0x1B; the first key unit within 0.5 s of the I picture; (a), (b): the chain stops within 1 s and the app's picture and sound are unaffected (Nitin at the set); (c): the holders are empty | (a) or (b) not met within 1 s; any `WEDGED` |

Built for the slot and tested on the host: the fake `video21` with dma-buf import (`venc/tests/fake_sys.c`), the decoder session (`venc/vdec.c`), the probes (`lumatv-venc vdec`, `chain`) and the child (`lumatv-venc xcode`). The engine side (the queue, preemption, the tuner's converter) is not built.

### What needs Nitin

1. The OK for the slot: P0 is read-only and P1 takes no hardware; P2 to P5 take the decoder and the encoder, and a wedge ends in a reboot.
2. Slice L0 (the phone's CPU and energy on 1080i MPEG-2 against `mobile` H.264): it says what this is worth, and needs no TV hardware.
3. The phone apps asking `?transcode=mobile` (Marquee for iPhone and for Android, on LumaTV only); the tuner side is ready behind its gate, with no converter installed yet.
4. A choice: the TV's own hardware (this plan, a wedge risk, one conversion at a time, only while the TV is idle) or a relay on BeastNAS (the same request, no risk to the TV, one more hop).

### Measured on the set, 2026-11-23

Home in front and nothing playing for every step, under the TV lock. Afterwards `failed_units` (7), `aud_dsp_reset_count` (0) and `ipc_reset_count` (2) were as before, `video-check` found a picture, no process held a device, and only the last-owner label of the `cpb` pool differed. The decoder is `/dev/video21` (`vdec-decoder`), the encoder `/dev/venc`; the child is `lumatv-venc xcode` (`venc/xcode.c`, wire format `venc/xcode_proto.h`) over the session library `venc/vdec.c`.

| Step | Measured |
|---|---|
| P1 | bitstream queue H264, MPEG2, HEVC, VP9, AV1 and others; capture NV12 to 1920x1088 |
| P3 MPEG-2 | 247 pictures/s flat out at 720p60 (4 % of a core); first picture 22 ms (720p60), 71 ms (1080i and 480i), 44 ms (1080p30); progressive pictures equal FFmpeg's (Y 76 to 78 dB) |
| P3 H.264 | 1080p30, 720p60 and 1080i decode; first picture 73 to 150 ms, about 119 ms from access unit to picture at 1080 (deeper reordering) |
| P3 interlaced | MPEG-2 1080i and 480i come out as the top field with the other lines interpolated: progressive at the frame rate, half the vertical detail in motion, no combing |
| P4 child | 720p60 to 720p29.97 at 3.5 Mbit/s: 3566 kbit/s, 9.2 ms to encode, PSNR Y 44.6 dB (lowest frame 40.6 dB), 41 ms mean from access unit to unit paced; 1080i to 1080p29.97 at 6 Mbit/s: 24.7 ms a frame, 128 ms flat out; 480i to 720x480 at 1.5 Mbit/s: 1541 kbit/s, 81 ms paced. The child is 2 to 3 % of a core |
| P4 scaled | 1080i to 1280x720 at 3.5 Mbit/s with the child's CPU scaler: 3613 kbit/s; flat out 180 frames took 7.9 s with the generic horizontal pass and 5.7 s once it was specialised by tap count (31.5 frames/s, byte-identical output); paced at 29.97 the generic build ended 2 s behind real time, the specialised one was not run paced |
| P5a | five 720p MPEG-2 decoders at once work (about 10 MB each); three 1080 ones fail at `cpb` (64 MiB) |

Every output decodes in FFmpeg as H.264 High with the level patched from the encoder's 5.1 to 3.0 (480), 3.1 (720p30) or 4.0 (1080p), a key unit every GOP and the rate within 3 % of the target. The binaries run were arm64 `lumatv-venc` `e45f175d` (generic scaler) and `ea012c5c` (specialised); both were deleted from the set afterwards.

Findings the engine has to respect:

- No hardware scaler sits on the decoder or encoder path. The child scales on the CPU (`venc/scale.c`, exact-area box filter) only when the output is smaller than the picture, and only the pictures it keeps. With plain loops 1080i to 720p left about 5 % headroom over 29.97 frames/s; its passes now run on NEON (below). 720p and 480 stations need no scaling.
- The unit carries the `pts` of the access unit its picture came from. The engine sends the PES PTS (display order), not a counter.
- A stream that changes picture size (a 480i advertisement inside a 1080i programme) ends the child with exit 7; the engine starts a new one for the new format.
- The display capture tap's `REQBUFS` answered ENOENT until the capture plane was set first (below).
- Priority for the engine's queue: live viewer over explicit recording over instant replay; a preempted job is paused with a reason and resumed; the tuner's job is not resumed, the client reconnects.

### Wired into the tuner, 2026-11-23

The engine is `lumaxcode` (`helper/xcode`, no LumaTV code in it). `Engine` runs jobs (an access-unit source into a sink, one child process each), restarts a child that fails (backoff 250 ms to 8 s), rests 30 s after 4 failures in 60 s, and takes holds: a hold stops every job within the child's term grace (300 ms) and kill wait (700 ms) and refuses new ones until it is released. Its glue in the helper (`helper/app/src/xcode/`) feeds the holds from `getForegroundAppInfo` and `getActivePipelines`: the gate is open only while Home or LumaTV is in front and no other program holds a decoder. The glue also carries the child (built into the helper for aarch64, written to `/tmp/lumatv-xcode/` only while the setting is on) and refuses pictures the child would get wrong (an SD picture in 16:9, an anamorphic HD one, a station that is H.264 already). The tuner's `?transcode=` sits behind the plugin setting *Make H.264 for players that ask*, off by default.

Measured on the set under the lock with helper `09cab7f6` (main `f35c05f2`) and child `d156fafc`:

| Step | Measured |
|---|---|
| scaler | 1920x1088 to 1280x720 on the CPU alone: plain loops 15.87 ms a frame, NEON 5.50 ms (p95 6.30), the same output; the 1080 variant 16.14 and 5.26 ms |
| flat out | 178 frames of channel 11.11 (1080 MPEG-2) to 1280x720 at 3.5 Mbit/s through `lumatv-venc xcode`: 55, 118 and 172 pictures out at 1, 2 and 3 s, about 58 frames/s (17 ms a frame, 31.7 ms with plain loops); the encoder takes 9.0 ms of it |
| 3 min, 5.1 and 11.11 | 3 connections, all H.264 High 1280x720 level 3.1, 29.74 to 30.01 frames/s, 4271 kbit/s with the AC-3 sound, first picture 1.5 s (median) and 2.8 s (largest), longest gap 0.41 s, no continuity or time-stamp errors |
| 14 min, 5.1, 11.11, 30.1, 2.1 | 10.4 min at Home, 5 H.264 connections: 29.96 to 29.97 frames/s on 5.1, 11.11 and 2.1, 25.5 and 27.6 on 30.1 (a film station with pulldown flags: 62 of 361 pictures repeat a field, so its own rate [INFERENCE]), 4234 kbit/s, first picture 1.7 s (median) and 4.2 s (largest), longest gap 0.48 s, no continuity errors, no time-stamp steps back, no lost sync; helper 30.5 to 31.9 MB, steady |
| helper killed at minute 8 | the stream ended; a connection 1 s later was refused (the helper starting); the next, 2 s after the kill, was answered, tuned and had its first picture 4.2 s after the request; no child was left and the hardware was free |
| an app in front | the 2.1 stream ended on its own at minute 10.4 and the next request was served as the station sends it (MPEG-2); the helper logged `app: org.nphil.lumagame is in front` for the request at minute 12, and every request for the 4 min the run went on was MPEG-2, with the child not running. The app was not started by the soak; that it ended the stream at minute 10.4 is [INFERENCE], the log lines of that minute had rotated out |
| child cost | 1 % of a core for 720p60 unscaled, 23 % for 1080 scaled to 720p (7.8 ms of CPU a frame); `CmaFree` 146 MB at rest, 90 to 111 MB while converting, 3.5 MB at the lowest, with LumaGame in front and the child not running |

Found on the set. After the setting was switched on the page kept saying `Paused: the foreground app is not known` until a stream came, because a hold and its release did not wake the page (fixed in `f35c05f2`). ffmpeg reported one damaged slice (`error while decoding MB .. bytestream -5`) in 3 of the 4 H.264 samples of the 14 min run (11.11, 30.1, 2.1), and none in the 5.1 sample or in a 12 s capture of 5.1. It is the last picture of each sample: the player cuts a sample mid-picture and frame threads decode past `-frames:v`, so leaving out the last two pictures did not remove it. Decoded on one thread, every sample of the second run complains at its last picture only (bytestream -7 to -11), and the packets and units (unbounded PES, AUD, SPS, PPS, slices) are in order; `soak.py` now decodes on one thread. The helper's log (512 KB, filled by the compositor's trace lines in about 5 min) had rotated out the line that carries the step-aside time, so it was not captured then; the second run measured it at about 0.25 s from the launch to the job ending (`HANDOFF.md`).

Capture tap. `REQBUFS` answered ENOENT because the capture plane was not set first. With `S_EXT_CTRLS` (`V4L2_CID_EXT_CAPTURE_PLANE_PROP`) before `REQBUFS`, as LG's library calls them, `tapopen -l display` gets 1 buffer at 960x540, 4 at 1280x720 and 2 at 1920x1080 from the `de-vt-m0` pool (24 MB); `dmesg` shows `vt_set_plane_property loc[2]` then `v4l2_vt_vidioc_reqbufs`, and nothing stayed held. At 1280x720 buffers 2 and 3 were given the same address. `blended` (`loc[3]`) gets 1 buffer at 960x540 the same way; `osd` (`loc[4]`) still fails at `REQBUFS` with ENOENT (`timeout_rem[-1]`, `queue_setup ... stride:0`: the driver has no OSD surface to give). Not tried: streaming from it (`STREAMON`, `DQBUF`).

Channel change, 12 changes each over 5.1, 11.11, 30.1 and 2.1 (`venc/tools/chbench.py`, a client that parses the stream as a player does; the page's slates off for the raw ones): the first whole key frame arrived at a median 1.75 s raw (0.83-3.41) and 1.72 s with `?transcode=mobile` (1.23-3.22). The helper's phases, from the log line it writes for each stream: tune 0.4-0.9 s, tables 0.1-0.7 s, the wait for the station's next key frame 0.1-2.0 s; opening the converter takes 17-33 ms, and the first H.264 key frame follows the stream's first bytes by a median 0.25 s (0.11-0.49) where a raw stream's whole key frame follows by 0.19 s. With slates on a raw stream shows its slate at 0.08 s and the channel at the figures above.

Not tested on the set: 30 minutes in one run (10.4 and 17.7 min at Home in two), the profiles other than `mobile`, 1080 decoding while Live TV or Marquee holds decoders, the AC-3 sound in a player, Marquee's phone apps as clients, standby (Always Ready with the panel off), and frames from the capture tap.
