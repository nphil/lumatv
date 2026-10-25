# gfx64: 64-bit Mali path (heap shim, launcher, blob bundle)

## Resume here

State (2026-10-25): done and pushed in `gfx64/`. Smoke run on the set by the main agent; this run never touched the TV.

Measured on the set (main agent, 2026-10-25), probe x1client through `launch.sh` and the shim, over idle Home:
- Screen on, 8 s: 60.63 fps, 4 frames over 20 ms, worst 24.5 ms, three dumb buffers allocated (0.8-2.5 ms each).
  Inside the PLAN range (60.2-60.8 fps, 2-10 over 20 ms).
- Screen on, 20 s: 59.81 fps, 6 frames over 20 ms, worst 29.8 ms, buffers: dumb 1 + dma_buf_te 2 (CMA short, so the
  fallback ran as designed). Not a verdict run; it carries LG's compositor floor and a mixed buffer set.
- Set in Active Standby (panel off): 152.7 fps, unpaced frame callbacks; the probe passed its verdict although the
  number means nothing. Fixed: `smoke.sh` now exits 2 with "VERDICT FAIL" unless tvpower's `getPowerState` is
  "Active", and `smoke-tv.py` refuses before uploading anything. Retest only with the screen on.

- Host check, passes: `gfx64/fetch-blob.sh && gfx64/build.sh && gfx64/check.sh` (policy against mock devices incl. 8
  racing threads, interposer fd bookkeeping under LD_PRELOAD, launcher fallback logic with a fake loader and clients).
- TV smoke (rerun): when the set is idle and the screen is on (`getPowerState` = Active): `python3 gfx64/smoke-tv.py 20 auto`. It uploads about 37 MB to volatile `/tmp/gfx64`, runs
  the on-screen probe (x1client, the wl.c lineage) as a full-screen overlay through `launch.sh` and the shim, an 8 s run
  then a 20 s run, and prints per run: frames, fps, frames over 20 ms, worst gap, shim allocations by source, CmaFree
  before/after, plus a VERDICT line for the 8 s run. Output is also saved to `gfx64/build/smoke-<time>.txt`.
  Optional second pass `python3 gfx64/smoke-tv.py 8 te` for the dma_buf_te fallback numbers.
- Expected (PLAN.md, 8 s over idle Home): dumb buffers 60.2-60.8 fps, 2-10 frames over 20 ms, shim log "via drm dumb";
  dma_buf_te 55.9-56.6 fps, 35-41 frames over 20 ms. The 20 s run also carries LG's compositor floor (X1: 51.2 fps, 17 %
  of frames late); a shortfall there is not the shim. VERDICT PASS needs probe exit 0, no failed allocation, dumb only
  and >= 59.5 fps (PASS (fallback): te only and >= 55 fps).
- Clean up after the run: nothing persistent; `pgrep -x x1client` must be empty (smoke.sh kills leftovers).
- If the smoke fails with a loader/lib error: the bundle drops `libdrm` and `libexpat` that the earlier tested layout
  (`/data/home/tmp/m64dl/deploy/lib`) carried; nothing in the blob's or Wayland libs' NEEDED lists asks for them
  (fetch-blob.sh checks the closure), but add `libdrm2` (2.4.114-1+b1) back in `blob.lock`/`files_of` first if a
  missing-library error names it.

## What was built

`gfx64/` (layout and usage in `gfx64/README.md`):

- `heapcore.h` + `heapshim.c`: the shim. Opens of `/dev/dma_heap/system*` get a `/dev/null` placeholder fd (tracked
  in a 4096-bit atomic bitmap, also across dup/dup2/dup3); `DMA_HEAP_IOCTL_ALLOC` on it tries a DRM dumb buffer on
  `card0` (one 4 KB row per page, exported as a dma-buf, GEM handle closed at once), then `dma_buf_te`; other heaps
  (`protected`) are ENOENT; ENOMEM when both fail, as the kernel heap does. Hot path cost: an ioctl is one compare
  against the heap request number, a close is one bit test; no locks, no allocation; the devices are opened once
  (compare-and-swap, a losing opener closes its copy). The policy keeps no "CMA exhausted" memory: each new buffer
  tries contiguous first (CMA frees when video stops). Allocation runs only when the blob creates a buffer (30-48 ms
  when pages migrate); the log shows the time. `LUMATV_HEAP=auto|dumb|te`, `LUMATV_HEAP_LOG=1`; fallbacks and failures
  are always logged.
- `launch.sh`: preflight (64-bit kernel, bundle, shim, client, `/dev/mali0`, `/dev/dri/card0` or `/dev/dma_buf_te`),
  then the client under the bundle's loader with `--preload`. Failed start = non-zero exit or signal within the grace
  period (default 5 s) -> runs the fallback command with the same arguments and `LUMATV_GFX_PATH=32`,
  `LUMATV_GFX_LIB=/usr/lib/libmali.so.0`. A later crash returns the client's code (the caller's watchdog owns restarts).
  69 when unavailable and no fallback. SIGTERM stops the client, exit 143, no fallback. `LUMATV_GFX64_DISABLE=1` forces
  the fallback.
