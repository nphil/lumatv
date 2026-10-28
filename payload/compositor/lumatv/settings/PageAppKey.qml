import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "AppKeys.js" as AK
import "QuickAccess.js" as QA
import "."

// The app chooser behind one of the remote's app keys (Remote & pointer › App keys; PLAN: open any installed app, or do
// nothing). Opened with `host.openPage("appkey", { key: "netflix" })`. The answer is one entry of `remote.appKeys` in the
// user's theme.json (settings/AppKeys.js has the shape), written through the host's patch like every theme choice, and the
// page then goes back to Remote & pointer. The list is the apps the TV launches (kit/Apps): LG's default first, then nothing,
// then every app by the dock's order and by name.
// The same chooser serves a number key (`{ digit: "4" }`, Quick Access, settings/QuickAccess.js): the first row is then the dock's
// own tile of that number, there is no "does nothing" (a number with no tile asks what to assign), and the answer is an entry of
// `remote.quickAccess.slots`.
PageList {
    id: page
    readonly property string keyId: arg && arg.key ? String(arg.key) : ""
    readonly property string digit: arg && arg.digit ? String(arg.digit) : ""
    readonly property var def: digit !== "" ? ({ label: "Number key " + digit }) : AK.key(keyId)
    readonly property var cfg: AK.config(Lu.theme)
    readonly property var installed: AK.installed(Apps.launchPoints)
    readonly property var now: digit !== "" ? numberNow() : AK.target(cfg, keyId, installed)
    readonly property var dockIds: Apps.dockKey === "" ? [] : Apps.dockKey.split("|")
    title: digit !== "" ? def.label : (def ? def.label + " key" : "App key")
    // (no subtitle: the title says which key, the rows say what it can open)
    back: "Remote & pointer"
    backId: "remote"

    function appById(id) { return Apps.byId(id) }
    function numberNow() {
        var t = QA.target(Lu.theme, digit, dockIds, appById)
        return { kind: t.kind === "slot" ? "app" : "default", appId: t.kind === "slot" ? t.id : "" }
    }
    function pick(choice) {
        if (!host || !def) return
        if (digit !== "") host.patch({ remote: { quickAccess: QA.chosen(Lu.theme, digit, choice) } })
        else host.patch({ remote: { appKeys: AK.chosen(cfg, keyId, choice) } })
        host.openPage("remote")
    }
    // The focus starts on the row that is in force.
    onOpened: {
        for (var i = 0; i < rows.length; i++) if (ev(rows[i].selected) === true && focusable(i)) { enter(i); return }
        enter(first())
    }

    function appRow(a) {
        return { kind: "value", id: "app:" + a.id, icon: "layout-grid", label: a.title, chevron: false,
                 selected: function() { return page.now.kind === "app" && page.now.appId === a.id },
                 act: function() { page.pick(a.id) } }
    }
    function build() {
        var out = []
        if (page.digit !== "") {
            var tile = page.dockIds[QA.tileOf(page.digit)]
            var tileApp = tile !== undefined ? Apps.byId(tile) : null
            out.push({ kind: "value", id: "default", icon: "rotate-ccw", label: "The dock's tile", chevron: false,
                       value: tileApp ? (tileApp.title || tileApp.id) : "None on the dock",
                       selected: function() { return page.now.kind === "default" },
                       act: function() { page.pick("default") } })
        } else {
            var lg = page.def ? AK.target({}, page.keyId, page.installed) : null
            out.push({ kind: "value", id: "default", icon: "rotate-ccw", label: "LG's default", chevron: false,
                       value: lg && lg.installed ? lg.title : "Not installed",
                       selected: function() { return page.now.kind === "default" },
                       act: function() { page.pick("default") } },
                     { kind: "value", id: "nothing", icon: "ban", label: "Does nothing", chevron: false,
                       selected: function() { return page.now.kind === "nothing" },
                       act: function() { page.pick("") } })
        }
        out.push(R.head("Open an app"))
        var apps = Apps.list
        for (var i = 0; i < apps.length; i++) out.push(appRow(apps[i]))
        return out
    }
    rows: build()
}
