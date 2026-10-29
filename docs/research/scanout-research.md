# Client-buffer scan-out on the LG C4

A compatible, sole, full-screen client buffer can go to the primary plane without the compositor's copy of it, 4.1 ms of GPU a frame on the set. That is built behind a switch that is off by default (`--scanout direct`, `ctl scanout`; § Built). What the display does with such a buffer is not yet shown on the set; § TV check list says how it will be. Whenever a buffer is refused, or anything else needs the screen, the frame is composed by the GL path as before.

Three facts decide the design:

- A descriptor alone is insufficient. The display driver needs contiguous device-addressed memory and a layout its OSD decoder really understands. LG app buffers normally use a different AFBC layout (`…151`) from the OSD's own (`…62`).
- **There is one graphics layer.** The installed driver ignores OSD1 while OSD0 is on, in its own log (§ Layers). A window over a directly scanned one is composed, and a side panel on OSD1 is not an option.
- LG's normal app path is itself GPU-composed in the measured cases. There is no evidence that its 32-bit compositor generally scans app buffers out directly. Its decoded video and HDMI bypass the GPU through separate display-engine windows; those windows are not generic RGBA graphics planes.

## Hardware layers

| Layer | Handles | What it is | With OSD0 on | For LumaTV |
|---|---|---|---|---|
| Video windows w0, w1 | `/proc/lg/de/win0..3/status`; the O22 source has `NUM_WIN_MAX 2`, so win2 and win3 are proc slots; no DRM plane | Display-engine windows fed by AVD, ADC, HDMI, VDEC or JPEG, PIP and PBP capable. Opaque pictures with no alpha, scaled by the engine, below every OSD. All four read zero when idle | Yes, below it | Video, HDMI, the video wallpaper. Never a layer over the UI |
| OSD0 | `fb0` `osd0_fb`; DRM primary plane 32; `viewctrl` OSD 0 (on) | The UI layer: 1920×1080 (virtual 1920×2160, two pages), scaled ×2 to 3840×2160 by the engine. AB24/XB24 linear or AFBC `…62`; per-pixel alpha and a constant alpha (`glalpha`, 0xff). About 157,000 flips since boot | n/a | The scan-out target |
| OSD1 | `fb1` `osd1_fb`; DRM overlay plane 34; `viewctrl` OSD 1 (off) | At most 512×2160 in the 3840×2160 panel space, a 256×1080 strip at UI scale (768×2160 buffer, 512×2160 shown); AFBC only; no constant alpha. The E-Streamer image in source. 24 flips at 23:25 UTC, 28 at 00:15 | **No.** The driver ignores its enable while OSD0 is on | Nothing |
| OSD2 | `fb2` `osd2_fb`; no DRM plane | 128×4, sysfs `state` 2 (the others 0), absent from `osdinfo`; the source has no hardware behind it | n/a | Nothing |
| OSD3, cursor | `fb3` `crsr_fb`; DRM cursor plane 36; `viewctrl` OSD 3 | 256×256 ARGB (the DRM plane caps it at 128×128), positioned in 1920×1080 and scaled ×2. Its own enable; 0 flips | Yes | A sprite up to 128×128: the pointer, a mute mark |
| 2D blitter | `/dev/lg/gfx0` (a `gfx1` node has no proc entry); `/proc/lg/gfx` | Memory-to-memory ARGB blend, scale, colour key and raster ops over physically addressed surfaces. 79 MB pool, 50 MB allocated. Open in pqcontroller, eplmanager and captureservice | Not a layer | Not for per-frame composition (§ Layers) |
| Mali G52 | `/dev/mali0` | Composes every frame today: one 1080p pass of 4.1 ms | Not a layer | The fallback |

Bottom to top the engine stacks the video windows, one OSD (OSD0 or OSD1, never both) and the cursor [INFERENCE for the cursor: its place was not read]. `pathctrl` (`bosd`, `sosd`) sets whether OSD and video share one path or have two; it is a mode of the whole screen, not a layer. Any write to `viewctrl`, `glalpha` or `pathctrl` is a display change and belongs in a test round (E3, E10, E11).

## Evidence key

`TV` means a read on this C4; `REPO` means a recorded TV result or current implementation; `SRC` means LG's published O22 source, which is newer than the installed firmware and must not be treated as a byte-identical running driver. `[INFERENCE]` marks a proposed mechanism or an unmeasured result.

| Key | Evidence |
|---|---|
| E1 | [Hardware census](hardware-census.md), [performance measurements](perf-tweaks.md), especially lines 355–381 and 617–630: LG compositor costs, devices and separate video path. |
| E2 | [LG protocol census, 2026-10-28](compositor/lg-wlprobe-2026-10-28.txt): `mali_buffer_sharing` v5, `wl_drm` v2 and `zwp_linux_dmabuf_v1` v3 are advertised. This is a registry census, not a trace of app buffer creation. |
| E3 | [KMS plane properties](arch/drmplanes-01.txt), [LG framebuffer state](arch/drm-debugfs-01.txt), [fbdev state](arch/lgproc-fbdev-01.txt), [DE state](arch/lgproc-gfx-de-01.txt). |
| E4 | [Recovered 32-bit protocol tables](arch/x2-libmali32-protocols.txt), [strings](arch/x2-libmali32-strings.txt), [X2 client selection](arch/x2/x2-run-01-logs.txt), [32/64-bit import matrix](arch/x2/x2-run-02-importers.txt), [successful 64-bit importer](arch/x2/x2-run-03-importer64.txt). |
| E5 | [C2/C3 results](compositor-c2-result.md), particularly the 2026-10-25 own/driver trials: an own-protocol WAM buffer reached the direct path, but several native clients crashed. |
| E6 | [120 Hz result](ui120-result.md), round 15 timeline at lines 202–236: the shell and compositor each have an approximately 4.1 ms fragment job. |
| E7 | `compositor/crates/lumatv-compositor/src/{buffer.rs,scene.rs,kms/mod.rs,kms/frames.rs,kms/assign.rs,render/egl.rs,render/gl.rs,server/mali.rs,server/dmabuf.rs,shell.rs}` and `compositor/shell/Shell.qml`. |
| E8 | `SRC` root `/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/`; paths and decisive functions below. The source is local research material, not copied into this repository. |
| E9 | Read-only TV observations on 2026-10-28 (first snapshot at 22:26:56 UTC), and offline ELF inspection, summarized below. No display change, app launch, install, restart, key press or buffer-import experiment was performed for this research. |
| E10 | [Read-only TV reads, 2026-10-28 23:21 to 2026-10-29 00:37 UTC](arch/scanout-live-01.txt): `osdinfo` of fb0, fb1 and fb3, `viewctrl`, `pathctrl`, `glalpha`, the DRM and fb holders, the kernel log's `fb(1)` lines, the DE windows, `/proc/lg/gfx`, the fb names and sizes. No write, no app launch, no display change. |
| E11 | `SRC` lines used by § Layers: `core/fbdev/fbdev_proc.c:471–483` (`viewctrl` calls `FBDEV_SetOSDEnable`), `core/fbdev/fbdev_drm.c:509–545` (`osd_drm_enable` forces OSD1 off when OSD0 turns on), `core/drm/drm.c:495–542` (the plane update), `chip/o22/fbdev/fbdev_drv_o22.c:236–274` (fb1's image by panel class), `include/gfx_kapi.h`, `core/gfx/gfx_drv.c:1340–1385` (a blend under the device lock, with retries), `chip/o22/gfx/gfx_drv_o22.c:170–225` (O22's configuration), `core/afbc_sw/afbc_sw_drv.h`. |

