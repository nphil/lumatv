# Gaming firmware findings — O22 / webOS 10

2026-11-15. Target: LG C4, `chip=O22A3`, `pqe.year=24`, host kernel
`5.4.268-329.ptl4tv.2`. These findings distinguish captured machine code,
related LG OSL source and measured TV state. Static analysis is not a
successful firmware experiment or optical presentation measurement.

## Decisions

| Question | Finding | Without replacing signed VDEC firmware? |
|---|---|---|
| Q1: on-glass timestamp | No per-frame optical/scanout timestamp identified in the traced NDL/Starfish path; position plus a notification-time clock is available elsewhere | Software telemetry or external optical measurement, yes; a native on-glass callback is not established |
| Q2: 144 unique decoded frames/s | VDEC presentation has a compiled 120 limit and fixed HFR vsync selection; DE 144 timing is downstream | Not through the traced rate/PTS controls; no authenticated VDEC modification route is established |
| Q3: HDR update drops 120→60 | HDR changes can regenerate decode metadata and reconfigure the video window; the exact faulty field in the reported incident is not captured | A user-space metadata correction is a candidate, not an accepted fix |
| Q3: Dolby Vision streams | Native parser/decoder support exists; static HDR metadata alone is insufficient | A compatible native streaming pipeline is a candidate; current NDL gaming acceptance is untested |
| AV1 `vFreq=600` | Source-rate metadata is 60 fps; the AV1 display adapter omits the shared codec-rate fields and low delay seeds 60/1 | Header timing insertion alone is not an established fix; a supported host rate route is not established |
| Q4: HEVC 4:4:4 | Captured Hantro parser rejects `chroma_format_idc != 1` before core opening | Not through the traced decoder path; silicon and output-layout support remain unknown |

