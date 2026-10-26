"""Run with: python3 -m unittest discover -s tests -p 'test_quick_panels.py'   (needs Xvfb and PyQt5; skipped without them)

LumaTV's quick panel (the gear's tap) and input picker (the Input key) in place of LG's, through the offline QML host: Home's side
(HomePanels.qml, asked through LumaBus.js as the key filter's second entry does) and the overlay's over an app (`--overlay`). The key
filter's own side is `tests/keyfilter.test.js` and `tests/bus.test.js`. What these check: the panels open, close and move on when asked,
that the panel slides from the left edge to where LG's Quick Settings sits (492 x 1008, 24 from the edges, measured on LG's own panel)
and stays drawn until it has gone, that nothing is built when one opens, what each row and button does, and that a panel LG should
have is not taken. Nothing here measures time: the set's gate does (docs/research/perf/gate.py, surfaces quick and picker).
"""
import json
import re
import unittest

from test_settings import HostCase, READY
from xdisplay import DISPLAY, HAVE_QT, own_tmp

FIND = """function find(p, name) {
    if (p[name] !== undefined && p.children !== undefined && p.visible !== undefined) return p;
    for (var i=0; i<p.children.length; i++) { var f=find(p.children[i], name); if (f) return f; }
    return null;
}
function nodes(p) { var n = 1; for (var i=0; i<p.children.length; i++) n += nodes(p.children[i]); return n; }"""
# LumaBus.js as the key filter's bridge reaches it: the same library instance Home and the overlay registered with
BUS = ("function bus() { return Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'LumaBus.js\" as Bus\\n"
       "QtObject { function ready(k) { return Bus.panelReady(k) } function open(k) { return Bus.panel(k) } function take() { return Bus.takeSettings() }"
       " function want(p) { Bus.wantSettings(p) } }', root) }")
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { u: c.uri, m: c.method, p: p } }))"
PLAIN = '{"perf":{"syncResidents":true}}'
REDUCED = '{"perf":{"syncResidents":true},"motion":{"reduced":true}}'
OFF = '{"perf":{"syncResidents":true},"remote":{"quickPanel":false,"inputPicker":false}}'


def probe(at, label, body, root="root"):
    """An eval at `at` ms that logs `[P:label] <json>` of what `body` returns; the helper functions above are in scope."""
    js = "(function(){%s%s var r=(function(){%s})(); console.info('[P:%s] '+JSON.stringify(r)); return 1;})()" % (FIND, BUS, body, label)
    return ["--eval", "%d:%s" % (at, js)]


