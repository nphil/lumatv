# Gaming display features — LumaGame contract

Status levels: **measured** (observed on the TV with receipt), **proven
offline** (executed on captured firmware, not on the TV), **inferred**
(derived from disassembly or architecture, not executed), **untested**.

Research artifacts: `/data/home/tmp/lumatv-gaming-fw-20261101/`.

---

## Capability summary

| Feature | Status | Implementable |
|---------|--------|---------------|
| 4K120 HDR 4:2:0 | measured | Yes — already working |
| ALLM / low latency | inferred | Implementable, untested — V4L2 + resource_mgr ioctl from disassembly |
| 4K144 HDR 4:2:0 | proven offline (DE selector only) | Blocked — VO presentation policy not yet traced |
| VRR (4K120) | inferred | Blocked — VRR register readback unqualified |
| Cloud G-SYNC | untested | Blocked — depends on VRR |
| HEVC 4:4:4 | inferred | No — strongly indicated hardware limitation |
| BFI / OLED Motion Pro | inferred | Implementable, untested — V4L2 control; inferred exclusive with VRR |

---

## Architecture

The feeder uses NDL (`libNDL_directmedia.so`). NDL exposes Load, Play,
SetFrameDropThreshold and SetHDRInfo. It has no HFR, low-latency, VRR or
frame-rate controls. NDL's `Load` takes width, height and codec type — no
fps field. The TV determines frame rate from SPS VUI timing or a PTS-spacing
estimator.

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

The helper itself restores when the feeder's pipe closes, covering crashes.

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
   similar). The expected reduction is unknown. The acceptance criterion is
   the lifecycle itself: read-back, set, read-back, restore, read-back.

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

### VO HFR enable decision (proven offline)

The VO HFR enable function at `0x505e242a` sets a flag (bit1 of the output
descriptor at struct+44) under two conditions:

1. `vo_force_hfr != 0` — debug force, bypasses all checks
2. Normal path: resolution > QHD (`w×h > 2764800`) AND fps > 60

For 4K144 (`3840×2160 = 8294400 > 2764800`, `144 > 60`), both checks pass.
The HFR flag **is already set** for 4K144.

### Where the 120 cap actually is

The 120000 literal at `0x505a6b0c` has zero `l32r` code references — it is
a value in a data table, not a code literal pool entry. The earlier claim
that it caps the rate via code at `0x505a6dca` was incorrect; that address
is also in a data table.

Since the VO HFR flag is set for 4K144, the 120 Hz cap is **downstream**
of the VO layer, in the DE output timing generator. The DE rate selector
(proven offline) returns 144 with readiness = 0, but the host-side DE
output timing setup (`DE_IO_SetFrRate`, `DE_REG_O22_SetFrRate`) may
configure the panel for 120. The cap is likely in the negotiation between
the VDEC VO timing report and the DE output configuration.

### Viable approach (ordered by risk)

**A. PTS / VUI rewriting (lowest risk, no firmware access)**

The TV estimates frame rate from SPS VUI timing or PTS spacing. The test
fixture carries VUI with `num_units_in_tick=1, time_scale=144`. Real GFN
NVENC streams often omit VUI timing, so the PTS estimator (`esFPS` /
`contFPS`) determines the rate. If the feeder rewrites PTS spacing to
1/144s, the VO estimator may report 144 to the DE, which would then call
`DE_IO_SetFrRate` with a 144 Hz target.

**B. DE_IO_SetHighFrameRate + DE_IO_SetFrRate (medium risk)**

`DE_IO_SetHighFrameRate` sends DE IPC command 0x63. `DE_IO_SetFrRate`
configures the actual output timing. Both are host-callable through the
DE driver. The function is gated by a platform model check (model IDs
9–10); the C4's model ID must be verified.

**C. VDEC debug parameters via IPC (medium risk)**

`vdec.vo_force_hfr` forces the VO HFR flag (already set for 4K144, so
this is redundant unless the normal-path conditions fail at runtime).
`vdec.vdec_force_framerate_num/den` can override the rate report to the DE.

**Dead ends:**
- `podi_frame_rate` sysfs — does not exist on the host (Xtensa coprocessor)
- DE/VDEC firmware binary patches — blocked by firmware signature verification
- The 120000 literal at `0x505a6b0c` — data table, not code

### Open prerequisites

1. Trace what `DE_IO_SetFrRate` / `DE_REG_O22_SetFrRate` configures and
   whether the DE accepts a 144 Hz timing request from the VO layer
2. Verify the C4's platform model ID against `DE_IO_SetHighFrameRate`'s
   model gate (sub 9, cmp 1 → accepts IDs 9 and 10)
3. Confirm GFN negotiates 144 fps and check NVENC VUI timing presence

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
content — only the decoder cannot produce it.

**One cheap live check:** the Hantro HW Config register dump is printed at
core reservation time. Look for it in existing `dmesg` or log captures before
requesting a TV window.

**Practical substitute:** 10-bit 4:2:0 HDR at high bitrate (up to 150 Mbps).
Expose bitrate control in settings.

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

1. DE output timing acceptance of 144 Hz — `DE_IO_SetFrRate` /
   `DE_REG_O22_SetFrRate` not traced; this is where the 120 cap likely lives
2. VRR tracking register readback — access path proven (no whitelist), but
   register contents not verified on TV (may be zero if clock-gated)
3. Panel safe VRR blanking bounds — VRRULIMIT/VRRLLIMIT values unknown
4. Physical panel scan measurement at any non-120 rate — not independently
   observed
5. Signed firmware load path — blocks any binary patch approach
6. `te_debug_param` write path from userspace — not confirmed
7. Hantro HW Config dump for conclusive 4:4:4 verdict — not captured
8. C4 platform model ID — must match DE_IO_SetHighFrameRate gate (IDs 9–10)
