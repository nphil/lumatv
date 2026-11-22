# Live compositor swap

**Verdict: conditional go.** LumaTV's compositor can be replaced while apps run and video plays, with a stall of a few frames, if the new version starts beside the running one, finishes its slow start-up there, and then takes over every client connection, open file and protocol object in place. LG's apps cannot reconnect, so nothing they hold may close: the swap hands over the same sockets and the same object ids. A host prototype does this for live clients in 1.3 to 2.2 ms, about one extra 120 Hz frame as the clients see it, and survives a successor that fails. The condition is LG's GPU driver: the server side of its buffer objects is the driver's own code, and recreating it in the successor is untested on the set (§ Driver objects, step 2 of § TV test plan). If that step fails, a swap would end every app that draws with the GPU, and installing at standby remains the only update path.

Measured on the host (Debian, libwayland 1.21.0, the same source as the bundle's) and read from the set (LG OLED65C4PUA, webOS 10.2.1, kernel 5.4.268) on 2026-11-21. Nothing on the set was changed.

## Why clients cannot reconnect

LG patched `libwayland-client` (`/usr/lib/libwayland-client.so.0.20.0`, md5 `00e44b27498921d20942a5cdc0be7b93`): its fatal-error path (`display_fatal_error`, 0x336c) logs "Exit client due to a fatal connection failure" and calls `exit(1)` on `ECONNRESET` or `EPIPE`, and end-of-file counts as `EPIPE`. Apps reach the compositor through this library, so a closed compositor socket ends the app whatever its toolkit does. Nor do the toolkits reconnect: Qt added an opt-in path (`QT_WAYLAND_RECONNECT`) in 6.6 and the set has Qt 5.12; SDL 2.0.14 and `libgm-wayland` have none.

## Designs

| | Exec in place | Pre-warmed successor | Split keeper and renderer |
|---|---|---|---|
| Stall on the set | at least 2.1 s: the cold start (EGL 143 ms, 48 shader programs 1,807 ms with none from cache, GPU warm-up 74 ms, KMS about 90 ms); about 0.3 s with a program-binary cache [INFERENCE] | the handover only: 1.3 to 2.2 ms of protocol state on the host, an estimated 1 to 3 frames with the first flip [INFERENCE] | none for a renderer swap; a keeper swap needs one of the other two |
| Supervisor | same pid; sm's heartbeat falls back after 1.5 to 2 s of silence, so the 2.1 s stall trips it | sm starts the successor and adopts its pid at commit | sm watches two processes |
| Extra hop per frame | none | none | scene to the renderer and presentation back, every frame; every driver buffer exported as a dma-buf |
| Size | the shared core plus about 2 days (a program-binary cache, the heartbeat) | the shared core plus about 5 days | the compositor split in two (30 to 45 days [INFERENCE]) on top of the shared core |
| Main risk | UI frozen for the stall (video plays on) | memory and CPU of two compositors during the warm-up | 120 Hz pacing across two processes (composition takes 4.1 ms of the 8.33 ms frame today) |

The shared core is the snapshot and restore of § State; every design needs it, because the long-lived part must still be upgraded somehow. The successor is the design that meets "at most a flicker". The split earns its cost only if renderer crashes become the problem to solve.

## The successor design

1. sm starts the successor on request: the same loader and arguments, plus a socket to the running compositor. The running compositor keeps serving.
2. The successor builds everything that does not need the clients: EGL, the shader programs, GPU warm-up, the UI, its scan-out buffers (through its own `card0` opener, authenticated by the running master with `drmAuthMagic` where the driver requires it). It then reports ready.
3. The running compositor quiesces at a frame boundary: no commit in flight, the luna writer flushed, no input method session, clipboard transfer, screen capture or animation in progress (otherwise the swap is refused and retried later). It stops reading clients.
4. It writes the snapshot (§ State) to a memfd and sends it, with every file the snapshot names, over `SCM_RIGHTS`.
5. The successor adopts each client socket, recreates every object at its id while the client is muted, replays the driver's objects (§ Driver objects), refills libwayland's unread and unsent buffers, and reports restored.
6. The running compositor answers commit and exits without closing anything; sm watches the successor from then on. Until commit, a successor that fails or does not answer is killed and the running compositor carries on: nothing has been sent to any client.
7. The successor takes the DRM master file it received, flips its first frame (same windows, same video hole), removes the old framebuffers after that flip, sends the buffer releases the old process owed, and resumes.

## State

**A. Files, carried as the same open files.** The Wayland listener and its lock (new clients wait in the backlog), each client socket (11 at the census, of 32 sockets), client shm pools, the tap's duplicates of driver buffers, evdev files (13; unread events belong to the open file), the bridge socket to sm (sm keeps the luna registrations and subscriptions by token; its hang-up is a fallback), the Rust UI's channel and key sockets, the UI shm files (58) and the read end of the UI's stderr pipe (if it closed, the UI's next log write would fail). The DRM master file is the critical one: master status, framebuffers, GEM handles and the CRTC belong to it, and closing its last copy removes every framebuffer in use and turns those planes off (`drm_fb_release`). The kernel's flicker-free `CLOSEFB` arrived in 6.8; the set runs 5.4. Recreated instead: epoll, timerfd, signalfd, eventfd, inotify, the compositor's own log pipe.

