pragma Singleton
import QtQuick 2.4

// Home's widget layout as the pages and the layout editor see it. LumaHome hands over the layout it draws (the shipped layout.json under
// the owner's, merged) and the shipped one, and does what is asked of it: it writes the owner's layout.json and applies the change at
// once (LumaHome.persistLayout). A page or the editor never writes a file; it builds the layout it wants with LayoutGrid.js (every
// change there returns a new layout) and asks for it here.
QtObject {
    id: hl

    // ---- handed over by Home (Bindings)
    property var layout: ({})                  // what Home draws
    property var shipped: ({})                 // the shipped layout.json
    property bool customised: false            // the owner's layout.json says something
    property bool editing: false               // the layout editor is on screen

    // ---- what a page asks Home to do
    signal changeRequested(var layout)         // write and apply this layout
    signal resetRequested()                    // back to the shipped layout
    signal editRequested(string id)            // open the layout editor on Home, a card selected ("" none)
    function change(next) { changeRequested(next) }
    function reset() { resetRequested() }
    function edit(id) { editRequested(id || "") }
}
