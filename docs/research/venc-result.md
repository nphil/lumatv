# The C4's hardware video encoder: identity, routes, prototype, first slot

## Resume here

Done (2026-10-28, offline): the encoder identified from LG's published tree and the set's own binaries; LG's three ways of driving it recovered byte for byte (the `/dev/venc` ioctl ABI, the V4L2 node, the display-feedback record mode); the video tap's capture ABI on this firmware; a prototype that captures the screen through the tap into the encoder with no CPU copy and serves the H.264 stream over TCP (`venc/`), host-tested against a fake of the three devices. First TV slot run (below): the read-only steps passed, the `/dev/venc` route failed on a call-order mistake of this prototype (buffer info asked before create; fixed, with the fake now refusing the same way), and the V4L2 M2M node wedged the kernel (LG's driver maps kernel memory into the caller; the node is never touched again). The probe now runs as small steps under a watchdog, dequeues only what it queued, checks every address against the HMA pool table, and the slot script kills over-budget steps and detects a kernel-stuck process.

Open: the second slot (`venc/tv_slot.sh`, step list below; **needs Nitin's OK: the first slot showed a kernel wedge is possible, and a wedge ends in a cold boot**); then the helper integration (start/stop behind Nitin's action, the on-screen indicator, go2rtc on BeastNAS, the Home Assistant camera), and the GeForce NOW clip ring, which this note argues should come from OpenNOW's own stream, not from the encoder.

## Evidence key

`TV:` measured on this set (OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, `chip=O22A3`), from the census captures (`docs/research/hardware-census.md`, `/tmp/hwcensus/tv`) and the copied binaries. `SRC:` LG's published O22 GPL tree (`linux-and-kdriver_paluma-5`, webOS 25: newer than this set). `BIN:` disassembly or DWARF of a binary copied off this set. `PUB:` public source with URL. `[INFERENCE]` needs the probe named next to it.

## The encoder

**A Hantro H1** (`hx280enc`): H.264 Baseline/Main/High (CAVLC and CABAC) and VP8, I and P frames only, Annex B out, 1080p at 30 or 60 fps depending on the clock (PUB: Bootlin, ELCE 2020, https://bootlin.com/pub/conferences/2020/elce/kocialkowski-supporting-hw-accelerated-video-encoding-with-mainline/kocialkowski-supporting-hw-accelerated-video-encoding-with-mainline.pdf). SRC: `kdriver/core/vdec/venc/hxenc/` (`hx280enc.c`, `h1encode_h264.c`, `h1encode_vp8.c`); on O22 the H1 sits at `0xEE440000` with IRQ 9 **on the video MCU** (`venc_cfg.c:164-167`, "enable venc in mcu from o22 chip", `vdec/Makefile:118`), the Xtensa core that also runs the decoder firmware. Linux never touches the registers; it sends IPC calls (`ipc/local/venc.c` to `ipc/remote/venc.c`). That is why the census saw no VENC interrupt line on the ARM side.

Nodes on the set (TV: `/tmp/hwcensus/tv/devices.json`, `extra.txt`):

| Node | What | Who may open |
|---|---|---|
| `/dev/venc` (char 1820:0, root:composit 0660) | the IPC front door: `LX_VENC_*` ioctls forwarded to the MCU; the first ioctl must be the KADP magic (`"SIC"`,`"KADP"`,`"VENC"`), every other answers EFAULT before it (SRC `ipc/local/venc.c:359-383`). mmap: offset 0 the AUI buffer, offset `ui32AUIBufSize` the ES ring (`:484-514`) | root, group `composit` |
| `/dev/video24` "sic-encoder" (81:21, root:composit 0660) | a V4L2 M2M multi-planar wrapper over the same IPC: out NV12M/NV21M/YUV420M/YVU420M, cap H264 and VP8, 240x120..1920x1080 step 4, bitrate 1..20,000,000 default 20 M, GOP default 60, profile 0..4 default High, level 0..13 default 4.0 (TV: first slot `caps`); MMAP buffers only, allocated by the driver from the HMA pool "photofb" (SRC `v4l2_venc.c`, `v4l2_venc.h`). **Unusable and dangerous from user space**: its mmap maps `page_address(vmalloc_to_page(<physical address>))`, i.e. whatever kernel page sits at that virtual address, so a frame written through the mapping lands in kernel memory, and the first slot's one run of it wedged the kernel (below). Never opened by LumaTV again, not even for `QUERYCAP` | root, `composit` |
| `/dev/lg/venc0` (162:0, **0777**) | the pre-MCU in-kernel driver (`venc/venc_drv.c`). Its ARM build has no hardware table for O22 (only o20/e60 under `#else // __XTENSA__`, `venc_cfg.c:180-190`), so it is a stub here [INFERENCE: `caps`-level open in a later slot]. Its mmap is `remap_pfn_range(vma->vm_pgoff)` of any page the caller names (`venc_drv.c:775-790`): **any process on this set can map any physical memory through it.** LG's bug; nothing here uses the node | anyone |
| `/dev/hma` (1819:0, 0666) | LG's contiguous-memory allocator: one allocation per fd from a named pool, physical address back, mmap, dma-buf export (SRC `hma/hma_drv.c`). LG's encoder plugin takes its frames from "photofb" (an alias of "cpb", 66 MB) | anyone |

Memory the MCU reserves for the encoder: 10 MB non-secure per instance (ES ring 0x270000 = 2.4 MB, AUI 64 KB, CPB 1 MB, H1 working memory 6.5 MB; `venc_cfg.c:133-142` "FHD"), taken from pool `venc0`, which on this model is an alias of `cpb` (66 MB at 0x77200000; TV: `/proc/hma/status` lists `cpb` and no `venc0`; SRC `platform_init.c:891`, `memalloc.c:117-134`), and 12 MB secure (`venc1`, 0x76200000; TV) for the HDCP path. The tap's buffers come from the display engine's pools `de-vt-m0` (24 MB at 0x80e28000) and `de-vt-m1` (12 MB at 0x9a800000) (TV). Memory-to-memory input frames are the caller's (HMA `photofb` = `cpb`, or the tap's buffers); nothing comes out of the UI's CMA (`CmaFree` was 16 MB at Home during the slot).

Limits that matter: 1920x1080 maximum (`VENC_MAX_WIDTH/HEIGHT`); in the display-feedback record mode the MCU encodes 1080p at 30 fps at most (`FHD_FRAMERATE_MAX 30`, every other vsync skipped, `h1encode.c:59,2175-2186`; 720p keeps 60); memory-to-memory runs as fast as frames are pushed (one blocking ioctl per frame, measured by the slot). Bit-rate control VBR/CBR/fixed QP; IDR on demand; bitrate, GOP and frame rate changeable between frames (`LX_VENC_IOWR_ENC_PARAM`). Every I frame starts with the 64-byte SPS/PPS block the MCU made at create time, zero padded (`h1encode.c:2386-2399`): the stream is self-describing at every keyframe, which is what a late-joining viewer needs. No B frames, no HEVC, no 4K.

### How LG drives it (three interfaces, recovered)

1. **Memory-to-memory through `/dev/venc`** (BIN: `libgstlxvideoenc.so`, GStreamer "lxvideoenc", NV12 in, `video/x-h264` out, DWARF intact: `LX_VENC_CTX_T { int dev_fd; LX_VENC_HANDLE_T h; }`, `VENC_ENC_CTX_T`, the `KADP_VENC_*` imports from `libkadaptor.so.2`): `KADP_VENC_Open` → `KADP_VENC_ENC_Create {w, h, framerate 30, gop 30, bitrate kbit/s, imageType 1 (NV12), codec 0 (H.264)}` → `KADP_VENC_GetBufferInfo` → `libhma_alloc("photofb", w*h*3/2, 4096)` for the input frame → `mmap(ES size, PROT_READ, MAP_SHARED, dev_fd, offset = AUI size)` for the output → per frame `memcpy` into the HMA frame, `KADP_VENC_ENC_Encode {id, bus_luma, bus_chroma_u, bus_chroma_v, p_out_buf, bus_output = ES base, out_buf_size = ES size, timeIncrement 1}` → `stream_size` bytes copied out of the mapping (`gst_lxvideoenc_chain`, addresses 0x2a9a-0x2f1e). `venc/venc_abi.h` is this ABI, sizes asserted (44/52/48/12/15 bytes, identical for a 32-bit and a 64-bit caller).
2. **V4L2 M2M on `/dev/video24`**: the standard stateful-encoder API (`S_FMT` NV12M/H264, `S_PARM`, controls, `REQBUFS` MMAP, `QBUF/DQBUF`, `poll`). LG's "Media VDA/VEA" header says it exists "for web applications to encode and decode" (SRC `v4l2-ext-broadcast-header/.../media-vda-vea.rst:59`), i.e. Chromium's video-encode accelerator for WebRTC in web apps [INFERENCE which app].
3. **Display feedback → record mode** (no capture buffer at all): the video scaler's sub window is connected `V4L2_EXT_VSC_INPUT_SRC_FB → V4L2_EXT_VSC_DEST_VENC` on `/dev/video31` (SRC `v4l2-ext-vsc.h:1591,1601`, "Miracast TX"; `v4l2_vsc_event.c:384-397,439-442`), the tap point is chosen with `V4L2_CID_EXT_CAPTURE_PLANE_PROP` and `buf_count 0` ("use display feedback for miracast-tx", `v4l2_vt.c:2270-2298`), the window size sets the encode size (`DE_IOW_SET_DVR_FMT_CTRL`, `_venc_control`), and the encoder runs in record mode: `LX_VENC_IOW_RECORD_SET_INPUT/SET_RC/SET_GOP/SET_ENCODETYPE`, `SET_COMMAND START`, then `LX_VENC_IOR_RECORD_GET_OUTPUT` blocks up to 100 ms per frame and returns ES and AUI ring offsets (`h1encode.c:970-996`; the AUI record is 16 bytes: AU type, size, start, timestamp). The sub scaler is the one `videooutputd` and `avmutemanager` hold for PIP/multiview (TV: census holders), so this route is LG's own but belongs to its resource manager.

### Who uses it on this set

| User | Path | Evidence |
|---|---|---|
| Miracast transmitter ("TV screen to a phone"; LG's service `com.webos.service.miracast` and `com.webos.service.wowplay.miracast`, setting `miracastTxPlaying`) | display feedback → record mode (3) | TV: `uiinv/ps.txt` (`/usr/sbin/miracast`), `tmp-research/census1/.../settings_all.json` (`miracastTxPlaying`); SRC strings "miracast-tx" |
| Web apps' WebRTC encode (video calls) | V4L2 M2M (2) | SRC media-vda-vea.rst; [INFERENCE: which app; the camera app is the candidate] |
| GStreamer pipelines (LG's camera/"Facebook chat" lineage: `LX_DE_DVR_FACEBOOK_CHAT`, `cam-venc` in the scaler) | `/dev/venc` (1) | BIN `libgstlxvideoenc.so`; SRC `v4l2_vsc_event.c:301,478` |
| Analog recording (ATV Rec) | record mode, fb position none | SRC; not a feature of this region's set |
| Screenshots (`com.webos.service.capture/executeOneShot`, `oledepl`'s GRAPHIC 640x360 every 3 s; the team's DISPLAY PNG bursts) | the tap and the 2D blitter, **not** the encoder | TV: `bus90.txt:200-218`, `tools/tvslot/burst.sh` |

Idle on this set: no process held `/dev/venc`, `video24`, `/dev/lg/venc0` or `video60` in the census (TV).

## What can be encoded

| Source | How frames reach the encoder | Protected video | CPU | Status |
|---|---|---|---|---|
| **The screen through the video tap** (`/dev/video60`), point `blended` (video + OSD), `osd` (graphics only), `display` (video after picture processing, no OSD), `scaler-output` (video before picture processing) | the tap scales to the asked size into its own contiguous NV12 buffers (from the display engine's own VT pool: "external buffer" when w×h×buffers ≤ 3840×2160, `v4l2_vt.c:1940-1945`; 4 buffers at 1080p fit exactly), `V4L2_CID_EXT_CAPTURE_PHYSICAL_MEMORY_INFO` gives each buffer's physical addresses, the encoder reads them in place | the kernel forces the OSD-only point at open while secure video shows and answers `DQBUF` with EACCES for a protected frame (`v4l2_vt.c:650-653, 1049-1054`); LG's library also refuses ("permission denied via SVP signal", BIN). Netflix and HDCP inputs come back black or refused, never captured | none for the capture, one ioctl per frame for the encode | **prototype built** (`venc/vtcap.c`, `venc/venc.c`) |
| LumaTV's own compositor output | render an NV12 copy on the GPU into an HMA buffer exported as a dma-buf (`HMA_IOCTL_GET_DMABUF`), hand its physical address to the encoder | the compositor never holds the video planes, so they are black by construction | one GPU pass per frame, about 1-2 ms at 1080p [INFERENCE] | next; needs the compositor's EGL import of a foreign dma-buf, which it already does for LG's buffers (`docs/research/compositor-*`) |
| Display feedback → record mode (LG's Miracast TX) | hardware, no buffers | the MCU skips frames flagged secure unless the encoder is in secure mode (`h1encode.c:2193-2196`) | none | route 3; touches the shared sub scaler |
| `/dev/video24` M2M with CPU frames | memcpy into driver buffers | n/a | ~1 ms per 1080p frame for the copy | fallback; `lumatv-venc m2m` |

The GeForce NOW frames are on the video plane through the NDL path (clear, not secure: `docs/research/opennow-result.md`), so the `blended` and `display` points see them [INFERENCE: the slot's HDMI/Netflix steps prove the two edges, a GFN session proves this one].

## Routes, ranked

| # | Route | Why | Risk | Effort |
|---|---|---|---|---|
| 1 | **Tap → `/dev/venc` memory-to-memory, zero copy** | the whole screen (or the UI alone), any size to 1080p, LG's own encoder interface (the magic gate is three strings), the tap's buffers already contiguous, no CMA from the UI budget, bitrate/GOP/IDR control, the SPS/PPS in every I frame | the encoder and the decoder share the MCU: a wedged encoder call could stall video for the whole set until a reboot [INFERENCE, never observed]; the tap is a single resource, so the ambient histogram and a capture cannot run at once; LG's screenshot `method: DISPLAY` fails while a capture holds the tap [INFERENCE] | done offline; one slot |
| 2 | Own compositor → GPU NV12 → HMA dma-buf → `/dev/venc` | works when the tap is busy (histogram) or for UI-only mirroring with no tap at all; the frames are LumaTV's own | the GPU pass costs frame time on a 60 fps UI; dma-buf import of an HMA buffer into Mali is unproven [INFERENCE: one compositor slot] | M |
| 3 | Display feedback → record mode | no buffers, hardware-timed, the exact Miracast TX path | reconfigures the sub scaler behind `videooutputd` (PIP/multiview owner); 1080p capped at 30 fps; `/dev/video31` is held by LG's services | M-L |
| 4 | `/dev/video24` M2M | standard V4L2 | **rejected after the first slot**: the driver's mmap maps kernel memory into the caller and the one run wedged the kernel (below) | never again |
| 5 | CPU encode (x264) | none: four A76 at 1.4 GHz have no budget for 1080p30 beside the UI | | rejected |

Chosen: 1, with 2 as the second source once the compositor exports a frame. There is no fallback through the V4L2 node.

## The prototype (`venc/`)

- `venc_abi.h`, `hma_abi.h`, `vt_abi.h`: the three ABIs, re-declared with the GPL header's field names and static size asserts. The tap's control base on this firmware is `V4L2_CID_USER_BASE + 0x2a00` (BIN: `VtDriver::setCapturePlanePropertes` issues 0x982a03, `setCaptureFrameRate` 0x982a08 as a plain `S_CTRL`; the webOS 25 header says +0x2100), the ext-control payload size is 16 for PLANE_PROP (BIN), the physical-address query writes through two pointers it reads as 32-bit values (SRC `v4l2_vt.c:981-1013`; `vtcap.c` keeps them below 4 GiB for a 64-bit caller).
- `venc.c`: open and the KADP gate; **create, then** the buffer map the MCU allocated for the instance (checked: non-zero, page-aligned, inside a pool of `/proc/hma/status`), the ES ring mapped; encode (physical Y/UV in, Annex B unit out of the ring); bitrate; IDR; close. The order is the MCU's: `ENC_Create` runs `H1EncodeMemoryInit`, which is where the ES and AUI buffers come from (`h1encode.c:2660, 762-840`), and `GET_BUFFER_INFO` reads the instance's fields (`:917-935`), zeros before create. `hma.c`: one contiguous buffer. `vtcap.c`: the tap in LG's own order (PLANE_PROP, S_FMT, DIVIDE, REQBUFS, PLANE_INFO, QUERYBUF, PHYSICAL_MEMORY_INFO, mmap, QBUF; BIN `VtDriver::preprocessVtCapture`), streaming started by a separate call, `poll`+`DQBUF` only while a buffer is queued, requeue only of a buffer this session holds, plane sizes and addresses checked before anything is mapped, refused frames (EACCES/ENODATA) reported, not fatal. `pools.c`: the `/proc/hma/status` table. `watchdog.c`: every call that can block (ioctls, poll, mmap, open, close) runs under an armed timer; when it fires the process names the call and exits 75. `annexb.c`: NAL walker, keyframe check. `synth.c`: the moving test picture with a frame counter.
- `probe.c` → `lumatv-venc`: small steps, one process each: `caps` (files only), `vtcaps`, `gate`, `create`, `map`, `hma`, `synth`, `tapopen`, `vt`, `serve` (TCP Annex B to one client, an IDR per connection, the `/tmp/lumatv/venc-active` file while a client is connected: the shell's indicator hook). A run stops after three encodes slower than 500 ms. One JSON line per run with frames, bytes, fps, kbit/s, CPU %, ioctl ms avg/max, the MCU's own ms, tap wait ms, refused frames and the first keyframe's SPS profile/level; `--csv` per frame. No code path opens `/dev/video24`.
- Builds (`build.sh`): the set's binary is 64-bit static musl (`build/arm64`: the kernel's own V4L2 layouts, no compat path), with a 32-bit glibc-2.28 build beside it (`build/arm`: the ABI LG's own 32-bit libraries speak). Not 32-bit musl: its 64-bit `time_t` makes `struct v4l2_buffer` 80 bytes where this 5.4 kernel knows only 68, so QUERYBUF/QBUF/DQBUF would be refused (measured: 80/68/88 bytes for musl-arm, glibc-arm, aarch64). Plus the host build against `tests/fake_sys.c`: a fake of the three devices that behaves as the MCU does (zeros from `GET_BUFFER_INFO` before create, instance id 0, the ES ring at mmap offset = AUI size, HMA physical addresses, the tap's buffers, EACCES/timeouts and a never-returning encode on demand).
- Tests (`test.sh`, all passing on the host): `tests/venc_test.c` (the call order and its refusals, GOP/IDR bookkeeping, SPS profile per create, the tap's buffers reach the encoder by physical address, dequeue refused before start and with nothing queued, requeue refused for a buffer the driver holds, protected and timeout answers, a hung encode ended by the watchdog with exit 75, fd hygiene, the pool table, Annex B edge cases, the picture's counter round trip), then the probe's steps in the slot's order, the denied gate, the hung encode, `synth`, `vt` (refusal path, width check) and `serve` (stream starts with SPS, the active file lives exactly while a client is connected). The Annex B reader was also run over an x264 stream (30 frames, 3 GOPs: 3/3/3 SPS/PPS/IDR, 27 P, 1 SEI).
- `tv_slot.sh`: the slot (below). `tools/decode_check.py`: pulls the numbers out of a slot directory, ffprobes every stream and decodes first/middle/last frames to PNG.

## Second TV slot (needs Nitin's OK first)

Preconditions: Home in front, nothing playing, nobody holding the four nodes (the script checks and stops). Everything stays under `/tmp`. Nitin's OK, because the first slot showed that a wrong call can wedge the kernel and a wedge ends in a cold boot. This slot never touches `/dev/video24`; the `/dev/venc` route runs one call further per step so a failure is one call wide.

```
scp venc/build/arm64/lumatv-venc venc/tv_slot.sh tv:/tmp/venc/
ssh tv 'sh /tmp/venc/tv_slot.sh'           # ~4 min, 15 steps, stops at the first failure
ssh tv 'sh /tmp/venc/tv_slot.sh --full'    # + 1080p, the unpaced runs, the display point (~6 min)
scp -r tv:/tmp/venc-slot/ docs/research/venc/ && python3 venc/tools/decode_check.py docs/research/venc/<dir>
```

Each step is its own process with a time budget (10-40 s). Over budget: SIGKILL, then five seconds for the process to disappear; if it is still there it is stuck in the kernel, the slot writes `WEDGED` with the task's `/proc` stack and stops. Any non-zero exit stops the slot too (the end snapshot is still taken).

| # | step | what it does | pass |
|---|---|---|---|
| 1 | `caps` | files only: nodes, `/proc/hma/status`, the pools the later steps need | rc 0, `cpb` and `de-vt-m0` present |
| 2 | `vtcaps` | opens the tap once for its capability record | rc 0, flags 0x3f, max 3840x2160 (as measured) |
| 3 | `gate` | open `/dev/venc`, KADP gate, close | rc 0; `dmesg` shows `LowPower::...::venc::...::open::ok` and `close::ok` |
| 4 | `create-320` | + `ENC_CREATE` 320x240, `GET_BUFFER_INFO`, `DESTROY` | rc 0; the line `venc: instance 0; es 0x7xxxxxxx+0x270000 (pool cpb) aui ...+0x10000 (pool cpb)` |
| 5 | `map-320` | + mmap of the ES ring, 16 bytes read | rc 0 |
| 6 | `hma-320` | one NV12 frame from `photofb`, pattern written and read back | rc 0, `counter 0x1234` |
| 7 | `one-320` | one frame encoded (`synth -n 1`) | rc 0, `first_key` sps 1 pps 1 idr 1; the `.h264` decodes on the host |
| 8 | `synth-320` | 30 frames | rc 0, 30 frames, ffprobe decodes 30 |
| 9 | `create-720` | the 720p instance | rc 0, same pools |
| 10 | `synth-720` | 60 frames at 720p30 | rc 0, `ioctl_ms_avg` is the encode cost, `hw_ms_avg` the MCU's |
| 11 | `tapopen-osd` | the tap at `osd`, 640x368, 2 buffers, no streaming | rc 0, two buffers with addresses in `de-vt-m0` (or `-m1`), stride 640 |
| 12 | `vt1-osd` | one captured frame encoded | rc 0, the PNG shows Home |
| 13 | `vt-osd-720` | 60 frames of the UI | rc 0, refused 0, wait ≈ 17 ms |
| 14 | `tapopen-blended` | the `blended` point at 720p | rc 0 |
| 15 | `vt-blended-720` | 60 frames of the screen | rc 0 |
| `--full` | `synth-1080`, `synth-720-max`, `synth-1080-max`, `vt-blended-1080`, `vt-display-720` | the ceilings | fps_got at each size |

End: no new failed unit, `holders-end.txt` empty, `hma-after` equals `hma-before`, Netflix still plays afterwards (Main's check).

Risks and rollback: a stuck step is detected and the slot stops; the only clean-up of a kernel wedge is a cold boot (Nitin). The encoder and the decoder share the MCU, so the slot runs at Home with nothing playing. HMA from `photofb` (`cpb`, 66 MB) is 115 KB to 3.1 MB per frame; ENOMEM stops cleanly. The tap is single-owner: LG's `method: DISPLAY` screenshots fail during a `vt` step [INFERENCE]. Nothing is written outside `/tmp`; no LG state is changed; no app is installed.

## Expected numbers

| Item | Expected | Basis |
|---|---|---|
| Encode time, 1080p frame | 10-25 ms, so 40-100 fps unpaced | PUB (H1 1080p30-60 by clock); the MCU caps its own record mode at 1080p30 [INFERENCE: `synth-1080p-max`] |
| Encode time, 720p frame | 5-12 ms | same |
| CPU on the set, tap → encoder, 1080p30 | under 2 % of a core: one ioctl and one 30-70 KB write per frame, no copy | design; the fill in `synth` is the only CPU work and it is not in the real path [INFERENCE: `vt-blended-1080p` cpu_pct] |
| Latency, tap to bytes | one display frame (17 ms) + the encode + the IPC; 30-50 ms | design |
| Memory | tap buffers from the display engine's VT pool (4 × 3.1 MB at 1080p), the encoder's own 10 MB, no CMA from the UI | SRC |
| Stream | H.264 High, level 5.1 default in this mode (`h1encode.c:2241`), SPS/PPS in every I frame; 4 Mbit/s at 720p30, 8 at 1080p30 (LG's defaults `H1ENCODE_BITRATE_HD/FHD`) | SRC |
| Ambient histogram and a capture | not at the same time (one tap owner); the capture client can compute the 16-zone colour statistics from its own frames (the scaled NV12 is already in CPU-visible memory) | SRC, BIN |

## Uses

**a. Live view in Home Assistant and on the phone.** `lumatv-venc serve -l blended -w 1280 -h 720 -b 4000 -p 8554` on the set; go2rtc on BeastNAS takes a raw H.264 bitstream over TCP as a source (`tcp://<tv>:8554`; PUB https://pkg.go.dev/github.com/AlexxIT/go2rtc "TCP streams with autodetection: MJPEG, H.264/H.265 bitstream, MPEG-TS") and serves WebRTC/MSE to the HA card and the phone; one encoder session fans out there. Latency in the 0.2-0.5 s class [INFERENCE]. Audio is a separate decision (the census's mixer capture, never a microphone).

**b. GeForce NOW clips, last 60 s.** Two sources compared:

| | OpenNOW's own stream (the access units the feeder already receives, `gfn/feeder/gf_proto.h` `GF_VIDEO`) | The encoder (tap `blended`) |
|---|---|---|
| Picture | the exact pixels GFN sent: up to 4K120 HDR10 HEVC (`opennow-result.md`) | 1080p30 SDR re-encode, H.264, 8 Mbit/s; the UI overlay included |
| Cost on the set | a tee of bytes already in hand, no encoder, no CPU | the tap and one ioctl per frame |
| Size of 60 s | 75-490 MB at 10-65 Mbit/s: not RAM on the set; a ring on BeastNAS over the LAN (the engine already sends every access unit over a socket to the feeder, `opennow-streamer-platform/src/feeder.rs`; a second sink is a tee) | 60 MB; RAM or the same ring |
| Clip start | needs an IDR: GFN streams refresh gradually, so "clip" asks the session for an IDR and the ring is cut there [INFERENCE on GFN's GOP structure] | every 30th frame is an IDR |

The encoder is the right source for everything that is not OpenNOW (an HDMI console, LG's apps, the UI); for OpenNOW the source stream wins on quality and cost. An HDMI console with HDCP on comes back black (the console's own setting decides; PS5 and Xbox can turn HDCP off for games).

**c. Mirroring to other Luma devices.** The same TCP stream, or go2rtc's RTSP of it, decoded on the device; one encoder session for all viewers.

**Gating (Principle 0).** Every capture is off by default and exists only as a process the helper starts after Nitin's own action on the set (a remote press on a LumaTV prompt or settings toggle; a Home Assistant button may only raise the prompt). While a client is connected the server keeps `/tmp/lumatv/venc-active` with the tap point, size and peer; the shell shows a persistent badge while that file exists and the helper kills the server when the action is withdrawn or the TV sleeps. Protected video is black by the kernel's rule, not by a check in this code.

## Needs Nitin

1. The consent flow for sharing the screen: remote press on the set per session (recommended), or a standing switch in settings.
2. Whether an HDMI console should be captured at all (HDCP off on the console is his setting; the TV never bypasses anything).
3. Where clip rings live (BeastNAS is the only place with room) and for how long.
4. Audio with the live view: the TV's own mix (census item 2), or none.
5. The `/dev/lg/venc0` 0777 raw memory mapping is an LG firmware weakness worth knowing about; LumaTV does not touch it.

## Open

- The second slot (above).
- Why `__dma_inv_area` faulted in the V4L2 node's path (the stack Main read): the driver's cache maintenance on `hma_map(<ES physical address>)` after the encode, or a consequence of the frame written through the wrong mapping. Academic: the node is not used.
- The `/dev/lg/venc0` open (read-only) in a later `caps`.
- Whether an HDCP HDMI input sets the tap's secure flag (a later slot, with the receiver muted).
- The compositor-side NV12 export (route 2) and the helper integration (start/stop API, indicator, go2rtc config).
- VP8 (`LX_VENC_ENCODE_TYPE_VP8`, exposed by the V4L2 node on this chip) is untested and unneeded.

## Changelog line (for Main)

`venc/`: the C4's hardware H.264 encoder reached from LumaTV's own process (video tap into `/dev/venc`, zero copy), with a probe in small steps under a watchdog, a TCP stream server and host tests; the V4L2 M2M node `/dev/video24` is never used (its mmap writes kernel memory; wedged the kernel once); second TV slot scripted with a kernel-wedge detector (`docs/research/venc-result.md`).

## First TV slot, run 2026-10-28 (`compositor/slot-2026-10-28-1300/venc/`)

Home in front, nothing playing, receiver muted, `venc/tv_slot.sh` and the 64-bit probe in `/tmp/venc` (no `--apps`). It stopped at step 5.

- 1. `caps`: rc 0. The `/dev/venc` nodes, the five V4L2 controls (profile 0 to 4, level 0 to 13) and `vtcaps` (flags 0x3f, up 3840x2160, down 320x240, 5 buffers) read
  as expected. `/proc/hma/status` lists `venc1` (0xc00000 at 0x76200000) beside `cpb` (0x4200000).
- 2. `synth-720p-short`: rc 2, `venc: buffer info unusable: es 0x00000000+0x0 aui 0x00000000+0x0`: the `/dev/venc` route returns no stream buffer address. No stream, so
  no later `/dev/venc` step ran. `decode_check.py`: 0 frames.
- 5. `m2m-720p` (the V4L2 fallback): the probe printed `out planes 921600+460800 bytes, es buffers 921600 bytes` and then spun at 100 % of one core for good.
  `/proc/PID/stack`: `v4l_dqbuf` -> `__dma_inv_area` -> `do_mem_abort` -> `do_page_fault` (a page fault that is retried forever inside `VIDIOC_DQBUF` on the mapped
  capture buffer). SIGTERM and SIGKILL are pending (`SigPnd`/`ShdPnd` set) and never acted on, because the task never leaves the kernel. The slot script was ended
  by hand so no step ran after it. Left as found: `lumatv-venc` pid 3262, state R, one core busy. The only clean-up is a reboot, which is Nitin's.
- The decoder is not stalled: `lumatv-vplane --once great-wave 8` played (`fed 306`) with the probe still spinning; Home stayed in front; `MemAvailable` 624 MB,
  `CmaFree` 21 MB. Steps 3, 4, 6, 7 (the paced and unpaced runs, the video tap steps) and 8 were not run.
- To do before another slot: why the capture planes' addresses are zero on the `/dev/venc` route, and the V4L2 node's DQBUF fault (the probe mapped the capture
  buffer; a fault in `__dma_inv_area` means the mapping is not valid for the CPU); the probe should poll with a timeout and `munmap` before the DQBUF, and never reach
  a DQBUF without a successful QBUF/STREAMON on the capture queue.

### Root causes and what changed (offline, after the slot)

- **The zeros.** `GET_BUFFER_INFO` reads the instance's `memESB`/`memAUI` fields (SRC `h1encode.c:917-935`), which `ENC_Create` fills through `H1EncodeMemoryInit` (`:2660`, `:762-840`, pool `venc0` = alias of `cpb` on this model, `platform_init.c:891`). The prototype asked before creating; LG's plugin asks after (`gst_lxvideoenc_chain`: `KADP_VENC_ENC_Create` at 0x2b4e, `KADP_VENC_GetBufferInfo` at 0x2ba8). Fixed in `venc.c` (create, then buffer info, then the ES mapping); the fake kernel now returns zeros before create and the tests cover the order. The instance id the MCU returns is 0 (`pstCreate->id = handle->id`, never set); `ENC_Encode` uses the fd's handle, not the id (`:2712-2726`).
- **The wedge.** The V4L2 node's mmap (SRC `v4l2_venc.c:1594-1632`) treats the buffer's physical address as a kernel virtual address: `page_address(vmalloc_to_page(phys))`, then `remap_pfn_range` of that page. On this kernel that is the page at kernel VA `0xffff0000_<phys>` (or `page_address(NULL)`), so the probe's test picture was written into kernel memory before the first `DQBUF`, and the task then spun in `v4l_dqbuf` → `__dma_inv_area` → `do_page_fault` (TV: Main's stack read) with SIGKILL pending. Nothing user space does can make that node safe; the m2m mode is gone from the probe and the slot, and no step opens the node.
- **Guards added**, so the next failure is one call wide and ends by itself: a watchdog around every blocking call (exit 75 names the call); `DQBUF` only after `poll` and only while a buffer is queued, requeue only of a held buffer, streaming started by a separate call; buffer sizes and every physical address (the MCU's ES/AUI, the tap's planes, HMA frames) checked against `/proc/hma/status` before any mmap or encode; a run stops after three encodes over 500 ms; the slot kills a step over its budget and detects a process that survives SIGKILL.

