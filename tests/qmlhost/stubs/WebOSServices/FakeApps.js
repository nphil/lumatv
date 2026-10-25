.pragma library

// The launch points the offline host's make-believe TV lists, shaped as `applicationManager.launchPointsList` is on the
// owner's set (a capture of the real one: docs/research/arch/x4/all.jsonl): the Apps tile first, then user and
// homebrew apps, the system apps LG shows and the ones it hides (Live TV among them), and the HDMI bookmarks, which are
// launch points too and are not apps. Icons are the mocks' (docs/mocks/img/apps), the same for the apps without art.
// 23 of them are what the library lists ("23 installed" in docs/mocks/12-all-apps).
var rows = [
    ["com.webos.app.discovery", "Apps", "inputs", { system: true, removable: false, unmovable: true, color: "#4faa6e" }],
    ["netflix", "Netflix", "netflix", {}],
    ["com.beb.plxnative", "PlxNative", "plex", {}],
    ["com.apple.appletv", "Apple TV", "appletv", {}],
    ["com.geforcenow.play", "GeForce NOW", "geforce", {}],
    ["com.crunchyroll.webapp", "Crunchyroll", "crunchyroll", {}],
    ["io.unom.punktfunk.client-webos", "Punktfunk", "homebrew", {}],
    ["com.webos.app.homeconnect", "Home Hub", "inputs", { system: true, removable: false }],
    ["com.webos.app.browser", "Web Browser", "browser", { system: true, removable: false }],
    ["com.lighting.huesync", "Hue Sync", "huesync", {}],
    ["org.webosbrew.hbchannel", "Homebrew Channel", "homebrew", {}],
    ["org.webosbrew.hyperhdr.loader", "HyperHDR", "homebrew", {}],
    ["com.amazoff.patcher", "AmazOff", "homebrew", {}],
    ["youtube.leanback.v4", "YouTube", "youtube", {}],
    ["com.limelight.webos", "Moonlight", "geforce", {}],
    ["org.webosbrew.safeupdate", "Homebrew Channel Updater", "homebrew", {}],
    ["org.webosbrew.appupdateblocker", "LG App Update Blocker", "homebrew", {}],
    ["org.webosbrew.inputhook", "LG Input Hook", "homebrew", {}],
    ["org.nphil.lumatv", "LumaTV", "@icon160", {}],
    ["org.webosbrew.bridge-64to32", "64 to 32-bit Bridge", "homebrew", {}],
    ["com.lgtvod.us.app", "LG Movies & TV", "lgmovies", {}],
    ["com.miniclip.eightballpool", "8 Ball Pool", "pool", {}],
    // LG keeps these off Home: the library drops them, except Live TV
    ["com.webos.app.livetv", "Live TV", "livetv", { system: true, removable: false, hidden: true }],
    ["com.webos.app.lifeonscreen", "LG Gallery+", "inputs", { system: true, removable: false, hidden: true }],
    ["com.webos.app.mediadiscovery", "Media Player", "inputs", { system: true, removable: false, hidden: true }],
    ["com.webos.app.camera", "Camera", "inputs", { system: true, removable: false, hidden: true }],
    ["com.webos.app.lgchannels", "LG Channels", "inputs", { system: true, removable: false, hidden: true }],
    ["com.webos.app.multiviewsettings", "Multi View", "inputs", { system: true, removable: false, hidden: true }]
]

// iconDir: the mocks' app icons; assetDir: the repository's assets. Returns the `launchPoints` array.
function launchPoints(iconDir, assetDir) {
    var out = []
    for (var i = 0; i < rows.length; i++) {
        var r = rows[i], o = r[3]
        var icon = r[2].charAt(0) === "@" ? assetDir + r[2].substring(1) + ".png" : iconDir + r[2] + ".png"
        out.push({ id: r[0], launchPointId: r[0] + "_default", title: r[1], icon: icon, largeIcon: icon, iconColor: o.color || "", bgColor: "",
                   hidden: !!o.hidden, systemApp: !!o.system, removable: o.removable !== false, lptype: "default", params: {}, unmovable: !!o.unmovable })
    }
    // the HDMI bookmarks LG keeps among the launch points: inputs, not apps
    out.push({ id: "com.webos.app.hdmi2", launchPointId: "com.webos.app.hdmi2_bookmark", title: "AVR-S760H", icon: iconDir + "inputs.png", largeIcon: iconDir + "inputs.png",
               hidden: false, systemApp: false, removable: false, lptype: "bookmark", params: {} })
    return out
}
