# LG surface catalogue

Every screen, popup, overlay, banner, menu and notification the LG C4
(OLED65C4PUA, webOS 10.2.1) can show, with LumaTV's planned answer for each.
Companion to `PLAN.md`, whose Surfaces table names about twenty of them, and
to `mocks/`. Source: the firmware snapshot `/data/home/lgtv/uiinv`, the QML and
app files read from the set, and read-only luna getters.

## Resume here

State on 2026-10-23 20:00: the catalogue is complete and nothing is left to
do. All 160 app ids and all 113 compositor view files are in a row or in "No
UI" (script check). Mock status is against the 24 screens of `mocks/` (batch 2
complete at 19:57). Raw rows with evidence for every slice are in
`docs/research/surfaces/*.json`; the English service message strings are in
`service-strings.txt` there. The TV was only read: no setting, app or power
change, left in standby.

To refresh after a firmware update: pull the small text files of
`/usr/palm/applications` and the QML modules from the set (paramiko, stream
`find ... | tar -T - -cf -`; BusyBox tar has no `--null`), diff the app ids
against `listApps`, and compare `configd` `systemUIManager` and `keyFilters`.

## Reading the tables

One row per surface, grouped by what Nitin sees. Columns: Surface (what it
shows) · LG id, kind and size · Seen · Plan · Answer with its reason · Hook ·
Ph.

- **Seen** is an estimate from how this set is used (Filmmaker mode, Denon
  AVR-S760H over eARC on HDMI 2, Live TV rare, Magic Remote pointer
  sometimes, Netflix, YouTube, Prime Video, Plex): `daily`, `sometimes`
  (about weekly), `rare` (a few times a year or less), `never` (cannot appear
  on this set, or only in factory, hotel or store modes). `[INFERENCE]` marks
  a frequency or fact not confirmed on the set.
- **Plan** is the coverage today: `Yes` named in the PLAN Surfaces table,
  `Part` named elsewhere in PLAN or only as a family, `Out` listed as not
  owned, `No` absent. A trailing `· mock NN` or `· no mock` says whether
  `mocks/` has the screen (24 screens, the list in `mocks/BATCH2.md`).
- **Answer**: Replace (LumaTV draws it), Restyle (LG's logic, LumaTV's look),
  Off (disabled or stubbed), Keep (LG's own, reason given). `Keep (dormant)`
  cannot appear on this set and needs no work.
- **Hook** uses the names of the PLAN Hooks table (System UI redirect, View
  swap, Shadow module, Overlay mount, Key filter and launch redirect) plus
  `Home stub` and `Launcher filter` (both in PLAN under Glasshouse), `LG
  setting` (switched off through settingsservice or configd), `Service off`
  (masked through Glasshouse) and `Overlay mount (theme)` (the shared web
  theme, below).
- **Ph** is the phase of PLAN (1-9). A `*` marks a phase proposed here
  because PLAN gives none; `—` means no work.
- **Kind** is web, qml, native, flutter, compositor-qml, system-qml, service
  or js. Sizes are the snapshot's `size_kb` (or `appsize` from `listApps`,
  or `du` on the set where the folder is not in the snapshot).
- Web apps: the `main.js` bundles of 43 of 44 checked LG web apps (Settings,
  First Use, Accessibility, the wizards, Scheduler, Game Optimizer, Media
  Player, Usage Care and the rest; the exception is the sign-language avatar)
  carry hundreds of Enact Sandstone strings, so the one shared theme of PLAN
  reaches every reachable one at no cost per app; those rows read Restyle.
  Apps that load a cloud page (Apps, LG Account, LG Channels, Premium) or
  draw their own UI (Netflix, Plex) are not themable that way.

## 1. Home & launching

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Home**: shelves, hero banner, Q-cards, app row, recents, edit mode; hero is off here (`homePromotion` off) | `com.webos.app.home` · flutter · 44.7 MB | daily | Yes | **Replace**: LumaHome draws it; LG's Flutter Home stays resident behind it as the fallback | Home stub | 1 |
| **Senior (Simple) Home**: large-print home for Simple Mode, hosted as window type `home` | `com.webos.app.seniorhome` · flutter · 172 KB; `WebOSCompositor/views/StarfishHomeView.qml` · compositor-qml · 61 lines | never | No | **Keep (dormant)**: Simple Mode is unsupported here (`seniorMode.support` false) | — | — |
| **Apps tile (LG Content Store)**: server-rendered store page: featured, categories, search, install | `com.webos.app.discovery` · web · 631 KB | sometimes | Yes · mock 12 | **Replace**: own all-apps grid with search and favourites (mock 12); LG's store stays reachable for installs | Key filter and launch redirect | 7 |
| **Launch splash**: full-screen app image from `splashBackground` (47 of 149 apps, incl. Netflix, YouTube, Prime, Plex), else grey default with 8 dots; skipped for Home and `noSplashOnLaunch` | `WebOSCompositor/views/StarfishImageSplash.qml` · compositor-qml · 333 lines | daily | Yes · mock 15 | **Replace**: one LumaTV launch screen; LG art only where an app ships its own | Shadow module | 5 |
| **Launch spinner and infinite spinner**: 72 px grey disc with a GIF, input-blocking, 500 ms delay, 120 s cap; only for overlay, popup and floating apps and when no splash shows; the owner-based infinite spinner has no caller here | `WebOSCompositor/views/StarfishSpinnerView.qml` · compositor-qml · 153 lines (2 instances) | sometimes | Yes · mock 15 | **Replace**: one shared LumaTV spinner; the `showSpinner` luna calls stay the trigger | Shadow module | 5 |
| **Long-press progress ring**: 72 px disc whose arc fills while Home, Mute, digit or AV-reset keys are held; only with Accessibility slow remote input | `WebOSCompositor/views/StarfishLongPressSpinnerView.qml` · compositor-qml · 154 lines | never | Yes · mock 15 | **Replace**: cheap with the spinners; `remoteSlowInput` is off, so it never shows today | Shadow module | 5 |
| **Recents (app switcher)**: row of recent app cards, drawn by Home from `recent/listRecents`; the compositor only keeps `recents.db` and a screenshot every 120 s | `WebOSCompositor/controllers/fullscreen/StarfishRecents.qml` · compositor-qml · with StarfishRecentsDb.qml, StarfishFullscreenCapture.qml, models/StarfishRecentsWindowModel.qml | sometimes | Yes · mock 14 | **Replace**: lives in LumaHome (close with `closeByAppId` before `removeRecent`); PLAN files it as a compositor restyle, but LG draws it in Home | Home stub | 1 |
| **Multi View window frames**: boundary line, option panel (more, mute, switch sound, mixing, quit, volume), PiP drag handle, standby tile for empty slots; hides after 10 s | `WebOSCompositor/views/decoration/StarfishServerDecoration.qml` · compositor-qml · 1038 lines + StarfishDecorationButton.qml, StarfishDecorationButtonLayer.qml, StarfishDecorationExtendedButtonLayer.qml, StarfishDecorationStyle.qml, StarfishServerDecorationService.qml | rare | Yes | **Restyle**: Multi View only; do last, and drop if Nitin never uses it | Shadow module | 5* |
| **Multi View picker**: Side by Side, PiP, Dual PC Monitor, 3-4 screens, recent and recommended combinations, options bar | `com.webos.app.multiviewsettings` · web · 5.4 MB; `multiviewcontroller` · service | rare | Part | **Restyle**: web overlay; takes the shared web theme with the Settings sub-apps; notices ("Multi View is running.") use the alert contract | Overlay mount (theme) | 6 |
| **Floating window host and FTMS info**: one free window above fullscreen, used only by the fitness-machine panel; its connect prompt is "Would you like to connect {fitness machine type} {model name}?"; the AirPlay toolbar (Move, Sound, Size, Full Screen, End) is dead because AirPlay runs as a card | `WebOSCompositor/views/StarfishFloatingView.qml` · compositor-qml · 146 lines + floating/StarfishFloatingSurface.qml, Toolbar.qml, ToolbarButton.qml, ToolbarStyle.qml, ToolbarAirPlayService.qml, ToolbarTtsService.qml; `com.webos.app.ftmsinfo` · web · 5.1 MB; `fitness-machine-service` · service | never | No | **Keep (dormant)**: no Bluetooth fitness gear here and the toolbar cannot trigger | — | — |
| **Fullscreen app host**: edge-to-edge window for every card app; draws only black backing, the Multi View standby tile and the "press and hold back" control-mode guide | `WebOSCompositor/views/StarfishFullscreenView.qml` · compositor-qml · 1262 lines; `WebOSCompositor/views/fullscreen/StarfishFullscreenContainer.qml` · compositor-qml · 1405 lines; `WebOSCompositor/views/fullscreen/StarfishFullscreenStandby.qml` · compositor-qml · 29 lines | daily | Part | **Keep**: window stacking, video holes and key routing stay LG's (PLAN: compositor not replaced); nothing visible but black | — | — |
| **Pointer right-click menu**: two round buttons, Back and Home, beside the pointer on a 30 % scrim; replays the key to the app below; not the hold-OK tile menu | `ContextMenu/ContextMenu.qml` · system-qml · 36 files / 14 qml; id `com.webos.app.contextmenu` | rare | Yes · no mock (mock 16 is the Home tile menu) | **Replace**: rebuilt resident; keep the two verbs and the key replay; [INFERENCE: the Magic Remote has no right button, so mostly a mouse opens it] | System UI redirect | 5 |
| **"More Actions" panel (on-screen remote)**: side panel: Quit App, User Guide, Info, Quick Help for apps; number pad, colour keys, Guide, Record, Scheduler on TV; trick-play and SIMPLINK keys for CEC devices | `ScreenRemote/ScreenRemote.qml` · system-qml · 133 files / 57 qml; id `com.webos.app.screenremote` | rare | Yes · no mock | **Replace**: rebuilt resident; Quit App is the one item an app user needs | System UI redirect | 5 |
| **Quick Access (number keys)**: digit held 1 s starts slot N with a toast, empty slot asks "add {app} to Quick Access N?"; 0 held opens the editor | `com.webos.app.magicnum` · web · 5.1 MB; `WebOSCompositor/controllers/base/StarfishMagicNumber.qml` · compositor-qml · 489 lines | sometimes | Part | **Replace**: LumaHome's dock numbers take over (mock 03); LG's digit handling stays untouched outside LumaHome | Key filter and launch redirect | 1* |

