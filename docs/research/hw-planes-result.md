# Hardware planes: video windows, the cursor plane, AFBC pages (HwPlanes, 2026-10-29)

Three features of the compositor, each behind a switch that is off by default, none run on the set (no TV access in this task). Evidence tags: `TV:` read on this set (file named), `SRC:` LG's published O22 tree under `/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/` (newer than the installed firmware: a claim from source alone is marked), `[INFERENCE]` not measured.

## Resume here

**State when this section was last written** (work in progress; update this block with each push).

Done and pushed (host-tested, no TV):

- `compositor/crates/lumatv-compositor/src/pointer.rs`: the cursor-plane pointer's policy: `Mode`, `Alpha`, `plane_image` (the 28 px disc centred in a 32x32 ARGB image), `Backend` trait, `Plane` (worker thread, newest position wins, `set` returns whether the plane carries the pointer), `atomic_origin`. 9 unit tests.
- `src/afbc.rs`: `Mode` (`off|probe|on`), `PageKind` (`Xrgb|Argb|Afbc`), AFBC layout arithmetic (`buffer_size` reproduces LG's two buffer sizes, 8,486,912 and 6,742,016), `describe(modifier)`, `scanout_reading` / `estimate` (bandwidth), `Probe` (what the plane and EGL list). 8 unit tests.
- `src/vwin.rs`: the pure allocator: `Mode` (`off|idle|apps`), `Policy`, `limits(slot, codec)` (capability file), `Kind` priorities, `Reason`, `Request::from_json`, `World`, `Allocator::{request, settle, update, release, occupancy}`. 18 unit tests.
- `config.rs`: `--pointer gpu|plane`, `--pointer-alpha`, `--page-alpha on|off`, `--afbc off|probe|on`, `--vwin off|idle|apps`, `--vwin-slots N` (parsed and validated; `--vwin` and `--afbc on` imply `--page-alpha on`).
- `output.rs`: `Output::{set_pointer, set_pointer_mode, pointer_report, afbc_report}` with defaults (no plane).

**Not done yet (in this order; each is a self-contained step):**

1. KMS side of the pointer: `kms/mod.rs` finds plane type 2, `DrmCursor` backends (legacy `set_cursor2`/`move_cursor` first, atomic plane commit second) implementing `pointer::Backend`; `Kms::{set_pointer, set_pointer_mode, pointer_report}`; `Kms::open` takes the new options (`PageKind`, pointer mode) from `main.rs`.
2. `State::pointer_sync` (called from `seat::pointer_motion`, `set_cursor_visible`, `tick_at`): `scene.cursor` is `None` while the plane carries the pointer; the plane is hidden while a capture is armed and while the screen is off. `ctl pointer [plane|gpu|X Y]`, `ctl status` gets `pointerPlane`.
3. Pages: `kms::make_page` takes a `PageKind` (AR24 with `--page-alpha`; AFBC: dumb buffer of `afbc::dumb_rows` rows, `add_planar_framebuffer` with the modifier, GL target with the modifier; fall back to linear with the reason). `Egl::dmabuf_modifiers` (`eglQueryDmaBufModifiersEXT`) for `afbc::Probe`. `Kms::capture` reads AFBC pages through the GPU. `ctl afbc` and `afbc_report`.
4. vwin glue (`impl State` in `vwin.rs`): `WindowType::VideoWindow` (`_WEBOS_WINDOW_TYPE_LUMATV_VIDEO`) and `Layer::Video` (between Overlays and SystemUi) in `window.rs`; `Window::{vwin_live, vwin_rect}`; `scene.rs` (`build_stack` leaves a window with no grant out, `surface_at`, `shell_home_covered` and the gain flag skip it); `server/foreign.rs` `exported_changed` takes the grant's rectangle for a video window; `Msg::Vwin` + `POST /v1/vwin` + topic `vwin` in `shell.rs`/`shell_rust.rs`; `ctl vwin`; the `videooutputd` `getStatus` subscription (`LunaState` token) for `World::other_connected`/`both_scalers` and each claim's real sink.
5. Tests: `tests/pointer_plane.rs`, `tests/vwin.rs` (harness clients: a `LUMATV_VIDEO` window with an exported element and a punch-through, an app card with video), a page-kind test through the GL renderer if Mesa lists the modifier.
6. Hooks: `gfn/feeder/src/feeder.c` `-w video`; `rustui/ui/src/host/vwin.rs` (the shell's model of its claims, over `Effect::Channel` and the `vwin` topic); `helper` config note (`cameras.feederWindow: "video"`).
7. This document: the design sections below and the TV-round checklist (exact commands, expected readings, revert).

**Decisions taken (conservative, all reversible):** default of every switch is off; the AFBC ring is chosen at start (a live switch would need 25 MB of CMA the set does not have free); `idle` mode never starts a second decoder next to an app's; the pointer's plane image is premultiplied until the TV says otherwise; claims are matched to windows by app id (empty app: the first video window without a claim).

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
