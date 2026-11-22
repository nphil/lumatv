# Gaming display features — LumaGame contract

Status levels: **measured** (observed on the TV with receipt), **proven
offline** (executed on captured firmware, not on the TV), **inferred**
(derived from disassembly or architecture, not executed), **untested**.

Research artifacts: `/data/home/tmp/lumatv-gaming-fw-20261101/`.

## Resume here: near-native streaming performance

Execution order, agreed 2026-11-09: lowest practical button-to-visible-response
latency at high image quality takes priority over additional gaming features.
The first target is stable 4K120 HEVC Main10 HDR10, not an unmeasured 144 Hz
mode. This is the shared handoff for the LumaGame app session; implementation
and fresh TV measurements have not started under this plan. Existing app-work
holds and TV-access requirements remain in force.

LG libraries are reverse-engineering, adaptation or replacement targets, not
product limits. Separate API restrictions, firmware policy and measured
hardware limits. Bypassing NDL is not itself an improvement: retain whichever
path produces the best measured response, image quality and stability.

### Established baseline and measurement gaps

- **Controller:** GameSir Cyclone v2, its USB wireless dongle plugged directly
  into the TV (owner-confirmed 2026-11-09). The earlier
  [receiver capture](research/gfn-input-result.md#the-receiver) identified a
  GameSir receiver presenting as a DualShock 4 v2: a 1 ms USB polling interval
  and 500 HID reports/s over two seconds. This is not a genuine Sony device
  identity or a measured 2 ms physical-button delay. Confirm the current
  dongle mode and cadence; do not substitute a Bluetooth or cabled baseline.
- **Throughput, not latency:** the
  [4K120 HDR receipt](research/gfn/tv-2026-10-26/log.txt) at lines 413–422
  records a local HEVC Main10 80 Mb/s clip looped for 300 s, 36,001 feeder
  accepts, zero feeder drops and zero retries. The
  [display-counter receipt](research/gfn/tv-2026-10-26/g1-4k120-80m-300s.measure.txt)
  records approximately 120 Hz. Neither establishes unique presented frames,
  real GFN/network performance or button-to-light latency.
- **Input already bypasses the UI frame loop:**
  [reader_loop](../gfn/engine/crates/lumatv-gfn-pad/src/lib.rs) blocks on device
  events and forwards changed state at each SYN_REPORT. The pinned OpenNOW
  engine drains input with a 250 µs event wait, not a guaranteed 250 µs maximum
  delay. Its input queue can hold 256 samples; measure age and overflow under
  load before changing it.
- **Potential video waiting, not measured constant lag:** OpenNOW's media
  channel holds 32 items (8 at the pinned revision; the engine carries
  upstream #1451), which may include audio; the NVST 52 ms reorder
  timeout applies only while a packet gap is open. The
  [feeder](../gfn/feeder/src/feeder.c) retries a full video feed every 0.5 ms
  for up to 8 ms by default. The clean clip above recorded no retries.
  Queue capacity and timeout values must not be added together as normal lag.
- **Current timing labels are not physical measurements:**
  [runtime timings](../gfn/engine/crates/opennow-streamer-platform/src/runtime.rs)
  reports feed-call time and queue-depth × frame-period estimates as decode
  timing, maps p99 into p95 fields and a queue average into a p50 field, and
  counts accepted submissions as outputs. The
  [input send tap](../gfn/engine/crates/lumatv-gfn-sendtap/src/lib.rs) matches
  the first send after handoff, which may be a different packet. The example
  0.4 ms / p99 1.3 ms in the older input note is not a TV result.

Transport observations refer to OpenNOW revision
`a49b124ce31e87113fd4961f6a321fbf0ad1fdc5`, pinned in
[the engine manifest](../gfn/engine/Cargo.toml), whose core and transport the
engine builds from vendored copies (changes in each crate's `VENDORED.md`):
[core's `lib.rs`](../gfn/engine/crates/opennow-streamer-core/src/lib.rs) for
input draining/media capacity, and
[transport's `nvst.rs`](../gfn/engine/crates/opennow-streamer-transport/src/nvst.rs)
for `MJOLNIR_REORDER_DEQUEUE_TIMEOUT`. Locate the app's actual consumers and
dependency revision before editing; do not assume a separate app has already
adopted every shared-source change or that its local-stream adapter is ready.

### Ordered execution and acceptance

1. **Make timing trustworthy, then establish the baseline.** Reuse the existing
   native reader, latency recorder and feeder counters. Separate USB report
   arrival, changed input events, app queue age, matched input-packet send,
   frame arrival, decoder submission, decoder completion where observable,
   and actual presentation. Correct percentile/estimate labels; unavailable
   decoder or display timestamps remain unavailable, not zero or inferred
   completion. Use a correlated input packet rather than the first arbitrary
   send. Host and TV timestamps cannot be subtracted without qualified clock
   alignment. In a coordinated TV window, measure the current dongle under
   idle and streaming load and physical button-to-light response with a
   timestamped button stimulus plus high-speed recording or a photodiode.
   Compare local streaming with direct HDMI from the same PC when available;
   compare the GFN app with the official LG GFN client at matched settings.
   Record any missing HDMI source, local adapter, account or physical timing
   equipment explicitly and finish reachable software measurements first.
   **Exit:** reproducible p50/p95/p99/max distributions with sample counts,
   measurement uncertainty, queue-age distributions and visible frame/drop
   evidence. Software handoff timing is never labelled end-to-end latency.

2. **Reduce waiting without breaking recovery.** Follow the oldest frame and
   input sample through transport, app queues, feeder and LG render queue.
   Compare bounded frame-age and backlog policies one change at a time; keep
   button press/release ordering and decoder reference dependencies intact.
   Do not indiscriminately discard compressed frames or set all buffering to
   zero. Measure recovery after isolated loss, bursts, reordering and load;
   cloud retransmission and a clean LAN need different deadlines.
   **Exit:** lower measured response delay or tail spikes beyond measurement
   uncertainty, without stuck/missed inputs, worse corruption, freezes or
   recovery time. A smaller queue alone is not acceptance evidence.

3. **Shorten LG's decode-to-display path.** Start from the observed NDL
   game/low-delay load and existing direct-ES alternative described below;
   neither is a missing backend to recreate. Trace hidden buffering, frame
   selection and presentation feedback through `libplayerAPIs`, the AV
   connector and VDEC VO scheduling. Keep decoded video on the direct video
   plane unless a measured alternative wins. Qualify extra low-delay controls
   only if the existing path leaves work to remove; inspect live state rather
   than assuming a successful setter changed the hardware. The current direct
   ES alternative is picture-only and is not a complete gaming replacement.
   **Exit:** a physical latency A/B with HDR, colour, audio sync and frame
   pacing intact, plus verified state restoration on stop, disconnect, error
   and crash. Unknown control semantics or unverified restoration stop the
   live experiment, not the remaining offline investigation.

4. **Tune quality and both transport classes.** Establish 4K120 HDR10 HEVC
   Main10 first; 80 Mb/s is an existing clip test point, not a proven network
   default. Increase bitrate only with measured throughput margin and stable
   tail latency. Compare high-motion detail, text, dark gradients, colour and
   HDR highlights, including large keyframe bursts. Compare other codecs and
   rates on their actual quality/latency results; AV1 is not inherently faster.
   For Apollo/Sunshine local streaming, measure game rendering, capture and
   encoder delay; avoid unnecessary future-frame/lookahead waits and GPU
   render backlog, and use low-latency host options supported by that host.
   For GFN, record the server, network round-trip time, negotiated mode and
   available server-side latency options. No TV-side change removes internet
   propagation or NVIDIA's server workload. Share the TV presentation/input
   path, not unqualified cloud/LAN recovery settings.
   **Exit:** separate reproducible local and GFN profiles, with typical and
   worst-1% response, actual delivered frames, visual quality and recovery
   recorded; no maximum-bitrate or HDMI-equivalence claim without evidence.

5. **Then qualify internal VRR and 144 Hz.** Reuse the firmware findings in
   sections 2 and 3; internal VRR is not dismissed as HDMI-only, and 120 Hz is
   not a proved universal firmware ceiling. Qualify safe readback, panel
   bounds, actual frame presentation and restoration before enabling either.
   Cloud G-SYNC also needs real presentation feedback, not a negotiated flag
   alone. At 120 Hz a refresh period is 8.33 ms, versus 6.94 ms at 144 Hz: the
   1.39 ms period reduction is not a promised input-lag saving and is smaller
   than one unnecessary 120 fps frame of waiting.
   **Exit:** independent physical scan/presentation evidence and an improved
   experience over the qualified 120 Hz baseline, not merely a rate counter.

### Execution discipline and receipts

The app session owns implementation; shared LG controls remain in the trusted
helper and active only while the gaming plugin is enabled. Run offline source
work first, then group prepared A/B cases into coordinated TV windows. Reuse
the existing fixtures and evidence above rather than repeating capability
discovery. TV installation requires a main-branch revision pushed to both
remotes and the normal device approval/coordination; this plan grants no
standing approval or permission to wake the TV.

Before each A/B, fix the game/scene, rendered and streamed frame rates, TV
picture/HDR mode, controller/dongle mode, physical measurement position,
network path and test duration; record unavoidable differences. Choose sample
counts and tolerances from baseline repeatability, report the slowest 1% as
well as the median, and reject regressions outside that uncertainty. Record
each result with source revision, installed binary hashes, configuration,
measurement method, raw-receipt path and restoration result. Label every
untested case. Retain small numerical receipts, not build output or video
recordings, in the repo; temporary captures belong in session scratch.

---

## Capability summary

| Feature | Status | Implementable |
|---------|--------|---------------|
| 4K120 HDR 4:2:0 | measured clip throughput and display counters | Working test workload; physical gaming latency unmeasured |
| Additional low-latency controls | measured round trip (MEMC, Motion Pro); VPQ set refused; DE node absent | MEMC low delay and Motion Pro only; NDL already selects game/low-delay mode |
| 4K144 HDR 4:2:0 | DE selector proven offline; VDEC 120 cap identified in captured code | Blocked — signed VDEC firmware clamps presentation to 120 |
| VRR (4K120) | forced VRR measured: refresh follows the decoder timer — the declared rate, or a 300-frame arrival average with RAM-only decoder parameters | Blocked — no ForceVRR lifecycle; no per-frame following |
| Cloud G-SYNC | untested | Blocked — depends on VRR |
| HEVC 4:4:4 | measured no picture; captured Hantro parser rejects non-4:2:0 | No working decoded path; silicon capability remains unknown |
| BFI / OLED Motion Pro | measured getter round trip at the off value only | Implementable, untested as a visible change; inferred exclusive with VRR |
| On-glass presentation timestamp | no such event identified in the traced NDL/Starfish path | Software position/clock telemetry is not optical presentation feedback |
| HDR updates / Dolby Vision streams | metadata paths traced; stream acceptance untested | Mid-stream 60 Hz regression not yet fixed; Dolby Vision needs a complete encoded-metadata path |

---

## Architecture

The gaming feeder uses NDL (`libNDL_directmedia.so`). NDL exposes Load, Play,
SetFrameDropThreshold and SetHDRInfo. Its public API exposes no HFR,
low-delay toggle, VRR or frame-rate control, but its underlying load sets
`lowDelayMode` and `videoInfo.isGameMode` true. The TV logs also show the game
picture mode and display low delay; physical response improvement is
unmeasured. [Existing pipeline evidence](research/vwin-refresh-result.md).

The feeder already has `gf_smp.c` alternatives: BUFFERSTREAM with explicit
sink selection, and direct ES with configurable low delay but no
`isGameMode`. Direct ES currently refuses audio and explicit sink selection;
it is not a finished gaming replacement. NDL's `Load` takes width, height
and codec type, not fps. Codec timing, configured rate and PTS estimation
are separate sources; a valid low-delay default of 60/1 can outrank PTS
estimation when codec timing is not exported, as in the traced AV1 adapter.

Privileged display controls (MEMC bypass, DE low-delay, VRR, HFR) use V4L2
controls and DE ioctls on `/dev/lg/de0`. Per project rules these go through
the **trusted helper** and run only while the gaming plugin is enabled.

`vpu.ko` is Xtensa (VDEC coprocessor, kernel 3.18). It is not on the host
ARM64 kernel (5.4). `/sys/module/vpu/parameters/` does not exist on the host.
Parameters like `podi_frame_rate` are not host-accessible via sysfs.

The installed O22 **DE loader does not authenticate the image**: it copies
bytes to MCU memory; its `Verify` operation compares bytes, not signatures.
The separate **VDEC loader is secure**: `lgkree_req_secure_fwload(0x11, …)`
calls SMC `0x8200ff09`. An unsigned DE route does not bypass VDEC's 120 cap.
Firmware modification remains an unapproved, brick-risk experiment, not a
plugin capability. Exact boundaries and the approval-only
[DE modification plan](research/rea-firmware-result.md#de-firmware-modification-route)
are recorded separately.

---

## 1. ALLM / low latency (partly measured)

### What it does

Replicates the pipeline bypass the TV applies when an HDMI 2.1 console sends
an ALLM InfoFrame: disables FRC motion interpolation and reduces DE
processing latency.

### Privileged operations (trusted helper)

**Device discovery:**
- VPQ V4L2 device: enumerate `/dev/videoN`, match platform device
  `v4l2_vpq` via sysfs. On the TV this is `/dev/video50` (81:11).
- DE: no `resource_mgr` character device is registered on the TV. The DE
  ioctls (`resource_mgr_ioctl`) are served by `/dev/lg/de0` (1105:0).

**On session start — read, set, verify:**

```c
// 1. Save original state
struct v4l2_control orig_memc, orig_motionpro;
orig_memc.id = 0x00983A01;    // V4L2_CID_EXT_MEMC_LOW_DELAY_MODE
ioctl(vpq_fd, VIDIOC_G_CTRL, &orig_memc);
orig_motionpro.id = 0x00983A03;  // V4L2_CID_EXT_MEMC_MOTION_PRO
ioctl(vpq_fd, VIDIOC_G_CTRL, &orig_motionpro);
// Save DE low-delay state via DE_IO_GetLowDelay or de_kwrap_getlowdelaymode

// 2. Enable low latency
struct v4l2_control ctrl = { .id = 0x00983A01, .value = 1 };
ioctl(vpq_fd, VIDIOC_S_CTRL, &ctrl);   // disable MEMC
ctrl.id = 0x00983A03; ctrl.value = 0;
ioctl(vpq_fd, VIDIOC_S_CTRL, &ctrl);   // disable Motion Pro
uint32_t one = 1;
ioctl(de_fd, 0x4004613c, &one);        // DE low-delay (cmd 0x49)

// 3. Read back to verify
ctrl.id = 0x00983A01;
ioctl(vpq_fd, VIDIOC_G_CTRL, &ctrl);
assert(ctrl.value == 1);
```

**On session stop, error, resource loss or helper crash — restore saved:**

```c
struct v4l2_control ctrl = { .id = 0x00983A01, .value = orig_memc.value };
ioctl(vpq_fd, VIDIOC_S_CTRL, &ctrl);
ctrl.id = 0x00983A03; ctrl.value = orig_motionpro.value;
ioctl(vpq_fd, VIDIOC_S_CTRL, &ctrl);
uint32_t orig_de = saved_de_low_delay;
ioctl(de_fd, 0x4004613c, &orig_de);
```

Feeder disconnect can trigger helper restoration. A helper crash requires a
surviving rollback owner; that lifecycle remains unqualified until tested.

**Call chain (LG kernel source):**
- MEMC: `v4l2_vpq_vidioc_s_ctrl_be` → `vpq_hwrap_memc_lowdelaymode` →
  `BE_FRC_HAL_Set_Low_Delay` → `BE_FRC_HW_O22_SetMemcLowDelay` →
  `BE_FRC_HW_O22_SetMemcMode` → `BE_KIPC_SetData(BE_FRC_SET_MEMC_MODE, 16
  bytes)` on the BE IPC channel (`chip/o22/be/frc/be_frc_hw_o22.c`). The
  message is sent only when it differs from the previous one. Low delay 1
  sends FRC level `LX_BE_FRC_MEMC_DISABLE` (0, the level TruMotion off
  sends); Motion Pro travels in the same message as `uFRCMotionProEn`.
- DE: `resource_mgr_ioctl` → DE IPC command 0x49 (4-byte payload)
- PE DNT: NOP on O22 — no action needed.

**Verification caveat:** `de_kwrap_setlowdelaymode` caches the value in a
kernel global at `0xffffffc012548afc`. Confirm that the getter reads live
hardware state, not that cache.

The DE firmware getter `DE_IOR_GET_LOW_DELAY` (`0x8008613d`) through
`/dev/lg/de0` is answered by an MCU IPC that timed out on the TV
(`DE_KIPC_GetData ... mcu[0] receive fail`, firmware status dump, about
512 ms stall). It is not a usable readback.

The LG kernel source confirms the cache problem for every getter involved
(`kdriver/core/pe/v4l2_vpq_be.c`, `v4l2_vpq_pe.c`, `core/de/de_kwrap.c`,
`de_proc.c`):
- MEMC low delay and Motion Pro `G_CTRL` return driver globals assigned
  before the hardware call's error check, so a failed hardware call still
  reads back as the requested value.
- VPQ low delay calls `de_kwrap_setlowdelaymode`, which sets the cache that
  `/proc/lg/de/win0/status` `lowdelaymode` prints.
- `DE_IOW_SET_LOW_DELAY` (`0x4004613c`) from userspace goes to
  `DE_IO_SetLowDelay` and does not update that cache, so the `/proc`
  readback cannot reflect the helper's own DE ioctl.

The helper's `vpq_low_delay` ID `0x00983D09` is `V4L2_CID_EXT_VPQ_BYPASS_BLOCK`
(VPQ base `0x00983D00` + 9). `V4L2_CID_EXT_VPQ_LOW_DELAY_MODE` is `0x00983D0E`
(base + 14, `v4l2-ext-pq.h`); the system sets it to 1 on every NDL load and
to 0 on unload (see "Low-latency readback" below). The `G_CTRL` 0 against
`lowdelaymode` 1 during playback and the `S_CTRL` EINVAL in the TV results
come from the wrong control. Restoring VPQ low delay to a saved 0 during
playback would switch DE low delay off.

### Qualification (must pass before shipping)

The control IDs and ioctl numbers come from offline disassembly; the TV
results below cover only the getter/setter round trip. LumaGame must run a
qualification step in a fresh leased TV window before treating any of them
as working:

1. Discover the VPQ V4L2 device and the DE character device
2. Read current values of both MEMC controls (`VIDIOC_G_CTRL`) and DE
   low-delay state — confirm they return without error
3. Write the low-latency values — confirm the set calls succeed
4. Read back — confirm the new values took effect
5. Restore saved values — confirm readback matches originals
6. Measure input lag delta with an external latency tester (photodiode or
   similar). The expected reduction is unknown. Read-back, set, verification
   and restoration qualify the control lifecycle, not a performance claim;
   a latency claim additionally requires a measured physical improvement.

If any step fails, record the error and stop — the control ID or device
path may differ from the disassembly.

### TV results (2026-11-15, helper `43e74d46`)

Qualified with `POST /v1/gaming/qualify` (no receipts, so nothing became
`qualified`) during an NDL 4K120 HDR playback (picture scope
`hdrGame`/`hdrALLM`):

| Control | Original | Set | Readback | Restored | Result |
|---------|----------|-----|----------|----------|--------|
| `memc_low_delay` `0x00983A01` | 0 | 1 | 1 | 0 | sequence passed; no driver error logged |
| `vpq_low_delay` `0x00983D09` | 0 | 1 | — | — | `S_CTRL` EINVAL for both set and restore |
| `motion_pro` `0x00983A03` | 0 | 0 | 0 | 0 | sequence passed; value never changed |
| `de_low_delay` (ioctl) | — | — | — | — | discovery failed: `resource_mgr` not registered |

A separate `G_CTRL` reader saw 0/0/0 before and after each step; the MEMC
readback of 1 comes from the driver global, so it does not prove the FRC
state changed. Turning the plugin off left 0/0/0 with no journal. Picture
settings in the playback scope were identical before and after; the idle
`filmMaker`/`sdr` scope's `backlight` moved 68 → 75 (Energy Saving is
`auto`; no control in this run writes it).

The failed `vpq_low_delay` set exposed a rollback entry that blocked further
qualification and leases and was rewritten on orderly shutdown. The helper
now drops entries when readback matches the saved value even if the restore
ioctl fails, clearing both memory and journal. VPQ low delay is marked
unsupported on O22 (`S_CTRL` EINVAL) and skipped; DE discovery now uses the
registered `/dev/lg/de0` node. DE remains unqualified without live-hardware
and lifecycle receipts. These fixes have no new TV receipt.

Receipts: `/data/home/tmp/lumagame-display-20261115/` (`qualify-results.json`,
`gaming-status-*.json`, `ld-*.txt`, `q-*-dmesg.txt`, `picture-*.json`,
`rollback.json.stale-copy`).

### Low-latency readback (2026-11-15, measured)

The MEMC and Motion Pro getters return driver globals, so these readbacks
come from the DE firmware, the decoder and the PE driver instead, all
read-only. Idle is Home (`filmMaker`, `memcDelayLevel mid`). Playback is the
NDL 4K120 HDR feeder clip in `hdrGame` (`memcDelayLevel none`, 120 frames/s
fed, none dropped, queue ≤ 2), read at 4 s and 20 s in two runs.

| Readback | Idle | 4K120 game playback |
|----------|------|---------------------|
| SYS_VERI_10 low_delay / frc / frame_delay | 1 / 0 / 0 | 1 / 0 / 0 |
| SYS_VERI_10 vrr / justscan | 0 / 1 | 0 / 1 |
| SYS_VERI_10 dco_lock | 0 | 0 at 4 s, 1 at 20 s |
| SYS_VERI_10 video_delay (µs) | 650–8372, wandering | 15800–15930 |
| SYS_VERI_11 frate_in / frate_out (Hz) | 120 / 120 | 120 / 120 |
| `/proc/lxvdec/*/status` flags | no decoder | `low_latency,hfr,uhd_cont` |
| `/proc/lxvo/vdisp/vpts` match mode | no decoder | `freerun_ignore_sync` |
| PE `SetLowDelayMode` (dmesg) | 0 | 1 |

SYS_VERI_10 is `CC482228` and SYS_VERI_11 is `CC48222C`, read with
`DE_IOR_REG_RD`. The field positions come from the DE firmware's status print
(inferred): SYS_VERI_10 video_delay 19:0 (µs), pic_init 22, vrr 23, justscan
24, frc 25, frame_delay 27:26, prew 28, skip_nd0 29, low_delay 30, dco_lock
31; SYS_VERI_11 frate_in 7:0, frate_out 15:8, VRR rate 31:16.

- The firmware low_delay bit stayed 1 while the PE control went 0 → 1 → 0,
  so it does not read that control back. frc 0 and frame_delay 0 show no FRC
  frame delay in the path in either state.
- video_delay holds 15.8–15.9 ms (about 1.9 frames at 120 Hz) during game
  playback; its exact definition is unknown (inferred: the firmware's
  input-to-output delay).
- The PE driver logs LG's declared latency on each change: `Input:9,
  Vfreq:1200, Lowdelay:1 → latency:17 ms` at clip start and `Lowdelay:0 →
  75 ms` at clip end. Both come from a static table (`v4l2_vpq_event_latency`
  in `core/pe/v4l2_vpq_latency.c`), not a measurement.
- The control set at each NDL load is `V4L2_CID_EXT_VPQ_LOW_DELAY_MODE`
  (`v4l2_vpq_vidioc_s_ctrl_pe` → `de_kwrap_setlowdelaymode`), sent by a
  system process (inferred: `pqcontroller`).

Verdict: NDL's game-mode load already applies every latency control found:
PE/DE low delay, the decoder's low-latency mode (`CONTROL_NO_DELAY`, frames
shown at the next vsync), and FRC off (`hdrGame` TruMotion off, firmware frc
0). This matches LG's renderer contract for Low Delay Mode: bypass the
delaying PQ blocks and turn MEMC off (`pq.rst`, "Low Delay Mode"). MEMC low
delay sends the FRC level TruMotion off already sends, Motion Pro is BFI, and
the source-sync low-delay clock (`hal_vsc_hwrap_SetLowDelayBySourceSync` →
`DE_REG_O22_SetLowDelayClock`) refuses anything but HDMI input. No control
was found that cuts latency beyond NDL's load (inferred from source and the
readbacks above); the remaining lever is LumaGame's own queue depth. Glass
latency still needs the photodiode measurement (qualification step 6).

Receipts: `/data/home/tmp/lumagame-vrrlat-20261115/`
(`reads-{idle,play-early,play-late,idle-after-play1,idle-after-play2}.json`,
`play.out`, `play2.out`, `session.log`, `as-found.json`, `as-left.json`).

---

## 2. 4K144 HDR

### What is proven

The DE MCU firmware's rate selector at `0x50266a3c` returns 144 for a
144 fps internal source with VRR readiness = 0. Proved by eight offline
synthetic-state instruction executions; all 598 bytes match the live firmware
image. Downstream timing-table code has a 144 Hz branch. A universal
display-firmware 120 Hz ceiling is therefore incorrect.

### VO HFR selection and the 120 cap

The captured VDEC code and matching OSL source identify the HFR operands:
decoded width × height must exceed 2,764,800 pixels and codec fps must be at
least 61, unless an HFR flag/force option already selects the mode.
Low-delay mode separately seeds the configured rate with 60/1. HFR selection
does not remove the later limit.

`vds/vdisp_core.c` fixes `MAX_FRAMERATE` at **120**. Its inlined progressive
branch contains `movi a11,120` at `0x505c6971` before
`vrate_set_rate_limit` (`0x505c8f88`). `vds/vrate.c` independently clamps
rates above 120; the HFR vsync connection selects fixed 120/1 or 100/1.
This is a constant in the captured signed `vdec.bin`, not a still-untraced
DE timing preference.

The measured ~145 frames/s entering VO, ~120 selected and ~25 dropped/s,
with fixed 120/1 decoder vsync, are consistent with these gates. A DE
144 Hz scan mode cannot supply 144 unique decoded frames past them.
PTS/VUI rewriting, `force_hfr`, a forced rate of 144 and DE rate controls
are therefore not established bypasses.

The Xtensa `/proc/lxvo/vdisp/*` host view is a read-only relay. MCU debug
rate setters still meet the clamp. Host `vdec_bootopts` is consumed on a
subsequent firmware run/recovery, not a live rate control; restarting the
MCU with different arguments cannot change the compiled constant.

### AV1 reporting `vFreq=600`

`vFreq` is source fps × 10, **not panel refresh**. The AV1 parser reads
sequence-header timing, but `gx_av1_get_disp_info` does not populate the
shared rate numerator/denominator at offsets `+0x28/+0x2c`; the H.264
adapter does. The generic GX path copies cached frame information rather
than deriving an AV1 rate. This leaves the low-delay configured 60/1 as
a usable rate source. The host codec policy also lists AV1 at 60 fps;
that policy alone is not a silicon throughput verdict.

Adding AV1 timing bits alone is not an established fix for the missing
adapter output. Source metadata, decoder selection rate, DE timing and
unique visible frames must be recorded separately before claiming AV1
120. [Firmware findings and evidence](research/rea-firmware-result.md).

### Remaining prerequisites

1. An authenticated VDEC change would be needed to remove the identified
   software ceiling; no such route is established or approved.
2. DE 144 Hz timing and the C4 model gate need separate acceptance,
   including optical measurement. The DE-only plan cannot promise
   144 unique NDL frames while VDEC remains capped.
3. GFN's negotiated rate and real stream timing still need capture for
   source compatibility, not as a proposed bypass of the constant.

---

## 3. VRR / Cloud G-SYNC

### What is proven

The DE MCU firmware has a shipped internal VRR tracking path for MVT/MVR
(memory-video) sources. ForceVRR sets window ready bits, which enable
tracking registers:
- `CC460B50` bit31 — tracking enable
- `CC400800` bit0 — ready flag
- `CC8C00A0` bit6 — internal source selection

Proved offline: ready=0 + 144fps input → rate selector returns 144.
Ready=7 + 144fps input → returns 120. ForceVRR and 144 Hz are mutually
exclusive.

Measured 2026-11-15: `VRR 1` engages tracking for a decoded full-screen
window. Refresh follows the decoder's declared-rate timing, not frame
arrival: 45/67/72/75/90 fps track one-for-one, while 50/60 fps use doubled
100/120 Hz decoder timers. Only a reboot fully undoes it (Experiment results).
RAM-only decoder parameters let that timer follow a 300-frame average of the
arrival rate (80–100 fps measured), never individual frames (True VRR).

### First target: 4K120 + VRR

4K120 is what GFN delivers today. ForceVRR at 120 fps has no rate conflict,
but it gains nothing there: a 4K120 window opened under it ran at 117.0 Hz
with held refreshes (step 4).

### VRR register readback (measured)

`DE_REG_O22_RD` has no address whitelist for `0xCCxxxxxx` addresses. On the
TV it is reached with ioctl `DE_IOR_REG_RD` (`0x80046164`) on `/dev/lg/de0`
(`resource_mgr_ioctl` → `DE_IO_RegRd` → `DE_HAL_ReadReg` →
`DE_REG_O22_RD`). `DE_IO_RegRd` treats the register value as an error code:
a nonzero register fails with EIO and logs
`DE_IO_RegRd 1197: Wrong ret(0x<value>)` under the caller's pid, which is
where the value is read from; a zero register returns normally.

| Register | Idle (Home) | NDL 4K120 HDR playback |
|----------|-------------|------------------------|
| `CC460B50` (bit31 tracking) | `0x0004C000` → 0 | `0x0004C000` → 0 |
| `CC400800` (bit0 ready) | `0x00000000` → 0 | `0x00000000` → 0 |
| `CC8C00A0` (bit6 internal source) | `0x00000020` → 0 | `0x00000020` → 0 |

The registers are live (nonzero values, not clock-gated) and VRR tracking is
off for decoded 4K120 by default (`win0` vFreq 1200, lowdelaymode 1). Receipts:
`regs-idle-{1,2,3,after}.txt`, `regs-play120-{1,2,3}.txt`,
`c420-win0-fr_rate.txt` in `/data/home/tmp/lumagame-display-20261115/`.

### Enable path (source and firmware strings)

Decoded video has no kernel or luna route into VRR. The kernel hands VRR to
the firmware only as HDMI 2.1 timing flags (`LX_DE_CVI_GAME_MODE_T`
`isVrrMode`/`isGSyncMode`/`isFreeSyncMode` in the CVI info, `vp_kwrap.c`);
`V4L2_CID_EXT_VSC_ADAPTIVE_STREAM` is the adaptive-streaming/AirPlay seamless
resize switch, not VRR. For MVT/MVR sources the only route is the firmware
debug command line:

`echo VRR <N> end > /proc/lg/de/command` → `_DE_WriteProcFunction`
(`core/de/de_proc.c`) → `BE_FRC_HW_O22_RunCommand` →
`BE_KIPC_SetData(BE_FRC_B0_RUN_COMMAND)` → firmware
`MCU_BE_IO_FRC_RunCommand` (O22 DE firmware `paluma_o22n3`).

From the firmware (disassembly and strings, inferred; the prints were read on
the TV on 2026-11-15, see Experiment results):
- `VRR` without a value is a query: it prints `Current mode = N` and
  `ctrl_sync_gen_mvc_trk0 = 0x…`.
- `VRR N` stores N in bits 13–14 of a firmware debug word in MCU RAM and
  prints `forceVRR : N`. 1 forces the window ready field to 7, 2 forces it to
  0, 0 re-derives it from the source.
- Ready drives the tracking hardware: `CC460B50` bit31 (ready and on),
  `CC400800` bit0, `CC460B84` bits 13–15 (frame doubling) and bit 7 (reset),
  `CC8C00A0` bit6 (internal source). The measured period is `CC460B90`
  (bits 14:0 total, bit 15 valid, bits 31:16 delay).
- The firmware rejects VRR for a windowed or overscanned picture
  (`VRR ERR : window mode`, `VRR ERR : overscan mode`). The feeder's
  full-screen window (`-w card`) is accepted (measured).

The ForceVRR debug word is RAM-only and is lost when the firmware reloads at
boot. The live mode read 0 on 2026-11-15, which is the BSS default.

### Limits

The firmware clamps the VRR rate between an upper and a lower limit, set by
the hidden `debugType` options `VRRULIMIT`/`VRRLLIMIT` and logged as
`VRR frate:%2u.%02uHz, vrrUpLimit:%2u.%02uHz` (and `vrrLwLimit`), so the
limits are in 0.01 Hz. It turns them into a vertical-total range
(`[[ VRR ]] … vt_num:%u(%u-%u)`) and doubles frames below the floor
(`vrrDoubleOn`). The limits live in MCU RAM; no kernel path reads them and
their defaults were not recovered from the firmware. An experiment stays at
the 120 Hz ceiling the output already runs at.

The TV's EDIDs give no VRR range either (measured): HDMI 1–4
(`/proc/lg/hdmi20/portN/edid`, 512 bytes, `LG TV SSCR2`) carry no HDMI Forum
VSDB and no FreeSync block, and their range descriptor allows 24–120 Hz
vertical at up to 300 MHz pixel clock. HDMI Deep Colour is off on every port
(`uhdDeepColorHDMI1`–`4` `off`, `freesync` `off`); the HDMI 2.1 EDID that
would declare a VRR range needs a settings change to read (inferred).

### Save and restore

Saved before any VRR change, all read-only:
1. The firmware force mode from the `VRR` query. It prints once `ipc log 1`
   forwards the MCU log, and it read 0.
2. `CC460B50`, `CC400800`, `CC460B84`, `CC8C00A0`, `CC460B90` and the
   firmware status words SYS_VERI_10/11 (section 1), through `DE_IOR_REG_RD`.
3. `/proc/lg/de/win0/status` (vFreq, lowdelaymode), the DE2VSYNC/osd_irq
   rates and `/proc/lxvo/vdisp/vsync`.

No command restores the state (measured 2026-11-15). `VRR 0 end` and
`VRR 2 end` both clear tracking and ready, but `CC460B50` keeps the delay
field VRR set (`0x4E000`), `CC460B8C` stays `0x88420000`, `CC8C00A0` bit6
stays set and the idle `CC460B90` reads 28673 instead of 3945. All of it
survives closing the window, Home, a new 4K120 window, `VRR 2` followed by
`VRR 0`, and 30 s of Active Standby. A reboot restores every value, and
`luna://com.webos.service.tvpower/power/reboot` restarts the kernel on the C4
(uptime 25810 s → 30 s). With the residue, 4K120 playback measured as in
step 1 (DE2VSYNC 119.96 Hz, fewest 12 per 100 ms, dco lock 1, video delay
16.0 ms). The follow-up below also found steady ~120 Hz output for 60 fps
decoded video and HDMI2 with the residue; this is not optical or long-term
qualification. Registers are read only with the screen on, because in
Active Standby the display blocks may be unclocked.

### Measuring the scan rate

The SoC counters all come from the DE timing generator, so they show what
the DE sends, not what the panel shows. Measured 2026-11-15 (receipts in
section 1):
- `DE2VSYNC` and `osd_irq` in `/proc/interrupts`: 119.85–120.34 Hz over 2 s
  windows, idle and playback (one count is 0.5 Hz).
- SYS_VERI_11 frate_out: 120.
- `CC460B90` bits 14:0: the main window's input period, not the output.
  2250 during 4K120 playback (120.0 Hz at 270000/total), 3000 for a 90 fps
  stream while DE2VSYNC stayed at 120.4 Hz (step 4); a stale 3945 with valid
  0 at Home idle.
- SYS_VERI_11 bits 31:16: 11791–11951 during playback, frozen at idle
  (11830, 11866, 11848). With vrr 0 this looks like the firmware's input-rate
  measurement in 0.01 Hz (inferred), 118–119.5 Hz against 120 frames/s fed.

The independent measure is optical: a photodiode or a 1000 fps camera on the
panel, recorded at 48 kHz or more. An OLED refresh shows as a short
brightness dip, so the dip interval gives the scan period even on a still
image (inferred), and a clip alternating black and white frames ties each
refresh to a source frame. LG's latency pattern
(`V4L2_CID_EXT_VSC_LATENCY_PATTERN` → `DE_IOW_SET_LATENCY_MEASURE`, a box
toggled black/white on each vsync for photodiode measurement) does the same
inside the SoC but is a write.

### Frame submission through NDL (measured and source)

NDL already presents on arrival. During the 4K120 feeder clip the decoder
runs LG's low-latency mode (section 1) and `/proc/lxvo/vdisp/vpts` reports
match mode `freerun_ignore_sync` (measured): each frame's PTS is replaced by
its enqueue time, and the frame shows at the next decoder vsync unless the
queue has reached the discard level (`vpts.c` `_freerun_ignore_sync`,
commented "for camera"). The ES player has no clock (`gf_ndl.c`), and
`/proc/lxvo/vdisp/vrate` takes 120/1 from the stream (`rate_src CODEC`,
`use_fixed 0`).

That vsync is a timer, and `/proc/lxvo/vdisp/vsync` shows it pinned
(measured): `fixed_frame_rate 1`, 120/1, interval 225000 ticks of 27 MHz
(8333 µs), bounded to 224570–225430 ticks (119.77–120.23 Hz; 2/1000 less 20
ticks, `VSYNC_SHIFT_LIMIT2` in `vsync.c`). Inside that band the driver
shifts the phase every 30 vsyncs to hold the average frame wait near a fixed
target and sets a long-term arrival-rate estimate every 300 frames
(`vdisp_core.c`); `phase_shift` read −430 and +430 and `dco_intv` 225356 and
224645 in the two runs. A variable arrival cadence therefore reaches the DE
as a 120 Hz source quantised to the 8.33 ms tick. Per-frame VRR is out of
reach from NDL without LG kernel changes: the firmware's internal tracking
follows the timer, not the per-frame IPC (`deif.c` sends `pts`, `next_pts`,
`direct_video` and `is_hfr` with every frame), as step 4 measured.

### Experiment results (2026-11-15)

Run with Nitin's approval under the plan's conditions:
- a lease and the screen on;
- the receiver muted and rechecked after every launch;
- the 4K120 HDR clip full screen (`-w card`) at 120 fps, no 144 Hz anywhere.

There was no optical meter, so every rate below is an SoC counter and the
optical criteria are not measured. Receipts are in
`/data/home/tmp/lumagame-vrrexp-20261115/` (`reads-*.json`,
`step2-query-after10s.txt`, `step3-fw.txt`, `drm.json`, `session.log`).

1. The baseline matched the earlier session (table).
2. `ipc log 1 end` forwards the DE MCU log into the kernel log as
   `DE_IPC_RcvMsg` lines, starting with the buffered backlog, at about one
   line per 20 ms. `VRR end` then printed:
   - `USAGE : VRR <0:normal mode, 1:force mode On, 2:force Mode off>`;
   - `Current mode = 0`;
   - `ctrl_sync_gen_mvc_trk0 = 0x4C000, disp_pulse_src_sel=1, sel_sync_vs=1,
     nosig_duration_de=3`.
3. `VRR 1 end` met every SoC pass criterion. The firmware printed
   `forceVRR : 1`, then `[[ VRR ]] i:3840x2160, ifrate:120, d:3840x2160,
   dfrate:120, rd_dly:128(2160), delay:4E000, vt_num:2114(2250-144)`, then
   `ready:7, onOff:1, hrr_en:0, double_en:0, tracking_en:1`. During the run:
   - the capture stayed intact (mean luma 116.7 against 119.4);
   - the video window was unmuted again after the switch;
   - the kernel log showed no watchdog or MCU error;
   - the feeder kept 120 frames/s with none dropped.
4. Run separately (step 4, below).
5. `VRR 0 end` printed `forceVRR : 0` and `ready:0, onOff:0, … tracking_en:0`,
   but it left three registers at their VRR values. A reboot restored them.

| | Step 1 | `VRR 1` | After `VRR 0` | After reboot |
|---|---|---|---|---|
| `CC460B50` | `0x0004C000` | `0x8004E000` | `0x0004E000` | `0x0004C000` |
| `CC400800` | `0x0` | `0x1` | `0x0` | `0x0` |
| `CC8C00A0` | `0x20` | `0x60` | `0x60` | `0x20` |
| `CC460B8C` | `0x84000000` | `0x88420000` | `0x88420000` | `0x84000000` |
| `CC460B98` | `0x0` | `0x230` | `0x0` | `0x0` |
| `CC460B90` total (valid) | 2250 fixed (0) | 2239–2255 (1) | 2239–2266 (0) | 2250 fixed (0) |
| SYS_VERI_10 vrr / dco lock | 0 / 1 | 1 / 0 | 0 / 1 | 0 / 1 |
| SYS_VERI_10 video delay | 15.8 ms | 17.2 ms | 15.9 ms | 15.9 ms |
| DE2VSYNC over 5 s (fewest per 100 ms) | 120.2 Hz (12) | 117.4–117.8 Hz (10) | 120.2 Hz (12) | 120.0 Hz (12) |

Each switch muted the video window for about 0.2 s: the firmware's `[MUTE]`
local value went 10 → 5 → 0 over 195 ms when VRR turned on and 199 ms when it
turned off.

Under forced VRR the DE left its fixed 120 Hz timing. Its output ran about 2 %
below the 120 frames/s fed, and some 100 ms windows held only 10 refreshes. The
display path therefore held or dropped some frames (inferred; the feeder
recorded no drops).

#### Step 4: 90 and 100 fps

Run the same way later the same day with 4K HDR clips declared at 90, 100
and 120 fps (`gfn/tools/gfn_streams.py`), about 30 s each in a window opened
under the mode shown; the VRR-off runs except 120 (120) followed the restore
reboot. Every run kept its feed rate with no drops, an intact capture (mean
luma 117.8–119.4) and no kernel-log alarm. Receipts are in
`/data/home/tmp/lumagame-vrrexp-20261115/step4/` (`summary.json`,
`reads-*.json`, `fw-*.txt`, `feeder-*.json`, `session.log`).

| Declared rate (fed) | VRR | DE2VSYNC over 5 s (fewest per 100 ms) | DE2VSYNC intervals over 14 ms | `CC460B90` total (valid) | SYS_VERI_11 in/out | Video delay (dco lock) |
|---|---|---|---|---|---|---|
| 90 (90) | off | 120.4 Hz (12) | 0 of 348 | 3000 (0) | 90/120 | 14.9 ms (0) |
| 90 (90) | forced | 90.4 Hz (9) | 0 of 257 | 2989–3006 (1) | 90/120 | 22.8 ms (0) |
| 100 (100) | off | 99.9 Hz (10) | 0 of 292 | 2700 (0) | 100/100 | 15.9 ms (1) |
| 100 (100) | forced | 99.7 Hz (10) | 0 of 279 | 2698–2704 (1) | 100/100 | 19.0 ms (0) |
| 120 (120) | off | 119.8 Hz (12) | 0 of 300 | 2250 (0) | 120/120 | 15.8 ms (1) |
| 120 (120) | forced | 117.0 Hz (10) | 10 of 333 | 2242–2262 (1) | 120/120 | 17.2 ms (0) |
| 120 (90) | off | 119.8 Hz (12) | 0 of 350 | 2250 (0) | 120/120 | 15.9 ms (1) |
| 120 (90) | forced | 114.7 Hz (9) | 11 of 326 | 2248–2270 (1) | 120/120 | 17.2 ms (0) |

Forced VRR makes the DE refresh follow the stream's declared rate, not frame
arrival:
- At 90 fps it changes the rate: the DE ran at 90.4 Hz (median interval
  11.07 ms) instead of 120 Hz. The firmware printed `ifrate: 90, dfrate:120,
  vt_num:2114(2250-144)` and stretched the 2250-line timing.
- 100 fps runs at 100 Hz with VRR off too, because the firmware switches to
  its 100 Hz timing (`MD [dType 6]`, vfp 458, 2700 lines). Under VRR it
  printed `ifrate:100, dfrate:100, vt_num:3014(2700-144)` and `CC460B8C` read
  `0x8BC60000`.
- The 120 fps clip fed at 90 frames/s kept its decoder timer at 120 Hz
  (`vdisp` 120/1 while `dco_intv` read 300153 ticks, the 90 Hz arrival).
  Under VRR the DE stayed near 120 Hz and held some refreshes for 16–17.5 ms
  instead of following 90.
- A 120 fps window opened under VRR held refreshes the same way, as in the
  first run; switching VRR on mid-stream gave 119.8 Hz with none.
- `osd_irq` followed the DE (90.2 Hz at 90 fps) with the flip divider at 2,
  so the compositor's clock fell to 45 Hz (inferred).

`VRR 2 end` during playback printed `forceVRR : 2` and `ready:0, onOff:0, …
tracking_en:0`, muted the window for 0.2 s and left the query at
`Current mode = 2`. It, a following `VRR 0 end` and 30 s of Active Standby
each left the residue described under Save and restore. The reboot restored
every value, and 4K120 playback after it matched step 1.

#### Odd rates and residue follow-up (measured)

1440p HEVC Main10 HDR10 clips at 30 Mb/s, generated by
`gfn/tools/gfn_streams.py`, ran full-screen for about 20 s each. Each four-second
clip looped; VUI timing was present and declared exactly 45/50/60/67/72/75 fps.
The VRR-off comparisons preceded forcing. Rates below are SoC counters, not
optical measurements; DE2VSYNC and osd_irq were counted for 5 s in ~100 ms
windows. Every completed constant-rate run fed its requested rate without
drops or feeder errors.

| Declared / decoder fps | VRR | DE2VSYNC Hz (fewest/100 ms) | osd_irq Hz (fewest/100 ms) | `CC460B90` total (valid) | SYS_VERI_11 in/out | SYS_VERI_10 vrr/dco; delay |
|---|---|---|---|---|---|---|
| 45 / 45 | off | 119.84 (12) | 119.84 (12) | 6000 (0) | 46/120 | 0/0; 28.1 ms |
| 45 / 45 | forced | 44.89 (4) | 45.09 (4) | 6003–6008 (1) | 46/120 | 1/0; 45.4 ms |
| 50 / 100 | forced | 99.73 (9) | 99.54 (9) | 2690–2716 (1) | 50/100 | 1/0; 19.0 ms |
| 60 / 120 | off | 120.40 (12) | 120.40 (12) | 2250–2251 (0) | 60/120 | 0/0; 15.3 ms |
| 60 / 120 | forced | 119.62 (11) | 119.43 (11) | 2240–2269 (1) | 60/120 | 1/0; 17.3 ms |
| 67 / 67 | off | 119.81 (12) | 119.81 (12) | 4029–4030 (0) | 68/120 | 0/0; 19.6 ms |
| 67 / 67 | forced | 67.17 (6) | 67.17 (6) | 4013–4025 (1) | 68/120 | 1/0; 30.5 ms |
| 72 / 72 | off | 143.84 (14) | 143.84 (14) | 4500 (0) | 72/144 | 0/1; 20.3 ms |
| 72 / 72 | forced | 72.13 (7) | 72.13 (7) | 3733–3746 (1) | 72/120 | 1/0; 28.4 ms |
| 75 / 75 | forced | 75.01 (7) | 75.21 (7) | 3586–3593 (1) | 76/120 | 1/0; 27.3 ms |

Forced VRR matches the odd declared rates, but it is not a general latency
improvement: the firmware delay reading rose at 45, 67 and 72 fps. The
45 fps run had no frame doubling (`CC460B84` doubling field 0). The 50/60 fps
streams instead used 100/120 Hz decoder timers even though their source
metadata still said 50/60. All decoder timers reported `fixed_frame_rate 1`,
`rate_src CODEC`.

At 72 fps, normal mode selected a 144 Hz DE scan automatically, with a 72 Hz
decoder timer; no 144 Hz command was sent, and this does not demonstrate
144 unique decoded frames/s. Forced VRR replaced that exact 2× scan with
72 Hz. `CC460B90` is not an output-refresh meter: the normal 72→144 case
read 4500, so `270000 / total` is not a mode-independent conversion.
SYS_VERI_11 also reported 46/68/76 for declared 45/67/75; clip timing and
the decoder timer establish the actual rates.

The OSD flip divider stayed 2 throughout playback. Its clock therefore fell
to approximately 22.5/33.6/36/37.5 Hz at forced 45/67/72/75 fps (inferred
from measured osd_irq and divider), another cost of forcing these modes.

The same 72 fps clip then ran in one uninterrupted NDL window with six-second
arrival phases of 72 → 55 → 67 → 72 fps. All 1,596 access units were accepted,
none dropped. The table uses the last ~3.9 s of each phase, excluding the
first 2 s after a rate change:

| Target / measured arrivals (fps) | DE2VSYNC / osd_irq (Hz) | Fewest DE / OSD per 100 ms | `CC460B90` total |
|---|---|---|---|
| 72 / 71.79 | 72.05 / 71.79 | 7 / 7 | 3745–3761 |
| 55 / 54.87 | 71.79 / 71.79 | 7 / 7 | 3743–3770 |
| 67 / 66.92 | 71.79 / 71.79 | 7 / 7 | 3742–3769 |
| 72 / 71.79 | 71.79 / 72.05 | 7 / 7 | 3749–3760 |

Refresh did not follow arrival. Tracking stayed on, `CC460B90` valid stayed
1 and SYS_VERI_11 stayed 72/120; the decoder snapshot during the slower
arrival phase still read 72/1. This confirms the step-4 result below 120 fps.

After `VRR 0`, `CC460B50`/`CC460B8C`/`CC8C00A0` remained
`0x0004E000`/`0x88420000`/`0x60`. Both the 60 fps decoded clip and HDMI2
retained those values, but neither lost its ~120 Hz output cadence:

| Source / state | DE2VSYNC / osd_irq (Hz) | Fewest DE / OSD per 100 ms | SYS_VERI_10 vrr/dco; delay | SYS_VERI_11 in/out |
|---|---|---|---|---|
| 1440p60 HDR, clean before | 120.40 / 120.40 | 12 / 12 | 0/0; 15.3 ms | 60/120 |
| 1440p60 HDR, residue | 119.88 / 119.88 | 12 / 12 | 0/0; 11.6 ms | 60/120 |
| 1440p60 HDR, after reboot | 120.39 / 120.39 | 12 / 12 | 0/0; 17.0 ms | 60/120 |
| HDMI2 4K60 SDR, clean | 120.17 / 119.84 | 12 / 12 | 0/1; 9.235 ms | 60/120 |
| HDMI2 4K60 SDR, residue | 119.89 / 119.89 | 12 / 12 | 0/1; 9.235 ms | 60/120 |

HDMI measurements used 3 s counter windows after 9 s of signal acquisition.
Its decoded status words matched, apart from the small input-rate estimate;
the video window was unmuted (`localmute=0`). HDMI2 was closed immediately
after each reading, with 12 s foreground checks before proceeding. The
receiver returned to input TV. CEC briefly selected MPLAY and cleared audio
mute on launches; a 250 ms receiver monitor sent MUON as soon as observed.
Audio silence in the room was not acoustically verified.

The 12:58:14 PST reboot cleared the three residue registers back to
`0x0004C000`/`0x84000000`/`0x20`, and idle `CC460B90` returned to 3945–3946.
The TV's `boot_id` stayed unchanged despite the uptime reset; reboot detection
must use uptime, not that UUID. The unchanged UUID delayed this run's lease
reacquisition until 13:02; no post-reboot TV write preceded reacquisition.

Post-checks: ssgw reported enabled, entry `ours`, gateway true and bootHook
true; the latest crash remained `20261114-170911-001`. Refresh reported
60 Hz, auto, divider 2. **The standby flip check failed:** 9 flips in 10.103 s,
then 18 in a settled 20.043 s (bursts of 9 about every 10 s). Logs recorded
an internal transition to Always Ready 7.7 s after power-off while the power
API continued to report Active Standby. No compositor/UI/settings change was
made, and this run is not accepted as a clean standby restoration.

At lease release, 13:05:33 PST, the power API reported Active Standby, HDMI2
was closed, and the receiver was OFF, input TV, mute off. HA's Apple TV entity
read `off`, but its last report stayed at the pre-test time, so the Apple TV's
actual CEC wake/standby transition remains unconfirmed. The staged TV probes,
including `cq.py`, were removed.

Receipts: `/data/home/tmp/lumagame-vrrexp-20261115/oddrates/`
(`streams-manifest.json`, `installed-feeder.json`, `reads-*.json`,
`fw-*.txt`, `feeder-*.json`, `variable.json`, `variable-summary.json`,
`summary.json`, `reboot.json`, `ssgw-after.txt`, `standby-settled-timeline.json`,
`standby-log.txt`, `finished.json`, `session.log`). The 45 fps forced run is
`p45-vrr1-run`.

### OSD and DRM route

Frames drawn by the GPU have no VRR route of their own. Measured 2026-11-15
unless marked; receipts are `drm.json`, `reads-*.json` and `osd-*.json`.
- DRM: driver `lg1k`, one CRTC running "1920x1080" at 60 and one connector
  (`TV-1`: 1920x1080, 3840x2160, 1280x720). The compositor is the DRM master.
  The CRTC carries `VRR_ENABLED` (value 0) only because the DRM core attaches
  it to every atomic CRTC (`drm_crtc.c`), and LG's driver never reads it
  (source). The connector has no `vrr_capable`; its only properties are EDID,
  DPMS, link-status, non-desktop and TILE.
- `osd_irq` counted the same vsyncs as `DE2VSYNC` in every window, forced VRR
  included (117.2 against 117.4 Hz). The OSD plane therefore scans out on the
  DE's timing. The compositor's flip clock is that vsync divided by
  `fbdev_vsync_cnt_on_flip`: 1 at Home (120 Hz) and 2 during video (60 Hz).
- Forced VRR timing follows the main video window's input (`ifrate` in the
  firmware print, `CC460B90`). `core/fbdev`, `core/gfx`, `core/drm` and
  `core/gpu` contain no VRR code, so OSD content cannot set the refresh
  (inferred from source).

GFN and local streaming (Sunshine, Apollo, Punktfunk) decode through an NDL
video window, the path this experiment used. Native games drawn on the OSD
plane could reach VRR only by being encoded into a video window (inferred).
Under forced VRR the compositor's vblanks also become irregular (measured
above), which its pacer sees as odd flip intervals (inferred).

### True VRR

Measured 2026-11-15 with the 4K120 HEVC Main10 HDR10 clip (declared 120 fps)
under forced VRR. DE2VSYNC and osd_irq were polled about every 1.4 ms, so
interval spreads below ~1.5 ms are polling jitter. Receipts:
`/data/home/tmp/lumagame-vrrexp-20261115/truevrr/`.

The VDEC MCU's module parameters are writable from the host:
`param:<value>:module/vdec/parameters/<name>` written to
`/proc/lxipc/vdec/user_command` reaches the MCU as an IPC command that its
init applies as a sysfs write (the MCU log prints `command "param" done`).
Values stay in MCU RAM until reboot and apply from the next decoder
connection. Three were changed:

| Parameter | Stock | Run | Effect |
|---|---|---|---|
| `vsync_disable_phaseshift` | 0 | 1 | Latency servo off: `next_phase_shift` read 0 instead of ±10⁷–10⁹ |
| `vsync_shift_limit2` | 2 | 800 | Timer clamp `min/max_intv` 45020–404980: 66.7 Hz up to 200 Hz, the HAL's 5 ms period floor, instead of 224570–225430 (120 Hz ±0.19 %) |
| `vsync_shift_delta` | 300 | 3000 | Period may move 111 µs per vsync instead of 11 µs (source) |

The decoder's DCO path (`check_frame`, `vdisp_core.c`) retargets the timer
every 300 displayed frames to their mean enqueue interval; the stock clamp
keeps that within ±0.19 %. With the run values the timer reaches the arrival
rate and forced VRR makes the DE follow it. Each row is a fresh load:

| Feed | Parameters | Timer (Hz) | DE2VSYNC / osd_irq (Hz) | Fewest DE per 100 ms | DE interval p5/p50/p95 (ms) | `CC460B90` median (Hz) | SYS_VERI_10 delay |
|---|---|---|---|---|---|---|---|
| 90 fps | stock | 120.23 | 91.85 / 91.65 | 8 | 7.54 / 8.43 / 17.01 | 2242 (120.43) | 17.3 ms |
| 100 fps | run | 100.03 | 100.06 / 100.06 | 10 | 9.36 / 9.80 / 11.02 | 2697 (100.11) | 20.6 ms |
| 90 fps | run | 89.99 | 90.01 / 90.01 | 9 | 10.73 / 11.04 / 12.20 | 2998 (90.06) | 22.9 ms |
| 80 fps | run | 79.99 | 80.08 / 80.08 | 8 | 11.64 / 12.46 / 13.41 | 3372 (80.07) | 25.7 ms |

The stock control stayed on the 120 Hz timer grid and held some frames for
a second period (8.4 and 16.8 ms intervals); step 4 measured 114.72 Hz for
the same feed, so the share of held frames varies between runs. With the run
values every frame got one period of 1/rate. The timer reached the rate one
window after the load (300 frames, 3.0–3.75 s), first within 1 % (100.86 Hz
for 100 fps), then within 0.03 Hz at the next window. No feeder drops or
errors occurred.
The firmware delay grew with the period, as in the odd-rate runs.

Variable arrival used one load per pattern, with a serve-mode client pacing
each access unit to an absolute deadline and the run values. Each row covers
the second half of its phase:

| Arrival pattern | Mean arrival (fps) | Timer (Hz) | DE2VSYNC (Hz) | DE per 100 ms | DE interval p5/p50/p95 (ms) |
|---|---|---|---|---|---|
| Step to 90 fps (10 s) | 90.00 | 90.00 | 91.02 | 9–10 | 9.66 / 11.01 / 12.05 |
| Step to 100 fps (10 s) | 100.00 | 99.99 | 99.99 | 9–11 | 9.45 / 9.97 / 10.90 |
| Step to 80 fps (10 s) | 80.00 | 79.97 | 80.00 | 8 | 11.32 / 12.51 / 13.78 |
| Random 70–110 fps per frame (30 s) | 85.99 | 85.37 | 85.78 | 8–9 | 10.77 / 11.50 / 12.78 |
| Alternating 70/110 fps (20 s) | 85.59 | 85.54 | 85.57 | 8–9 | 10.75 / 11.51 / 12.89 |
| Sine 70–110 fps, 12 s period (36 s) | 85.75 | 73.8–108.1 | 92.46 | 7–11 | 8.51 / 10.46 / 13.77 |

Arrivals alternating 14.3 and 9.1 ms apart produced a single DE interval
distribution around 11.5 ms: the refresh did not follow individual frames.
Rate steps reached the timer one or two windows later; the change to 80 fps
at 26 s read 93.4 Hz at 27 s and 80.0 Hz at 31 s. In the sine run the timer
stepped every ~3.5 s to the previous window's mean and lagged the arrival by
about a quarter period, reading 103.4 Hz while frames arrived at 71 fps and
73.8 Hz while they arrived at 96–104 fps; DE2VSYNC followed the timer. All
10,940 access units were accepted with the feeder queue at most 3, so frames
arriving faster than the timer were dropped inside the decoder (inferred).

The step load's opening 120 fps phase estimated 121.0 Hz in its first
window. Until the next window the DE ran at 55–59 Hz, with 6 pulses per
100 ms in 18 of 58 windows. A timer above 120 Hz can therefore halve the
refresh (cause not established), and `vsync_shift_limit2` widens the clamp
symmetrically, so it cannot cap the top rate.

Verdict: the refresh follows the arrival rate averaged over 300 frames
(about 3.5 s), not individual frames. A stream held at a steady rate below
120 fps gains a matched refresh after one window: 11.0 ms per frame at
90 fps instead of the 8.4/16.8 ms grid. A varying cadence gets lag, repeats
and decoder drops instead. The window is a compiled `30 * 10`, and the
decoder presents only on its timer; changing either needs the secure-loaded
VDEC image (Architecture). `vdisp_set_vsync_shift`, a one-shot phase shift
applied at the next displayed frame through the same route, is the only
per-frame host lever in the source. It is ignored while
`vsync_disable_phaseshift` is 1 and is untested.

OSD content and native games are unaffected: the timer exists only for
decoded windows, the OSD has no sync source of its own and DRM `lg1k` has no
VRR (OSD and DRM route above).

DE firmware plan (plan only; nothing was loaded or written): DE image 0
(`y24_pine25_o22n2_20250414.1`, SHA-256 `bc5aa595…`; main-segment VA = file
offset + `0x501dc9dc`) has no host-triggered frame start. Its sync debug
commands select fixed modes:

| Command | VA | File offset | Function |
|---|---|---|---|
| DSYNC | `0x5023d64c` | `0x60c70` | One of 26 fixed output timings |
| PSYNC | `0x5023b1e0` | `0x5e804` | Pseudo input sync at a listed rate (120, 100, 60, …) |
| DISPFR | `0x5023f02c` | `0x62650` | Display rate from a list (60/50/48/120/100/96 Hz) or force 60 Hz |
| HFR | `0x502406e0` | `0x63d04` | Forced HFR mode |
| RESET | `0x5025100c` | `0x74630` | Restarts the DE MCU |

The smallest candidate for OSD-driven refresh is a one-shot PSYNC pulse,
armed by a new command once per compositor flip (inferred). It needs the
panel's blanking bounds (open prerequisite 3) and follows the brick-risk
plan in the
[DE firmware modification route](research/rea-firmware-result.md#de-firmware-modification-route).
It cannot help decoded video, whose frames reach the DE on the decoder's
timer.

The run ended with `tvpower/power/reboot`; a backgrounded `reboot` over SSH
did not restart the TV. Post-checks: ssgw enabled, entry `ours`, gateway
true; latest crash still `20261114-170911-001`; in Active Standby, refresh
60 Hz auto, divider 2, and 0 fb0 flips in 30 s. After MUOFF and the
receiver's unmuted audio report, the TV's `getVolume` `muteStatus` changed
from true to false. Receipts: `reads-*.json`, `vds-*.json`, `feeder-*.json`,
`vfeed-var-wide.json`, `analysis-var-wide.json`, `var-wide-per-second.json`,
`fw-*.txt`, `params-*.json`, `reboot.json`, `as-left.json`, `session.log`.

#### Decoded frames through the GPU plane (source)

Whether stream frames can reach the OSD through the GPU, off the decoder
timer, so that an OSD-driven refresh could follow each frame. Evidence: LG's
kdriver tree, the installed kernel (`Image.lz4-5.4.268-329.ptl4tv.2`, code
and kallsyms), DE image 0 and the 64-bit Mali g24p0 blob; untested on the TV.

Ways out of the decoder:

- Video window (NDL, Starfish, ES player): pictures stay in the decoder's
  store as G1 8×4 tiles, HEVC packed (optionally lossy, 384 or 448 B) or AV1
  FBC, 8- or 10-bit (`kdriver/core/vdec/ipc/local/v4l2_vdec.c:490-512`).
  `libgstlxvideodec`, `libgstlxvideosink`, `libplayerAPIs`, `libpf`,
  `starfish-media-pipeline` and `videooutputd` carry no dma-buf, EGL-image,
  GP-scaler or `/dev/video21` strings.
- GP scaler (`/dev/video70`; UMS `GP_SCALER` ×4, `VTG` ×1): LG's GAV
  compositor samples a DE-scaled copy, NV12 or ARGB, at most 1920×1080,
  HDR10/HLG tone-mapped (`gp-scaler.rst:104,113,217`). The decoder's vsync
  callback releases GAV frames (`vdisp_core.c:966-984`) at twice the stream
  rate capped at 120 Hz (`vsync.c:452,474-494`): the same signed timer.
- VT capture (`/dev/video60`): copies the main scaler, sub scaler or mixer
  output at a set rate and refuses secure video (`video-texture.rst:103,
  762-763,775`).
- Stateful decoder (`/dev/video21`, `lgsoc-vdec-decoder`, the web apps'
  decoder, `media-vda-vea.rst:59`): a picture is handed over when the VDEC
  MCU reports it decoded (`DECO`/`ENDD`, `vdec_api.c:303-318`; IPC poll,
  `driver.c:893-906`), not on the timer. Each NV12 capture is a DE DMA copy
  out of the store (`fbc_to_yuv` → `dma_start`, `v4l2_vdec.c:538-570`;
  `v4l2_gps.c:2691-2741`), one at a time, waiting for the DE MCU's GPS
  interrupt, 22 ms per try (`VIDEO_GPSDMA_WaitVsync`, `de_ipc.c:1921-1949`).
  The installed kernel matches: `dst_worker` (`0xffffffc010d2a710`) calls
  `dma_start` (`0xffffffc01095b6b4`) at `0xffffffc010d2ab70`, which calls
  `gps_if_dma_start` → `DE_IO_SetGPSDMA`, then `VIDEO_GPSDMA_WaitVsync`. In
  DE image 0 the copy runs in the IPP4 task that also runs the GP scalers
  (`PQE_TASK_Ipp4DMAManager`, `PQE_TASK_Ipp4GpsManager`,
  `MCU_DE_IO_SetGpsDMA`).

`/dev/video21` output:

- Capture formats are NV12 and `LFBC` (`v4l2_vdec.c:265-279`). NV12 is 8-bit
  in one buffer, chroma at width × height (`:556`), at the decoded size up to
  3840×2160 (`:71-72,557-558`); DE image 0 has an `Invalid GPS-DMA out size`
  check with an unknown limit. `LFBC` carries no pixels: it is a copy of the
  store descriptor for the video output, the picture held until returned
  (`fbc_to_vo`, `:572-588,771-774`).
- 10-bit pictures (`bpp` 1, `:497-512`) are written into the 8-bit buffer and
  the DMA request has no HDR field (`de_kapi.h:3655-3676`), so HDR10 arrives
  as 8-bit PQ values without tone mapping (inferred). No P010 output exists.
- Export: `VIDIOC_EXPBUF` (`:2003`) through `vb2_hma` (`vb2_hma.c:351-374,
  574`), one contiguous scatterlist entry (`:155-187`); the kernel has
  `kbase_mem_import` (`0xffffffc010d53920`). The Mali blob has
  `EGL_EXT_image_dma_buf_import_modifiers`, `GL_OES_EGL_image_external_essl3`,
  `GL_EXT_YUV_target` and a DRM format table with NV12, NV21, NV16, P010,
  P210, Y210 and Q410 (file offset `0x2a14350`). An NV12 import should work
  on both sides (inferred).
- Secure mode (`V4L2_CID_EXT_SVP`, `:1268`) puts the bitstream in the secure
  pool `vpb` (`:844-847`). Game streams carry no DRM and can run in the clear
  (inferred). The client must acquire `VDEC` through UMS first
  ([hw-transcode](research/hw-transcode.md)).

No route is zero-copy: Mali imports linear YUV and Arm AFBC, not the
decoder's tiling or compression (inferred), and the only GPU-readable output
is the DE copy.

The OSD takes at most 1920×1080 at 8 bits per channel (AR24, XR24, AB24,
XB24, one AFBC layout); its scaler doubles that to 3860×2170 and crops
3840×2160. 4K OSD is a boot option, off here (`support_4kosd 0`). Flips land
on DE2VSYNC at 120 Hz, and `lg1k` has no VRR or async flip
([display-chip-direct](research/display-chip-direct.md),
[scanout-research](research/scanout-research.md)). HDR tone mapping, noise
reduction, super-resolution and MEMC act only on video windows; graphics get
the HDR-UI LUT and CSC block (`0xCC480700`) when the output is HDR. A stream
on the OSD is therefore 1080p, 8-bit and SDR, upscaled by the OSD scaler.

GPU cost (inferred, scaled by bytes moved): one 1080p ARGB full-screen quad
into the linear page takes 4.1 ms, about 16.6 MB at ~4 GB/s
([ui120-result](research/ui120-result.md)). A 1080p NV12 frame drawn into
that page is then about 2.8 ms and a 4K one about 5.1 ms: 34 % or 61 % of the
GPU at 120 fps before any UI, against none for a video window. Sampling
imported memory is unmeasured (rendering into it measured 5–20× slower). The
DE copy writes another 3.1 MB (1080p) or 12.4 MB (4K) per frame.

Verdict: not viable as zero-copy. `/dev/video21` is the one route off the
decoder timer, so with a flip per frame and the OSD-driven refresh above it
could give per-frame VRR. Per frame it adds driver hand-offs (under 0.5 ms),
the DE copy (unmeasured) and the GPU pass, and drops the timer wait (0–8.3
ms). Taking 1–4 ms for a 1080p copy, the net is −4 to +7 ms at 1080p with
OSD-driven refresh; a copy started only on a DE vsync, or a fixed 120 Hz
flip, each add up to 8.3 ms. At 4K the net is +1 to +22 ms (inferred). The
DE's video delay (SYS_VERI_10, 15.8 ms at 120 fps) is for video windows; the
OSD path's delay is unmeasured. Picture cost: 1080p, 8-bit, no HDR and none
of LG's video processing.

Decisive TV test, only if the zero-copy bar is relaxed for 1080p SDR: the
`/dev/video21` tool of hw-transcode step 4 (`VDEC` acquired through UMS, NV12
MMAP capture) with a 1080p HEVC game clip fed at alternating 7 and 12 ms
gaps, logging QBUF and DQBUF times per frame. It shows whether completions
follow the feed or an 8.33 ms grid and bounds decode plus copy; one 4K clip
shows whether the DE accepts a 3840×2160 copy. Risk medium: first use of the
node, and an MCU call may not return.

### Cloud G-SYNC

NVIDIA supports it only on GeForce NOW Ultimate, in the native Windows
(Turing or newer NVIDIA GPU) and macOS (Apple silicon or listed Intel Macs)
apps from 2.0.59, on a VRR display above 60 Hz; browser, mobile, TV and Linux
clients are unsupported
([5504](https://nvidia.custhelp.com/app/answers/detail/a_id/5504),
[5786](https://nvidia.custhelp.com/app/answers/detail/a_id/5786)). Ultimate
already varies the stream rate on Reflex games; Cloud G-SYNC additionally
lets the client's display refresh follow the frames
([CES 2024](https://www.nvidia.com/en-us/geforce/news/g-sync-pulsar-gaming-monitor/)).
OpenNOW (`a49b124`) requests it with `requestedStreamingFeatures.cloudGsync`
(off by default) and requests Reflex with it or at 120 fps (`cloudmatch.rs`).
The streamer acts only when the negotiated profile agrees, and its Linux
pacer then presents the newest frame at once with no queue
(`linux_frame_pacing.rs`). LumaGame's engine carries the field
(`types.rs:244`) and its platform does nothing with it. The Stream setting
*Cloud G-SYNC (experimental)* (off by default) sends the request and drops the
fixed 120 fps encode lines from the ANNOUNCE
([request, answer and timing](research/decoded-vrr-4k.md#cloud-g-sync-request-server-answer-observed-timing)).

Verdict: off by default; the setting probes what the server does and is not a
VRR path. NVIDIA does not support TV clients, and
the C4's decoded path presents on a 120 Hz decoder timer (measured
`fixed_frame_rate 1`, ±0.19 %), so a varying server cadence would show as
8.33 ms-quantised judder rather than a varying refresh, while Reflex is
already requested at 120 fps. Forced VRR does not change this: it follows
the decoder's declared-rate timing, not frame arrival (step 4 and odd-rate
follow-up), and with widened decoder parameters a 300-frame arrival average
(True VRR).

### Open prerequisites

1. ~~Verify CC460B50/CC400800/CC8C00A0 readback via `DE_REG_O22_RD`~~ —
   measured 2026-11-15 (above)
2. ~~MCU print route for the `VRR` query~~ — `ipc log 1`, measured
   2026-11-15
3. VRR limits: VRRULIMIT/VRRLLIMIT defaults are not host-readable, and the
   current EDIDs declare no VRR range. DE image 0 derives 47.5 Hz (or
   39.5 Hz) to 167.0 Hz for HDMI sources, which those options override, and
   bounds decoder sources only by vertical totals 2114–28672 (≈127.7–9.4 Hz)
   (inferred); a 121 Hz decoder timer nevertheless halved the refresh (True VRR)
4. ~~Whether internal tracking follows frame arrival or the decoder timer~~ —
   the decoder's declared-rate timer, measured 2026-11-15 (step 4 and
   follow-up); widened decoder parameters make that timer follow a 300-frame
   arrival average (True VRR)
5. Optical scan measurement, including the 90 Hz and odd-rate cases
6. VRR lifecycle with save/verify/restore in the trusted helper. `VRR 0`,
   `VRR 2` and Active Standby leave residue that only a reboot clears; its
   effect on 60 fps video and HDMI2 showed no output-rate loss in short runs,
   but optical, long-term and automated-lifecycle qualification remain open.
   The odd-rate follow-up's standby flip check failed.

---

## 4. HEVC 4:4:4

The captured Hantro/VeriSilicon firmware rejects HEVC non-4:2:0 **before
opening a decode core**. `HEVCDecodeSeqParamSet` at `0x1ff44` reads
`chroma_format_idc` and returns error 3 unless it equals 1 (4:2:0).
This identifies an actual software gate; it does not establish whether
the silicon can decode 4:4:4.

`DWLGetCoreCapability` calls `HantroDecGetCoreCapability`, establishing the
decoder family, not a precise VC8000D model. The exposed OSL output also
announces `VDISP_FRAME_FORMAT_420`. Removing one parser check would not
establish a supported output layout, buffer allocation or hardware path;
the VDEC secure loader independently prevents an ordinary patched-image
load. Claims that every codec or every hardware core rejects 4:4:4 exceed
the inspected evidence.

The display side supports 4:4:4 for HDMI (`BE_FRC_Set_YUV_444_Mode`,
`vsc_set_rgb444`); that is not proof of decoded 4:4:4 support.
[Parser and secure-loader evidence](research/rea-firmware-result.md#q4-hevc-444).

**Live check (2026-11-15):** the 4K120 Main 4:4:4 10 HDR clip through the
feeder's NDL path reported `playing:1`, 1201 units fed, 0 dropped, and the
VDEC opened in HEVC low-delay HFR mode, but no picture followed: no
`display_picture`, VO initialisation or `vdisp` monitor lines (the 4:2:0
control clip logs all three), a black screen, and on close
`VCoreDrvClose 174: invalid handle` / `DecHEVCClose 532: vcore close failed`,
meaning no decode core was ever opened for the stream. No Hantro or
VC8000 HW Config dump appears in `dmesg` in either run, so it is not
reachable this way. Receipts: `e444-*` and `c420-*` in
`/data/home/tmp/lumagame-display-20261115/`.

**Practical baseline:** 10-bit 4:2:0 HDR with bitrate control. The 80 Mb/s
clip receipt above establishes one workload; higher bitrates need separate
network, decoder and image-quality qualification rather than a fixed maximum
presented as universally usable.

---

## 5. BFI / OLED Motion Pro

V4L2 control `0x00983A03` (`V4L2_CID_EXT_MEMC_MOTION_PRO`). Inferred to
be mutually exclusive with VRR — no software exclusion was found in code,
but VRR varies the refresh interval, which prevents fixed-cadence black
frame insertion. Offer only when VRR is disabled.

On the TV (2026-11-15) Motion Pro read 0 during NDL 4K120 game-mode
playback; the qualify round trip (0 → 0 → 0) passed but never changed the
value, and the getter returns a driver global (section 1), so a visible
effect is untested.

**Open question:** `BE_TOP_HAL_SetTPCParam` and the
`V4L2_CID_EXT_VBE_TPC_RECOVERY_PARAM/STATUS` controls may be panel
compensation / OLED recovery, not BFI. If so, they are relevant to VRR
safety bounds rather than BFI. Investigate before treating them as BFI
controls.

---

## 6. Presentation feedback

No traced NDL/Starfish event supplies a per-frame **on-glass timestamp**.
The decoder's display notification identifies a PTS and a firmware handoff,
not light emitted by the panel; `current_stc` in the VO notification is a
scheduled/compensated timestamp, not a measured scanout time.

Starfish's `option.useCurrentTimeWithSystemClock` converts a position update
to event 72 with `currentTime` in milliseconds and a host monotonic
`systemClock` in nanoseconds. The generic pipeline polls at intervals of at
least 50 ms. NDL's callback drops that event, and DirectMediaPlayer's timer
reports drops rather than producing periodic position events. The option
alone therefore does not add presentation feedback to the gaming path.
Optical measurement remains the reference for button-to-light latency.

## 7. HDR updates and Dolby Vision

`SetHDRInfo` can immediately regenerate `decodeInfo` from cached video
metadata and call the video-output service. That service can reconfigure
the window when HDR, rate or HFR metadata changes; missing `hfrEnabled`
clears the HFR flag. No unconditional “HDR means 60 Hz” assignment was
found in the setter. The exact metadata transition behind the reported
mid-stream 120→60 drop remains unmeasured; no fix is claimed.

The integration must preserve negotiated rate, HFR and game/low-delay
metadata across genuine HDR changes. Avoid redundant HDR calls, establish
metadata before normal streaming where supported, and compare the emitted
metadata with decoder and DE rates before accepting a correction.

The native media stack understands Dolby Vision caps and profile metadata,
but `SetHDRInfo` alone cannot supply Dolby Vision's per-picture RPU
metadata. Direct ES constructs codec caps without Dolby Vision fields and
does not build the normal parser chain. A compatible encoded stream,
profile-aware parser/decoder path and retained RPU data are required.
Dolby Vision through NDL, Dolby Vision 120 fps and gaming latency remain
untested. [Detailed findings](research/rea-firmware-result.md#q3-hdr-updates-and-dolby-vision-streams).

---

## Kernel symbols reference

### Low latency
| Symbol | Address |
|--------|---------|
| `DE_IO_SetLowDelay` | `ffffffc01092f370` |
| `DE_IO_GetLowDelay` | `ffffffc01092f4c0` |
| `de_kwrap_setlowdelaymode` | `ffffffc01093ea10` |
| `de_kwrap_getlowdelaymode` | `ffffffc01093ea94` |
| `vpq_hwrap_memc_lowdelaymode` | `ffffffc010aebc00` |
| `vpq_hwrap_memc_motionpro` | `ffffffc010aebcb0` |

### Frame rate / HFR
| Symbol | Address |
|--------|---------|
| `DE_IO_SetHighFrameRate` | `ffffffc010933580` |
| `DE_IO_SetFrRate` | `ffffffc01092d940` |
| `DE_IO_GetOutFrRate` | `ffffffc01092d824` |
| `DE_REG_O22_SetFrRate` | `ffffffc010968130` |
| `DE_REG_O22_GetFrRate` | `ffffffc0109681a4` |
| `te_debug_param_create` | `ffffffc010b9d224` |
| `te_debug_param_get_num` | `ffffffc010b9d5d0` |

### VRR
| Symbol | Address |
|--------|---------|
| `vsc_check_vrr_freq` | `ffffffc01094ac30` |
| `DE_REG_O22_RD` | `ffffffc010968000` |

---

## VDEC debug parameters

These symbols are inside the VDEC MCU. Host writes reach its module
parameters as `param:<value>:module/vdec/parameters/<name>` through
`/proc/lxipc/vdec/user_command`, RAM-only (measured for three vsync
parameters in section 3, True VRR); `/proc/lxvo/vdisp/*` is a read-only
relay. None of the rate parameters removes the compiled 120 limit.

| Parameter | Value address | Purpose |
|-----------|---------------|---------|
| `vdec.vdec_force_hfr` | `0x5067097c` | Force HFR decode mode |
| `vdec.vo_force_hfr` | `0x50670c44` | Force VO HFR presentation |
| `vdec.vdec_force_framerate_num` | `0x506708b8` | Force rate numerator |
| `vdec.vdec_force_framerate_den` | `0x506708b4` | Force rate denominator |
| `vdec.vdec_force_low_delay` | `0x50670970` | Force low-delay mode |

---

## Open prerequisites (LumaGame must not build on these)

1. VDEC's compiled 120 presentation ceiling — no authenticated change
   route is established (section 2). DE 144 timing acceptance is separate
2. ~~VRR tracking register readback~~ — measured 2026-11-15: live values,
   tracking off at idle and at 4K120 (section 3)
3. Panel safe VRR blanking bounds — VRRULIMIT/VRRLLIMIT defaults are not
   host-readable and the current EDIDs declare no VRR range (section 3)
4. Physical panel scan measurement at any rate — the SoC counters show only
   what the DE sends; the optical method is in section 3
5. VDEC secure firmware loading — blocks an ordinary patched VDEC image;
   O22 DE loading is unsigned but its modification plan needs owner approval
6. `te_debug_param` write path from userspace — not confirmed
7. HEVC 4:4:4 silicon capability and output layout — the firmware parser
   refusal is identified; removing it alone would not establish support
8. C4 platform model ID — must match DE_IO_SetHighFrameRate gate (IDs 9–10)
