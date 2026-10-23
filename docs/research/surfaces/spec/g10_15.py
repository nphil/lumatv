from core import *

# ---------------------------------------------------------------- 10 Casting & sharing
g = 10
row(g, 'AirPlay', 'Apple receiver in a native shell: video with transport OSD, audio now-playing, mirroring, PIN panel, info and HomeKit "curtain" card, remote keyboard; prompts "AirPlay is disconnected.", Wi-Fi turn-on',
    ['airplay', S('airplay-adaptor')], 'rare', 'No', 'Keep', "Apple's receiver UI; its prompts use the alert contract", 'View swap (prompts)', 2)
row(g, 'Screen Share (Miracast) and App Casting', 'waiting screen with the TV name, phone screen full-screen with a top menu (Options, Size, Move, Hide, End), "requests screen sharing to your TV" Accept, Decline or Block with a PIN; App Casting asks "exit screen sharing?"',
    ['com.webos.app.miracast', 'com.webos.app.appcasting', S('miracast', 'appcasting')], 'rare', 'No', 'Keep', "mirrored phone content is the screen; prompts use the alert contract", 'View swap (prompts)', 2)
row(g, 'Google Cast', 'Cast receiver runtime with its own screen; LG adds the terms and network dialogs and a disconnect toast; the settings page is Google\'s',
    ['com.webos.chromecast', 'com.webos.chromecast-settings', S('chromecast-provisioning')], 'rare', 'No', 'Keep', "Google's cast screen; dialogs use the alert contract", 'View swap (dialogs)', 2)
row(g, 'DLNA remote control', '"The external device is requesting TV control. Do you want to continue?", "Buffering is in progress..."',
    [S('dmr')], 'rare', 'No', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Sound Share', 'phone audio through the TV: black "Playing the sound from your connected device." screen, Bluetooth Audio Playback panel (device list, play, pause, Screen off), "{Device} requests Sound Share to your TV" Accept, Decline or Block',
    ['com.webos.app.hdmioptical', 'com.webos.app.btspeakerapp', S('bluetooth-audio-sink', 'soundshare')], 'rare', 'No', 'Keep', 'phone-audio panels; prompts use the alert contract', 'View swap (prompts)', 2)
row(g, 'Mobile pairing PIN popup', 'grey centred panel "<device> is trying to connect." with the PIN digits and a 10-device note, Cancel, 30 s; plus "Currently connecting to the mobile device" toasts (LG ThinQ, TV Plus)',
    [F('RemoteUserList/PinPopup.qml', 'system-qml', '182 files / 11 qml; id `com.webos.systemui.mobileconnection`'), F('WebOSCompositor/views/StarfishRemoteUserListView.qml', 'compositor-qml', '17 lines')], 'rare', 'No', 'Replace', 'registered in configd `systemUIManager`, so the redirect already reaches it', 'System UI redirect', '3*')
row(g, 'TV Plus guide and device row', 'five-page "LG TV Plus Guide" and a device row with Disconnect, Screen Share and Content Share; nothing instantiates it',
    [F('RemoteUserList/RemoteUserListTooltip.qml', 'system-qml', '1171 lines + Delegate, Model, Shadow, OpacityMaskImage, Style')], 'never', 'No', 'Keep (dormant)', 'dead code in the module', '—', '—')

# ---------------------------------------------------------------- 11 Ads, promos & tracking
g = 11
row(g, 'Home hero banner, sponsored shelves and usage logging', 'hero banner, inline banners, "Sponsored" shelves from `admanager` slots, `NL_*` usage events; `homePromotion` is off but recommended shelves are still requested while terms allow',
    ['com.webos.app.home'], 'never', 'Part', 'Off', 'the Home stub removes the Flutter process; Glasshouse blocks the hosts', 'Home stub', 1)
row(g, 'Q-cards', 'Home tiles with an ad-slot banner, content tabs, recently watched, Edit App List; mostly empty with LG hosts blocked',
    ['com.webos.app.universalqcard', 'com.webos.app.universalqcard1', 'com.webos.app.universalqcard2', 'com.webos.app.universalqcard3', 'com.webos.app.learning'], 'never', 'No', 'Off', 'LumaHome has no Q-card tiles; the four copies share one bundle', 'Launcher filter', 7)
row(g, 'New & Hot and Music card', 'upcoming and popular titles with reminders and promo banners; mood playlists with a promo banner',
    ['com.webos.app.newandhot', 'com.webos.app.totalmusic'], 'never', 'No', 'Off', 'cloud shelves needing an LG account; not offered in LumaTV', 'Launcher filter', 7)
row(g, 'Premium', 'loader for a legacy store page at `lgrecommends.lgappstv.com`',
    'com.webos.app.premium', 'never', 'No', 'Off', 'legacy; only a MIME launch reaches it', 'Launcher filter', 7)
