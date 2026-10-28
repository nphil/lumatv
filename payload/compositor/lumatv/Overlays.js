.pragma library

// The queue behind the overlay cards (panels/Overlays.qml): which of the things that want the person's eye now are on the screen, in what
// order, for how long. Pure state and numbers, time an argument, so every rule is a host test (tests/overlays.test.js).
//
// What shows is a doorbell, an alarm or a timer that finished, a Home Assistant alert, a score. They come as the plugin broker's records
// (the helper's `plugin.<id>.notifications` and the cards at the `ringing` priority: the records LumaShow's devices draw too), and the
// camera pop-up (panels/Cameras.qml) is the doorbell's own card: this queue does not draw it, it makes room for it.
//
// The rules (docs/research/overlays-result.md):
//   - at most `maxVisible` cards (2), the most important class first, and only cards of the most important class that is waiting: while a
//     doorbell or an alarm is up, an alert or a score waits (and goes when it has gone, if it is still in date); a score never shows beside
//     another score;
//   - no card ends below `limit` (810, 75 % of the height): a second card that would not fit waits; the first always shows;
//   - nothing shows while the host is busy (a PIN, an alert, the keyboard, a panel): the cards wait and come back if still in date;
//   - a card has a time on the screen (its class's `dwellMs`, counted while it is shown), a record's own end (`expiresAt`) and a time it
//     may wait (`waitMs`); a card the person dismissed is not shown again unless the record changed;
//   - the keys are the person's own until a card has been up `armMs`: a press made as it arrives was meant for the app.
//
// Numbers are the theme's (`overlays` in theme.json); DEFAULTS is what a theme that says nothing gets, and a value of the wrong kind is
// not taken.

var CLASSES = ["doorbell", "alarm", "timer", "alert", "score"]
var MAX_QUEUE = 24               // cards waiting or shown, all classes

var DEFAULTS = {
    enabled: false,
    maxVisible: 2,
    top: 134,                     // where the first card starts: below the status bar's row (54 + 60 + 20), as the camera pop-up
    right: 96,                    // from the right edge: the 5 % safe area
    width: 560,                   // as the camera pop-up and the toasts
    gap: 16,                      // between two cards
    limit: 810,                   // no card ends below this: three quarters of 1080, so the film's subtitles and the volume bar keep the rest
    armMs: 600,
    mergeMs: 20000,               // a doorbell record and a camera's doorbell pop-up within this are one ring
    classes: {
        doorbell: { priority: 0, dwellMs: 15000, waitMs: 20000, icon: "bell-ring", tone: "accent" },
        alarm: { priority: 0, dwellMs: 60000, waitMs: 120000, icon: "alarm-clock", tone: "warning" },
        timer: { priority: 0, dwellMs: 30000, waitMs: 60000, icon: "timer", tone: "accent" },
        alert: { priority: 1, dwellMs: 10000, waitMs: 120000, icon: "circle-alert", tone: "accent" },
        score: { priority: 2, dwellMs: 8000, waitMs: 30000, icon: "trophy", tone: "accent", cooldownMs: 30000 }
    }
}

function num(v, d, lo, hi) { return typeof v === "number" && isFinite(v) && v >= lo && v <= hi ? v : d }

// The settings in force: the theme's `overlays` over DEFAULTS, a number that is not one dropped.
function config(t) {
    t = t || {}
    var c = {
        enabled: t.enabled === true,
        maxVisible: Math.round(num(t.maxVisible, DEFAULTS.maxVisible, 1, 3)),
        top: num(t.top, DEFAULTS.top, 0, 540),
        right: num(t.right, DEFAULTS.right, 0, 480),
        width: num(t.width, DEFAULTS.width, 320, 900),
        gap: num(t.gap, DEFAULTS.gap, 0, 64),
        limit: num(t.limit, DEFAULTS.limit, 360, 1080),
        armMs: num(t.armMs, DEFAULTS.armMs, 0, 5000),
        mergeMs: num(t.mergeMs, DEFAULTS.mergeMs, 0, 120000),
        classes: {}
    }
    for (var i = 0; i < CLASSES.length; i++) {
        var name = CLASSES[i], d = DEFAULTS.classes[name], o = (t.classes && t.classes[name]) || {}
        c.classes[name] = {
            priority: Math.round(num(o.priority, d.priority, 0, 9)),
            dwellMs: num(o.dwellMs, d.dwellMs, 500, 600000),
            waitMs: num(o.waitMs, d.waitMs, 0, 3600000),
            cooldownMs: num(o.cooldownMs, d.cooldownMs || 0, 0, 3600000),
            icon: typeof o.icon === "string" && o.icon !== "" ? o.icon : d.icon,
            tone: o.tone === "warning" || o.tone === "danger" || o.tone === "accent" ? o.tone : d.tone
        }
    }
    return c
}