## 2. Inputs & devices

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Input picker**: bottom ribbon: Home Hub tile first, then one tile per connected input with icon, CEC name (AVR-S760H) and tick on the current one; add-on buttons (Eject, Universal Control, Channel Tuning); repeated Input presses step and a 5 s idle launches | `InputPicker/InputPicker.qml` · system-qml · 89 files / 51 qml; id `com.webos.app.quickinputpicker` | sometimes | Yes · mock 08 | **Replace**: rebuilt resident, own inputs only (no Home Hub tile) | System UI redirect | 4 |
| **Home Hub**: hub of TV inputs, ThinQ and Matter devices, Bluetooth, screen share, Google Home, USB and recordings; first tile of the input picker, opened by holding Input | `com.webos.app.homeconnect` · web · 22.5 MB | rare | No | **Off**: LumaTV's picker lists inputs only; repoint `keyaction.inputHub` | Key filter and launch redirect | 4 |
| **Touch edge swipes and Touch Lock**: curved bulge from the left or bottom edge with a back, home or recents icon; "Touch Lock" toast | `WebOSCompositor/views/StarfishGestureUIFeedbackView.qml` · compositor-qml · 464 lines | never | No | **Keep (dormant)**: the C4 has no touch panel | — | — |
| **HDMI input banner**: dark pill, lower left, 5 s: input name and CEC label; OK adds an Options button; INFO opens a detail banner (resolution, HDR, audio codec, Dolby Vision or Atmos, clock) | `com.webos.app.inputcommon` · native · 10.2 MB; `inputcommon/UserInterfaceLayer/Containers/MainViewLayer/InputBanner/*` · qml · SimpleBanner, InputBanner, DetailInfoBanner, MiniBanner (JP only) | sometimes | Part · no mock (mock 20 is the channel banner) | **Restyle**: PLAN names the Live TV banner only; the HDMI one is what Nitin sees most | Overlay mount | 6 |
| **No-signal screen**: black with a moving "No Signal" box and two hints, or 16 art photos every 10 s (`noSignalScreenSaver` is on); the box relocates every 10 s against burn-in; silent power-off after 860 s | `inputcommon/UserInterfaceLayer/Containers/BgViewLayer/Screensaver.qml` · qml · 849 lines + ScreensaverMainText, ScreensaverSubText, ScreensaverKeyHelp, PhotoSlide, RandomMoveItem | sometimes | Yes · mock 18 | **Restyle**: keep the burn-in mover and the 860 s auto-off; LumaTV look | Overlay mount | 6 |
| **Banner position editor**: four boxes (upper left, upper right, lower left, lower right) with a tick on the current one; OK saves `option.bannerPosition`, then the banner shows there; 5 s idle hides it | `inputcommon/UserInterfaceLayer/Containers/MainViewLayer/BannerEditView.qml` · qml · 215 lines + BannerItems/BannerEditItem.qml 84 | rare | No | **Restyle**: opened from Settings or the on-screen remote ("Change Banner Location"); same module as the banners | Overlay mount | 6 |
| **Live-preview launch fade**: black rectangle held, then a 400 ms fade, when an input starts as a live preview (PIP) from Home | `inputcommon/UserInterfaceLayer/Containers/BgViewLayer/AppViewEffect.qml` · qml · 72 lines | rare | Part | **Keep**: only LG Home's live tiles start it; an Inputs-row preview in LumaHome (PLAN reference) can reuse it | — | — |
| **Set-top and lock states on an input**: "Do you want to turn on your set-top box?" No/Yes (HDMI input set up as a box); "Input Locked" with OK to open the PIN prompt | `inputcommon/Interfaces/ScreensaverInterfaces/NoSignalAlertController.qml` · qml · 254 lines + PincodePromptController 56; `inputcommon/UserInterfaceLayer/Containers/PopupLayer/AlertPopupView.qml` · qml · 432 lines | rare | Part | **Restyle**: inputcommon popups share the Overlay mount | Overlay mount | 6 |
| **HDMI input apps 1-4**: no UI of their own: a transparent inputcommon window over the video plane; a cold launch shows `HDMI-N_splash.png`; HDMI 2 (Denon AVR, `lastInputApp`) is the daily one | `com.webos.app.hdmi1` · native · 99 KB; `com.webos.app.hdmi2` · native · 53 KB; `com.webos.app.hdmi3` · native · 53 KB; `com.webos.app.hdmi4` · native · 54 KB | daily | Part | **Keep**: window host over protected video; its banners and no-signal rows are above | — | — |
| **Inputs the C4 does not have**: DisplayPort, USB-C, AV, Component and SCART placeholders: same inputcommon window, never selectable | `com.webos.app.dp1` · native · 122 KB; `com.webos.app.dp2` · native · 63 KB; `com.webos.app.usbc1` · native · 126 KB; `com.webos.app.usbc2` · native · 67 KB; `com.webos.app.externalinput.av1` · native · 64 KB; `com.webos.app.externalinput.av2` · native · 64 KB; `com.webos.app.externalinput.component` · native · 90 KB; `com.webos.app.externalinput.scart` · native · 2.1 MB | never | No | **Keep (dormant)**: `eim` reports 4 HDMI ports and none of these (`maxDisplayportCount` 0, `maxUsbcCount` 0) | — | — |
| **HDMI or CEC device connected**: input alert: white parallelogram cards with port icon and name ("Do you want to switch to the screen of the connected device?", "[AVR-S760H] is connected.", "SIMPLINK : On", "The current input has been changed to {device1} from {device2}.") | `eim` · service | sometimes | Yes · mock 18 | **Replace**: same item contract as toasts and alerts | View swap | 2 |
| **HDMI format prompts**: "the Ultra HD Deep Color of the current HDMI port will be changed to 8K" with Go to Settings; "Audio is not supported" | `extinput-integration` · service | rare | Part | **Replace**: plain alert contract | View swap | 2 |
| **Universal Control (IR blaster, set-top box)**: wizard for the TV remote to drive a box (input, brand, model, key tests); prompts "set the current input as the detected device?" and "Additional Settings have been detected ... set up Universal Control Settings?" (Accept, Decline, Block); Universal Control cards in the on-screen remote | `com.webos.app.connectionwizard` · web · 8.7 MB; `irdbmanager` · service | rare | Part | **Restyle**: web app themed with the Settings sub-apps; alerts by view swap | Overlay mount (theme) | 6 |
| **USB and storage prompts**: "USB device is connected.", "Do you want to browse the file from the connected device?", format prompts, eject warnings, USB-stick verification for recording, wired headphones toast | `physical-device-manager2`, `asm`, `audiod` · service | rare | Yes | **Replace**: toasts and alerts through the one renderer | View swap | 2 |
| **Media Player**: file browser and player for USB, DLNA and cloud photos, video and music; subtitles, 360 VR, lyrics | `com.webos.app.mediadiscovery` · web · 43.1 MB | rare | No | **Keep**: Plex covers media; LG's logic is the screen; theme only if it is an Enact app (unchecked) | — | — |
| **Camera**: live preview, camera list, screen position, apps using the camera | `com.webos.app.camera` · web · 10.2 MB; `starfish-camera-plugins` · service | never | No | **Keep (dormant)**: no camera on the set; its prompts use the alert contract | — | — |
| **Bluetooth keyboard, mouse and gamepad pairing**: "[Device] wants to connect to your TV. Do you want to connect it?"; "Enter the confirmation code {passKey} on the keyboard" | `bthidmanager` · service | rare | Part · mock 22 | **Replace**: alert contract; Settings › Inputs & devices lists the devices | View swap | 2 |
| **Magic Remote status**: "Magic Remote battery is low", "signal is unstable", "Press OK(Wheel) to pair" | `lginput2` · service | rare | No | **Replace**: toasts and alerts; battery warning must never be filtered | View swap | 2 |
| **On-TV button panel**: the TV's one-key joystick: press opens a small Local Control Panel; Quick Settings and the input picker show "Press: OK, hold: Power Off"; first power-on by the button shows a key guide for 5 s | `com.webos.app.tvhotkey` · native · 5.3 MB; `inputcommon/UserInterfaceLayer/Containers/PopupLayer/LocalKeyView.qml` · qml · 136 lines + LocalKeyPowerOnOSD.qml 182; `QuickSettings/Containers/LocalKeyGuidance.qml` · system-qml · 123 lines + InputPicker/Containers/LocalKeyGuidance.qml 96 | rare | No | **Keep**: the recovery path when the remote is lost; LumaTV's panels must still accept the `localkey` launch | — | — |
| **Wi-Fi Direct, cable and Bluetooth turn-on toasts**: "Wired LAN cable disconnected.", "[%s] wants to connect via Wi-Fi Direct. Accept?", "{Mobile} is ready to connect." | `event-monitor-network-starfish`, `bluetooth-asc-service` · service | rare | No | **Replace**: alert contract | View swap | 2 |

## 3. Volume & sound

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Volume OSD**: right-edge for 1.5 s; on this set (`external_arc`, `volumeSyncable` false) a speaker with + and - and a highlighted circle, no number or bar; TV speaker, headphones, Bluetooth, WiSA and WOWCAST show icon, number and bar | `WebOSCompositor/views/StarfishVolumeView.qml` · compositor-qml · 240 lines; `WebOSCompositor/views/volume/StarfishVolume.qml` · compositor-qml · 484 lines | daily | Yes · mock 11 | **Replace**: level comes from the Denon (`MV` events), not webOS's drifting counter | Shadow module | 4 |
| **Mute bubble**: 96 px bubble at the right edge while muted (kept for `external_arc`), hops 94 px every 10 s against burn-in; Mute held 2 s opens the accessibility panel instead | `WebOSCompositor/views/volume/StarfishVolumeMute.qml` · compositor-qml · 130 lines | sometimes | Yes · mock 11 | **Replace**: Denon `MU` events drive it; keep the burn-in hop | Shadow module | 4 |
| **ARC, soundbar and wireless-speaker suggestions**: "change Digital Sound Output to Pass Through to minimize audio delay while playing game?", "LG sound bar is now connected", WOWCAST and WiSA reconnect messages | `arccontroller`, `wowplay`, `wisa-manager`, `wireless-av-controller` · service | rare | No | **Replace**: alert contract; the LG-soundbar texts cannot appear with the Denon | View swap | 2 |
| **Bluetooth headphone or speaker connection**: "[Device] is connected.", "Failed to connect to [Device]", "reconnect the previously connected [Device]?"; Sound Out falls back to the TV speaker when it drops | `bluetooth-audio-source`, `bluetooth-le-audio` · service | rare | No | **Replace**: toasts and alerts through the one renderer | View swap | 2 |
| **Auracast broadcast settings**: Start or Stop Broadcasting, TV name, authentication code, audio quality, registered devices, sound test | `com.webos.app.leaudio` · web · 9.4 MB | never | No | **Keep (dormant)**: sound goes to the Denon over eARC; the app says some Sound Out modes block Auracast | — | — |
| **Personalized Sound Wizard and AI Acoustic Tuning**: pick audio samples for vocal tone, bass-treble balance and spatial sound, then Before/After and Apply; the tuning app plays an 8 s test heard by the Magic Remote microphone | `com.webos.app.soundwizard` · web · 5.0 MB; `com.webos.app.onetouchsoundtuning` · web · 9.0 MB | rare | Part | **Restyle**: Enact web apps; sound goes to the Denon, so they are seldom opened | Overlay mount (theme) | 6 |
| **Bluetooth surround tuning and LG soundbar app**: one or two Bluetooth speakers, auto and manual tuning; LG soundbar mode, room calibration, EQ, night mode | `com.webos.app.btsurroundautotuning` · web · 15.7 MB; `com.webos.app.soundbar-setting` · web · 12.2 MB | never | No | **Keep (dormant)**: needs Bluetooth speakers or an LG soundbar; the Denon is neither | — | — |

