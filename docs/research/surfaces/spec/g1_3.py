from core import *

# ---------------------------------------------------------------- 1 Home & launching
g = 1
row(g, 'Home', 'shelves, hero banner, Q-cards, app row, recents, edit mode; hero is off here (`homePromotion` off)',
    'com.webos.app.home', 'daily', 'Yes', 'Replace',
    "LumaHome draws it; LG's Flutter Home stays resident behind it as the fallback", 'Home stub', 1)
row(g, 'Apps tile (LG Content Store)', 'server-rendered store page: featured, categories, search, install',
    'com.webos.app.discovery', 'sometimes', 'Yes', 'Replace',
    "own all-apps grid with search and favourites (mock 12); LG's store stays reachable for installs", 'Key filter and launch redirect', 7)
row(g, 'Launch splash', 'full-screen app image from `splashBackground` (47 of 149 apps, incl. Netflix, YouTube, Prime, Plex), else grey default with 8 dots; skipped for Home and `noSplashOnLaunch`',
    F('WebOSCompositor/views/StarfishImageSplash.qml', 'compositor-qml', '333 lines'), 'daily', 'Yes · mock 15', 'Replace',
    'one LumaTV launch screen; LG art only where an app ships its own', 'Shadow module', 5)
row(g, 'Launch spinner and infinite spinner', '72 px grey disc with a GIF, input-blocking, 500 ms delay, 120 s cap; only for overlay, popup and floating apps and when no splash shows; the owner-based infinite spinner has no caller here',
    F('WebOSCompositor/views/StarfishSpinnerView.qml', 'compositor-qml', '153 lines (2 instances)'), 'sometimes', 'Yes · mock 15', 'Replace',
    'one shared LumaTV spinner; the `showSpinner` luna calls stay the trigger', 'Shadow module', 5)
row(g, 'Long-press progress ring', '72 px disc whose arc fills while Home, Mute, digit or AV-reset keys are held; only with Accessibility slow remote input',
    F('WebOSCompositor/views/StarfishLongPressSpinnerView.qml', 'compositor-qml', '154 lines'), 'never', 'Yes · mock 15', 'Replace',
    'cheap with the spinners; `remoteSlowInput` is off, so it never shows today', 'Shadow module', 5)
row(g, 'Recents (app switcher)', 'row of recent app cards, drawn by Home from `recent/listRecents`; the compositor only keeps `recents.db` and a screenshot every 120 s',
    [F('WebOSCompositor/controllers/fullscreen/StarfishRecents.qml', 'compositor-qml', 'with StarfishRecentsDb.qml, StarfishFullscreenCapture.qml, models/StarfishRecentsWindowModel.qml')],
    'sometimes', 'Yes · mock 14', 'Replace',
    'lives in LumaHome (close with `closeByAppId` before `removeRecent`); PLAN files it as a compositor restyle, but LG draws it in Home', 'Home stub', 1)
row(g, 'Multi View window frames', 'boundary line, option panel (more, mute, switch sound, mixing, quit, volume), PiP drag handle, standby tile for empty slots; hides after 10 s',
    F('WebOSCompositor/views/decoration/StarfishServerDecoration.qml', 'compositor-qml', '1038 lines + StarfishDecorationButton, StarfishDecorationButtonLayer, StarfishDecorationExtendedButtonLayer, StarfishDecorationStyle, StarfishServerDecorationService'),
    'rare', 'Yes', 'Restyle', "Multi View only; do last, and drop if Nitin never uses it", 'Shadow module', '5*')
row(g, 'Multi View picker', 'Side by Side, PiP, Dual PC Monitor, 3-4 screens, recent and recommended combinations, options bar',
    'com.webos.app.multiviewsettings', 'rare', 'Part', 'Restyle', 'web overlay; takes the shared web theme with the Settings sub-apps', 'Overlay mount (theme)', 6)
