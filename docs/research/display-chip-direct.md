# Driving the display chip directly

Research only, 2026-10-29. No TV access and no hardware write. The question: can LumaTV use the C4's display hardware (OSD layers, video windows, scalers, AFBC, picture-quality blocks) directly, past LG's driver limits, and what is worth doing first. The next worker starts from § Open questions, § Dead ends and the **Actionable items** at the end.

## Verdict

- The display chip is LG's own: kernel command line `chip=O22A3 pqe.tool=C4 pqe.year=24`, device-tree model "LG Electronics, DTV SoC LG1213" (TV). MediaTek is only the MT7668 Wi-Fi/Bluetooth radio, so MediaTek display sources do not apply, and LG publishes no datasheet. LG's GPL tree is the only register documentation.
- That tree documents the **graphics side to bit-field level** (OSD layers, AFBC readers, OSD sharpener, 2D blitter) and gives **names and addresses** of 204 video-pipeline blocks and about 2,860 picture-engine registers. It does not say how the scalers, picture quality, MEMC, orbit and HDR work: that runs as closed firmware on the display engine's MCUs, shipped in the tree only as byte arrays.
- The OSD registers already have three writers: the `lg1k` driver at every flip, the PQE firmware at every frame, and LG's picture and panel services. A raw write fights at least one of them, so raw registers are the last resort. A kernel module is feasible in principle (public precedent below), is Nitin's decision, and buys little that user space cannot do.
- The best wins use doors LG already opens: a read-only observation kit, the KMS plane position as hardware orbit, the cursor plane, a smaller primary buffer scaled by the OSD scaler, and the OSD sharpening knobs. Long shots: OSD0_1 as a second layer, video tiles through the four graphic scalers, the OSD's contrast and saturation registers.

## Evidence and sources

