.pragma library

// Which tiles the quick panel shows and in what order (docs/mocks/29-qp-edit, 29-qp-add). The owner's arrangement is the user's theme.json
// `quickPanel.tiles`: the ids shown, top to bottom. It is written the first time the panel is edited and until then absent, which means the
// panel's own list (`dflt` tiles in the catalogue's order). A saved id that no tile answers to now (a plugin that is not connected) is not
// drawn, and a tile that is in the catalogue and not in the list is one "Add a tile" offers. Nothing here touches the theme: each function
// takes arrays and returns arrays, and `patch` is what the host writes.
//
// `all` is the catalogue: [{ id, dflt }] in the order a panel with no arrangement shows its default ones.

function shown(saved, all) {
    if (!Array.isArray(saved)) return all.filter(function(c) { return c.dflt }).map(function(c) { return c.id })
    var known = {}, seen = {}, out = []
    for (var i = 0; i < all.length; i++) known[all[i].id] = true
    for (var j = 0; j < saved.length; j++) {
        var id = String(saved[j])
        if (known[id] && !seen[id]) { seen[id] = true; out.push(id) }
    }
    return out
}
// What "Add a tile" offers: every tile of the catalogue that is not shown, in the catalogue's order.
function missing(order, all) {
    return all.filter(function(c) { return order.indexOf(c.id) < 0 }).map(function(c) { return c.id })
}
function without(order, id) { return order.filter(function(x) { return x !== id }) }
function added(order, id) { return order.indexOf(id) >= 0 ? order.slice() : order.concat([id]) }
// One place up (dir -1) or down (dir 1); at an end the order is as it was.
function moved(order, id, dir) {
    var i = order.indexOf(id), j = i + dir
    var out = order.slice()
    if (i < 0 || j < 0 || j >= out.length) return out
    var t = out[i]; out[i] = out[j]; out[j] = t
    return out
}
function same(a, b) {
    if (a.length !== b.length) return false
    for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
    return true
}
// The theme patch that keeps an arrangement. The host's patch merges one level deep, so `tiles` is the whole list every time.
function patch(order) { return { quickPanel: { tiles: order.slice() } } }