## 4. Notifications & prompts

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Toast: standard**: grey bar 900 px wide, bottom centre, 1-3 lines of 30 px text, no buttons, slides up 400 ms, 5 s default (1-10 s); also carries the key-handler "not available" messages | `SystemUI/Notification/Toast.qml` · system-qml · 286 lines + ToastView.qml 149 | daily | Yes · mock 07 | **Replace**: rebuilt to the item contract, upper right (mock 07); Home Assistant `notify` arrives here too [INFERENCE: via apiadapter] | View swap | 2 |
| **Toast: advanced (icon and link)**: 66 px icon plus text; with `onclick` it is a pointer link (hover scale 1.02) that runs the action and closes; no key focus | `SystemUI/Notification/Toast.qml` · system-qml · type advanced | rare | Yes | **Replace**: keep the click action: it calls `surfacemanager` | View swap | 2 |
| **Toast: light**: small 50 % black pill, top right, 36 px icon and text (e.g. "Touch Lock") | `SystemUI/Notification/Toast.qml` · system-qml · type light | never | Yes | **Replace**: same renderer; only the touch-lock toast uses it and the C4 has no touch panel | View swap | 2 |
| **Alert: standard and modal**: light-grey card 1014 px wide, 180 px above the bottom, optional title, up to 10 lines, 225x54 buttons; modal adds a 0.3 scrim and ignores Back, Home and scrim clicks | `SystemUI/Notification/Alert.qml` · system-qml · 672 lines + AlertView 194, SimpleButton 203, Selectable 202, MoonstoneShadow 114 | sometimes | Yes · mock 06 | **Replace**: a button is a luna call made as the source app (`callForApplication`); keep that | View swap | 2 |
| **Alert: full-screen warning**: black page, 72 px title, 39 px body, centred 270x72 buttons; used by power-off alerts, second-screen PIN pairing and Usage Care | `SystemUI/Notification/Alert.qml` · system-qml · type warning | rare | Yes · mock 06 (card only) | **Replace**: same contract, full-screen layout | View swap | 2 |
| **Alert: icon, checkbox, countdown, busy, progress**: wider card with icon or thumbnail, tick box line, auto-close countdown with spinner, busy ring, 750x6 progress bar fed by `setAlertProgress` | `SystemUI/Notification/Spinner.qml` · system-qml · 208 lines + ProgressBar 63, CheckBoxItem 128 | rare | Yes | **Replace**: one card, variants by field; progress and countdown need the update signal | View swap | 2 |
| **Input alert**: white parallelogram cards 300x262 with port icon and title, close button above, 0.6 scrim, bounce-in; created by `eim` and `asm` for connected devices and USB | `SystemUI/Notification/AdvancedAlertView.qml` · system-qml · 210 lines + AdvancedAlertDelegate 238, AdvancedAlertCard 264 | sometimes | Yes · mock 18 | **Replace**: mock 18 shows it as a wide toast with Switch focused | View swap | 2 |
| **PIN prompt**: full-screen black page: title, four boxes, 0-9 keypad, backspace, OK; wrong code shows "Check Password" and the third try closes it; first-use and Settings add set-new and confirm pages | `SystemUI/Notification/PincodePrompt.qml` · system-qml · 734 lines + PincodePromptView 171, PincodePromptButton, PincodePromptNumButton, PincodePromptPasswordButton, PincodePromptBaseButton | rare | Yes · mock 19 | **Replace**: digits return through `closePincodePrompt` and the daemon validates; keep that relay | View swap | 2 |
| **Notification variants that cannot appear**: partial-view alert and PIN strip (rollable TVs), secure arrow-pad PIN, PIN tooltip with empty text, 18 unused 9-slice images | `SystemUI/Notification/Tooltip.qml` · system-qml · 75 lines + PincodePromptArrowpad 30, ArrowLockInput 108 | never | No | **Keep (dormant)**: `supportScreenType` none, `pincode` methods keyboardMode only | — | — |
| **Second-screen pairing prompt**: "Connection Request: {name}" Yes/No, or the PIN as the title with Cancel; shown when a phone, LG TV Plus or Home Assistant `webostv` pairs; wakes the screen first | `com.webos.service.secondscreen.gateway` · service | rare | No | **Replace**: must stay answerable only by a physical remote key (Principle 0); buttons call `pairing/acceptPairing` | View swap | 2 |
| **Exit-app alert**: "Do you want to exit the app?" Exit/Cancel, non-modal, from Back or Close at an app that does not take Back | `KeyFilters/systemUi.js` · js · handleSystemKeys | rare | Yes | **Replace**: alert contract | View swap | 2 |
| **Notification Centre**: list with unread badge, delete mode, select all, mark all read, reminder settings, groups; no key opens it, Home has a bell and only Japan gets a Screen Remote button | `com.webos.app.notificationcenter` · web · 9.8 MB | rare | Yes · mock 13 | **Replace**: own drawer reading the same history | Key filter and launch redirect | 7 |
| **App permission prompts**: "Do you want to enable microphone access on {appId}?", file access, Nearby Devices; Allow or Block | `ppm`, `blegatt` · service | rare | No | **Replace**: alert contract; granting stays a human choice | View swap | 2 |
| **Software update prompt**: "The latest version of the software is available for your TV. Do you want to update now?", "Restart your TV to finish installing important updates"; the banner adds a download badge | `update` · service | rare | Part · mock 22 | **Replace**: 'Not now' focused; a firmware update can undo the root | View swap | 2 |
| **App launch prompts**: "A new version of the app is available. Launch current version of app or update to latest version.", "You must update this app to launch it ... Do you want to update?", "This app or service is currently not available in your country." | `sam` · service | rare | No | **Replace**: alert contract, Launch focused; [INFERENCE: more often while LG app updates are blocked] | View swap | 2 |
| **App, AI package and storage messages**: "Application has been installed.", "Some apps are available for update", "AI 8K Upscaling is Updated.", "Not enough storage available" | `appinstalld`, `pkginstall`, `adaptive-partition-installer`, `ai-inference-manager`, `nn-proxy`, `aiservice` · service | rare | No | **Replace**: alert contract | View swap | 2 |
| **Low-memory app restart**: "This app will now restart to free up more memory." | `memchute` · service | rare | No | **Replace**: toast; a 2 GB set restarts heavy apps now and then | View swap | 2 |
| **Smart-home and ThinQ messages**: appliance finished or door open, DR Mode, Independa incoming call | `iot-client-webos`, `independa-notification`, `google-home-controller`, `lg.thinqai.adapter` · service | never | No | **Keep (dormant)**: no ThinQ appliances or account known on the set | — | — |

## 5. Settings & wizards

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Quick panel**: left-edge card 492x1056: icon row (All Settings, Chatbot, Accessibility, Network), "Recent History", 2-column grid of up to 12 tiles; on this set Picture Mode, OLED Pixel Brightness, Sleep Timer, Game Optimizer, Screen Off, Energy Saving | `QuickSettings/QuickSettings.qml` · system-qml · 799 files / 165 qml; id `com.webos.app.quicksettings` | sometimes | Yes · mock 10 | **Replace**: built and resident; LG rebuilds per open; tiles drive the same `settingsservice` keys | System UI redirect | 3 |
| **Quick panel popups and edit mode**: slider popup with value text, value picker with sub-buttons (Advanced Settings, wizards, Device List), edit mode (Move or Delete), "Select a menu to register" list, "Add to Favorites"; delete, Bluetooth-guide and disabled-tile messages go through the notification layer | `QuickSettings/Containers/BarSliderPopup.qml` · system-qml · with DotPickerPopup, MenuPopupArea, GridEditor, EditOptional, QuickSettingsCandidateListView | sometimes | Yes · no mock (mock 10 is the panel with steppers) | **Replace**: same module; alert buttons call back with `params.action`, keep that | System UI redirect | 3 |
| **Quick panel tile set**: every tile LG can place (12 at most): Picture Mode, Sound Mode, Sound Out, Sleep Timer, Screen Off, Eye Comfort, Game Optimizer, Multi View, Music Search (needs an LG server answer), sliders (OLED Pixel Brightness, Black Level, Contrast, Sharpness, Color, Color Temperature, Tint), launchers (Energy Saving, OLED Care, Care Picture, Privacy & Terms, External Devices), Time Locks, Volume Limit | `QuickSettings/Controllers/PictureModeInterfaces` · system-qml · 34 controller folders (about 2 qml each); saved list `other.quickSettingsMenuList` | sometimes | Yes · mock 10 (6 tiles shown) | **Replace**: LumaTV picks its own tiles from the same settings keys; this set holds picture, brightness, timer, game, screen off, energy saving | System UI redirect | 3 |
| **Quick panel pieces that cannot appear**: Easy (Senior) panel, Touch Lock, Battery Brightness, Wireless AV, External Devices tiles, local-key guidance strips | `QuickSettings/Containers/SeniorTVQuickSettings/EasySettingsWindow.qml` · system-qml · 362 lines + 6 qml | never | No | **Keep (dormant)**: `seniorMode.support` false, no touch panel, no battery, `supportWirelessAV` false, one-key local button | — | — |
| **Settings**: tabs Picture, Sound, General, Support; General holds Accessibility, AI Service, Game Optimizer, Channels, Network, System, Energy Saving, Usage Care, OLED Care; gear held 1 s opens it, apps call it with {target, item} | `com.palm.app.settings` · web · 25.0 MB | sometimes | Yes · mocks 01-04, 24 | **Replace**: own resident Settings in nine sections; same `settingsservice` writes | Key filter and launch redirect | 3 |
| **Settings sections without a mock**: Inputs & devices, Network, Ambient light, Home & look, System (time, language, OLED care, accessibility rows, About) | `com.palm.app.settings` · web · 25.0 MB | sometimes | Yes · no mock | **Replace**: sections per `settings-map.md`; Ambient light is LumaTV-only | Key filter and launch redirect | 3 |
| **Settings deep screens and sub-apps**: expert calibration, channel tuning, legal, LG account, Network, Family Settings, software update: "More in LG settings" rows open them directly | `com.palm.app.settings` · web · 25.0 MB | rare | Yes | **Restyle**: Enact Sandstone bundles; one shared theme | Overlay mount (theme) | 6 |
| **Setting-change confirmations and suggestions**: "Picture Mode has changed.", energy-use notices, "Do you want to keep the changed screen settings?", "When movie content is recognized, you can change to Filmmaker mode.", "You are watching TV quite a lot." | `pqcontroller`, `com.webos.service.ocp`, `lightingcontroller` · service | rare | No | **Replace**: alert contract; drop the recommendation ones by source (Filmmaker mode stays) | View swap | 2 |
| **Personalized Picture Wizard**: pick images for preference, sharpness, colour tone and temperature, then Before/After and Apply; saves to the LG account when signed in | `com.webos.app.picturewizard` · web · 15.0 MB | rare | Part | **Restyle**: Enact web app; Filmmaker mode stays the picture mode | Overlay mount (theme) | 6 |
| **Support tools**: Quick Help symptom list and recommended settings, picture and sound self-test, LG Remote Service (shows a code to read to an LG agent); nothing opens them unasked | `com.webos.app.self-diagnosis` · web · 6.5 MB; `com.webos.app.customersupport` · web · 19.6 MB; `com.webos.app.remoteservice` · web · 4.9 MB | rare | Part | **Restyle**: Enact web apps; Remote Service needs Nitin to read out the code | Overlay mount (theme) | 6 |
| **Help and guides**: Tips cards, offline TV User Guide (Help key is not on the Magic Remote), "Try Now" links into Settings | `com.webos.app.helpandtips` · web · 18.4 MB; `com.webos.app.tvuserguide` · web · 104.1 MB | rare | Part | **Restyle**: Enact web apps | Overlay mount (theme) | 6 |
| **Usage Care**: weekly viewing time and top apps, Time Locks, Parental Control Mode, Eye Care, Volume Limit, input, app and channel locks; the service opens it at end of use when limits are set | `com.webos.app.familycare` · web · 7.1 MB | rare | Part | **Restyle**: Enact web app | Overlay mount (theme) | 6 |
| **Weather location**: city search that stores the Home weather location; needs LG weather servers | `com.webos.app.weatherlocation` · web · 4.5 MB | rare | Part | **Off**: LumaHome's weather widget has its own location | Launcher filter | 7 |

