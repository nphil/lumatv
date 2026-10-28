"""Run with: cd tests && python3 -m unittest test_ha_plugin_page   (needs Xvfb and PyQt5; skipped without them)

The rest of the Home Assistant plugin on the TV, through the offline QML host against the fake helper, with what the real plugin and
helper publish (tests/qmlhost/fixtures/ha-plugin.json). The quick panel has the plugin's tiles and a tap is the plugin's action; the
drawer has its notifications with their buttons. Its page in Settings (Home & look > Home Assistant, or OK on the lane) shows the
address and whether a token is kept and nothing else of them, forgets them on a second OK, and draws the plugin's own settings: a
picked room, a picked light (in the order picked), a switch and a number are the plugin's settings, written to the helper in their kinds.
"""
import json
import unittest

from ha_case import FOCUS, HUB, HaCase, feed  # noqa: E402
from test_plugins_surfaces import GET, js, probe, probes  # noqa: E402
from test_settings import READY  # noqa: E402
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Panels(HaCase):
    def test_the_quick_panel_has_the_plugins_tiles_and_a_tap_is_its_action(self):
        # the rows the quick panel draws after its own tiles (settings/PluginTiles asks PluginHub for them), and the panel itself open
        rows = (HUB + "var h = hub(); var q = byProp(root, 'footFocus'); var all = q.tiles().map(function(r) { return r.id });"
                "return {panel: all.filter(function(id) { return String(id).indexOf('plugin.home-assistant.') === 0 }),"
                " rows: h.rows().map(function(r) { return {id: r.id, kind: r.kind, label: r.label, icon: r.icon, detail: r.detail(), on: r.on(), enabled: r.enabled()} })};")
        rc, out = self.run_host(
            {}, *feed(), *js(READY - 600, "root.openLayer('quick')" + ";"),
            *probe(READY, "tiles", rows, find="root"),
            *probe(READY + 300, "tapped", HUB + "var r = hub().rows(); r[0].act(); r[2].act(); return 1;", find="root"),
            *js(READY + 700, GET % ("posts", "__posts")),
            "--wait", str(READY + 1300))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        tiles = probes(out)["tiles"]
        self.assertEqual(tiles["panel"], ["plugin.home-assistant.light.lamp", "plugin.home-assistant.light.shelf", "plugin.home-assistant.scene.movie"],
                         "the panel has them, after its own")
        rows = tiles["rows"]
        self.assertEqual([t["label"] for t in rows], ["Lamp", "Shelf light", "Movie night"])
        self.assertEqual([(t["kind"], t["on"]) for t in rows][:2], [("toggle", True), ("toggle", False)], "a light is a switch that shows its state")
        self.assertEqual([t["icon"] for t in rows], ["sun", "sun", "house"], "the plugin's own icon names are drawn with the closest this TV has")
        self.assertEqual([t["detail"] for t in rows], ["", "", ""], "a tile says its name and its state, not which plugin it is")
        self.assertEqual(sorted(self.plugin_posts(out)), [("/v1/plugins/home-assistant/actions/activate", {"target": "scene.movie"}),
                                                          ("/v1/plugins/home-assistant/actions/toggle", {"target": "light.lamp"})],
                         "a tap on a light is the plugin's toggle, on a scene its activate")

    def test_the_drawer_has_its_notifications_and_a_ring_can_be_cleared_from_it(self):
        go = "s.index = 1; for (var guard = 0; guard < 30 && s.items[s.index - 1].title !== %s; guard++) s.key('down');"
        rc, out = self.run_host(
            {}, *feed(),
            *js(READY - 900, "root.openLayer('drawer')"),
            *probe(READY, "shape", "return {heads: s.entries.filter(function(e) { return e.head }).map(function(e) { return e.source + ':' + e.count }), titles: s.items.map(function(i) { return i.title + '|' + i.icon }), buttons: s.items.map(function(i) { return (i.actions || []).map(function(a) { return a.label }) })};",
                   find="byProp(root, 'sheetTop')"),
            *probe(READY + 300, "toward", go % json.dumps("Doorbell") + " return s.items[s.index - 1].title;", find="byProp(root, 'sheetTop')"),
            *probe(READY + 500, "ok", "s.key('ok'); return 1;", find="byProp(root, 'sheetTop')"),
            *js(READY + 900, GET % ("posts", "__posts")),
            "--wait", str(READY + 1500))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        shape = probes(out)["shape"]
        self.assertEqual(shape["heads"], ["Home Assistant:2"], "one group, the plugin's")
        self.assertIn("Doorbell|bell", shape["titles"], "the channel's icon, drawn with the closest this TV has")
        self.assertIn("Low battery|house", shape["titles"])
        self.assertIn(["Clear"], shape["buttons"])
        self.assertEqual(probes(out)["toward"], "Doorbell")
        self.assertEqual(self.plugin_posts(out), [("/v1/plugins/home-assistant/actions/clear_doorbell", {"target": "ring-1"})])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Page(HaCase):
    # With a card in the lane OK enters the chips; with none it opens the page, which is the way these runs get there.
    NOTHING = dict(cards=False, tiles=False, notes=False)
    SHAPE = ("var p = byProp(root, 'roomAsked'); var st = byProp(root, 'railIndex');"
             " return {title: p.title, back: p.back, tabs: p.tabs, tab: p.tab, stop: p.stop, armed: p.armed, chip: p.chip, settings: root.settingsOpen, current: st.current, zone: st.zone, on: p.rows[p.stop] ? p.rows[p.stop].label : '',"
             " rows: p.rows.map(function(r, i) { return {kind: r.kind, tab: r.tab || 0, label: r.label || r.text || '', shown: p.shown(i),"
             " value: typeof r.value === 'function' ? r.value() : (r.value || ''), on: typeof r.on === 'function' ? r.on() : null,"
             " num: typeof r.num === 'function' ? r.num() : null,"
             " selected: typeof r.selected === 'function' ? r.selected() : null, detail: typeof r.detail === 'function' ? r.detail() : (r.detail || ''),"
             " button: typeof r.button === 'function' ? r.button() : (r.button || ''), danger: r.danger === true} }).filter(function(r) { return r.shown && r.kind !== 'note' })};")
    # Home & look: the row that opens the page, found by what it says
    FIND = ("function look(p) { if (p.title === 'Home & look' && p.rows !== undefined) return p; var k = p.children; for (var i = 0; i < k.length; i++) { var f = look(k[i]); if (f) return f } return null }"
            "var pg = look(root); var r = pg ? pg.rows.filter(function(r) { return r.id === 'homeassistant' })[0] : null; var st = byProp(root, 'railIndex');")
    LOOK = FIND + "return {found: !!pg, current: st.current, row: r ? {label: r.label, value: r.value(), detail: r.detail || ''} : null};"
    OPEN = FIND + "r.act(); return 1;"

    def labels(self, shape):
        return [(r["label"], r["value"]) for r in shape["rows"]]

    def test_the_page_shows_the_address_and_whether_a_token_is_kept_and_forgets_them_on_a_second_ok(self):
        rc, out = self.run_host(
            {}, *feed(**self.NOTHING), *js(READY - 600, FOCUS),
            *self.keys(READY, "ok"), *probe(READY + 2300, "opened", self.SHAPE, find="root"),
            *self.keys(READY + 2400, "down", "down"), *probe(READY + 2700, "toforget", self.SHAPE, find="root"),
            *self.keys(READY + 2800, "ok"), *probe(READY + 3100, "armed", self.SHAPE, find="root"),
            *self.keys(READY + 3200, "ok"), *probe(READY + 4000, "forgotten", self.SHAPE, find="root"),
            *self.keys(READY + 4100, "back"), *probe(READY + 4600, "look", self.LOOK, find="root"),
            *probe(READY + 4700, "open", self.OPEN, find="root"),
            *probe(READY + 5500, "again", "var pg = byProp(root, 'roomAsked'); var st = byProp(root, 'railIndex'); return {title: pg.title, current: st.current};", find="root"),
            *js(READY + 5600, GET % ("posts", "__posts")),
            "--wait", str(READY + 6200))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertTrue(p["opened"]["settings"], "OK on the lane opens Settings")
        self.assertEqual((p["opened"]["current"], p["opened"]["zone"]), ("homeassistant", "page"), "at the page, with the focus in it")
        self.assertEqual((p["opened"]["title"], p["opened"]["back"]), ("Home Assistant", "Home & look"))
        self.assertEqual(p["opened"]["tabs"], ["Connection", "Room", "Doorbell", "Presence", "Alerts"])
        self.assertEqual(p["opened"]["chip"], "Connected")
        rows = dict(self.labels(p["opened"]))
        self.assertEqual(rows["Address"], "http://192.168.1.146:8123")
        self.assertEqual(rows["Access token"], "Saved", "whether one is kept, and never what it is")
        self.assertNotIn("eyJ", json.dumps(p["opened"]))
        self.assertEqual(rows["Connection"], "Connected to Home Assistant 2026.9.4", "the plugin's own words for the link, as the value of its row")
        self.assertEqual([r["label"] for r in p["opened"]["rows"] if r["detail"]], [], "a row says its name and its value, nothing under it")
        forget = [r for r in p["opened"]["rows"] if r["button"]]
        self.assertEqual([(r["label"], r["button"], r["danger"]) for r in forget], [("Forget link", "Forget", True)], "what deletes is drawn as danger")
        self.assertEqual(p["opened"]["on"], "Address", "the first row that can be focused")
        self.assertEqual(p["toforget"]["on"], "Forget link")
        self.assertFalse(p["opened"]["armed"])
        self.assertTrue(p["armed"]["armed"], "one OK on Forget only asks again")
        self.assertEqual([r["button"] for r in p["armed"]["rows"] if r["button"]], ["Sure?"])
        self.assertEqual(self.plugin_posts(out), [("/v1/plugins/home-assistant/actions/lease.ha.forget", {})], "the second OK forgets")
        self.assertFalse(p["forgotten"]["armed"])
        after = dict(self.labels(p["forgotten"]))
        self.assertEqual((after["Address"], after["Access token"]), ("Not set", "Not saved"))
        self.assertEqual(p["forgotten"]["chip"], "Not linked")
        self.assertEqual((p["look"]["found"], p["look"]["current"]), (True, "look"), "Back is one step up, to Home & look")
        self.assertEqual(p["look"]["row"], {"label": "Home Assistant", "value": "Not linked", "detail": ""}, "where the page is, with where it stands")
        self.assertEqual((p["again"]["title"], p["again"]["current"]), ("Home Assistant", "homeassistant"), "OK on that row opens the page")

    PICK = ("var p = byProp(root, 'roomAsked'); return {tab: p.tab, rows: p.rows.map(function(r, i) { return {kind: r.kind, tab: r.tab || 0, label: r.label || r.text || '', shown: p.shown(i),"
            " selected: typeof r.selected === 'function' ? r.selected() : null, value: typeof r.value === 'function' ? r.value() : (r.value || ''), on: typeof r.on === 'function' ? r.on() : null,"
            " num: typeof r.num === 'function' ? r.num() : null} }).filter(function(r) { return r.shown && r.kind !== 'note' })};")

    def test_a_room_and_its_lights_are_picked_in_order_and_written_in_the_plugins_kinds(self):
        rc, out = self.run_host(
            {}, *feed(**self.NOTHING), *js(READY - 600, FOCUS),
            *self.keys(READY, "ok"),
            # the Room tab: Up to the tab bar, Right, Down into the first row. The rooms, then the room's lights and scenes.
            *self.keys(READY + 2300, "up", "right", "down", "down"), *probe(READY + 2800, "room", self.PICK, find="root"),
            *self.keys(READY + 2900, "ok"),                                      # Kitchen (the title above it has a help mark, which takes the first Down)
            *self.keys(READY + 3300, "down", "down", "down", "ok", "down", "ok"),   # Living room, the next title, then Lamp picked, Movie night picked
            *probe(READY + 4200, "lights", self.PICK, find="root"),
            *js(READY + 4300, GET % ("posts", "__posts")),
            "--wait", str(READY + 4900))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["room"]["tab"], 1)
        pickers = [(r["label"], r["selected"]) for r in p["room"]["rows"] if r["selected"] is not None]
        self.assertEqual(pickers[:2], [("Kitchen", False), ("Living room", True)], "the rooms the plugin listed, the chosen one checked")
        self.assertEqual([l for l, _ in pickers[2:]], ["Lamp", "Movie night", "Shelf light"], "then its lights and scenes, in the plugin's order")
        self.assertEqual(self.settings_posts(out), [
            {"changes": [{"key": "room", "value": {"type": "choice", "value": "kitchen"}}]},
            {"changes": [{"key": "controls", "value": {"type": "ordered", "value": ["light.lamp"]}}]},
            {"changes": [{"key": "controls", "value": {"type": "ordered", "value": ["light.lamp", "scene.movie"]}}]},
        ], "a room is a choice; a light picked is the first of the list, and the next after it: the order picked")
        lights = [(r["label"], r["selected"], r["value"]) for r in p["lights"]["rows"] if r["selected"] is not None]
        self.assertIn(("Lamp", True, "1"), lights)
        self.assertIn(("Movie night", True, "2"), lights, "the number is the place in the order it was picked")

    def test_a_doorbell_a_switch_and_a_number_are_written_in_their_kinds(self):
        rc, out = self.run_host(
            {}, *feed(**self.NOTHING), *js(READY - 600, FOCUS),
            *self.keys(READY, "ok"),
            *self.keys(READY + 2300, "up", "right", "right", "down", "down"),    # Doorbell: under its title, the doorbell picked is the first row
            *probe(READY + 2800, "doorbell", self.PICK, find="root"),
            *self.keys(READY + 2900, "ok", "down", "down", "down", "ok"),        # take it off, then the switch below the two camera pickers
            *self.keys(READY + 3500, "up", "up", "up", "up", "up", "right", "down", "down", "right"),   # Presence: no sensors in this room; the number, +10 s
            *probe(READY + 4200, "presence", self.PICK, find="root"),
            *self.keys(READY + 4300, "up", "up", "right", "down", "ok"),         # Alerts: the switch
            *js(READY + 4800, GET % ("posts", "__posts")),
            "--wait", str(READY + 5400))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual([(r["label"], r["selected"], r["value"]) for r in p["doorbell"]["rows"] if r["selected"] is not None], [("Front doorbell", True, "1")])
        self.assertEqual([(r["label"], r["num"]) for r in p["presence"]["rows"] if r["kind"] == "slider"], [("Still here for", 70)])
        self.assertEqual(self.settings_posts(out), [
            {"changes": [{"key": "doorbell", "value": {"type": "ordered", "value": []}}]},
            {"changes": [{"key": "notify_doorbell", "value": {"type": "bool", "value": False}}]},
            {"changes": [{"key": "hold", "value": {"type": "integer", "value": 70}}]},
            {"changes": [{"key": "notify_ha", "value": {"type": "bool", "value": False}}]},
        ], "a second OK on a picked one takes it off; a switch is a bool; a number is an integer in steps of its own")


if __name__ == "__main__":
    unittest.main()
