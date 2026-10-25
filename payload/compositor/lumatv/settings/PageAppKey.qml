import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "AppKeys.js" as AK
import "."

// The app chooser behind one of the remote's app keys (Remote & pointer › App keys; PLAN: open any installed app, or do
// nothing). Opened with `host.openPage("appkey", { key: "netflix" })`. The answer is one entry of `remote.appKeys` in the
// user's theme.json (settings/AppKeys.js has the shape), written through the host's patch like every theme choice, and the
// page then goes back to Remote & pointer. The list is the apps the TV launches (kit/Apps): LG's default first, then nothing,
// then every app by the dock's order and by name.
PageList {
    id: page
    readonly property string keyId: arg && arg.key ? String(arg.key) : ""
    readonly property var def: AK.key(keyId)
    readonly property var cfg: AK.config(Lu.theme)
    readonly property var installed: AK.installed(Apps.launchPoints)
    readonly property var now: AK.target(cfg, keyId, installed)
    title: def ? def.label + " key" : "App key"
    subtitle: "Opens one app, or nothing"
    back: "Remote & pointer"
    backId: "remote"

    function pick(choice) {
        if (!host || !def) return
        host.patch({ remote: { appKeys: AK.chosen(cfg, keyId, choice) } })
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
        var lg = page.def ? AK.target({}, page.keyId, page.installed) : null
        var out = [
            { kind: "value", id: "default", icon: "rotate-ccw", label: "LG's default", chevron: false,
              detail: lg && lg.installed ? "Opens " + lg.title + "." : "LG's app is not installed here, so the key does nothing.",
              selected: function() { return page.now.kind === "default" },
              act: function() { page.pick("default") } },
            { kind: "value", id: "nothing", icon: "ban", label: "Does nothing", chevron: false, detail: "The key is ignored.",
              selected: function() { return page.now.kind === "nothing" },
              act: function() { page.pick("") } },
            R.head("Open an app")
        ]
        var apps = Apps.list
        for (var i = 0; i < apps.length; i++) out.push(appRow(apps[i]))
        return out
    }
    rows: build()
}