## 6. Live TV

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Live TV app**: no UI of its own: inputcommon in Live TV mode over the tuner video; operator-app variant unused | `com.webos.app.livetv` · native · 261 KB; `com.webos.app.livetvopapp` · native · 87 KB | rare | Part | **Keep**: window host; its pixels are inputcommon rows below | — | — |
| **Channel banner**: lower-left strip: channel number (105 px), logo, source icon, programme, start-end time and progress bar; OK adds Options and Guide buttons; 5 s | `inputcommon/UserInterfaceLayer/Containers/MainViewLayer/ChannelBanner/ZappingBanner.qml` · qml · 574 lines + ChannelBanner 283, DeeplinkView 74 | rare | Yes · mock 20 | **Restyle**: the one Live TV surface PLAN restyles (Decided 2026-10-23) | Overlay mount | 6 |
| **Channel detail banner**: programme list (previous, now, next) with times, progress, synopsis scroll, rating, audio and subtitle icons, REC reservation; INFO key, 30 s | `inputcommon/.../ChannelBanner/DetailInfoBanner.qml` · qml · 373 lines + DetailInfoItem 786, SynopsisScrollBar 58, ProgressBar 81 | rare | Part | **Restyle**: same module as the banner; no guide screens | Overlay mount | 6 |
| **Guide and channel list**: left tab menu (TV Guide, Channels, Scheduler), channel list, EPG time grid, PIP, options; Guide, Channel List and Live TV Menu keys | `com.webos.app.livemenu` · native · 4.6 MB | rare | No | **Keep**: Nitin: Live TV is rare, no guide (2026-10-23) | — | — |
| **Channel number entry**: list of channels matching typed digits (ATSC dash major-minor); Italy-only overlay app for number keys | `inputcommon/.../SimilarListView.qml` · qml · 650 lines; `com.webos.app.similarlist` · flutter · 5.4 MB | rare | No | **Keep**: antenna use only; the Flutter overlay is Italy-only | — | — |
| **TV Scheduler**: list of reminders and scheduled recordings with start time, title, channel; add or edit with storage info | `com.webos.app.scheduler` · web · 8.8 MB | rare | No | **Restyle**: Enact web app; Live TV only | Overlay mount (theme) | 6 |
| **Channel tuning and Channel Manager**: auto and manual tuning for antenna, cable and satellite with progress; list with All and Favourites, edit, delete, lock, sort | `com.webos.app.channelsetting` · web · 23.1 MB; `com.webos.app.channeledit` · web · 6.5 MB | rare | Part | **Restyle**: Enact web apps; done once per antenna | Overlay mount (theme) | 6 |
| **Emergency alert (US EAS)**: popup, bottom then top every 2 min: title ("BLUE ALERT"), locations and text, scrollbar, Close; video keeps playing; auto-hides after 2 min | `inputcommon/.../PopupLayer/EwsUsView.qml` · qml · 369 lines | rare | No | **Keep**: safety text from the broadcaster; appears only on the antenna input | — | — |
| **Live Pick and recording indicators**: Live Pick strip ("Would you like to learn more about ...?") and REC or Live Playback chips on banners | `inputcommon/.../LivePick/LivePickView.qml` · qml · 549 lines + 3 qml; RecInfoIcon, DvrIndicatorView | never | No | **Off**: `livepick` service absent, `broadcastInfoNoti` off, `dvrReady` false | LG setting | 7 |
| **DVR, recordings and live playback**: transport bar, info chip, storage and CI-password dialogs, record dialogs, recordings list | `com.webos.app.dvrpopup` · native · 1.1 MB; `com.webos.app.recordings` · web · 8.3 MB; `inputcommon/.../Dvr/DvrMainView.qml` · qml · 742 lines + DvrPlayView, DvrProgressBarView, DvrIconButton, DvrFormattedTimeText, DvrInfoView, DvrPopupView, DvrCiPincodeView, DvrInputText, LoadingMessageView | never | No | **Keep (dormant)**: `dvrReady` false in `tv.model` and `tv.rmm` | — | — |
| **Broadcast families that cannot appear**: CI+ menus, Teletext, LCN pickers (UK, Italy), Music Catch, Ginga list, EMM and CA messages, Japan mini banners, Freeview Play splash, Indonesia EWS, on-screen clock, voice zapping, DPMS standby warning, hybrid-TV spinner, LG Channels splash and activation; HbbTV, Ginga, Interactive and Red Button layers | `com.webos.app.crb` · web · 6 KB; `com.webos.app.livedmost` · web · 142 KB; `hybridtv-manager`, `hbbtv3-pluggable`, `opapp-manager`, `dmost` · service; `inputcommon/UserInterfaceLayer/Containers/PopupLayer, BgViewLayer` · qml · 19 regional views: CiPopup, TeletextOptionView, LcnConflict*, MusicCatchPopupView, GingaAppCatalog, EwsView, EmmMessageView, CaMessageView, FvpSplash, ClockView, VoiceChannelZapping, DpmsPopup, LoadingView, LgChannel* | never | No | **Keep (dormant)**: US ATSC set, HbbTV off, no CAM, country gates | — | — |
| **Live TV prompts**: "To watch Live TV, channels must be tuned. Start Channel Tuning now?", "The program you set a reminder for will begin in 1 minute", "TV is turned on because your scheduled program is about to start", "Recording stopped. Not enough storage to record." | `broadcast-systemui-manager`, `tv-reservation-agent`, `gibbips` · service; `inputcommon/Interfaces/ChannelStatusInterfaces` · qml · 514 lines | rare | No | **Replace**: alert contract; inputcommon pop-ups restyle with the banner | View swap | 2 |
| **My Channels wedge**: ribbon of up to 20 favourite channels under Home with Add Channel, edit and empty state; shown only where a host loads it | `FavoriteChannels/FavoriteChannels.qml` · system-qml · 141 files / 51 qml | never | No | **Off**: LumaHome has no channel wedge; the channel list is empty here | Home stub | 1 |

## 7. Search, voice & keyboard

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **On-screen keyboard**: IME panel at the bottom with a 600 ms slide, drawn by maliit; opens on a text field or an app request, closes on app change, alert or Home | `WebOSCompositor/views/StarfishKeyboardView.qml` · compositor-qml · 342 lines; IME `com.webos.service.ime`, maliit 85 qml | sometimes | Yes · mock 09 | **Restyle**: deterministic-row keyboard look on LG's IME | Overlay mount | 6 |
| **Search and voice results**: answers and result lists (apps, shows, channels, sports, weather, shopping, music), keyword grid with Sponsored keywords, chat box, Voice ID, keyboard; the Movies key opens it with "movie" | `com.webos.app.voice` · native · 14.4 MB | sometimes | Yes · mock 09 | **Restyle**: mock 09 shows local results; LG's cloud answers need Nitin's call (Needs Nitin) | Overlay mount | 6 |
| **Voice listening bar**: 540x90 light bar lower left: animated chrome, "Speak now.", live transcript and reply; Back and Exit go to it; opened by the mic key through `voiceconductor` | `com.webos.app.voiceagent` · system-qml · 7.5 MB; `VoiceAgent/VoiceAgentView.qml` · system-qml · 1954 files / 10 qml (1940 animation PNGs, 3 earcons); host WebOSCompositor/views/StarfishVoiceAgentView.qml 31 lines | sometimes | No | **Restyle**: keep the earcons and states, replace the 485-frame animation with one arc | Shadow module | 6* |
| **Microphone indicator**: 81 px green microphone, bottom right, fades to 60 % after 3 s; `showIndicator` has no known caller here | `WebOSCompositor/views/StarfishIndicatorView.qml` · compositor-qml · 106 lines | rare | No | **Replace**: tiny; do with the other shadow-module views | Shadow module | 5* |
| **AI Concierge**: welcome message, keyword chips, tips and content cards (EPG, VOD, YouTube) with a content-provider chooser; needs LG terms | `com.webos.app.aiplatform` · flutter · 7.9 MB | rare | No | **Off**: LG cloud assistant; LumaHome has no entry to it | Launcher filter | 7 |
| **AI voice settings and Voice ID**: hands-free, far-field and sensitivity settings, help categories, Voice ID register and manage with a recording flow | `com.webos.app.aiplatformsupport` · flutter · 26.8 MB | rare | No | **Keep**: Flutter, reached only from Settings › AI Service | — | — |
| **TV help chatbot**: chat bubbles, suggestion buttons, toggles, brightness slider, QR codes; the quick panel shows its icon when the app exists | `com.webos.app.assistant` · native · 1.0 MB | rare | No | **Off**: LG cloud helpdesk; LumaTV's quick panel does not link it | Launcher filter | 7 |
| **Voice web overlay**: transparent overlay hosting an iframe URL given at launch (web results, hints), closes by message | `com.webos.app.voiceweb` · web · 23 KB | rare | No | **Keep**: launched by the voice framework with a URL; nothing of its own to restyle | — | — |
| **Other assistants**: Amazon Alexa view and presentation overlay, Yandex Alice, Dangbei voice hints (China) | `amazon.alexa.view` · web · 1.4 MB; `amazon.alexapr` · web · 1.3 MB; `yandex.alice` · web · 4 KB; `com.webos.app.dangbei-overlay` · web · 42 KB | never | No | **Keep (dormant)**: Alexa needs the `voice2` terms (false here); Alice and Dangbei are other markets | — | — |
| **Voice notices**: "Voice recognition will be available by connecting TV to the network", "The built-in mic is turned off.", Alexa and Voice ID messages, TTS language pack prompts | `voice-framework`, `voiceinput`, `amazon-alexa-adapter`, `tts` · service | rare | No | **Replace**: toasts and alerts through the one renderer | View swap | 2 |

## 8. Power, boot & screensaver

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Boot logo**: animated "webOS" wordmark (88-frame sprite, 60 ms per frame) over a 1.2 s fade, closes with a 1.2 s fade, 10 s cap; launched by `bootd`; the LG-logo switch is `other.lgLogoDisplay` | `BootLogo/BootLogo.qml` · system-qml · 142 files / 5 qml; id `com.webos.app.bootlogo`, 520 lines | sometimes | Yes · mock 23 | **Replace**: cold boots only; the set normally resumes from Active Standby | System UI redirect | 5 |
| **Power-off logo**: black scrim fades in over 1 s while the red LG logo fades in 1 s and out 1.5 s (about 3 s); `tvpowerd` waits for the compositor to report it finished | `MasterBrand/MasterBrand.qml` · system-qml · 21 files / 5 qml; id `com.webos.app.masterbrand`, 365 lines | daily | Yes · no mock (mock 23 is the boot screen) | **Replace**: own animation; must answer `responsePowerOffLogoRequest` or power-off stalls | System UI redirect | 5 |
| **Startup cover**: black screen with the first app name (120 px) and "Loading..."; opened by `bootd`, launches the first app, emits `smart-service-ready`, swallows keys; also the stolen-device block screen [INFERENCE: whether a warm wake shows it] | `CoverUI/CoverUI.qml` · system-qml · 55 files / 35 qml; `WebOSCompositor/views/StarfishCoverUIView.qml` · compositor-qml · 67 lines; `WebOSCompositor/views/cover/StarfishCoverUI.qml` · compositor-qml · 16 lines | sometimes | No | **Restyle**: boot dependency: restyle the picture, keep the sequencing | Shadow module | 5* |
| **Boot popup**: black bar 130 px high slides up: "One moment please. Smart service will be available shortly."; a hotkey pressed before boot finishes | `WebOSCompositor/views/StarfishBootPopup.qml` · compositor-qml · 121 lines | rare | No | **Replace**: small; same toast look | Shadow module | 5* |
| **Screensaver**: default particles with text, analogue or digital clock with date, sponsored image slideshow (off), Gallery+ art (off); starts when `tvpowerd` reports Screen Saver, any key wakes it; a separate 120 s compositor timer closes overlays and alerts | `com.webos.app.screensaver` · flutter · 13.1 MB; `WebOSCompositor/views/StarfishScreenSaverView.qml` · compositor-qml · 136 lines; `WebOSCompositor/views/screenSaver/StarfishScreenSaver.qml` · compositor-qml · 40 lines | sometimes | Yes · mock 21 | **Replace**: drawn in the compositor with OLED pixel shift; `tvpowerd`'s idle question is answered no while Home is in front, so LG's app never starts, and any key ends it | Shadow module (research) | 8 |
| **Video screensaver container**: blank web page that loads an LG Channels video-ad page | `com.webos.app.videoads` · web · 9 KB | never | Part | **Off**: `lgchannels.isVideoScreenSaverEnabled` false and LG hosts blocked | LG setting | 7 |
| **Always Ready and Gallery+**: art and clock when the set is "off": themes, AI art, photos, briefing; `lifeOnScreenMode` is `alwaysReady` and the power state reads Screen Saver in standby [INFERENCE: what is drawn is unverified]; the service asks "Going to {screen} screen within 1 minute" OK or Cancel | `com.webos.app.lifeonscreen` · web · 84.9 MB; `alwaysready` · service | daily | Part | **Replace**: LumaTV's screensaver or standby look takes over (Needs Nitin) | Shadow module (research) | 8 |
| **Line View (rollable TVs)**: clock, music player, weather, home menu for partial view of rollable TVs | `com.webos.app.atelier` · web · 125.1 MB | never | No | **Keep (dormant)**: `supportScreenType` is none on a fixed panel | — | — |
| **Auto power-off countdown**: toast "TV will be turned off in 5 minutes." then alert "in 1 minute ... press any button except Power"; the silent 15-minute no-signal off has no UI | `nopd` · service | rare | Part · mock 17 | **Replace**: toast then alert; countdown toast with a draining bar (mock 17) | View swap | 2 |
| **Sleep timer and timer power-off**: Sleep Timer tile (Off, 10 min to 4 h) and the one-minute alert "Your TV will be turned off ... Cancel or OK" | `tv-reservation-agent` · service | rare | Part · mock 17 | **Replace**: quick-panel tile plus the alert contract | View swap | 2 |
| **OLED Pixel Cleaning prompts**: "Pixel Cleaning will start automatically when the TV is turned off", "Do you want to start right now?", "Pixel Cleaning is in progress", "do not unplug the power" | `eplmanager` · service | rare | Part | **Replace**: alert contract; care rows live in Settings › System | View swap | 2 |
| **Thermal and fan shutdown**: "Your TV will turn off shortly as the TV's temperature is too high" and the cooling-fault text | `fancontroller` · service | rare | No | **Replace**: alert contract; never filtered, it is a safety warning | View swap | 2 |

