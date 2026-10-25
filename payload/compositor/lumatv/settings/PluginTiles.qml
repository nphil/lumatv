import QtQuick 2.4
import "../kit"

// The quick panel's tiles that plugins put (PLAN, Plugins; helper topics `plugin.<id>.tiles`), as the row
// descriptors the quick panel's PageList draws. The descriptors come from PluginHub.tileRows(): a tile with an
// on/off state is a toggle, any other a button, and OK on either is PluginHub.act, a POST to the plugin's
// action route. The list is rebuilt only when the set of tiles changes (a plugin connects, a tile is added
// or withdrawn), never when a state changes: each descriptor reads its own tile live, so a toggle that flips
// redraws its row and nothing else, and the panel's focus stays where it is.
QtObject {
    id: tiles

    property var rows: []
    // pluginId/tileId/stateful of each tile: a tile that turns stateful (unknown -> on) changes its row kind.
    readonly property string signature: PluginHub.tiles.map(function(t) {
        return t.pluginId + "/" + t.id + "/" + (t.state && (t.state.type === "on" || t.state.type === "off" || t.state.type === "observed") ? "s" : "b")
    }).join(",")

    onSignatureChanged: rows = PluginHub.tileRows()
    Component.onCompleted: rows = PluginHub.tileRows()
}
