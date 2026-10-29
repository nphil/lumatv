# Display layers, deep: what the chip gives beyond one OSD

Continues `display-chip-direct.md` (items 1-9) with the read-only observation of 2026-10-29 03:46 EDT
(`docs/research/compositor/layerdeep-2026-10-29/`), LG's GPL tree, two switches built on the host, and the scripts for
Nitin at the set. No TV access this session; nothing here has run on the panel.

## Resume here

Done (2026-10-29, offline): the evidence committed (`76160d6`, serial and MAC redacted), the plane-offset orbit
(`93f6e74`: `--scanout-offset`, `ctl scanout offset on|off`, off by default), the page probe (`5c9dc80`: `ctl pageprobe
WxH SECONDS`), `tools/layerdeep-tv.sh` (the set-side steps with their reverts), this document. Host checks green:
the whole `cargo test -p lumatv-compositor`, `cargo clippy -D
warnings` on the lib, bins and the touched test crates, `cargo check --features ui`. `tests/vwin.rs:332` (HwPlanes2's)
fails clippy on `main` with an unused `mut`; told HwPlanes2.

Next, in order:

1. Nitin at the set, LumaTV's compositor live: `sh /tmp/lumatv/layerdeep-tv.sh` steps `observe`, `orbit`, `pageprobe`,
   `sharp` (§ 6; no raw register write). Each prints its readings and its photo points.
2. The raw-register steps `vsp`, `hole`, `osd01` (§ 6), Nitin at the set, each with its dead-man; `osd01` last and only if
   he wants it (its prior is about 1 in 20).
3. After `orbit` has shown the units: either keep the offset as built (the care's shifts right or down go direct, about
   3 beats in 10 at radius 3) or move the ring's origin so every beat does (§ 5.1, one constant).
4. After `pageprobe` has shown the picture: decide whether a smaller composed page is worth the engine change (§ 5.2).

Decisions taken: OSD0_1 is the 4K path's right-half reader and is closed as an overlay route (§ 3); the plane offset
refuses a shift up or left until the units are measured; the reduced page is a probe, not a mode, because the Rust UI
engine draws into the page and has no render scale; kernel modules and persistent register code stay parked (Nitin's).

## 1. What the observation settled

Tags as in `display-chip-direct.md`: `TV` (read on the set), `SRC` (LG's tree, paths under `kdriver/`), `[INFERENCE]`.

| Fact | Status |
|---|---|
| A module loads with a vermagic that differs from the kernel's local version: `btmtk_usb` is live with `5.4.268 SMP preempt mod_unload aarch64` on `5.4.268-329.ptl4tv.2`; LG's `CONFIG_MODULE_IGNORE_SUBVER` (`default y if WEBOS`) compares only `5.4` and the flags | verified (`kernel.txt`, `vermagic.txt`; `linux-rockhopper/kernel/module.c:1396-1416`, `init/Kconfig:2147`) |
| No module signing, `modules_disabled 0`, `/proc/config.gz` absent | verified |
| `kprobe_events`, `dynamic_events` and `uprobe_events` exist and are empty; `available_filter_functions` is absent (no function tracer); the symbols `osd_drm_flip_image`, `FBDEV_O22_CMN_WriteHdrRegs`, `FBDEV_O22_CMN_SetOSDEnable`, `osd_drm_enable`, `FBDEV_O22_CMN_SetOutputCropRegion` are in kallsyms | verified (`tracing.txt`, `kallsyms.txt`); a probe's write is not tried |
| LG's Screen Shift moves the main video window and the OSD crop together, one panel pixel on both axes every 180 s: `vsc_set_orbit_move win0 ... oi[3840x2160(15,0)]` at 20537 s, (16,1) at 20717, (17,2) at 20897, (18,3) at 21077; the regdumps at 21121-21135 s read `OSD0.osd_crop_ctrl_1/2` = (18, 3) | verified (`dmesg-osd.txt`, `regdump-*.txt`); the OSD crop as the orbit's OSD half is the firmware's doing [INFERENCE from the equal numbers] |
| OSD0's position register `hdr0` is 0 under LG's compositor; `lg1k` flushes `hdr0` at every flip and leaves `hdr2` (output size) and the crop to the firmware (`fbdev_pqefw_osd0_out_ctrl 1`, `hw_wa_flag 0x5`) | verified (`regdump`, `chip-diag.txt`; `fbdev_hw_o22_cmn.c:2687-2727`) |
| `CRTC_X/Y` of plane 32 reach `osd_hdr_xpos/ypos` unclamped; the fields are 12-bit unsigned; the driver's signature is `unsigned int x, y` and no `atomic_check` clips the OSD planes (only the cursor has one) | verified (`fbdev_drm.c:300`, `:494-495`; `fbdev_hw_o22_cmn.c:3158-3159`; `osd_reg_o22.h:1320-1322`; `drm.c:798-801`) |
| What one unit of `xpos` is on the panel: the output window is the scaler's 3860x2170 canvas, so one unit is one panel pixel (half a UI pixel) | [INFERENCE from `hdr2 3860x2170`; `orbit` measures it] |
| Nothing rewrote the VSP (`0xCC480360/364`), the mixer holes or the crop over three regdumps a second apart on a still screen; only status, AFBC debug and `osd_gsr_ind_data` moved | verified (`regdump-1..3.txt`, `tools/regdecode.py diff`) |
| The blend mixer in use is B.OSD1 (`0xCC480280`, 3840x2160, `mux0 0xff000008`, `vid_force_en 1`); S.OSD's output is off; B.OSD0 is all zero. Each mixer names per-layer features for `osd0` and `osd4` (the cursor) only (`awb`, `rm_en`, `rm_alpha`, the status counters); the eight mux slots and hole selects are a template | verified (`regdump`, `osd_reg_o22.h:400-640`) |
| OSD0_1 holds the 4K path's right-half header: 1920x2160 at `reg_4k_xpos 1920`, scaler output 3860x2170, layer off, `4kosd.muxcfg 0`, `support_4kosd 0` | verified (`regdump`, `chip-diag.txt`) |
| The cursor plane was idle under LG's compositor at the time (`plane 36: crtc 0 fb 0`); its last header reads 120x170 at (1542, 1268): LG's sprite scaled x2 and placed in panel pixels | verified (`drmplanes.txt`, `dri-state.txt`, `regdump`) |
| `/dev/mem`, `/dev/video60`, `/dev/video70` and `/dev/fb0` are `root:composit 0660`; `/dev/vo` is 0666; `/dev/dri` and `/dev/lg` listings are in `nodes.txt` | verified |
| The firmware reads the OSD position (`Pos:(%d,%d)` in its log format); whether it ever writes `hdr0` is not shown | unknown (`orbit` watches `hdr0` across a Screen Shift step) |

The three regdumps and the OSD0_1 probe run (E13) also agree on the canvas: `OSD0.osd_ctrl_hdr2` 3860x2170, input 1920x1080,
crop 3840x2160, so LG's "overscan" orbit arrangement is what this set runs (`display-chip-direct.md` § 1.3).

## 2. The linear rerun: what it needs and what it would show

The E13 probe ran under LG's compositor with AFBC pages. Under LumaTV's compositor the differences are: LumaTV is DRM
master, so `lg1k` writes the OSD0 header only when LumaTV commits (a still screen holds any register written by hand
until the next flip); the pages are linear XR24 (`ctl afbc off` makes sure), so the AFBC decoder and its TOP path bits
are idle; LG's capture service still taps the layer before the scaler, so the panel and a photo are the only evidence.
`tools/osd01-probe.sh` runs unchanged (it reads its baseline live); `layerdeep-tv.sh osd01` wraps it and adds the two
untried bits as a third stage: `reg_4k_mode_en` (bit 31 of `0xCC480440`, which E13 did not set; it set bit 0, the layer's
`osd_sync_enable`) and the TOP combiner's bypass (`reg_osd0_1x2_bypass_en`, bit 0 of `0xCC480060`, live 1).

## 3. What OSD0_1 is

OSD0_1 is the second reader of LG's 4K graphics path, not a second layer. In 4K OSD mode (`b_support_4kosd`, a boot
option, 0 here) the driver programs two headers for one framebuffer: `osd_hw_hdr[0]` reads the left 1920 columns at
`xpos 0`, `osd_hw_hdr[1]` the right 1920 at `xpos 1920` (`osd_hdr_ptr_bmp += img_stride / 2`, `w_mem /= 2`,
`w_out /= 2`), and the 4K main control registers (`reg_4k_mode_en`, scaler input `reg_sc_i_w/h` = the framebuffer,
output `reg_sc_o_w/h` = the display) join the halves before one scaler (`fbdev_hw_o22_cmn.c:2655-2670`, `:960-979`). The
hardware reason is the blend path's width: in BLEND mode every OSD layer runs at half the display width (`sync_disp_width
= disp_width / 2`, `reg_ctrl_path_width 1920`, "it's HW spec", `:815-830`, `:943-955`), so a 3840-wide picture needs
two readers. OSD0_1 has its own AFBC1, MIF1 and SHP1 blocks because each reader decodes and sharpens its half.

What the mixers know: per-layer registers exist for `osd0` and `osd4` (the cursor) only; the generic eight-slot mux is
set to `0x76543210` and never changed. There is no mixer input that is OSD0_1 on its own: its pixels reach the blend only
through the 4K combiner, as the right half of OSD0's picture. That is why E13 saw nothing: every write held, the layer
was on, and nothing routed it. Verified from the headers and the driver; the combiner's behaviour at 2K is not (it is the
third stage of the rerun, prior about 1 in 20 of showing anything, and the likely picture is a broken OSD0, not an overlay).

Verdict: closed as an overlay route. The second hardware layer this chip offers at 2K is the cursor plane (osd4, 128x128,
`hw-planes-result.md`), and hardware blending beyond it is the mixer's holes and the VSP (§ 4).

## 4. Routes to more, ranked

| # | Route | What it gives | Verified | Inferred | Risk | Next |
|---|---|---|---|---|---|---|
| 1 | Plane offset for the OLED orbit (built, § 5.1) | Home direct while shifted right or down | `CRTC_X/Y` reach `hdr0` unclamped, unsigned 12-bit, flushed each flip | one unit = one panel pixel; the firmware leaves `hdr0` alone | low: a wrong unit is a 1.5 px jump; a refused commit composes the next frame | `orbit` |
| 2 | Cursor plane for the pointer (HwPlanes) | the pointer off the composed frame | plane 36 AR24 128x128, header at panel units, x2 scaling in the driver | works under LumaTV's master | low | HwPlanes' round |
| 3 | Reduced page on the OSD scaler (probe built, § 5.2) | cheaper composed frames | the driver sets the input window from the framebuffer; LG builds 1280x720 UIs on this driver | the firmware accepts a 1280x720 input at run time; the sharpness is acceptable | low: a refused commit, or a soft picture for N seconds | `pageprobe` |
| 4 | OSD sharpening knobs | crisper text at x2 for nothing | the knobs and their registers | the visible effect | very low | `sharp` |
| 5 | VSP contrast, grayscale (`0xCC480360`) | a hardware dim and a grayscale for the OLED care, with Home direct | the fields and their live values; nobody rewrote them in 3 s | that the firmware's `OSD VSP WA` leaves them alone; what a value looks like | medium: a wrong value is a dark or tinted UI until the revert | `vsp` |
| 6 | Mixer holes on B.OSD1 (`0xCC4802C8-0xCC4802DC`, `0xCC48029C`) | a hardware video hole in an opaque page; a full-screen hole with alpha as a dim | the registers, the live mixer, their zero state | the semantics (a rectangle whose alpha replaces the layer's) and the select encoding | medium: a wrong write blanks the UI until the revert; no hang path known | `hole` |
| 7 | OSD0_1 (§ 3) | nothing at 2K | the 4K split and the mixer's inputs | the combiner at 2K | medium: a broken OSD0 picture until the revert | `osd01`, optional |
| 8 | Video tiles through the four graphic scalers | live camera tiles | the resource table (`GP_SCALER 4`) | the sampling cost, the VTG protocol | study only | unchanged from item 7 |
| 9 | kprobes on `osd_drm_flip_image` / `FBDEV_O22_CMN_WriteHdrRegs` | every header write LG's code makes, without a module | the files and symbols exist | that a probe's write is accepted | low, but it is runtime kernel code: Nitin's call | `observe` lists the exact line; not run |
| 10 | Kernel module | a persistent dim or a plane remap | it would load (`IGNORE_SUBVER`) | nothing it buys that 1-6 do not | high | parked |

Principle 0: every write below the DRM layer in § 6 is typed by Nitin at the set; `layerdeep-tv.sh` is not reachable
from any AI-facing tool, and its raw steps print their writes and their revert file, then wait for a typed `yes` from a
terminal (no terminal, no writes). The two switches built in the compositor are reads and standard DRM commits.

## 5. Built on the host

### 5.1 The care's orbit as the plane's position (`--scanout-offset`)

`Plan::sole_placed` is `Plan::sole_candidate` with the window's place: a sole full-screen window that sits exactly where
the OLED care moved it (`Item::drift`, now recorded for the shell's own screens as it was for the app features) is a
candidate at that offset; a window placed anywhere else by its own position is `Geometry` as before. `Scanout::choose`
takes the offset when `Policy::offset` is on and both components are non-negative; otherwise the frame is composed with
the new reason `shifted` (`ctl scanout` counts it; the gate's `geometry` no longer mixes the care's shifts with real
geometry). The choice carries `at`, the assignment carries it (`Assignment::at`), and the commit sets `CRTC_X/Y` of the
primary plane to it, with the size and source the mode's. The journal says `scanout: direct at (x, y)` when the place
changes; the report has `offset.on`, `offset.placedFrames`, `offset.lastAt`. Tests: `tests/scanout.rs`
(`the_cares_orbit_is_the_planes_position_right_or_down_of_the_origin_when_allowed_and_composes_the_frame_otherwise`, the
run-time switch), `kms/assign.rs`.

Why not every beat: the ring is signed (`oledcare::ring`, corners at (-r, -r)) and the driver's position is unsigned, so a
shift up or left would land at 4093 panel pixels. At radius 3, 7 of the 24 ring points are non-negative. Once `orbit`
has shown the units, a ring whose origin is (r, r) makes every beat direct; that is one line in `oledcare` (the offset
plus `(r, r)`) and a 3 px average shift of Home that no eye sees, and it is left until the units are known so the strips
are the ones the software orbit shows.

What it serves: a client window the plane takes and the care moves, so the QML shell's Home (its buffers linear,
`--scanout-shell-env WAYLAND_AFBC=0`) and an app's card under `appShift`. The Rust UI engine draws its Home into the
composed page, not as a client window, so a Rust Home is never direct and the offset does nothing for it; a direct Rust
Home would need the engine's page on the plane with the same offset, which the commit already allows for a composed
page (`Assignment::at` is 0 for one today). The strip the plane leaves at the left or top shows what is under the OSD:
the mixer's black, as the composed frame's clear colour is black [INFERENCE: `reg_mixer_out_*` 0; `orbit`'s photo].

### 5.2 The page probe (`ctl pageprobe WxH SECONDS`)

`kms/pageprobe.rs` has the rules (within the mode, width a multiple of 16, height even, 256x144 at least, 1-120 s) and
the picture: a one-pixel white grid every 32 buffer pixels, a white border, one-pixel stripes top left, a diagonal, red,
green and blue squares, a grey ramp along the bottom. `Kms::start_page_probe` writes it into an XR24 dumb buffer of that
size, adds a framebuffer and the next frame commits it on the primary plane with `SRC_W/H` the buffer's and `CRTC_W/H` the
mode's; the ring's page comes back by itself when the time is up (`ui_needs_frame` owes the frame), or at `ctl pageprobe
off`, and the buffer goes after that flip has landed. Never automatic; one at a time; the report is `ctl pageprobe`.

What a smaller composed page would need, if the probe's picture is acceptable: the Rust UI engine draws its stacks into
the page (`ui/mod.rs`, `Engine::draw_stack`) at `HostConfig::{width,height}` with no render scale (`rustui/render/src/host.rs`,
`geometry.rs` `Panel` has logical and physical sizes equal), so the engine needs a scale (viewport and scissor by the page
size, layout at 1920x1080); then the compositor's `Gl` (viewport, `scissor`, `grab_target`, the stand-in copies) and
`Kms::commit` (`SRC` the page's size) follow, `capture` reads the page's size, and the damage tiles stay in output pixels.
That is the engine's owner's change, decided after the probe.

## 6. At the set: `tools/layerdeep-tv.sh`

Copied to `/tmp/lumatv/layerdeep-tv.sh` and run as root from a terminal on the set, LumaTV's compositor live (runtime dir
`/tmp/xdg`; `LD_RT=/tmp/lumatv` for a maintenance window). Every step prints what it reads, what to photograph and what
"as expected" is, and writes its log and dumps under `/tmp/lumatv/out/layerdeep/`. The raw steps arm a detached dead-man
that runs the exact revert file `DEADMAN` seconds after the write (default 12), and revert on exit as well; `revert`
runs the last step's revert file by hand. Photographs: the panel's top-left corner for strips, the middle for holes,
text for sharpness; one before, one during, one after each step.

| Step | Writes | Expected reading | Photo | Revert |
|---|---|---|---|---|
| `observe` | none | `ctl status`, `ctl scanout`, `ctl pageprobe`, fb0 header (`hdr.xpos/ypos`, `w_mem`, `w_out`), `chip/diag`, three regdumps a second apart and their diff, the last `ORBIT_MOVE` lines, the kprobe line it would take | none | none |
| `orbit` | `lumatv theme set` (orbit radius 1, step 10 s, care on; the theme file is backed up), `ctl scanout offset on` | within about a minute `ctl scanout` shows `offset.placedFrames` rising and `lastAt` at (1, 0) or (1, 1); `fb0 hdr.xpos/ypos` equal `lastAt` on a placed frame; the journal has `scanout: direct at (1, ...)`; `refused.shifted` counts the beats up or left; across one `ORBIT_MOVE` (up to 180 s) `hdr.xpos` keeps its value | the top-left corner on a placed frame: a 1-unit strip at the left or top; its width on a 4K photo is the unit (1 panel pixel or 2) | theme file restored, `ctl scanout offset off` |
| `pageprobe` | `ctl pageprobe 1280x720 20`, then `1440x810 20`, then `1920x1080 10` (control) | `fb0 hdr.w_mem/h_mem` read 1280x720 while it shows, `w_out/h_out` stay 3860x2170; no `error` line in dmesg; the page is back when `ctl pageprobe` says `active false` | the whole panel (40 grid cells across at 1280, 45 at 1440, 60 at 1920), the stripe block, the diagonal, the ramp | by itself; `ctl pageprobe off` |
| `sharp` | `/proc/lg/fbdev/chip/osd_sr_gain` 0x00, 0x40, 0x80, then `osd_sr` 0, each 8 s | `chip/diag` echoes `sr_ctrl[0].ti_gain` and `osd_sr.sw_status` | Home's text at each setting | `osd_sr 1`, gain `0x10` |
| `vsp` | `0xCC480360` = `0x01000001` (contrast half), then `0x02000003` (grayscale), 4 s each | the register reads back; the UI darkens, then loses its colour; `fb0` flips do not change it | Home during each | `0x02000001`; the dead-man |
| `hole` | B.OSD1 hole 0: `0xCC4802C8` = x 1440 w 960 enabled, `0xCC4802CC` = y 810 h 540 alpha 0; `0xCC48029C` = `osd0_en 1, hole_sel 0`, then `hole_sel 1`, 4 s each | the registers read back; a 960x540 rectangle in the middle shows what is under the OSD (black, or video) | the middle of the panel | all three to 0; the dead-man |
| `osd01` | `tools/osd01-probe.sh all` (its own dead-man), then stage 3: bit 31 of `0xCC480440`, then bit 0 of `0xCC480060` cleared, 4 s each | as E13 (every write holds, nothing shows); stage 3: most likely nothing or a broken OSD0 for 4 s | the panel during stage 2 and 3 | the probe's revert; stage 3's dead-man |

Worst cases, by step: `orbit` and `pageprobe` end in a composed frame or `tv-recover-home.sh`; `sharp` in a soft or
over-sharp UI until the knob is put back; `vsp` and `hole` in a dark, grey or holed UI until the revert (12 s at most);
`osd01` as E13 said, a power cycle. None of the raw writes touches the back end, OVI or the OLED blocks.

## 7. Open questions left

1. One unit of `CRTC_X` on the panel (`orbit`, photo).
2. Whether the firmware rewrites `hdr0` at a Screen Shift step (`orbit`, 180 s).
3. The OSD scaler's picture at 1280x720 and 1440x810 (`pageprobe`, Nitin's eyes).
4. What the VSP's contrast and grayscale look like, and whether the firmware fights them (`vsp`).
5. The mixer hole's semantics and the select encoding (`hole`).
6. Whether a kprobe write is accepted (not run; Nitin decides).
7. The 4K combiner at 2K (`osd01` stage 3, optional).

Dead ends added to `display-chip-direct.md`'s list: OSD0_1 as a layer (§ 3).

## 8. TV results (2026-10-29, Nitin at the set)

Evidence in `compositor/display-hw-2026-10-29/`. Only `observe` and the first `pageprobe` ran; the rest was held after the probe's failure
(below), and the set had cold-booted at 15:13:40 beforehand (`compositor/wake-2026-10-29/README.md`).

- `observe` (read-only, 15:12): compositor `kms`, scanout mode `off` (`orbit` needs `ctl scanout direct` first), fb0 header xpos/ypos 0,
  mem 1920x1080, out 3860x2170, `osd_sr` 1 and gain 0x10, VSP `0x02000001` / `0x02008000`, B.OSD1 hole registers 0, OSD0 crop (2, 7) following
  LG's Screen Shift (`oi[3840x2160(2,7)]`), `hdr0` 0.
- Remote wheel (event2, `LGE M-RCU`, recorded with Nitin turning and pressing): one `REL_WHEEL` (type 2, code 8, +1) was captured, then the
  pointer as absolute X/Y (type 3, codes 0 and 1) with key code 1198 pulsing every 0.5 s, and the press as key code 1199 down and up. The press
  produced no `BTN_LEFT` (272) and no `KEY_ENTER` (28). Only one of the three clicks was in the recording (an earlier attempt hit a recorder that
  crashed: the TV's userland is 32-bit, `struct input_event` is 16 bytes there). `wheel-events.txt`.
- `pageprobe 1280x720 8` failed. The page probe's first commit was refused by the driver (`error present: atomic commit: Invalid argument (os
  error 22)`, 15 ms after `kms: page probe 1280x720 (fb 40)`), and every present after it failed the same way for the rest of the probe and
  beyond (79 errors in the bundle's tail). `fb0` read `hdr.w_mem` 1280x720 while it ran, so the header was written before the commit was
  refused; flips went 5060 to 5061 to 5063. Nitin reported seeing the grid screen and nothing odd; the panel most likely showed the probe's
  frame or the last one. The compositor's watchdog fired at 15:20:37 (`output frozen: 60 frames drawn and no page flip for 6 s`, crash bundle
  `20261029-152037-001`, `failures` 1) and the supervisor fell back to LG's compositor (Home back at 15:20:44). `lumatv compositor on` brought
  LumaTV back at 15:23 (`failures` 0, `gate.sh live` PASS, Home in `img/restored-home.jpg`). The second probe (1440x810) did not run.
- `sharp` (OSD sharpener gain 0x00 / 0x40 / 0x80 and `osd_sr` 0, 15 s each, first on Home, then labelled with toasts on LG's Settings, then as
  A/B flicker pairs on a test card): Nitin could not tell the states apart by eye in any of the three runs, so the sharpener at the x2 OSD scale
  has no visible effect that he could find; the readbacks were correct each time (`chip/diag` `ti_gain`, `osd_sr.sw_status`).
- Not run: `orbit`, `vsp`, `hole`, `osd01` (the next test is a guided on-screen app Nitin drives with the remote).

Fix the probe needs before another run: do a `DRM_MODE_ATOMIC_TEST_ONLY` commit of the probe's plane state first and show it only if that
passes; on any commit error of a probe frame, put the normal page back at once (the next frame) instead of repeating the refused commit.
Reading: the driver's `atomic_check` refuses an OSD plane `SRC` of 1280x720 under `CRTC` 3840x2160 (or the framebuffer's stride/format for it);
which is not known, the property values of the refused state were not logged (add them to the error line).

### 8.1 Findings for the next test

- Wheel press on the Magic Remote (`event2`, `LGE M-RCU`): key code 1199 down and up; no `BTN_LEFT`, no `KEY_ENTER`. Pointer motion is absolute X/Y with
  key code 1198 pulsing every 0.5 s while it moves. The TV's userland is 32-bit: `struct input_event` is 16 bytes (`evrec.py`).
- LG's browser (`com.webos.app.browser`, `luna://com.webos.service.applicationmanager/launch` with `params.target`): `file://` (in `/tmp` and
  `/media/developer`) is refused ("webpage is not available"); a `data:text/html;base64,...` URL works (`open-card.sh`). Its toolbar takes the top
  99 panel px; the page area is 1920x981 with `devicePixelRatio` 1.25 (viewport 1536x785), so a page that wants 1 CSS px = 1 plane px sets
  `document.documentElement.style.zoom = 1/devicePixelRatio` (`tools/sharpness-card.html`). `closeByAppId` ends it; `launch com.webos.app.home` brings Home back.
- `com.webos.notification/createToast` as a label raises entries in the notification centre (bell badge 17 after the runs; db8 kind
  `com.webos.notificationhistory:1` is not readable by `luna-send` as root, no list method, so they cannot be removed by API). Do not use it for labels.
- The LG Settings app launched by `com.palm.app.settings` is not closed by launching Home; `closeByAppId` is needed.
- OSD0 VSP (`0xCC480360`, `0xCC480364`) reads 0 and the B.OSD1 hole registers read 0 on this boot (a resume boot, `/sys/power/mode` `resume`); at 15:12
  on the previous boot the VSP read `0x02000001` / `0x02008000`. The script's "expected 0x02000001" is therefore boot-dependent.

### 8.2 Registers and switches used (restored by read-back)

| What | Address / path | Original | Test values | Restore |
|---|---|---|---|---|
| OSD sharpener gain | `/proc/lg/fbdev/chip/osd_sr_gain` (0..255) | `0x10` (`chip/diag` `sr_ctrl[0].ti_gain`) | `0x00`, `0x40`, `0x80` | `echo 0x10`; read `ti_gain` |
| OSD sharpener on/off | `/proc/lg/fbdev/chip/osd_sr` (0 off, 1 on) | 1 (`chip/diag` `osd_sr.sw_status` 0x1) | 0 | `echo 1`; read `sw_status` |
| OSD0 VSP contrast / gray | `0xCC480360` (`vspyc_en` bit 0, `graycolor_en` bit 1, `r_center` 8:15, `r_contrast` 16:25) | 0x00000000 on this boot (0x02000001 on the previous) | `0x01000001` (half contrast), `0x02000003` (gray) | write the read-back original; not written today |
| B.OSD1 hole 0 | `0xCC4802C8` (w 0:12, x 13:25, en 28), `0xCC4802CC` (h 0:11, y 12:23, alpha 24:31), `0xCC48029C` (`osd0_en` bit 0, `hole_sel` 1:3) | all 0x00000000 | `0x10B403C0`, `0x0032A21C`, sel `1` then `3` | all three back to 0; not written today |
| Compositor scanout mode | `ctl scanout [off|direct]` | `off` | `direct` for `orbit` | `ctl scanout off`; not changed today |
| Page probe | `ctl pageprobe WxH SECONDS` | inactive | 1280x720 | commit refused; see above, do not use until the probe does a TEST_ONLY commit |

Each write step arms a detached dead-man (`sleep N; restore`) before the first write and the dead-man is longer than the whole sequence. At the end:
`osd_sr_gain` `0x10`, `osd_sr` 1, VSP and hole registers 0, scanout `off`, no probe, Home foreground, LumaTV's compositor live (`gate.sh live` PASS, `failures` 0).