row(g, 'Floating window host and FTMS info', 'one free window above fullscreen, used only by the fitness-machine panel; the AirPlay toolbar (Move, Sound, Size, Full Screen, End) is dead because AirPlay runs as a card',
    [F('WebOSCompositor/views/StarfishFloatingView.qml', 'compositor-qml', '146 lines + floating/StarfishFloatingSurface.qml, Toolbar.qml, ToolbarButton.qml, ToolbarStyle.qml, ToolbarAirPlayService.qml, ToolbarTtsService.qml'), 'com.webos.app.ftmsinfo'],
    'never', 'No', 'Keep (dormant)', 'no Bluetooth fitness gear here and the toolbar cannot trigger', '—', '—')
row(g, 'Fullscreen app host', 'edge-to-edge window for every card app; draws only black backing, the Multi View standby tile and the "press and hold back" control-mode guide',
    [F('WebOSCompositor/views/StarfishFullscreenView.qml', 'compositor-qml', '1262 lines'), F('WebOSCompositor/views/fullscreen/StarfishFullscreenContainer.qml', 'compositor-qml', '1405 lines'), F('WebOSCompositor/views/fullscreen/StarfishFullscreenStandby.qml', 'compositor-qml', '29 lines')],
    'daily', 'Part', 'Keep', 'window stacking, video holes and key routing stay LG\'s (PLAN: compositor not replaced); nothing visible but black', '—', '—')
row(g, 'Pointer right-click menu', 'two round buttons, Back and Home, beside the pointer on a 30 % scrim; replays the key to the app below; not the hold-OK tile menu',
    [F('ContextMenu/ContextMenu.qml', 'system-qml', '36 files / 14 qml; id `com.webos.app.contextmenu`')],
    'sometimes', 'Yes · no mock (mock 16 is the Home tile menu)', 'Replace', 'rebuilt resident; keep the two verbs and the key replay', 'System UI redirect', 5)
row(g, '"More Actions" panel (on-screen remote)', 'side panel: Quit App, User Guide, Info, Quick Help for apps; number pad, colour keys, Guide, Record, Scheduler on TV; trick-play and SIMPLINK keys for CEC devices',
    [F('ScreenRemote/ScreenRemote.qml', 'system-qml', '133 files / 57 qml; id `com.webos.app.screenremote`')],
    'rare', 'Yes · no mock', 'Replace', 'rebuilt resident; Quit App is the one item an app user needs', 'System UI redirect', 5)
row(g, 'Quick Access (number keys)', 'digit held 1 s starts slot N with a toast, empty slot asks "add <app> to Quick Access N?"; 0 held opens the editor',
    ['com.webos.app.magicnum', F('WebOSCompositor/controllers/base/StarfishMagicNumber.qml', 'compositor-qml', '489 lines')],
    'sometimes', 'Part', 'Replace', "LumaHome's dock numbers take over (mock 03); LG's digit handling stays untouched outside LumaHome", 'Key filter and launch redirect', '1*')

# ---------------------------------------------------------------- 2 Inputs & devices
g = 2
row(g, 'Input picker', 'bottom ribbon: Home Hub tile first, then one tile per connected input with icon, CEC name (AVR-S760H) and tick on the current one; add-on buttons (Eject, Universal Control, Channel Tuning); repeated Input presses step and a 5 s idle launches',
    [F('InputPicker/InputPicker.qml', 'system-qml', '89 files / 51 qml; id `com.webos.app.quickinputpicker`')],
    'sometimes', 'Yes · mock 08', 'Replace', 'rebuilt resident, own inputs only (no Home Hub tile)', 'System UI redirect', 4)
row(g, 'HDMI input banner', 'dark pill, lower left, 5 s: input name and CEC label; OK adds an Options button; INFO opens a detail banner (resolution, HDR, audio codec, Dolby Vision or Atmos, clock)',
    ['com.webos.app.inputcommon', F('inputcommon/UserInterfaceLayer/Containers/MainViewLayer/InputBanner/*', 'qml', 'SimpleBanner, InputBanner, DetailInfoBanner, MiniBanner (JP only)')],
    'sometimes', 'Part · no mock (mock 20 is the channel banner)', 'Restyle', 'PLAN names the Live TV banner only; the HDMI one is what Nitin sees most', 'Overlay mount', 6)
