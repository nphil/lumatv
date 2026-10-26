# lg-luma review (2026-10-25)

## Short answer

Do not switch to OpenGL for the sake of OpenGL. Luma's speed comes from a few choices, and those can be taken into Qt or the 64-bit toolkit without a rewrite. The bigger limit is LG's compositor, and no app-side OpenGL code gets around it. Our own X1 test on this set shows that: an idle client held only 51.2 fps.

## Q1: How lg-luma draws (what the code does)

- **Stack:** SDL-webOS (LG's SDL build, pinned release-2.30.12-webos.5) opens a Wayland window with a GLES 2.0 context, double buffered, vsync on (swap interval 1). Rust (`src/renderer.rs`) does the scene work.
- **Scene and drawing:** each screen change builds a draw list (shapes, gradients, shadows, text, images, clips). Draws are grouped by texture. A per-frame stats record reports draw calls, CPU submit time and redrawn fraction.
- **Partial redraw:** `src/egl_damage.rs` uses EGL buffer age or `EGL_KHR_partial_update` to repaint only the changed tiles; otherwise it repaints everything. Changelog 2.8.1 reports focus moves at 60 fps versus 39 fps at 1080p on a webOS 5 TV (p95 frame time 17 vs 34 ms).
- **Text and logos:** text is vector outlines, tessellated once and cached per text run. Logos are pre-built meshes. No runtime SVG or font parsing.
- **Images:** a worker thread reads and decodes png/jpeg/webp. Source files are capped at 512 KiB and 2048x2048; decoded icons at 256x256; CPU cache 16 MiB; GPU cache 48 MiB. Failures are remembered until a refresh.
- **Event loop:** waits 0 ms while animating, 8 ms while work is pending, 100 ms when idle, 500 ms while prewarming, and 250 ms when hidden. Idle prewarm of heavy pages starts after 1.5 s with no animation, so the first Apps visit is smoother (changelog: worst frame 179 to 40 ms on webOS 5).
- **Multisample:** 4x MSAA by default with 2x and 1x fallback. Analytic single-sample mode is a Settings option (2.9.0).
- **Startup and memory:** the code has startup stage timings, but I found no published cold-start time and no measured memory. The caps above are ceilings, not measured use, so they tell us nothing about typical memory.
- **Caveat:** every published performance number is from a webOS 5 TV. Nothing is published for the C4 (webOS 10.2.1). The README table lists the C4 as working on webOS 25, with Home takeover not reported.

## Q2: Does OpenGL-direct help us?

Short answer: not by itself.

- **Memory:** a plain OpenGL app avoids the Qt runtime, which could mean less memory and a quicker start. [INFERENCE] The size of that saving is unmeasured. Our LG Flutter Home reference is 67 MB resident; we have no comparable figure for LumaHome.
- **Frame pacing:** both Qt and lg-luma wait for vsync, so pacing is the same. Under LG's compositor, every app pays the same composition cost. X1 measured 6.3 ms GPU and about 5 ms CPU per frame for composition alone, and the frame clock followed the content's refresh rate. Only our own compositor (track C) changes that.
- **Input latency:** lg-luma's non-blocking event wait is sensible, but we have no measurement of our own key-to-pixel delay, so I cannot compare it. [INFERENCE]
- **Where it would fit:**
  - *Volume OSD and input picker:* these are LG compositor surfaces (the input picker is in `com.webos.surfacemanager`). A separate app window cannot draw over another app. They fit only inside our own compositor.
  - *GFN app:* a full-screen client like any other. OpenGL gains nothing over Qt here; pacing is set by LG's compositor.
  - *Screensaver:* the same limit applies. Our own screensaver can only show when our compositor is in front.
- **The real lift** is partial redraw plus idle prewarm. Both work in any toolkit. They reduce the app's own GPU and CPU work, which matters because LumaHome's idle frame costs 8.1 ms GPU of the 16.7 ms budget (PLAN). [INFERENCE] The saving is unknown until we measure partial redraw on the C4.

## Q3: Items LumaTV lacks (not already in PLAN.md)

Ranked by value and cost. Items already in PLAN.md are excluded.

1. **Partial redraw with buffer age (`egl_damage.rs`).** Effort: medium, 1-3 days including a C4 measurement. Fit: Home and Apps focus moves. Start with a measurement: how much of LumaHome's 8.1 ms GPU frame is spent on a single tile change. Their `LUMA_DAMAGE_VERIFY` checker is worth copying for tests.
2. **Idle prewarm of heavy pages.** Effort: small. Fit: first Apps visit and Recents. Check first whether LumaHome already has a first-visit spike before building.
3. **Image worker with hard caps.** Effort: small. Fit: app icons on Home, Apps and Recents. Their limits (source 512 KiB, 2048 px, 256 px decode, failures cached until refresh) are a good starting template.
4. **Recents close and launch calls.** `com.webos.service.recent/removeRecent` with `{appId}` closes one app. Launch uses `com.webos.service.applicationmanager/launch` with `reason: recent`. Their docs say these were captured on webOS 26, not 10. Effort: small to check on the C4. Fit: LumaTV's Recents. PLAN lists `listRecents` already; `removeRecent` appears to be new.
5. **Key handling for webOS codes.** Scancode 482 is Back; 505 is the long-Back Exit, which opens the switcher at once; 484 and 485 (cursor show and hide) must be dropped so they do not cancel a Back hold. Effort: small. Fit: LumaTV's key filter. We have not checked whether our filter handles 505 or 484/485, so this is a check first.
6. **Screensaver behaviour.** SDL suppresses LG's screensaver by default. Luma 2.8.5 allows it through `SDL_VIDEO_ALLOW_SCREENSAVER`. Effort: tiny. Fit: any app of ours that is in front when the TV goes idle.
7. **Service-name spelling.** lg-luma says capitalisation in luna service names matters. Our PLAN uses `com.webos.applicationManager` while theirs uses `com.webos.service.applicationmanager`. Effort: tiny. Check which spelling our calls actually use.
8. **Icon read through an elevated helper.** When an app's sandbox cannot read another app's files, lg-luma reads them through Homebrew Channel's root `exec` service (`tv/read-app-icon.cjs`). Effort: small. Fit: only if LumaTV needs icons from folders its sandbox cannot read.

Skip: the `getauxval` shim (only needed by their 32-bit build against an old libc), vector text (Qt already handles text well), and the screenshot and test tooling.

## Not checked

- `src/app_artwork.rs`, `src/state.rs`, `src/luna.rs` and the rest of `renderer.rs` (lines 300 onwards) were not read in full.
- The 21 open issues and the releases page were not scanned for webOS 10 or C4 reports.
- No LumaTV measurements were taken, and nothing was run on the TV.

## Verdict

Luma is a good reference, not an architecture to copy. Its OpenGL layer is not why it is fast. Lift the partial redraw, the idle prewarm, the image caps and the key details. Measure partial redraw on the C4 before committing to it.

## Optional detail

- **Version discrepancy to check:** lg-luma lists the C4 as webOS 25. Our set reports platform 10.2.1. [INFERENCE] These are probably the same firmware under two names. Confirm before relying on the compatibility table.
- **Licence:** Luma is MIT. Lifting code is allowed with attribution.
## Decided (Main, 2026-10-25)

- No OpenGL rewrite: Qt Quick already draws on the GPU, and pacing under LG's compositor is the compositor's. Partial redraw and damage tracking are already built in LumaTV's own compositor (C3); the startup warm-up covers idle prewarm; `removeRecent` with a close first is in PLAN.md.
- Taken: the key codes (505 long-Back, 484/485 cursor show/hide must not cancel a Back hold) are checked in LumaTV's key filter by its owner.