Tags: `TV` (read on this set), `REPO` (recorded result or code here), `SRC` (LG's GPL tree), `FW` (strings of the firmware images inside that tree), `PUB` (public page), `[INFERENCE]` (not shown; the check that would show it is named).

`SRC` root: `/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/` (tarball `linux-and-kdriver_paluma-5-241025-0301.tar.gz`, LG's public drop, https://opensource.lge.com/product). Paths below are relative to its `kdriver/` unless they start with `linux-rockhopper/`. Its `build.sh` defaults to the O24 chip; the C4 is O22 (`chip/o22`).

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
| TV captures | `docs/research/arch/{scanout-hw-02.txt,scanout-live-01.txt,lgproc-fbdev-01.txt,lgproc-gfx-de-01.txt,drmplanes-01.txt,drm-debugfs-01.txt}`, `docs/research/framerate/*/before.txt` (kernel command line), `docs/research/gfn/tv-2026-10-26/umediaserver_resource_config.txt`, `docs/research/hardware-census.md` |
| Earlier results | `scanout-research.md`, `framerate-result.md`, `gfn-444-vrr-result.md`, `ui120-result.md`, `videoplane-saver-result.md`, `oledcare-result.md`, `hearth-update-result.md`, `compositor-gate9-result.md` (`:887-890`), `scrypted-cameras-design.md` (`:145`) |
| Public | https://github.com/throwaway96/webos-kernel-drivers (out-of-tree modules for O22 webOS kernels), https://github.com/cfernande1470/webos-wayland (`fbdev_readme_section.md`: another LG set, same fb layout) |

## 1. What the chip contains

```
video   HDMI A-D / ADC / CVD ─ input mux ─┐
        VDEC 1-4 ─ VDO ───────────────────┴─ main:  HDR ─ ISC ─ NR, DI ─ DDR ─ OSC ─ PQ ─ MEMC ─┐
                                              sub0:  ISC ─ DDR ─ OSC ──────────────────────────────┤
                                                                         video compositor ────────┘
graphics  CMA buffer (linear or AFBC 32x8) ─ AFBC0 / MIF0 ─ OSD0 layer ─ scaler ─ SHP ─ mixers ─┐
          cursor layer (256x256) ──────────────────────────────────────────────────────────────┤
          video compositor + OSD blend ─ post blocks (gamma incl. OSD, white balance, OLED, orbit crop) ─ OVI (Vx1) ─ panel
```

Source: LG's scaler diagram (`Scaler_IP_Block_Diagram_n.PNG`), the pattern probe points in `v4l2-ext-vsc.h:731-732`, and the gamma note "must be set including the OSD" (`v4l2-ext-pq.h:4083`). The sub0 path has no PQ or MEMC box. Picture quality and MEMC therefore act on the video windows and not on the UI (agrees with `videoplane-saver-result.md`: the plane reads 0.93 of its clip, the UI plane 1.01-1.03).

### 1.1 Blocks, owners, documentation

| Block | Where | Who programs it | Documentation in the GPL tree | LG's door |
|---|---|---|---|---|
| OSD0 layer with AFBC0, MIF0, SHP0 | `0xCC480300`, `0xCC480800`, `0xCC480C00`, `0xCC481C00` | `lg1k` per flip (`hdr0,1,3-6`, AFBC pointers, alpha, colour key); PQE firmware (output size, crop, orbit, SHP size, clock gates, workarounds) | bit fields: `osd_reg_o22.h` 294 structs, AFBC 65, TOP 59, SHP 7 | DRM plane 32, `/dev/fb0`, `/proc/lg/fbdev` |
| OSD0_1 with AFBC1, MIF1, SHP1 | `0xCC480400`, `0xCC482700`, `0xCC482800`, `0xCC482300` | `lg1k`, only in 4K-OSD mode (`support_4kosd 0` here) | same headers | none |
| Cursor layer (`osd4_*`) | `0xCC480600` | `lg1k` | headers | DRM plane 36, `fb3` |
| fb1 / "OSD1" | alias of OSD0 | `lg1k` swaps ownership | none | DRM plane 34 |
| Mixers S.OSD, B.OSD0, B.OSD1; HDR-UI | `0xCC480180-0xCC4802E4`; `0xCC480700` | `lg1k` init (mux fixed `0x76543210`); firmware | headers (hole selects, white balance, LED and object alpha; HDR-UI: three LUTs and a CSC) | `pathctrl` |
| 2D blitter | `/dev/lg/gfx0` | `lg1k` gfx | `gfx_reg_o22.h` 112 structs; a second header marked "reference only, DO NOT use" | 0666 node, `/proc/lg/gfx` |
| Video windows: input mux, VDO, HDR, ISC/OSC scalers (main, sub0), NR/DI, super-resolution, MEMC | `0xCC300000-0xCCCC2B98` | DE MCU firmware through IPC; kernel DE, VSC, PE drivers | names and bases for 204 blocks; PE registers 2,860 (names, some comments); DE CVI, CVD, CTR, IPC, IPP, ATP, VBI headers | V4L2 ext (VSC, PQ, VT, GPSC), `/dev/lg/de0`, `/proc/lg/de` |
| Video to memory: VT capture, GP scaler | `/dev/video60`, `/dev/video70`, `/dev/video27` | DE firmware (`PQE_TASK_Ipp4GpsManager`) | `v4l2-ext-vt.h`, `v4l2-ext-gpsc.h`, rst documents | `libvtcapture`, LG's compositor (GAV, VTG) |
| Back end: FRC, LED, OLED, WCG, PWM; orbit, LSR/GSR, OSD gain | `0xCC400000-0xCC403F00`, `0xCC4C0000`, `0xCC9C0000` | BE firmware (`MCU_BE_IO_*`) | `be_reg_dpe_o22a0.h` 739 registers, 58 PWM, IPC structs | `/dev/lg/be0`, V4L2 VBE |
| Panel link (OVI, Vx1, TCON) | `ovi_reg_o22a0.h` | kernel `ovi` | 70 registers and macros | `/dev/lg/ovi0` (0666) |

Windows: two real scaler windows (main, sub0). `LX_DE_WIN_ID_T` has five ids, but window 2 is the encoder tap and 3 and 4 are firmware placeholders; O22 maps only two (`include/de_kapi.h:462-473`, `chip/o22/de/de_reg_o22.c:140`, `:1685-1780`). LG's resource table on the set: `MAIN_SCALER 1`, `SUB_SCALER 1`, `GP_SCALER 4`, `VTG 1`, `VT_CAPTURE 1`, `VDEC 4`, `DM 3`, `IMG_DEC 1` (TV). The `/proc/lg/de/win0..3` nodes exist for all four ids.

Firmware: three images run on this set (regions `g_fw_mcu0` 4 MB, `g_fw_mcu1` 4 MB, `g_fw_mcu2` 13 MB; mcu3 and mcu4 have none; TV `/proc/lg/de/status`). The kernel embeds the images as C arrays and downloads them at boot (`chip/o22/de/de_fw_o22.c:60-130`): 1.8 MB, 0.57 MB, 4.5 MB and two of 9 KB. Their strings are symbol-rich (`PQE_TASK_*`, `MCU_DE_IO_SetWinInfo`, `MCU_DE_IO_SetOrbit`, `MCU_BE_IO_SET_GSR`, `PQE_CMD_*`, `OSD VSP WA`, `[CG] OSD`, `[OSD] Off:%d, OsdIn:%dx%d, Crop:%dx%d(%d,%d), Pos:(%d,%d)`) and show that the MCU0 firmware reads and rewrites OSD geometry; no source exists (FW).

### 1.2 Left unused or hidden by LG's drivers

Present in the headers and the silicon, not written by `lg1k` or not offered to user space:

- **OSD video-signal processor (VSP).** Contrast, saturation, offset, grayscale and RGB-inverse registers (`osd0_vsp_bosd_ctrl_0/1`, `osd_reg_o22.h:1537-1560`). Live: enabled at neutral values, `0xCC480360 = 0x02000001` (contrast 0x200, centre 0) and `0xCC480364 = 0x02008000` (saturation 0x80, offset 0x200) (TV `scanout-hw-02.txt:302-303`). `lg1k` never writes them (SRC grep over `chip/o22`, `core`), so the firmware or the boot code did. A hardware UI dim or grayscale without composition would live here.
- **OSD scaler filter select** (`osd_ctrl_scaler`: bilinear, vertical cubic, phase offsets): live 0, the hardware default (TV `:0xCC480310`).
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
| LG user space | `pqcontroller`, `panelcontroller`, `eplmanager` through V4L2 VBE and PQ controls (OSD gain, LSR/GSR, orbit, gamma every 30 ms) | `v4l2-ext-vbe.h:777-3345`, `v4l2-ext-pq.h:4083`; `hardware-census.md` holders |
| LumaTV today | DRM atomic commits only | `compositor/crates/lumatv-compositor/src/kms/` |

Live state of OSD0 (TV `scanout-hw-02.txt`, LumaTV's compositor on screen, XR24 pages): input 1920×1080 (`hdr1 0x07800438`); output 3860×2170 (`hdr2 0x0f14087a`, line 288); crop 3840×2160 at (10, 5) of that canvas (`crop_ctrl_1/2 0x000a0f00`, `0x00050870`, lines 314-315); global alpha 0xff enabled (`hdr3 0xeeff03c0`, the XR24 rule of `fbdev_drm.c:319-328`); `chip/diag`: `hw_wa.pqwfw_osd0_out_ctrl 1`, `hw_wa_flag 0x5`, `osd_sr.sw_status 1`, `support_4kosd 0`, `vsync_clk 120Hz`.

The 3860×2170 canvas with a 20×10 margin is LG's OLED orbit arrangement ("overscan" case: OSD input 1908×1074 to output 3860×2170 in the example, `v4l2-ext-vsc.h:2090-2165`; LG's Screen Shift is on, `hearth-update-result.md:30`). The UI therefore scales by 2.0104, not exactly 2 [INFERENCE: the cause; the numbers are measured]. The exact-×2 arrangement is the "just scan" orbit of the same document; which one the set uses is `videooutputd`'s decision, and it is panel-care policy.

## 2. Does the GPL source document the register maps?

| Area | Coverage | Quality |
|---|---|---|
| OSD, AFBC, TOP, SHP | complete bit fields | Generated from LG's register manual (`osd_shp_reg_o22.h` footer: `O22_PQE_OSD_SHP_reg_man.csv`). Names, almost no descriptions. Base addresses in comments are stale (`0xC901xxxx`); the live dump (`0xCC48xxxx`) is right. The master struct `OSD_O22_REG_T` (`osd_reg_o22.h:3364-3779`) places blocks at +0x000 status and common, +0x080/+0x100/+0x180 mixers, +0x200..+0x500 layers (from `0xCC480100`), HDR-UI at `0xCC480700`. |
| GFX blitter | complete | 112 structs plus a manual-style header. |
| Picture engine (`chip/o22/pe/reg`) | about 2,860 registers, 10,500 fields | Names; comments in some files (`pe_osd_reg_o22a0.h` has them per field). |
| Display engine (`chip/o22/de`) | CVI, CVD, CTR, IPC, IPP, ATP, VBI registers | Scaler, window and VDO logic is in the firmware; the kernel sends IPC commands (`core/de/mcu/de_ipc_def.h:103-335`: window, source, zlist, orbit, GPS, HFR, PQL, DIMM). |
| Back end | 739 + 58 registers, firmware IPC structs | Algorithms in firmware. |
| Whole-chip map | 204 `APB_*` bases | `pe_reg_def_o22.h:50-245`, the PQE firmware's own list, disabled by `#if 0`. Groups: OSD (`0xCC480xxx`), colour correction beside it (`CC` `0xCC481000`; `pe_ccm_hw_o22.c` calls it "color correction module"), input and scalers (`0xCC8C…`, `0xCCC4…`, `0xCC50…`), VDO (`0xCCC8…`), NR and neural (`0xCC80…`, `0xCCC0…`, `0xCC88…`), HDR (`0xCCCC…`), FRC (`0xCC9C…`, `0xCC30…`, `0xCC4C…`), LED and OLED (`0xCC40…`, OLED `0xCC403000`), MCU (`0xCC08…`). |
| Firmware | binary only | Five arrays plus `led_fw_o22a0.h`. |
| Prose documentation | LG's V4L2 API manuals | `include/webosext/…/renderer/*.rst`: the nearest thing to a datasheet. |

The register headers parse mechanically: each register is a comment `0xOFFSET name ''` followed by a `typedef struct` of bit fields from bit 0 up. Decoding the live `regdump` against `osd_reg_o22.h` takes a short script (none is committed).

## 3. Routes and their risks

### 3.1 KMS: what `lg1k` honours and ignores

| Object or property | What the driver does | Basis |
|---|---|---|
| Plane 32 `FB_ID` | Honoured: address plus offset, pitch, width and height as the OSD input window, XRGB/ARGB/XBGR/ABGR, linear or AFBC `…62` | `drm.c:448-540`, `fbdev_drm.c:300-505` |
| Plane 32 `CRTC_X/Y` | Passed on as the OSD output position and written to `osd_ctrl_hdr0` at every flip. Units: panel pixels [INFERENCE: the cursor path multiplies by 2, this path does not] | `fbdev_drm.c:494-495`, `fbdev_hw_o22_cmn.c:3158-3159`, `:2687-2695` |
| Plane 32 `CRTC_W/H`, `SRC_*` | Ignored (the primary's `CRTC_W/H` only set the cursor's coordinate space): no crop, no scaling property | `drm.c:520-530` |
| Alpha, zpos, rotation, blend | None exposed. Global alpha is reset at every flip: XR24/XB24 to 0xff on, AR24/AB24 off (pixel alpha) | `fbdev_drm.c:319-328`; TV `hdr.global_alpha_en 1` with XR24 |
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

- `/dev/lg/de0` (0666 on the set): 99 ioctls (`include/de_kapi.h`) for windows, source selection, mute, freeze, HFR, orbit, GP scaler, and `DE_IOR_REG_RD` / `DE_IOW_REG_WR`, which read and write **any physical address** with no filter (`core/de/de_io.c:1157-1204`, `chip/o22/de/de_reg_o22.c:886-984`: `ioremap` under `0xCC000000`, `vmap_phys` elsewhere).
- `/proc/lg/de/*`: `fr_rate` (already used by the helper), `command` (forwards text to the firmware's command line: `osdinit`, `orbitnen`, `mcususpend`, `ipc log` [FW]), `disp` and `v4l2_vsc_debug_func` (window and source macros), `bwm` (bandwidth monitor), `wdog`, `bin_download` (loads MCU firmware).
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
| 3 | Hardware cursor plane for the pointer | The pointer stops forcing composed frames and repaints | low | yes |
| 4 | Primary buffer smaller than the mode | Cheaper composed frames; path to direct-scanned reduced-resolution frames | low to medium | yes |
| 5 | OSD sharpening knobs | UI crispness at ×2, free | very low | yes (eyes) |
| 6 | Video tiles through the 4 graphic scalers | Up to four live hardware-decoded tiles (camera wall) instead of stills | study low; build high | yes, later |
| 7 | OSD VSP registers | UI dim or grayscale with no composition | medium (writer unknown) | yes, Nitin at the set |
| 8 | OSD0_1 second-layer probe | A hardware overlay (`scanout-research.md` design) | medium (hang) | yes, maintenance window |
| 9 | Kernel module, raw writes | none that the above does not give | high | Nitin's decision |

### 4.1 Orbit through the plane offset (win 2)

Today `oledcare` draws Home offset, so Home is composed (`geometry` refusal); the round-17 gate counted 327 such frames of 22,946 with the care on (`compositor-gate9-result.md:887-890`; the system layer caused most of the rest), and a care that is on all day never lets Home go direct. The primary plane's `CRTC_X/Y` already reaches the header's position fields (`hdr0`, 12 bits each, flushed every flip). A shifted Home can stay a direct buffer: the plane commit carries `(dx, dy)` and the strips it exposes are the strips the software orbit shows (3 px at two edges).

Unknowns: the units (panel pixels or UI pixels), whether the position is applied before the firmware's crop (a shift inside the 20×10 margin would then show no gap), the interplay with LG's Screen Shift (on; it moves the crop offset over the same margin), and only non-negative values (`crtc_x` is passed as unsigned). Check: one atomic commit with `CRTC_X` 1, 2, 4, 8 over the `x2cli --pattern` picture; photograph the panel edges; read `0xCC480320` before and after; watch ten seconds for a firmware rewrite. Offline part: let `Plan::sole_candidate` accept an offset when the buffer covers the output and pass it to the commit; host-test against the fake KMS.

### 4.2 Cursor plane (win 3)

The pointer is composed (`NotSole::Cursor`, `scene.rs:264-358`) and refuses scan-out. Plane 36 takes one AR24 image up to 128×128; the driver scales it ×2 when the framebuffer changes (a blitter call under the device lock the panel-care daemons share) and moves it with a header write. Use hotspot 0 and subtract the hotspot in LumaTV (the driver's hotspot path is "not tested"); upload the image once. LG draws its own pointer this way (`render/cursor.rs` header). Check: the sprite shows over Home and over a direct frame, follows at 120 Hz without a repaint, the flip counter of `fb3` rises, no stall in `/proc/lg/gfx/status`.

### 4.3 Smaller primary buffer (win 4)

The driver sets the OSD input window from the framebuffer's own size and leaves the output to the firmware, so a 1280×720 or 1440×810 XR24 buffer on plane 32 makes the OSD scaler magnify it (×3 or ×2.67). LG builds board options for 4K, 2K and HD graphics on this driver (`fbdev_drv_o22.c`, `LX_BOARD_OPT_*_GRAPHIC`; HD means a 1280×720 UI on a 4K panel), and the firmware tracks the OSD input size (`OsdIn`, `OSD lineCnt WA`, `OSD VSP WA`), so the path is designed for it [INFERENCE for this board at run time]. Check: a `TEST_ONLY` commit, then a real one with the pattern; read `hdr1` (memory size), the scaler status (`osd0_info_scaler`, `osd0_out_buf_status`) and `dmesg`; photograph; commit 1080p again. Value is bounded: the compositor's 4.1 ms pass shrinks with its output, but the shell's own render does not unless the shell renders smaller too; a direct-scanned smaller shell frame removes the pass entirely. Text at ×3 is soft, so this fits animation bursts, not a still Home.

### 4.4 Sharpening knobs (win 5)

`echo 0 > /proc/lg/fbdev/chip/osd_sr`, then `echo 1`, and `echo 0x40 > …/osd_sr_gain` against the live `0x10`. Photograph Home text at the three settings (LG's capture taps the layer before the scaler and cannot show it), restore `1` and `0x10`. `osd_sr` also routes through an RGB to YC round trip with dither, so a colour or banding change is possible, not only sharpness.

### 4.5 Video tiles (win 6)

LG's GAV compositor plays two or more hardware-decoded videos by taking a scaled copy of each (NV12 or ARGB, at most 1920×1080, four instances) through `/dev/video70` into dma-buf textures; "previously only two media could be played" (`gp-scaler.rst`, `video-texture.rst`). The set's resource table has `GP_SCALER 4` and `VTG 1`. The camera design uses stills for the grid because "decoders are the scarce resource" (`scrypted-cameras-design.md:29`); four graphic scalers change that count. Unmeasured: the cost of sampling such a buffer (`PLAN.md` measured 5-20× for rendering into imported memory, not for sampling), and whether LumaTV's compositor can serve LG's VTG protocol. Study first, build only if sampling is cheap.

### 4.6 OSD VSP (win 7) and OSD0_1 (win 8)

VSP: read `0xCC480360-0xCC48036C` twice, a second apart, with the screen changing, to see whether anything rewrites them; only then a timed write of `r_contrast` to 0x100 with a revert. OSD0_1: the probe in `scanout-research.md` § The probe stays as designed and parked; new evidence against: the firmware is told about OSD state through a handshake (`InformFBDEVStatus`), a second layer is a state it does not know; for: the second reader is powered and its size fields are 12 bits.

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
| Direct picture-engine or PQ writes | `pqcontroller` rewrites gamma every 30 ms; panel care shares the blocks; the settings service already exposes every user-facing control | `v4l2-ext-pq.h:4083`; `settings-coverage.md` |
| DE firmware download, firmware command line | The whole picture pipeline, no source; `osdinit`, `orbitnen`, `mcususpend` act at once | `de_proc.c:903-947` |
| A second video window as a UI layer | Opaque, below the OSD, no PQ or MEMC on the sub path, content must be video frames | LG diagram; `scanout-research.md` |
| GPU sampling the video plane | Not possible; a GPSC or VT copy is the only way | `videoplane-saver-result.md` |
| Overlay via `/dev/vo` raw frames | Needs physical addresses and LG's pipeline for the window; no gain over NDL for LumaTV's content | `vo_kapi.h` |

## Open questions (ranked)

1. **Can a module load, and with what vermagic?** Read: `/proc/version`, `/proc/modules`, an installed `.ko`'s vermagic and signature fields, `/proc/sys/kernel/modules_disabled`, `ls /lib/modules/$(uname -r)`. Decides § 3.5.
2. **Are kprobes available?** `ls /sys/kernel/debug/tracing/kprobe_events`; `grep -E ' (osd_drm_flip_image|FBDEV_O22_CMN_WriteHdrRegs|FBDEV_O22_CMN_SetOSDEnable)$' /proc/kallsyms`.
3. **What do `CRTC_X/Y` do on the panel** (units, gaps, firmware rewrite, Screen Shift)? § 4.1.
4. **Does the cursor plane work under LumaTV's DRM master**, and does the pointer stay put through a direct frame? § 4.2.
5. **Does the OSD accept a 1280×720 or 1440×810 buffer** under the firmware, and what is the largest magnification? § 4.3.
6. **Who rewrites OSD0's VSP and crop registers, and how often?** Three `regdump` reads one second apart with the screen changing.
7. **Do the OSD status monitor and the AFBC error flags give a camera-free verdict?** `osd_status_mon_cfg/pos/data` and the AFBC error status (`error_detection_flag`, `sync_error`, `y_div16_error`) could say whether a buffer was decoded right, which would settle `scanout-research.md` check (b) case b without photographs [INFERENCE: field names only]. Read first; the monitor takes two config writes.
8. OSD0_1 blending at 2K (the existing probe).
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

One display slot with LumaTV's compositor, Nitin at the set (wins 2-5), in this order: cursor plane; `CRTC_X/Y`; the 720p buffer; the sharpening knobs. Each ends with `ctl` back to the normal path; `python3 compositor/tools/tv_kms.py restore` ends an interrupted run. Raw-register tests (7, 8) only after these.

## Actionable items

1. **Read-only observation slot.** Run the TV checks above (kernel and module facts, kprobe and kallsyms availability, parameters, `regdump` three times, `/proc/logm`, node modes). Value: high, answers open questions 1, 2, 6, 7 and sets up everything else. Risk: none. Needs TV round: yes (no display change).
2. **Orbit through the plane offset.** Let the direct plan carry a plane offset and drive `oledcare` from it; host tests first, then one display slot to see units, edge strips and the firmware. Value: high (OLED care and direct scan-out together). Risk: low. Needs TV round: yes (photo).
3. **Hardware cursor plane for the pointer.** Plane 36, AR24 up to 128×128, hotspot handled in LumaTV, behind a switch. Value: medium to high (pointer no longer forces composed frames). Risk: low. Needs TV round: yes.
4. **Smaller primary buffer.** One commit of a 1280×720 or 1440×810 XR24 buffer on plane 32, scaled by the OSD scaler. Value: medium (cheaper composed frames; direct reduced-resolution frames later). Risk: low to medium (softness, firmware workarounds). Needs TV round: yes.
5. **OSD sharpening knobs.** `/proc/lg/fbdev/chip/osd_sr` and `osd_sr_gain` photographed at three settings, restored to 1 and 0x10. Value: low to medium (free UI crispness). Risk: very low. Needs TV round: yes (Nitin's eyes).
6. **Offline register tool.** A script that decodes `regdump` against `osd_reg_o22.h` and diffs two dumps. Value: speeds every later read. Risk: none. Needs TV round: no.
7. **Video tiles through the four graphic scalers.** Design study and one measurement of sampling a GPSC buffer; build only if cheap. Value: medium (live camera tiles). Risk: low for the study. Needs TV round: yes (measurement).
8. **OSD VSP and OSD0_1 raw-register tests.** Read-only first (open question 6), then the timed `r_contrast` write and the existing OSD0_1 probe. Value: medium (hardware dim) and high-if-it-works (second layer, prior under 1 in 5). Risk: medium (hang, firmware fight). Needs TV round: yes, Nitin at the set.
9. **Kernel module and persistent register code stay parked.** Revisit only for a measured need the items above cannot meet; Nitin decides and approves each release. Value: n/a now. Risk: high. Needs TV round: no (decision first).