row(g, 'No-signal screen', 'black with a moving "No Signal" box and two hints, or 16 art photos every 10 s (`noSignalScreenSaver` is on); the box relocates every 10 s against burn-in; silent power-off after 860 s',
    [F('inputcommon/UserInterfaceLayer/Containers/BgViewLayer/Screensaver.qml', 'qml', '849 lines + ScreensaverMainText, ScreensaverSubText, ScreensaverKeyHelp, PhotoSlide, RandomMoveItem')],
    'sometimes', 'Yes · mock 18', 'Restyle', 'keep the burn-in mover and the 860 s auto-off; LumaTV look', 'Overlay mount', 6)
row(g, 'Set-top and lock states on an input', '"Do you want to turn on your set-top box?" No/Yes (HDMI input set up as a box); "Input Locked" with OK to open the PIN prompt',
    [F('inputcommon/Interfaces/ScreensaverInterfaces/NoSignalAlertController.qml', 'qml', '254 lines + PincodePromptController 56'), F('inputcommon/UserInterfaceLayer/Containers/PopupLayer/AlertPopupView.qml', 'qml', '432 lines')],
    'rare', 'Part', 'Restyle', 'inputcommon popups share the Overlay mount', 'Overlay mount', 6)
row(g, 'HDMI input apps 1-4', 'no UI of their own: a transparent inputcommon window over the video plane; a cold launch shows `HDMI-N_splash.png`; HDMI 2 (Denon AVR, `lastInputApp`) is the daily one',
    ['com.webos.app.hdmi1', 'com.webos.app.hdmi2', 'com.webos.app.hdmi3', 'com.webos.app.hdmi4'], 'daily', 'Out', 'Keep',
    'window host over protected video; its banners and no-signal rows are above', '—', '—')
row(g, 'Inputs the C4 does not have', 'DisplayPort, USB-C, AV, Component and SCART placeholders: same inputcommon window, never selectable',
    ['com.webos.app.dp1', 'com.webos.app.dp2', 'com.webos.app.usbc1', 'com.webos.app.usbc2', 'com.webos.app.externalinput.av1', 'com.webos.app.externalinput.av2', 'com.webos.app.externalinput.component', 'com.webos.app.externalinput.scart'],
    'never', 'No', 'Keep (dormant)', '`eim` reports 4 HDMI ports and none of these (`maxDisplayportCount` 0, `maxUsbcCount` 0)', '—', '—')
row(g, 'HDMI or CEC device connected', 'input alert: white parallelogram cards with port icon and name ("Do you want to switch to the screen of the connected device?" / "[AVR-S760H] is connected.")',
    [S('eim')], 'sometimes', 'Yes · mock 18', 'Replace', 'same item contract as toasts and alerts', 'View swap', 2)
row(g, 'HDMI format prompts', '"the Ultra HD Deep Color of the current HDMI port will be changed to 8K" with Go to Settings; "Audio is not supported"',
    [S('extinput-integration')], 'rare', 'Part', 'Replace', 'plain alert contract', 'View swap', 2)
row(g, 'Universal Control (IR blaster, set-top box)', 'wizard for the TV remote to drive a box (input, brand, model, key tests); prompts "set the current input as the detected device?"; Universal Control cards in the on-screen remote',
    ['com.webos.app.connectionwizard', S('irdbmanager')], 'rare', 'Part', 'Restyle', 'web app themed with the Settings sub-apps; alerts by view swap', 'Overlay mount (theme)', 6)
row(g, 'USB and storage prompts', '"USB device is connected.", "Do you want to browse the file from the connected device?", format prompts, eject warnings, wired headphones toast',
    [S('physical-device-manager2','asm','audiod')], 'rare', 'Yes', 'Replace', 'toasts and alerts through the one renderer', 'View swap', 2)
row(g, 'Media Player', 'file browser and player for USB, DLNA and cloud photos, video and music; subtitles, 360 VR, lyrics',
    'com.webos.app.mediadiscovery', 'rare', 'No', 'Keep', "Plex covers media; LG's logic is the screen; theme only if it is an Enact app (unchecked)", '—', '—')
