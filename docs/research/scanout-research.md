# Client-buffer scan-out on the LG C4

The best first route is to keep the working Mali driver buffer protocol, retain the dma-buf descriptor it already receives, and promote a compatible, sole full-screen buffer to the primary plane. The first target should be LumaTV's own shell, whose allocation and format can be changed without changing every LG app. This could remove the measured 4.1 ms compositor GPU pass per eligible frame. It is not yet a demonstrated end-to-end path with the current driver-served buffers.

Two constraints matter more than the missing EGL export extension:

- A descriptor alone is insufficient. The display driver needs contiguous device-addressed memory and a layout its OSD decoder really understands. LG app buffers normally use a different AFBC layout from LG's own output buffers.
- **The advertised overlay is not a proven second graphics layer.** LG's O22 source maps fb0 and fb1 to the same OSD hardware and enforces mutually exclusive ownership. A shell-over-app plan must initially fall back to GPU composition, not assume that DRM plane 34 can blend over plane 32.

LG's normal app path is itself GPU-composed in the measured cases. There is no evidence that its 32-bit compositor generally scans app buffers out directly. Its decoded video and HDMI bypass the GPU through separate display-engine windows; those windows are not generic RGBA graphics planes.

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
| Overlay 34 | The same four formats and two modifier entries | Maps to virtual fb1/graphic1. O22 source aliases its hardware to fb0; simultaneous blending is not established. |
| Cursor 36 | AR24; cursor caps 128×128 | A cursor path, with driver-side scaling/copy behavior. Not a full-screen shell plane. |

The graphics planes expose `IN_FENCE_FD`; the CRTC exposes `OUT_FENCE_PTR`. Atomic operation and PRIME import/export are supported (`PRIME=3`), asynchronous flips are not. There is no configurable `zpos`, plane-wide `alpha`, rotation or pixel-blend-mode property in the recorded property set. The debug state’s `normalized-zpos=0` is not proof of independent layers or blending order.

The important correction to older repository comments is in O22 source:

- `kdriver/core/drm/drm.c:901–905` maps primary to graphic0 and overlay to graphic1; `core/fbdev/fbdev_drm.c:164–166` maps these to fb0 and fb1.
- `chip/o22/fbdev/fbdev_hw_o22_cmn.c:196–225`, `FBDEV_O22_CMN_GetHWXListEx`, maps fb0 to physical OSD 0 (or 0+1 for 4K splitting), and fb1 to the same hardware. The source explicitly describes fb1 as a virtual special-purpose/E-Streamer interface.
- The same file at `3383–3386` ignores a fb1 request while fb0 is enabled, and at `3583–3590` changes `hw_osd0_owner` between the two. `fbdev_hw_o22.h:541` names the shared ownership explicitly.
- `core/fbdev/fbdev_drm.c:528–534` disables fb1 when graphic0 is enabled. This is not merely a missing alpha property that userspace can fix.

Consequently the generic primary-under-overlay assumption in `kms/{mod,assign,frames}.rs` must not be used as C4 hardware evidence. The source is newer than the installed image, so a narrowly gated hardware proof could overturn this conclusion for the installed driver. Until then, the safe design treats plane 34 as an alternative output, not an extra RGBA layer.

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

### A. Keep driver compatibility; observe its incoming dma-buf — first choice

[INFERENCE] Add a libwayland protocol logger on LumaTV's own server display. For `mali_buffer_sharing.create_buffer`, duplicate the received descriptor inside the callback and retain dimensions, stride, fourcc and modifier in metadata associated with that client's new buffer ID. The driver must continue to create, own and import the resource. A buffer-destruction listener and plane-retirement tracking own the metadata's lifetime; replacing the driver's resource implementation or user data is not required.

The GL fallback must continue to treat the resource as `External` and import through `EGL_WAYLAND_BUFFER_WL`. `kms::direct_fb` can consult the separate metadata when considering a buffer for scan-out. This preserves the import path that works with native LG clients instead of repeating the incompatible own-global cutover. The protocol includes the descriptor already; EGL export is not intrinsically required. Metadata is captured once per allocation, not reconstructed or allocated every frame.

**Unblocks:** descriptor access without changing client protocol behavior. It does not fix scattered memory, incompatible AFBC, orientation or fences. First use an off-screen 256×256 LG-driver client on a private socket, record metadata only, then attempt PRIME import without a modeset. Compare the driver's `alloc_device` event with the own-global event as part of the compatibility question; its exact current value has not been captured here.

**Benefit:** up to the whole measured 4.1 ms compositor pass when a buffer passes all scan-out gates. No saving when the buffer remains ineligible. Cost: a narrow server-side metadata/lifetime addition plus an import-only probe, not a renderer rewrite. [INFERENCE]

### B. Make the controlled shell produce a compatible buffer

