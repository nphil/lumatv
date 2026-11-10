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
  channel holds eight items, which may include audio; the NVST 52 ms reorder
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
[the engine manifest](../gfn/engine/Cargo.toml):
`native/opennow-streamer/crates/opennow-streamer-core/src/lib.rs` for input
draining/media capacity, and `opennow-streamer-transport/src/nvst.rs` for
`MJOLNIR_REORDER_DEQUEUE_TIMEOUT`. Locate the app's actual consumers and
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
| Additional low-latency controls | inferred | V4L2 + resource_mgr ioctl unqualified; NDL already selects game/low-delay mode |
| 4K144 HDR 4:2:0 | proven offline (DE selector only) | Blocked — VO presentation policy not yet traced |
| VRR (4K120) | inferred | Blocked — VRR register readback unqualified |
| Cloud G-SYNC | untested | Blocked — depends on VRR |
| HEVC 4:4:4 | hardware support unresolved | No working decoded path established; hardware limitation strongly indicated, not conclusive |
| BFI / OLED Motion Pro | inferred | Implementable, untested — V4L2 control; inferred exclusive with VRR |

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
and codec type, not fps. The TV determines frame rate from SPS VUI timing or
a PTS-spacing estimator.

Privileged display controls (MEMC bypass, DE low-delay, VRR, HFR) use V4L2
extended controls and resource_mgr ioctls. Per project rules these go through
the **trusted helper** and run only while the gaming plugin is enabled.

`vpu.ko` is Xtensa (VDEC coprocessor, kernel 3.18). It is not on the host
ARM64 kernel (5.4). `/sys/module/vpu/parameters/` does not exist on the host.
Parameters like `podi_frame_rate` are not host-accessible via sysfs.

Firmware signature verification is established. Binary patches to the DE or
VDEC images are blocked until the signed-load path is solved.

---

## 1. ALLM / low latency (implementable, untested)

### What it does

Replicates the pipeline bypass the TV applies when an HDMI 2.1 console sends
an ALLM InfoFrame: disables FRC motion interpolation and reduces DE
processing latency.

### Privileged operations (trusted helper)

**Device discovery:**
- VPQ V4L2 device: enumerate `/dev/videoN`, match platform device
  `v4l2_vpq` via sysfs.