## 9. Game

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Game Dashboard**: bottom-centre 962x309 panel replacing the quick panel in game mode: Dark Room, Multi View, Menu Color, Game Optimizer, genre carousel, live strip (FPS, VRR, Low Latency, HDR, input lag), edit screen; HDMI game mode only | `QuickSettings/Containers/QuickGameSettings/QuickGameSettingsMainView.qml` · system-qml · 624 lines + GameEditMainMenu, QuickGameGenreButton, QuickGameInformation, QuickGameGuide, GameLocalKeyGuidance | rare | Part | **Replace**: the quick panel's game card (PLAN, Glasshouse); only matters if a console or PC is on HDMI (Needs Nitin) | System UI redirect | 3 |
| **Game Optimizer**: genre, picture and sound controls, FPS counter, crosshair, VRR and low latency, audio out, menu colour, screen position | `com.webos.app.gameoptimizer` · web · 6.6 MB | rare | Part | **Restyle**: Enact web overlay; PLAN keeps a Settings row that opens it | Overlay mount (theme) | 6 |
| **Games hub**: recommended games and gameplay videos, recent, console inputs, Bluetooth controller settings; needs the LG network | `com.webos.app.gamehome` · web · 11.1 MB | never | No | **Off**: cloud shelf; Moonlight and GeForce NOW are separate apps | Launcher filter | 7 |
| **Game FPS counter and crosshair**: FPS badge in a corner and a centre cross or dot on HDMI game input | `inputcommon/.../BgViewLayer/GameOptionView.qml` · qml · 57 lines + GameFpsItem 244, GameCrosshair 49 | never | No | **Keep (dormant)**: needs the `monitor` profile, which this set lacks | — | — |

## 10. Casting & sharing

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **AirPlay**: Apple receiver in a native shell: video with transport OSD, audio now-playing, mirroring, PIN panel, info and HomeKit "curtain" card, remote keyboard; prompts "AirPlay is disconnected.", Wi-Fi turn-on | `airplay` · native · 29.3 MB; `airplay-adaptor` · service | rare | No | **Keep**: Apple's receiver UI; its prompts use the alert contract | View swap (prompts) | 2 |
| **Screen Share (Miracast) and App Casting**: waiting screen with the TV name, phone screen full-screen with a top menu (Options, Size, Move, Hide, End), "requests screen sharing to your TV" Accept, Decline or Block with a PIN; App Casting asks "exit screen sharing?" | `com.webos.app.miracast` · web · 5.1 MB; `com.webos.app.appcasting` · web · 4.6 MB; `miracast`, `appcasting` · service | rare | No | **Keep**: mirrored phone content is the screen; prompts use the alert contract | View swap (prompts) | 2 |
| **Google Cast**: Cast receiver runtime with its own screen; LG adds the terms and network dialogs and a disconnect toast; the settings page is Google's | `com.webos.chromecast` · native · 16 KB; `com.webos.chromecast-settings` · web · 16 KB; `chromecast-provisioning` · service | rare | No | **Keep**: Google's cast screen; dialogs use the alert contract | View swap (dialogs) | 2 |
| **DLNA remote control**: "The external device is requesting TV control. Do you want to continue?", "Buffering is in progress..." | `dmr` · service | rare | No | **Replace**: alert contract | View swap | 2 |
| **Sound Share**: phone audio through the TV: black "Playing the sound from your connected device." screen, Bluetooth Audio Playback panel (device list, play, pause, Screen off), "{Device} requests Sound Share to your TV" Accept, Decline or Block | `com.webos.app.hdmioptical` · qml · 293 KB; `com.webos.app.btspeakerapp` · web · 5.6 MB; `bluetooth-audio-sink`, `soundshare` · service | rare | No | **Keep**: phone-audio panels; prompts use the alert contract | View swap (prompts) | 2 |
| **Mobile pairing PIN popup**: grey centred panel "{device} is trying to connect." with the PIN digits and a 10-device note, Cancel, 30 s; plus "Currently connecting to the mobile device" toasts (LG ThinQ, TV Plus) | `RemoteUserList/PinPopup.qml` · system-qml · 182 files / 11 qml; id `com.webos.systemui.mobileconnection`; `WebOSCompositor/views/StarfishRemoteUserListView.qml` · compositor-qml · 17 lines | rare | No | **Replace**: registered in configd `systemUIManager`, so the redirect already reaches it | System UI redirect | 3* |
| **TV Plus guide and device row**: five-page "LG TV Plus Guide" and a device row with Disconnect, Screen Share and Content Share; nothing instantiates it | `RemoteUserList/RemoteUserListTooltip.qml` · system-qml · 1171 lines + Delegate, Model, Shadow, OpacityMaskImage, Style | never | No | **Keep (dormant)**: dead code in the module | — | — |

## 11. Ads, promos & tracking

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Home hero banner, sponsored shelves and usage logging**: hero banner, inline banners, "Sponsored" shelves from `admanager` slots, `NL_*` usage events; `homePromotion` is off but recommended shelves are still requested while terms allow | `com.webos.app.home` · flutter · 44.7 MB | never | Part | **Off**: the Home stub removes the Flutter process; Glasshouse blocks the hosts | Home stub | 1 |
| **Q-cards**: Home tiles with an ad-slot banner, content tabs, recently watched, Edit App List; mostly empty with LG hosts blocked | `com.webos.app.universalqcard` · web · 12.1 MB; `com.webos.app.universalqcard1` · web · 59 KB; `com.webos.app.universalqcard2` · web · 59 KB; `com.webos.app.universalqcard3` · web · 57 KB; `com.webos.app.learning` · web · 905 KB | never | No | **Off**: LumaHome has no Q-card tiles; the four copies share one bundle | Launcher filter | 7 |
| **New & Hot and Music card**: upcoming and popular titles with reminders and promo banners; mood playlists with a promo banner; the `universalqcard` service raises "Watch Now / Close" reminder alerts | `com.webos.app.newandhot` · web · 11.9 MB; `com.webos.app.totalmusic` · web · 9.3 MB; `universalqcard` · service | never | No | **Off**: cloud shelves needing an LG account; not offered in LumaTV | Launcher filter | 7 |
| **Premium**: loader for a legacy store page at `lgrecommends.lgappstv.com` | `com.webos.app.premium` · web · 791 KB | never | No | **Off**: legacy; only a MIME launch reaches it | Launcher filter | 7 |
| **LG Channels**: server-hosted free channel guide and VOD with ads; account required; the remote has an LG Channels key | `com.webos.app.lgchannels` · web · 383 KB | rare | No | **Off**: ad-funded; not offered in LumaTV (its preload goes in phase 8, Decided); the remote key still opens it unless remapped | Launcher filter | 7 |
| **LG Movies & TV**: LG rental and purchase store | `com.lgtvod.us.app` · web · 574 KB | never | No | **Off**: store promotion; not offered in LumaTV | Launcher filter | 7 |
| **Sports (My Team) and sports alarm**: league and team setup, schedules; popups "The {League} game starts soon" with Watch Live, scores, golf leaderboard, Open with | `com.webos.app.sportsteamsettings` · web · 29.2 MB; `SportsAlarm/SportsAlarm.qml` · system-qml · 245 files / 50 qml; id `com.webos.app.sportsalarm`; `sportsalarm` · service | never | Yes | **Off**: no team set (`mySportTeamList` empty); empty view through the redirect | System UI redirect | 7 |
| **Nudges**: bottom-left ribbon with up to 5 lines and an OK button, optional 150 px tiles: AI Recommendation promos, "You may like this content", Alexa, Bluetooth speaker, Magic Explorer, shortcut-key offers, optimise-for-game, AI Channel "Next Picks" panel, app-driven alarm banners; follow-up toasts | `Nudge/NudgeSystemUI.qml` · system-qml · 297 files / 51 qml; id `com.webos.app.nudge`; `nudge` · service | never | Yes | **Off**: `aiNudge` and `aiChannel` are off; the empty view and `nudge.service` off stop the rest | System UI redirect + Service off | 7 |
| **Live Plus (content recognition) overlays**: transparent "LivePlus" page in the input's ACR layer, loads a card from the ACR service; can shrink the video on close | `com.webos.app.acrcomponent` · web · 17 KB; `com.webos.app.acrhdmi1` · web · 17 KB; `com.webos.app.acrhdmi2` · web · 17 KB; `com.webos.app.acrhdmi3` · web · 17 KB; `com.webos.app.acrhdmi4` · web · 17 KB; `com.webos.app.acroverlay` · web · 17 KB | never | Yes | **Off**: `livePlus` off, ACR opt-out file present, `acr2` not running | LG setting | 7 |
| **Ad and shopping overlays**: transparent page in the ACR layer that redirects to an ad or shopping URL from `adoverlay-service`; the shopping overlay apps are not installed | `com.webos.app.adhdmi1` · web · 2 KB; `com.webos.app.adhdmi2` · web · 2 KB; `com.webos.app.adhdmi3` · web · 2 KB; `com.webos.app.adhdmi4` · web · 2 KB; `com.webos.app.adoverlay` · web · 2 KB; `com.webos.app.adoverlayex` · web · 2 KB | never | Yes | **Off**: `customizedAd` off, `doNotSellMyPersonalInformation` on, `getAppStatus` false | LG setting | 7 |
| **Ad banner card and ad manager**: transparent card with a "Sponsored ad" banner, Exit ad, More and a network popup; the manager also raises parental-control and "no longer supported" alerts | `com.webos.app.adapp` · web · 4.5 MB; `admanager` · service | never | Yes | **Off**: nothing is requested with the ad hosts blocked | LG setting | 7 |
| **Store promotion strip**: moving strip beside the picture with model name, "New", store logo and promo images; retail demo mode only | `StoreStreamer/StoreStreamer.qml` · system-qml · 49 files / 35 qml; id `com.webos.app.store-streamer` | never | Yes | **Off**: `storeMode` is home | System UI redirect | 7 |
| **Home Office and Remote PC**: RDP or VNC connection manager with a promo banner and website shortcuts; full-screen FreeRDP session | `com.webos.app.homeoffice` · web · 7.8 MB; `com.webos.app.rdp` · native · 45 KB | never | No | **Off**: not used; no LumaHome tile | Launcher filter | 7 |
| **LG Buddy and Copilot link**: family sharing over KakaoTalk (QR connect, photos, help requests); the Copilot quick link only opens the browser | `com.webos.app.buddy` · web · 8.5 MB; `com.webos.app.quicklinkcopilot` · flutter · 4.3 MB; `com.webos.service.buddyconnector` · service | never | No | **Off**: Korea-oriented; no LumaHome tile | Launcher filter | 7 |
| **Viewing-data messages**: "The Usage Data is deleted.", "Last month, you watched TV for an average of about {hour} hours per day ..." | `user-context-manager`, `user-intent-manager` · service | never | No | **Off**: `personalRecommend` is off | LG setting | 7 |