row(g, 'LG Channels', 'server-hosted free channel guide and VOD with ads; account required; the remote has an LG Channels key',
    'com.webos.app.lgchannels', 'rare', 'No', 'Off', 'ad-funded; not offered in LumaTV (preload already dropped); the remote key still opens it unless remapped', 'Launcher filter', 7)
row(g, 'LG Movies & TV', 'LG rental and purchase store', 'com.lgtvod.us.app', 'never', 'No', 'Off', 'store promotion; not offered in LumaTV', 'Launcher filter', 7)
row(g, 'Sports (My Team) and sports alarm', 'league and team setup, schedules; popups "The {League} game starts soon" with Watch Live, scores, Open with',
    ['com.webos.app.sportsteamsettings', F('SportsAlarm/SportsAlarm.qml', 'system-qml', '245 files / 50 qml; id `com.webos.app.sportsalarm`'), S('sportsalarm')], 'never', 'Yes', 'Off', 'no team set (`mySportTeamList` empty); empty view through the redirect', 'System UI redirect', 7)
row(g, 'Nudges', 'bottom-left ribbon with up to 5 lines and an OK button, optional 150 px tiles: AI Recommendation promos, "You may like this content", Alexa, Bluetooth speaker, Magic Explorer, shortcut-key offers, optimise-for-game; follow-up toasts',
    [F('Nudge/NudgeSystemUI.qml', 'system-qml', '297 files / 51 qml; id `com.webos.app.nudge`'), S('nudge')], 'never', 'Yes', 'Off', '`aiNudge` and `aiChannel` are off; the empty view and `nudge.service` off stop the rest', 'System UI redirect + Service off', 7)
row(g, 'Live Plus (content recognition) overlays', 'transparent "LivePlus" page in the input\'s ACR layer, loads a card from the ACR service; can shrink the video on close',
    ['com.webos.app.acrcomponent', 'com.webos.app.acrhdmi1', 'com.webos.app.acrhdmi2', 'com.webos.app.acrhdmi3', 'com.webos.app.acrhdmi4', 'com.webos.app.acroverlay'], 'never', 'Yes', 'Off', '`livePlus` off, ACR opt-out file present, `acr2` not running', 'LG setting', 7)
row(g, 'Ad and shopping overlays', 'transparent page in the ACR layer that redirects to an ad or shopping URL from `adoverlay-service`',
    ['com.webos.app.adhdmi1', 'com.webos.app.adhdmi2', 'com.webos.app.adhdmi3', 'com.webos.app.adhdmi4', 'com.webos.app.adoverlay', 'com.webos.app.adoverlayex'], 'never', 'Yes', 'Off', '`customizedAd` off, `doNotSellMyPersonalInformation` on, `getAppStatus` false', 'LG setting', 7)
row(g, 'Ad banner card and ad manager', 'transparent card with a "Sponsored ad" banner, Exit ad, More and a network popup; the manager also raises parental-control and "no longer supported" alerts',
    ['com.webos.app.adapp', S('admanager')], 'never', 'Yes', 'Off', 'nothing is requested with the ad hosts blocked', 'LG setting', 7)
row(g, 'Store promotion strip', 'moving strip beside the picture with model name, "New", store logo and promo images; retail demo mode only',
    [F('StoreStreamer/StoreStreamer.qml', 'system-qml', '49 files / 35 qml; id `com.webos.app.store-streamer`')], 'never', 'Yes', 'Off', '`storeMode` is home', 'System UI redirect', 7)
row(g, 'Home Office and Remote PC', 'RDP or VNC connection manager with a promo banner and website shortcuts; full-screen FreeRDP session',
    ['com.webos.app.homeoffice', 'com.webos.app.rdp'], 'never', 'No', 'Off', 'not used; no LumaHome tile', 'Launcher filter', 7)
row(g, 'LG Buddy and Copilot link', 'family sharing over KakaoTalk (QR connect, photos, help requests); the Copilot quick link only opens the browser',
    ['com.webos.app.buddy', 'com.webos.app.quicklinkcopilot', S('com.webos.service.buddyconnector')], 'never', 'No', 'Off', 'Korea-oriented; no LumaHome tile', 'Launcher filter', 7)
row(g, 'Viewing-data messages', '"The Usage Data is deleted.", "Last month, you watched TV for an average of about {hour} hours per day ..."',
    [S('user-context-manager', 'user-intent-manager')], 'never', 'No', 'Off', '`personalRecommend` is off', 'LG setting', 7)

# ---------------------------------------------------------------- 12 Accounts & first run
g = 12
row(g, 'First-use wizard', 'language, country, time zone, accessibility, remote or phone setup, network, terms, analysis, check, install type, AI, ambient, software update, parental PIN',
    'com.palm.app.firstuse', 'never', 'No', 'Keep (dormant)', 'one-time after a factory reset', '—', '—')
