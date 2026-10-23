from core import *
RW = dict(ans='Restyle', hook='Overlay mount (theme)', ph=6)

# ---------------------------------------------------------------- 5 Settings & wizards
g = 5
row(g, 'Quick panel', 'left-edge card 492x1056: icon row (All Settings, Chatbot, Accessibility, Network), "Recent History", 2-column grid of up to 12 tiles; on this set Picture Mode, OLED Pixel Brightness, Sleep Timer, Game Optimizer, Screen Off, Energy Saving',
    [F('QuickSettings/QuickSettings.qml', 'system-qml', '799 files / 165 qml; id `com.webos.app.quicksettings`')], 'sometimes', 'Yes · mock 10', 'Replace',
    'built and resident; LG rebuilds per open; tiles drive the same `settingsservice` keys', 'System UI redirect', 3)
row(g, 'Quick panel popups and edit mode', 'slider popup with value text, value picker with sub-buttons (Advanced Settings, wizards, Device List), edit mode (Move or Delete), "Select a menu to register" list, "Add to Favorites"; delete, Bluetooth-guide and disabled-tile messages go through the notification layer',
    [F('QuickSettings/Containers/BarSliderPopup.qml', 'system-qml', 'with DotPickerPopup, MenuPopupArea, GridEditor, EditOptional, QuickSettingsCandidateListView')], 'sometimes', 'Yes · no mock (mock 10 is the panel with steppers)', 'Replace',
    'same module; alert buttons call back with `params.action`, keep that', 'System UI redirect', 3)
row(g, 'Quick panel pieces that cannot appear', 'Easy (Senior) panel, Touch Lock, Battery Brightness, Wireless AV, External Devices tiles, local-key guidance strips',
    [F('QuickSettings/Containers/SeniorTVQuickSettings/EasySettingsWindow.qml', 'system-qml', '362 lines + 6 qml')], 'never', 'No', 'Keep (dormant)', '`seniorMode.support` false, no touch panel, no battery, `supportWirelessAV` false, one-key local button', '—', '—')
row(g, 'Settings', 'tabs Picture, Sound, General, Support; General holds Accessibility, AI Service, Game Optimizer, Channels, Network, System, Energy Saving, Usage Care, OLED Care; gear held 1 s opens it, apps call it with {target, item}',
    'com.palm.app.settings', 'sometimes', 'Yes · mocks 01-04, 24', 'Replace', 'own resident Settings in nine sections; same `settingsservice` writes', 'Key filter and launch redirect', 3)
row(g, 'Settings sections without a mock', 'Inputs & devices, Network, Ambient light, Home & look, System (time, language, OLED care, accessibility rows, About)',
    ['com.palm.app.settings'], 'sometimes', 'Yes · no mock', 'Replace', 'sections per `settings-map.md`; Ambient light is LumaTV-only', 'Key filter and launch redirect', 3)
row(g, 'Settings deep screens and sub-apps', 'expert calibration, channel tuning, legal, LG account, Network, Family Settings, software update: "More in LG settings" rows open them directly',
    ['com.palm.app.settings'], 'rare', 'Yes', 'Restyle', 'Enact Sandstone bundles; one shared theme', 'Overlay mount (theme)', 6)
row(g, 'Setting-change confirmations and suggestions', '"Picture Mode has changed.", energy-use notices, "Do you want to keep the changed screen settings?", "When movie content is recognized, you can change to Filmmaker mode.", "You are watching TV quite a lot."',
    [S('pqcontroller', 'com.webos.service.ocp', 'lightingcontroller')], 'rare', 'No', 'Replace', 'alert contract; drop the recommendation ones by source (Filmmaker mode stays)', 'View swap', 2)
row(g, 'Personalized Picture Wizard', 'pick images for preference, sharpness, colour tone and temperature, then Before/After and Apply; saves to the LG account when signed in',
    'com.webos.app.picturewizard', 'rare', 'Part', 'Restyle', 'Enact web app; Filmmaker mode stays the picture mode', 'Overlay mount (theme)', 6)
row(g, 'Support tools', 'Quick Help symptom list and recommended settings, picture and sound self-test, LG Remote Service (shows a code to read to an LG agent); nothing opens them unasked',
    ['com.webos.app.self-diagnosis', 'com.webos.app.customersupport', 'com.webos.app.remoteservice'], 'rare', 'Part', 'Restyle', 'Enact web apps; Remote Service needs Nitin to read out the code', 'Overlay mount (theme)', 6)
