## Missing from PLAN.md

Surfaces that PLAN does not cover, or covers with a different scope than the
LG code has:

1. **HDMI input banner**: the pill shown on every input switch, with the OK
   and INFO banners (resolution, HDR, audio codec). PLAN restyles "the channel
   banner only" (Decided), but this banner and the No Signal screen are what
   HDMI 2 use shows; mock 20 is the channel banner.
2. **Voice listening bar** (`VoiceAgent`, 1954 files of animation) and the
   microphone indicator: separate from the Search app, drawn when the mic key
   is held. Not in PLAN or the mocks.
3. **Startup cover and boot popup**: `bootd` opens CoverUI, which launches the
   first app, so stubbing it can stall boot; the boot popup answers early
   hotkeys. Neither is in PLAN.
4. **Second-screen pairing prompt**: the modal "Connection Request" alert
   through which Home Assistant (`webostv`) and phones pair. It must survive
   the notification rebuild and stay answerable only with a physical remote
   key; PLAN's AI prompt is a different surface.
5. **Pointer right-click menu**: LG's `contextmenu` has two buttons, Back and
   Home. PLAN's "Context menu" row and mock 16 describe LumaHome's tile
   options, a different surface.
6. **Recents and Home phase**: LG draws recents in Home, not in the
   compositor (PLAN's Surfaces table says "Shadow module"); the same table
   gives Home "Phase 8" while Phases and Glasshouse say 1.
7. **What the set shows when "off"**: `lifeOnScreenMode` is `alwaysReady`
   (Gallery+ art and clock) and `alwaysready` raises its own countdown alert;
   PLAN covers only the idle screensaver.
8. **App launch prompts** from `sam` ("A new version of the app is
   available", "You must update this app to launch it"): likely with LG app
   updates blocked, not in PLAN.
9. **Remote shortcut keys**: Disney+ and Sling are not installed, so LG's
   auto-install path runs; Movies opens Search with "movie", New the Store,
   LG Channels its app. PLAN names no answer.
10. **Hidden key combinations and Home hold**: gear three times (AV reset),
    Yellow held 5 s, Green or Blue seven times, and Home held 2.5 s (Screen
    Zoom) which PLAN's 650 ms hold pre-empts.
11. **One-key TV button path**: Quick Settings and the input picker take the
    `localkey` launch flag; their LumaTV versions must keep it, since it is the
    way to operate the set without a remote.
12. **Messages that must never be filtered**: US EAS (antenna only), thermal
    shutdown, Magic Remote battery, LG Remote Service session, app permission
    and pairing prompts.
13. **Game Dashboard**: LG's quick panel switches to it in HDMI game mode;
    PLAN has the idea of a game card, no mock and no rule for game mode.
14. **Mobile pairing PIN popup** (`com.webos.systemui.mobileconnection`) is
    registered in configd `systemUIManager`, so the redirect reaches it; not
    in PLAN.
15. **Setting-change confirmations** from `pqcontroller` and `ocp` (energy
    notices, "keep the changed screen settings?", Filmmaker suggestions) fire
    when a picture mode changes; PLAN does not say which to drop.
16. **Settings sections without a mock**: Inputs & devices, Network, Ambient
    light, Home & look, System.
