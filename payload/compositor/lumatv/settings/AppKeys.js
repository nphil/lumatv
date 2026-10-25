.pragma library

// The Magic Remote's app keys and what each can be set to. The choice lives in the user's theme.json as
// `remote.appKeys` (the key filter in payload/keyfilter and Home read it; this page only writes it):
//   { netflix: "<app id>", amazon: "", ... }
// A key the owner never set is absent and means LG's own target, where that app is installed and nothing where it
// is not (so no key starts LG's auto-install). An app id launches that app; "" does nothing.
// `qt` is the key the stock chain delivers (StarfishKeyFilter, handleCPHotkeys), `lg` the app LG launches for it
// (docs/research/surfaces/KeyEntryPoints.json).
var keys = [
    { id: "netflix", label: "Netflix", icon: "clapperboard", qt: "Key_webOS_Netflix", lg: "netflix" },
    { id: "amazon", label: "Prime Video", icon: "play", qt: "Key_webOS_Amazon", lg: "amazon" },
    { id: "disneyplus", label: "Disney+", icon: "sparkles", qt: "Key_webOS_Disneyplus", lg: "com.disney.disneyplus-prod" },
    { id: "lgchannels", label: "LG Channels", icon: "tv-minimal-play", qt: "Key_webOS_LGChannels", lg: "com.webos.app.lgchannels" }
]

function key(id) {
    for (var i = 0; i < keys.length; i++) if (keys[i].id === id) return keys[i]
    return null
}

// The theme's `remote.appKeys` as an object, whatever the theme holds.
function config(theme) {
    var a = theme && theme.remote ? theme.remote.appKeys : null
    return a && typeof a === "object" ? a : ({})
}

// Every app the TV can launch, hidden system apps included (LG Channels is one: the library lists neither it nor its like),
// as { id, title } from the app manager's launch points. The HDMI inputs are launch points of another type.
function installed(launchPoints) {
    var out = []
    for (var i = 0; launchPoints && i < launchPoints.length; i++) {
        var p = launchPoints[i]
        if ((p.lptype || "default") === "default" && p.id) out.push({ id: p.id, title: p.title || p.id })
    }
    return out
}

// What the key does now: { kind: "default"|"nothing"|"app", appId, installed, title } with `apps` from installed().
function target(cfg, id, apps) {
    var k = key(id)
    var set = cfg[id]
    var appId = set === undefined ? (k ? k.lg : "") : String(set)
    var found = null
    for (var i = 0; appId !== "" && apps && i < apps.length; i++) if (apps[i].id === appId) { found = apps[i]; break }
    return { kind: set === undefined ? "default" : appId === "" ? "nothing" : "app", appId: appId, installed: found !== null, title: found ? found.title : "" }
}

// The whole `appKeys` object after the owner picked `choice` for key `id`: "default" removes the entry, "" is nothing, anything else an app id.
function chosen(cfg, id, choice) {
    var out = {}
    for (var k in cfg) if (k !== id) out[k] = cfg[k]
    if (choice !== "default") out[id] = choice
    return out
}