- `blob.lock` + `fetch-blob.sh`: Rockchip blob pinned by mirror commit 077bc8f5 and sha256; six Debian bookworm arm64
  debs pinned by sha256, pool then snapshot.debian.org by hash; verified identical in size and content to the tested
  `/data/home/tmp/m64dl/deploy/lib` for libc, ld.so, libstdc++; closure check of NEEDED libs.
- `build.sh` (zig aarch64-linux-gnu.2.36: heapshim.so, x1client), `check.sh`, `smoke.sh`, `smoke-tv.py`.

## Under SAM's jail: `/dev/dma_buf_unified` (2026-10-25, host-proven, not on the TV)

The card app's first TV run (`cardapp-result.md`, "TV results") showed that SAM runs a dev-installed native app in LG's jail (uid 5227), whose `/dev` has `mali0` and `dma_buf_unified` and neither `dri/card0` nor `dma_buf_te`, so neither source of the first two had a device. The shim has a third source, tried last in auto mode (`LUMATV_HEAP=unified` forces it), and `launch.sh` accepts `dma_buf_unified` as an allocator.

ABI, from LG's kernel source (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver/core/gpu/kernel/`, makefile `Kbuild`: `GPU_RELEASE-o22 = b46/`, and the set is chip O22A3):

- On the set's GPU release (`b46/drivers/base/arm/dma_buf_test_exporter/dma-buf-test-exporter.c`, `dma_buf_te_init`, lines 887-911) `dma_buf_unified` is a second misc device registered from a copy of the `dma_buf_te` device ("LGE ext"), with the same fops and mode 0666. The request is `DMA_BUF_TE_ALLOC = _IOR('E', 0x01, struct {u64 pages})`, which returns the dma-buf fd (`include/uapi/base/arm/dma_buf_test_exporter/dma-buf-test-exporter.h`). Pages are zeroed (`alloc_page(GFP_KERNEL | __GFP_ZERO)`, line 621).
- LG's older exporter (`{m12,r9,r17,r28}/drivers/base/dma_buf_unified/dma_buf_unified.c`, `include/linux/dma-buf-unified.h`) is a separate driver with `DMA_BUF_UNIFIED_ALLOC = _IOR('E', 0x01, struct {u32 size in pages; u32 map_cached})`: same number and size, the page count in the low word, `map_cached` 0 (write-combined) in the high word of a little-endian u64. One request serves both, so the shim sends the u64 page count. `DMA_BUF_UNIFIED_ALLOC_CONT` (0x04, contiguous, from `dma_alloc_attrs`) exists in both; it is not used: nothing here has measured it, and nothing says the exporter's device has CMA behind it.

Both exporters hand out ordinary pages (the dumb-buffer path's contiguous CMA is not available in the jail), so the expected frame rate is the `dma_buf_te` one (55.9-56.6 fps on Home, PLAN.md) [INFERENCE: the jail's buffers are the same kind of pages; not measured].

Host checks (`gfx64/check.sh`): `test_heapcore` (mock devices: unified alone, unified after te, the three modes, the request word layout), `test_shim_jail` (the whole interposer under `LD_PRELOAD="heapshim.so fake_exporter.so"`: `fake_exporter.c` hides `dri/card0` and `dma_buf_te`, answers `dma_buf_unified` with memfds sized as asked and decodes the request as LG's older struct, `FAKE_EXPORTER_FULL=1` answers ENOMEM), `test_launch.sh` (`dma_buf_unified` alone passes the preflight). Not shown: that the jail lets uid 5227 open `/dev/dma_buf_unified` and `/dev/mali0`, that the set's kernel is the b46 one, what the blob does with such buffers.

## Notes for the main agent to fold into PLAN/HANDOFF

- Heap shim phase item: done offline (`gfx64/`), TV smoke pending. Production env names are `LUMATV_HEAP*`, not the
  research `X1_HEAP`.
- A helper process on this path should be started as `gfx64/launch.sh --fallback <32-bit build> -- <64-bit build>`
  and read `LUMATV_GFX_PATH`/`LUMATV_GFX_LIB`; the 32-bit build needs its own pacing since fallback is 55.5-55.8 fps.
- The bundle is fetched, never committed; an ipk that ships it must run `fetch-blob.sh` at build time (about 37 MB).
- Not done on purpose: no zero-fill of buffers (kernel dma-heap buffers are zeroed; whether `dma_buf_te` and CMA dumb
  buffers are zeroed was not measured here [INFERENCE: both zero]. If a first-frame noise shows up before the client's
  first clear, check with a mmap of a fresh buffer); `fcntl(F_DUPFD)` on the heap fd is not tracked (the blob imports
  `dup`/`dup2`, which are).
- LumaShow lesson item 15 (GPU textures may hold noise) is the same concern for the zero-fill point.