row(g, 'Welcome after setup (OOBE)', 'Amazon Alexa offer, Recommended Apps and Services, Useful Features (Simple Mode, Quick Settings, Help key, My Channel)',
    'com.webos.app.oobe', 'never', 'No', 'Keep (dormant)', 'one-time after a factory reset', '—', '—')
row(g, 'Network and terms overlay', 'Wi-Fi or Ethernet setup (`target` network) and User Agreements (`target` eula) over the current app; same bundle as First Use',
    'com.webos.app.firstuse-overlay', 'rare', 'Part', 'Restyle', 'Enact web app; Settings › Network opens it', 'Overlay mount (theme)', 6)
row(g, 'LG Account', 'loader for the hosted LG Account page (sign in, sign up, billing); overlay variant with a local network-error page; "{ID} logged out", "log in again because account information has expired"',
    ['com.webos.app.membership', 'com.webos.app.overlaymembership', S('accountmanager')], 'rare', 'Part', 'Keep', 'hosted page and account prompts; the Home chip reads `accountmanager` only', '—', '—')
row(g, 'Terms, privacy and country-change prompts', '"Our Privacy Policy has changed. The TV will now restart to show the updated User Agreements", ad-consent withdrawn notices, "Changing the country will cause your TV to restart"',
    [S('eula-service', 'sdx')], 'rare', 'No', 'Replace', 'alert contract; a restart prompt must stay answerable', 'View swap', 2)
row(g, 'Server notice', 'LG server notice with More and OK', [S('sdp-server-notice')], 'rare', 'No', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Consent client', 'blank page that loads an interactive consent URL given at launch',
    'com.webos.app.cmp-client', 'never', 'No', 'Keep (dormant)', 'no launcher found; needs a URL from a service', '—', '—')

# ---------------------------------------------------------------- 13 Service & factory
g = 13
row(g, 'Installer and hotel menu', 'power-on state, default input, IP control and SDDP, limited mode, key management, cloning, volume, auto browser',
    'com.webos.app.installation', 'never', 'No', 'Keep (dormant)', 'behind the IP Control PIN or hotel-mode key', '—', '—')
row(g, 'Factory window', 'EZ-Adjust, In-Start, power-only, test patterns, board options, panel repair, OLED menus (299 qml)',
    'com.webos.app.factorywin', 'never', 'No', 'Keep (dormant)', 'service IR keys only; not on the Magic Remote', '—', '—')
row(g, 'Service diagnostics', 'HDMI error history, Wi-Fi and Bluetooth diagnostics, panel errors, power history, firmware versions',
    'com.webos.app.svcdiagnostics', 'never', 'No', 'Keep (dormant)', 'opens only for country KOR from Quick Help', '—', '—')
row(g, 'Store demo', 'promo video player, USB copy, passwords, sync of playlists between store TVs; store alerts "TV is changed to Store Mode"',
    ['com.webos.app.store-demo', 'com.webos.app.sync-demo'], 'never', 'No', 'Keep (dormant)', '`storeMode` is home', '—', '—')
row(g, 'USB and expert software update', 'installed vs update version with Update and Cancel, progress "do not remove USB", reboot notice',
    'com.webos.app.softwareupdate', 'never', 'No', 'Keep (dormant)', 'only with the expert or USB flag; needs a `mode` parameter', '—', '—')
row(g, 'Hotel (HCAP) view', 'transparent layer for hotel client windows over input apps; hotel key policy',
    [F('WebOSCompositor/views/StarfishHCAPView.qml', 'compositor-qml', '135 lines + hcap/StarfishHCAPSurface.qml')], 'never', 'No', 'Keep (dormant)', '`enableHotelMode` off', '—', '—')
row(g, 'Hidden key combinations', 'gear three times in 3 s opens the AV-reset menu ("All video/audio settings will be reset?"), gear held 5 s, Yellow held 5 s (panel test pattern), Green seven times (FreeSync info), Blue seven times (panel usage dump)',
    [F('KeyFilters/appLaunch.js', 'js', 'handleAvReset, handleHotelmode, handleFreeSyncInfo, handlePanelUTTUsbDownload, handleHotkeys'), 'com.webos.app.tvhotkey'], 'rare', 'No', 'Off', 'swallow the accident-prone ones in the key filter; AV reset stays under Settings', 'Key filter and launch redirect', '1*')
row(g, 'LG Remote Service session', '"The LG Remote Service has started.", "Real-time remote inspection is in progress.", "has ended. Your TV will now restart."',
    [S('remotediag')], 'never', 'No', 'Replace', 'alert contract; never filtered, it shows when LG support is connected', 'View swap', 2)
