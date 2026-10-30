.pragma library

// The Magic Remote's app keys and what each can be set to. The choice lives in the user's theme.json as `remote.buttons`, one word per
// button (the compositor's key path applies them; this page writes the app keys' and reads the rest):
//   { netflix: "app:<app id>", amazon: "nothing", red: "quick", ... }
// A key the owner never set is absent and means LG's own target, where that app is installed and nothing where it
// is not (so no key starts LG's auto-install). "app:<id>" launches that app; "nothing" does nothing.
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

// The theme's `remote.buttons` as an object, whatever the theme holds.
function config(theme) {
    var a = theme && theme.remote ? theme.remote.buttons : null
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

// What the key does now: { kind: "default"|"nothing"|"app"|"other", appId, installed, title } with `apps` from installed(). "other" is an
// action only the compositor's own key path carries out (Go Home, the quick panel, acting as another button, ...): LumaHome swallows the key.
function target(cfg, id, apps) {
    var k = key(id)
    var word = cfg[id] === undefined ? undefined : String(cfg[id])
    var appId = word === undefined ? (k ? k.lg : "") : word.indexOf("app:") === 0 ? word.substring(4) : ""
    var found = null
    for (var i = 0; appId !== "" && apps && i < apps.length; i++) if (apps[i].id === appId) { found = apps[i]; break }
    var kind = word === undefined ? "default" : word === "nothing" ? "nothing" : appId !== "" ? "app" : "other"
    return { kind: kind, appId: appId, installed: found !== null, title: found ? found.title : "" }
}

// The whole `buttons` object after the owner picked `choice` for key `id`: "default" removes the entry, "" is nothing, anything else an app id.
function chosen(cfg, id, choice) {
    var out = {}
    for (var k in cfg) if (k !== id) out[k] = cfg[k]
    if (choice !== "default") out[id] = choice === "" ? "nothing" : "app:" + choice
    return out
}

// The buttons the page lists besides the app keys: the compositor's own key path acts on them, this page only says what each does.
var others = [
    { id: "red", label: "Red", icon: "circle" },
    { id: "green", label: "Green", icon: "circle" },
    { id: "yellow", label: "Yellow", icon: "circle" },
    { id: "blue", label: "Blue", icon: "circle" },
    { id: "chup", label: "Channel up", icon: "arrow-big-up" },
    { id: "chdown", label: "Channel down", icon: "arrow-big-up" },
    { id: "list", label: "List", icon: "list-video" },
    { id: "more", label: "More (...)", icon: "ellipsis" }
]

var goes = { home: "Goes Home", quick: "Opens the quick panel", settings: "Opens Settings", notifications: "Opens notifications",
             mute: "Mutes or unmutes", pictureoff: "Turns the picture off" }

function name(id) {
    var all = keys.concat(others)
    for (var i = 0; i < all.length; i++) if (all[i].id === id) return all[i].label
    return id.indexOf("code:") === 0 ? "Button " + id.substring(5) : id
}

// What a button does now, in words (what Remote & pointer › Buttons shows): `apps` from installed().
function describe(cfg, id, apps) {
    var word = cfg[id] === undefined ? undefined : String(cfg[id])
    if (word === undefined) {
        var k = key(id)
        if (!k) return "Not changed"
        var t = target({}, id, apps)
        return t.installed ? "Opens " + t.title : "Nothing"
    }
    if (word === "nothing") return "Does nothing"
    if (goes[word] !== undefined) return goes[word]
    if (word.indexOf("app:") === 0) {
        var app = word.substring(4)
        for (var i = 0; apps && i < apps.length; i++) if (apps[i].id === app) return "Opens " + apps[i].title
        return /^com\.webos\.app\.(hdmi\d|livetv)$/.test(app) ? "Switches to " + (app === "com.webos.app.livetv" ? "Live TV" : "HDMI " + app.slice(-1)) : "App not installed"
    }
    if (word.indexOf("as:") === 0) return "Acts as " + name(word.substring(3))
    return "Not changed"
}