## 1. How LG gets an app to the screen

### Graphics: client dma-buf → Mali texture → LG output buffer → KMS → OSD

The supported path observed in X2 is:

1. LG's app renders with its 32-bit `libmali.so.0.46.0`, Bifrost r46p0. It binds `mali_buffer_sharing` v5, including when `wl_drm` and linux-dmabuf are also offered.
2. `create_buffer(new_id, width, height, stride, fourcc, modifier_hi, modifier_lo, fd)` transfers a real dma-buf descriptor. The recorded 1080p buffers were AB24, stride 7680, modifier `0x0800000000000151`, 8,986,624 bytes. AB24 is DRM's name for ABGR8888; a dma-buf is a kernel-managed shared allocation, not a copy of its pixels.
3. The driver owning the server-side protocol imports the resource using `eglCreateImageKHR(EGL_WAYLAND_BUFFER_WL)`. Both the 32-bit LG driver and the 64-bit g24p0 driver sampled all six X2 frames pixel-exactly. `EGL_EXT_image_dma_buf_import[_modifiers]` is also available for descriptor-based imports. Import support is not export support. [E4]
4. LG's compositor renders a scene into its own buffers. Recorded KMS state shows `surface-manager` owning imported AB24 output framebuffers, 1920×1080, stride 7680, modifier `0x0800000000000062`, 8,486,912 bytes. The distinct size and modifier from the client buffers, together with the measured full-screen-client composition cost, support this path; `allocated by = surface-manager` alone would not prove a buffer's origin. [E1, E3]
5. `lg1k_plane_update` obtains the imported GEM buffer's device address, adds its offset, and calls `osd_drm_flip_image`. That feeds the fbdev OSD code and its AFBC decoder, then the display engine blends graphics with video and scales the 1080p UI to the panel. KMS and LG fbdev are front ends to the same graphics hardware, not two independent output paths. [E8]

`perf-tweaks.md` measured 6.3 ms of GPU and about 5 ms of GUI-thread CPU per presented full-screen browser frame under LG's compositor with Home hidden. That is evidence against the premise that LG normally has a free direct-app path in these cases. It does not exclude an unobserved app-specific optimization. [E1]

### Video: a separate path, already used

LG's media services arrange decoder/HDMI output in display-engine windows. The Wayland foreign/video protocol communicates the rectangle and punch-through hole: graphics above the video are transparent there. `/proc/lg/de/win0/status` through `win3/status` report video dimensions, frequency, protected-video (`svp`) state and other video attributes. DE source selection lists HDMI, VDEC and JPEG, not arbitrary client RGBA dma-bufs. [E3, E7]

The four DE window status directories do not establish four simultaneously usable graphics overlays. Video routing, scaler availability and resource-manager ownership remain separate constraints. Protected video must remain in LG's authorized media pipeline; no proposed scan-out proof needs its pixels or secure allocation.

### Live read-only cross-check

The set was left in its as-found state. All three DRM planes had `fb=0`, all four DE windows had zero active dimensions, and `/proc/lg/fbdev/viewctrl` reported all OSDs off. Therefore this sample is not an active-playback or visible-overlay proof.

The retained allocations nevertheless distinguished the two paths:

```
DRM master: surface-manager, pid 13556
retained DRM output: AB24, 1920x1080, pitch 7680, modifier 0x0800000000000062
retained DRM secondary output: AB24, 768x2160, pitch 3072, same modifier
fb0/fbmem: 0x883ac000 7680 1920 1080 afbc 32x8 abgr
fb1/fbmem: 0x89234000 3072 768 2160 afbc 32x8 abgr
```

`/sys/kernel/debug/dma_buf/bufinfo` listed three 8,986,624-byte `dma_buf_te` buffers attached to `c4250000.gpu` twice each, without `soc:display`; two 8,486,912-byte and two 6,742,016-byte `surface-manager` buffers were attached to both GPU and `soc:display`. Exporter names and attachment lists are corroboration, not a mapping of every allocation to an app. Total: 132 objects, 94,650,368 bytes. `CmaFree` was only 3,500 kB despite `MemAvailable` of 805,064 kB: ordinary free RAM is not the contiguous display-buffer budget. [E9]

## 2. Which sharing and display routes exist

### KMS planes: three objects, not three general-purpose layers

The running set's recorded properties are authoritative for the exposed API. [E3]

| Object | Exposed formats and layout | Actual constraint |
|---|---|---|
| Primary 32 | AR24/XR24/AB24/XB24 linear; AB24/XB24 AFBC `0x0800000000000062` | One CRTC, 38; connector 31. Appropriate target for a sole compatible app or shell buffer. |
| Overlay 34 | The same four formats and two modifier entries | Maps to virtual fb1/OSD1. The installed driver ignores its enable while the primary is on (§ Layers): no simultaneous blending. |
| Cursor 36 | AR24; cursor caps 128×128 | A cursor path, with driver-side scaling/copy behavior. Not a full-screen shell plane. |

The graphics planes expose `IN_FENCE_FD`; the CRTC exposes `OUT_FENCE_PTR`. Atomic operation and PRIME import/export are supported (`PRIME=3`), asynchronous flips are not. There is no configurable `zpos`, plane-wide `alpha`, rotation or pixel-blend-mode property in the recorded property set. The debug state’s `normalized-zpos=0` is not proof of independent layers or blending order.

The important correction to older repository comments is in O22 source:

- `kdriver/core/drm/drm.c:901–905` maps primary to graphic0 and overlay to graphic1; `core/fbdev/fbdev_drm.c:164–166` maps these to fb0 and fb1.
- `chip/o22/fbdev/fbdev_hw_o22_cmn.c:196–225`, `FBDEV_O22_CMN_GetHWXListEx`, maps fb0 to physical OSD 0 (or 0+1 for 4K splitting), and fb1 to the same hardware. The source explicitly describes fb1 as a virtual special-purpose/E-Streamer interface.
- The same file at `3383–3386` ignores a fb1 request while fb0 is enabled, and at `3583–3590` changes `hw_osd0_owner` between the two. `fbdev_hw_o22.h:541` names the shared ownership explicitly. The installed build logs the same rule (its line 3401, § Layers).
- `core/fbdev/fbdev_drm.c:528–534` disables fb1 when graphic0 is enabled. This is not merely a missing alpha property that userspace can fix.

Consequently the generic primary-under-overlay assumption in `kms/{mod,assign,frames}.rs` is not C4 hardware evidence. The installed driver's own log agrees with the source (§ Layers), so the design treats plane 34 as an alternative output, not an extra RGBA layer, and the `split` path stays a diagnostic of the `ui` feature.

### Layout and allocation are separate gates

The two observed AFBC modifiers describe different memory organizations:

| Modifier | Meaning |
|---|---|
| `0x0800000000000062` | 32×8 blocks, split, sparse; advertised OSD AB24/XB24 layout and LG output layout. |
| `0x0800000000000151` | 16×16 blocks, YTR color transform, sparse, tiled headers; normal recorded LG client layout. |

`core/drm/drm.c:571–581` accepts linear, and permissively accepts non-linear AB24/XB24. At `517–532` it reduces the modifier to an AFBC/AFRC flag before sending the address and stride to fbdev. The complete AFBC flags do not reach that interface. O22 AFBC setup is in `fbdev_hw_o22_cmn.c:2359–2556`.

This explains why a successful `ADDFB2` for `…151` does **not** establish that the decoder displays that layout correctly. The own-buffer trial recorded `direct: fb 45 … modifier …151` and `direct 20`, but also five invalid atomic commits and no pixel-exact scan-out verdict. The earlier X2 pixel checks tested GPU sampling, not OSD decoding. Never relabel a `…151` allocation as `…62`. [E4, E5, E8]

The other gate is contiguity. `lg1k` uses `drm_gem_cma_prime_import_sg_table` (`core/drm/drm.c:1065–1071`). In `linux-rockhopper/drivers/gpu/drm/drm_gem_cma_helper.c:467–505`, adjacent scatter-list segments are allowed, but a device-address gap returns `EINVAL`. An arbitrary shareable dma-buf therefore need not be scan-out capable.

The b46 exporter source (`kdriver/core/gpu/kernel/b46/drivers/base/arm/dma_buf_test_exporter/dma-buf-test-exporter.c`) allocates ordinary buffers with individual `alloc_page` calls at line 621. `DMA_BUF_TE_ALLOC_CONT`, dispatched at 867, uses `dma_alloc_attrs` at 605. `/dev/dma_buf_unified` is registered from the same device implementation at 898–899. Contiguous allocation exists in source; availability and successful `lg1k` import on this firmware still need a small proof. The current gfx64 shim instead tries a DRM dumb/CMA allocation, then ordinary `te`/`unified` pages; the fallback can draw through Mali but cannot promise scan-out. [E8; gfx64-result.md]

`/dev/ion` exists, `/dev/dma_heap` does not. ION and HMA are allocators, not buffer-description or presentation protocols. Finding another allocator does not supply a missing modifier, fence or independent display plane. The 64-bit shim already bridges the absent system dma-heap for controlled clients; it deliberately does not implement a protected heap. [E9; gfx64-result.md]

### Driver and protocol capabilities

| Route | LG 32-bit r46p0 | Rockchip 64-bit g24p0 |
|---|---|---|
| `EGL_EXT_image_dma_buf_import` and modifiers | Listed in runtime X2 display extensions; sampling works | Listed; sampling works |
| `EGL_WL_bind_wayland_display` | Serves `mali_buffer_sharing` and `wl_drm`; X2 works | Serves `mali_buffer_sharing`; X2 works with GBM-null/surfaceless EGL |
| Supported `EGL_MESA_image_dma_buf_export` | Not in the complete X2 importer extension list | Not in X2 list or the inspected blob's string data |
| `eglExportDMABUFImage{Query,}MESA` symbols | Neither is defined in the live stock driver's dynamic symbol table | Both are defined, despite the absent extension; not a supported-export proof |
| Native GBM device | `/dev/dri/renderD128` succeeded in X2 | `/dev/dri/renderD128` and `/dev/dri/card0` both returned null in X2 |
| `gbm_bo_get_fd`, `gbm_bo_get_modifier`, `gbm_bo_import`, `gbm_surface_lock_front_buffer` | Defined in the live stock driver's dynamic symbol table | Defined in the inspected blob |
| Client protocol selection | X2 chose Mali even with linux-dmabuf and `wl_drm` present; removing Mali made EGL initialization fail | Blob includes linux-dmabuf support and a `MALI_WAYLAND_DMABUF_PROTOCOL` string; forced selection is unmeasured |

The 32-bit symbol check read the ELF file only; it did not load the driver. The 64-bit file inspected offline was 56,358,648 bytes, SHA-256 `15f94e34138ab315daab8d90b2bd17b792dc32d0b2206c6cc2efc670eff5f93d`. Its defined export functions are at virtual addresses `0x391f00` (query, 232 bytes) and `0x392000` (export, 352 bytes). They are not constant-failure stubs, but contain image-type checks involving `EGL_LINUX_DMA_BUF_EXT` and `EGL_NATIVE_PIXMAP_KHR`; exporting a Wayland-created EGLImage remains unproved. Unsupported entry points are a diagnostic possibility, not a production contract. [E9]

`gbm_bo_get_fd` is useful when the caller owns or imports a GBM buffer object. It cannot be applied to an arbitrary `wl_buffer` or EGLImage, and it does not make the failed 64-bit GBM-device creation succeed. LG's native GBM output path demonstrates a route to output buffers, not general export of any attached app resource.

LG's compositor advertises standard linux-dmabuf, but that does not establish that Netflix, Cobalt, Flutter or Qt clients select it. X2 directly demonstrates the opposite for LG's EGL client stack. The TV has `wayland-graphics-integration-server/liblinux-dmabuf-unstable-v1.so`; no corresponding client integration directory was present. The `QT_WAYLAND_HARDWARE_INTEGRATION=linux-dmabuf-unstable-v1` recorded on surface-manager is therefore not evidence of a switch that converts the shell's client buffers to linux-dmabuf. `wl_drm` v2 can pass PRIME descriptors, but merely advertising it did not redirect the recorded LG client. [E2, E4, E9]

