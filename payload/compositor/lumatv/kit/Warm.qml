pragma Singleton
import QtQuick 2.4

// What the surfaces need hot before the first time they show. A surface calls Warm.request() when
// it loads: text styles it draws in (weight, size, spacing), image URLs, and components that carry
// a shader of their own. The host in LumaHome (Warmup.qml) draws each once, hidden, in the seconds
// after Home has presented, and keeps the items alive so Qt does not drop the glyph and pixmap
// caches again. A surface built from the kit's roles and shared quads needs no request: the roles
// are warmed by default and the shared shaders by the host.
QtObject {
    id: warm

    property var texts: []          // [{weight, size, spacing}]
    property var images: []         // [url]
    property var components: []     // [Component]
    signal requested()

    function request(spec) {
        var changed = false
        if (spec.texts) {
            var t = texts.slice()
            for (var i = 0; i < spec.texts.length; i++) {
                var s = spec.texts[i]
                if (!has(t, s)) { t.push({ weight: s.weight, size: s.size, spacing: s.spacing || 0 }); changed = true }
            }
            if (changed) texts = t
        }
        if (spec.images) {
            var im = images.slice(), n = im.length
            for (var j = 0; j < spec.images.length; j++) if (im.indexOf(spec.images[j]) < 0) im.push(spec.images[j])
            if (im.length !== n) { images = im; changed = true }
        }
        if (spec.components) {
            var cs = components.slice(), m = cs.length
            for (var k = 0; k < spec.components.length; k++) if (cs.indexOf(spec.components[k]) < 0) cs.push(spec.components[k])
            if (cs.length !== m) { components = cs; changed = true }
        }
        if (changed) requested()
    }
    function has(list, s) {
        for (var i = 0; i < list.length; i++)
            if (list[i].weight === s.weight && list[i].size === s.size && list[i].spacing === (s.spacing || 0)) return true
        return false
    }
}