def probes(out):
    return {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[P:(\w+)\] (.*)", out)}


def launches(out_value):
    return [(c["u"].rsplit("/", 1)[-1], c["m"], c["p"]) for c in json.loads(out_value)]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class HomePanels(HostCase):
    """Home in front: the key filter's second entry asks the bus, the bus asks HomePanels."""

    def test_the_gear_shows_the_quick_panel_where_LGs_was_and_the_gear_again_slides_it_away(self):
        panel = "var s = find(root, 'footFocus');"
        rc, out = self.host(
            *probe(READY + 300, "ready", "var b = bus(); return [b.ready('quick'), b.ready('picker'), b.ready('nothing')];"),
            *probe(READY + 400, "nodes0", panel + "return nodes(s);"),
            *probe(READY + 500, "open", "var b = bus(); var r = b.open('quick'); return [r, root.layerOpen];"),
            *probe(READY + 1300, "shown", panel + "return {open: root.layerOpen, presented: s.presented, progress: s.progress, x: s.slideX, y: s.slideY, w: s.sheetW, h: s.sheetH, "
                   "visible: s.visible, nodes: nodes(s)};"),
            *probe(READY + 1400, "close", "var b = bus(); var r = b.open('quick'); var s = find(root, 'footFocus'); return [r, root.layerOpen, s.leaving, s.visible];"),
            *probe(READY + 2200, "gone", panel + "return {leaving: s.leaving, progress: s.progress, visible: s.visible, presented: s.presented};"),
            "--wait", str(READY + 2700))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["ready"], [True, True, False], out[-2500:])
        self.assertEqual(p["open"], [True, "quick"])
        # it is LG's Quick Settings panel in its place and size: 492 by 1008, 24 from the left and top edges (qs_res_warm.png)
        s = p["shown"]
        self.assertEqual((s["open"], s["presented"], s["progress"], s["visible"]), ("quick", True, 1, True), out[-2500:])
        self.assertEqual((s["x"], s["y"], s["w"], s["h"]), (24, 24, 492, 1008))
        # nothing was created when it opened
        self.assertEqual(s["nodes"], p["nodes0"], "the panel built something when it opened")
        # shown again, the gear hides it: it is told to go at once and stays drawn while it slides back, then it is away
        self.assertEqual(p["close"][:2], [True, ""])
        self.assertEqual(p["close"][2:], [True, True], "the panel was cut off instead of sliding away")
        self.assertEqual(p["gone"], {"leaving": False, "progress": 0, "visible": False, "presented": False})

    def test_reduced_motion_is_a_fade_with_no_travel(self):
        panel = "var s = find(root, 'footFocus');"
        rc, out = self.host(
            "--theme", REDUCED,
            *probe(READY + 500, "open", "var b = bus(); return b.open('quick');"),
            *probe(READY + 520, "early", panel + "return {x: s.slideX, progress: s.progress};"),
            *probe(READY + 1300, "shown", panel + "return {x: s.slideX, progress: s.progress};"),
            "--wait", str(READY + 1700))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["early"]["x"], 24, "the sheet travelled in reduced motion")
        self.assertEqual(p["shown"], {"x": 24, "progress": 1})

    def test_the_input_key_shows_the_picker_and_the_key_again_goes_on_to_the_next_input(self):
        panel = "var s = find(root, 'soundLine');"
        rc, out = self.host(
            *probe(READY + 400, "nodes0", panel + "return nodes(s);"),
            *probe(READY + 500, "open", "var b = bus(); return [b.open('picker'), root.layerOpen];"),
            *probe(READY + 1300, "at", panel + "return {index: s.index, n: s.tiles.length, nodes: nodes(s), y: s.sheetY, progress: s.progress};"),
            *probe(READY + 1400, "next", "var b = bus(); var s = find(root, 'soundLine'); var i = s.index; b.open('picker'); return [i, s.index, root.layerOpen];"),
            *probe(READY + 1500, "wrap", "var b = bus(); var s = find(root, 'soundLine'); for (var i = 0; i < s.tiles.length; i++) b.open('picker'); return [s.index, root.layerOpen];"),
            "--wait", str(READY + 2000))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["open"], [True, "picker"])
        self.assertGreaterEqual(p["at"]["n"], 2)
        self.assertEqual((p["at"]["progress"], p["at"]["y"]), (1, 612), "the sheet rests 64 px above the bottom edge")
        self.assertEqual(p["at"]["nodes"], p["nodes0"], "the picker built something when it opened")
        self.assertEqual(p["next"][1], (p["next"][0] + 1) % p["at"]["n"])
        self.assertEqual(p["next"][2], "picker")
        self.assertEqual(p["wrap"], [p["next"][1], "picker"], "going round all the tiles comes back to the same one")

    def test_the_open_path_asks_for_no_reading_until_the_slide_is_over_and_the_slide_is_short(self):
        # The set's gate wants the frame burst a key starts to end within 180 ms of it, and the first frame comes about 41 ms after the gear:
        # the slide is at most 110 ms, and nothing that rebinds the rows is asked for while it runs (the answers came mid-slide).
        asked = "root.lunaCalls.filter(function(c) { return c.method === '/getAllInputStatus' }).length"
        for kind, find in (("quick", "footFocus"), ("picker", "soundLine")):
            rc, out = self.host(
                *probe(READY + 500, "before", "var b = bus(); var n = %s; b.open('%s'); return n;" % (asked, kind)),
                *probe(READY + 560, "during", "return %s;" % asked),
                *probe(READY + 700, "done", "var s = find(root, '%s'); return {n: %s, progress: s.progress};" % (find, asked)),
                *probe(READY + 1000, "after", "return %s;" % asked),
                "--wait", str(READY + 1400))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertEqual(p["during"], p["before"], "the %s asked for a reading while it was sliding in" % kind)
            self.assertEqual(p["done"]["progress"], 1, "the %s slide was not over 200 ms after it opened" % kind)
            self.assertGreater(p["after"], p["before"], "the %s never read the TV after it opened" % kind)

    def test_one_panel_gives_way_to_the_other_and_to_what_was_up(self):
        rc, out = self.host(
            *probe(READY + 400, "a", "var b = bus(); b.open('quick'); return root.layerOpen;"),
            *probe(READY + 800, "b", "var b = bus(); b.open('picker'); return [root.layerOpen, root.layerStack.length];"),
            *probe(READY + 1200, "c", "var b = bus(); root.openSettings(); b.open('quick'); return [root.layerOpen, root.settingsOpen, root.layerStack.length];"),
            "--wait", str(READY + 1700))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["a"], "quick")
        self.assertEqual(p["b"], ["picker", 0], "the picker replaced the quick panel")
        self.assertEqual(p["c"], ["quick", False, 0], "Settings gave way to the panel")

    def test_nothing_that_covers_the_screen_fades_while_a_panel_slides(self):
        # Each change of a full-screen item's opacity made Qt's renderer batch the whole scene under it again, every frame of the slide
        # (round 11 over Home: quick p90 3.70 ms). The scrim is whole from the first frame; only the sheet moves and fades.
        cover = ("var c = s.children, f = []; for (var i = 0; i < c.length; i++) if (c[i].visible && c[i].width >= root.width "
                 "&& c[i].height >= root.height) f.push(c[i].opacity); return {drawn: s.visible, progress: s.progress, full: f};")
        for kind, find in (("quick", "footFocus"), ("picker", "soundLine")):
            # The software renderer's first frame of a panel can outlast the slide; a probe before the slide has ended is enough (the faded
            # scrim drew at the opacity of the progress, 0 at the start).
            steps = list(probe(READY + 500, "open", "return bus().open('%s');" % kind))
            for k in range(16):
                steps += probe(READY + 510 + 25 * k, "mid%d" % k, "var s = find(root, '%s'); " % find + cover)
            rc, out = self.host(*steps, "--wait", str(READY + 1000))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertTrue(p["open"])
            drawn = [m for m in (p["mid%d" % k] for k in range(16)) if m["drawn"]]
            self.assertTrue(any(m["progress"] < 1 for m in drawn), "no probe saw the %s before its slide ended: %s" % (kind, drawn))
            for m in drawn:
                self.assertTrue(m["full"], "the %s has no full-screen scrim" % kind)
                self.assertEqual(set(m["full"]), {1}, "a full-screen item of the %s faded at progress %s" % (kind, m["progress"]))

    def test_a_panel_is_not_shown_when_the_owner_turned_it_off_or_Home_is_not_in_front(self):
        rc, out = self.host(
            "--theme", OFF,
            *probe(READY + 300, "off", "var b = bus(); return [b.ready('quick'), b.ready('picker'), b.open('quick'), b.open('picker'), root.layerOpen];"),
            "--wait", str(READY + 700))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["off"], [False, False, False, False, ""], "LG's windows are the key's when the switches are off")
        rc, out = self.host(
            *probe(READY + 300, "hidden", "root.hostActive = false; var b = bus(); return [b.ready('quick'), b.open('quick'), root.layerOpen];"),
            "--wait", str(READY + 700))
        self.assertEqual(probes(out)["hidden"], [False, False, ""])

    def test_the_buttons_under_the_list_are_all_settings_and_LGs_own_quick_settings(self):
        go = ("var s = find(root, 'footFocus'); var n = 0; while (s.focusId !== 'ai' && n++ < 12) s.key('down'); ")
        rc, out = self.host(
            *probe(READY + 300, "open", "var b = bus(); return b.open('quick');"),
            *probe(READY + 1000, "foot", go + "s.key('down'); var a = [s.footFocus, s.footIndex]; s.key('right'); var b2 = [s.footFocus, s.footIndex]; s.key('left'); return [a, b2, s.footIndex];"),
            *probe(READY + 1200, "lg", "var s = find(root, 'footFocus'); s.key('right'); s.key('ok'); return [root.layerOpen];"),
            "--eval", "%d:%s" % (READY + 1500, CALLS), "--wait", str(READY + 2000))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["foot"], [[True, 0], [True, 1], 0])
        self.assertEqual(p["lg"], [""], "LG's panel was asked for and ours slid away")
        got = launches(self.values(out)[-1])
        self.assertIn(("systemUIManager", "/launch", {"appId": "com.webos.app.quicksettings"}), got, out[-2500:])

    def test_a_settings_page_asked_for_over_an_app_opens_when_home_is_back(self):
        rc, out = self.host(
            *probe(READY + 300, "away", "root.hostActive = false; return root.hostActive;"),
            *probe(READY + 500, "want", "bus().want('network'); return 1;"),
            *probe(READY + 700, "back", "root.hostActive = true; return root.hostActive;"),
            *probe(READY + 1600, "open", "return [root.settingsOpen, root.layerOpen];"),
            "--wait", str(READY + 2000))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["open"], [True, ""], out[-2500:])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class OverlayPanels(HostCase):
    """An app in front: the overlay's own panels, asked through the same bus."""

    def overlay_host(self, steps, wait, theme=PLAIN):
        argv = ["--eval", "100:(function(){ overlay.warm = true; overlay.warmQuick = true; overlay.warmPicker = true; return 1 })()"]
        for at, label, body in steps:
            argv += probe(at, label, body)
        rc, out = self.host(*argv, "--overlay", "--prop", "hostActive=false", "--theme", theme, "--wait", str(wait))
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        return probes(out), out

    def test_the_gear_shows_the_quick_panel_over_an_app_and_back_slides_it_away(self):
        panel = "var s = find(overlay, 'footFocus');"
        p, out = self.overlay_host([
            (900, "idle", "return [overlay.panelReady('quick'), overlay.panelReady('picker')];"),
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "ready", "return [overlay.panelReady('quick'), overlay.panelReady('picker'), overlay.flatGlass];"),
            (1250, "nodes0", panel + "return nodes(s);"),
            (1300, "open", "return [overlay.panel('quick'), overlay.surface, overlay.wants, overlay.up];"),
            (2000, "shown", panel + "return {progress: s.progress, x: s.slideX, y: s.slideY, w: s.sheetW, h: s.sheetH, presented: s.presented, nodes: nodes(s)};"),
            (2100, "down", panel + "overlay.key(Qt.Key_Down, true, false, 0); overlay.key(Qt.Key_Down, false, false, 0); return s.focusId;"),
            (2200, "back", panel + "overlay.key(Qt.Key_Back, true, false, 0); overlay.key(Qt.Key_Back, false, false, 0); return [overlay.surface, s.leaving, s.visible, overlay.visible, overlay.panelLeaving];"),
            (2900, "gone", panel + "return [overlay.surface, s.leaving, s.visible, overlay.visible, overlay.panelLeaving];"),
        ], 3300)
        self.assertEqual(p["idle"], [False, False], "nothing over Home: Home draws its own")
        self.assertEqual(p["ready"], [True, True, True], out[-2000:])
        self.assertEqual(p["open"], [True, "quick", True, True])
        s = p["shown"]
        self.assertEqual((s["progress"], s["x"], s["y"], s["w"], s["h"], s["presented"]), (1, 24, 24, 492, 1008, True))
        self.assertEqual(s["nodes"], p["nodes0"], "the panel built something when it opened")
        self.assertNotEqual(p["down"], "", "the D-pad did not reach the panel")
        self.assertEqual(p["back"], ["", True, True, True, True], "Back must start the slide, not cut it")
        self.assertEqual(p["gone"], ["", False, False, False, False])

    def test_the_gear_again_hides_it_and_a_gear_hold_replaces_it_with_the_drawer(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "return [overlay.panel('quick'), overlay.surface];"),
            (1700, "again", "return [overlay.panel('quick'), overlay.surface];"),
            (2300, "reopen", "return [overlay.panel('quick'), overlay.surface];"),
            (2800, "hold", "return [overlay.gearHold(), overlay.surface];"),
        ], 3300)
        self.assertEqual(p["open"], [True, "quick"])
        self.assertEqual(p["again"], [True, ""])
        self.assertEqual(p["reopen"], [True, "quick"])
        self.assertEqual(p["hold"], [True, "drawer"])

    def test_the_picker_over_an_app_goes_on_to_the_next_input_and_OK_launches_the_one_picked(self):
        pick = "var s = find(overlay, 'soundLine');"
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "return [overlay.panel('picker'), overlay.surface];"),
            (2000, "at", pick + "return {index: s.index, n: s.tiles.length, y: s.sheetY, progress: s.progress};"),
            (2100, "next", pick + "var i = s.index; overlay.panel('picker'); return [i, s.index, overlay.surface];"),
            (2200, "ok", pick + "var t = s.tiles[s.index]; overlay.key(Qt.Key_Return, true, false, 0); return [t.appId, overlay.surface];"),
            (2300, "calls", "return JSON.parse(%s);" % CALLS),
        ], 3000)
        self.assertEqual(p["open"], [True, "picker"])
        self.assertEqual((p["at"]["progress"], p["at"]["y"]), (1, 612))
        self.assertEqual(p["next"][1], (p["next"][0] + 1) % p["at"]["n"])
        app, surface = p["ok"]
        self.assertEqual(surface, "", "the picker stays up after an input was picked")
        got = [c for c in p["calls"] if c["u"].endswith("applicationManager") and c["m"] == "/launch"]
        self.assertEqual(got[-1]["p"], {"id": app}, out[-2000:])

    def test_all_settings_over_an_app_takes_the_tv_home_and_Home_opens_Settings(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "overlay.panel('quick'); var s = find(overlay, 'footFocus'); var n = 0; while (s.focusId !== 'ai' && n++ < 12) s.key('down'); s.key('down'); s.key('ok'); return [overlay.surface, s.footIndex];"),
            (1400, "waiting", "return bus().take();"),
            (1500, "calls", "return JSON.parse(%s);" % CALLS),
        ], 2000)
        self.assertEqual(p["open"], ["", 0])
        self.assertEqual(p["waiting"], "", "the request for Settings (no page) waits for Home")
        self.assertIn({"u": "luna://com.webos.applicationManager", "m": "/launchDefaultApp", "p": {"category": "home"}}, p["calls"], out[-2000:])

    def test_a_row_that_opens_a_page_waits_for_Home_with_the_page(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "overlay.panel('quick'); var s = find(overlay, 'footFocus'); var n = 0; while (s.focusId !== 'wifi' && n++ < 12) s.key('down'); s.key('ok'); return overlay.surface;"),
            (1400, "waiting", "return bus().take();"),
        ], 1900)
        self.assertEqual(p["open"], "")
        self.assertEqual(p["waiting"], "network")

    def test_LGs_own_quick_settings_is_one_button_away_and_opens_over_the_app(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "overlay.panel('quick'); var s = find(overlay, 'footFocus'); var n = 0; while (s.focusId !== 'ai' && n++ < 12) s.key('down'); s.key('down'); s.key('right'); s.key('ok'); return overlay.surface;"),
            (1400, "calls", "return JSON.parse(%s);" % CALLS),
        ], 1900)
        self.assertEqual(p["open"], "")
        self.assertIn({"u": "luna://com.webos.service.systemUIManager", "m": "/launch", "p": {"appId": "com.webos.app.quicksettings"}}, p["calls"], out[-2000:])

    def test_the_owners_switches_leave_the_gear_and_the_input_key_to_LG(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "off", "return [overlay.panelReady('quick'), overlay.panelReady('picker'), overlay.panel('quick'), overlay.panel('picker'), overlay.surface];"),
        ], 1700, theme=OFF)
        self.assertEqual(p["off"], [False, False, False, False, ""])

    def test_the_door_opens_a_panel_for_the_gate_and_the_tv_steps(self):
        import os
        root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
        with open(os.path.join(root, "brand.json")) as f:
            door = "%s-overlay" % json.load(f)["shortId"]
        with open(os.path.join(own_tmp(self), door), "w") as f:
            f.write('{"open":"quick"}')
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1500, "read", "overlay.readDoor(); return [overlay.surface, overlay.wants];"),       # the poll reads it every second; the test does not wait for the tick
        ], 2400)
        self.assertEqual(p["read"], ["quick", True], out[-2000:])


if __name__ == "__main__":
    unittest.main()