## 3. Routes for LumaTV, benefit and smallest proof

### A. Keep driver compatibility; observe its incoming dma-buf — built

A libwayland protocol logger on LumaTV's own server display (`scanout::tap`) duplicates the descriptor of every `mali_buffer_sharing.create_buffer` and keeps its dimensions, stride, fourcc and modifier with the client's new buffer id (`scanout::meta`); a destruction listener and the plane's hold tracking own its lifetime. The driver still creates, owns and imports the resource, and the GL fallback treats it as `External` and imports through `EGL_WAYLAND_BUFFER_WL`, so the import path that works with native LG clients is unchanged and the incompatible own-global cutover is not repeated. The tap goes on only when the switch or the off-screen proof asks for it. [E4, E5]

**Unblocks:** descriptor access without changing client protocol behavior. It does not fix scattered memory, incompatible AFBC, orientation or fences: the eligibility rules (§ Built) refuse those, and the off-screen proof (§ TV check list a) says which of them the set's buffers have, with the driver's `alloc_device` event beside them.

**Benefit:** up to the whole measured 4.1 ms compositor pass when a buffer passes all scan-out gates. No saving when the buffer remains ineligible. Cost: a narrow server-side metadata/lifetime addition plus an import-only probe, not a renderer rewrite. [INFERENCE]

### B. Make the controlled shell produce a compatible buffer — plumbing built, layout unproven

Use A with shell-only contiguous allocations and an explicitly verified layout, initially linear AB24/XB24. Candidates are the exporter's contiguous request or the existing DRM-dumb/CMA policy, applied only to LumaTV's client. `WAYLAND_AFBC=0` is a candidate client hint, not a demonstrated result: `--scanout-shell-env WAYLAND_AFBC=0` (or `ctl scanout shell-env`) puts it in the shell client's environment, and the proof's `afbc0` variant asks the same of the 256×256 client. Whether the library returns a zero modifier, and whether lg1k imports the buffer, is what § TV check list (a) answers before the hint is relied on. If the driver cannot select the layout, a shell-specific GBM/client integration that supplies explicit buffers is the larger, deterministic option.

The existing 32-bit Qt shell need not be rewritten merely to retain the protocol's descriptor. Altering how it allocates or exports its render targets is additional client integration work. A 32-bit allocator shim would need the real b46 request ABI; replacing every application's allocator is neither necessary nor proposed.

**Benefit:** remove the 4.1 ms compositor copy when Home, its in-window Settings or its panels are one eligible window. `Shell.qml` already puts those Home panels in `homeWin`; the separate `sysWin` carries app overlays, volume, alerts and other system content. A visible second window still requires composition under the current hardware constraint. A video-wallpaper hole, transition, cursor or dimming effect can also make Home ineligible. Linear rendering may increase the client's own bandwidth/GPU cost, so net saving must be measured rather than assumed to be exactly 4.1 ms. Cost: controlled allocation/layout integration and about 25 MB for three linear 1080p client buffers, with CMA pressure and allocation stalls as the principal risks. [INFERENCE]

**Smallest proof:** § TV check list: (a) the off-screen client's descriptor imported into `lg1k`, (b) the full-screen pattern in an approved display slot, (d) an ordinary Home slide. No all-app cutover is needed to answer the allocation/layout question.

### C. Standard linux-dmabuf or `wl_drm` PRIME

LumaTV already implements linux-dmabuf v3 and retains its descriptors in `DmabufBuffer`. A controlled client sending a compatible buffer can already reach the KMS direct path, through the same `scanout::decide` as the driver's buffers. The next implementation change is to offer/choose the scan-out intersection where appropriate: the current `server/dmabuf.rs` list contains linear and client AFBC `…151`, not the OSD's `…62`. GPU-import formats and direct-display formats must remain distinct.

**Unblocks:** explicit metadata and an ordinary ownership model. For unmodified LG clients, the missing piece is a demonstrated client backend/selection setting, not another registry global. A new client buffer integration or a proven blob selection option costs client-specific work and an app-compatibility run. Legacy `wl_drm` buffer names also do not avoid this; only its PRIME request supplies a descriptor.

**Benefit:** the same up-to-4.1 ms eligible-frame saving, not an additional saving on top of A. **Smallest proof:** an off-screen controlled client sends one known linear contiguous buffer through linux-dmabuf; separately observe an LG client binding and creating through that protocol before proposing it as an LG-app solution. [INFERENCE]

### D. Export from EGL or a GBM buffer object

For the 64-bit blob, test the unadvertised MESA query/export functions only on a harmless off-screen image. Success must include a valid descriptor, actual fourcc/modifier/stride/offsets, ownership behavior and correct import—not just a non-null function pointer. For the installed 32-bit driver, these public dynamic symbols are absent; its supported alternative is a native GBM buffer object or the already-transferred protocol descriptor.

A native GBM output ring could replace the compositor's linear dumb render targets with scan-out-friendly AFBC buffers, reducing the cost of frames that still need composition. The 32-bit GBM path works; the 64-bit device-creation failure must first be diagnosed and resolved, or a small 32-bit allocation/output helper must transfer descriptors with a defined synchronization contract. Exporting a new GBM render target does not recover a descriptor for an unrelated external app image.

**Benefit:** eligible direct buffers can remove the 4.1 ms pass; an AFBC composition ring only makes that pass cheaper, by an unmeasured amount. It is not zero-copy app scan-out. **Cost/unblock:** one small off-screen export/GBM experiment first; a helper or compatible GBM backend if it fails, rather than an arbitrary driver replacement. Risks include unsupported image types, borrowed versus owned descriptors, wrong modifiers, and cross-process fence/lifetime errors. [INFERENCE]

### E. Own `mali_buffer_sharing` global

The repository already retains the descriptor with `--buffers own`; the WAM trial reached direct scan-out. It also crashed `tvhotkey`, HDMI/Flutter Home clients and repeatedly lost clients, while the driver-served path survived. This is not a ready global switch. [E5]

**Unblock:** recover the complete working driver's behavior, allocation-device exchange and resource semantics with A, then fix parity and run the native-client matrix. No evidence here isolates one missing request as the cause. **Benefit:** the same direct-frame saving if compatible allocations/layouts result. **Cost/risk:** broader protocol compatibility and lifetime work than passive metadata capture. The first proof should compare one off-screen native client under driver and own modes, not restart the live compositor or relaunch the whole app set. [INFERENCE]

