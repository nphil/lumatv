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
| Additional low-latency controls | measured round trip (MEMC, Motion Pro); VPQ set refused; DE node absent | MEMC low delay and Motion Pro only; NDL already selects game/low-delay mode |
| 4K144 HDR 4:2:0 | proven offline (DE selector only) | Blocked — VO presentation policy not yet traced |
| VRR (4K120) | readback measured: tracking off at idle and at NDL 4K120 | Blocked — no ForceVRR lifecycle |
| Cloud G-SYNC | untested | Blocked — depends on VRR |
| HEVC 4:4:4 | measured: decoder shows no picture; hardware support unresolved | No working decoded path; no HW Config dump reachable |
| BFI / OLED Motion Pro | measured getter round trip at the off value only | Implementable, untested as a visible change; inferred exclusive with VRR |

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
controls and DE ioctls on `/dev/lg/de0`. Per project rules these go through
the **trusted helper** and run only while the gaming plugin is enabled.

`vpu.ko` is Xtensa (VDEC coprocessor, kernel 3.18). It is not on the host
ARM64 kernel (5.4). `/sys/module/vpu/parameters/` does not exist on the host.
Parameters like `podi_frame_rate` are not host-accessible via sysfs.

Firmware signature verification is established. Binary patches to the DE or
VDEC images are blocked until the signed-load path is solved.

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

Measured 2026-11-15: `VRR 1` engages tracking for a decoded full-screen 4K120
window, and `VRR 0` does not fully undo it (Experiment results).

### First target: 4K120 + VRR

4K120 is what GFN delivers today. ForceVRR at 120 fps has no rate conflict.

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

`VRR <saved N> end` alone does not restore the state (measured 2026-11-15).
It clears tracking and ready, but `CC460B50` keeps the delay field VRR set
(`0x4E000`), `CC460B8C` stays `0x88420000` and `CC8C00A0` bit6 stays set.
All three survive closing the window, Home and a new 4K120 window. A reboot
restores every value, and `luna://com.webos.service.tvpower/power/reboot`
restarts the kernel on the C4 (uptime 25810 s → 30 s). Whether `VRR 2 end` or
Active Standby clears the residue is untested. Registers are read only with
the screen on, because in Active Standby the display blocks may be
unclocked.

### Measuring the scan rate

The SoC counters all come from the DE timing generator, so they show what
the DE sends, not what the panel shows. Measured 2026-11-15 (receipts in
section 1):
- `DE2VSYNC` and `osd_irq` in `/proc/interrupts`: 119.85–120.34 Hz over 2 s
  windows, idle and playback (one count is 0.5 Hz).
- SYS_VERI_11 frate_out: 120.
- `CC460B90` bits 14:0: 2250 during 4K120 playback, 120.0 Hz at
  270000/total; a stale 3945 with valid 0 at Home idle.
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
reach from NDL without LG kernel changes, unless the firmware's internal
tracking follows the per-frame IPC (`deif.c` sends `pts`, `next_pts`,
`direct_video` and `is_hfr` with every frame) rather than the timer; the
experiment decides that.

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
4. Not run (90 and 100 fps).
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

Step 4 remains: under `VRR 1`, feed the same clip at 90 and 100 fps (`--fps`)
and check whether `CC460B90`, DE2VSYNC and the optical rate follow the frames
or stay at 120. The abort conditions stand: a black screen, flicker,
`localmute`, watchdog or MCU messages, tracking bits that disagree with the
status words, or DE2VSYNC gaps. Restore with a reboot unless `VRR 2 end` is
shown to clear the residue.

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
(`types.rs:244`, default false) and does nothing with it.

Verdict: `cloud_gsync` stays false. NVIDIA does not support TV clients, and
the C4's decoded path presents on a 120 Hz decoder timer (measured
`fixed_frame_rate 1`, ±0.19 %), so a varying server cadence would show as
8.33 ms-quantised judder rather than a varying refresh, while Reflex is
already requested at 120 fps. It is worth revisiting only if step 4 shows the
DE refresh following frame arrival.

### Open prerequisites

1. ~~Verify CC460B50/CC400800/CC8C00A0 readback via `DE_REG_O22_RD`~~ —
   measured 2026-11-15 (above)
2. ~~MCU print route for the `VRR` query~~ — `ipc log 1`, measured
   2026-11-15
3. VRR limits: VRRULIMIT/VRRLLIMIT defaults are not host-readable, and the
   current EDIDs declare no VRR range
4. Whether internal tracking follows frame arrival or the decoder timer
   (experiment step 4, not run)
5. Optical scan measurement
6. VRR lifecycle with save/verify/restore in the trusted helper. `VRR 0`
   leaves residue that only a reboot has cleared so far

---

## 4. HEVC 4:4:4

Decoded 4:4:4 never produces a picture on the TV; whether the HEVC cores lack
4:4:4 or the firmware gates it remains unresolved.

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
2. ~~VRR tracking register readback~~ — measured 2026-11-15: live values,
   tracking off at idle and at 4K120 (section 3)
3. Panel safe VRR blanking bounds — VRRULIMIT/VRRLLIMIT defaults are not
   host-readable and the current EDIDs declare no VRR range (section 3)
4. Physical panel scan measurement at any rate — the SoC counters show only
   what the DE sends; the optical method is in section 3
5. Signed firmware load path — blocks any binary patch approach
6. `te_debug_param` write path from userspace — not confirmed
7. Conclusive 4:4:4 verdict — no HW Config dump in `dmesg`; decoded 4:4:4
   opens no decode core (section 4)
8. C4 platform model ID — must match DE_IO_SetHighFrameRate gate (IDs 9–10)
