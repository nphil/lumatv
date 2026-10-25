# gfx64

The 64-bit GPU path for LumaTV's own processes: Rockchip's Mali G52 userspace driver (`libmali-bifrost-g52-g24p0-wayland-gbm`) running on Debian bookworm arm64 libraries, with a shim that supplies the dma-heap the driver expects. Background and measurements: `docs/PLAN.md` ("64-bit drawing path").

The kernel on the set (5.4) has no `/dev/dma_heap`; the driver allocates its window buffers from `/dev/dma_heap/system-uncached`. Contiguous buffers are what make the driver's window path fast (60.2-60.8 fps against 55.9-56.6 on ordinary pages).

| File | Role |
|---|---|
| `heapcore.h` | Allocation policy: DRM dumb buffer on `/dev/dri/card0` (contiguous CMA), then `/dev/dma_buf_te` when CMA is short. Libc-free, driven by an ops table. |
| `heapshim.c` | `LD_PRELOAD` interposer around `heapcore.h`: answers opens of `/dev/dma_heap/system*` and `DMA_HEAP_IOCTL_ALLOC`; `protected` and other heaps are absent; every other call passes straight through. |
| `launch.sh` | Runs a 64-bit client with the shim and the bundle; falls back to LG's 32-bit driver when the client fails to start. |
| `blob.lock`, `fetch-blob.sh` | Pinned inputs and the script that assembles the bundle into `build/bundle/` (ignored by git). |
| `build.sh` | Cross-builds `heapshim.so` and the probe into `build/out/` with zig. |
| `check.sh` | Host tests: policy against mock devices, interposer bookkeeping, launcher fallback logic. |
| `smoke.sh`, `smoke-tv.py` | On-set smoke test with the on-screen probe. |

## Bundle

`fetch-blob.sh` downloads, hash-checks against `blob.lock` and unpacks into `build/bundle/lib/`, with no binary in git:

| File | From |
|---|---|
| `libmali-bifrost-g52-g24p0-wayland-gbm.so` (56 MB) | JeffyCN/mirrors, branch `libmali`, pinned commit (g24p0-10) |
| `ld-linux-aarch64.so.1`, `libc.so.6`, `libm.so.6`, `libdl.so.2`, `libpthread.so.0`, `librt.so.1` | `libc6` 2.36-9+deb12u14 |
| `libgcc_s.so.1` | `libgcc-s1` 12.2.0-14+deb12u1 |
| `libstdc++.so.6` | `libstdc++6` 12.2.0-14+deb12u1 |
| `libffi.so.8` | `libffi8` 3.4.4-1 |
| `libwayland-client.so.0`, `libwayland-server.so.0` | `libwayland-client0`, `libwayland-server0` 1.21.0-1 |

The driver links `libwayland-client`, `libwayland-server`, `libstdc++`, `libgcc_s`, libc and friends and exports `wl_egl_window_*` itself, so no `libwayland-egl` is needed; `libdrm` and `libexpat` are not needed either. The script fails if a bundled library needs one that is missing. Debian drops superseded point releases from its pool, so a missing file is fetched from snapshot.debian.org by the same hash. The 64-bit loader is the bundle's own (`lib/ld-linux-aarch64.so.1 --library-path lib --preload heapshim.so client`); the set's `/etc/ld.so.preload` entry is 32-bit and is ignored with a harmless message. Bundle size: about 37 MB, mostly the driver.

Updating the driver or any library is a change to `blob.lock` that passes the gate on the set.

## Use

```
gfx64/fetch-blob.sh && gfx64/build.sh && gfx64/check.sh
gfx64/launch.sh --fallback /path/to/client32 -- /path/to/client64 ARGS...
```

The client reads `LUMATV_GFX_PATH` (`64` or `32`) and `LUMATV_GFX_LIB` (the libmali to `dlopen`) to know which path it is on. Shim environment: `LUMATV_HEAP=auto|dumb|te`, `LUMATV_HEAP_LOG=1` (every allocation; fallbacks and failures are always logged). `launch.sh` documents its options, the failure rule (non-zero exit within the grace period) and its overrides in its header.

On the set: `python3 gfx64/smoke-tv.py [seconds] [auto|dumb|te]` (or `sh /tmp/gfx64/smoke.sh` after upload).