## 12. Accounts & first run

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **First-use wizard**: language, country, time zone, accessibility, remote or phone setup, network, terms, analysis, check, install type, AI, ambient, software update, parental PIN | `com.palm.app.firstuse` · web · 41.4 MB | never | No | **Keep (dormant)**: one-time after a factory reset | — | — |
| **Welcome after setup (OOBE)**: Amazon Alexa offer, Recommended Apps and Services, Useful Features (Simple Mode, Quick Settings, Help key, My Channel) | `com.webos.app.oobe` · web · 30.8 MB | never | No | **Keep (dormant)**: one-time after a factory reset | — | — |
| **Network and terms overlay**: Wi-Fi or Ethernet setup (`target` network) and User Agreements (`target` eula) over the current app; same bundle as First Use | `com.webos.app.firstuse-overlay` · web · 7 KB | rare | Part | **Restyle**: Enact web app; Settings › Network opens it | Overlay mount (theme) | 6 |
| **LG Account**: loader for the hosted LG Account page (sign in, sign up, billing); overlay variant with a local network-error page; "{ID} logged out", "log in again because account information has expired" | `com.webos.app.membership` · web · 5.0 MB; `com.webos.app.overlaymembership` · web · 123 KB; `accountmanager` · service | rare | Part | **Keep**: hosted page and account prompts; the Home chip reads `accountmanager` only | — | — |
| **Terms, privacy and country-change prompts**: "Our Privacy Policy has changed. The TV will now restart to show the updated User Agreements", ad-consent withdrawn notices, "Changing the country will cause your TV to restart" | `eula-service`, `sdx` · service | rare | No | **Replace**: alert contract; a restart prompt must stay answerable | View swap | 2 |
| **Server notice**: LG server notice with More and OK | `sdp-server-notice` · service | rare | No | **Replace**: alert contract | View swap | 2 |
| **Consent client**: blank page that loads an interactive consent URL given at launch | `com.webos.app.cmp-client` · web · 4 KB | never | No | **Keep (dormant)**: no launcher found; needs a URL from a service | — | — |

## 13. Service/factory

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Installer and hotel menu**: power-on state, default input, IP control and SDDP, limited mode, key management, cloning, volume, auto browser | `com.webos.app.installation` · web · 5.6 MB | never | No | **Keep (dormant)**: behind the IP Control PIN or hotel-mode key | — | — |
| **Factory window**: EZ-Adjust, In-Start, power-only, test patterns, board options, panel repair, OLED menus (299 qml) | `com.webos.app.factorywin` · native · 3.5 MB | never | No | **Keep (dormant)**: service IR keys only; not on the Magic Remote | — | — |
| **Service diagnostics**: HDMI error history, Wi-Fi and Bluetooth diagnostics, panel errors, power history, firmware versions | `com.webos.app.svcdiagnostics` · flutter · 6.3 MB | never | No | **Keep (dormant)**: opens only for country KOR from Quick Help | — | — |
| **Store demo**: promo video player, USB copy, passwords, sync of playlists between store TVs; store alerts "TV is changed to Store Mode" | `com.webos.app.store-demo` · web · 11.9 MB; `com.webos.app.sync-demo` · web · 4.7 MB | never | No | **Keep (dormant)**: `storeMode` is home | — | — |
| **USB and expert software update**: installed vs update version with Update and Cancel, progress "do not remove USB", reboot notice | `com.webos.app.softwareupdate` · web · 4.5 MB | never | No | **Keep (dormant)**: only with the expert or USB flag; needs a `mode` parameter | — | — |
| **Hotel (HCAP) view**: transparent layer for hotel client windows over input apps; hotel key policy | `WebOSCompositor/views/StarfishHCAPView.qml` · compositor-qml · 135 lines + hcap/StarfishHCAPSurface.qml | never | No | **Keep (dormant)**: `enableHotelMode` off | — | — |
| **Hidden key combinations**: gear three times in 3 s opens the AV-reset menu ("All video/audio settings will be reset?"), gear held 5 s, Yellow held 5 s (panel test pattern), Green seven times (FreeSync info), Blue seven times (panel usage dump) | `KeyFilters/appLaunch.js` · js · handleAvReset, handleHotelmode, handleFreeSyncInfo, handlePanelUTTUsbDownload, handleHotkeys; `com.webos.app.tvhotkey` · native · 5.3 MB | rare | No | **Off**: swallow the accident-prone ones in the key filter; AV reset stays under Settings | Key filter and launch redirect | 1* |
| **LG Remote Service session**: "The LG Remote Service has started.", "Real-time remote inspection is in progress.", "has ended. Your TV will now restart." | `remotediag` · service | never | No | **Replace**: alert contract; never filtered, it shows when LG support is connected | View swap | 2 |
| **System recovery messages**: "Update Failed", "System ran into a problem and needs to be repaired.", "The system is successfully recovered." | `data-migrator`, `db8-translator` · service | never | No | **Replace**: alert contract | View swap | 2 |
| **Rollable-TV stand errors**: "An error has occurred to the TV stand", "Press [OK] to switch the screen to Line View", partial-view volume placement | `rollingscreen` · service; `WebOSCompositor/views/PartialViewPositioner.qml` · compositor-qml · 75 lines | never | No | **Keep (dormant)**: fixed panel | — | — |
| **Developer overlays**: FPS box (needs a flag file), debug window, focus and surface consoles, resource monitor, log console | `WebOSCompositorBase/views/debug/DebugOverlay.qml` · compositor-qml · 10 files; WebOS DebugConsole, MemoryView, ResourceView; `WebOSCompositor/views/StarfishViewsRoot.qml` · compositor-qml · 252 lines (FpsCounter box) | never | No | **Keep (dormant)**: every switch defaults to off and the flag file is absent | — | — |
| **Secondary window**: a 768x2160 second Qt window on `fb1` for system-UI overlays; only one panel exists | `WebOSCompositor/StarfishSecondaryMain.qml` · compositor-qml · 94 lines | never | No | **Keep (dormant)**: the compositor opens only `card0` | — | — |

## 14. Accessibility

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Accessibility**: Vision (Audio Guidance, zoom, high contrast, invert, grayscale), Hearing (descriptions, amplify dialogue, hearing aid, sign language), Control (Learn Remote Control, shortcuts, Tips) | `com.webos.app.accessibility` · web · 34.4 MB | rare | Part | **Restyle**: Enact web app; the switches also appear as Settings › System rows | Overlay mount (theme) | 6 |
| **Accessibility quick panel**: compact shortcuts panel over the current app; opens when Mute is held 2 s or on the Multikey code | `com.webos.app.accessibility-overlay` · web · 4 KB | rare | No | **Keep**: a deliberate 2 s hold on Mute; the key filter must not swallow Mute | — | — |
| **Screen Zoom, grayscale and invert**: whole-UI shader: zoom 110-200 % with a pan minimap, key guide and four arrows, grayscale, inverted colours; Home held 2.5 s starts it | `WebOSCompositor/views/StarfishViewAdjustments.qml` · compositor-qml · 868 lines; `WebOSCompositorBase/zoom/ZoomView.qml` · compositor-qml · 3 files incl. ZoomKeyGuide.qml, ZoomDecorationStyle.qml | rare | Part | **Keep**: accessibility; PLAN's 650 ms Home hold reaches LG's Home first, so offer zoom as a Settings row | Key filter and launch redirect | 1* |
| **Learn Remote Control**: press a key on the real remote and its function appears; Back twice to leave | `com.webos.app.remotecontrolguide` · web · 7.4 MB | rare | No | **Restyle**: Enact web app | Overlay mount (theme) | 6 |
| **Sign Language Zoom**: a magnifier window over the chosen picture area and an "Select the area for sign language" frame; `isSLZoomOn` is off | `inputcommon/.../FgViewLayer/SLZoomView.qml` · qml · 230 lines + SLZoomSelectArea 450 | never | No | **Keep (dormant)**: switched off in Settings | — | — |
| **Sign-language avatar**: 3D avatar drawn over content in a top-UI window; menu language must be Korean; download prompts | `com.webos.app.signlanguage-avatar` · web · 7.8 MB; `signlanguage-avatar` · service; `WebOSCompositor/views/StarfishTopUIView.qml` · compositor-qml · 115 lines + topui/StarfishTopUISurface.qml | never | No | **Keep (dormant)**: Korean only and not in `listApps` | — | — |
| **AI Caption**: "Analyzing sound to generate AI Caption.", "AI Caption has ended.", "{mobile} is requesting to connect to the device." | `nais-offloading-manager` · service | never | No | **Replace**: alert contract | View swap | 2 |
| **Audio Guidance and language packs**: spoken focus and alert text through `tts`; prompts to download a high-quality language pack; `audioGuidance` is off | `tts` · service | never | Part | **Keep**: LG voice; the spoken hooks in alerts must survive the rebuild | — | — |
| **Captions and subtitle layers**: caption and subtitle layers under the transparent inputcommon window on Live TV and inputs | `inputcommon window-group layers` · qml · subtitle, ttxSubtitle, ttmlSubtitle | rare | Part | **Keep**: drawn by the media pipeline; the style is a Settings row | — | — |