row(g, 'Camera', 'live preview, camera list, screen position, apps using the camera', ['com.webos.app.camera', S('starfish-camera-plugins')],
    'never', 'No', 'Keep (dormant)', 'no camera on the set; its prompts use the alert contract', '—', '—')
row(g, 'Bluetooth keyboard, mouse and gamepad pairing', '"[Device] wants to connect to your TV. Do you want to connect it?"; "Enter the confirmation code {passKey} on the keyboard"',
    [S('bthidmanager')], 'rare', 'Part · mock 22', 'Replace', 'alert contract; Settings › Inputs & devices lists the devices', 'View swap', 2)
row(g, 'Magic Remote status', '"Magic Remote battery is low", "signal is unstable", "Press OK(Wheel) to pair"',
    [S('lginput2')], 'rare', 'No', 'Replace', 'toasts and alerts; battery warning must never be filtered', 'View swap', 2)
row(g, 'On-TV button panel', 'the TV\'s one-key joystick: press opens a small Local Control Panel; Quick Settings and the input picker show "Press: OK, hold: Power Off"; first power-on by the button shows a key guide for 5 s',
    ['com.webos.app.tvhotkey', F('inputcommon/.../LocalKeyView.qml, LocalKeyPowerOnOSD.qml; QuickSettings and InputPicker LocalKeyGuidance.qml', 'qml', 'local-key variants')],
    'rare', 'No', 'Keep', "the recovery path when the remote is lost; LumaTV's panels must still accept the `localkey` launch", '—', '—')
row(g, 'Wi-Fi Direct, cable and Bluetooth turn-on toasts', '"Wired LAN cable disconnected.", "[%s] wants to connect via Wi-Fi Direct. Accept?", "{Mobile} is ready to connect."',
    [S('event-monitor-network-starfish','bluetooth-asc-service')], 'rare', 'No', 'Replace', 'alert contract', 'View swap', 2)

# ---------------------------------------------------------------- 3 Volume & sound
g = 3
row(g, 'Volume OSD', 'right-edge for 1.5 s; on this set (`external_arc`, `volumeSyncable` false) a speaker with + and - and a highlighted circle, no number or bar; TV speaker, headphones, Bluetooth, WiSA and WOWCAST show icon, number and bar',
    [F('WebOSCompositor/views/StarfishVolumeView.qml', 'compositor-qml', '240 lines'), F('WebOSCompositor/views/volume/StarfishVolume.qml', 'compositor-qml', '484 lines')],
    'daily', 'Yes · mock 11', 'Replace', "level comes from the Denon (`MV` events), not webOS's drifting counter", 'Shadow module', 4)
row(g, 'Mute bubble', '96 px bubble at the right edge while muted (kept for `external_arc`), hops 94 px every 10 s against burn-in; Mute held 2 s opens the accessibility panel instead',
    F('WebOSCompositor/views/volume/StarfishVolumeMute.qml', 'compositor-qml', '130 lines'), 'sometimes', 'Yes · mock 11', 'Replace',
    'Denon `MU` events drive it; keep the burn-in hop', 'Shadow module', 4)
row(g, 'ARC, soundbar and wireless-speaker suggestions', '"change Digital Sound Output to Pass Through to minimize audio delay while playing game?", "LG sound bar is now connected", WOWCAST and WiSA reconnect messages',
    [S('arccontroller', 'wowplay', 'wisa-manager', 'wireless-av-controller')], 'rare', 'No', 'Replace', 'alert contract; the LG-soundbar texts cannot appear with the Denon', 'View swap', 2)
row(g, 'Bluetooth headphone or speaker connection', '"[Device] is connected.", "Failed to connect to [Device]", "reconnect the previously connected [Device]?"; Sound Out falls back to the TV speaker when it drops',
    [S('bluetooth-audio-source', 'bluetooth-le-audio')], 'rare', 'No', 'Replace', 'toasts and alerts through the one renderer', 'View swap', 2)
