# Driving the display chip directly

Research only, 2026-10-29. No TV access and no hardware write. The question: can LumaTV use the C4's display hardware (OSD layers, video windows, scalers, AFBC, picture-quality blocks) directly, past LG's driver limits, and what is worth doing first. The next worker starts from § Open questions, § Dead ends and the **Actionable items** at the end.

## Resume here

Done: this document, and `tools/regdecode.py` (item 6: names `regdump` registers by plane window, diffs two dumps with a noise pair, finds a register by name) with `tests/test_regdecode.py`. No TV access, nothing written to any hardware; committed to forge/main.

Next, in order:

1. A read-only TV slot with the commands in § TV checks (answers open questions 1, 2, 6, 7). Needs a TV window, no display change.
2. Offline, behind switches, host-tested: a plane offset for the direct plan, driven by `oledcare`; and the KMS side of the cursor plane (its policy and tests already exist in HwPlanes' `src/pointer.rs`). Files: `compositor/crates/lumatv-compositor/src/scene.rs` (`Plan::sole_candidate`, `NotSole`), `src/scanout/mod.rs` (`Scanout::choose`), `src/kms/mod.rs` (plane commit, `PlaneType::Cursor`), `src/oledcare.rs`, `src/render/cursor.rs`; tests in `tests/scanout.rs` and `tests/oledcare.rs` against the fake KMS.
3. One display slot with Nitin at the set for wins 2-5 (§ 4); raw-register tests (wins 7 and 8) only if he wants them.

Decisions taken: KMS-level doors rank above raw registers; the O22 tree (not O24) is the reference; OSD0_1 stays parked behind items 1-5; kernel-module and raw-write work is not started (Nitin's decision); no extracted firmware, script or capture was added to the repository. The pointer plane (win 3) is already HwPlanes' work (`docs/research/hw-planes-result.md`; `src/pointer.rs`, `src/afbc.rs`, `src/vwin.rs`): continue that, do not start a second one.

Open for Nitin: whether OLED-care orbit may move to the hardware plane offset, and whether a read-only slot may run now.

## Verdict

- The display chip is LG's own: kernel command line `chip=O22A3 pqe.tool=C4 pqe.year=24`, device-tree model "LG Electronics, DTV SoC LG1213" (TV). MediaTek is only the MT7668 Wi-Fi/Bluetooth radio, so MediaTek display sources do not apply, and no public datasheet for the chip turned up. LG's GPL tree is the only register documentation.
- That tree documents the **graphics side to bit-field level** (OSD layers, AFBC readers, OSD sharpener, 2D blitter) and gives **names and addresses** of 204 blocks of the whole display chip and about 2,860 picture-engine registers. It does not say how the scalers, picture quality, MEMC, orbit and HDR work: that runs as closed firmware on the display engine's MCUs, shipped in the tree only as byte arrays.
- The OSD registers already have three writers: the `lg1k` driver at every flip, the PQE firmware at every frame, and LG's picture and panel services. A raw write fights at least one of them, so raw registers are the last resort. A kernel module is feasible in principle (public precedent below), is Nitin's decision, and buys little that user space cannot do.
- The best wins use doors LG already opens: a read-only observation kit, the KMS plane position as hardware orbit, the cursor plane, a smaller primary buffer scaled by the OSD scaler, and the OSD sharpening knobs. Long shots: OSD0_1 as a second layer (one probe run showed nothing), video tiles through the four graphic scalers, the OSD's contrast, saturation and mixer-hole registers.

## Verified and inferred at a glance

| Claim | Status |
|---|---|
| Chip O22A3, y24 PQE firmware (`pqe.year=24`) | verified (TV command line) |
| One blended graphics layer; fb1 is an alias of OSD0 | verified (TV dmesg, SRC) |
| Two scaler windows (main, sub0) and four graphic scalers | verified (TV resource table, SRC) |
| `lg1k` honours FB address, size, format and `CRTC_X/Y` of plane 32; ignores `CRTC_W/H` and `SRC_*` | verified in source; not exercised on the set |
| `CRTC_X/Y` moves the picture on the panel, and in which units | inferred |
| OSD0 output size and crop belong to the firmware; live canvas 3860×2170, crop 3840×2160 at (10, 5) | verified (TV registers, SRC); the cause (LG orbit) is inferred |
| OSD VSP is enabled at neutral values and `lg1k` never writes it | verified; who writes it is unknown |
| Sharpener knobs exist; live gain 0x10 | verified; effect on the panel not seen |
| The AFBC decoder has no layout registers; z-list, video mixer, pivot, 3D, CSC ioctls are no-ops | verified (SRC) |
| `DE_IOW_REG_WR` writes any physical address through a 0666 node | verified (SRC; node mode TV) |
| A kernel module loads on the set | inferred (defconfig, public precedent); not tried |
| Cursor plane works under LumaTV's DRM master; the OSD scaler takes a 720p buffer | inferred / unknown |
| `kprobe_events` is available | unknown |
| OSD0_1 blends at 2K | probe run once under LG's compositor: every write held, no second layer seen (E13); not tried under LumaTV's compositor |

## Evidence and sources

Tags: `TV` (read on this set), `REPO` (recorded result or code here), `SRC` (LG's GPL tree), `FW` (strings of the firmware images inside that tree), `PUB` (public page), `[INFERENCE]` (not shown; the check that would show it is named).

`SRC` root: `/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/` (tarball `linux-and-kdriver_paluma-5-241025-0301.tar.gz`, LG's GPL drop; LG publishes such drops at https://opensource.lge.com/product). Paths below are relative to its `kdriver/` unless they start with `linux-rockhopper/`. Its `build.sh` defaults to the O24 chip; the C4 is O22 (`chip/o22`).

The drop is not byte-identical to the set: kernel 5.4.268 submission 324 (the set runs 5.4.268-329.ptl4tv.2), kdriver `1237.paluma.1` (webOS 25), DE firmware images per board year (`BUILD_FEATURE_y24_board` links `de_fw_pak_o22a0_y25_o22n2.h`; the set reports firmware 7356, 2025-04-14). `SRC` says how LG's code works; `TV` says what this set did.

| Topic | Where |
|---|---|
| Planes, modes, cursor | `core/drm/drm.c` (`:298-358` modes, `:432-540` plane update, `:586-640` cursor, `:822-1003` plane creation), `core/fbdev/fbdev_drm.c` (`:300-545`) |
| OSD driver, proc and parameters | `core/fbdev/{fbdev_proc.c,fbdev_drv.c}`, `chip/o22/fbdev/{fbdev_hw_o22_cmn.c,fbdev_hw_o22.h,fbdev_drv_o22.c,fbdev_proc_o22.c}` |
| OSD register headers | `chip/o22/fbdev/{osd_reg_o22.h,osd_afbc_reg_o22.h,osd_top_reg_o22.h,osd_shp_reg_o22.h}` |
| User ioctl headers | `include/{fbdev_kapi.h,de_kapi.h,be_kapi.h,pe_kapi.h,gfx_kapi.h}`, `core/vdec/include/vo_kapi.h` |
| Display engine | `core/de/{de_io.c,de_proc.c,mcu/de_ipc_def.h}`, `chip/o22/de/{de_reg_o22.c,de_reg_def_o22.h,de_fw_o22.c,de_fw_pak_o22a0_y25_o22n2.h,de_fw1..4_pak_o22a0.h}` |
| Block address map | `chip/o22/pe/reg/pe_reg_def_o22.h:50-245` (inside `#if 0`, "REGISTER/abp_base.h (O22A0_PQE_FW)") |
| LG's V4L2 API and its documentation | `include/webosext/v4l2-ext-renderer-header/linux/*.h` and `.../documentation/source/renderer/{scaler,gp-scaler,video-texture,panel-controller,pq}.rst` (block diagram: `resources/Scaler_IP_Block_Diagram_n.PNG`) |
| Kernel configuration | `linux-rockhopper/arch/arm64/configs/lg1k_defconfig`, `build.sh`, `submissions` |
| TV captures | `docs/research/arch/{scanout-hw-02.txt,scanout-live-01.txt,lgproc-fbdev-01.txt,lgproc-gfx-de-01.txt,drmplanes-01.txt,drm-debugfs-01.txt}`, `docs/research/compositor/osd01-probe-2026-10-29/` (E13: `regdump` before, with the layer on, after, and a noise pair), `docs/research/framerate/*/before.txt` (kernel command line), `docs/research/gfn/tv-2026-10-26/umediaserver_resource_config.txt`, `docs/research/hardware-census.md` |
| Earlier results | `scanout-research.md`, `hw-planes-result.md`, `framerate-result.md`, `gfn-444-vrr-result.md`, `ui120-result.md`, `videoplane-saver-result.md`, `oledcare-result.md`, `hearth-update-result.md`, `compositor-gate9-result.md` (`:887-890`), `scrypted-cameras-design.md` (`:29`, `:145`) |
| Public | https://github.com/throwaway96/webos-kernel-drivers (out-of-tree modules for O22 webOS kernels), https://github.com/cfernande1470/webos-wayland (`fbdev_readme_section.md`: another LG set, same fb layout) |

## 1. What the chip contains

```
video     inputs (HDMI A-D, ADC, CVD) ─ input mux ─┐
          decoders (VDEC 1-4) ─ VDO ────────────────┤
              main path:  HDR ─ ISC ─ NR, DI ─ DDR ─ OSC ─ PQ ─ MEMC ─┐
              sub0 path:  ISC ─ DDR ─ OSC ─────────────────────────────┴─ video compositor ─┐
graphics  CMA buffer (linear or AFBC 32x8) ─ AFBC0, MIF0 ─ OSD0 layer ─ scaler ─ SHP ─ mixers ─┼─ OSD blend ─ post blocks ─ OVI ─ panel
          cursor layer (256x256) ─────────────────────────────────────────────────────────────┘
post blocks: gamma (includes the OSD), white balance, OLED compensation, orbit crop
ISC, OSC: input and output scaler. Simplified from LG's diagram; only the main path carries PQ and MEMC.
```

Source: LG's scaler diagram (`Scaler_IP_Block_Diagram_n.PNG`), the pattern probe points in `v4l2-ext-vsc.h:731-732`, and the gamma note "must be set including the OSD" (`v4l2-ext-pq.h:4083`). The sub0 path has no PQ or MEMC box. Picture quality and MEMC therefore act on the video windows and not on the UI (agrees with `videoplane-saver-result.md`: the plane reads 0.93 of its clip, the UI plane 1.01-1.03).

### 1.1 Blocks, owners, documentation

| Block | Where | Who programs it | Documentation in the GPL tree | LG's door |
|---|---|---|---|---|
| OSD0 layer with AFBC0, MIF0, SHP0 | `0xCC480300`, `0xCC480800`, `0xCC480C00`, `0xCC481C00` | `lg1k` per flip (`hdr0,1,3-6`, AFBC pointers, alpha, colour key); PQE firmware (output size, crop, orbit, SHP size, clock gates, workarounds) | bit fields: `osd_reg_o22.h` 294 structs, AFBC 65, TOP 59, SHP 7 | DRM plane 32, `/dev/fb0`, `/proc/lg/fbdev` |
| OSD0_1 with AFBC1, MIF1, SHP1 | `0xCC480400`, `0xCC482700`, `0xCC482800`, `0xCC482300` | `lg1k`, only in 4K-OSD mode (`support_4kosd 0` here); yet its header already reads 1920×2160 at x 1920 with the 3860×2170 scaler output, layer off (TV E13) | same headers | none |
| Cursor layer (`osd4_*`) | `0xCC480600` | `lg1k` | headers | DRM plane 36, `fb3` |
| fb1 / "OSD1" | alias of OSD0 | `lg1k` swaps ownership | none | DRM plane 34 |
| Mixers S.OSD, B.OSD0, B.OSD1; HDR-UI | `0xCC480180-0xCC4802E4`; `0xCC480700` | `lg1k` init (mux fixed `0x76543210`); firmware | headers (three rectangular holes each with 8-bit alpha and a per-input select, white balance, LED and object alpha; HDR-UI: three LUTs and a CSC) | `pathctrl` |
| 2D blitter | `/dev/lg/gfx0` | `lg1k` gfx | `gfx_reg_o22.h` 112 structs; a second header marked "reference only, DO NOT use" | 0666 node, `/proc/lg/gfx` |
| Video windows: input mux, VDO, HDR, ISC/OSC scalers (main, sub0), NR/DI, super-resolution, MEMC | `0xCC300000-0xCCCC2B98` | DE MCU firmware through IPC; kernel DE, VSC, PE drivers | names and bases for 204 blocks; PE registers 2,860 (names, some comments); DE CVI, CVD, CTR, IPC, IPP, ATP, VBI headers | V4L2 ext (VSC, PQ, VT, GPSC), `/dev/lg/de0`, `/proc/lg/de` |
| Video to memory: VT capture, GP scaler | `/dev/video60`, `/dev/video70`, `/dev/video27` | DE firmware (`PQE_TASK_Ipp4GpsManager`) | `v4l2-ext-vt.h`, `v4l2-ext-gpsc.h`, rst documents | `libvtcapture`, LG's compositor (GAV, VTG) |
| Back end: FRC, LED, OLED, WCG, PWM; orbit, LSR/GSR, OSD gain | `0xCC400000-0xCC403F00`, `0xCC4C0000`, `0xCC9C0000` | BE firmware (`MCU_BE_IO_*`) | `be_reg_dpe_o22a0.h` 739 registers, 58 PWM, IPC structs | `/dev/lg/be0`, V4L2 VBE |
| Panel link (OVI, Vx1, TCON) | `ovi_reg_o22a0.h` | kernel `ovi` | 70 registers and macros | `/dev/lg/ovi0` (0666) |

Windows: two real scaler windows (main, sub0). `LX_DE_WIN_ID_T` has five ids, but window 2 is the encoder tap and 3 and 4 are firmware placeholders; O22 maps only two (`include/de_kapi.h:462-473`, `chip/o22/de/de_reg_o22.c:140`, `:1685-1780`). LG's resource table on the set: `MAIN_SCALER 1`, `SUB_SCALER 1`, `GP_SCALER 4`, `VTG 1`, `VT_CAPTURE 1`, `VDEC 4`, `DM 3`, `IMG_DEC 1` (TV). The `/proc/lg/de/win0..3` nodes exist for all four ids. The sub window decodes at most 1920×1088 at 30 fps, the main 4096×2304 at 60 (H.265 at 120), and `videooutputd` lists exactly two sinks (`hw-planes-result.md` § Video windows).

Firmware: three images run on this set (regions `g_fw_mcu0` 4 MB, `g_fw_mcu1` 4 MB, `g_fw_mcu2` 13 MB; mcu3 and mcu4 have none; TV `/proc/lg/de/status`). The kernel embeds the images as C arrays and downloads them at boot (`chip/o22/de/de_fw_o22.c:60-130`): 1.8 MB, 0.57 MB, 4.5 MB and two of 9 KB. Their strings are symbol-rich (`PQE_TASK_*`, `MCU_DE_IO_SetWinInfo`, `MCU_DE_IO_SetOrbit`, `MCU_BE_IO_SET_GSR`, `PQE_CMD_*`, `OSD VSP WA`, `[CG] OSD`, `[OSD] Off:%d, OsdIn:%dx%d, Crop:%dx%d(%d,%d), Pos:(%d,%d)`) and show that the MCU0 firmware reads and rewrites OSD geometry; no source exists (FW).

### 1.2 Left unused or hidden by LG's drivers

Present in the headers and the silicon, not written by `lg1k` or not offered to user space:

- **OSD video-signal processor (VSP).** Contrast, saturation, offset, grayscale and RGB-inverse registers (`osd0_vsp_bosd_ctrl_0/1`, `osd_reg_o22.h:1537-1560`). Live: enabled at neutral values, `0xCC480360 = 0x02000001` (contrast 0x200, centre 0) and `0xCC480364 = 0x02008000` (saturation 0x80, offset 0x200) (TV `scanout-hw-02.txt:302-303`). `lg1k` never writes them (SRC grep over `chip/o22`, `core`), so the firmware or the boot code did. A hardware UI dim or grayscale without composition would live here.
- **Mixer holes.** Each mixer (S.OSD, B.OSD0, B.OSD1) has three rectangular holes (x, y, w, h, 8-bit alpha, enable, reverse) and a per-input select (`ctrl_*_mixer_hole` and `mux_a..f`, `osd_reg_o22.h:548-625` and the same layout in the two B.OSD blocks). All are disabled live (TV: the B.OSD1 registers `0xCC4802C8-0xCC4802DC` and the other two mixers read 0). `lg1k` never programs them. Semantics are unwritten [INFERENCE: a rectangle where the OSD input is cut to the hole's alpha, so a hardware video hole or a fade]. HwPlanes found the same registers (`hw-planes-result.md` § Video windows).
- **OSD scaler filter select** (`osd_ctrl_scaler`: bilinear, vertical cubic, phase offsets): live 0, the hardware default (TV `:0xCC480310`).
- **TOP AFBC path registers** (`ctrl_fbdc0`, `ctrl_fbdc1`: `reg_osd{0,1}_1x2_bypass_en`, `reg_osd{0,1}_1x2_sel_id`, `reg_fbdc{0,1}_ctrl_path`, `_comp`, `_alpha`): `lg1k` never writes them (SRC grep); both read `0x000ff111` live (TV `scanout-hw-02.txt`). The 1×2 fields look like the splitter of the 4K two-reader path [INFERENCE from the names]; nothing documents them.
- **Sharpener gain**: only as a proc knob (§ 3.2); live `ti_gain 0x10` (TV `chip/diag`).
- **Colour key**: reachable only through the fb ioctl path.
- **Local alpha, LSR/GSR (sticky-image reduction, OLED panel care), status monitors** (`osd_status_mon_*`, `osd0_status_alpha_50x50`, picture and mixer-line counters), **AFBC stream-error flags**, **HDR-UI LUTs**, **vertical mirror**, **4K OSD** (a boot option).
- **Public header features that are stubs on O22**: z-order list, video mixer window, pivot (rotation), 3D, CSC and endian ioctls (`core/fbdev/fbdev_drv.c:4462-4491`, `chip/o22/fbdev/fbdev_drv_o22.c:843-870`).

Hardware limits, not driver limits: one blended graphics layer at 2K; the AFBC decoder has no layout, block-size or YTR registers (`osd_afbc_reg_o22.h`: control, addresses, region, width, endian, error status), so it reads one fixed 32x8 layout (`…62`) and LG's client layout `…151` cannot be shown; the blitter reads linear surfaces only.

### 1.3 Who writes OSD0 today

| Writer | What | Evidence |
|---|---|---|
| `lg1k` (kernel) | at every flip: `osd_ctrl_hdr0` (position, colour key), `hdr1` (memory size), `hdr3-6` (pitch, alpha, format, pointers), AFBC pointers and region; enable and fb0/fb1 ownership | `fbdev_hw_o22_cmn.c:2601-2822`, `:3120-3270` |
| PQE firmware (MCU0) | output size `hdr2`, SHP size, crop and orbit offsets, OSD init (SR, orbit), clock-gate bits, size workarounds; told about every OSD enable through `NotifyPQEDriver` and `InformFBDEVStatus` | `fbdev_hw_o22_cmn.c:2702-2790`, `:519-560`; TV dmesg (`scanout-live-01.txt:7-27`); FW strings |
| LG user space | `pqcontroller`, `panelcontroller`, `eplmanager` through V4L2 VBE and PQ controls (OSD gain, LSR/GSR, orbit; the gamma control is documented as set every 30 ms) | `v4l2-ext-vbe.h:777-3345`, `v4l2-ext-pq.h:4083`; `hardware-census.md` holders |
| LumaTV today | DRM atomic commits; the 120 Hz mode writes `fbdev_vsync_cnt_on_flip`; the helper writes `fr_rate` | `compositor/crates/lumatv-compositor/src/{kms,refresh.rs}`, `framerate-result.md` |

Live state of OSD0 (TV `scanout-hw-02.txt`, LumaTV's compositor on screen, XR24 pages): input 1920×1080 (`hdr1 0x07800438`); output 3860×2170 (`hdr2 0x0f14087a`, line 288); crop 3840×2160 at (10, 5) of that canvas (`crop_ctrl_1/2 0x000a0f00`, `0x00050870`, lines 314-315); global alpha 0xff enabled (`hdr3 0xeeff03c0`, the XR24 rule of `fbdev_drm.c:319-328`); `chip/diag`: `hw_wa.pqwfw_osd0_out_ctrl 1`, `hw_wa_flag 0x5` (bit 0: cursor-width workaround on the blend path; bit 2: the PQE firmware owns OSD0's output window; `chip/o22/fbdev/fbdev_hw_o22.h:483-486`), `osd_sr.sw_status 1`, `support_4kosd 0`, `vsync_clk 120Hz`.

The 3860×2170 canvas with a 20×10 margin matches LG's OLED orbit arrangement [INFERENCE]: LG's VSC orbit document gives an "overscan" example that scales a 1908×1074 crop to 3860×2170 (`v4l2-ext-vsc.h:2090-2165`), and LG's Screen Shift is on (`hearth-update-result.md:30`). The measured scale is 2.0104 horizontally and 2.0093 vertically, not exactly 2. The same document's "just scan" orbit scales the OSD to exactly 3840×2160; which arrangement applies is `videooutputd`'s decision and panel-care policy. Supporting evidence: in the OSD0_1 probe run (E13, LG's compositor) the same canvas is live and the crop offset moves one pixel between two dumps seconds apart, (3, 2) then (2, 3), inside the 0..20 × 0..10 margin (`tools/regdecode.py diff`, with the noise pair excluding the registers that tick by themselves); LumaTV's capture reads (10, 5), the middle.

## 2. Does the GPL source document the register maps?

| Area | Coverage | Quality |
|---|---|---|
| OSD, AFBC, TOP, SHP | complete bit fields | The SHP and GFX headers carry LG's generator footer (`O22_PQE_OSD_SHP_reg_man.csv` 2017-06-09, `o22_gfx_register_manual.csv` 2021-03-15), so they come from LG's register manuals. Names, almost no descriptions. Base addresses in comments are stale (`0xC901xxxx`); the live dump (`0xCC48xxxx`) is right. The master struct `OSD_O22_REG_T` (`osd_reg_o22.h:3364-3779`) places blocks at +0x000 status and common, +0x080/+0x100/+0x180 mixers, +0x200..+0x500 layers (from `0xCC480100`), HDR-UI at `0xCC480700`. |
| GFX blitter | complete | 112 structs plus a manual-style header. |
| Picture engine (`chip/o22/pe/reg`) | about 2,860 registers, 10,500 fields | Names; comments in some files (`pe_osd_reg_o22a0.h` has them per field). |
| Display engine (`chip/o22/de`) | CVI, CVD, CTR, IPC, IPP, ATP, VBI registers | Scaler, window and VDO logic is in the firmware; the kernel sends IPC commands (`core/de/mcu/de_ipc_def.h:103-335`: window, source, zlist, orbit, GPS, HFR, PQL, DIMM). |
| Back end | 739 + 58 registers, firmware IPC structs | Algorithms in firmware. |
| Whole-chip map | 204 `APB_*` bases | `pe_reg_def_o22.h:50-245`, the PQE firmware's own list, disabled by `#if 0`. Groups: OSD (`0xCC480xxx`), colour correction beside it (`CC` `0xCC481000`; `pe_ccm_hw_o22.c` calls it "color correction module"), input and scalers (`0xCC8C…`, `0xCCC4…`, `0xCC50…`), VDO (`0xCCC8…`), NR and neural (`0xCC80…`, `0xCCC0…`, `0xCC88…`), HDR (`0xCCCC…`), FRC (`0xCC9C…`, `0xCC30…`, `0xCC4C…`), LED and OLED (`0xCC40…`, OLED `0xCC403000`), MCU (`0xCC08…`). |
| Firmware | binary only | Five arrays plus `led_fw_o22a0.h`. |
| Prose documentation | LG's V4L2 API manuals | `include/webosext/…/renderer/*.rst`: the nearest thing to a datasheet. |

`tools/regdecode.py` joins a `regdump` (or `devmem` lines) with these headers offline: `decode` splits every register into fields, `diff A B [--noise N1 N2]` prints the changed fields and drops what changes by itself, `find REGEX` gives absolute addresses. The OSD block is a C struct (`OSD_O22_REG_T`) whose layer registers restart at zero in each 256-byte window, so the tool lays the struct out like a compiler and names registers `OSD0.`, `OSD0_1.`, `OSD1.` (removed in hardware) and `CURSOR.`; the other blocks are flat. It needs LG's tree for the headers (`--kdriver` or `LG_KDRIVER`; nothing from the tree is committed); `tests/test_regdecode.py` pins the addresses this document quotes. Decoded facts it showed: the SHP block's `reg_lti_2d_gain` is 0x10, the `ti_gain` of `chip/diag`, so `osd_sr_gain` writes the register the doc names; the cursor window reads 1280×720 in memory and 1920×1080 out, off.

## 3. Routes and their risks

### 3.1 KMS: what `lg1k` honours and ignores

| Object or property | What the driver does | Basis |
|---|---|---|
| Plane 32 `FB_ID` | Honoured: address plus offset, pitch, width and height as the OSD input window, XRGB/ARGB/XBGR/ABGR, linear or AFBC `…62` | `drm.c:448-540`, `fbdev_drm.c:300-505` |
| Plane 32 `CRTC_X/Y` | Passed on as the OSD output position and written to `osd_ctrl_hdr0` at every flip. Units: panel pixels [INFERENCE: the cursor path multiplies by 2, this path does not] | `fbdev_drm.c:494-495`, `fbdev_hw_o22_cmn.c:3158-3159`, `:2687-2695` |
| Plane 32 `CRTC_W/H`, `SRC_*` | Ignored (the primary's `CRTC_W/H` only set the cursor's coordinate space): no crop, no scaling property | `drm.c:532-536` |
| Alpha, zpos, rotation, blend | None exposed. Global alpha is reset at every flip: XR24/XB24 to 0xff on (the OSD then ignores pixel alpha, so a video hole in an XR24 page is opaque black), AR24/AB24 off (pixel alpha) | `fbdev_drm.c:319-328`; TV `hdr.global_alpha_en 1` with XR24; `hw-planes-result.md` § The holes |
| Plane 34 | Alias of OSD0; its enable is ignored while fb0 is on | `scanout-research.md` § Layers |
| Plane 36 (cursor) | AR24, clamped to 128×128; image scaled ×2 by the blitter when it changes (`detect_cursor_change`); position ×2; hotspot from `fb->hot_x` ("not tested") | `drm.c:586-640`, `fbdev_drm.c:225-395` |
| CRTC `VRR_ENABLED`; `IN_FENCE_FD`, `OUT_FENCE_PTR` | `VRR_ENABLED` is the DRM core's default and has no driver code; fences work | TV `drmplanes-01.txt`; `gfn-444-vrr-result.md` |
| Connector modes | Cosmetic; the CRTC never sets panel timing; one connector, no writeback; `.ioctls = NULL` | `drm.c:298-358`, `:1052` |
| `use_cursor`, `use_overlay`, `use_afbc`, `use_afrc` | Read once at probe; `use_afrc` would add an AFRC modifier the Mali G52 cannot produce | `drm.c:95-99`, `:822-830`, `:996-1003` |

Risk: lowest of all routes. LumaTV is the DRM master while its compositor runs; the worst outcome is a refused commit or one bad frame.

### 3.2 fbdev ioctls, `/proc/lg/fbdev`, module parameters

| Knob | Effect | Survives a DRM flip | Risk |
|---|---|---|---|
| `chip/osd_sr` 0/1 | OSD sharpener path on (RGB to YC, SHP, YC to RGB, dither) or bypassed (`reg_osd_shp_detour_en`) | yes | very low |
| `chip/osd_sr_gain` 0x00-0xff (live 0x10) | `shp_lti_ctrl_00.reg_lti_2d_gain` | yes | very low |
| `chip/test_reconfig W H` | writes OSD0 `hdr2` output size directly | until the firmware rewrites it | high: breaks the orbit canvas |
| `chip/mirror_test` | vertical mirror of the OSD and AFBC (`osd_v_reverse_en`, `flip_mode`) | yes | low, no use |
| `viewctrl`, `pathctrl`, `syncctrl`, `freeze`, `drm_freeze` | layer enable, OSD/video path, vsync use, freeze | n/a | display changes; diagnostics only |
| `glalpha`, `chgres`, `test_output_win` | header fields of the layer context | alpha: no (reset); input size: no (reset); output x/y: set by `CRTC_X/Y`; output w/h: not flushed for OSD0 | dead ends (§ Dead ends) |
| `capture`, `fbmem_hook`, `user_flip_opt` | hooks in the fb `pan_display` path | n/a under DRM | blocking waits |
| `/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip` | UI vblank divider; already used by the 120 Hz mode | n/a | used |
| `fbdev_o22_pqefw_osd0_out_ctrl` (1) | 0 makes `lg1k` flush OSD0's output size and SHP size, which removes the PQE orbit control from OSD0 | n/a | panel care: never without Nitin |
| `fbdev_cursor_x_comp/y_comp`, `fbdev_flip_check`, `fbdev_o22_afbc_version`, `fbdev_use_hw_debug` | compensation and debug | n/a | read-only interest |

Sources: `core/fbdev/fbdev_proc.c:169-201, 471-646`, `chip/o22/fbdev/fbdev_proc_o22.c:94-254`, `fbdev_drv.c:171-216, 393-424`, `fbdev_hw_o22_cmn.c:133-175`; the parameter names are on the set (TV `scanout-live-01.txt:36-66`).

### 3.3 LG's own kernel interfaces

- `/dev/lg/de0` (0666 on the set): about 100 ioctls (`include/de_kapi.h`) for windows, source selection, mute, freeze, HFR, orbit, GP scaler, and `DE_IOR_REG_RD` / `DE_IOW_REG_WR`, which read and write **any physical address** with no filter (`core/de/de_io.c:1157-1204`, `chip/o22/de/de_reg_o22.c:886-984`: `ioremap` under `0xCC000000`, `vmap_phys` elsewhere).
- `/proc/lg/de/*`: `fr_rate` (already used by the helper); `command` (text for the MCU firmware's command line: `ipc log`, `curr`, `pat ipp m0`; the firmware also carries `OSDINIT` and `ORBITNEN` [FW]; the kernel handles `mcususpend` and `ipcdisable` itself); `disp` and `v4l2_vsc_debug_func` (window and source macros); `bwm` (bandwidth monitor); `wdog`; `bin_download` (loads MCU firmware).
- V4L2 ext controls on LG's video nodes (root, group `composit`): VSC (`WIN_REGION`, `ORBIT_WINDOW`, `ORBIT_MOVE`, `PATTERN`), PQ, VBE (`ORBIT`, `LSR`, `GSR`, `OSD_GAIN`, `ALPHA_OSD`), VT and GPSC. These are what `videooutputd`, `pqcontroller` and `panelcontroller` drive.
- `/dev/vo` (0666): raw frames to a DE window: `VO_WRITE_PICTURE` takes physical Y/Cb/Cr addresses, `VO_SET_DE_ID` picks the window, flags for HFR, low latency and secure video (`core/vdec/include/vo_kapi.h`). LG's media pipeline and the NDL path ride on it.
- `/dev/lg/{be0,pe0,ovi0,sys0}`: back end, picture engine (one packet ioctl), panel link, system.

Risk: the services above assume they are the only writer. Panel and picture state can be corrupted until the service restarts; calls into `be0` and `ovi0` can touch panel timing and OLED care. The API is stable across LG's years of drops, so these survive updates better than raw registers.

### 3.4 Raw registers

Channels: `/dev/mem` (devmem reads of `0xCC48xxxx` worked on the set; the defconfig has `# CONFIG_STRICT_DEVMEM is not set`), `DE_IOW_REG_WR`, the firmware command line.

- **Driver fights:** all three OSD writers (§ 1.3). A raw write to a register the firmware also writes is undone or fights, with visible flicker; one the driver rewrites per flip is undone at the next flip.
- **Hang:** an access to an unclocked or unmapped block can wedge the bus. `/proc/iomem` has no entry over `0xCC48xxxx`. The kernel command line carries `wdtlog=dump@1M` and `mmcoops=dump`, so a watchdog reset is likely, but a power unplug is the pessimistic case [INFERENCE: not measured]. Every raw-write test needs Nitin at the set.
- **Panel safety:** the same aperture holds the OLED compensation (`APB_OLED_BASE 0xCC403000`), demura, LED/PWM, orbit and the panel link. A wrong write there is a panel matter, not a picture bug.
- **Updates:** register addresses are silicon-stable; the firmware's use of them is not (firmware 7356 is newer than the drop). LG's update servers are blocked on the set (`settings-coverage.md:455`), so an update happens only when Nitin allows it; every raw result must then be re-read.

### 3.5 A loadable kernel module

- Support: `CONFIG_MODULES=y`, `MODULE_UNLOAD=y`, no `MODVERSIONS` and no `MODULE_SIG*` in `lg1k_defconfig` (SRC); `npu_drv` and the MT7668 drivers are modules on the set (TV, `hardware-census.md`). Public precedent: `throwaway96/webos-kernel-drivers` ships out-of-tree `ch341.ko` and `cp210x.ko` for `aarch64-5.4.96-o22`, loaded with `insmod` from `/home/root` and at boot from the Homebrew Channel `init.d`, built from LG's open-source kernel and toolchain (PUB). No rootfs write is needed.
- Needs: LG's tree matching the set (324 against 329 is a gap), the toolchain, the defconfig and `LOCALVERSION` that give vermagic `5.4.268-329.ptl4tv.2 …` (read the exact string from an installed `.ko`; the shipped config is not on hand and `/proc/config.gz` is absent). The display code is mostly not exported: only `osd_drm_flip_image`, `osd_drm_enable`, `osd_drm_set_cursor_region`, `osd_drm_register_vsync_handler`, `osd_drm_initialize`, `osd_drm_check_mode`, `FBDEV_OpenDRMContext`, `FBDEV_CloseDRMContext` are (`fbdev_drm.c`, `fbdev_drv.c`); anything else needs kprobes (`CONFIG_KPROBES=y`) or raw access.
- What it would buy that user space cannot: changing the flip path (a persistent global alpha or dim, the VSP registers each frame, plane 34 mapped to OSD0_1). What it risks: a kernel panic at boot if loaded from `init.d`, a vermagic break at every kernel update, and persistent AI-authored code in the kernel.
- Verdict: parked. Only a measured need that survives the cheaper wins justifies it, and the decision is Nitin's.

### 3.6 Observation (read-mostly)

- `/proc/lg/fbdev/regdump` (OSD, TOP, AFBC, SHP, MIF), `chip/diag`, `osdinfo`, `chip/check_stuck` (reads the OSD status monitor 1,000 times at 100 µs), the `fbdev` and `be` NOTI lines in `dmesg`, `/sys/kernel/debug/dri/0/state`.
- `/proc/logm/*` per-module log masks, root (`core/logm/logm_proc.c:445-539`); `/proc/lg/de/bwm` (bandwidth monitor).
- tracefs works on the set with `drm`, `sched`, `irq`, `mali`, `cma` events and `stacktrace` triggers (REPO `docs/research/perf/capture.sh`). The defconfig has `CONFIG_KPROBES=y`, `BPF_SYSCALL`, `PERF_EVENTS`, `DYNAMIC_DEBUG`; `/proc/kallsyms` names were read on the set (`gfn-input-result.md`). Whether `kprobe_events` exists is not known. If it does, `FBDEV_O22_CMN_WriteHdrRegs` and `osd_drm_flip_image` can be traced by name, which shows every header write LG's code makes without loading anything.

### 3.7 Risk matrix

| Route | Fights | Worst realistic outcome | Panel | After an update | Principle 0 |
|---|---|---|---|---|---|
| KMS | nobody while LumaTV is DRM master | refused commit, one bad frame | none | stable API | none |
| fbdev proc, module parameters | `lg1k` (per flip), PQE firmware | UI glitch; `pqefw_osd0_out_ctrl=0` drops LG's orbit for OSD0 | only that parameter | names may move | root writes as the supervisor already does; panel-care parameters: Nitin |
| LG ioctls, V4L2 | `pqcontroller`, `videooutputd`, `panelcontroller` | picture or panel-care state wrong until a restart | high for BE and OVI | stable | panel-affecting calls: Nitin |
| Raw registers | all three OSD writers | bus hang, power cycle; wrong-block write on panel blocks | high | addresses stable, firmware semantics not | Nitin; unreachable from AI-facing code |
| Kernel module | everything it hooks | panic, boot loop if loaded at boot | high | vermagic breaks each kernel | Nitin; persistent kernel code is standing power |
| Observation | none | trace overhead | none | tracefs names may differ | none (reads) |

## 4. Ranked wins (value against risk)

| # | Win | Value | Risk | TV round |
|---|---|---|---|---|
| 1 | Observation kit | Answers open questions 1-6 and de-risks every item below | none (reads; at most tracefs) | yes, read-only |
| 2 | Orbit through the KMS plane offset | OLED care and direct scan-out stop excluding each other | low | yes |
| 3 | Hardware cursor plane for the pointer (HwPlanes has the policy and tests; the KMS side is next) | The pointer stops forcing composed frames and repaints | low | yes |
| 4 | Primary buffer smaller than the mode | Cheaper composed frames; path to direct-scanned reduced-resolution frames | low to medium | yes |
| 5 | OSD sharpening knobs | UI crispness at ×2, free | very low | yes (eyes) |
| 6 | Video tiles through the 4 graphic scalers | Up to four live hardware-decoded tiles (camera wall) instead of stills | study low; build high | yes, later |
| 7 | OSD VSP and mixer-hole registers | UI dim or grayscale, hardware video holes, with no composition | medium (writers unknown) | yes, Nitin at the set |
| 8 | OSD0_1 second layer | A hardware overlay (`scanout-research.md` design) | medium (hang) | probe ran once, no layer seen; two follow-ups left |
| 9 | Kernel module, raw writes | none that the above does not give | high | Nitin's decision |

### 4.1 Orbit through the plane offset (win 2)

Today `oledcare` draws Home offset, so Home is composed (`geometry` refusal); the round-17 gate counted 327 such frames of 22,946 with the care on (`compositor-gate9-result.md:887-890`; the system layer caused most of the rest), and a care that is on all day never lets Home go direct. The primary plane's `CRTC_X/Y` already reaches the header's position fields (`hdr0`, 12 bits each, flushed every flip). A shifted Home can stay a direct buffer: the plane commit carries `(dx, dy)` and the strips it exposes are the strips the software orbit shows (3 px at two edges).

Unknowns: the units (panel pixels or UI pixels), whether the position is applied before the firmware's crop (a shift inside the 20×10 margin would then show no gap), the interplay with LG's Screen Shift (on; E13 shows the firmware moving the crop offset, `OSD0.osd_crop_ctrl_1` and `_2`, by one pixel between two dumps seconds apart, so a plane offset may add to it or fight it), and only non-negative values (`crtc_x` is passed as unsigned). Check: one atomic commit with `CRTC_X` 1, 2, 4, 8 over the `x2cli --pattern` picture; photograph the panel edges; read `0xCC480320` and `0xCC480390-0xCC480394` before and after (`tools/regdecode.py diff`); watch ten seconds for a firmware rewrite. Offline part: let `Plan::sole_candidate` accept an offset when the buffer covers the output and pass it to the commit; host-test against the fake KMS.

### 4.2 Cursor plane (win 3)

The pointer is composed (`NotSole::Cursor`, `scene.rs:264-358`) and refuses scan-out. Plane 36 takes one AR24 image up to 128×128; the driver scales it ×2 when the framebuffer changes (a blitter call under the device lock the panel-care daemons share) and moves it with a header write. The hotspot comes only from `DRM_IOCTL_MODE_CURSOR2`; an atomic plane update has none and a negative `crtc_x` wraps; the legacy cursor ioctls take the kernel's unsynchronised path while an atomic commit waits for the flip in flight, so HwPlanes' design uses the legacy ioctls on a thread of its own (`hw-planes-result.md` § The cursor plane, `src/pointer.rs`). LG draws its own pointer this way (`render/cursor.rs` header). Check: the sprite shows over Home and over a direct frame, follows at 120 Hz without a repaint, the flip counter of `fb3` rises, no stall in `/proc/lg/gfx/status`.

### 4.3 Smaller primary buffer (win 4)

The driver sets the OSD input window from the framebuffer's own size and leaves the output to the firmware, so a 1280×720 or 1440×810 XR24 buffer on plane 32 makes the OSD scaler magnify it (about ×3.0 or ×2.7 into the 3860-wide canvas). LG builds board options for 4K, 2K and HD graphics on this driver (`fbdev_drv_o22.c:225-300`, `LX_BOARD_OPT_*_GRAPHIC`; HD means a 1280×720 UI on a 4K panel), and the firmware tracks the OSD input size (`OsdIn`, `OSD lineCnt WA`, `OSD VSP WA`), so the path looks designed for it [INFERENCE for this board at run time]. Check: a `TEST_ONLY` commit, then a real one with the pattern; read `hdr1` (memory size), the scaler status (`osd0_info_scaler`, `osd0_out_buf_status`) and `dmesg`; photograph; commit 1080p again. Value is bounded: the compositor's 4.1 ms pass shrinks with its output, but the shell's own render does not unless the shell renders smaller too; a direct-scanned smaller shell frame removes the pass entirely. Text at ×3 is soft, so this fits animation bursts, not a still Home.

### 4.4 Sharpening knobs (win 5)

`echo 0 > /proc/lg/fbdev/chip/osd_sr`, then `echo 1`, and `echo 0x40 > …/osd_sr_gain` against the live `0x10`. Photograph Home text at the three settings (LG's capture taps the layer before the scaler and cannot show it), restore `1` and `0x10`. `osd_sr` also routes through an RGB to YC round trip with dither, so a colour or banding change is possible, not only sharpness.

### 4.5 Video tiles (win 6)

LG's GAV compositor plays two or more hardware-decoded videos by taking a scaled copy of each (NV12 or ARGB, at most 1920×1080) through `/dev/video70` into dma-buf textures; "previously, only up to two media could be played due to hardware limitations" (`gp-scaler.rst`). The set's resource table has `GP_SCALER 4` and `VTG 1`. The camera design uses stills for the grid, "never one decoder a tile", because decoders are the scarce part (`scrypted-cameras-design.md:29`); four graphic scalers change that count. Unmeasured: the cost of sampling such a buffer (`PLAN.md` measured 5-20× for rendering into imported memory, not for sampling), and whether LumaTV's compositor can serve LG's VTG protocol. Study first, build only if sampling is cheap.

### 4.6 OSD VSP and mixer holes (win 7), OSD0_1 (win 8)

VSP and holes: read `0xCC480360-0xCC48036C` (VSP) and the three hole rectangles of each mixer (`0xCC4801C8-0xCC4801DC`, `0xCC480248-0xCC48025C`, `0xCC4802C8-0xCC4802DC`) twice, a second apart, with the screen changing, to see whether anything rewrites them; only then a timed write with a revert: `r_contrast` to 0x100, or one full-screen hole with alpha 0x80 on B.OSD1 (the mixer OSD0 selects). A hole with alpha is the candidate hardware dim (OLED care and direct scan-out together) and a hardware video hole for an opaque page.

OSD0_1: `tools/osd01-probe.sh` ran once under LG's compositor (E13, 2026-10-29 06:45 UTC, six seconds, the owner at the panel). Every write held (`regdump-on.diff`: `OSD0_1.osd_ctrl_main` 0x30080011, header 1920×1080 at x 1920, constant alpha 0x80, `reg_4k_osd_main_ctrl_0` 1), and neither LG's capture (`s2-on.png` equals `s2-pre.png`) nor the panel (reported) showed a second layer. By the probe's own table that means "not an overlay at 2K, or under OSD0"; it names two follow-ups, OSD0 at half alpha (`echo 0 80 > /proc/lg/fbdev/glalpha`) and the 4K mode bit (`0xCC480440` = 0x80000001), and says stop if both show nothing. Not tried under LumaTV's compositor (linear pages, `ctrl_dpath` 0), and TOP's AFBC splitter bits (§ 1.2) were not touched. The prior falls from under 1 in 5 to about 1 in 10; keep it parked. Against: the firmware is told about OSD state through a handshake (`InformFBDEVStatus`), and a second layer is a state it does not know. For: the second reader is powered, its header is already set up as the right half of a 4K UI, and its size fields are 12 bits.

## 5. Rules for any write below the DRM layer

- Reads and standard DRM commits from LumaTV's compositor need no new approval. Each of these waits for Nitin, described by the exact commands: loading any kernel module; any raw register write; any module parameter that changes LG's panel care (`fbdev_o22_pqefw_osd0_out_ctrl`); any call into `be0`, `ovi0` or the VBE controls; firmware updates.
- Nitin at the set for every raw-write test, with a dead-man revert like the one in `scanout-research.md` and a power cycle as the known worst case.
- `/dev/mem`, `/dev/lg/{de0,be0,pe0,ovi0,sys0}` and `/proc/lg/*` writes must never be reachable from AI-facing code (assistant tools, the helper's MCP surface). Several of these nodes are mode 0666 on the set (`hardware-census.md`), so the compositor and shell must not hand paths to them to anything AI-driven.
- Anything persistent in the kernel or at boot is standing power over the display and, through `/dev/mem`, over RAM: one explicit OK per release.

## Dead ends

| Idea | Why it fails | Evidence |
|---|---|---|
| OSD1 / fb1 as a second layer | Alias of OSD0; the enable is ignored while fb0 is on | `fbdev_hw_o22_cmn.c:196-225, 3370-3401`; TV dmesg; `scanout-research.md` |
| z-order list, video-mixer window, pivot, 3D, CSC ioctls | No-ops on O22; the mixer mux is fixed ("DO NOT change") | `fbdev_drv.c:4462-4491`, `fbdev_hw_o22_cmn.c:685-698` |
| `glalpha` or fb `SET_CONFIG` alpha as a standing dim | Reset by every DRM flip; holds only on a static screen | `fbdev_drm.c:319-328` |
| `chgres`, `test_output_win` to resize or place the UI | The input size is reset each flip; OSD0's output size is the firmware's (`hdr2` not flushed); only `test_reconfig` or `pqefw_osd0_out_ctrl=0` change it, and both break LG's orbit; x/y come from `CRTC_X/Y` anyway | `fbdev_hw_o22_cmn.c:2702-2728, 3123-3160` |
| 4K graphics | A board option read from the boot command line (`hwopt`), `support_4kosd 0`, half-width blend path per layer, 4× GPU cost | `fbdev_drv_o22.c:225-300`, `fbdev_hw_o22_cmn.c:811-843` |
| Direct scan-out of LG's client AFBC `…151` | The decoder has no layout or YTR registers | `osd_afbc_reg_o22.h` |
| Blitter as the compositor | Linear only, device lock, synchronous, no fence | `scanout-research.md` § The 2D blitter |
| `capture`, `fbmem_hook`, `user_flip_opt` | They sit in the fb `pan_display` path, not the DRM flip; the first two wait on a mutex with no timeout | `fbdev_drv.c:842-1015` |
| DRM VRR, writeback, async flips, AFRC at run time | `VRR_ENABLED` is core default; no writeback connector; async cap 0; `use_afrc` is a probe-time parameter and the G52 has no AFRC encoder | TV `drmplanes-01.txt`; `drm.c:822-830` |
| Direct picture-engine or PQ writes | LG documents the gamma control as set every 30 ms by `pqcontroller`; panel care shares the blocks; the settings service already exposes every user-facing control | `v4l2-ext-pq.h:4083`; `settings-coverage.md` |
| DE firmware download, firmware command line | The whole picture pipeline, no source; commands such as `mcususpend` act at once | `core/de/de_proc.c:903-947`, FW strings |
| A second video window as a UI layer | Opaque, below the OSD, no PQ or MEMC on the sub path, content must be video frames | LG diagram; `scanout-research.md` |
| GPU sampling the video plane | Not possible; a GPSC or VT copy is the only way | `videoplane-saver-result.md` |
| Overlay via `/dev/vo` raw frames | Needs physical addresses and LG's pipeline for the window; no gain over NDL for LumaTV's content | `vo_kapi.h` |

## Open questions (ranked)

1. **Can a module load, and with what vermagic?** Read: `/proc/version`, `/proc/modules`, an installed `.ko`'s vermagic and signature fields, `/proc/sys/kernel/modules_disabled`, `ls /lib/modules/$(uname -r)`. Decides § 3.5.
2. **Are kprobes available?** `ls /sys/kernel/debug/tracing/kprobe_events`; `grep -E ' (osd_drm_flip_image|FBDEV_O22_CMN_WriteHdrRegs|FBDEV_O22_CMN_SetOSDEnable)$' /proc/kallsyms`.
3. **What do `CRTC_X/Y` do on the panel** (units, gaps, firmware rewrite, Screen Shift)? § 4.1.
4. **Does the cursor plane work under LumaTV's DRM master**, and does the pointer stay put through a direct frame? § 4.2.
5. **Does the OSD accept a 1280×720 or 1440×810 buffer** under the firmware, and what is the largest magnification? § 4.3.
6. **Who rewrites OSD0's VSP, crop and mixer-hole registers, and how often?** Three `regdump` reads one second apart with the screen changing.
7. **Do the OSD status monitor and the AFBC error flags give a camera-free verdict?** `osd_status_mon_cfg/pos/data` and the AFBC error status (`error_detection_flag`, `sync_error`, `y_div16_error`) could say whether a buffer was decoded right, which would settle `scanout-research.md` check (b) case b without photographs [INFERENCE: field names only]. Read first; the monitor takes two config writes.
8. OSD0_1 under the two follow-ups its probe table names (OSD0 at half alpha, the 4K mode bit) and under LumaTV's compositor (linear pages); the first run (E13, LG's compositor) showed no layer.
9. GPSC sampling cost and the VTG protocol (§ 4.5).
10. Whether the exact-×2 "just scan" orbit applies to the OSD and whether it can be chosen without weakening panel care (Nitin's policy).
11. DDR bandwidth per master (`/proc/lg/de/bwm`): what AFBC output would save at 120 Hz.
12. Whether a bus hang is recovered by the watchdog. Not to be tested.

## TV checks

Read-only slot, no display change (answers 1, 2, 6, 7 and parts of 11):

```sh
cat /proc/version /proc/sys/kernel/modules_disabled; head -30 /proc/modules; ls /lib/modules/$(uname -r) | head
ls /sys/kernel/debug/tracing/kprobe_events /sys/kernel/debug/tracing/dynamic_events
grep -E ' (osd_drm_flip_image|FBDEV_O22_CMN_WriteHdrRegs|FBDEV_O22_CMN_SetOSDEnable)$' /proc/kallsyms
for p in /sys/module/lg1k/parameters/* /sys/module/lg1kdrm/parameters/*; do echo "$p=$(cat $p)"; done
cat /proc/lg/fbdev/chip/diag; ls /proc/logm; cat /proc/logm/status | head -40
ls -l /dev/mem /dev/vo /dev/lg /dev/video60 /dev/video70; head -20 /proc/iomem
for i in 1 2 3; do cat /proc/lg/fbdev/regdump > /tmp/rd$i; sleep 1; done; diff /tmp/rd1 /tmp/rd3 | head -60
```

One display slot with LumaTV's compositor, Nitin at the set (wins 2-5), in this order: cursor plane; `CRTC_X/Y`; the 720p buffer; the sharpening knobs. Each step is one commit and one revert; `python3 compositor/tools/tv_kms.py restore` ends an interrupted run (`scanout-research.md` § TV check list, b). Raw-register tests (wins 7 and 8) only after these.

## Actionable items

1. **Read-only observation slot.** Run the TV checks above (kernel and module facts, kprobe and kallsyms availability, parameters, `regdump` three times, `/proc/logm`, node modes). Value: high, answers open questions 1, 2, 6, 7 and sets up everything else. Risk: none. Needs TV round: yes (no display change).
2. **Orbit through the plane offset.** Let the direct plan carry a plane offset and drive `oledcare` from it; host tests first, then one display slot to see units, edge strips and the firmware. Value: high (OLED care and direct scan-out together). Risk: low. Needs TV round: yes (photo).
3. **Hardware cursor plane for the pointer.** Continue HwPlanes' pointer work (`docs/research/hw-planes-result.md`: policy and tests done, KMS side next): plane 36, AR24 up to 128×128, hotspot through the legacy cursor ioctl, behind its switch. Value: medium to high (the pointer no longer forces composed frames). Risk: low. Needs TV round: yes.
4. **Smaller primary buffer.** One commit of a 1280×720 or 1440×810 XR24 buffer on plane 32, scaled by the OSD scaler. Value: medium (cheaper composed frames; direct reduced-resolution frames later). Risk: low to medium (softness, firmware workarounds). Needs TV round: yes.
5. **OSD sharpening knobs.** `/proc/lg/fbdev/chip/osd_sr` and `osd_sr_gain` photographed at three settings, restored to 1 and 0x10. Value: low to medium (free UI crispness). Risk: very low. Needs TV round: yes (Nitin's eyes).
6. **Offline register tool (done).** `tools/regdecode.py decode|diff|find` names `regdump` registers by plane window and diffs two dumps with a noise pair; tests in `tests/test_regdecode.py`. Value: speeds every later read. Risk: none. Needs TV round: no.
7. **Video tiles through the four graphic scalers.** Design study and one measurement of sampling a GPSC buffer; build only if cheap. Value: medium (live camera tiles). Risk: low for the study. Needs TV round: yes (measurement).
8. **OSD VSP and mixer-hole raw-register tests.** Read-only first (open question 6), then a timed `r_contrast` or full-screen hole write. The OSD0_1 probe already ran once without a visible layer (E13); its two follow-ups (OSD0 at half alpha, the 4K mode bit) are optional, high value if a layer ever shows, prior now about 1 in 10. Value: medium (hardware dim, hardware video hole). Risk: medium (hang, firmware fight). Needs TV round: yes, Nitin at the set.
9. **Kernel module and persistent register code stay parked.** Revisit only for a measured need the items above cannot meet; Nitin decides and approves each release. Value: n/a now. Risk: high. Needs TV round: no (decision first).
