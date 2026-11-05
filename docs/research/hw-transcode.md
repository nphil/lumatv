# Transcoding the antenna stream to H.264 on the TV's own hardware (phase 1)

Nitin asked on 2026-11-05 whether the set can turn its antenna stream (ATSC 1.0: MPEG-2 video at 480i, 720p or 1080i, AC-3 audio) into H.264 in real time on its own hardware, so Plex and phones could play it without Plex's transcoder. It is only worth doing with under ~300 ms of added latency, a first H.264 keyframe within ~0.5 s of the MPEG-2 keyframe, no effect on the TV's own viewing, and no wedge risk in the chosen path. This note is phase 1: routes, what each costs, what Plex gains, and a probe plan. Nothing in it touched the decoder or encoder nodes on the set.

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
| Who holds decoders while the tuner streams | the windowless `SCAN` pipeline the facade opens claims `ADTU`, `SDEC`, `VTP`, two `VDEC`, `G1_H264`, `G1_MSVC`; LG's Live TV claims two `VDEC` and preempts it | `live-tv-capture.md` |
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
- **Contention:** one more decoder, outside UMS's count unless the process registers and acquires `VDEC` itself. The facade's `SCAN` pipeline already holds two `VDEC`; Live TV takes two and preempts it. An uncounted third decoder lets UMS hand out a fourth and fifth to an app, which then fails. The transcoder must therefore acquire `VDEC` and `VENC` through UMS at background level and stop on its policy action, and keep the facade's rule (stop when Live TV is in front or has a pipeline). `VENC` is single: LG's Miracast transmit and camera encode cannot run beside it.
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
| d2. The graphic scaler's capture (`video70`, `GP_SCALER` 4) of the decoder LG's `SCAN` pipeline already runs, into `/dev/venc` | no second decode, NV12 dma-buf ready for the encoder | whether a windowless pipeline's decoder can be connected to it at all; the connection belongs to LG's GAV pipeline (`videooutputd`, VDO) | park |
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

## Phase 2 probe plan

Not run: with the payoff above, no probe touches the encoder or decoder nodes. The plan stands for the case where a client is shown to direct-stream live H.264 and start faster. Every step is its own process under the `venc/` probe's per-call watchdog (exit 75 names the call) and the slot script's budget (SIGKILL when over; a process still there 5 s later is stuck in the kernel: `WEDGED` with its `/proc/PID/stack`, and the slot stops). Preconditions for steps 2-8: Home in front, nothing playing, the facade idle, receiver muted, Nitin's OK for the slot. Nothing is written outside `/tmp`.

