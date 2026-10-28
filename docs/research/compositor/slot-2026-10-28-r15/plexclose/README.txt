This copy keeps the timelines, the crashed close's files and one sampler sample; the raw `tail -F` streams (messages-follow.txt, 0.3-0.9 MB each), the sampler's other output and the rest are in /tmp/appcompat-ev/r15b/ on the host.
Directory names here: A-lumatv-crashed, A2-lumatv-sampler, A3-lg, A4-lumatv-sweeps; B and C are in ../gfn-browser/ (B-lumatv, C-lg). Scripts: tv-plexclose3.sh (on the set), tv-stall-snap.py, closetimeline.py.

Round 15 follow-up for AppCompat, 2026-10-28 14:50-15:03 PDT (TV clock +3 h), build c1a2736 (forge/main 5401834 has it); receiver muted through Home Assistant.
Monotonic seconds: messages ([n.nnn]), ring and compositor.log ([srv n.nnn]) and the steps files (/proc/uptime) share one clock. messages-follow.txt is `tail -F /var/log/messages` from before the
first launch to the end of the run (nothing lost to the rotation, which is about every minute while the TV is on); plex-msgs.txt is `grep -n 'plxnative\|youtube.leanback\|exit_status'` of it.

A-plex-lumatv-crashed/   YouTube 12 s, close, 3 s on Home, Plex 12 s, `ctl note plex-close-1`, close: Plex SIGSEGV (exit 11). it1-* are the copies made at once after the close
                         (journal, ring-700 = ring.json, ring-4000, compositor.log tail, windows, messages filtered, /tmp/plxnative-crash.log tail, events tail, dmesg), crash-files/ the new crash report.
A2-plex-lumatv-clean-with-sampler/  the same, 3 iterations, no crash, with a 25 Hz sampler on every Plex thread (it*-stall.txt: state, wchan, syscall nr/args/sp/pc, kernel stack; .maps-start, .fds)
A4-plex-lumatv-oneshot-sweeps/      the same, 5 iterations, no crash, sweeps only at 0.25/0.5/0.8 s after the close call (it*-stall.txt says GONE at 0.25 s: the clean close is over in 0.13 s)
A3-plex-lg-clean/        the same under LG's compositor, 2 iterations, exit 0
close-table.md, */close-timeline.txt   the close as ms after the closeByAppId call reached SAM
B-gfn-lumatv/, C-gfn-lg/ com.geforcenow.play 8 s (LG's browser opens), `ctl note gfn-home`, launch Home, 6 s; gfn-*-msgs.txt as asked (`grep -n 'chrome\|browser\|Crow\|crow\|WebAppMgr\|surface-manager\|geforce' | tail -400`),
                         journal.json, luna.json (`ctl luna`, B only), *-fullscreen-statechange-lines.txt, steps.txt; the browser was closed through SAM at the end of each.

Plex close (ms after the closeByAppId call reached SAM), the one that crashed against the clean ones:
  event       close(RegistNativeApp)  SDL_Quit start  Destroy window  EGL unload  UnloadWEBOSBASELibrary  forceQuit  APP_CLOSED
  crashed          12.4                  45.0             55.3           55.4          (never printed)       1012.6    1059.3, exit_status 11
  8 LumaTV clean  11.6-12.9            18.3-29.5        29.4-39.9      29.5-41.1      31.5-48.6                 -      120-149, exit 0
  2 LG clean      10.7-13.3            21.6-24.1        32.0-35.2      32.0-35.3      35.5-40.7                 -       87-98, exit 0
The crash is the second SDL_Quit: `callback_forceQuit` runs 1000.0 ms after the close event (SDL tick 11696 -> 12696) because the first quit stopped after "WebOS_EGL_UnloadLibrary unload" (the thread never
reached "UnloadWEBOSBASELibrary start", 2-9 ms away in every clean close), and the second one then dies on a NULL pointer (libSDL2 +0x875c6 `ldr r0,[r7,#0x18]` with r7 = 0, called from +0x724b4; that the first quit had already torn it down is a reading of the code [INFERENCE]).
The compositor sees nothing in the stall: Plex's window is destroyed at 28694.4931 (Home raised in the same pass, journal `window 5: fullscreen`), frames keep flipping for 0.2 s and the loop is idle after that, `client disconnected`
at 28695.493 (after the crash). The crash happens in 1 of 9 YouTube->Plex closes of this follow-up (4 of 13 over the round), never in 3 closes without YouTube; no stalled close was caught with the sampler
or the sweeps running (0 of 8), so what the thread waits on is still open (the sweep script tv-plexclose3.sh + tv-stall-snap.py in the round's evidence runs it again).

GFN browser (B against C), seconds after the launch of com.geforcenow.play unless noted:
  LumaTV: Browser::RequestWindowFullscreen() every ~0.30 s from 3.38 s on (40 calls by 15.6 s), SetVisible[1] after each. Home launched at 9.0 s: journal `window 5: raised ... window 7: minimized ... foreground [home]`,
          0.04 s later SetVisible[0] + cloudgame_active=false, 0.23 s after the launch RequestWindowFullscreen again -> `window 7: raised to the front (set_state fullscreen)`, `foreground [browser]`; the browser stays in front.
  LG:     RequestWindowFullscreen once (3.35 s), then none. Home launched at 9.0 s: `WebOSShellSurface::set_state ... Fullscreen` for Home, `WebOSExported::updateWindowState Qt::WindowMinimized for _Window_Id_4`,
          `NL_SURFACE_STATE_CHANGE browser MINIMIZED`, `getForegroundAppState` pushes {"appId":"com.webos.app.browser","state":"minimized"} (14 lines in the run, 0 in LumaTV's), browser SetVisible[0] 0.155 s after the call, no further request.