// ---- what a record is -------------------------------------------------------------------------------------------------------------
// The class of a record, from what a plugin says of it: an explicit `kind` (the proposal for LumaShow's plugin API: on a channel and on
// a post) when it has one, else by what the plugin called its channel and its icon, else an alert.
function base(name) { return String(name || "").split("/").pop().toLowerCase() }
function classOf(rec) {
    var post = rec.post || rec.card || {}
    var explicit = rec.kind || post.kind || rec.channelKind
    if (CLASSES.indexOf(explicit) >= 0) return explicit
    var ch = base(rec.channel), icon = base(rec.channelIcon || (post.content && post.content.icon) || (rec.content && rec.content.icon))
    if (/^(doorbell|door-bell|ring|rings|visitor)$/.test(ch) || /^(doorbell|bell-ring)$/.test(icon)) return "doorbell"
    if (/^(alarm|alarms|wake|wakeup)$/.test(ch) || /^(alarm|alarm-clock)$/.test(icon)) return "alarm"
    if (/^(timer|timers|countdown)$/.test(ch) || /^(timer|hourglass)$/.test(icon)) return "timer"
    if (/^(score|scores|sport|sports|game|games|match|matches)$/.test(ch) || /^(trophy|score|sports)$/.test(icon)) return "score"
    return "alert"
}
function toneOf(rec, cls, cfg) {
    var post = rec.post || {}
    var level = post.level || rec.level
    if (level === "critical") return "danger"
    if (level === "warning") return "warning"
    var role = rec.card && rec.card.content ? rec.card.content.role : ""
    if (role === "danger") return "danger"
    if (role === "warning") return "warning"
    return cfg.classes[cls].tone
}

function secs(v) { return typeof v === "number" && v > 0 ? v * 1000 : 0 }
function cleanScore(s) {
    // the structured score of the proposal (`post.score`): two sides with a name and a score, a competition, a clock. Anything else is the title.
    if (!s || typeof s !== "object") return null
    function side(x) { return x && typeof x === "object" && (typeof x.score === "number" || typeof x.score === "string") ? { name: String(x.name || x.abbr || ""), abbr: String(x.abbr || ""), score: String(x.score) } : null }
    var home = side(s.home), away = side(s.away)
    if (!home || !away) return null
    return { league: String(s.league || ""), status: String(s.status || ""), live: s.live === true, home: home, away: away }
}

// A notification record (`plugin.<id>.notifications`, as PluginHub.notes lists them) as an entry; null when it is not one for the screen
// (a record of `low` importance is the drawer's, and so is one with nothing to say).
function fromNote(n, cfg, now) {
    // PluginHub flattens a record (`title`, `body`, `actions` at the top); a raw one keeps them in `post`: either is read
    var post = n.post || {}
    if (n.importance === "low") return null
    var title = String(n.title !== undefined ? n.title : post.title || "")
    var body = String(n.body !== undefined ? n.body : post.body || "")
    if (title === "" && body === "") return null
    var kind = n.kind || post.kind
    var level = n.level || post.level
    var privacy = n.privacy || post.privacy
    var cls = classOf({ kind: kind, channel: n.channel, channelIcon: n.channelIcon, channelKind: n.channelKind })
    var spec = cfg.classes[cls]
    var secret = privacy === "secret"
    var dwell = spec.dwellMs * (level === "critical" ? 2 : 1)
    return {
        key: "n:" + n.pluginId + "/" + n.id, src: "note", pluginId: n.pluginId, id: n.id, revision: n.revision | 0,
        cls: cls, priority: spec.priority, icon: spec.icon, iconName: String(n.channelIcon || ""),
        tone: toneOf({ level: level }, cls, cfg),
        title: secret ? String(n.channelTitle || title) : title, body: secret ? "" : body, source: String(n.channelTitle || n.plugin || ""),
        actions: (n.actions || post.actions || []).slice(0, 2), score: cls === "score" ? cleanScore(n.score || post.score) : null,
        arrivedAt: now, expiresAt: secs(n.expiresAt), dwellMs: dwell, waitMs: spec.waitMs,
        shownAt: 0, shownMs: 0, armAt: 0, bornAt: 0, hiddenAt: 0, visible: false
    }
}