row(g, 'Help and guides', 'Tips cards, offline TV User Guide (Help key is not on the Magic Remote), "Try Now" links into Settings',
    ['com.webos.app.helpandtips', 'com.webos.app.tvuserguide'], 'rare', 'No', 'Restyle', 'Enact web apps', 'Overlay mount (theme)', 6)
row(g, 'Usage Care', 'weekly viewing time and top apps, Time Locks, Parental Control Mode, Eye Care, Volume Limit, input, app and channel locks; the service opens it at end of use when limits are set',
    'com.webos.app.familycare', 'rare', 'Part', 'Restyle', 'Enact web app', 'Overlay mount (theme)', 6)
row(g, 'Weather location', 'city search that stores the Home weather location; needs LG weather servers',
    'com.webos.app.weatherlocation', 'rare', 'No', 'Off', "LumaHome's weather widget has its own location", 'Launcher filter', 7)

# ---------------------------------------------------------------- 6 Live TV
g = 6
row(g, 'Live TV app', 'no UI of its own: inputcommon in Live TV mode over the tuner video; operator-app variant unused',
    ['com.webos.app.livetv', 'com.webos.app.livetvopapp'], 'rare', 'Part', 'Keep', 'window host; its pixels are inputcommon rows below', '—', '—')
row(g, 'Channel banner', 'lower-left strip: channel number (105 px), logo, source icon, programme, start-end time and progress bar; OK adds Options and Guide buttons; 5 s',
    [F('inputcommon/UserInterfaceLayer/Containers/MainViewLayer/ChannelBanner/ZappingBanner.qml', 'qml', '574 lines + ChannelBanner 283, DeeplinkView 74')], 'rare', 'Yes · mock 20', 'Restyle', 'the one Live TV surface PLAN restyles (Decided 2026-10-23)', 'Overlay mount', 6)
row(g, 'Channel detail banner', 'programme list (previous, now, next) with times, progress, synopsis scroll, rating, audio and subtitle icons, REC reservation; INFO key, 30 s',
    [F('inputcommon/.../ChannelBanner/DetailInfoBanner.qml', 'qml', '373 lines + DetailInfoItem 786, SynopsisScrollBar 58, ProgressBar 81')], 'rare', 'Part', 'Restyle', 'same module as the banner; no guide screens', 'Overlay mount', 6)
row(g, 'Guide and channel list', 'left tab menu (TV Guide, Channels, Scheduler), channel list, EPG time grid, PIP, options; Guide, Channel List and Live TV Menu keys',
    'com.webos.app.livemenu', 'rare', 'No', 'Keep', 'Nitin: Live TV is rare, no guide (2026-10-23)', '—', '—')
row(g, 'Channel number entry', 'list of channels matching typed digits (ATSC dash major-minor); Italy-only overlay app for number keys',
    [F('inputcommon/.../SimilarListView.qml', 'qml', '650 lines'), 'com.webos.app.similarlist'], 'rare', 'No', 'Keep', 'antenna use only; the Flutter overlay is Italy-only', '—', '—')
row(g, 'TV Scheduler', 'list of reminders and scheduled recordings with start time, title, channel; add or edit with storage info',
    'com.webos.app.scheduler', 'rare', 'No', 'Restyle', 'Enact web app; Live TV only', 'Overlay mount (theme)', 6)
row(g, 'Channel tuning and Channel Manager', 'auto and manual tuning for antenna, cable and satellite with progress; list with All and Favourites, edit, delete, lock, sort',
    ['com.webos.app.channelsetting', 'com.webos.app.channeledit'], 'rare', 'Part', 'Restyle', 'Enact web apps; done once per antenna', 'Overlay mount (theme)', 6)
row(g, 'Emergency alert (US EAS)', 'popup, bottom then top every 2 min: title ("BLUE ALERT"), locations and text, scrollbar, Close; video keeps playing; auto-hides after 2 min',
    [F('inputcommon/.../PopupLayer/EwsUsView.qml', 'qml', '369 lines')], 'rare', 'No', 'Keep', 'safety text from the broadcaster; appears only on the antenna input', '—', '—')
row(g, 'Live Pick and recording indicators', 'Live Pick strip ("Would you like to learn more about ...?") and REC or Live Playback chips on banners',
    [F('inputcommon/.../LivePick/LivePickView.qml', 'qml', '549 lines + 3 qml; RecInfoIcon, DvrIndicatorView')], 'never', 'Part', 'Off', '`livepick` service absent, `broadcastInfoNoti` off, `dvrReady` false', 'LG setting', 7)