row(g, 'Auracast broadcast settings', 'Start or Stop Broadcasting, TV name, authentication code, audio quality, registered devices, sound test',
    'com.webos.app.leaudio', 'never', 'No', 'Keep (dormant)', 'sound goes to the Denon over eARC, which blocks Auracast', '—', '—')
row(g, 'Personalized Sound Wizard and AI Acoustic Tuning', 'pick audio samples for vocal tone, bass-treble balance and spatial sound, then Before/After and Apply; the tuning app plays an 8 s test heard by the Magic Remote microphone',
    ['com.webos.app.soundwizard', 'com.webos.app.onetouchsoundtuning'], 'rare', 'Part', 'Restyle', 'Enact web apps; sound goes to the Denon, so they are seldom opened', 'Overlay mount (theme)', 6)
row(g, 'Bluetooth surround tuning and LG soundbar app', 'one or two Bluetooth speakers, auto and manual tuning; LG soundbar mode, room calibration, EQ, night mode',
    ['com.webos.app.btsurroundautotuning', 'com.webos.app.soundbar-setting'], 'never', 'No', 'Keep (dormant)', 'needs Bluetooth speakers or an LG soundbar; the Denon is neither', '—', '—')

# ---------------------------------------------------------------- 4 Notifications & prompts
g = 4
row(g, 'Toast: standard', 'grey bar 900 px wide, bottom centre, 1-3 lines of 30 px text, no buttons, slides up 400 ms, 5 s default (1-10 s); also carries the key-handler "not available" messages',
    [F('SystemUI/Notification/Toast.qml', 'system-qml', '286 lines + ToastView.qml 149')], 'daily', 'Yes · mock 07', 'Replace',
    'rebuilt to the item contract, upper right (mock 07); Home Assistant `notify` arrives here too [INFERENCE: via apiadapter]', 'View swap', 2)
row(g, 'Toast: advanced (icon and link)', '66 px icon plus text; with `onclick` it is a pointer link (hover scale 1.02) that runs the action and closes; no key focus',
    F('SystemUI/Notification/Toast.qml', 'system-qml', 'type advanced'), 'sometimes', 'Yes', 'Replace', 'keep the click action: it calls `surfacemanager`', 'View swap', 2)
row(g, 'Toast: light', 'small 50 % black pill, top right, 36 px icon and text (e.g. "Touch Lock")',
    F('SystemUI/Notification/Toast.qml', 'system-qml', 'type light'), 'never', 'Yes', 'Replace', 'same renderer; only the touch-lock toast uses it and the C4 has no touch panel', 'View swap', 2)
row(g, 'Alert: standard and modal', 'light-grey card 1014 px wide, 180 px above the bottom, optional title, up to 10 lines, 225x54 buttons; modal adds a 0.3 scrim and ignores Back, Home and scrim clicks',
    [F('SystemUI/Notification/Alert.qml', 'system-qml', '672 lines + AlertView 194, SimpleButton 203, Selectable 202, MoonstoneShadow 114')], 'sometimes', 'Yes · mock 06', 'Replace',
    'a button is a luna call made as the source app (`callForApplication`); keep that', 'View swap', 2)
row(g, 'Alert: full-screen warning', 'black page, 72 px title, 39 px body, centred 270x72 buttons; used by power-off alerts, second-screen PIN pairing and Usage Care',
    F('SystemUI/Notification/Alert.qml', 'system-qml', 'type warning'), 'rare', 'Yes · mock 06 (card only)', 'Replace', 'same contract, full-screen layout', 'View swap', 2)
row(g, 'Alert: icon, checkbox, countdown, busy, progress', 'wider card with icon or thumbnail, tick box line, auto-close countdown with spinner, busy ring, 750x6 progress bar fed by `setAlertProgress`',
    [F('SystemUI/Notification/Spinner.qml', 'system-qml', '208 lines + ProgressBar 63, CheckBoxItem 128')], 'rare', 'Yes', 'Replace', 'one card, variants by field; progress and countdown need the update signal', 'View swap', 2)