// A card at the `ringing` priority (PluginHub.cards): something is ringing now. It is up while the card exists; the plugin takes it away.
function fromCard(c, cfg, now) {
    if (c.priority !== "ringing") return null
    var k = c.content || {}
    var cls = classOf({ kind: c.kind, channel: c.id, post: c })
    if (cls === "alert") cls = "alarm"                       // ringing says it is not a mere message
    var spec = cfg.classes[cls]
    var detail = k.detail || k.subtitle || (k.facts && k.facts.length ? k.facts[0].value : "")
    return {
        key: "c:" + c.pluginId + "/" + c.id, src: "card", pluginId: c.pluginId, id: c.id, revision: c.revision | 0,
        cls: cls, priority: spec.priority, icon: spec.icon, iconName: String(k.icon || ""), tone: toneOf({ card: c }, cls, cfg),
        title: String(k.title || c.plugin || ""), body: String(detail || ""), source: String(c.plugin || ""),
        actions: (c.actions || []).slice(0, 2), score: null,
        arrivedAt: now, expiresAt: secs(c.expiresAt), dwellMs: spec.dwellMs, waitMs: spec.waitMs,
        shownAt: 0, shownMs: 0, armAt: 0, bornAt: 0, hiddenAt: 0, visible: false
    }
}

// ---- the queue --------------------------------------------------------------------------------------------------------------------
function create() { return { list: [], dismissed: {}, lastShown: {}, camera: null, seq: 0 } }

function find(q, key) {
    for (var i = 0; i < q.list.length; i++) if (q.list[i].key === key) return q.list[i]
    return null
}
function remove(q, e) {
    var i = q.list.indexOf(e)
    if (i >= 0) q.list.splice(i, 1)
}

// Takes an entry in: "new" (queued), "updated" (a record already queued or shown, changed in place), or why it was not taken: "dismissed"
// (the person dismissed this revision), "cooldown" (a score again too soon), "merged" (a doorbell the camera pop-up already shows).
function offer(q, e, cfg, now) {
    var mine = find(q, e.key)
    var seen = q.dismissed[e.key]
    if (seen !== undefined && e.revision <= seen && !mine) return "dismissed"
    if (e.cls === "doorbell" && q.camera && q.camera.kind === "doorbell" && now - q.camera.at < cfg.mergeMs) {
        q.dismissed[e.key] = e.revision                   // it is the camera's card now; the record staying in the drawer is not a second ring
        return "merged"
    }
    if (mine) {
        var changed = mine.revision !== e.revision || mine.title !== e.title || mine.body !== e.body
        mine.revision = e.revision; mine.title = e.title; mine.body = e.body; mine.source = e.source; mine.actions = e.actions
        mine.tone = e.tone; mine.score = e.score; mine.expiresAt = e.expiresAt; mine.iconName = e.iconName
        // a score that moved is news again: it has its time on the screen afresh, though never more than three times over
        if (changed && mine.visible && mine.cls === "score" && now - mine.bornAt < mine.dwellMs * 3) { mine.shownMs = 0; mine.shownAt = now }
        return "updated"
    }
    var spec = cfg.classes[e.cls]
    var last = q.lastShown[e.key]
    if (spec.cooldownMs > 0 && last !== undefined && now - last < spec.cooldownMs) return "cooldown"
    if (seen !== undefined) delete q.dismissed[e.key]
    e.arrivedAt = now
    e.seqNo = ++q.seq
    q.list.push(e)
    // a plugin that floods the queue does not grow it for ever: past the cap the oldest of the least important class that is not on the screen goes
    while (q.list.length > MAX_QUEUE) {
        var drop = null
        for (var i = 0; i < q.list.length; i++) {
            var c = q.list[i]
            if (c.visible || c === e) continue
            if (!drop || c.priority > drop.priority || (c.priority === drop.priority && c.seqNo < drop.seqNo)) drop = c
        }
        if (!drop) break
        remove(q, drop)
    }
    return "new"
}

// A record the plugin ended (or that expired): its card goes, and a later record of the same id is news.
function withdraw(q, key) {
    var e = find(q, key)
    if (e) remove(q, e)
    delete q.dismissed[key]
    return !!e
}

// The person dismissed it (Back, or OK after its action ran): gone, and not back for this revision.
function dismiss(q, key, now) {
    var e = find(q, key)
    if (!e) return false
    q.dismissed[key] = e.revision
    if (e.visible) q.lastShown[key] = now
    remove(q, e)
    return true
}

// The camera's pop-up showed a card of this kind: a doorbell record of the same ring is not a second card, and one already queued or
// shown is taken back into the camera's card.
function cameraShown(q, kind, cfg, now) {
    q.camera = { kind: kind, at: now }
    if (kind !== "doorbell") return 0
    var gone = 0
    for (var i = q.list.length - 1; i >= 0; i--) {
        var e = q.list[i]
        if (e.cls === "doorbell" && e.src === "note" && now - e.arrivedAt < cfg.mergeMs) { q.list.splice(i, 1); gone++ }
    }
    return gone
}

