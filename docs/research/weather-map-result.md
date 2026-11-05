# Weather view: how the map and the radar are loaded (2026-11-05)

State: built and tested on the host; not run on the TV. `tools/wx-memory-probe.sh` is the TV's check (criteria in its header).

## The base map stays raster

The question was whether the view should draw OpenFreeMap's vector tiles as geometry on the GPU (a mesh) instead of the 256 px cells the helper draws on the CPU (`map` crate, `tiny-skia`). Measured on the host (release build, a busy machine, Atlanta, a 1920 x 1080 view; the set's CPU is 6 to 10 times slower):

| zoom | tiles | read a tile | draw a tile (screen) | tessellate a tile (screen) | mesh per screen |
|---|---|---|---|---|---|
| 9 | 48 | 0.73 ms | 14.8 to 18 ms (0.7 to 0.9 s) | 7.8 ms (0.37 s) | 1.07 M triangles, 21 MB |
| 8 | 40 | 0.77 ms | 17.2 ms (0.69 s) | 10.5 ms (0.42 s) | 1.0 M, 19.5 MB |
| 7 | 40 | 0.90 ms | 11.9 to 15 ms (0.5 to 0.6 s) | 11.1 ms (0.45 s) | 0.93 M, 18 MB |
| 5 | 45 | 0.64 ms | 3.1 ms (0.14 s) | 1.0 ms (0.05 s) | 0.38 M, 7.5 MB |

- Tessellating is about twice as fast as drawing, which is 0.3 to 0.4 s a cold screen on the host. The tiles come over the network either way (4.2 to 6.9 MB a screen), and that is most of the time to a first picture.
- A mesh is not lighter on the graphics side: about a million triangles and 18 to 21 MB of buffers redrawn every frame, against a picture of 12 MB that is uploaded once and drawn as one quad.
- It needs what is not there: an orthographic mesh path in the shared compositor (`mesh3d` is perspective only and its multisample target is about 9 MB), a protocol change, a tessellation dependency (`lyon`) and a way to draw the labels. LumaShow's `docs/mini-apps.md` section 3 rejected the same design for the same reasons.
- Vector is sharper through a zoom: a picture stretched 1.19 times keeps 62 to 65 % of an edge's contrast, 1.41 times 58 to 61 %, 2 times 43 to 46 %; a mesh keeps all of it. The page keeps the stretch under 1.19 times at rest (a map picture is cut at the nearest half level of zoom, `wxpic::map_grid`), cuts another when the zoom leaves the level, and holds a coarser one under it, so a glide never shows a bare place.

## What the helper holds and does

`GET /v1/wx/pic?k=map|backdrop|radar&cx&cy&zoom&w&h&dark&epoch[&t]` replaces `/v1/wx/map` and `/v1/wx/frame`. It names the view; the helper picks the level and the whole cells that cover it and answers one picture: raw premultiplied RGBA and a 64-byte trailer, so neither side encodes or decodes a PNG (17 to 47 ms and 18 to 26 ms for a screen). A scene may hold 256 layers in all, so a tile cannot be a layer: a picture is a mosaic.