### F. Shell hardware overlay, cursor and display-engine alternatives

A full-screen shell over a directly scanned app would be attractive, but primary and overlay are one OSD. The installed driver accepts the fb1 enable and silently ignores it while fb0 is on (§ Layers), which is also why `TEST_ONLY` success could never prove simultaneous visibility.

**Unblock:** nothing in user space. A second graphics layer would be display-driver work (mixer routing, format handling, fences, power transitions and recovery), not an extra KMS property, and a separate human-approved display experiment; changing the alias alone cannot create a layer. § TV check list (c) is the one write that could contradict the log.

The 128×128 AR24 cursor path could cover a genuinely cursor-sized sprite, not the full shell or volume layout. Feeding the video/JPEG windows requires LG's media/resource APIs and the supported media formats; an arbitrary RGBA app would need conversion or encoding, adding work and latency instead of automatically removing it. Those windows remain useful for existing video/HDMI and the video wallpaper. No protected-video extraction or allocator bypass is proposed.

**Built behaviour:** direct primary while a sole app or shell window is eligible; composed in the frame a system layer, pointer, effect or video hole needs the screen; direct again the frame it leaves. This keeps the saving for ordinary full-screen frames without betting the UI on plane 34.

## Boundaries the build keeps

- `Plan::sole_candidate` takes one picture that covers the output exactly and names what refuses it: more than one picture, the app switcher's backdrop, a transition, the pointer, a video hole or a window expecting one, a dim, the box dim's map, a window that is not the screen, a buffer transform or scale. Opacity is the surface's declaration or the buffer's format. A translucent frame, or one with a video hole, is not relaxed: it needs its own proof (§ Not built).
- A real eligibility decision (`scanout::decide`) stands in for `ADDFB2`'s permissiveness, and every refusal leaves the GL path to draw the same frame.
- `kms/frames.rs`'s split renders the optional built-in Rust UI, not an arbitrary second Wayland `ShellSystem` item; it is unchanged and belongs to the `ui` feature.
- An acquire fence covers the client's rendering: the GL side samples one texel of the buffer, and the commit carries that job's fence as `IN_FENCE_FD`, so the job waits on the dma-buf's implicit fences as composition does. X2 found explicit-sync protocol tables in the client driver, but the global is not advertised, so nothing is negotiated. A buffer the display reads, or will once a frame already drawn is committed, is not released to its client until a later flip has replaced it, and its framebuffer is destroyed only after.
- A screenshot or recording composes every frame (`compose_only`), since the capture reads the last composed page. A protected allocation is ineligible, and none is ever mapped by the CPU.

Removing 4.1 ms saves approximately 246 ms of GPU work per second at 60 eligible frames/s, or 492 ms/s at 120—24.6 or 49.2 percentage points of one GPU's time at the measured clock. That is not a wattage measurement. In round 15's approximately 8.2 ms pair of shell/compositor jobs, removing one is about a 50% reduction in that pair, leaving room inside 8.33 ms; app drawing, client-side layout changes and display deadlines remain. [E6; arithmetic]

## Layers

What the installed set does with the other hardware layers. Reads are E10; the source is the newer O22 tree (E8, E11), so a claim from source alone is marked.

### OSD1 beside OSD0: not on this firmware

The installed fbdev answered its one request for OSD1 while OSD0 was on in the kernel log (uptime 34175 s, E10):

```
FBDEV_O22_CMN_WriteOSDHeader 3282: fb(1) is not osd0 owner, ignore
osd_drm_enable 537: id=1 (fb1), view=0 -> 1
FBDEV_O22_CMN_SetOSDEnable 3401: ++ fb(1) on ignore. fb(0) is already enabled
```

- **Both at once?** No. The enable is accepted and ignored while fb0 is on, and `viewctrl` read `OSD 1 : 0` at every read from 22:26 to 00:37 UTC. In source the reverse holds as well: enabling OSD0 forces OSD1 off, "called by driver due to UI ON" (`fbdev_drm.c:526–535`), and `echo 1 1 > viewctrl` calls the same `FBDEV_SetOSDEnable` as the DRM path (`fbdev_proc.c:471–483`), so the proc knob is not a way around it. [INFERENCE for the proc path: source newer than the image; § TV check list c is the write that tests it.]
- **Order and blend?** None between OSD1 and OSD0: they exclude each other.
- **Which DRM object?** Overlay plane 34. `lg1k_plane_atomic_update` flips the framebuffer and then enables the OSD (`core/drm/drm.c:495–542`, SRC), so every commit that gives plane 34 a framebuffer counts a fb1 flip, shown or ignored.
- **Who made the 23 flips?** The log cannot say: the enable runs in a kernel worker, not in the process that committed. LumaTV's compositor only ever sets plane 34's `FB_ID` and `CRTC_ID` to 0 (without the `ui` feature it never gives the plane a picture), so LG's surface-manager is the likelier source: the buffer fb1 flipped, 768×2160 at 0x89234000, is its secondary output, and its address did not change between 22:26 and 23:21 [INFERENCE]. The counter read 1 at 22:26, 23 at 23:21, 24 at 23:25 (one enable in the log, at a start of the DRM master) and 28 at 00:15. None was ever shown.
- **Whose coordinates?** The 3840×2160 panel's. A `max_yres` of 2160 cannot exist in the 1080-high UI space; the source sizes fb1 at 13.3% of the panel's width for the 8K, 4K and FHD classes (1024×4320, 512×2160, 256×1080; `fbdev_drv_o22.c:236–274`), and OSD0 is the layer scaled ×2. 512 panel pixels are 256 UI pixels.

### A side panel on OSD1: the design, and why it is not built

The design asked for: Home or an app stays on OSD0 and goes direct, while the quick panel, the input picker or the volume bar slides on OSD1 as a window of its own, so that no frame of the slide is composed.

It cannot work on this set, and each of three reasons is enough:

1. The driver ignores OSD1 while OSD0 is on (above).
2. The strip is narrower than any panel. At most 256 UI pixels fit in OSD1; the quick panel's sheet is 648 wide (`QuickPanel.qml`), the notification drawer's 640 (`Drawer.qml`), the input picker is a 1792-wide sheet along the bottom and the volume bar a centred pill at least 440 wide (`VolumeOsd.qml`). The quick panel and the picker also dim the whole picture with a scrim, which a strip cannot do.
3. The panels are not windows of their own. Over Home they are layers of Home's window, and over an app they share the system window with everything else (`Shell.qml`). One over the other would be a new window class in the shell and a plane assignment in the compositor (the `split` path of `kms/frames.rs` takes the built-in Rust UI, not a Wayland window).

