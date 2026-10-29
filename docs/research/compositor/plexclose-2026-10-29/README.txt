PlxNative's close under LumaTV's compositor, 2026-10-29: the cause read from LG's binaries, the compositor probed, the fix.

The crash (rounds 17 and 18, libSDL2 +0x875c6, addr 0x18, lr +0x724b5) is Wayland_DestroyWindow+0x4e, `ldr r0, [r7, #0x18]` with
r7 = window->driverdata = NULL, called from SDL_DestroyWindow (+0x89) on a thread that is not the main one. LG's SDL
(/usr/lib/libSDL2-2.0.so.0.14.0, build ed86ea97) holds the app's SAM `registerApp` subscription itself: on `{"event":"close"}`
callback_RegistNativeApp calls SDL_SendQuit and SDL_AddTimer(1000, callback_forceQuit), and callback_forceQuit (the SDLTimer thread)
calls SDL_Quit again. SDL_Quit sets SDL_bInMainQuit, so the second SDL_VideoQuit runs although the first is still inside it: the
window is still linked with its driverdata already NULL. That is the signature. The first SDL_Quit is stuck in
WebOS_EGL_UnloadLibrary -> eglTerminate (the main thread in futex_wait, r17's plexstall-syscalls.txt).

LG's libmali (/usr/lib/libmali.so.0.46.0, r46p0, not stripped; the wayland winsys at 0x1e2a60-0x1e46f0):
- eglp_gles_flush_operation (the swap): registers eglp_frame_complete_callback on the frame's GPU event, then sem_wait(surface+0xc4),
  a semaphore created with 2 (eglp_create_surface): the app's swap returns up to two frames ahead of the present.
- eglp_frame_complete_callback (the mali-cmar-backend thread, when the GPU is done): eglp_display_frame -> winsys display_window_buffer
  unless egl_color_buffer_get_early_display, which get_window_target_buffer sets only when the compositor has
  zwp_linux_explicit_synchronization_v1 (registry_handler). Neither LG's compositor nor LumaTV's has it.
- display_window_buffer: while ws->frame_callback (+0xc) is set: wl_display_get_error -> return 0; dispatch_queue(ws->queue_b, 1000 ms)
  (prepare_read_queue, flush, poll(fd, POLLIN, 1000), read_events, dispatch_queue_pending); on a timeout the callback is destroyed and
  the present goes on. Then wl_proxy_create_wrapper(window->surface) (the wl_egl_window SDL freed first), wl_surface.frame on it when
  the swap interval is not 0, wl_surface.attach of the buffer, damage, commit, flush. frame_done (the wl_callback listener) only
  destroys ws->frame_callback and clears it.
- eglp_delete_surface (eglTerminate -> eglp_try_display_finish_terminating -> eglp_destroy_all_non_current_surfaces): sem_wait(surface
  +0x98), posted by eglp_delete_surface_callback when the surface's references (+0xac) reach 0; each frame in flight holds one until
  eglp_callback_refcount_callback runs after its present.
So a present waiting for the frame callback of the frame before when SDL destroys the wl_surface waits out its second (the callback
goes with the surface: delete_id, never done), eglTerminate waits for it, and SDL's force-quit runs at 1 s. Answered after the destroy
instead (round 16), the present ran on the freed wl_egl_window and wl_surface. The 1 s and the 0x18 are the same bug seen twice.

Why LG's compositor never showed it: plexclose-probe.py (a plain-Python client) against LG's compositor, the set awake, a full-screen
card: commit -> done 5.4 to 24.6 ms, mean 9.0 (phase A, 30 commits at spread phases: a commit right after the last done is answered in
~10 ms, one 15 ms after it in ~5.5 ms, so LG's compositor renders on demand rather than at a fixed latch); the buffer the commit replaced
comes back 80 us before the done. A callback whose surface is destroyed in the same flush gets delete_id (B, D); one whose destroy
follows 3 ms later had its done at +6 ms (C). Under LumaTV's compositor with the callbacks at the draw the commit libmali makes on a
callback lands 1-2 ms after the draw (the r18 trace: commits at +1 ms of each flip) and waits a whole tick, so a present was waiting
at almost every close; LG's answers before the app's teardown (close event +11-13 ms, SDL_Quit +22-24, destroy +32-35) is reached.

The fix (ClientCallbacks::Commit, the default; `ctl callbacks clients commit|early|landing`): a card's frame callbacks go out with the
commit that carried them, so libmali's present never waits and nothing is pending when SDL destroys the surface. The buffers a commit
replaced are still given back at the draw. Overlays, the keyboard and the shell keep the draw's pacing (the x1 probe is an overlay).
plexclose-probe-host-commit.txt: the probe against the host build (software renderer, 65 ms draws): phase C's done at +3.2 ms, the
others at the end of the draw the commit waited behind. Host tests: tests/flips.rs (the Mali close model under each setting).
