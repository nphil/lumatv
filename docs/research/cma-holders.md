# Who holds the TV's contiguous memory (CMA)

Measured on the C4 (webOS 10.2.1, kernel 5.4) on 2026-11-24, as root, with no visible launch: Home in front, the kept apps resident or
closed through SAM. Numbers are MB unless a unit is given.

## What `CmaFree` is

`CmaFree` (`nr_free_cma`, zone Movable) is the part of the 692 MB (`CmaTotal`) that is free. The kernel lends free CMA pages to ordinary
movable allocations (an app's heap), so the figure falls as apps run and rises when they close. A request for contiguous memory
(`cma_alloc`) takes the lent pages back by migrating them unless they are pinned (standard CMA behaviour, not tested on this set). The
converter's wedge guard (`xcode/codec.rs`, 40 MB at 1080) and the broker (`prewarm/room.rs`) read `CmaFree` as a proxy for what a start can
get; it is a conservative one.

## Where the memory is, all five kept apps closed

Page-kind scan of the zone's frames (`/proc/kpageflags`, `/proc/kpagecgroup`): 676 MB of the 692 are present in the Movable zone.

| Kind | MB | Whose |
|---|---|---|
| Not on an LRU list (drivers, DMA, kernel) | 297 | 276 are `nr_alloc_cma` (69,091 pages, `CmaDeviceAlloc`): allocated through the CMA allocator by drivers, constant whatever apps run; the other 21 are not attributed |
| Anonymous, on an LRU | 171 | processes' heaps, movable: `ls-hubd` 72, the compositor (`surface-manager`) 57, the root group 35, `webapp-mgr` 6 |
| Anonymous, mlocked | 39 | processes' pinned heaps (migratable, not reclaimable) |
| Free | 168 | what `CmaFree` shows (173-181 at the time of the reads) |
| File cache | 1 | |

So of the 510 MB that `CmaFree` does not show with every app closed, 276 are driver allocations that nothing in user space releases, and
about 210 are the heaps of system services (`ls-hubd`, the compositor, `webapp-mgr`), which stay. Which driver holds the 276 is not
attributed: no `/sys/kernel/debug/cma`, no `page_owner`, and `/proc/hma/<pool>` answers with an I/O error (`/proc/hma/status` lists the pools'
addresses and sizes only).

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

1. The guard's proxy. `CmaFree` falls whenever an app runs, yet `cma_alloc` can migrate those pages. Whether the chain starts with `CmaFree`
   near 3 MB is the one experiment that decides it: start a conversion with the guard off and the five kept apps resident, and read
   `nr_alloc_cma` and `nr_free_cma` before, during and after. A start that finds the contiguous memory short has wedged the set before
   (`xcode/codec.rs`), so it needs the owner's approval and a moment when nobody watches. If it starts, the broker and the app eviction go.
2. Marquee's window buffers. A hidden Marquee keeps 3 window buffers and its GL context (about 52 MB; [INFERENCE] system memory, not CMA, since
   `nr_alloc_cma` does not move). The change belongs to Marquee, not the compositor: on SAM's `hidden` state destroy the platform window
   (`QWindow::destroy()`, `QQuickWindow::releaseResources()`) and create it again on `relaunch`. The cost is the window's re-creation on the
   reopen, unmeasured, so it should be adopted only if the reopen stays within what a preloaded start takes now. The other four apps hold no
   device memory, so nothing needs releasing for them.

## Not measured

- A hibernated app's reopen and playback (above), and whether a visible-then-Home app differs from a preloaded one.
- Which driver holds the 276 MB of `nr_alloc_cma`, and what the converter's chain adds to it while it runs.
- The converter beside a game stream (`docs/plugins/live-tv-tuner.md`, the gate): the game's pipeline owner id, `CmaFree` during the stream and
  whether the chain starts.