**B. libwayland's private state (1.21.0).** Per client: unread request bytes and fds, unsent event bytes and fds, the id map's size (so the client's next new ids are accepted), server-created ids (`wl_data_offer`). Per display: global names, the next name, the serial. The unsent buffer is never empty at a frame boundary: libwayland queues `delete_id` without asking for a flush. `wl_client_create` adopts an existing socket and `wl_resource_create` takes the client's id, so the public API covers adoption; the buffers, map sizes and names need private access. The prototype uses pinned 1.21.0 offsets with layout checks at start; production should patch the bundled 1.21.0 to export those few accessors, since LumaTV ships that library (`gfx64/README.md`).

**C. Protocol objects.** LumaTV's 21 global interfaces and roughly 45 object classes, recreated at the same id and version with the same data, in ascending id order, holes filled with a placeholder; then references relinked (surface roles, attached buffers, frame callbacks, subsurfaces, surface groups, foreign exports and imports by window id, seat focus). A class without a serializer refuses the swap. `wayland-backend` 0.3.17 needs `Handle::create_object_at` and `ClientId::as_ptr`, `wayland-server` 0.31.14 a public `ResourceData::new` (about 70 lines of patch in the prototype).

**D. Driver objects.** Replayed into the driver (§ Driver objects).

**E. The State model.** Carried: surfaces and the scene (placement, z, roles, attached and displayed buffers), client and window records, webOS shell surfaces, seat focus and pointer position, serials, `foreign` and `vwin` (sinks, the allocator, the status subscription token, placement times: monotonic nanoseconds are system-wide), luna subscriptions, the Rust UI's pid and channels, panel dim and refresh policy, the switcher, buffers the display still reads (their releases are owed) and the `drawn`/`flips` counters sm's watchdog compares. Refused while in progress: a key or button held, a luna call awaiting its reply, input method, clipboard or drag transfer, screen capture. Animations snap to their end.

**F. Rebuilt.** Before the swap, in the successor: EGL, shader programs, GPU warm-up, UI atlases, scan-out buffers. After it: EGL images and textures of client buffers, for the displayed ones first.

**G. Threads.** None crosses. The KMS committer is drained, the luna writer flushed, the control and UI servers idle, the log drains stopped at a line end; the successor starts its own.

## Driver objects

Every app that draws with the GPU binds `mali_buffer_sharing`, the global the driver creates in `eglBindWaylandDisplayWL`, and creates its buffers with `create_buffer` (dimensions, stride, format, modifier and a dma-buf file; `scanout-research.md`). The server side of these objects is the driver's code. The protocol's only event is `alloc_device`, and the driver never sends it (`compositor/slot-2026-10-29-r18/scanout/proof-a-verdicts.txt`: "alloc_device: none seen").

At the start of the restore the successor binds its display to the driver, with libwayland's next global name set to the old global's name. It then calls the driver's own bind function for each client's old id and the driver's own `create_buffer` for each live buffer, with the tap's recorded arguments and a duplicate of the buffer's file, in id order with the other objects. The driver then owns these objects as it did before, the import path (`EGL_WAYLAND_BUFFER_WL`) is unchanged, and no byte reaches the client. The prototype restores libwayland's own registries this way, through the display's request table. What the driver does internally on a replayed bind is unknown. The tap must be on from start; today it starts only with the scan-out switch, and a buffer made before it has no record.

The fallback is LumaTV's own `mali_buffer_sharing` objects at the same ids, with buffers imported by dma-buf. Own mode has crashes on record: `com.webos.app.tvhotkey` exited 139 0.5 s after binding LumaTV's global, and `hdmi2` and LG's Flutter Home died after binding it (`compositor-c2-result.md`). Own mode sends `alloc_device("/dev/dma_buf_te")` at bind and the driver does not; that difference is the likely cause [INFERENCE]. A restored object gets no bind event at all.

## Video

The decoded picture never passes through the compositor. videooutputd owns the display-engine windows (MAIN, SUB0) and the pipelines. The compositor owns the hole in the OSD frame where the video shows, the luna calls that place the windows (`setDisplayWindow`, `setCompositing`) and its `getStatus` subscription, all through sm's bridge, and the webOS foreign objects that tie an app's surface to its video. During the stall the OSD plane keeps its last frame, hole included, and the video plays on beneath it. The successor's first frame must have the same hole, and it must not place again a window that did not move. A lost `wl_webos_imported` link ends the app's media connection (`compositor-c2-result.md`: "media connection dropped on `wl_webos_imported` opcode 7").

## Prototype

`handoff-proto` (host only, not merged): a small compositor on libwayland's C server (`wl_compositor`, `wl_shm`, `wl_shell`, `wl_webos_shell`, 120 Hz frame callbacks) that hands itself over either by exec or to a pre-warmed successor, and a C test client that, like LG's library, exits on any connection failure. Each client frame writes its frame number into the buffer and creates and destroys a region, so object ids churn.

- **Successor**, 4 clients (one on `wayland-rs`), 47 objects, 18 files, 4.7 KB of state, 300 ms simulated warm-up during which the clients ran at full rate, 5 swaps: quiesce to restored 1.28 to 2.03 ms (snapshot 0.50 to 0.78, transfer 0.62 to 0.98, restore 0.64 to 1.05), commit at 1.41 to 2.18 ms, first frame 9.9 to 11.1 ms after the start. Clients: frame callbacks p50 7.9 ms, p99 10.1 to 12.1 ms, worst 19.1 ms. All clients exited 0.
- **Failure:** a sixth successor, made to fail after receiving the state, was killed; the running compositor carried on and no client noticed.
- **Exec**, 16 swaps at 4 Hz, up to 5 clients and 59 objects, the fifth client connecting after swap 6 through the adopted listener: swap 4.42 ms mean, 6.13 ms worst (of which exec 2.61 and restore 1.81 on average); first frame 11.8 to 15.3 ms after the start; frame callbacks p50 7.8 ms, worst 23.8 ms. All clients exited 0.
- **Half a request:** a `wl_shm.create_pool` split mid-message, its file already received, was completed by the new process; the first frame showed the pixel the client drew.
- **Unsent events:** with flushing disabled, 44 bytes (a release, a callback `done`, `delete_id`) crossed and were delivered.

The prototype has no GL, KMS, driver objects, input or State model beyond surfaces and buffers.

## Risks

- The driver's objects (§ Driver objects): the deciding unknown.
- Completeness: any object or state left out is a protocol error in a client after the swap, and LG's client then exits. Refusing a swap is always safe; guessing is not.
- libwayland's private layout: pinned to the bundled 1.21.0, checked at start.
- sm: the running compositor's exit is a fault today, and the heartbeat falls back after 1.5 to 2 s of silence.
- The video hole: a first frame without it would flash UI or black over the picture for one frame.
- Two compositors during the warm-up: memory (166 MB resident today; 1.18 GiB available at the census), contiguous scan-out memory, and 1.8 s of shader compilation on the cores that run the frame loop. A failed allocation aborts the swap; the compile runs at low priority.

## Estimate

About 28 work-days (24 to 34):

- libwayland accessors as a patch to the bundled 1.21.0, with host tests: 2
- snapshot and restore of the ~48 object classes, server-created ids included: 8
- the State model, the refusal rules and the owed releases: 5
- driver objects: tap from start, replayed bind and `create_buffer`: 2
- successor handshake, quiesce, KMS takeover and first flip: 5
- sm: start the successor, adopt its pid at commit: 2
- per-class round-trip tests, a randomized swap-under-load test, the TV runs: 4

Step 2 of the test plan decides the project and needs only the first and the driver item plus a private-socket harness (about 5 days), so it goes first.

## TV test plan

1. **Read-only, any time:** the compositor's open files with their kinds, flags and peers (`/proc/<pid>/fd`, `fdinfo`, `/proc/net/unix`); hashes of the bundled `libwayland-server` and the driver; `ctl status` for clients and surfaces.
2. **Driver objects on a private socket, with the owner's OK:** a second compositor instance beside the running one, no KMS, an LG GPU app (`com.webos.app.tvhotkey`, as in the c2 buffers test) pointed at it; ten swaps with replayed driver objects. Pass: the app keeps drawing, the successor imports its buffers, no exit.
3. **One swap with only LumaTV Home up, with the owner's OK:** the installed build swaps to itself. Pass: flip gap within 3 frames, sm sees no fault, the escape still works.
4. **One swap during playback, with the owner's OK:** Netflix playing. Pass: the app's pid unchanged, picture and sound uninterrupted (videooutputd status, LG's capture shows no black frame), the hole in place, flip gap within 3 frames.
5. **Soak:** 50 swaps over an hour with apps open.

Each step records its receipt path.