row(g, 'DVR, recordings and live playback', 'transport bar, info chip, storage and CI-password dialogs, record dialogs, recordings list',
    ['com.webos.app.dvrpopup', 'com.webos.app.recordings', F('inputcommon/.../Dvr/DvrMainView.qml', 'qml', '742 lines + DvrPlayView, DvrProgressBarView, DvrIconButton, DvrFormattedTimeText, DvrInfoView, DvrPopupView, DvrCiPincodeView, DvrInputText, LoadingMessageView')],
    'never', 'No', 'Keep (dormant)', '`dvrReady` false in `tv.model` and `tv.rmm`', '—', '—')
row(g, 'Broadcast families that cannot appear', 'CI+ menus, Teletext, LCN pickers (UK, Italy), Music Catch, Ginga list, EMM and CA messages, Japan mini banners, Freeview Play splash, Indonesia EWS, on-screen clock, voice zapping, LG Channels splash and activation; HbbTV, Ginga, Interactive and Red Button layers',
    ['com.webos.app.crb', 'com.webos.app.livedmost', S('hybridtv-manager', 'hbbtv3-pluggable', 'opapp-manager', 'dmost'), F('inputcommon/.../PopupLayer + BgViewLayer regional views', 'qml', 'CiPopup, TeletextOptionView, LcnConflictGbr, LcnConflictIta, MusicCatchPopupView, GingaAppCatalog, EmmMessageView, CaMessageView, FvpSplash, EwsView, ClockView, VoiceChannelZapping, LgChannelSplash, LgChannelActivation, MiniBanner')],
    'never', 'No', 'Keep (dormant)', 'US ATSC set, HbbTV off, no CAM, country gates', '—', '—')
row(g, 'Live TV prompts', '"To watch Live TV, channels must be tuned. Start Channel Tuning now?", "The program you set a reminder for will begin in 1 minute", "TV is turned on because your scheduled program is about to start"',
    [S('broadcast-systemui-manager', 'tv-reservation-agent'), F('inputcommon/Interfaces/ChannelStatusInterfaces', 'qml', '514 lines')], 'rare', 'No', 'Replace', 'alert contract; inputcommon pop-ups restyle with the banner', 'View swap', 2)
row(g, 'My Channels wedge', 'ribbon of up to 20 favourite channels under Home with Add Channel, edit and empty state; shown only where a host loads it',
    [F('FavoriteChannels/FavoriteChannels.qml', 'system-qml', '141 files / 51 qml')], 'never', 'No', 'Off', 'LumaHome has no channel wedge; the channel list is empty here', 'Home stub', 1)

# ---------------------------------------------------------------- 7 Search, voice & keyboard
g = 7
row(g, 'On-screen keyboard', 'IME panel at the bottom with a 600 ms slide, drawn by maliit; opens on a text field or an app request, closes on app change, alert or Home',
    [F('WebOSCompositor/views/StarfishKeyboardView.qml', 'compositor-qml', '342 lines; IME `com.webos.service.ime`, maliit 85 qml')], 'sometimes', 'Yes · mock 09', 'Restyle', 'deterministic-row keyboard look on LG\'s IME', 'Overlay mount', 6)
row(g, 'Search and voice results', 'answers and result lists (apps, shows, channels, sports, weather, shopping, music), keyword grid with Sponsored keywords, chat box, Voice ID, keyboard; the Movies key opens it with "movie"',
    'com.webos.app.voice', 'sometimes', 'Yes · mock 09', 'Restyle', 'mock 09 shows local results; LG\'s cloud answers need Nitin\'s call (Needs Nitin)', 'Overlay mount', 6)
row(g, 'Voice listening bar', '540x90 light bar lower left: animated chrome, "Speak now.", live transcript and reply; Back and Exit go to it; opened by the mic key through `voiceconductor`',
    ['com.webos.app.voiceagent', F('VoiceAgent/VoiceAgentView.qml', 'system-qml', '1954 files / 10 qml (1940 animation PNGs, 3 earcons); host WebOSCompositor/views/StarfishVoiceAgentView.qml 31 lines')], 'sometimes', 'No', 'Restyle', 'keep the earcons and states, replace the 485-frame animation with one arc', 'Shadow module', '6*')
row(g, 'Microphone indicator', '81 px green microphone, bottom right, fades to 60 % after 3 s; `showIndicator` has no known caller here',
    F('WebOSCompositor/views/StarfishIndicatorView.qml', 'compositor-qml', '106 lines'), 'rare', 'No', 'Replace', 'tiny; do with the other shadow-module views', 'Shadow module', '5*')