Use A with shell-only contiguous allocations and an explicitly verified layout, initially linear AB24/XB24. Candidates are the exporter's contiguous request or the existing DRM-dumb/CMA policy, applied only to LumaTV's client. `WAYLAND_AFBC=0` is a candidate client hint, not a demonstrated result: a private-socket run must confirm a zero modifier and correct pixels before it is relied on. If the driver cannot select the layout, a shell-specific GBM/client integration that supplies explicit buffers is the larger, deterministic option.

The existing 32-bit Qt shell need not be rewritten merely to retain the protocol's descriptor. Altering how it allocates or exports its render targets is additional client integration work. A 32-bit allocator shim would need the real b46 request ABI; replacing every application's allocator is neither necessary nor proposed.

**Benefit:** remove the 4.1 ms compositor copy when Home, its in-window Settings or its panels are one eligible window. `Shell.qml` already puts those Home panels in `homeWin`; the separate `sysWin` carries app overlays, volume, alerts and other system content. A visible second window still requires composition under the current hardware constraint. A video-wallpaper hole, transition, cursor or dimming effect can also make Home ineligible. Linear rendering may increase the client's own bandwidth/GPU cost, so net saving must be measured rather than assumed to be exactly 4.1 ms. Cost: controlled allocation/layout integration and about 25 MB for three linear 1080p client buffers, with CMA pressure and allocation stalls as the principal risks. [INFERENCE]

**Smallest proof:** first a 256×256 off-screen linear/contiguous client whose duplicate descriptor imports into `lg1k`; then a full-screen color/edge pattern in an approved display slot, followed by an ordinary Home slide. No all-app cutover is needed to answer the allocation/layout question.

### C. Standard linux-dmabuf or `wl_drm` PRIME

LumaTV already implements linux-dmabuf v3 and retains its descriptors in `DmabufBuffer`. A controlled client sending a compatible buffer can already reach the KMS direct path. The next implementation change is to offer/choose the scan-out intersection where appropriate: the current `server/dmabuf.rs` list contains linear and client AFBC `…151`, not the OSD's `…62`. GPU-import formats and direct-display formats must remain distinct.

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

A full-screen shell over a directly scanned app would be attractive, but source says primary/overlay share the hardware. `TEST_ONLY` success cannot alone prove simultaneous visibility: the driver can accept an atomic state and silently ignore one fb1 enable. Correct alpha, independent content and repeated updates on both layers must be seen together before enabling this path.

**Unblock:** evidence that this installed O22 driver has an independent graphics mix path despite the published aliasing, or a different real display-engine graphics route. Exposing such a route would be display-driver work, including mixer routing, format handling, fences, power transitions and recovery—not an extra KMS property. Changing the alias alone cannot create a layer. This is a separate, human-approved infrastructure/display experiment, not part of the first userspace proof.

The 128×128 AR24 cursor path could cover a genuinely cursor-sized sprite, not the full shell or volume layout. Feeding the video/JPEG windows requires LG's media/resource APIs and the supported media formats; an arbitrary RGBA app would need conversion or encoding, adding work and latency instead of automatically removing it. Those windows remain useful for existing video/HDMI and the video wallpaper. No protected-video extraction or allocator bypass is proposed.

**Practical fallback:** direct primary while a sole app/shell is eligible; compose when a system layer, pointer, effect or video hole needs it; return to direct when it leaves. This retains the saving for ordinary full-screen frames without betting the UI on plane 34. [INFERENCE]

## Required correctness and performance boundaries

- `scene::Plan::sole_fullscreen` already rejects multiple items, non-opaque content, dimming, transitions, gain maps, video holes, cursor and switcher backdrop. Preserve these exclusions initially. A transparent video-punch-through graphics path would need a separate proof, not relaxed opacity checks.
- `kms::direct_fb` currently only accepts `BufferKindRef::Dmabuf`, and does not call `primary_takes` before `ADDFB2`. Add a real layout/plane eligibility decision for retained driver metadata; permissive `ADDFB2` is not that decision. Refusals must leave the existing GL path usable.
- `kms/frames.rs`'s split renders the optional built-in Rust UI, not an arbitrary second Wayland `ShellSystem` item. Promoting the QML system window would need its own scene/assignment change even on hardware with a real second layer.
- An acquire fence must cover the client's rendering; the compositor's GL fence cannot substitute when GL did not sample that buffer. X2 found explicit-sync protocol tables in the client driver, but E2 did not advertise that global. Verify implicit dma-buf synchronization or implement and test the negotiated explicit route. Hold the scan-out buffer until a later flip has replaced it; only then release it for client reuse. Otherwise the display can read a buffer as the client overwrites it, causing tearing.
- Keep GL composition for rejected layouts, imports, effects and capture. The current capture path deliberately sets `compose_only`; direct output is not in the last composed page. An unknown/protected allocation is ineligible, not a request to CPU-map it. On failed promotion, preserve the last valid frame and clean up descriptors/GEM/framebuffers at the correct retirement point.