- **Only what is visible, at the zoom asked.** Map cells at the nearest half level (zoom 5 to 11), a backdrop two levels coarser (from zoom 7), radar tiles at the zoom 3 levels under the view's (a tile fetch is a child process, so few of them). At most 80 map cells and 16 radar tiles to a picture.
- **Bounded caches.** Drawn map cells 16 MiB (64 cells), decoded vector tiles 8 MiB, radar fields 3 MiB (48 tiles; an empty tile costs nothing); each a byte-capped least-recently-used cache (`wxcache::ByteLru`). On the disk: drawn cells as PNGs 24 MiB, vector tiles 24 MiB, radar tiles by address. A view opened again, or a zoom back, reads cells (about 1 ms) and does not draw them (12 to 18 ms on the host).
- **Vector tiles four at a time** (`wxvector`): the engine's one worker asked for each tile it lacked itself, one request after another. A cold screen of 48 cells took 8.0 s to complete (15.6 s on a slower network), and now 2.1 s; a new zoom level cold 1.9 s (was 3.0 to 3.7 s), half a screen of pan 0.77 s (was 2.6 s).
- **Cancelling.** Every ask carries the view's number (`epoch`, milliseconds, only growing). An older one is refused (410), the engines drop queued work for cells no request still wants, and the prefetch drops tiles not begun. A fetch already on the wire finishes: the transport cannot call a child process back (at most 4 vector tiles and 2 radar tiles).
- **Closing.** `POST /v1/wx/close`, or ten minutes without a request, closes both engines (the worker, the queues, the caches in memory) and trims the heap (glibc; musl, the set's allocator, returns a large block when it is freed). The five-minute warming makes its own engines, draws the opening view's cells and fetches the newest two radar frames' tiles to the disk, and lets them go: nothing of it stays in memory.

## What the page holds and does

`radar_pics.rs` (the model) holds four pictures at most: the map, the one under it, the radar frame on show and the next one (`radar_view.rs` asks for the next as the loop steps, and the loop steps when it is in). A picture serves the view while it covers the screen and is of the level the view calls for; a small zoom or pan asks for nothing, and past it a new one is asked for once the view has rested 150 ms. A replaced picture stays 0.4 s for the cross-fade and is released; the coarse map under the map is drawn only while the map does not cover the view (the compositor does not treat a picture as hiding what is under it, so a full-screen layer under the map would be drawn in every frame of a glide or of the loop). Asks in flight are forgotten when the view moves and late answers dropped. The layers hang under the group the compositor scales for the zoom, are posed by the world they cover, and release this process's pages of their pixels once painted. The view the zoom and its pan come to (the world at the screen's middle) is the surface's to say. `host::memory::tune` fixes glibc's mmap threshold at 1 MiB: with the default a closed view kept 26 MB of freed heap.

## Measured on the host (helper: glibc with the thresholds fixed at 128 KiB for musl)

| | before | after |
|---|---|---|
| base map complete, cold | 8.0 s | 2.1 s |
| base map, disk warm | 0.99 s | 0.32 s |
| first picture, cold | 1.6 s (partial map) | 0.65 s (backdrop) |
| radar frame nearest now, cold | 8.0 s after the open (all 36 by 8.7 s) | 0.22 s from its ask, which the page makes with the map's; the loop's next frame 0.22 s on average, 0.44 s at worst |
| helper resident, view open | 49 to 59 MB | 22 to 29 MB |
| after zoom and pan | 59 to 76 MB (peak 89) | 28 to 56 MB |
| closed, 3 s later | 23 to 39 MB; 30 to 47 MB with the service dropped | 6.8 MB (2.9 MB at start) |
| the warming | left 49 to 66 MB resident | nothing |

UI (`ui/tests/weather_memory.rs`): layers 43, 67 with the view up, 43 after; pictures held 14.8 MB (the map, its backdrop, two frames), layer files 18.7 MB, 35.2 MB, 18.7 MB; resident +1.5 MB open, +2.4 MB after zoom and pan, +1.7 MB closed (+25.9 MB closed with glibc's default mmap threshold). Helper: `tests/wx_memory.rs` (caches within budget, threads gone, resident back within 6 MB and anonymous within 3 MB).

## Not tested on the TV

The cells' drawing time on the set's CPU (a cold screen of 48 cells is estimated at 4 to 9 s; the disk cache, the backdrop first and the parallel fetch are what shorten it), four vector fetches at once on a 2 GB set (each an `openssl` child), musl's return of freed memory, the compositor's GPU memory after the close, and the frame cost of a 12 MB picture arriving (a copy into a layer) while the loop plays. The probe records the UI's, the compositor's and the helper's resident and proportional memory and the pictures' files at the four points and judges them.