row(g, 'Input alert', 'white parallelogram cards 300x262 with port icon and title, close button above, 0.6 scrim, bounce-in; created by `eim` and `asm` for connected devices and USB',
    [F('SystemUI/Notification/AdvancedAlertView.qml', 'system-qml', '210 lines + AdvancedAlertDelegate 238, AdvancedAlertCard 264')], 'sometimes', 'Yes · mock 18', 'Replace', 'mock 18 shows it as a wide toast with Switch focused', 'View swap', 2)
row(g, 'PIN prompt', 'full-screen black page: title, four boxes, 0-9 keypad, backspace, OK; wrong code shows "Check Password" and the third try closes it; first-use and Settings add set-new and confirm pages',
    [F('SystemUI/Notification/PincodePrompt.qml', 'system-qml', '734 lines + PincodePromptView 171, PincodePromptButton, PincodePromptNumButton, PincodePromptPasswordButton, PincodePromptBaseButton')], 'rare', 'Yes · mock 19', 'Replace',
    'digits return through `closePincodePrompt` and the daemon validates; keep that relay', 'View swap', 2)
row(g, 'Notification variants that cannot appear', 'partial-view alert and PIN strip (rollable TVs), secure arrow-pad PIN, PIN tooltip with empty text, 18 unused 9-slice images',
    [F('SystemUI/Notification/Tooltip.qml', 'system-qml', '75 lines + PincodePromptArrowpad 30, ArrowLockInput 108')], 'never', 'No', 'Keep (dormant)', '`supportScreenType` none, `pincode` methods keyboardMode only', '—', '—')
row(g, 'Second-screen pairing prompt', '"Connection Request: {name}" Yes/No, or the PIN as the title with Cancel; shown when a phone, LG TV Plus or Home Assistant `webostv` pairs; wakes the screen first',
    [S('com.webos.service.secondscreen.gateway')], 'rare', 'No', 'Replace', 'must stay answerable only by a physical remote key (Principle 0); buttons call `pairing/acceptPairing`', 'View swap', 2)
row(g, 'Exit-app alert', '"Do you want to exit the app?" Exit/Cancel, non-modal, from Back or Close at an app that does not take Back',
    F('KeyFilters/systemUi.js', 'js', 'handleSystemKeys'), 'rare', 'Yes', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Notification Centre', 'list with unread badge, delete mode, select all, mark all read, reminder settings, groups; no key opens it, Home has a bell and only Japan gets a Screen Remote button',
    'com.webos.app.notificationcenter', 'rare', 'Yes · mock 13', 'Replace', 'own drawer reading the same history', 'Key filter and launch redirect', 7)
row(g, 'App permission prompts', '"Do you want to enable microphone access on {appId}?", file access, Nearby Devices; Allow or Block',
    [S('ppm', 'blegatt')], 'rare', 'No', 'Replace', 'alert contract; granting stays a human choice', 'View swap', 2)
row(g, 'Software update prompt', '"The latest version of the software is available for your TV. Do you want to update now?", "Restart your TV to finish installing important updates"; the banner adds a download badge',
    [S('update')], 'rare', 'Part · mock 22', 'Replace', "'Not now' focused; a firmware update can undo the root", 'View swap', 2)
row(g, 'App, AI package and storage messages', '"Application has been installed.", "Some apps are available for update", "AI 8K Upscaling is Updated.", "Not enough storage available"',
    [S('appinstalld', 'pkginstall', 'adaptive-partition-installer', 'sam', 'ai-inference-manager', 'nn-proxy', 'aiservice')], 'rare', 'No', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Low-memory app restart', '"This app will now restart to free up more memory."',
    [S('memchute')], 'rare', 'No', 'Replace', 'toast; a 2 GB set restarts heavy apps now and then', 'View swap', 2)
row(g, 'Smart-home and ThinQ messages', 'appliance finished or door open, DR Mode, Independa incoming call',
    [S('iot-client-webos', 'independa-notification', 'google-home-controller', 'lg.thinqai.adapter')], 'never', 'No', 'Keep (dormant)', 'no ThinQ appliances or account known on the set', '—', '—')