What the same goal costs instead, in the order to try:

1. Panels over Home need nothing: Home is one window while they slide, so with the switch on the slide frames are direct (§ TV check list d). This is the case round 15's 120 Hz numbers describe (quick `burst_over20` 14, picker 10).
2. The system window over an app is two pictures, so the frames it shows are composed. The compositor draws it only while it shows something (`live`), so the app is direct the rest of the time, and a composed frame draws only the tiles that changed.
3. The cursor plane is the one layer with its own enable beside OSD0: a sprite up to 128×128. It fits a mute mark or a small level mark, not a panel, and the pointer shares it.
4. A video wallpaper moves the biggest still part of Home to a video window below the OSD (round 15; § Video windows).

If § TV check list (c) shows OSD1 on beside OSD0 after all, the design is: the panel as a shell window of its own, at most 256×1080 UI pixels with no full-screen scrim, an AFBC `…62` buffer (OSD1 takes no other) that the scene assigns to plane 34 in place of the system window's item, the primary staying direct, and one atomic commit carrying both planes with an `IN_FENCE_FD` each. The stacking order and the blend would be the next measurements. That is a design for a result the log rules out; none of it is built.

### The 2D blitter

`/dev/lg/gfx0` is LG's memory-to-memory engine (E11). Its ioctls allocate a surface (in its own pool, or a preallocated buffer given by physical address), blend up to three source ports into a destination (premultiply, constant alpha, colour key, mask, raster ops, colour-space conversion, a hardware scaler: `b_hw_scaler` is true on O22) and wait for a sync. It does ARGB8888; an AFBC ARGB8888 enum (`0xafbc`) exists, but the hardware census lists no AFBC support and nothing shows that the blend block reads it.

As a function it is what a panel over Home needs: an alpha blend or fill of a 648×1080 sheet into a 1080p ARGB target. As a way to replace the Mali pass it fails on five counts:

- Memory. Surfaces are physical: its own pool (79 MB, 50 MB allocated and 28 MB free at the read) or a buffer given by physical address. A client's dma-buf has no physical address in user space, so Home's and an app's buffers cannot be sources without a driver-side step. [INFERENCE: root could read page frames from `/proc/self/pagemap`; not proposed.]
- Sharing. A blend runs under one device-wide lock (`gfx_dev_mtx`) with a 500 ms sync timeout and two retries (`gfx_drv.c:1340–1385`; `/proc/lg/gfx/synctm` read the same). The device is open in pqcontroller (panel care), eplmanager and captureservice, two fds each, so a stuck blend on a frame path would stall them. O22 sets all four stuck-workarounds to 0 (`gfx_drv_o22.c:205–225`), so the engine's soft reset (`gfx_drv.c:1380`) is not part of a normal blend.
- Synchrony. The blend returns after its sync and exports no fence for the display's `IN_FENCE_FD`, so the compositor's thread would wait for each frame.
- Cost is unmeasured and cannot be measured read-only. A full-screen 1080p blend moves about 25 MB (two reads and a write of 8.3 MB), 1.5 GB/s at 60 frames a second and twice that at 120, against the Mali pass of 4.1 ms. [arithmetic]
- `afbc_sw` (`/proc/lg/afbc_sw`) is not an alternative: it is the kernel's CPU fill, same-layout copy and top-bottom to side-by-side transform of AFBC memory, with no layout conversion and no blend (`afbc_sw_drv.h`).

Verdict: not for per-frame composition. One-off work, composing a still into a buffer, stays possible, and any timing needs the owner's approval since the block is shared with panel care and capture.

### Video windows

A video window carries what its source delivers: decoded video, HDMI or a JPEG still, as opaque pixels with no alpha, below every OSD. That makes it a background and nothing else: a wallpaper, still or looping (the round-15 video wallpaper), under an OSD0 that has holes where it shows. It cannot carry a panel or any RGBA content over the UI, and a source outside LG's media paths needs its services, conversion or encoding, which adds cost instead of removing it. For this build the use is the wallpaper: a Home whose wallpaper is a window has holes, and holes are refused as `video`, so Home with a still wallpaper can go direct and Home with a video wallpaper is composed (§ Not built).

## Built

All of it is in `compositor/crates/lumatv-compositor/` (`src/scanout/`, and `src/kms/mod.rs`, `src/scene.rs`, `src/server/state.rs`, `src/control.rs`, `src/config.rs`, `src/main.rs` where it meets the rest) with the tooling named below.

**Switch.** Off unless asked. `--scanout off|direct` at start (direct needs `--renderer kms`) and `ctl scanout off|direct` while running; the screen is redrawn through the new path in the next frame. `--scanout-acquire sample|none` and `--scanout-trust 0xMOD` are for experiments: `none` trusts the commit to come after the client's last pixel (a frame may be shown half drawn), `trust` tries a modifier the plane does not list. `ctl scanout shell-env NAME=VALUE...|none` (or `--scanout-shell-env`) sets the shell client's extra environment for its next start (`ctl shell restart`). `lumatv-sm` passes no `--scanout`: a production start is off until the check list says otherwise.

**Report.** `ctl status` carries `scanout` and `ctl scanout` prints it: `mode`, `frames` (direct, composed), `refused` (frames per reason key), `transitions`, `last` (the state, the buffer or the reason), `lastBufferRefusal`, `held` (shown, pending, waiting), `framebuffers` (made, waiting, imports, importFailures), `descriptors` (retained, created, destroyed, `allocDevices`), `tap`, `probe`, `shellEnv`. The journal (`ctl journal`) gets a line for each change: `scanout: direct (1920x1080 XB24 linear as fb N)`, `scanout: composed (reason)`, `scanout: refused: reason` once per distinct reason about a buffer, `scanout: fb N for buffer ID (...)` for each import, `scanout: refused: fb N at commit: ...`, `scanout: <mode> (switch)`.

**Decision.** `Scanout::choose` runs once a frame and the first rule that fails is the counted reason:

| Rule | Keys | Meaning |
|---|---|---|
| Switch, capture | `off`, `capture` | the switch is off; a screenshot or recording is armed |
| Plan | `empty`, `layers`, `switcher`, `transition`, `cursor`, `video`, `dimmed`, `gain`, `geometry`, `transformed` | not one picture covering the output with nothing over or under it, as `Plan::sole_candidate` says |
| Mode | `no-mode` | the session's first commit, which carries the mode, has not gone out: that frame is composed |
| Source | `shm`, `no-descriptor` | not a dma-buf; a driver buffer the tap did not see |
| Shape | `protected`, `upside-down`, `planes`, `offset` | one plane, offset 0, rows top-down |
| Layout | `format`, `modifier` | AR24, XR24, AB24 or XB24 with a (format, modifier) that the plane's `IN_FORMATS` lists: linear, or AFBC `…62` for AB24 and XB24. The client's `…151` is refused here: the driver would accept it and decode it as `…62` |
| Size | `size`, `stride`, `too-small` | the mode's size, pitch `width × 4`, a dma-buf of at least `stride × height` bytes |
| Opacity | `translucent` | the surface declared the picture opaque, or the format has no alpha (XR24, XB24) |
| Display | `import`, `addfb`, `test-commit` | PRIME import into lg1k (a buffer of scattered pages fails with `EINVAL`: that is the contiguity test), `ADDFB2`, and the driver's own test-only atomic commit of the plane with the framebuffer |
| Acquire | `acquire` | the GL side could not make the barrier |

A refusal about the buffer is remembered for it, so the import is not tried every frame; a refusal about the screen is not. A commit the driver refuses after all remembers the buffer as `test-commit`, and the next frame is composed. `ui-layer` counts a frame the plane could have taken that the UI's own layer (the `ui` feature) composed anyway.

