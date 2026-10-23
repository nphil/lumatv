from core import *
g = 15
row(g, 'Netflix', 'native client with its own UI and protected video', 'netflix', 'daily', 'Out', 'Keep', "third-party UI and DRM video; only LumaTV's launch splash and spinner surround it", '—', '—')
row(g, 'YouTube', 'Cobalt client; a homebrew build with the same id sits under `/media/developer`, `listApps` registers the native one', 'youtube.leanback.v4', 'daily', 'Out', 'Keep', 'third-party UI', '—', '—')
row(g, 'Prime Video', 'native client (`ignition`)', 'amazon', 'sometimes', 'Out', 'Keep', 'third-party UI and DRM video', '—', '—')
row(g, 'Apple TV', 'native client; an Apple TV box also sits behind the Denon on HDMI 2', 'com.apple.appletv', 'rare', 'Out', 'Keep', 'third-party UI and DRM video', '—', '—')
row(g, 'Crunchyroll and GeForce NOW', 'web clients', ['com.crunchyroll.webapp', 'com.geforcenow.play'], 'rare', 'Out', 'Keep', 'third-party UI', '—', '—')
row(g, 'PlxNative', 'unofficial native Plex client with its own renderer', 'com.beb.plxnative', 'sometimes', 'Out', 'Keep', 'homebrew UI, its own window', '—', '—')
row(g, 'Moonlight and Punktfunk', 'game and desktop streaming clients', ['com.limelight.webos', 'io.unom.punktfunk.client-webos'], 'rare', 'Out', 'Keep', 'homebrew UI', '—', '—')
row(g, 'Hue Sync', "Signify's sync app", 'com.lighting.huesync', 'rare', 'Out', 'Keep', "third-party UI; LumaTV's Ambient light replaces its use", '—', '—')
row(g, 'Web Browser', 'Chromium-based Crow UI: URL bar, tabs, bookmarks, start page with Most Visited and Recommended Sites; microphone allowed',
    'com.webos.app.browser', 'rare', 'Part', 'Keep', 'web content in LG\'s browser; its 72 MB preload is already dropped', '—', '—')
row(g, 'Browser helpers', 'floating voice-search browser (Naver, Korea), PWA shell that opens a URL, web-app host with a grey spinner and an error image, single-image viewer',
    ['com.webos.app.beanbrowser', 'com.webos.app.tinybrowser', 'com.webos.app.webapphost', 'com.webos.app.tvsimpleviewer'], 'rare', 'No', 'Keep', 'content shells opened with a URL or a file; nothing of their own to restyle', '—', '—')
row(g, 'Store stubs', 'Viewster, 8 Ball Pool, Twitch, DAZN, Highbrow, Tennis TV and Karaoke Smart folders; only the first two are in the snapshot or `listApps`',
    ['com.lge.app.viewster', 'com.miniclip.eightballpool (live `listApps` only)', 'tv.twitch.tv.starshot.lg, dazn, com.joinhighbrow.highbrow, com.deltatre.tennistv.lgapp, karaokesmart (folders under `/media/cryptofs`)'], 'never', 'No', 'Keep (dormant)', 'stubs without a UI of ours', '—', '—')
row(g, 'AmazOff', 'Prime Video ad patcher; an nginx and a monitor run in the background without UI', 'com.amazoff.patcher', 'rare', 'No', 'Keep', 'homebrew; background processes only', '—', '—')
row(g, 'Homebrew Channel and helpers', 'installer, its updater, the LG app-update blocker and the input-hook front',
    ['org.webosbrew.hbchannel', 'org.webosbrew.safeupdate', 'org.webosbrew.appupdateblocker', 'org.webosbrew.inputhook'], 'rare', 'Part', 'Keep', 'root access and update blocking depend on them; the input-hook admin server is a PLAN "not taken"', 'Launcher filter', '1*')
row(g, '64 to 32-bit bridge and HyperHDR', 'EGL, GLES and Wayland bridge for 64-bit apps; HyperHDR loader',
    ['org.webosbrew.bridge-64to32', 'org.webosbrew.hyperhdr.loader'], 'never', 'Part', 'Keep', 'support software; hidden from the launcher', 'Launcher filter', '1*')
row(g, 'LumaTV app and frame', 'the LumaGlass app (renamed `org.nphil.lumatv` in phase 1) and its overlay pane for mini apps',
    ['org.nphil.lumaglass', 'org.nphil.lumaglass.frame'], 'sometimes', 'Yes', 'Keep', 'ours', '—', 1)
row(g, 'Magic Remote pointer image', 'native cursor (`pointerResourceStyle` LG); every surface has a hover state', ['native (compositor)'], 'sometimes', 'Out', 'Keep', 'native, swap unverified; `general.remotePointer` turns it off', '—', '—')
row(g, 'Video planes', 'apps, inputs and protected content on hardware planes the compositor places', ['hardware planes'], 'daily', 'Out', 'Keep', 'protected video stays on LG\'s planes', '—', '—')