## 15. Third-party/protected

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Netflix**: native client with its own UI and protected video | `netflix` · native · 85.5 MB | daily | Out | **Keep**: third-party UI and DRM video; only LumaTV's launch splash and spinner surround it | — | — |
| **YouTube**: Cobalt client; a homebrew build with the same id sits under `/media/developer`, `listApps` registers the native one | `youtube.leanback.v4` · native · 19.8 MB | daily | Out | **Keep**: third-party UI | — | — |
| **Prime Video**: native client (`ignition`) | `amazon` · native · 22.6 MB | sometimes | Out | **Keep**: third-party UI and DRM video | — | — |
| **Apple TV**: native client; an Apple TV box also sits behind the Denon on HDMI 2 | `com.apple.appletv` · native · 50.1 MB | rare | Out | **Keep**: third-party UI and DRM video | — | — |
| **Crunchyroll and GeForce NOW**: web clients | `com.crunchyroll.webapp` · web · 746 KB; `com.geforcenow.play` · web · 789 KB | rare | Out | **Keep**: third-party UI | — | — |
| **PlxNative**: unofficial native Plex client with its own renderer | `com.beb.plxnative` · native · 35.4 MB | sometimes | Out | **Keep**: homebrew UI, its own window | — | — |
| **Moonlight and Punktfunk**: game and desktop streaming clients | `com.limelight.webos` · native · 4.2 MB; `io.unom.punktfunk.client-webos` · native · 24.9 MB | rare | Out | **Keep**: homebrew UI | — | — |
| **Hue Sync**: Signify's sync app | `com.lighting.huesync` · web · 15.3 MB | rare | Out | **Keep**: third-party UI; LumaTV's Ambient light replaces its use | — | — |
| **Web Browser**: Chromium-based Crow UI: URL bar, tabs, bookmarks, start page with Most Visited and Recommended Sites; microphone allowed | `com.webos.app.browser` · native · 11.9 MB | rare | Part | **Keep**: web content in LG's browser; its 72 MB preload is dropped in phase 8 (Decided) | — | — |
| **Browser helpers**: floating voice-search browser (Naver, Korea), PWA shell that opens a URL, web-app host with a grey spinner and an error image, single-image viewer | `com.webos.app.beanbrowser` · web · 33 KB; `com.webos.app.tinybrowser` · web · 103 KB; `com.webos.app.webapphost` · web · 443 KB; `com.webos.app.tvsimpleviewer` · web · 85 KB | rare | No | **Keep**: content shells opened with a URL or a file; nothing of their own to restyle | — | — |
| **Store stubs**: Viewster, 8 Ball Pool, Twitch, DAZN, Highbrow, Tennis TV and Karaoke Smart folders; only the first two are in the snapshot or `listApps` | `com.lge.app.viewster` · web · 11 KB; `com.miniclip.eightballpool` · stub · live `listApps` only; `tv.twitch.tv.starshot.lg`, `dazn`, `com.joinhighbrow.highbrow`, `com.deltatre.tennistv.lgapp`, `karaokesmart` · stubs · folders under `/media/cryptofs` | never | No | **Keep (dormant)**: stubs without a UI of ours | — | — |
| **AmazOff**: Prime Video ad patcher; an nginx and a monitor run in the background without UI | `com.amazoff.patcher` · web · 7.9 MB | rare | No | **Keep**: homebrew; background processes only | — | — |
| **Homebrew Channel and helpers**: installer, its updater, the LG app-update blocker and the input-hook front | `org.webosbrew.hbchannel` · web · 2.5 MB; `org.webosbrew.safeupdate` · web · 664 KB; `org.webosbrew.appupdateblocker` · web · 128 KB; `org.webosbrew.inputhook` · web · 52 KB | rare | Part | **Keep**: root access and update blocking depend on them; the input-hook admin server is a PLAN "not taken" | Launcher filter | 1* |
| **64 to 32-bit bridge and HyperHDR**: EGL, GLES and Wayland bridge for 64-bit apps; HyperHDR loader | `org.webosbrew.bridge-64to32` · native · 36.6 MB; `org.webosbrew.hyperhdr.loader` · web · 4.2 MB | never | Part | **Keep**: support software; hidden from the launcher | Launcher filter | 1* |
| **LumaTV app and frame**: the LumaGlass app (renamed `org.nphil.lumatv` in phase 1) and its overlay pane for mini apps | `org.nphil.lumaglass` · web · 4.9 MB; `org.nphil.lumaglass.frame` · web · 340 KB | sometimes | Yes | **Keep**: ours | — | 1 |
| **Magic Remote pointer image**: native cursor (`pointerResourceStyle` LG); every surface has a hover state | native (compositor) | sometimes | Out | **Keep**: native, swap unverified; `general.remotePointer` turns it off | — | — |
| **Video planes**: apps, inputs and protected content on hardware planes the compositor places | hardware planes | daily | Out | **Keep**: protected video stays on LG's planes | — | — |

## Keys and what they open

The compositor's key chain (`KeyFilters/*.js`, 22 handlers from configd
`keyFilters`) decides which surface a key opens. LumaTV's `handleLumaHome` is
first in the chain. Power has no handler in the QML chain.

| Key | LG today | LumaTV |
|---|---|---|
| Home, release | `launchDefaultApp(home)`; from Home it closes the Home view | slot 1 launches LumaHome from any surface; replayed Home keys re-enter the chain, so the handler must be idempotent |
| Home, held 2.5 s | Screen Zoom (accessibility) | PLAN's 650 ms hold reaches LG's Home first; zoom moves to a Settings row |
| Back, release | closes the top overlay, alert, keyboard or spinner, else goes to the app; an app that ignores Back gets "Do you want to exit the app?" | untouched; claimed only while LumaHome is fullscreen and focused |
| Back, held 1 s (Exit) | closes the foreground app | untouched |
| Gear (Settings), short | `quicksettings` overlay, already LumaTV's by redirect | no key code needed |
| Gear, held 1 s | `com.palm.app.settings` (toggle) | launch redirect to LumaTV Settings |
| Gear, three times in 3 s | hidden AV-reset menu | Off: swallow |
| Input, short | `quickinputpicker` overlay | redirect through configd; no key code needed |
| Input, held 1 s | Home Hub (`homeconnect`) from `keyaction.inputHub` | repoint the config |
| 123 (Screen Remote), short | "More Actions" panel; held 1 s: audio description | redirect |
| Digits, held 1 s | Quick Access slot N; 0 opens the editor | LumaHome dock numbers while it is in front; digits untouched elsewhere |
| Mic | `voiceconductor` hold, then the Search app and the listening bar | untouched |
| Netflix, Prime Video | launch the app | untouched |
| Disney+, Sling | not installed: LG's auto-install path | remap or Off (Needs Nitin) |
| LG Channels, Movies, New | LG Channels app; Search with "movie"; Store category | remap or Off (Needs Nitin) |
| Guide, Channel list, Live TV | Guide overlay; channel list; Live TV | untouched (Live TV is rare) |
| Volume, Mute | `audioService`; the OSD is the compositor's | untouched; Mute held 2 s opens the accessibility panel |
| Colour keys | to the app; Yellow held 5 s shows a panel test pattern, Green seven times FreeSync info (HDMI), Blue seven times a panel-usage dump | Off for the held and repeated combinations |
| Menu, keyboard Windows | Home with `myapps` | route to the LumaHome apps grid |
| TV joystick press | Local Control Panel (`tvhotkey`) | untouched: the recovery path without a remote |
| Keyboard F2 F4 F5 F7, gamepad Menu, Settings and Xbox | Input, Home, Settings, LG Channels; Menu and Settings buttons; the Xbox button starts LG's auto-install of the (absent) Xbox app | untouched |

## No UI

Apps, files and services that draw nothing, or only host the surfaces in the
tables. Listed so that no app folder and no compositor view file is left out.

### Apps with no UI

- Developer samples for window groups and key masks, never launched on a
  retail set: `com.webos.exampleapp.enyoapp.epg`,
  `com.webos.exampleapp.groupowner`, `com.webos.exampleapp.nav`,
  `com.webos.exampleapp.qmlapp.client.negative.one`,
  `com.webos.exampleapp.qmlapp.client.negative.two`,
  `com.webos.exampleapp.qmlapp.client.positive.one`,
  `com.webos.exampleapp.qmlapp.client.positive.two`,
  `com.webos.exampleapp.qmlapp.discover`, `com.webos.exampleapp.qmlapp.epg`,
  `com.webos.exampleapp.qmlapp.hbbtv`, `com.webos.exampleapp.qmlapp.livetv`,
  `com.webos.exampleapp.qmlapp.search`, `com.webos.exampleapp.systemui` (a red
  rectangle fading every 5 s).
- Blank layer hosts that a broadcaster page would fill, dormant here (HbbTV
  off, US ATSC): `com.webos.app.livehbbtv`,
  `com.webos.app.liveinteractivecontent`, `com.webos.app.liveginga`.
- Transparent launchers that call a service and close:
  `com.webos.service.billing.app`, `com.webos.service.homeconnect.app`.

### Compositor files that host surfaces or help, with no pixels of their own

| File | Role |
|---|---|
| `views/StarfishViewsRoot.qml` | instantiates the 26 views; paint order is declaration order |
| `views/StarfishOverlayView.qml` | one overlay at a time: Settings, Guide, quick panel, input picker, Search |
| `views/StarfishOverlayGroupView.qml` | overlay that owns a window group; no app uses it here |
| `views/StarfishPopupView.qml`, `popup/StarfishPopupSurface.qml`, `popup/popuphandler.js` | movable popup windows (DVR popup, TV hotkey panel) |
| `views/StarfishScreenRemoteView.qml` | hosts `screenremote`, `contextmenu`, `toss` |
| `views/StarfishNudgeView.qml` | hosts nudge windows; blocked over Home, alerts, overlays, keyboard |
| `views/StarfishNotificationView.qml` | loads `SystemUI/Notification`; focus layers 7 and 8 |
| `views/StarfishScreenCoverView.qml` | hosts `bootlogo`, `masterbrand`, `woweffect` |
| `views/systemUi/StarfishSystemUIManager.qml`, `StarfishSystemUIContainer.qml`, `StarfishSystemUIComponentContainer.qml`, `StarfishSystemUIView.qml`, `StarfishSystemUIViewContainer.qml`, `StarfishSystemUIViewStateTransition.qml`, `StarfishSystemUIViewStateTransitionContainer.qml`, `StarfishSystemUIOutProcView.qml`, `StarfishSystemUIOutProcViewPlaceHolder.qml`, `StarfishSurfaceContainer.qml`, `StarfishViewContainer.qml`, `StarfishFocusableView.qml`, `StarfishOverlaySurface.qml`, `StarfishHomeSurface.qml` | load the system-UI QML named in configd `systemUIManager`; a failed load shows nothing, the out-of-process placeholder prints "Can not load {appId}" |
| `views/base/StarfishImage.qml`, `views/base/imageutils.js`, `views/containerController.js`, `views/punchThrough.js`, `views/fullscreen/fullscreencontainerhandler.js` | scaling, video punch-through and container helpers |
| `WebOSCompositorBase/views/ScreenFreezer.qml` | holds a snapshot of the UI for up to 2 s while the output geometry changes |
| `WebOSCompositorBase/views/Launcher.qml`, `NotificationView.qml`, `OverlayView.qml`, `base/BaseView.qml`, `base/FocusableView.qml`, `base/SurfaceView.qml`, `base/PopupSurface.qml`, `base/FocusHandler.js`, `base/popuphandler.js` | reference-compositor classes that `StarfishViewsRoot` overrides or never instantiates |
| `WebOSCompositorBase/views/notification/` `Alert.qml`, `AlertView.qml`, `Button.qml`, `CloseButton.qml`, `MarqueeText.qml`, `Notification.qml`, `NotificationModel.qml`, `NotificationService.qml`, `NotificationStack.qml`, `Popup.qml`, `PincodePrompt.qml`, `PincodePromptView.qml`, `PincodePromptBaseButton.qml`, `PincodePromptNumButton.qml`, `PincodePromptPasswordButton.qml`, `Toast.qml`, `ToastView.qml` | the reference notification set; the shipping one is `SystemUI/Notification` |
| `WebOSCompositorBase/views/debug/` `DebugOverlay.qml`, `DebugWindow.qml`, `FocusConsole.qml`, `FocusHighlight.qml`, `FpsGraphOverlay.qml`, `LogConsole.qml`, `ResourceMonitor.qml`, `SurfaceConsole.qml`, `TouchOverlay.qml`, `exportedItemDebug.qml` | developer overlays behind `debug.enable` (false) |

### QML modules and plumbing with no surface of their own

- `SystemUI/Notification`: `Services.qml`, `NotificationModel.qml`,
  `NotificationView.qml`, `NotificationStyle.qml`, `Notification.qml`,
  `NotificationStack.qml`, `FocusableView.qml`, `SystemUIView.qml`,
  `NLocaleService`, `Utils.js`; `Popup.qml` is the base class that owns the
  scrim, not a luna type; `Button.qml` and `PincodePromptBackButton.qml` are
  never instantiated; `MarqueeText.qml`, `ScrollText.qml`, `TextScroller.qml`,
  `ElideShader.qml` scroll long labels.
- Adaptation, interface and controller layers inside `QuickSettings`,
  `InputPicker`, `ContextMenu`, `ScreenRemote`, `BootLogo`, `MasterBrand`,
  `CoverUI`, `Nudge`, `SportsAlarm`, `StoreStreamer`, `FavoriteChannels`,
  `RemoteUserList` and `inputcommon` (113 interface files, handlers, layer
  containers, key routers, 33 shared widgets); five unreferenced Quick Settings
  components and unreferenced SportsAlarm and RemoteUserList helpers.