row(g, 'AI Concierge', 'welcome message, keyword chips, tips and content cards (EPG, VOD, YouTube) with a content-provider chooser; needs LG terms',
    'com.webos.app.aiplatform', 'rare', 'No', 'Off', 'LG cloud assistant; LumaHome has no entry to it', 'Launcher filter', 7)
row(g, 'AI voice settings and Voice ID', 'hands-free, far-field and sensitivity settings, help categories, Voice ID register and manage with a recording flow',
    'com.webos.app.aiplatformsupport', 'rare', 'No', 'Keep', 'Flutter, reached only from Settings › AI Service', '—', '—')
row(g, 'TV help chatbot', 'chat bubbles, suggestion buttons, toggles, brightness slider, QR codes; the quick panel shows its icon when the app exists',
    'com.webos.app.assistant', 'rare', 'No', 'Off', "LG cloud helpdesk; LumaTV's quick panel does not link it", 'Launcher filter', 7)
row(g, 'Voice web overlay', 'transparent overlay hosting an iframe URL given at launch (web results, hints), closes by message',
    'com.webos.app.voiceweb', 'rare', 'No', 'Keep', 'launched by the voice framework with a URL; nothing of its own to restyle', '—', '—')
row(g, 'Other assistants', 'Amazon Alexa view and presentation overlay, Yandex Alice, Dangbei voice hints (China)',
    ['amazon.alexa.view', 'amazon.alexapr', 'yandex.alice', 'com.webos.app.dangbei-overlay'], 'never', 'No', 'Keep (dormant)', 'Alexa needs the `voice2` terms (false here); Alice and Dangbei are other markets', '—', '—')
row(g, 'Voice notices', '"Voice recognition will be available by connecting TV to the network", "The built-in mic is turned off.", Alexa and Voice ID messages, TTS language pack prompts',
    [S('voice-framework', 'voiceinput', 'amazon-alexa-adapter', 'tts')], 'rare', 'No', 'Replace', 'toasts and alerts through the one renderer', 'View swap', 2)

# ---------------------------------------------------------------- 8 Power, boot & screensaver
g = 8
row(g, 'Boot logo', 'animated "webOS" wordmark (88-frame sprite, 60 ms per frame) over a 1.2 s fade, closes with a 1.2 s fade, 10 s cap; launched by `bootd`; the LG-logo switch is `other.lgLogoDisplay`',
    [F('BootLogo/BootLogo.qml', 'system-qml', '142 files / 5 qml; id `com.webos.app.bootlogo`, 520 lines')], 'sometimes', 'Yes · mock 23', 'Replace', 'cold boots only; the set normally resumes from Active Standby', 'System UI redirect', 5)
row(g, 'Power-off logo', 'black scrim fades in over 1 s while the red LG logo fades in 1 s and out 1.5 s (about 3 s); `tvpowerd` waits for the compositor to report it finished',
    [F('MasterBrand/MasterBrand.qml', 'system-qml', '21 files / 5 qml; id `com.webos.app.masterbrand`, 365 lines')], 'daily', 'Yes · no mock (mock 23 is the boot screen)', 'Replace', 'own animation; must answer `responsePowerOffLogoRequest` or power-off stalls', 'System UI redirect', 5)
row(g, 'Startup cover', 'black screen with the first app name (120 px) and "Loading..."; opened by `bootd`, launches the first app, emits `smart-service-ready`, swallows keys; also the stolen-device block screen',
    [F('CoverUI/CoverUI.qml', 'system-qml', '55 files / 35 qml'), F('WebOSCompositor/views/StarfishCoverUIView.qml', 'compositor-qml', '67 lines'), F('WebOSCompositor/views/cover/StarfishCoverUI.qml', 'compositor-qml', '16 lines')], 'sometimes', 'No', 'Restyle', 'boot dependency: restyle the picture, keep the sequencing', 'Shadow module', '5*')
row(g, 'Boot popup', 'black bar 130 px high slides up: "One moment please. Smart service will be available shortly."; a hotkey pressed before boot finishes',
    F('WebOSCompositor/views/StarfishBootPopup.qml', 'compositor-qml', '121 lines'), 'rare', 'No', 'Replace', 'small; same toast look', 'Shadow module', '5*')
row(g, 'Screensaver', 'default particles with text, analogue or digital clock with date, sponsored image slideshow (off), Gallery+ art (off); starts when `tvpowerd` reports Screen Saver, any key wakes it; a 120 s timer closes overlays',
    ['com.webos.app.screensaver', F('WebOSCompositor/views/StarfishScreenSaverView.qml', 'compositor-qml', '136 lines'), F('WebOSCompositor/views/screenSaver/StarfishScreenSaver.qml', 'compositor-qml', '40 lines')], 'sometimes', 'Yes · mock 21', 'Replace', 'drawn in the compositor with OLED pixel shift; swallowed keys must still reset the idle timer', 'Shadow module (research)', 8)
