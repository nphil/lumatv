# Hardware planes: video windows, the cursor plane, AFBC pages (HwPlanes, 2026-10-29)

Three features of the compositor, each behind a switch that is off by default, none run on the set (no TV access in this task). Evidence tags: `TV:` read on this set (file named), `SRC:` LG's published O22 tree under `/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/` (newer than the installed firmware: a claim from source alone is marked), `[INFERENCE]` not measured.

## Resume here

**State when this section was last written** (update with each push). Host-tested and pushed; nothing has run on the set. Switches, all off at start: `--page-alpha`, `--afbc`, `--pointer plane`, `--vwin off|idle|apps`, `--vwin-slots N`; every one can also be turned on while running with `ctl` (below), so a round needs no restart.

Done:

- Cursor plane: `src/pointer.rs` (policy and worker thread), `src/kms/cursor.rs` (`DrmCursor`: legacy `set_cursor2`/`move_cursor`, atomic plane commit as the fallback), `Kms::{set_pointer, set_pointer_mode, pointer_report}`, `State::pointer_sync`, `ctl pointer [plane|gpu|X Y]`, `ctl status` `pointerPlane`. Tests: `tests/pointer_plane.rs`, unit tests in `pointer.rs`.
- AFBC and alpha pages: `src/afbc.rs` (layout arithmetic, probe, bandwidth estimate), `kms::make_page`/`make_ring` with `PageKind` (`Xrgb|Argb|Afbc`), `Egl::dmabuf_modifiers`, `Kms::capture` reading AFBC pages through the GPU. **Live switching**: `ctl afbc on|off`, `ctl pagealpha on|off` -> `Output::set_page_kind` -> `Kms::convert_page`, which makes over the page the next composed frame draws into (never the one on screen or in a flip), one page per composed frame; the new page is made before the old is freed, so a set short of memory (a 1080p page is 8.3 MB, the CMA free was 3.5 to 12 MB at the reads on record) keeps the old layout and says why in `ctl afbc` (`pages.fallback`, `pages.kinds`, `pages.converting`); a failed conversion is not retried until the next ask. `afbc on` implies page alpha (AFBC pages carry it); `pagealpha off` is refused while AFBC or `--vwin` is on. `Kms::page_size()` is the size the pages are made at (LayerDeep2's reduced primary buffer changes it). Tests: `Ring::forget_page` (`damage.rs`), `tests/vwin.rs` (the ctl refusals).
- Video windows, compositor: `src/vwin.rs` (the allocator and the glue: `vwin_sync`, `vwin_command`, `vwin_ctl`, `vwin_status`, `vwin_topic`), `WindowType::VideoWindow` / `Layer::Video`, `Msg::Vwin` + `POST /v1/vwin` + topic `vwin`, the `videooutputd` status subscription, `ctl vwin [off|idle|apps|slots N|claim|move|release]`. Claims of kind `camera-full` (priority 50), `camera-popup` (40) and `pip` (30) take the display's windows by priority, never an app's; `--vwin-slots 3|4` lets the allocator hand out `win2`/`win3`, which the O22 source says have no window behind them (check V5 reads whether the set agrees). Tests: `tests/vwin.rs` (11), unit tests in `vwin.rs`.
- Video windows, the feeder: `gfn/feeder/src/feeder.c` `-w video` (16 px window of type `_WEBOS_WINDOW_TYPE_LUMATV_VIDEO`), `tests/test_gfn_feeder.py`. The helper passes `cameras.feederWindow` straight to `-w`: the config value `"video"` is all the helper needs (`helper/app/src/config.rs`), set for the round, not by default (`card`).
- Video windows, the shell: `rustui/ui/src/host/vwin.rs`: the pure model of the claims the screens want, now a SET (`Vwin::want(&[request], now)`, `grant(id)`, `free()`, `due_at()`): one post per claim, a move when only the rectangle changed, a release per claim dropped, a refused claim asked again only when the topic's world changed. `Pip` is the picture-in-picture surface API: `Pip::corner(id, w)` / `Pip::new(id, rect).stream(codec, w, h, fps).app(app).request()` is the claim; a screen asks for it every time it draws and draws stills when `grant(id).carries()` is false. 9 tests.
- The driver: `compositor/tools/tv-hwplanes.sh` (on the set as `/tmp/lumatv/hwplanes.sh`: read, alpha, pointer, afbc, layout, shot, feeder, vwin, sizes, stop) and `compositor/tools/tv_hwplanes.py` (puts it, runs a step in a PTY, `ctl WORDS`, `put-feeder`, `put-clip`, `fetch`, `compare`). The TV-round checklist below is complete.

Not done:

1. `Shell` asks for nothing on a video window yet: the pieces are in (`Shell.vwin` is fed the topic and the ticks and posts what `want` says) but `Shell::vwin_wants()` is not, because `cameras::app::App::live_claim()` (RustCameras' answer to what the full view wants: `LiveClaim::request()` is exactly the claim shape) is not on `main`. When it lands: `vwin_wants()` returns `self.cameras.app.live_claim().map(|c| c.request()).into_iter().collect()` plus the `Pip`s any screen has up; the grant goes back to the Cameras surface through `Vwin::grant("cameras-full")`. Until then step C1 of the checklist cannot be run; V1 to V7 and P/A steps can (they use `ctl`).
2. Whether a pip's z-order is above the main video is by `videooutputd`'s `zOrder` (main 0, sub 1), not by the claim's priority: the allocator hands the first free window, which is the sub window only when something holds main. Check V3.
3. The reduced primary buffer and the plane-offset orbit are LayerDeep2's; page conversion calls `Kms::page_size()`.
4. Final runs on the integrated tree: `cargo test -p lumatv-compositor --offline --no-fail-fast`, `cargo check --features ui`, `sh compositor/tools/build-tv.sh`.

**Decisions taken (conservative, all reversible):** every switch is off by default; `idle` mode never starts a second decoder next to an app's; the pointer's plane image is premultiplied until the TV says otherwise; claims are matched to windows by app id (empty app: the first video window without a claim); a claim outranks the wallpaper and the grant closes the wallpaper's window (as `wall_handover` does); a video window nobody claimed within 3 s is closed so its decoder is free; page conversion never frees the old page before the new one exists.

## What was found

### The holes (the finding that decides the rest)

`TV: docs/research/arch/scanout-hw-02.txt` (LumaTV's compositor holding the DRM master): `fb0/status` reads `hdr.global_alpha_en: 1`, `hdr.global_alpha: 0xff`; under LG's compositor (AB24 pages) the same field reads 0 (`scanout-live-01.txt`, `framerate/*/before.txt`). `SRC: core/fbdev/fbdev_drm.c osd_drm_flip_image`: "setup global alpha if pixel format is xRGB": for XR24 the driver turns the constant alpha on, so the OSD ignores every pixel's alpha. The compositor's primary pages are XR24 (`kms/mod.rs make_page(.., alpha=false)`), and a video hole is a clear to transparent black (`gl.rs clear_holes_before`): on the panel that is **opaque black**. The recorded symptoms fit: the VIDEO capture is the clip (round 15/16 wallpaper, correlation 0.998) while LG's DISPLAY capture is black in every hole (`slot-2026-10-28-r15/img/wallexp-v1-display.jpg` against `wallexp-v1-video.jpg`), and the Netflix DISPLAY capture of 2026-10-26 is black at the centre (mean 0.0) while the picture mode says it plays. The panel itself was never looked at. `--page-alpha on` makes the pages AR24 (the plane lists it, `drmplanes-01.txt`): the frame already holds correct premultiplied alpha. Whether the OSD blends premultiplied is check P2 below.

### Video windows

- Two windows, not four. `SRC: chip/o22/de/de_reg_o22.c:140` `NUM_WIN_MAX 2`; `win2`/`win3` exist as proc nodes and enum values, `DE_REG_O22_CviMap` gives win1 and win2 one clock select and win3 none. `TV:` `videooutputd`'s status lists exactly two sinks, `MAIN` (z 0) and `SUB0` (z 1) (`arch/x4/all.jsonl`); across the seven app runs of `framerate/` (Netflix 4K24, Plex 1080p24, YouTube 1080p and 4K, 24 to 60 fps) `MAIN` is connected and `SUB0` never is, `subScalerOccupied` is `false`.
- The one exception: LumaTV's own NDL session. `TV: round8/tv-2026-10-25/log.txt:392` (the saver, 1856x1024 at 60, low-delay and game mode on): `sink MAIN`, `subScalerOccupied: true`. If every NDL session does that, the sub window is never free next to LumaTV's own video. Check V2 reads it for the wallpaper's session.
- The sub window is small. `TV: gfn/tv-2026-10-26/device_codec_capability_config.json`: `multiview_sub` is 1920x1088 at 30 (VP9 at 60) for H.264, H.265, AV1; the main window is 4096x2304 at 60 (H.265 at 120). A camera's 4K main stream needs the main window.
- A 4K120 video takes both scalers (`framerate-result.md`: `V4L2_CID_EXT_VSC_OCCUPATION_SUB_SCALER`, `subScalerOccupied`). Resource table: one main scaler, one sub scaler, three display managers (`umediaserver_resource_config.txt`).
- Z-order: below every OSD pixel; between windows `videooutputd` orders by `zOrder` (main 0, sub 1: the sub window is over the main one). The compositor's `setCompositing` exists (`luna::video::set_compositing`) and is sent when a client sets the element's z index.
- Scaling limits: none in the source (`limited_win_ratio_*` always read 0, `v4l2_vsc_event.c:3143`). Unmeasured; check V6.
- The OSD mixer has three rectangular hardware holes (`osd_reg_o22.h` `reg_hole0..2`, x/y/w/h/alpha/reverse, per-input select): LG's drivers never program them. Not built; a register write needs the owner's approval.

### The cursor plane

`SRC: core/drm/drm.c`, `core/fbdev/fbdev_drm.c`, `chip/o22/fbdev/fbdev_hw_o22_cmn.c:2895-3050`:

- Plane 36 (`crsr_fb`, OSD3), AR24 only, at most 128x128. Positions are `unsigned` in the UI's 1920x1080 space (`osd_drm_set_cursor_region` follows the primary plane's size) and are scaled to the panel (x2); the image is scaled x2 as well, by LG's blitter, bilinear, only when the image's framebuffer changes (`detect_cursor_change` compares the struct, not the pixels).
- The hotspot is the framebuffer's `hot_x/hot_y`, which only `DRM_IOCTL_MODE_CURSOR2` sets; the display driver subtracts it and clips at the top and left edge ("H/W doesn't accept negative position"). An atomic plane update has no hotspot and a negative `crtc_x` becomes a huge unsigned value: that route clamps at the edge.
- The legacy cursor ioctls take the kernel's unsynchronised path (`drm_atomic_helper_update_plane` sets `legacy_cursor_update` for the CRTC's cursor plane, `drm_atomic_helper.c:2886`): no vblank wait, no `-EBUSY` against the flip in flight. An atomic commit through the `ATOMIC` ioctl does wait/EBUSY. Hence the legacy route first, on a thread of its own (the commit's `hw_done` still waits for the in-flight frame's fence).
- The buffer's width: `lg1k_drm_gem_fb_create` aligns pitches to 64 bytes and the legacy ioctl names the pitch as `width * 4`: 32 pixels (pitch 128).
- `TV: scanout-hw-02.txt`: `fb3/status` is `crsr_fb`, `visible: off`, 0 flips; `fbdev` module parameters `fbdev_cursor_x_comp`, `fbdev_cursor_y_comp`, `fbdev_cursor_ignore_fbmem_update` exist on the installed module. The installed driver is dated 2021-02-15, older than the source.

### AFBC pages

- The plane lists `0x0800000000000062` (AFBC 32x8 split sparse) for AB24 and XB24 (`TV: drmplanes-01.txt`); `lg1k_get_format_info` knows it for those two formats only; the OSD decodes one AFBC layout (the kernel passes "AFBC on", not the flags).
- LG's compositor scans such buffers out: AB24, 1920x1080, pitch 7680, 8,486,912 bytes, and a 768x2160 secondary of 6,742,016 (`TV: scanout-research.md` E3). The size is `align4096(superblocks * (16 + 1024))` with the height taken up to 1088 rows: 60 x 136 x 1040 = 8,486,400 -> 8,486,912, and 24 x 270 x 1040 -> 6,742,016, both exact (`afbc::buffer_size`, tested). Sparse layout: the memory a page takes does not shrink, only the bytes read and written.
- Not known: whether the 64-bit g24p0 blob draws into an AFBC image. `EGL_EXT_image_dma_buf_import_modifiers` is listed on both blobs (`arch/x2/x2-run-02-importers.txt`); `ctl afbc` / `--afbc probe` read `eglQueryDmaBufModifiersEXT` (check A1). The 32-bit r46p0 blob draws them (it made LG's buffers).
- CMA: three linear 1080p pages are 24.9 MB of a free CMA of 3.5 to 12 MB at the reads on record, so the ring cannot be swapped live (the new ring is allocated before the old is freed): the layout is chosen at start.

## TV-round checklist

To be completed (step 7 above). The commands will use the existing machinery (`sh compositor/tools/tv-first-light.sh deploy`, `python3 docs/research/arch/tv.py 'CMD'`, `compositor/tools/tv-kms-window.sh` cases).