| # | Step and call | Expected | Deadline | Wedge risk |
|---|---|---|---|---|
| 0 | Read only: `/proc/hma/status`, `/proc/meminfo`, UMS `getActivePipelines`, the holders of `video21` and `/dev/venc`, once at Home and once while Nitin plays Live TV | how much of `cpb` LG's own playback uses; no holder of either node | 10 s | none |
| 1 | Host only: a fake stateful decoder with dma-buf import beside `venc/tests/fake_sys.c`, the TS demux (video PES to access units) and the TS mux (H.264 + copied AC-3, PCR from video), tested against FFmpeg's decode of the 2.1 capture | tests pass | - | none |
| 2 | `vdec-caps`: open `video21`, `QUERYCAP`, `ENUM_FMT` on both queues, `QUERY_EXT_CTRL` walk, close; no buffers | driver `lgsoc-vdec-decoder`; `MPG2` on the bitstream queue; `NV12`, `LFBC` on the capture queue; `RES_INFO` among the controls | 5 s per call, 10 s | very low (read-only ioctls) |
| 3 | The encoder's own second slot (`venc-result.md`, steps 3-10, call order fixed): KADP gate, create 1280x720, buffer info, ES map, one then 60 synthetic NV12 frames from `photofb` | ES and AUI inside `cpb`; a decodable H.264 stream; `hw_ms` per frame | 10-40 s per step; three encodes over 500 ms stop it | low-medium (MCU calls) |
| 4 | `vdec-mmap-720p`: VDEC acquired through UMS first; 120 pictures of the 2.1 capture's MPEG-2 into driver-allocated `NV12` capture buffers; one plane read back for a checksum | 120 pictures at 40 dB or better against FFmpeg's decode; `QBUF`-to-`DQBUF` time per picture; no change in other pipelines | `poll` 1 s per `DQBUF`, 30 s | medium (first use of the node by LumaTV) |
| 5 | `vdec-mmap-1080i`: the same with a 1080i recording (11.11 or 46.1) | how the woven frame arrives (field order, one buffer per frame); the conversion time per picture | as 4 | medium |
| 6 | `vdec-dmabuf`: capture buffers from HMA (`HMA_IOCTL_ALLOC` on `cpb`, `HMA_IOCTL_GET_DMABUF`), queued as `V4L2_MEMORY_DMABUF` | pictures land at the HMA addresses (read through the HMA mapping) and equal step 4's | as 4 | medium |
| 7 | `chain-720p`: step 6 into `ENC_ENCODE` by physical address, 300 pictures; then 1080i split into fields | decodable H.264; bitstream-in to ES-out under 100 ms at the 95th percentile; first IDR under 200 ms after the first I picture is queued; under 10 % of a core | as 4, plus 500 ms per encode | medium |
| 8 | `chain-live`: the facade's tap into the chain, a TS over HTTP on its own port, the test DVR pulls it; then Live TV opened mid-run | Plex numbers as above; the chain stops within 1 s of UMS's policy action or Live TV in front, Live TV's picture and sound unaffected (Nitin at the set) | stream length | medium |

Recovery, in order:

1. A call that blocks: the watchdog ends the process (exit 75).
2. A process stuck in the kernel with the rest of the set alive (the first slot's case: one core spinning, SSH and the decoder still working): `sync; reboot` over SSH, which comes back powered on. If shutdown hangs on that process: `echo 1 > /proc/sys/kernel/sysrq; echo b > /proc/sysrq-trigger` (immediate restart, no sync) [INFERENCE: not tried on this set]. Nitin's OK for either.
3. The kernel hung, no SSH: `watchdogd` stops feeding `/dev/watchdog`, and the hardware watchdog should restart the SoC [INFERENCE: its timeout is unknown and it has never been seen to fire]. `panic` is 0, so a panic restarts the set only through that watchdog. Last resort: power at the plug, Nitin at the set.
4. Standby or off after a restart: Wake-on-LAN.

Active Standby is no recovery: the same kernel keeps running. Calls no step makes: `LX_VENC_IOWR_ENC_SCALE`, anything on `video24`, `/dev/lg/venc0` or `/dev/mem`.

## Recommendation

| Route | Against the target (< 300 ms added, first keyframe < 0.5 s) | Go / no-go |
|---|---|---|
| a. LG's record service | DASH output, seconds; scaler-timed; request format unknown | no-go |
| b. V4L2 decoder into `/dev/venc` | 100-170 ms added, first IDR 0.1-0.2 s after the I picture [INFERENCE] | the only candidate; no-go while no client direct-streams live H.264 |
| c. LG's GStreamer elements | route b plus CPU copies | no-go |
| d1. Sub scaler into record mode | display-timed, 1080p30 cap, takes the sub window | no-go |
| d2. Graphic scaler capture of LG's decode | unknown whether it can be connected | park |
| d3. CPU decode into `/dev/venc` | about route b plus 10-20 ms per picture, half a core [INFERENCE] | fallback for b |
| d4. Transcode on BeastNAS (today) | Plex's hardware transcode adds no start-up time | keep |

For Plex: keep the broadcast as MPEG-2 and let Plex transcode on BeastNAS. Plex transcodes live video for both phones whatever the codec, so H.264 from the set would not change what they play; the start-up time worth chasing is the tune (1.1-3.0 s from request to the facade's first byte here), Plex's probe and segmenting, and the player's buffer.
