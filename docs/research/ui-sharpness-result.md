# UI sharpness: why Home looks soft up close, and what makes it sharper

Research of 2026-10-29, no TV access, nothing written to the set. The question: close to the panel, Home's text and the app
icons look a little blurry; find the cause with evidence, rank the fixes by sharpness gained against cost, build what the
host can prove. Prior evidence: `display-chip-direct.md`, `layer-deep-result.md` (§ 8, the TV session of 2026-10-29),
`scanout-research.md`, `hw-planes-result.md`, `rust-shell-plan.md`, `rustui-result.md`, `ui120-result.md`.

## Resume here

Done (host, committed to forge/main): the Rust UI's lint finds text layers between whole pixels (`Soft`) and the three
spots it found are on whole pixels now (the dock's name pill, the quick panel's slider marks; the keyboard's caps and the
clock's lines were in-block false positives, the lint now judges a block's lines by the block's layer); the Rust dock tile
keeps a store icon at the lifted box's size so the focused tile reads it 1:1; the QML dock tile samples its icon without
mipmaps (RMS against a Lanczos downscale 6.8 to 3.8); `tools/layerdeep-tv.sh scaler` and `shift` for the set. Checks run:
`cargo test -p lumatv-ui` (950 pass, after the rebase onto the drawer commit), `cargo clippy -D warnings` on the crate and its tests, `tests/qmlhost/compile_all.py`
(151 files), `tests/test_rust_home.py` (14 pass, the renders in `docs/mocks/host-rust/home/`).

Next, in order:

1. One TV round (§ 6): the side-by-side photographs that separate the page from the OSD (LG's Home against LumaTV's Home,
   the same text, 50 cm, tripod), the `scaler` step (raw, Nitin at the set) and the `shift` step. These settle whether the
   OSD's own filter can be sharper and whether LG's Screen Shift costs sharpness.
2. If `scaler` shows a sharper filter: tell Main; the Display lab (`helper/app/src/displaylab/allow.rs`) takes
   `0xCC480310` as a lab step (DisplayLab's small change), and `tools/lumatv` applies it at the compositor's start behind a
   theme key, off by default.
3. Nothing else is worth building before those photographs: the page is 1080p by hardware (§ 4), so what remains is the
   OSD's kernel and the type's weight.

Decisions taken: a 4K page is not built (the driver reads the 4K-OSD option at boot and the blend path is half the panel
wide per reader, § 4; the budget would not hold it either); no change to type sizes or weights (a design choice for Nitin,
§ 4 option h); the OSD sharpener is closed (three TV runs, no visible effect).

## Verdict

- The UI is rendered at 1920x1080 everywhere (the QML shell's window, the compositor's page, the Rust UI's page) and the
  display chip's OSD scaler takes it to the panel: input 1920x1080, output 3860x2170 (2.0104 by 2.0093), cropped to
  3840x2160 at LG's orbit offset. Verified on the set (`regdump`, `fb0` header). That x2 interpolation is the blur Nitin
  sees: every stem of a 26 px label is 2.3 UI pixels, so 4.7 panel pixels with a soft shoulder on each side, and at 1.5 m
  or closer the eye resolves single panel pixels (0.44 mm on the 77 inch panel; one arc minute is 0.44 mm at 1.5 m, 0.87 mm
  at 3 m, where it stops showing). LG's own Home has the same limit; a photograph pair proves it (§ 6).
- Beyond the x2, the page itself added blur in three places, all fixed on the host: the QML tiles read a 192 px store icon
  through mipmaps at a level two thirds of the way to 96 px; the Rust dock's name pill and the slider marks sat between whole
  pixels (bilinear smear of every glyph); the Rust tile grew a 120 px copy of the icon by 1.12 when focused.
- A native 4K UI is closed on this set: `lg1k` decides 4K OSD at driver init from a board option on the kernel command line
  (`b_support_4kosd` 0 here), each OSD reader is half the panel wide by hardware, and a 3840-wide framebuffer on the primary
  plane would be written as one header on a 1920-wide path. Were it open, three 4K pages take 99.5 MB of CMA (48-68 MB free)
  and a full Home redraw 20-24 ms of GPU (5-6 ms at 1080p; the 120 Hz budget is 7.5 ms).
- The one hardware knob left on the x2 kernel is the OSD scaler's own filter select (`osd_ctrl_scaler`, 0xCC480310: a 2-bit
  vertical cubic select, a bilinear bit, a sampling-mode bit), which `lg1k` leaves at the hardware default and never rewrites
  after init. A raw-register step with a dead-man is written for Nitin at the set; its effect is unknown.

## 1. Verified and inferred at a glance

Tags: `TV` (read on this set), `SRC` (LG's GPL tree, `kdriver/` paths as in `display-chip-direct.md`), `HOST` (measured on
the workstation), `REPO` (code here), `[INFERENCE]` (not shown; the check that would show it is named).

| Claim | Status |
|---|---|
| The compositor's output and page are 1920x1080 (`--output`, default `OUTPUT_WIDTH/HEIGHT`); the QML shell's window and the Rust UI engine draw at that size, 1:1, at whole-pixel positions (the care's drifts are `i32`) | verified `REPO` (`config.rs`, `kms/mod.rs`, `oledcare.rs`, `scene.rs`) |
| OSD0 input 1920x1080, output 3860x2170, crop 3840x2160 at the orbit offset; the scale is 2.0104 by 2.0093 | verified `TV` (`layer-deep-result.md` § 8 `observe`, `scanout-hw-02.txt`) |
| The OSD sharpener (`osd_sr`, gain 0x10) has no effect Nitin could see at 0x00, 0x40, 0x80 or off | verified `TV` (three runs, 2026-10-29) |
| `osd_ctrl_scaler` (0xCC480310) reads 0: hardware default filter, cubic select 0, no bilinear bit; `lg1k` writes it at the layer's init only (`KEEP HW DEFAULT`) | verified `TV` (regdump) and `SRC` (`fbdev_hw_o22_cmn.c:868-927`, `osd_reg_o22.h:1261-1275`) |
| What each filter select looks like on the panel | unknown (`scaler`, § 6) |
| 4K OSD is a board option read at driver init (`LX_BOARD_OPT_4K_GRAPHIC` or `osd.fb.fb0=` on the kernel command line); `b_support_4kosd` is set only when fb0 is at least 2048 wide; the set reports `support_4kosd 0` | verified `SRC` (`fbdev_drv_o22.c:279-297, 449-457`) and `TV` (`chip/diag`) |
| In blend mode every OSD reader is half the panel wide ("it's HW spec"); a 3840-wide picture needs two readers and the 4K combiner, programmed only in 4K-OSD mode; a 3840-wide framebuffer on plane 32 today becomes one header with `w_mem 3840` on the 1920-wide path | verified `SRC` (`:816, :840-845, :949-956, :960-977, :2667-2676, :3128`); the picture that gives is `[INFERENCE]` (a broken OSD, not a sharp one) |
| The DRM core accepts any framebuffer size on plane 32 (no plane `atomic_check`, `max_width 3840`) | verified `SRC` (`drm.c:798-801, :960-963`) |
| A 4K page costs 33.2 MB (XR24) a page, 99.5 MB a ring of three; CMA free 48-68 MB; a full Home redraw 5-6 ms of GPU at 1080p | verified `REPO`/`TV` (`rust-shell-plan.md`, `rustui-result.md`); x4 at 4K `[INFERENCE]` |
| QML text is drawn natively on whole pixels: all 56 `Text` items and the 191 `LuText` have `renderType: Text.NativeRendering` | verified `REPO` |
| QML tiles: a 192 px icon decoded at 240 with `mipmap: true` and sampled in a 120 px box reads mip level 0.68; RMS against a Lanczos downscale 6.8 with mipmaps, 3.8 without (82 px inset box: 8.2 against 7.0; a 130 px icon: 4.9 against 4.3) | verified `HOST` (Qt 5.15, Mesa llvmpipe, `tests/qmlhost`; § 3.2) |
| Rust UI: glyphs are placed on whole pixels inside a run; a run layer at a fractional position is sampled bilinearly (half a pixel of smear) | verified `REPO` (LumaShow `crates/ui/src/atlas.rs:1137-1138`, `:188-192`) |
| Rust UI: three text layers sat between whole pixels (the dock pill at y 903.2, the slider marks at x 924.67 and 1059.33); every other surface's words are on whole pixels | verified `HOST` (the new `Soft` lint over every lint test) |
| Rust dock tile: the icon was a 120 px (82 inset) box-filtered copy grown 1.12 by the lift when focused | verified `REPO` (`home/surface.rs` `tile_icon`) |
| Icon sources on this set: `extraLargeIcon` 192 px (14 of 33 launch points, the store apps), `largeIcon` 130 px (32), `icon` 80 px (33); the tile is 120 UI px = 241 panel px (270 focused): a 192 px icon is magnified 1.26 on the panel, a 130 px one 1.85, an 80 px one 3.0 | verified `TV` (`docs/research/cardapp/tv-20261025-1939/launchpoints.json`, the file names) |
| LG's Screen Shift (`picture.screenShift` on) is the "overscan" orbit (the 3860x2170 canvas); off, the OSD output would be exactly 3840x2160 | `[INFERENCE]` from LG's VSC orbit document (`display-chip-direct.md` § 1.3); `shift` reads it (§ 6) |

## 2. The render path, measured

```
QML shell (Qt, 1920x1080 window)  ─┐
Rust UI engine (draws into the page)┼─ compositor page 1920x1080 XR24 ─ plane 32 ─ OSD0 reader ─ OSD scaler x2.01 ─ SHP ─ mixer ─ panel 3840x2160
app window (1920x1080, direct)     ─┘                                                  (filter select 0, phase 0)   (gain 0x10, no visible effect)
```

Where the pixels come from, and what each stage does to a 26 px label (Lucent `label`, Manrope medium):

| Stage | Size | What it does to sharpness | Evidence |
|---|---|---|---|
| Glyph raster | 1080p | QML: FreeType hinted antialiasing (`NativeRendering`), whole pixels. Rust: `fontdue` coverage, no hinting, placed on whole pixels in the run; a 26 px stem is about 2.3 px, so one grey edge pixel each side | `REPO` |
| Composition | 1080p | 1:1 copies at whole-pixel positions (the QML window, the Rust layers at rest); the care's orbit is whole pixels. Lifted or scrolling items are resampled while they move, by design | `REPO` |
| OSD scaler | 1080p to 3860x2170 | Every UI pixel becomes 2.01 panel pixels; with an interpolating kernel a 1 px edge becomes 2 to 4 panel pixels of ramp, and the 0.01 excess walks the sampling phase one pixel every 96 UI pixels | `TV` regdump; the kernel's shape is unknown (filter select 0) |
| OSD sharpener | panel | A 2D LTI gain (0x10); no visible change at 0, 0x40, 0x80, off | `TV` |
| Orbit crop | panel | 3840x2160 of the canvas, moved one panel pixel every 180 s (LG's Screen Shift) | `TV` dmesg |

Pixel geometry on the 77 inch panel: 0.4435 mm a panel pixel, 0.89 mm a UI pixel. One arc minute (what a normal eye
separates) is 0.29 mm at 1 m, 0.44 mm at 1.5 m, 0.73 mm at 2.5 m, 0.87 mm at 3 m. Closer than 1.5 m the panel's own pixels
are visible and the interpolation ramps with them; from the sofa they are not. This is why the softness is a "close to the
TV" finding and why LG's own 1080p UI shares it.

## 3. Blur beyond the x2: what the page added

### 3.1 Text

QML: every text item draws natively (`NativeRendering`) and the kit's `LuText` rounds nothing it does not have to; the
dock's name pill rounds its position (`Tile.qml`). No change.

Rust UI: a label is a run of glyph quads of one atlas; the glyphs sit on whole pixels of the run (LumaShow `crates/ui/src/atlas.rs:1137`), and the
run layer is placed by its box's `Props::at(x, y)`. A box between whole pixels makes the compositor sample the whole run
bilinearly: every stem is smeared across a neighbour by the fraction, and the OSD doubles that. The lint
(`kit/lint.rs`) now has a `Soft` rule: a word's layer origin more than 0.01 px from a whole pixel is a finding. Over every
surface the lint tests cover it found three real cases and two false ones:

| Words | Origin | Cause | Fix |
|---|---|---|---|
| `dock.pill` (the name under the focused tile) | y 903.2 | `tile * (tileScale - 1) / 2` is 7.2 at a 120 px tile | rounded (`home/surface.rs`) |
| `card mark 1`, `card mark 2` (the OLED slider's marks) | x 924.67, 1059.33 | thirds of the marks' width | rounded (`panels/quick_surface.rs`) |
| the keyboard's 42 caps | x and y at a 119.85 px pitch | one block at the grid (whole pixels); the caps are lines of it | the lint judges a block's lines by the block's layer (`word_on`) |
| the clock's lines | y 288.25 | lines of one block | the same |

### 3.2 Icons

The QML tile (`Tile.qml`) decoded the icon at up to 240 px with mipmaps and sampled it in the shader. A 192 px store icon
in the 120 px box is a 0.625 minification: the GPU's trilinear read sits at mip level log2(192/120) = 0.68, two thirds from
the 96 px level, so the icon arrived at the panel with about the detail of a 110 px picture, then doubled. Measured on the
host harness (`tests/qmlhost`, Qt 5.15 on llvmpipe, a 192 px test icon with a letter, small text, 1 px rules and a dot
grid), RMS error of the rendered box against a Lanczos downscale of the icon to the same size, lower is better:

| Box | mipmaps (before) | no mipmaps (after) | decoded at the lifted size, no mipmaps |
|---|---|---|---|
| 120 px, at rest | 6.79 | **3.75** | 6.82 |
| 134 px, focused | 17.0 | **16.0** | 18.1 |
| 82 px inset, at rest | 8.22 | **7.01** | 7.71 |
| 92 px inset, focused | 12.8 | 11.3 | **10.9** |
| 130 px icon, 120 px box | 4.88 | **4.26** | |
| 130 px icon, 82 px box | 6.79 | **4.76** | |

(The focused rows carry the sheen and rim of the lifted tile in the error; compare across a row.) Plain bilinear reading of
the 192 px texture is the most faithful at every size the tile uses, because the box never shrinks the icon below 0.43 and
at that scale bilinear still reads every texel. `Tile.qml` now has `mipmap: false`; `kit/LuAppIcon.qml`, which draws one
app at 96 px and smaller (down to 0.25), keeps its mipmaps. Picture: `ui-sharpness/tile-mipmap-before-after.png`.

(Superseded for the Rust tile by `icon-sharpness-result.md`: Lanczos to the exact size, two pictures, 1:1 at rest and at full lift.)

The Rust tile (`home/surface.rs`) box-filtered the icon to the box (120 or 82 px) on the CPU, which is right at rest, and
the lift then grew that copy by 1.12 on the GPU when focused: the one tile being looked at was the soft one. The layer now
holds the icon at `icon_texture_side`: the picture's own side held between the box and the box at full lift (192 shrinks
once to 135, 130 is kept as it is, 80 grows once to 120), shown at `box / side` through `Props::scale`, so the focused tile
reads a store icon 1:1 and at rest a 0.89 minification.

The sources are the bound: LG's app manager offers nothing above `extraLargeIcon` (192 px, store apps) and `largeIcon`
(130 px, every homebrew app), and the tile is 241 panel pixels wide at rest. `bgImage` (21 launch points) is a backdrop,
not an icon. A sharper icon than the panel shows today needs the source at 240 px or more, which only LumaTV's own theme
icons (`tiles.icons`) can be.

## 4. A native 4K UI, and the alternatives

### 4.1 Why 4K pages are closed on this driver

`lg1k` lists a 3840x2160 mode and the DRM core would take a 3840x2160 framebuffer on plane 32 (no plane check,
`max_width 3840`), so `--output 3840x2160` would get as far as the OSD header. There `fbdev_hw_o22_cmn.c` clamps the input
window to the display (3840, no effect), and `FBDEV_O22_CMN_WriteHdrRegs` writes one header, `w_mem 3840`, because
`FBDEV_O22_CMN_GetHWXList` gives two readers only when `b_support_4kosd` is set (`:205`), and that flag is decided once at
driver init from the board option on the kernel command line (`LX_BOARD_OPT_4K_GRAPHIC`, or the debug override
`osd.fb.fb0=WxH`, `fbdev_drv_o22.c:279-297, 449-457`). The hardware reason is at `:812-816`: in blend mode (this set:
`path=B.OSD`) every OSD layer runs at half the display width (`reg_ctrl_path_width 1920`, `osd_sync_disp_w 1920`), so a
3840-wide picture is read as two 1920-wide halves by OSD0 and OSD0_1 and joined by the 4K combiner (`:960-977`;
`layer-deep-result.md` § 3). One reader on a 3840-wide line is outside what LG's own code ever programs; the likely picture
is a broken OSD, not a sharp one [INFERENCE: the `scaler` photos would not show it; a `pageprobe` at 3840x2160 would, and
the probe refuses sizes above the mode].

Turning the board option on means the kernel command line, which is the bootloader's: a firmware change, human-only
(Principle 0), and it would break LG's own 1080p clients, since this build has `FBDEV_O22_SUPPORT_4KOSD_UPSCALER`
undefined (`fbdev_hw_o22.h:133`, "support 2K frame buffer on 4K OSD display environment"), turns the OSD sharpener off in
4K mode (`:724-727`) and does not work with the orbit ("4KOSD test mode: OSD output not normal with orbit", `:2708`).

Even with the door open the budget says no: a 4K XR24 page is 33.2 MB, a ring of three 99.5 MB against 48-68 MB of CMA
free; a full Home redraw measured 5-6 ms of GPU at 1080p (`rustui-result.md`) is 20-24 ms at four times the pixels, past
the 8.3 ms frame at 120 Hz and the 16.7 ms at 60; LG's own note on a 3840x2160 OSD stream (the 8K sets' E-streamer path) is "excessive OSD DDR
bandwidth (2 GB/s)", the reason that path went AFBC (`fbdev_hw_o22_cmn.c:3054-3063`). Damage-only redraws and AFBC pages would cut the steady state, not the worst frame.

### 4.2 The options, ranked by sharpness gained against cost

| # | Option | Sharpness gained | Cost | Verdict |
|---|---|---|---|---|
| 1 | The OSD scaler's filter select (`scaler`, § 6): cubic 1-3, the bilinear bit, the sampling mode | Unknown; the only knob on the x2 kernel itself, so potentially the largest gain available | One raw register, holds across flips, restored by read-back; 5 photographs; Nitin at the set | **Test first** |
| 2 | Page-side fixes (§ 3): no mipmaps on the QML tile, the Rust tile at the lifted size, text layers on whole pixels | Icons: RMS error against the ideal halved at rest; the focused Rust tile from a 1.12 bilinear blow-up to 1:1; text: the half-pixel smear gone where it was | Built, host-tested, no TV change | **Done** |
| 3 | LG's Screen Shift off (`shift`, § 6): the OSD output exactly 3840x2160 [INFERENCE] | Every UI pixel on exactly two panel pixels with a fixed phase; small against the kernel's own blur, and only if the output size really changes | A panel-care setting Nitin owns; LumaTV's own care orbits Home, LG's moves apps and video too | Measure, then Nitin's policy |
| 4 | Type weight and size: Lucent `label` 26 px medium; a semibold label or 28 px reads sharper through the x2 because the stem spans more whole panel pixels | Perceived; a design decision | A theme change, every surface | Nitin's call |
| 5 | Better icon sources: LumaTV theme icons at 240 px or more for the dock's apps (`tiles.icons`) | The focused tile's icon from 1.26x magnification to 1:1 on the panel; nothing for LG's own files | Artwork per app; the loaders already take any size | Later, per app |
| 6 | Home at rest as a 4K still on the main video window (a hardware JPEG decode into the window, the OSD transparent over it) | 4K luma for the static picture | The GPU renders a 4K page once per change (33 MB, 20-24 ms), a 4K JPEG encode on the CPU, LG's picture processing on the video path (the UI would follow the picture mode; the video plane reads 0.93 of its clip against the OSD's 1.01-1.03, `videoplane-saver-result.md`), every moving element still at 1080p on the OSD above it, the hardware encoder is 1080p-only (`venc-result.md`), the vplane path not yet live on the set | Research only; not worth it |
| 7 | A 4K glyph or icon layer over a 1080p page | None exists: the only other graphics layer is the 128x128 cursor, which the driver itself scales x2 with the blitter | | Dead |
| 8 | A native 4K page | The whole UI 1:1 on the panel | § 4.1: closed by the driver, the boot option, LG's clients and the budget | Dead |
| 9 | The OSD sharpener | None visible (three runs) | | Dead |

## 5. What was built

- `rustui/ui/src/kit/lint.rs`: the `Soft` rule and `Lint::word_on` (a block's line judged by the block's layer), with tests;
  `home/widgets.rs`, `panels/ui.rs`, `panels/entry_surface.rs` feed the layer origin for blocks.
- `rustui/ui/src/home/surface.rs`: the dock pill on whole pixels; `icon_texture_side` and the tile icon at the lifted size,
  shown through `Props::scale`; `panels/quick_surface.rs`: the slider marks on whole pixels.
- `payload/compositor/lumatv/Tile.qml`: `mipmap: false`.
- `tools/layerdeep-tv.sh`: the `scaler` step (raw, confirm, dead-man, read-back revert) and the `shift` step.
- `docs/research/ui-sharpness/tile-mipmap-before-after.png`: the host render of the 120 px tile, 3x, before and after.

Not built, on the evidence: a 4K page option in the compositor and a render scale in the Rust UI engine (§ 4.1).

## 6. TV check

Round19 owns the set; these are reads, LumaTV's own switches, one user setting and one raw register. Camera on a tripod,
50 cm from the panel, the same spot (the status bar's clock and the first card's title, then a dock tile), manual focus and
exposure, one frame per state. Nitin at the set for `scaler`.

1. **The page against the OSD** (no change on the set). Photograph the same text under LG's Home and under LumaTV's Home
   (both 1080p through the same scaler). Expected: the same softness in both; LumaTV's text sharper or equal. If LumaTV's is
   softer, the page is the cause and § 3 is incomplete: note which surface.
2. `sh /tmp/lumatv/layerdeep-tv.sh observe`: `fb0` `mem 1920x1080 out 3860x2170`, `0xCC480310` in the regdump reads
   `0x00000000`, `osd_sr 1`, gain `0x10`.
3. `sh /tmp/lumatv/layerdeep-tv.sh scaler` (raw; the script prints the five writes and the revert file, waits for a typed
   `yes`, arms a 12 s dead-man). Each value holds 4 s (`HOLD=8` for longer): `0x10000000` cubic 1, `0x20000000` cubic 2,
   `0x30000000` cubic 3, `0x00000008` bilinear, `0x00000100` the GFX sampling mode. Expected readings: each write reads back
   `ok`; `fb0` unchanged; after the revert `0xCC480310` reads `0x00000000`, flips still rising. Photo points: the same text at
   each value. A broken or shifted picture at any value: say which, let the dead-man revert, stop. What to look for: a
   value where the stems' edges are one grey panel pixel instead of two (sharper), ringing (a dark line beside bright stems),
   or no change. Bilinear is expected softer than the default [INFERENCE]; the cubic selects are the candidates.
4. `sh /tmp/lumatv/layerdeep-tv.sh shift`: Screen Shift off for 20 s, then back. Expected: `hdr2` either stays
   `0x0f14087a` (3860x2170; the orbit canvas is not the setting's) or becomes `0x0f000870` (3840x2160, exactly x2) with the
   crop at (0, 0). Photo points: text before and at +20 s. Only a changed `hdr2` can change the picture.
5. After the next release with `Tile.qml` and the Rust tile change: the Netflix tile (192 px `extraLargeIcon`) at rest and
   focused, before and after, 50 cm. Expected: the letter's edges and the small text crisper; no shimmer when the dock scrolls.

Rust UI text: `tests/test_rust_home.py` on the host already holds every lint surface to `Soft`; nothing to read on the set.

## 7. Dead ends and open questions

Dead ends: the OSD sharpener (TV, no visible effect); a 4K page (§ 4.1); a second 4K-capable graphics layer (none: the
cursor is x2-scaled by the blitter); AFRC and AFBC change bandwidth, not resolution; decoding the QML icon at the lifted
size (measured no better than mipmaps, § 3.2).

Open: (1) the OSD scaler's kernel at filter select 0 and the three cubic selects (`scaler`); (2) whether Screen Shift off
gives the exact x2 output (`shift`); (3) whether a `pageprobe` at 3840x2160 is worth one look (it would need the probe to
accept a size above the mode and a 4K dumb buffer: 33 MB of CMA for a picture that is most likely broken; not recommended);
(4) `fontdue` has no hinting: at 24-28 px Manrope's stems fall on 2.2-2.5 px and come out as two grey columns where
FreeType's light hinting would snap one; a comparison photograph of LG's (Qt, hinted) and the Rust shell's text at the same
size would say whether a hinting rasteriser (`swash`/`skrifa`) is worth its weight in the atlas.