**Descriptors.** `scanout::tap` installs libwayland's protocol logger on the C display and reads `mali_buffer_sharing.create_buffer` (duplicating the fd), `wl_buffer.destroy` and the `alloc_device` event; `scanout::meta` keeps each by (client, buffer id) until the buffer or the client goes. The driver still creates and owns the resource. The tap exists where the compositor runs on libwayland's C server (the set's build); `tap` in the report says `off`, `active` or why it is unavailable. It goes on at start with `--scanout direct` or `--scanout-probe`, or at `ctl scanout direct`; a buffer made before that has no descriptor and is composed (`no-descriptor`) until its client makes new ones.

**Off-screen proof.** `--scanout-probe` records every driver-served buffer's descriptor and PRIME-imports it into lg1k from a descriptor opened without master, then closes the handle: no framebuffer, no mode set, no commit. `compositor/tools/tv-scanout-proof.sh` (`/tmp/lumatv/scanout-proof.sh` in the bundle, run by `tv-first-light.sh scanout-proof`) runs it with LG's 32-bit libmali client `x2cli` at 256×256 on a private socket under `/tmp/scanout-proof`: variants `stock`, `afbc0` and `big` (§ TV check list a).

**Tooling.** `x2cli --pattern --opaque` (the check list's picture, and an alpha-free window); `tv-kms-window.sh` cases `b` (LG's client in its own layout, `--scanout-trust`) and `c` (route A: the driver-served pattern, direct and then composed), run by `tv-first-light.sh kms [CASE...]`; `gate.sh scanout [direct|off|read]` and `tv_gate.py --scanout direct`.

**Tests.** Unit tests in `scanout` (each rule with its reason, the experiment flag, descriptors, the tap's signature and arguments, the probe's records); `tests/scanout.rs` under real clients and `State::tick` with a fake plane (a taken buffer is direct and a refused one composed in the same frame, every rule on a real buffer, a system layer composing the frame it appears in and direct again when it leaves, the shell's Home as a linear buffer, what else the plan has, no release and no framebuffer destruction while the display reads a buffer, a display refusal at import and at commit, the barrier, the mode not set, `ctl scanout`); `tests/tap.rs` against the host's real libwayland-server; `tests/scanout_gl.rs` on Mesa (the barrier leaves the renderer as it found it); the gate's and the cutover's Python tests.

## Not built

- Translucent and video-hole frames. The OSD blends alpha over video natively, so a Home with a video wallpaper could go direct as AB24 with its holes; the build refuses them (`translucent`, `video`) until the opaque case is shown right on the panel.
- Explicit sync. The acquire is a GL sample on implicit fences (§ Risks).
- A second graphics layer, the OSD1 panel, composition on the blitter (§ Layers).
- A persistent switch. `lumatv-sm` starts the compositor without `--scanout`.

## TV check list

Order: (a) needs no display time; (b) needs a maintenance window, (c)'s write a free set and (d) a gate session, each only with the owner's go. (a) and (b) decide whether (d) is worth a session. Commands run from the repository root with the set at its usual address; `python3 docs/research/arch/tv.py 'CMD'` runs `CMD` on it.

### a. Off-screen proof: no display change

```sh
sh compositor/tools/tv-first-light.sh deploy        # builds the bundle and copies it to /tmp/lumatv, the test copy: not while a round uses it
sh compositor/tools/tv-first-light.sh scanout-proof stock afbc0 big
python3 docs/research/arch/tv.py 'cat /tmp/scanout-proof/out/*/verdict.txt'
python3 docs/research/arch/tv.py 'sh /tmp/lumatv/scanout-proof.sh clean'
```

It touches a private directory and socket under `/tmp/scanout-proof`, a GL context on `/dev/mali0` for a few seconds a variant, and a PRIME import into `/dev/dri/card0` opened without master; `big` holds three 9 MB buffers of CMA. LG's compositor, the screen and the input are not touched. A variant's verdict lists each buffer (layout, size, stride, exporter, lg1k's answer), the `alloc_device` path the driver gave the client, the six-frame pixel check (`check ok 6 bad 0`) and CmaFree before, during and after.

| Result | Means | Next |
|---|---|---|
| `stock`: AB24 `…151`, import ok | lg1k takes the memory; the plane's list refuses the layout | (b), case b: does the OSD decode `…151` at all |
| any variant: import refused, `Invalid argument` | scattered pages | route B needs a contiguous allocation (the exporter's contiguous request) before anything else |
| `afbc0`: linear, import ok | `WAYLAND_AFBC=0` gives the plane's layout; route B is open | (b), case c; then (d) |
| `afbc0`: still `…151` | the library does not honor the hint | route A has no eligible buffer until the layout can be chosen another way |
| `big`: CmaFree falls by more than the buffers | CMA pressure decides whether linear buffers are affordable | measure again with Home and an app up |

### b. Full-screen pattern in a test round: the display changes

```sh
sh compositor/tools/tv-first-light.sh kms b c       # about 30 s a case: LG's compositor stops (the panel is black), LumaTV's takes the screen, LG's starts again
```

Needs a free set, the owner's go and the bundle of (a) deployed, so that the compositor and `x2cli` are this build's (an older `x2cli` ignores `--pattern` and draws a colour clear). Results land in `/tmp/lumatv-tv-out/out/kms/`: `panel-b.png`, `panel-c.png`, `panel-c2.png` (LG's capture service, 960×540), `b-server.log`, `c-server.log`, `c-client.log`, `c-scanout-direct.json`, `c-scanout-off.json`, `c-scanout-end.json`, `c-status.json`. An interrupted run is ended by `python3 compositor/tools/tv_kms.py restore`.

The picture (`x2cli --pattern`, 1920×1080): eight bars along the middle third (white, yellow, cyan, green, magenta, red, blue, black), a white 4 px frame, a marker in each corner (top left red, top right green, bottom left blue, bottom right white), an orange bar that moves 16 px a frame along the middle, and the frame number in twelve bits as a row at the top and the same row at the bottom.

- **Case c, route A** (`--buffers driver --scanout direct`; the client with `WAYLAND_AFBC=0` and no alpha). `c-scanout-direct.json` says `"mode":"direct"`, direct frames in `frames`, `"last":{"state":"direct"…}` with the layout the plane took, and `held.shown` true. `panel-c.png` shows the picture as described, with colours and orientation right (red at the top left). `panel-c2.png`, taken after `ctl scanout off` with the same buffers composed, shows the same picture. In both, the number rows at the top and the bottom agree: a mismatch is a frame shown torn or half drawn, which is the acquire's test. `c-status.json` has no `import`, `addfb`, `test-commit` or `acquire` count.
- If `c-scanout-direct.json` says composed, `refused` names why: `no-descriptor` (the tap missed the buffers), `modifier` (the client still made `…151`; (a) says so), `translucent` (libmali refused the alpha-free window: `c-client.log` says `no config`), `import` (scattered pages).
- **Case b, LG's client in its own layout** through the own-protocol path (`--buffers own --scanout-trust 0x0800000000000151`). `panel-b.png` answers whether the OSD decodes `…151`: a right picture means the layout could be trusted for the formats that worked (a rule change, never a relabel as `…62`); a scrambled or striped one means the driver decodes it as `…62`, as the source says (the expected result); black means the plane refused it, and `b-server.log` says at which step.

### c. OSD1 beside OSD0: the one write that could contradict the log

Read-only first, any time:

```sh
python3 docs/research/arch/tv.py 'cat /proc/lg/fbdev/viewctrl; dmesg | grep "fb(1)" | tail -4; grep -E "^FB|fb_flip_cnt" /proc/lg/fbdev/osdinfo'
```

It reads `OSD 1 : 0`, the `fb(1) on ignore. fb(0) is already enabled` line after a start of the DRM master, and the flip counter of FB(1) (28 at 00:15 UTC); read again around the next start of LG's surface-manager, it settles who flips it.

The write is a display change, for a test round, with any picture on the panel (OSD0 is on whenever Home or an app shows):

```sh
python3 docs/research/arch/tv.py 'echo 1 1 > /proc/lg/fbdev/viewctrl; sleep 1; cat /proc/lg/fbdev/viewctrl; dmesg | tail -4; echo 1 0 > /proc/lg/fbdev/viewctrl'
```

Expected, from the log and the source: `OSD 1 : 0`, a new `fb(1) on ignore. fb(0) is already enabled`, nothing on the panel. That closes the side-panel design (§ Layers). If `OSD 1 : 1` instead, OSD1 is on beside OSD0: look at the panel and capture it (the strip shows LG's retained 768×2160 buffer, content unknown), restore with the last command, and take § Layers' conditional design as the next round's work, starting with `echo 1 80 > /proc/lg/fbdev/glalpha` and the stacking order. `pathctrl` is not touched in either case.

### d. Home slide at 120 Hz with the switch on, against round 16

```sh
python3 compositor/tools/tv_gate.py --only frames --refresh 120 --scanout direct
```

A gate session: it restarts the shell and changes the refresh divider, then puts it back (`docs/research/ui120-result.md`). In order: `gate.sh scanout direct` (the shell asked for `WAYLAND_AFBC=0`, the switch on, the shell started again so that its buffers pass the tap), `gate.sh refresh 120`, the surface frame gate (`frames-120-scanout.json`, trace tag `c4shell-120-scanout`), `gate.sh scanout read` (`scanout-direct.json` and `scanout-read.json` in the gate's output directory), `gate.sh refresh 60`.

The baseline is round 16's `frames-120.json`, the same gate without `--scanout`. Compare per surface `burst_over20` and `dropped` (round 15: quick 14, picker 10, toast 3, home-dpad 2), `late` over all frames and the idle pipeline's p99, and the Mali busy time per refresh in the `GATE refresh` lines. The compositor's 4.1 ms pass should be gone in direct frames, and the shell's own render may cost more into a linear buffer: the sum is the measurement.

Read `scanout-read.json`: `frames.direct` against `composed`, and `refused` for what composed the rest: `layers` (the system window was live over Home: the volume display shows over everything), `modifier` (the shell still made `…151`), `no-descriptor` (a buffer made before the tap), `video`, `cursor`, `dimmed`, `translucent`. Pass: the slide frames are direct, `late` and `burst_over20` are below round 16's, and there is no `import`, `test-commit` or `acquire` count. Tearing and black frames are looked for on the panel through LG's capture service, as in (b); the compositor's own `screencap` composes every frame it takes.

## Risks

- Implicit sync is assumed. The acquire sample waits for the dma-buf's fences only if the Mali import attaches them; (b)'s number rows are the test, and `--scanout-acquire none` is only for a measurement.
- Contiguity and CMA. A scattered buffer is refused at import (`import`); three linear 1080p client buffers cost about 25 MB against a CmaFree of 3.5 MB at the earlier read, so allocation stalls are the principal risk of route B.
- Orientation. A bottom-up buffer is refused (`upside-down`); Qt's windows are expected upright, and the counter will say.
- Layout. `…151` stays refused by the plane's list; `--scanout-trust` is an experiment, and an accepted `ADDFB2` is never a pixel verdict.
- Transparency. Qt's windows are alpha buffers. The scene counts Home's picture opaque by definition (it fills its window) and an app's only if the app says so. A video wallpaper puts holes in Home, which is composed (`video`).
- Protection. A descriptor carries no protection flag. Protected video lives in display-engine windows, whose holes are refused, and the proof records every buffer's exporter (`dma_buf_te` expected) so that another allocator is seen before anything is shown. No buffer is mapped by the CPU.
- The OSD controls and the blitter are untouched: every `viewctrl`, `glalpha`, `pathctrl` and blitter write is for an approved round.