row(g, 'Video screensaver container', 'blank web page that loads an LG Channels video-ad page',
    'com.webos.app.videoads', 'never', 'Part', 'Off', '`lgchannels.isVideoScreenSaverEnabled` false and LG hosts blocked', 'LG setting', 7)
row(g, 'Always Ready and Gallery+', 'art and clock when the set is "off": themes, AI art, photos, briefing; `lifeOnScreenMode` is `alwaysReady` today; the service asks "Going to {screen} screen within 1 minute" OK or Cancel',
    ['com.webos.app.lifeonscreen', S('alwaysready')], 'daily', 'Part', 'Replace', "[INFERENCE: what the set shows in standby is unverified] LumaTV's screensaver or standby look takes over (Needs Nitin)", 'Shadow module (research)', 8)
row(g, 'Line View (rollable TVs)', 'clock, music player, weather, home menu for partial view of rollable TVs',
    'com.webos.app.atelier', 'never', 'No', 'Keep (dormant)', '`supportScreenType` is none on a fixed panel', '—', '—')
row(g, 'Auto power-off countdown', 'toast "TV will be turned off in 5 minutes." then alert "in 1 minute ... press any button except Power"; the silent 15-minute no-signal off has no UI',
    [S('nopd')], 'rare', 'Part · mock 17', 'Replace', 'toast then alert; countdown toast with a draining bar (mock 17)', 'View swap', 2)
row(g, 'Sleep timer and timer power-off', 'Sleep Timer tile (Off, 10 min to 4 h) and the one-minute alert "Your TV will be turned off ... Cancel or OK"',
    [S('tv-reservation-agent')], 'rare', 'Part · mock 17', 'Replace', 'quick-panel tile plus the alert contract', 'View swap', 2)
row(g, 'OLED Pixel Cleaning prompts', '"Pixel Cleaning will start automatically when the TV is turned off", "Do you want to start right now?", "Pixel Cleaning is in progress", "do not unplug the power"',
    [S('eplmanager')], 'rare', 'Part', 'Replace', 'alert contract; care rows live in Settings › System', 'View swap', 2)
row(g, 'Thermal and fan shutdown', '"Your TV will turn off shortly as the TV\'s temperature is too high" and the cooling-fault text',
    [S('fancontroller')], 'rare', 'No', 'Replace', 'alert contract; never filtered, it is a safety warning', 'View swap', 2)

# ---------------------------------------------------------------- 9 Game
g = 9
row(g, 'Game Dashboard', 'bottom-centre 962x309 panel replacing the quick panel in game mode: Dark Room, Multi View, Menu Color, Game Optimizer, genre carousel, live strip (FPS, VRR, Low Latency, HDR, input lag), edit screen; HDMI game mode only',
    [F('QuickSettings/Containers/QuickGameSettings/QuickGameSettingsMainView.qml', 'system-qml', '624 lines + GameEditMainMenu, QuickGameGenreButton, QuickGameInformation, QuickGameGuide, GameLocalKeyGuidance')], 'rare', 'Part', 'Replace', 'the quick panel\'s game card (PLAN, Glasshouse); only matters if a console or PC is on HDMI (Needs Nitin)', 'System UI redirect', 3)
row(g, 'Game Optimizer', 'genre, picture and sound controls, FPS counter, crosshair, VRR and low latency, audio out, menu colour, screen position',
    'com.webos.app.gameoptimizer', 'rare', 'Part', 'Restyle', 'Enact web overlay; PLAN keeps a Settings row that opens it', 'Overlay mount (theme)', 6)
row(g, 'Games hub', 'recommended games and gameplay videos, recent, console inputs, Bluetooth controller settings; needs the LG network',
    'com.webos.app.gamehome', 'never', 'No', 'Off', 'cloud shelf; Moonlight and GeForce NOW are separate apps', 'Launcher filter', 7)
row(g, 'Game FPS counter and crosshair', 'FPS badge in a corner and a centre cross or dot on HDMI game input',
    [F('inputcommon/.../BgViewLayer/GameOptionView.qml', 'qml', '57 lines + GameFpsItem 244, GameCrosshair 49')], 'never', 'No', 'Keep (dormant)', 'needs the `monitor` profile, which this set lacks', '—', '—')