The O22 **DE loader is unsigned**, unlike the **secure VDEC loader**.
That distinction creates an investigation route, not permission to load
modified firmware. The [DE plan](#de-firmware-modification-route) below has
not been executed.

## Q1: presentation timestamp

The firmware's `GOOD_VIDEO` notification follows `set_new_frame` but precedes
`deif_update_display` in `vds/vdisp_core.c`. It identifies a firmware handoff,
not the time a pixel lights. DISP information carries PTS, frame identity,
rate and drop/free-run state. Host `v4l2_live.c` `display_picture_pts` and
`lxvideosink`'s current timestamp follow the source PTS; querying another
pipeline's V4L2 handle does not create a public NDL presentation callback.

`vo.c:push_info` assigns `current_stc = dis_gstc`. The name is misleading:
`get_match_result` → `vpts_match_check` → `check_pts_match` supplies a
compensated scheduled PTS; free-run paths can leave it invalid. It is not a
sampled panel clock. Decoder events, DE timing and photons remain distinct.

In `libplayerAPIs`, `MediaApiPrivate::sendCurrentTimeWithSystemClock`
(`0x52234`) samples `CLOCK_MONOTONIC` and emits event **72 / 0x48** containing
`{"currentTime":…, "systemClock":…}`. The clock is notification time in
nanoseconds. `option.useCurrentTimeWithSystemClock` enables conversion of
ordinary position events in `callbackHook` (`0x5d9d4`); it does not install a
per-frame presentation hook.

For the normal `libpf` `Pipeline`, `updatePlayPosition` (`0x7b3ec`) queries
GStreamer time in nanoseconds, divides by 1,000,000 and notifies a changed
position in **milliseconds**. `informationMonitorStart` (`0x81a0c`) is a
periodic timer; Starfish's default/minimum update interval is 50 ms.
Two limits prevent assuming this works unchanged for gaming:

- NDL `DMPlayer::smpCallback` (`0x1dc74`) does not forward event 72.
- `DirectMediaPlayer::position` (`0x146f78`) returns raw GStreamer
  nanoseconds on demand, or -1 when unavailable. Its timer
  `monitoringCallback` (`0x14a100`) calls `reportDroppedFrame` (`0x149ea0`,
  event 0x30), not `notifyPositionUpdate`; its inspected `handleMessage`
  (`0x14c684`) has no periodic position-event producer.

**Integration decision:** label any future PTS/host-clock pair as software
telemetry, including its units and sampling point. Do not turn event 72,
queue consumption, frame-drop counts or source PTS into an on-glass latency
claim. A tagged frame and photodiode/high-speed camera remain the reference
for button-to-light latency. No new presentation API was exercised on TV.

## Q2: the 120 presentation ceiling

The OSL source identifies the gate, and the captured `vdec.bin` contains it:

- `vds/vdisp_core.c:55,259–278`: `MAX_FRAMERATE=120`; the progressive
  branch at `0x505c6958–0x505c697a` includes **`movi a11,120` at
  `0x505c6971`**, then calls `vrate_set_rate_limit` (`0x505c8f88`).
  Interlaced cases select lower limits.
- `vds/vrate.c:40,565–576`: `FRAMERATE_MAX=120`; `update_vrate` clamps
  rates above the maximum to maximum/1. Duration calculations also clamp.
- `vds/vdisp_core.c:connect_vsync` (`1089–1102`) chooses **120/1 or 100/1**
  for HFR and enables fixed-rate vsync. The captured HFR connection log
  reference is `0x505ac9bd`.

This explains why downstream DE timing support does not remove VDEC's frame
selection ceiling. Existing measurements report fixed 120/1 decoder vsync,
225,000 ticks at 27 MHz, and approximately 25 drops/s with a 145 fps feed.
The earlier DE selector experiment proves a DE timing branch, not 144
unique decoded pictures reaching the panel.

HFR enablement is a separate decision: `driver.c:init_output` selects it
from explicit flags or decoded width × height > 2,764,800 and codec fps ≥61.
Low-delay mode separately seeds configured rate 60/1. Neither this decision
nor a PTS/VUI/forced-rate value above 120 rewrites the later constant.

The VDS scheduler runs on the Xtensa MCU. Host `/proc/lxvo/vdisp/*` is a
read-only relay, not the MCU's writable debug interface. MCU rate setters
still call the clamp. Host `vdec_bootopts` is consumed on a later firmware
run/recovery; changing arguments and restarting the MCU cannot alter the
constant. The old `0x505a6b0c` 120000 data literal is not this code gate.

**Decision:** retain 120 as the decoded-stream ceiling of the traced path.
A VDEC change would require the secure-load boundary below to be satisfied;
no bypass or accepted replacement exists. PTS rewriting and DE 144 controls
must not be advertised as a solution.

## Q3: HDR updates and Dolby Vision streams

### Mid-stream HDR update and the 60 Hz drop

The reported drop is an observed regression, but no before/after metadata
capture identifies its exact triggering field. The traced chain is concrete:

1. `StarfishMediaAPIs::setHdrInfo` (`0x5ce78`) parses type/SEI/VUI and
   disables automatic HDR-event handling for that instance. It does **not**
   directly assign a 60 fps rate.
2. `updateHdrInfo` (`0x5cba8`) compares HDR state; when changed and cached
   video information exists, it regenerates decode information and calls
   `AVProxy::setDecodeInfo` immediately.
3. `generateVideoDecodeInfo` (`0x54470`) uses cached rate at object `+0x60`.
   It recomputes HFR from rate >60 and pixel count >`0x1fe000`, unless
   adaptive track information overrides HFR. These are not the separate
   VDEC HFR thresholds described in Q2.
4. `videooutputd` `VideoScalerService::updateVssData` (`0x84148`) copies
   the reported rate and HDR state. An absent `hfrEnabled` clears HFR;
   changed rate/HFR/HDR state can reach `checkVideoDataSetWindow`
   (`0x82a0c`) and reconfigure the window.

`ResourceManager::preprocessInvalidVideoInfo` (`0x99fbc`) also substitutes
platform maxima for incomplete width/height/rate during resource
acquisition. Its 60/120 fallbacks are not evidence of an unconditional
mid-stream HDR=60 rule. DirectMediaPlayer's video-info message parser also
initialises missing rate to zero before generating its video-info event;
partial metadata and metadata ordering require attention, not a hardcoded
120 replacement.

**Candidate correction, not yet accepted:** establish HDR metadata before
normal streaming where the pipeline permits it, avoid redundant setters,
and preserve actual negotiated rate, HFR, game and low-delay information
across genuine HDR transitions. Equality checks already suppress some
unchanged HDR updates. Do not discard real metadata changes or force 120
for all content. Acceptance requires the actual before/after `decodeInfo`
and video-output metadata, VDEC `vrate`/vsync, DE timing and optical result.
No mid-stream playback experiment or fix is claimed here.

### Dolby Vision for streams

The installed native stack is Dolby Vision aware:

- `libgstlxvideodec` `gst_lx_videodec_handle_caps` (`0x1d3e0`) consumes
  `dolby-vision` and `dolby-vision-profile`, configures `lxvdec_set_dolby_vision`
  and propagates Dolby fields; base/enhancement-layer handling exists.
  The captured media inventory includes the RPU parser and `librpu_m.so`.
- Starfish parses `option.externalStreamingInfo.contents.DolbyHdrInfo`
  and `platformSupportDolbyVision`. Normal `Pipeline`'s
  `handleDolbyVisionInfoMessage` (`0x8fbd8`) maps profile, level and CCID
  to `dolby_vision_profile`, `dolby_vision_level`, `dolby_vision_ccid`
  and emits event 0x52.
- `DirectMediaPlayer::createVideoPath` (`0x148738`) builds codec caps and
  an appsrc → decoder → sink path, without those Dolby input fields or
  the normal parser chain. It can recognise Dolby output metadata; that
  does not prove that arbitrary raw NDL ES input establishes the full path.

`SetHDRInfo` supplies static display metadata, not the **RPU**, Dolby
Vision's per-picture processing metadata carried with encoded video.
Changing an HDR enum cannot manufacture Dolby Vision from HDR10 or SDR.
The source must actually encode a compatible Dolby profile and retain its
metadata through parsing/feeding. An HDR10-compatible profile 8.1 base layer
is a useful fallback candidate; neither profile 5 nor every profile 8 stream
can simply be treated as HDR10.

**Decision:** investigate a complete native streaming/parser path with a
known compatible Dolby stream and explicit fallback, not a firmware patch
or an HDR flag toggle. NDL Dolby Vision, profile coverage, 120 fps and
low-latency gaming behaviour remain untested. Existing Dolby stack support
is not a promise that the game-stream source supplies Dolby Vision.

## AV1 source rate: why `vFreq=600` is not a panel measurement

`vFreq` is **source fps ×10**. A value of 600 can coexist with a 120 Hz DE
output. Neither that field nor successful submission of 120 frames/s
establishes 120 unique visible frames.

The captured AV1 firmware does parse sequence timing:
`Av1DecodeObuHeaders` (`0xc380c`) reads the two 32-bit timing values,
`equal_picture_interval` and the variable-length interval count. The gap is
in exported frame information, not the absence of an AV1 timing parser.

`gx_av1_get_disp_info` (`0x5981c`) does **not** fill shared display-report
words 10/11 (`+0x28/+0x2c`). In the H.264 counterpart (`0x53844`), those
words are filled from picture words `0x42/0x43`. OSL
`control_dec_frame_info` places rate after the three-word size and
six-word plane-address structures; the preceding report index accounts for
the extra word. `_gx_get_pic_display` (`0x4de64`) copies cached frame
information around the codec call rather than deriving an AV1 rate.
`control/callback.c:___fill_framerate` accepts `fi.rate.res/div` only when
valid. The AV1 adapter's `+0x64/+0x68` pair is metadata address/size,
**not** frame-rate numerator/denominator.

The low-delay output seed is 60/1 (`driver.c:1759–1762`). Rate-source
selection can retain valid CONF when CODEC rate is absent instead of
replacing it with the PTS estimator. The host codec policy separately
lists AV1 at 60 fps and H.265 at 120; those declarations do not establish a
physical AV1 silicon limit.

**Decision:** sequence-header timing insertion alone is not an established
cure for this adapter omission. Correct propagation through this firmware
would cross VDEC's secure-load boundary; no supported equivalent host
control is established. Acceptance of any alternative needs codec rate,
selected rate, drop/unique-frame counts and DE/optical timing kept separate.
No AV1 120 guarantee follows from this analysis.

## Q4: HEVC 4:4:4

`HEVCDecodeSeqParamSet` (`0x1ff44`) in the captured `vpu.ko` stores
`chroma_format_idc` and returns **error 3 unless it equals 1 (4:2:0)**,
before `DecodeSPSBasicInfo` and decode-core opening. This is an identified
software refusal, consistent with the existing 4:4:4 black-picture/no-core
receipt and its working 4:2:0 control.

`DWLGetCoreCapability` (`0x5dc1c`) calls `HantroDecGetCoreCapability`
(`0x62910`); Hantro/VeriSilicon identity is established, not an exact
VC8000D SKU. IRQ labels containing CNM are not proof of Chips&Media IP.
The OSL host-facing output also announces `VDISP_FRAME_FORMAT_420`
(`v4l2_vdec.c`). Removing the parser refusal would not establish compatible
allocation, plane packing, output transport or silicon capability.

**Decision:** keep 10-bit 4:2:0 HDR as the working baseline. The signed VDEC
image cannot be replaced by an ordinary edited file; a parser-only patch
would be insufficient even if authenticated loading were available. HDMI
4:4:4 capability is a separate display path, not decoder proof. Existing
live receipts: `/data/home/tmp/lumagame-display-20261115/{e444-*,c420-*}`.

## Installed loader trust boundaries

### O22 DE: raw load, not signature authentication

The installed kernel's `DE_IO_FW_Download` (`0xffffffc01092e694`) calls
`vsc_fw_io_download` (`0xffffffc01094fec4`). O22 `vsc_fw_init`
(`0xffffffc01094f1f0`) installs no secure-load handler;
`vsc_fw_hal_load` (`0xffffffc01094f710`) reaches `DE_HAL_FW_Download`.
`DE_REG_O22_FW_Download` (`0xffffffc010969af4`) copies user/kernel/file
bytes into MCU DDR and controls boot mapping and run-stall. In the published
O22 MCU0 sequence the explicit soft-reset writes are commented out;
releasing stall is not proof of reset (`de_reg_o22.c:2741–2751,2805–2821`).
`DE_REG_O22_FW_Verify` (`0xffffffc01096a6b0`) compares DDR bytes with the
source; **it is not a cryptographic signature check**.

Live read-only `fw-status` agrees: `fwload handler:0000000000000000`.
This establishes the installed loader's boundary, not successful execution
of a modified image or safe hot replacement.

In LG's published `core/de/vsc_fw.c:524–546,568–576`, the 16 MiB
front-end check can select the embedded header on invalid buffer input;
valid user data can be staged in kernel memory. Neither is the size of
the TV's MCU0 region (4 MiB) or an assurance that an image will run.
`vsc_fw.c:1217–1254` retries image 0 from retained data or header after
100 unsuccessful polls and times out after 200. These are counts, not
wall-clock guarantees. Neither IPC readiness nor the [intrusive byte
comparison](decoded-vrr-attended.md#what-would-reopen-b) establishes playback recovery.

### VDEC: secure firmware load

`vdec_copy_to_ddr` (`0xffffffc010c8b8a0`) sets request **0x11** at
`0xffffffc010c8b938` and calls `lgkree_req_secure_fwload`
(`0xffffffc010c204b4`). The bridge forms **SMC 0x8200ff09** at
`0xffffffc010c20570–0xffffffc010c20578` and calls `ree_smccc_smc` at
`0xffffffc010c20594`. Failure returns `-EIO`; the retail path has no plain
copy fallback. Filename override, user-buffer LOAD2 and RUN converge on
this boundary.

The observed nonsecure exception checks for FPGA chip-ID high nibble 0xf,
not retail O22A3 or `debugMode=5`. O24 DE `dispopts=debugmap` source is not
an O22 VDEC bypass. Secure-monitor internals, keys and exact authentication
policy were not recovered; no claim of a universal impossibility proof or
usable bypass follows. Ordinary edited VDEC loading is not an established
route.

## DE firmware modification route

**PLAN ONLY — requires Nitin's later approval. No firmware was loaded,
patched or written to the TV.** Unsigned loading can still hang the display
engine, blank the screen or require physical recovery.

### Image origin and boot stage

The normal loader first tries **`/var/firmware/pqefw%d.bin`** for each
image index. With no override it uses images compiled into the installed
kernel **`/boot/Image.lz4-5.4.268-329.ptl4tv.2`**. This is not a requirement
to create a standalone DE file on the TV.

`v4l2_vsc_probe` (`0xffffffc010940494`) calls
`vsc_fw_download(VSC_FW_FROM_NORMAL)` at `0xffffffc010940530`: a kernel
VSC platform-device probe, before ordinary user-space playback. The loader
processes image indices in descending order, then enables IPC. Suspend,
hibernation and recovery paths can retain/reload firmware state; standby
must not be treated as a guaranteed clean rollback.

The screen-off, leased read-only capture recorded:

- `/var/firmware`: **ENOENT**, so no `pqefw*.bin` override exists there.
- `/proc/lg/de/fw-status`: `fw_opt:none`, `usr_opt:none`, `usr_num:0`,
  `max_num:3`, `year:24`, no current/new paths or allocated override data,
  and a null firmware-load handler. The command line has `pqe.year=24`
  and no `dispopts` override.
- The installed kernel's `DE_FW_O22_DownloadFromHead`
  (`0xffffffc01096e060`) selects year-24 image 0 from descriptor
  `0xffffffc011b26fb0`. Its embedded identity is
  **`y24_pine25_o22n2_20250414.1`**, not `paluma_o22n3`.

**Active load provenance:** the measured configuration selects the
kernel-embedded year-24 O22n2 family, with indices 2, 1, 0 in the normal
load. This is an inference from the installed loader plus live status,
**not a byte-for-byte readback of currently executing MCU RAM**. No safe
read-only MCU-image export was identified in the inspected DE proc
interface; `bin_download` and `fw_dl_history` are write controls, not such
an export. Active RAM identity must be resolved before any modified load;
older firmware addresses must not be transplanted by name alone.

The captured `Image.lz4-5.4.268-329.ptl4tv.2` (SHA-256 `3e13cfe557ad8037972c77f03bd4d8d226e2f58eee0127fd38a98ae23fa52ca8`) uses LG's `LZ4P` wrapper, not a standard LZ4 frame. A 32-byte header and 141 little-endian 32-bit compressed lengths precede the block data at `0x254`. The 256 KiB raw-block decode gives 36,837,888 bytes, SHA-256 `0fffb8b098be13810c188f2244458ff81136dbadcb1a97a97d90bf784d06dfe3`. The offsets below are in that **decompressed** image; the kernel addresses equal offset + `0xffffffc010088000`.

| Index | Bytes | Raw offset | Kernel data VA | SHA-256 of payload |
|---|---:|---|---|---|
| 0 | 1,821,632 | `0x1f71028` | `0xffffffc011ff9028` | `bc5aa595f350a0dc89f6f314c8cfe6977863ab021f7cda133bdf0795bbefd6de` |
| 1 | 569,616 | `0x1ee5f18` | `0xffffffc011f6df18` | `025b2cfb1721ace729da259ccbdc205fb6741f8c9ae5485120152728d08af306` |
| 2 | 4,466,688 | `0x1aa3718` | `0xffffffc011b2b718` | `66f001b5a31c3afe18a60cce1de57d0d448ead8c6932ef0f08f8383e202044d7` |

**Historical TV observations (Firmware Study, 2026-11-17).** Status reported build 7356 dated 2025-04-14; image 0 contains the matching date string at payload offset `0x1b8971`. Two historical embedded-load sequences were reported, with order 2 → 1 → 0 and physical destinations `0x97200000`, `0x96e00000`, `0x7ea80000` respectively. In the `0xcc080000` register bank, control read `0x00040004` and the two boot-map words for each core agreed: MCU0 `0x7ea80000`, MCU1 `0x96a00000` (4 MiB below its destination). Sampled MCU0 RAM matched entry bytes and diagnostic text, **not a whole-image hash**. Physical boot maps are not MCU virtual addresses. The O22 loader has no separate core start for index 2; its exact role remains unknown.

The historical inspection included diagnostic command and logging-control writes. Logging controls were returned to presumed defaults, not verified pre-inspection values. That study performed no firmware reload, patch, forced VRR, HDMI on/off comparison, power-cycle recovery test or OLED-protection change. The observed `snapshot`/`resume` command line and two earlier loads do not assign those loads to specific boot branches. Loss of power clears volatile state; it does not prove what a subsequent snapshot, retained backup or embedded-load branch restores (`vsc_fw.c:447–488`). [Recovery remains unqualified](decoded-vrr-attended.md).

### What a modified DE image might unlock

- **NDL/OSD VRR tracking:** a narrowly scoped DE change could admit an
  internal-source tracking mode currently tied to other source/state
  conditions. It still needs a real frame-arrival signal and safe panel
  blanking bounds. Fixed 120 VDEC delivery is not automatically variable
  NDL presentation; OSD also needs its own presentation/DRM integration.
- **144 Hz DE timing:** a matched image could change internal-source
  selection or timing policy while keeping panel limits intact. This
  cannot remove the upstream signed VDEC 120 selection ceiling or prove
  144 unique decoded frames. Existing selector evidence must first be
  matched to this exact image revision.

Neither outcome is established. Panel/TCON firmware, OLED compensation,
brightness protection and panel-care behaviour are outside this plan.

### Brick-risk-minimising test plan

1. **Verified original backup and identity.** Before approval to load,
   preserve the original compressed kernel and every selected DE payload
   in durable host storage, with independent copies, hashes, lengths,
   target revision and extraction offsets. Establish active RAM identity
   through a demonstrated safe readback, or an approved controlled stock
   boot with sufficient load evidence; do not assume a historical dump is
   current. Verify the recovery copy against those originals. No candidate
   may change memory layout, image length or load addresses without a
   separate review.
2. **Establish recovery before experimentation.** Require a person at the
   TV, independent host/network access and a known full stock cold-boot
   route; confirm it does not simply restore a hibernation snapshot.
   Document a physical/bootloader/service recovery option if the host also
   hangs. Those recovery guarantees are not established by this research:
   if a black screen cannot be recovered without the modified DE, stop
   before loading anything. A remote SSH session alone is not recovery.
3. **Qualify a RAM-only route, if safe.** Audit the exact O22 user-buffer
   `DE_IOW_FW_DOWNLOAD` path, ABI, size bounds, MCU stall/reset sequence,
   retained backup state and IPC reinitialisation. It copies into DDR and
   appears suitable for a nonpersistent experiment, but safe hot loading
   is unverified. The handler can stall before an error and has no proven
   automatic rollback. First test an unchanged original only after
   approval. If a strictly RAM-only, recoverable load cannot be demonstrated,
   do not substitute a persistent route.
4. **One bounded candidate at a time.** After the unchanged-image baseline
   and restoration succeed, review an exact host-side diff against matching
   bytes and exercise the changed branch offline. Keep the original image
   resident on the recovery host. Load at most one narrowly scoped change,
   with a short watchdog deadline and optical/counter observations. Restore
   the original immediately after the observation, then require a clean
   stock restart and normal panel behaviour before any further candidate.
5. **Black-screen response and abort criteria.** Stop the experiment on a
   hash/length/revision mismatch, load/IPC timeout, watchdog reset, lost
   host access, black/flickering picture, timing outside verified bounds,
   or any changed panel-protection behaviour. If the host remains healthy,
   use only the prequalified original-image restoration or stock reboot;
   otherwise use the prequalified physical recovery path. No speculative
   register writes, repeated blind loads or unbounded reboot loops. A
   failure to restore is a hard stop for owner/service recovery.

No step may write `/var/firmware`, replace the boot kernel, alter boot
arguments, flash a partition or change the secure monitor. RAM-only means
no persistence mechanism anywhere in the load/recovery chain. A standard
reboot is an intended recovery route, not a guarantee against a hard hang
or damage; this plan reduces risk rather than eliminating it.

## Evidence identity and limits

Captured inputs are under `/data/home/tmp/lumagame-rea-20261115/tv/`.
Continuation receipts are under
`/tmp/cs/3dc58534e64e/rea-firmware/`: `kernel-loaders.asm`,
`kernel-de-origin.asm`, `tv-de-origin/receipt.json`, and the named function
decompilations `playerclock.c`, `pfposition.c`, `directmedia.c`,
`directmedia-fields.txt`, `hdrresource.c`, `videooutput.c`, `dolby.c`,
`vpu.c`, `av1.c`, `picturerate.c`, `gxdisplay.c`. DE selector and prior
playback evidence remain in [the gaming contract](../lumagame-gaming-features.md)
and [pipeline research](vwin-refresh-result.md).

| Input | SHA-256 |
|---|---|
| `vdec.bin` | `30a6eb9c343ced9ff2620f6fadff925284f8cb319e41b13430a4785675aebd2d` |
| extracted `vpu.ko` | `378ecd6534518182dfac666ed010ffed247e90c0554aaef97d2ee2e3bf653881` |
| `libNDL_directmedia_impl.so.1.0.0` | `674d64ace4b4eb4caa11e6df7e3f93155184fbbbe8de5189ccbff65992d630e0` |
| `libplayerAPIs.so.1.0.0` | `87da8403b4ceb3df993ecfe4ef0fd1fd44f9391839e44c165076a1506e6a26b7` |
| `libpf-1.0.so.1.0.0` | `e8a966b26476f647bfcebf02b08404fc52f0198c52195e37354baa3d712fc36f` |
| `libgstlxvideodec.so` | `6a10f85d6eccfb752aaedfa751cc571b84cd52adb364d534ba4b288ac3846c9e` |
| `libgstlxvideosink.so` | `57c5a4946ae04544de548e6d73af1a7d8f6229e5f3121d284e9a9e9a8c60d711` |
| `videooutputd` | `86ff460d7eb5daeae83458d309a6d6bc939c8c71d93d89cf64f96c8d5e5cefdb` |
| `/boot/Image.lz4-5.4.268-329.ptl4tv.2` | `3e13cfe557ad8037972c77f03bd4d8d226e2f58eee0127fd38a98ae23fa52ca8` |
| decompressed `Image` | `0fffb8b098be13810c188f2244458ff81136dbadcb1a97a97d90bf784d06dfe3` |

ARM shared-library and relocatable Xtensa-module addresses above are Ghidra
import coordinates, not process ASLR addresses or raw module offsets.
VDEC image addresses use payload offset `0x13c`, base `0x50000000`; kernel
addresses use the captured symbol map and image base `0xffffffc010088000`.
LG OSL source supplies names/layouts, not automatic proof that every source
branch matches the installed binary. Xtensa analysis used the installed
Ghidra backend because the available REA importer does not admit this target.

The final provenance pass used a lease, SFTP directory/status reads and host
hashing. Power was **Active Standby before and after**, and the lease was
released. No file-list shell expansion, playback, display-register access,
firmware reload or firmware modification occurred in that pass. The exact
running MCU RAM bytes, HDR-drop cause, Dolby stream acceptance, AV1 unique
120 presentation and physical 144 Hz/VRR behaviour remain untested or
unresolved as explicitly stated above.