- Libraries: `Eos` (Moonstone controls, items, styles, three native plugins),
  `QmlAppComponents`, `QmlAppComponentsStarfish`, `WebOS` (developer
  consoles), `WebOSServices` (models), `iLib`, `Qt*`, `QtGraphicalEffects`,
  `QtMultimedia`, `AudioService`, `FpsCounter`, `KeyFilters` (JS),
  `PerformanceLog`, `PmLog`, `PmTrace`, `RdxLog`, `TextFile`, `Voice` (Opus
  player), `SystemUIComponent`, `SystemUIAssets` (volume and list icons).
- configd `systemUIManager` entries whose module is absent from the set:
  `com.webos.app.woweffect` (`ScreenCover`) and `com.webos.app.toss`
  (`Toss`); `com.webos.app.sportsalarm` and `com.webos.app.homegymnudge` have
  no `main`.
- Services that show nothing: `rdxd`, `uploadd`, `contentminer`,
  `remotelogger`, `sleepd`, `avmutemanager`, `lowlevelstorage`,
  `systemprofile`, `sysfaultdetector`, `activitymanager`, `logoutd`,
  `audiooutputd`, `devicereset`, `color-info-miner`, `ledbarcontroller`,
  `snapshot-boot-manager`, `browser-service` and the camera and microphone
  services; `flutter-wayland-client` only holds screen-reader labels.

## Totals

178 surfaces in 15 groups, one row each; a row may list several ids (160 app ids, 113 compositor view files and the system-UI modules are all covered). Rows by answer and how often they are seen:

| Answer | daily | sometimes | rare | never | Total |
|---|---|---|---|---|---|
| Replace | 6 | 16 | 35 | 5 | 62 |
| Restyle | 0 | 6 | 19 | 0 | 25 |
| Off | 0 | 0 | 6 | 18 | 24 |
| Keep | 5 | 4 | 25 | 33 | 67 (31 dormant) |
| **Total** | **11** | **26** | **85** | **56** | **178** |

Plan coverage today: Yes 44, Part 37, Out 10, No 87. Of the 37 daily and sometimes surfaces, 19 have a mock.

## Not yet mocked

Daily or sometimes surfaces with no screen in `mocks/`, most seen first (8; Home is built and is the visual reference):

1. **HDMI input banner** (sometimes): the pill on every input switch, plus OK and INFO banners; mock 20 is the channel banner.
2. **Power-off logo** (daily): every power-off, about 3 s; mock 23 is the boot screen.
3. **Always Ready and Gallery+** (daily): the standby screen, if Always Ready is on (unverified).
4. **Voice listening bar** (sometimes): mic key held: "Speak now.", transcript, reply.
5. **Quick panel popups and edit mode** (sometimes): slider, value picker, Move or Delete, add-tile list.
6. **Settings sections without a mock** (sometimes): Inputs & devices, Network, Ambient light, Home & look, System.
7. **Startup cover** (sometimes): first-app cover with "Loading..." at boot (and wake, unverified).
8. **Quick Access (number keys)** (sometimes): toast on launch and the "add to slot N?" alert.

Not enough daily or sometimes surfaces remain to make fifteen; the next seven are rare, ordered by what each protects:

9. **Second-screen pairing prompt**: security prompt; Home Assistant pairs through it.
10. **On-TV button panel**: recovery path without a remote.
11. **Pointer right-click menu**: Back and Home beside the pointer; mock 16 is the Home tile menu.
12. **"More Actions" panel (on-screen remote)**: Quit App, number pad, colour keys.
13. **Game Dashboard**: HDMI game mode replaces the quick panel.
14. **App launch prompts**: update-required and not-available alerts when an app starts.
15. **Toast: advanced (icon and link)**: icon toast whose click runs an action.

## Facts the replacements must keep

Behaviour of LG's code that the rows above depend on. Evidence per row is in
`docs/research/surfaces/*.json`.

Notifications (phase 2):

- Every service message funnels into `com.webos.notification`, served by
  `notificationmgr`: four subscriptions (`getToastNotification`,
  `getAlertNotification`, `getInputAlertNotification`,
  `getPincodePromptNotification`). Queueing, default timeouts and PIN
  validation live in that daemon, so a view swap keeps it.
- An alert button is a luna call made as the source app
  (`callForApplication(sourceId, uri, method, params)`), not as
  surfacemanager. Toast and input-alert actions and `onfail` and `onclose` are
  plain calls. `createAlert` returns an `alertId`; callers close with
  `closeAlert` and move a progress bar with `setAlertProgress`.
- PIN digits go back through `closePincodePrompt {closeType: relay, pincode,
  mode, appId}`; Back at zero digits, or the third wrong code, sends `close`.
  A PIN prompt closes a non-modal alert and the reverse; a modal alert closes
  input alerts.
- Items are dropped while the foreground window's access policy forbids them,
  while the set powers off and while the OSD is off; a dropped alert fires its
  `onfail` call.
- Timings: a toast stays 5 s (1-10 s), one is visible at a time, a queued one
  replaces it after 2 s, hard cap 15 s. Z-order: PIN 10001, alert 10002,
  input-alert scrim 10003, input alert 10004, voice agent 10005, toast 10006.
- Alert text is spoken through `tts` when Audio Guidance is on, and buttons
  register with `voiceconductor` (`ALERT_SELECT_n`, `PINCODE_SELECT_n`).

Keys (phase 1):

- Slot 5 of the chain (`handleScreenSaver`) restarts the compositor's 120 s
  activity timer (`userActivityTimeout`) on every key. When it expires with
  the pointer hidden, the compositor closes overlays, alerts, the PIN prompt,
  the keyboard, the spinner, the splash and the Home-type view; it does not
  start the screensaver, `tvpowerd` does. A key swallowed in slot 1 never
  restarts it, so LumaTV calls the exposed `restartUserActivityTimer`
  (service app id `LS.appId + '.screenSaver'`) for the keys it takes, or LG's
  pending alerts and overlays close after two minutes. Whether `tvpowerd`
  counts keys the compositor swallows is unverified [INFERENCE].
- Slot 4 (`handleLongPressKeys`) swallows the press of Back, Gear, Input,
  Guide, 123 and Red and delivers it on release; it also shows and hides the
  Magic Remote pointer for them. A slot-1 handler that takes such a key calls
  `setMRCUCursorVisibility` itself. Replayed keys re-enter at slot 1, so the
  handler must be idempotent, and its timers must not reuse LG's shared
  `longPressTimer`.
- Writing configd `keyFilters` unsubscribes every filter, reruns each file's
  `init()` and reopens Quick Settings in Game mode.

Power and boot (phases 1 and 5):

- `tvpowerd` waits for the compositor to answer `responsePreparePowerOff`,
  `responsePreparePowerOn` and `responsePowerOffLogoRequest`; a replacement
  power-off logo reports done or power-off stalls.
- CoverUI is part of boot: `bootd` opens it through `coverUI/open`, it
  launches the first app and emits `smart-service-ready` and
  `firstapp-launched`.
- After boot, `preloadSystemUI()` launches Quick Settings and the input picker
  with `reason: preload`. Their replacements handle `{params, reason:
  'preload'}`, `params.activateType` and the `localkey` flag the one-key TV
  button sends.

Inputs and Home:

- `com.webos.app.inputcommon` is one binary behind Live TV and every input
  app, preloaded for `livetv` and `hdmi2`. Its launch parameters
  (`showBanner`, `showBannerWithEdit`, `toggleInfo`, `sendKeyPress`,
  `uniqueId`) are how services reach it. Overlays sit in window-group layers
  (ACR -9, captions -19 to -21).
- The picker lists only inputs that `eim/getAllInputStatus` reports connected
  with `hdmiPlugIn`; the set has four HDMI ports and no other input.
- Recents are drawn by Home from `recent/listRecents`. Closing the last app
  relaunches the latest recent, then `lastInputApp`, then `profile.idleApp`
  (Home).
- Today LG's Flutter Home still runs behind LumaHome; once the Home stub
  replaces it (phase 1) the hero banner, sponsored shelves and `NL_*` usage
  events stop at the source.
- On this set the volume OSD is the ARC form (speaker, + and -, no number)
  because `external_arc` reports `volumeSyncable` false.

## Missing from PLAN.md

Surfaces that PLAN does not cover, or covers with a narrower scope than the LG
code has:

1. **HDMI input banner**: the pill shown on every input switch, with the OK
   and INFO banners (resolution, HDR, audio codec) and the update-progress
   badge. PLAN restyles "the channel banner only" (Decided), but this banner
   and the No Signal screen are what HDMI 2 use shows.
2. **Voice listening bar** (`VoiceAgent`, 1954 files of animation) and the
   microphone indicator: separate from the Search app, drawn while the mic key
   is held.
3. **Startup cover and boot popup**: `bootd` opens CoverUI, which launches the
   first app, so stubbing it can stall boot; the boot popup answers hotkeys
   pressed before boot finishes.
4. **Pairing prompts**: the modal "Connection Request" alert through which
   Home Assistant (`webostv`) and phones pair, and the mobile PIN popup
   (`com.webos.systemui.mobileconnection`, already reachable by the
   redirect). The alert must survive the notification rebuild and stay
   answerable only with a physical remote key; PLAN's AI prompt is a
   different surface.
5. **Pointer right-click menu**: LG's `contextmenu` has two buttons, Back and
   Home. PLAN's "Context menu" row and mock 16 describe LumaHome's tile
   options, a different surface.
6. **Standby screen**: `lifeOnScreenMode` is `alwaysReady` (Gallery+ art and
   clock) and `alwaysready` raises its own countdown alert; PLAN covers only
   the idle screensaver.
7. **App launch prompts** from `sam` ("A new version of the app is
   available", "You must update this app to launch it"): probable with LG app
   updates blocked [INFERENCE].
8. **Remote shortcut keys**: Disney+ and Sling are not installed, so LG's
   auto-install path runs; Movies opens Search with "movie", New the Store,
   LG Channels its app.
9. **Hidden key combinations and Home hold**: gear three times (AV reset),
   Yellow held 5 s, Green or Blue seven times, and Home held 2.5 s (Screen
   Zoom), which PLAN's 650 ms hold pre-empts.
10. **One-key TV button path**: Quick Settings and the input picker take the
    `localkey` launch flag; their LumaTV versions keep it, since it operates
    the set without a remote.
11. **Messages that must never be filtered**: US EAS (antenna only), thermal
    shutdown, Magic Remote battery, LG Remote Service session, app permission
    and pairing prompts.
12. **Game Dashboard**: LG's quick panel switches to it in HDMI game mode;
    PLAN has the idea of a game card, no mock and no rule for game mode.
13. **Setting-change confirmations** from `pqcontroller` and `ocp` (energy
    notices, "keep the changed screen settings?", Filmmaker suggestions) fire
    when a picture mode changes; PLAN does not say which to drop.

Corrections to the Surfaces table: recents are drawn by LG's Home, not the
compositor; Home is listed as "Phase 8" while Phases and Glasshouse say 1.

## Needs Nitin

1. **Standby look.** `lifeOnScreenMode` is `alwaysReady` today. Keep LG's art
   and clock when the set is off, replace it with LumaTV's screensaver or
   clock, or show nothing (OLED care).
2. **Voice.** Is the mic key used? If yes, should the result screen keep LG's
   cloud answers and content search, or LumaTV's local search only (apps,
   settings, web)?
3. **Game.** Does a console or PC run on HDMI? If not, the game card and the
   Game Optimizer row are not worth building.
4. **Multi View.** Used? If not, its frames and picker stay LG's and leave
   the plan.
5. **Remote shortcut keys.** Remap Disney+, Sling, Movies, New and LG
   Channels to LumaTV actions, or leave LG's behaviour (Disney+ and Sling
   start an app install, the others open LG's apps).
6. **Home hold.** PLAN's 650 ms hold is LG's Home escape; LG's Screen Zoom
   (2.5 s) becomes unreachable. Accept, or move one of them.
7. **Notification drawer and Apps grid.** The Magic Remote has no key for
   either. Open them from LumaHome only, or from the Menu key?