- resource_mgr: char device registered by `resource_mgr_probe`.

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
ioctl(resmgr_fd, 0x4004613c, &one);    // DE low-delay (cmd 0x49)

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
ioctl(resmgr_fd, 0x4004613c, &orig_de);
```

Feeder disconnect can trigger helper restoration. A helper crash requires a
surviving rollback owner; that lifecycle remains unqualified until tested.

**Call chain (from disassembly):**
- MEMC: `v4l2_vpq_vidioc_s_ctrl_be` → `vpq_hwrap_memc_lowdelaymode` →
  `BE_FRC_HAL_Set_Low_Delay` → `BE_FRC_HW_O22_Set_Low_Delay` →
  `BE_KIPC_SetData(0x80010001, buf, 16)`
- DE: `resource_mgr_ioctl` → DE IPC command 0x49 (4-byte payload)
- PE DNT: NOP on O22 — no action needed.

**Verification caveat:** `de_kwrap_setlowdelaymode` caches the value in a
kernel global at `0xffffffc012548afc`. Confirm that the getter reads live
hardware state, not that cache.

### Qualification (must pass before shipping)

None of the V4L2 control IDs, ioctl numbers or device paths have been
verified on the TV. All are from offline disassembly. LumaGame must run a
qualification step in a fresh leased TV window before treating them as
working:

1. Discover the VPQ V4L2 device and resource_mgr char device
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

---

## 2. 4K144 HDR

### What is proven

The DE MCU firmware's rate selector at `0x50266a3c` returns 144 for a
144 fps internal source with VRR readiness = 0. Proved by eight offline
synthetic-state instruction executions; all 598 bytes match the live firmware
image. Downstream timing-table code has a 144 Hz branch. A universal
display-firmware 120 Hz ceiling is therefore incorrect.

### VO HFR enable decision (inferred from disassembly)

The VO HFR enable function at `0x505e242a` sets a flag (bit1 of the output
descriptor at struct+44) under two conditions:

1. `vo_force_hfr != 0` — debug force, bypasses all checks
2. Normal path: the product of two instance fields at offsets 24 and 28
   exceeds 2764800, AND a computed rate value exceeds 60

The operands at offsets 24/28 are unidentified. 2764800 equals 1920×1440
(QHD pixel count), suggesting a resolution threshold, but this is a guess
from the value — the field names are not established. The rate comparison
against 60 at `0x505e25a5` is clearer but the rate source field is also
unidentified. Both conditions are **inferred**, not proven.

### Where the 120 cap is (uncertain)

The 120000 literal at `0x505a6b0c` has zero `l32r` code references — it is
a value in a data table, not a code literal pool entry.

The measured evidence points at the **VDEC VO presentation layer**, not the
DE: the morning report records ~145 frames/s written to VO buffers but only
~120 selected for presentation, with ~25 dropped per second. This is a
VO-layer frame selection decision, not a DE output timing problem. The VO
layer selects which decoded frames to present at each vsync, and the frame
scheduling/matching code (`vdisp_set_frame_rate`, `vpts_match`) is where
the 120 selection most likely lives. The exact code path is not yet traced.

### Viable approach (ordered by priority)

**A. Trace the VO presentation cap (offline, highest priority)**

The measured ~25 drops/s at 4K144 places the cap in the VO frame selection
code. The `vdisp_set_frame_rate` function and the `vpts_match` duration
switch at `0x505ca144` decide which decoded frames to present at each
vsync. Tracing these with synthetic state (same method as
`emulate-de-rate-selector.py`) would show exactly where 120 is selected.
The `esFPS` / `contFPS` estimator feeds into this decision.

**B. PTS / VUI rewriting (lowest live-test risk)**

The TV estimates frame rate from SPS VUI timing or PTS spacing. The test
fixture carries VUI with `num_units_in_tick=1, time_scale=144`. Real GFN
NVENC streams often omit VUI timing, so the PTS estimator determines the
rate. If the feeder rewrites PTS spacing to 1/144s, the VO estimator may
see 144 and adjust its frame selection accordingly.

**C. VDEC debug parameters via IPC (medium risk)**

`vdec.vo_force_hfr` forces the VO HFR flag.
`vdec.vdec_force_framerate_num/den` can override the rate the VO uses for
frame selection. If the 120 cap is gated by the estimated rate, forcing
144 may lift it.

**D. DE_IO_SetHighFrameRate + DE_IO_SetFrRate (medium risk)**

If the DE also needs to be told to scan at 144, these host-callable
functions configure DE output timing. Gated by a platform model check
(model IDs 9–10).

**Dead ends:**
- `podi_frame_rate` sysfs — does not exist on the host (Xtensa coprocessor)
- DE/VDEC firmware binary patches — blocked by firmware signature verification
- The 120000 literal at `0x505a6b0c` — data table, not code

### Open prerequisites

1. Trace VO `vdisp_set_frame_rate` and `vpts_match` with synthetic state
   to find the exact 120 selection branch
2. Confirm GFN negotiates 144 fps and check NVENC VUI timing presence
3. Verify C4 platform model ID against `DE_IO_SetHighFrameRate` gate

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

### First target: 4K120 + VRR

4K120 is what GFN delivers today. ForceVRR at 120 fps has no rate conflict.

### VRR register readback (proven offline)

`DE_REG_O22_RD` (`0xffffffc010968000`) has **no address whitelist** for
`0xCCxxxxxx` addresses. It uses `__ioremap` for any address with 0xCC top
byte. Small addresses (≤0xFFFF) are auto-prefixed with 0xCC000000.

All three VRR tracking registers are accepted:
```
(DE_REG_O22_RD(0xCC460B50) >> 31) & 1  → tracking bit
DE_REG_O22_RD(0xCC400800) & 1          → ready bit
(DE_REG_O22_RD(0xCC8C00A0) >> 6) & 1   → internal source bit
```

This provides host-side VRR state verification without the MCU print
channel. The reads are non-destructive hardware register accesses via
`__ioremap`. This must still be verified on the TV (the register may
return zero if the display engine clock is gated or the register is
write-only), but the access path is proven.

The ForceVRR debug word is RAM-only — does not persist across standby/reboot.
Force=0 re-derives ready from upstream state. Restoration target is the BSS
default of 0; provenance: packaged initial data, not a verified live reading.

### Cloud G-SYNC integration

`MediaStreamConfig.cloud_gsync: bool` (`types.rs:244`) is negotiated but
ignored by the TV runtime. Real implementation needs:

1. VRR on/off lifecycle in trusted helper with verified restoration
2. Variable frame submission — stop fixed-cadence; use freerun or
   `adaptive_freerun` for AV-sync relaxation
3. Safe blanking bounds — VRRULIMIT/VRRLLIMIT panel values are unknown
4. Frame presentation feedback — report actual display timestamps to GFN
5. Independent physical scan measurement — DE-derived Hz
   (270000/total from CC460B90) is not independently verified

### Open prerequisites

1. Verify CC460B50/CC400800/CC8C00A0 readback via `DE_REG_O22_RD`
2. Establish MCU print route for VRR query/response
3. Qualify panel safe blanking bounds
4. Build VRR lifecycle with save/verify/restore in trusted helper

---

## 4. HEVC 4:4:4

Strongly indicated hardware limitation, not proven by a live HW Config dump.

Five independent evidence chains (all inferred from firmware analysis):
1. HEVC engine descriptor has no chroma_format_idc field
2. DPB allocator uses NV12 layout (half-resolution CbCr)
3. Every decoder (HEVC, H.264, AV1, VP9, MPEG-2, MPEG-4) rejects non-4:2:0
4. VO layer independently rejects non-4:2:0 subsampling
5. No debug parameter for chroma override among 30+ force params

The decoder IP is Hantro VC8000D-series. The H.264 engine descriptor does
include a chroma field (2 bits at word2), but the H.264 parser still rejects
non-4:2:0 — the field exists in the hardware but the firmware gates it.
Whether the HEVC ASIC cores have 4:4:4 blocks is not conclusively
established without a live HW Config dump.

The display pipeline supports 4:4:4 for HDMI input
(`BE_FRC_Set_YUV_444_Mode`, `vsc_set_rgb444`). The panel can show 4:4:4
content; the investigated decoded paths reject it. That does not by itself
establish which restriction is firmware policy versus decoder hardware.

**One cheap live check:** the Hantro HW Config register dump is printed at
core reservation time. Look for it in existing `dmesg` or log captures before
requesting a TV window.

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

**Open question:** `BE_TOP_HAL_SetTPCParam` and the
`V4L2_CID_EXT_VBE_TPC_RECOVERY_PARAM/STATUS` controls may be panel
compensation / OLED recovery, not BFI. If so, they are relevant to VRR
safety bounds rather than BFI. Investigate before treating them as BFI
controls.

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
| `vsc_set_adaptives_stream` | `ffffffc010948b20` |
| `DE_REG_O22_RD` | `ffffffc010968000` |

---

## VDEC debug parameters

Accessed through `te_debug_param_*` / VDEC IPC, not sysfs.

| Parameter | Value address | Purpose |
|-----------|---------------|---------|
| `vdec.vdec_force_hfr` | `0x5067097c` | Force HFR decode mode |
| `vdec.vo_force_hfr` | `0x50670c44` | Force VO HFR presentation |
| `vdec.vdec_force_framerate_num` | `0x506708b8` | Force rate numerator |
| `vdec.vdec_force_framerate_den` | `0x506708b4` | Force rate denominator |
| `vdec.vdec_force_low_delay` | `0x50670970` | Force low-delay mode |

---

## Open prerequisites (LumaGame must not build on these)

1. VO frame selection at 144 fps — trace `vdisp_set_frame_rate` /
   `vpts_match` first (section 2); DE output timing acceptance through
   `DE_IO_SetFrRate` / `DE_REG_O22_SetFrRate` is a separate open prerequisite
2. VRR tracking register readback — access path proven (no whitelist), but
   register contents not verified on TV (may be zero if clock-gated)
3. Panel safe VRR blanking bounds — VRRULIMIT/VRRLLIMIT values unknown
4. Physical panel scan measurement at any non-120 rate — not independently
   observed
5. Signed firmware load path — blocks any binary patch approach
6. `te_debug_param` write path from userspace — not confirmed
7. Hantro HW Config dump for conclusive 4:4:4 verdict — not captured
8. C4 platform model ID — must match DE_IO_SetHighFrameRate gate (IDs 9–10)
