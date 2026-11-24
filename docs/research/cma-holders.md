# Who holds the TV's contiguous memory (CMA)

Measured on the C4 (webOS 10.2.1, kernel 5.4) on 2026-11-24, as root, with no visible launch: Home in front, the kept apps resident or
closed through SAM. Numbers are MB unless a unit is given.

## What `CmaFree` is

`CmaFree` (`nr_free_cma`, zone Movable) is the part of the 692 MB (`CmaTotal`) that is free. The kernel lends free CMA pages to ordinary
movable allocations (an app's heap), so the figure falls as apps run and rises when they close. A request for contiguous memory
(`cma_alloc`) takes the lent pages back by migrating them unless they are pinned (standard CMA behaviour; the start at 3-7 MB under Decision
tests it). The
converter's wedge guard (`xcode/codec.rs`, 40 MB at 1080) and the broker (`prewarm/room.rs`) read `CmaFree` as a proxy for what a start can
get; it is a conservative one.

## Where the memory is, all five kept apps closed

Page-kind scan of the zone's frames (`/proc/kpageflags`, `/proc/kpagecgroup`), in MiB (256 pages): the zone holds 173,056 pages, 676 MiB, which
`CmaTotal` (692,224 kB) and the guard count as 692 MB.

| Kind | MiB | Whose |
|---|---|---|
| Not on an LRU list (drivers, DMA, kernel) | 297 | 270 are `nr_alloc_cma` (69,091 pages, `CmaDeviceAlloc`): allocated through the CMA allocator by drivers, constant whatever apps run; the other 27 are not attributed |
| Anonymous, on an LRU | 171 | processes' heaps, movable: `ls-hubd` 72, the compositor (`surface-manager`) 57, the root group 35, `webapp-mgr` 6 |
| Anonymous, mlocked | 39 | processes' pinned heaps (migratable, not reclaimable) |
| Free | 168 | what `CmaFree` shows (173-181 MB in the guard's unit at the time of the reads) |
| File cache | 1 | |

So of the 508 MiB (the 510 of the guard's unit) that `CmaFree` does not show with every app closed, 270 are driver allocations that nothing in
user space releases, and about 210 are the heaps of system services (`ls-hubd`, the compositor, `webapp-mgr`), which stay. Which driver holds the 270 is not
attributed: no `/sys/kernel/debug/cma`, no `page_owner`, and `/proc/hma/<pool>` answers with an I/O error (`/proc/hma/status` lists the pools'
addresses and sizes only).

Zone map (`/proc/zoneinfo`, `/proc/iomem`): Movable starts at pfn 432,128 (0x69800000) and spans 211,968 pages (828 MiB) of which 173,056 are
present; the three System RAM ranges at 0x69800000-0x72bfffff, 0x77000000-0x917fffff and 0x96c00000-0x9d3fffff are the zone, and the 152 MiB
between them are holes. During one `heavy` stream of 11.11 (1080i, `CmaFree` 87 MB before it, so nothing was closed) `nr_alloc_cma` rose from
69,091 to 78,671 pages (+37.4 MiB) and `nr_free_cma` fell from 21,802 to 12,025 (-38.2 MiB); within 9 s of the stream's end both were back
(69,091 and 22,037). The converter's chain therefore takes about 37 MiB of contiguous memory while it runs and gives it all back; the guard's
40 MB (32 MB up to 720 lines) is the right size. Not correlated: which of the pools in `/proc/hma/status` lie in these ranges.

## What the kept apps hold while they are hidden

With Marquee and Prime Video resident the same scan gives 320 not on an LRU (22 more than closed), 306 anonymous on an LRU (135 more), 8.5 free.
Prime's group (`/amazon`) holds 109 of the anonymous pages in the zone, Marquee's (`/com.beb.plxnative`) 23. Closing frees: Prime Video 35,
Netflix 32, Live TV 32, Marquee 11, Apple TV 15 or more. LG already swaps hidden apps to zram (Prime 107, Apple TV 76, Netflix 39, Marquee 20).

Device memory (`/sys/kernel/debug/dma_buf/bufinfo`, `/sys/kernel/debug/mali0/gpu_memory`), Marquee hidden:

| Holder | What | MB |
|---|---|---|
| compositor (`drm` buffers) | 6 buffers of 8.3 | 47.5 |
| `gal_renderd` (`kadpd`) | 4 buffers of 8.1 | 32.4 |
| Marquee (`dma_buf_te`) | 3 window buffers of 9.0 | 25.7 |
| Marquee (Mali context) | 6,683-7,536 pages | 26-29 |
| compositor (Mali context) | 21,105 pages | 82 |

Hidden Netflix, Prime Video, Apple TV and Live TV have no Mali context and no dma-buf at all: a hidden app of LG's holds heap only, and that
heap is movable. Marquee is the one hidden app with device memory (about 52 MB of window buffers and GL context). None of it is video
memory: the decoders' pools (`vpb`, `vpb8k`, `cpb`, `venc1`) are fixed regions that apps do not grow.

## LG's hibernation

- `AppHibernationEnable` is `false` in `/etc/configd/layers/base/com.webos.memorymanager.json` (`KeepAppHib` lists only
  `com.webos.app.lgchannels`; thresholds `AppHibAnonThreshold` 30, `AppHibMaxSize` 60), and `useAppHibernation` is `true` in the app info of
  `lgchannels` only. CRIU is enabled (`/tmp/criu_enable`, images in `/tmp/checkpoint`) and SAM logged `HANDLE_CRIU ... restored_image` for Live TV.
- Nothing triggers it for Netflix, Prime Video, Apple TV or Marquee. [INFERENCE] A native GPU app such as Marquee cannot be checkpointed
  (its GPU and DRM file descriptors are not restorable).
- Not measured: a hibernated app's reopen time and playback, since that needs the setting changed and a visible launch.
- `memory.force_empty` on Netflix's group swapped 62 MB to zram and raised `CmaFree` by 27 MB two seconds later; it was back at 4 MB after
  five seconds, the freed pages lent out again. SAM `pause`, `drop_caches`, compaction and `setRequiredMemory` freed nothing durable.

## Decision

The broker stays: it closes the kept apps in the owner's order (Live TV, Apple TV, Prime Video, Netflix, Marquee last) until the guard's
figure is met, and the policy brings them back above its 70 MB floor. Hibernation and trimming give no gain that lasts, and the apps that
cost CMA cost heap, not device memory.

Changes that could make closing unnecessary, none built:

1. The guard's proxy, tried and kept (2026-11-24 14:19-14:22 EST, helper `59688e98`: `9e66db9d` plus a flag file that skipped the broker and the
   40 MB check; `9e66db9d` is back on the set). Marquee, Netflix, Prime Video and Live TV were resident and `CmaFree` was 3-7 MB. Two
   `?transcode=heavy` starts on 11.11 (1080i) with nothing closed: the first reached the decoder's ready state in 610 ms and its first picture in
   0.1 s (longest gap 115 ms); the second took 468 ms and 1.6 s, and the engine ended it at 5.7 s (`Stalled("a picture took 1632 ms")`). Neither
   wedged the set or put a kernel warning in `dmesg`; `nr_alloc_cma` rose 9,806 pages (38 MiB) and returned, `pgmigrate_fail` did not move,
   and `pswpout` rose 22,437 pages (88 MB) during the second. With the broker the same start takes 44 ms. A start at 3 MB migrates but can
   stall past the engine's limit, so the broker and the 40 MB check stay. Not run: 720p60 and a third 1080i start.

   Five apps do not stay resident at 3 MB. With LumaGame, Marquee, Netflix and Prime Video resident (`CmaFree` 3 MB, `MemAvailable` 343 MB), a
   hidden launch of Apple TV was followed within 6 s by the close of every background app, itself included: SAM's `running` list was Home
   only, four Wayland clients disconnected within 1 s, `dmesg` had no OOM message. [INFERENCE] LG's memory manager closing by launch
   (`clearBackgroundApp`, `docs/research/lg-memchute.md` section 5); the launches were made by hand with prewarm off, so the policy's 400 MB
   headroom floor was not in play.
2. Marquee's window buffers. A hidden Marquee keeps 3 window buffers and its GL context (about 52 MB; [INFERENCE] system memory, not CMA, since
   `nr_alloc_cma` does not move). The change belongs to Marquee, not the compositor: on SAM's `hidden` state destroy the platform window
   (`QWindow::destroy()`, `QQuickWindow::releaseResources()`) and create it again on `relaunch`. The cost is the window's re-creation on the
   reopen, unmeasured, so it should be adopted only if the reopen stays within what a preloaded start takes now. The other four apps hold no
   device memory, so nothing needs releasing for them.

## Not measured

- A hibernated app's reopen and playback (above), and whether a visible-then-Home app differs from a preloaded one.
- Which driver holds the 270 MiB of `nr_alloc_cma`, and which of the pools in `/proc/hma/status` lie in the zone.
- The converter beside a game stream (`docs/plugins/live-tv-tuner.md`, the gate): the game's pipeline owner id, `CmaFree` during the stream and
  whether the chain starts.
- The converter at 3-7 MB on 720p60, and how often a start there stalls (one of two so far).