row(g, 'System recovery messages', '"Update Failed", "System ran into a problem and needs to be repaired.", "The system is successfully recovered."',
    [S('data-migrator', 'db8-translator')], 'never', 'No', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Rollable-TV stand errors', '"An error has occurred to the TV stand", "Press [OK] to switch the screen to Line View", partial-view volume placement',
    [S('rollingscreen'), F('WebOSCompositor/views/PartialViewPositioner.qml', 'compositor-qml', '75 lines')], 'never', 'No', 'Keep (dormant)', 'fixed panel', '—', '—')
row(g, 'Developer overlays', 'FPS box (needs a flag file), debug window, focus and surface consoles, resource monitor, log console',
    [F('WebOSCompositorBase/views/debug/DebugOverlay.qml', 'compositor-qml', '10 files; WebOS DebugConsole, MemoryView, ResourceView'), F('WebOSCompositor/views/StarfishViewsRoot.qml', 'compositor-qml', '252 lines (FpsCounter box)')], 'never', 'No', 'Keep (dormant)', 'every switch defaults to off and the flag file is absent', '—', '—')
row(g, 'Secondary window', 'a 768x2160 second Qt window on `fb1` for system-UI overlays; only one panel exists',
    F('WebOSCompositor/StarfishSecondaryMain.qml', 'compositor-qml', '94 lines'), 'never', 'No', 'Keep (dormant)', 'the compositor opens only `card0`', '—', '—')

# ---------------------------------------------------------------- 14 Accessibility
g = 14
row(g, 'Accessibility', 'Vision (Audio Guidance, zoom, high contrast, invert, grayscale), Hearing (descriptions, amplify dialogue, hearing aid, sign language), Control (Learn Remote Control, shortcuts, Tips)',
    'com.webos.app.accessibility', 'rare', 'Part', 'Restyle', 'Enact web app; the switches also appear as Settings › System rows', 'Overlay mount (theme)', 6)
row(g, 'Accessibility quick panel', 'compact shortcuts panel over the current app; opens when Mute is held 2 s or on the Multikey code',
    'com.webos.app.accessibility-overlay', 'rare', 'No', 'Keep', 'a deliberate 2 s hold on Mute; the key filter must not swallow Mute', '—', '—')
row(g, 'Screen Zoom, grayscale and invert', 'whole-UI shader: zoom 110-200 % with a pan minimap, key guide and four arrows, grayscale, inverted colours; Home held 2.5 s starts it',
    [F('WebOSCompositor/views/StarfishViewAdjustments.qml', 'compositor-qml', '868 lines'), F('WebOSCompositorBase/zoom/ZoomView.qml', 'compositor-qml', '3 files incl. ZoomKeyGuide.qml, ZoomDecorationStyle.qml')], 'rare', 'No', 'Keep', 'accessibility; PLAN\'s 650 ms Home hold reaches LG\'s Home first, so offer zoom as a Settings row', 'Key filter and launch redirect', '1*')
row(g, 'Learn Remote Control', 'press a key on the real remote and its function appears; Back twice to leave',
    'com.webos.app.remotecontrolguide', 'rare', 'No', 'Restyle', 'Enact web app', 'Overlay mount (theme)', 6)
row(g, 'Sign Language Zoom', 'a magnifier window over the chosen picture area and an "Select the area for sign language" frame; `isSLZoomOn` is off',
    [F('inputcommon/.../FgViewLayer/SLZoomView.qml', 'qml', '230 lines + SLZoomSelectArea 450')], 'never', 'No', 'Keep (dormant)', 'switched off in Settings', '—', '—')
row(g, 'Sign-language avatar', '3D avatar drawn over content in a top-UI window; menu language must be Korean; download prompts',
    ['com.webos.app.signlanguage-avatar', S('signlanguage-avatar'), F('WebOSCompositor/views/StarfishTopUIView.qml', 'compositor-qml', '115 lines + topui/StarfishTopUISurface.qml')], 'never', 'No', 'Keep (dormant)', 'Korean only and not in `listApps`', '—', '—')
row(g, 'AI Caption', '"Analyzing sound to generate AI Caption.", "AI Caption has ended.", "{mobile} is requesting to connect to the device."',
    [S('nais-offloading-manager')], 'never', 'No', 'Replace', 'alert contract', 'View swap', 2)
row(g, 'Audio Guidance and language packs', 'spoken focus and alert text through `tts`; prompts to download a high-quality language pack; `audioGuidance` is off',
    [S('tts')], 'never', 'No', 'Keep', 'LG voice; the spoken hooks in alerts must survive the rebuild', '—', '—')
row(g, 'Captions and subtitle layers', 'caption and subtitle layers under the transparent inputcommon window on Live TV and inputs',
    [F('inputcommon window-group layers', 'qml', 'subtitle, ttxSubtitle, ttmlSubtitle')], 'rare', 'No', 'Keep', 'drawn by the media pipeline; the style is a Settings row', '—', '—')