function shownFor(e, now) { return e.shownMs + (e.visible ? now - e.shownAt : 0) }
function hide(e, now) {
    if (!e.visible) return
    e.shownMs += now - e.shownAt
    e.shownAt = 0
    e.hiddenAt = now
    e.visible = false
}

// What is on the screen now. `ctx`: `dnd` (the host is busy), `camera` (the camera pop-up is up: it takes a slot and its place at the top),
// `top` (where the first card of the queue starts: the camera's bottom and the gap, else `cfg.top`) and `heightOf(entry)` (its height as it
// is drawn). Returns the visible entries, top first, and when to look again (`wakeAt`, 0 for never): the time at which a card's time is up,
// a record ends, a wait runs out or the keys become the card's (`armMs`).
function select(q, ctx, cfg, now) {
    var i, e
    for (var k in q.lastShown) if (now - q.lastShown[k] > 3600000) delete q.lastShown[k]       // a cooldown is minutes; the map does not grow for ever
    // the end of what a card may be
    for (i = q.list.length - 1; i >= 0; i--) {
        e = q.list[i]
        var over = e.expiresAt > 0 && now >= e.expiresAt
        var used = shownFor(e, now) >= e.dwellMs
        var stale = !e.visible && e.src !== "card" && e.waitMs > 0 && now - (e.hiddenAt || e.arrivedAt) >= e.waitMs
        if (over || used || stale) {
            if (e.visible) q.lastShown[e.key] = now
            // a card that had its time is done with: this revision is not news again (a ringing card that is still there is not shown a second time)
            if (used) q.dismissed[e.key] = e.revision
            q.list.splice(i, 1)
        }
    }
    var chosen = []
    if (!ctx.dnd) {
        var order = q.list.slice().sort(function(a, b) {
            if (a.priority !== b.priority) return a.priority - b.priority
            if (a.visible !== b.visible) return a.visible ? -1 : 1
            return a.seqNo - b.seqNo
        })
        var best = ctx.camera ? 0 : (order.length ? order[0].priority : 0)
        var slots = cfg.maxVisible - (ctx.camera ? 1 : 0)
        var y = ctx.camera ? ctx.top : cfg.top
        var haveScore = false
        for (i = 0; i < order.length && chosen.length < slots; i++) {
            e = order[i]
            if (e.priority > best) break
            if (e.cls === "score" && haveScore) continue
            var h = ctx.heightOf(e)
            // the first card of an empty screen always shows (a doorbell is not left unseen for its size); any other must end above the limit
            if ((chosen.length > 0 || ctx.camera) && y + h > cfg.limit) continue
            chosen.push(e)
            y += h + cfg.gap
            if (e.cls === "score") haveScore = true
        }
    }
    for (i = 0; i < q.list.length; i++) {
        e = q.list[i]
        if (chosen.indexOf(e) >= 0) {
            if (!e.visible) {
                e.visible = true
                e.shownAt = now
                e.armAt = now
                if (!e.bornAt) e.bornAt = now
                q.lastShown[e.key] = now
            }
        } else hide(e, now)
    }
    var wake = 0
    function soon(t) { if (t > now && (wake === 0 || t < wake)) wake = t }
    for (i = 0; i < q.list.length; i++) {
        e = q.list[i]
        if (e.expiresAt > 0) soon(e.expiresAt)
        if (e.visible) { soon(e.shownAt + (e.dwellMs - e.shownMs)); soon(e.armAt + cfg.armMs) }
        else if (e.src !== "card" && e.waitMs > 0) soon((e.hiddenAt || e.arrivedAt) + e.waitMs)
    }
    return { visible: chosen, wakeAt: wake }
}

// Whether the keys are the top card's: it has been up for `armMs` (a press made as it arrived was the app's).
function armed(visible, cfg, now) {
    return visible.length > 0 && visible[0].visible && now - visible[0].armAt >= cfg.armMs
}

// ---- what a card says ------------------------------------------------------------------------------------------------------------
function clip(text, max) { return text.length > max ? text.substring(0, max - 1) + "\u2026" : text }
// What OK does on a card, in one word, and the button it runs: the record's first action, else the drawer (when it can be opened here).
function okOf(e, drawerReady) {
    if (e.actions && e.actions.length) return { label: clip(String(e.actions[0].label || "OK"), 24), action: e.actions[0] }
    if (drawerReady) return { label: "Open", action: null }
    return { label: "Dismiss", action: null }
}
// The hint's two keys: OK where it does something Back does not.
function keysOf(e, drawerReady) {
    var ok = okOf(e, drawerReady)
    return ok.label === "Dismiss" ? [{ key: "OK", what: "Dismiss" }] : [{ key: "OK", what: ok.label }, { key: "Back", what: "Dismiss" }]
}