Removing 4.1 ms saves approximately 246 ms of GPU work per second at 60 eligible frames/s, or 492 ms/s at 120—24.6 or 49.2 percentage points of one GPU's time at the measured clock. That is not a wattage measurement. In round 15's approximately 8.2 ms pair of shell/compositor jobs, removing one is about a 50% reduction in that pair, leaving room inside 8.33 ms; app drawing, client-side layout changes and display deadlines remain. [E6; arithmetic]

## Next proof: exact commands and limits

### Read-only first; safe without taking the screen

These commands read the already-known nodes through the existing helper. No broad `/proc/lg` walk, secret/environment dump, output capture, register write or DRM commit is involved. A snapshot with inactive planes is not a failed active scan-out test; an active snapshot must wait for normal owner use, not wake the TV for this research.

```sh
python3 /tmp/lt/tv.py 'date -u; cat /sys/kernel/debug/dri/0/clients; cat /sys/kernel/debug/dri/0/state; cat /sys/kernel/debug/dri/0/framebuffer' 15
python3 /tmp/lt/tv.py 'cat /proc/lg/fbdev/fb0/fbmem; cat /proc/lg/fbdev/fb1/fbmem; cat /proc/lg/fbdev/viewctrl; cat /proc/lg/fbdev/pathctrl' 15
python3 /tmp/lt/tv.py 'cat /proc/lg/de/win0/status; cat /proc/lg/de/win1/status; cat /proc/lg/de/win2/status; cat /proc/lg/de/win3/status' 15
python3 /tmp/lt/tv.py 'cat /sys/kernel/debug/dma_buf/bufinfo' 15
```

Compare app-buffer sizes/exporters and display attachments with the OSD layout; do not infer contiguity from exporter name. E9 records the result of these reads in this session. Another passive snapshot can answer whether LG ever attaches a client-sized `…151` buffer to the display during ordinary use, but cannot by itself prove its pixel correctness.

### Smallest off-screen experiment; separate approval and instrumentation required

This is the next implementation/proof, not an experiment run by this research. First add the narrow metadata logger and import-only diagnostic described in A; the current binary does not provide those results. Stage that build only under an approved plan. The existing flags below isolate it from LG's socket, input and display; the server command creates a private directory/socket and allocates GPU memory, so it is **not** part of the read-only permission.

Server, in one terminal:

```sh
python3 /tmp/lt/tv.py 'mkdir -p /tmp/scanout-proof; D=/tmp/lumatv; L=$D/lib; XDG_RUNTIME_DIR=/tmp/scanout-proof $L/ld-linux-aarch64.so.1 --library-path $L $D/bin/lumatv-compositor --runtime-dir /tmp/scanout-proof --socket wayland-scanout-proof --renderer gl --driver $L/libmali-bifrost-g52-g24p0-wayland-gbm.so --buffers driver --shell off --seconds 12 --log-protocol' 20
```

After that socket exists, client in another terminal:

```sh
python3 /tmp/lt/tv.py 'XDG_RUNTIME_DIR=/tmp/scanout-proof WAYLAND_DISPLAY=wayland-scanout-proof /tmp/lumatv/bin/x2cli /usr/lib/libmali.so.0 --frames 6 --size 256x256' 15
```

The diagnostic must record one metadata entry per allocation, preserve the six-frame GPU-import pixel check from X2, and try/close only a PRIME import handle—no `SETCRTC`, atomic commit, master acquisition or pixel read of an unrelated app. A scattered-buffer import refusal is useful: it selects B's controlled-contiguous allocation experiment rather than disproving descriptor access. Then compare `WAYLAND_AFBC=0` on this same harmless client and the contiguous allocation candidate, reporting the actual returned modifier and import result rather than the requested settings.

### Smallest display proof; only after the two buffer gates pass

An owner-approved display slot is required. First show one unprotected full-screen test pattern on the primary, with corners, colored bands, alpha/opaque edges and orientation markers; compare the panel with GPU composition of the same buffer. Check two alternating buffers, acquire/release timing, late frames and a transition back to composition. Do not start with Netflix or use a successful `ADDFB2`/`TEST_ONLY` as the pixel verdict.

Only after that succeeds should the existing 120 Hz surface gate be used:

```sh
cd /data/home/lumaglass
python3 compositor/tools/tv_gate.py --only frames --refresh 120
```

That gate is not read-only: it changes the refresh divider, installs/runs the gate machinery and drives UI. It must use the established approval/recovery path and must not be run merely because it appears in this document. The shell overlay case needs its own two-visible-pattern hardware proof; until independent blending is established, the gate must exercise and expect composition fallback whenever a second graphics layer is required.
