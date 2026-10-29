"""Run with: python3 -m unittest discover -s tests -p 'test_quick_panels.py'   (needs Xvfb and PyQt5; skipped without them)

LumaTV's quick panel (the gear's tap) and input picker (the Input key) in place of LG's, through the offline QML host: Home's side
(HomePanels.qml, asked through LumaBus.js as the key filter's second entry does) and the overlay's over an app (`--overlay`). The key
filter's own side is `tests/keyfilter.test.js` and `tests/bus.test.js`. What these check: the panels open, close and move on when asked,
that the panel slides from the right edge to its place (560 x 968, 40 from the right edge, 56 from the top and bottom)
and stays drawn until it has gone, that nothing is built when one opens, what each row and button does, and that a panel LG should
have is not taken. Nothing here measures time: the set's gate does (docs/research/perf/gate.py, surfaces quick and picker).
"""
import json
import os
import re
import sys
import unittest
from unittest import mock

from test_settings import HostCase, READY
from xdisplay import DISPLAY, HAVE_QT, own_tmp

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "qmlhost"))
import renderstats  # noqa: E402

FIND = """function find(p, name) {
    if (p[name] !== undefined && p.children !== undefined && p.visible !== undefined) return p;
    for (var i=0; i<p.children.length; i++) { var f=find(p.children[i], name); if (f) return f; }
    return null;
}
function nodes(p) { var n = 1; for (var i=0; i<p.children.length; i++) n += nodes(p.children[i]); return n; }"""
# LumaBus.js as the key filter's bridge reaches it: the same library instance Home and the overlay registered with
BUS = ("function bus() { return Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'LumaBus.js\" as Bus\\n"
       "QtObject { function ready(k) { return Bus.panelReady(k) } function open(k) { return Bus.panel(k) } function take() { return Bus.takeSettings() }"
       " function want(p) { Bus.wantSettings(p) } function warm(k) { return Bus.warmPanel(k) } }', root) }")
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { u: c.uri, m: c.method, p: p } }))"
PLAIN = '{"perf":{"syncResidents":true}}'
REDUCED = '{"perf":{"syncResidents":true},"motion":{"reduced":true}}'
OFF = '{"perf":{"syncResidents":true},"remote":{"quickPanel":false,"inputPicker":false}}'


# The panel's drawn parts are its direct children (the scrim and the sheet): how many of them lie on the screen at all
ONSCREEN = ("function onscreen(p) { var n = 0; for (var i = 0; i < p.children.length; i++) { var c = p.children[i]; "
            "if (c.visible && c.width > 0 && c.height > 0 && c.x + c.width > 0 && c.x < root.width && c.y + c.height > 0 && c.y < root.height) n++ } return n }")


def probe(at, label, body, root="root"):
    """An eval at `at` ms that logs `[P:label] <json>` of what `body` returns; the helper functions above are in scope."""
    js = "(function(){%s%s%s var r=(function(){%s})(); console.info('[P:%s] '+JSON.stringify(r)); return 1;})()" % (FIND, ONSCREEN, BUS, body, label)
    return ["--eval", "%d:%s" % (at, js)]


def probes(out):
    return {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[P:(\w+)\] (.*)", out)}


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class HomePanels(HostCase):
    """Home in front: the key filter's second entry asks the bus, the bus asks HomePanels."""

    def test_repeated_input_answers_preserve_the_tiles_and_real_changes_still_update(self):
        rc, out = self.host(*probe(READY + 400, 'inputs', "var api = Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\" as Kit\\nQtObject { function run() { var i = Kit.Inputs; var d = {devices:[{appId:\"com.webos.app.hdmi1\",label:\"Apple TV\",connected:true}]}; i.parse(JSON.stringify(d)); var before = i.list; i.parse(JSON.stringify(d)); var same = before === i.list; d.devices[0].label = \"PlayStation\"; i.parse(JSON.stringify(d)); var renamed = i.list !== before && i.list[0].icon === \"gamepad-2\"; before = i.list; d.devices[0].connected = false; i.parse(JSON.stringify(d)); return [same, renamed, i.list !== before, i.list[0].icon] } }', root); return api.run();"), '--wait', str(READY + 900))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)['inputs'], [True, True, True, 'unplug'])

    def test_the_gear_shows_the_quick_panel_at_the_right_edge_and_the_gear_again_slides_it_away(self):
        panel = "var s = find(root, 'footFocus');"
        rc, out = self.host(
            *probe(READY + 300, "ready", "var b = bus(); return [b.ready('quick'), b.ready('picker'), b.ready('nothing')];"),
            *probe(READY + 350, "linger", panel + "s.lingerMs = 1000; return s.lingerMs;"),
            *probe(READY + 400, "nodes0", panel + "return nodes(s);"),
            *probe(READY + 500, "open", "var b = bus(); var r = b.open('quick'); return [r, root.layerOpen];"),
            *probe(READY + 1300, "shown", panel + "return {open: root.layerOpen, presented: s.presented, progress: s.progress, x: s.slideX, y: s.slideY, w: s.sheetW, h: s.sheetH, "
                   "visible: s.visible, nodes: nodes(s)};"),
            *probe(READY + 1400, "close", "var b = bus(); var r = b.open('quick'); var s = find(root, 'footFocus'); return [r, root.layerOpen, s.leaving, s.visible];"),
            *probe(READY + 1900, "parked", panel + "return {leaving: s.leaving, progress: s.progress, visible: s.visible, presented: s.presented, held: s.held, parked: s.parked};"),
            *probe(READY + 3000, "gone", panel + "return {leaving: s.leaving, progress: s.progress, visible: s.visible, presented: s.presented, held: s.held};"),
            "--wait", str(READY + 3400))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["ready"], [True, True, False], out[-2500:])
        self.assertEqual(p["open"], [True, "quick"])
        # its place and size: 560 by 968, 40 from the right edge and 56 from the top
        s = p["shown"]
        self.assertEqual((s["open"], s["presented"], s["progress"], s["visible"]), ("quick", True, 1, True), out[-2500:])
        self.assertEqual((s["x"], s["y"], s["w"], s["h"]), (1320, 56, 560, 968))
        # nothing was created when it opened
        self.assertEqual(s["nodes"], p["nodes0"], "the panel built something when it opened")
        # shown again, the gear hides it: it is told to go at once and stays drawn while it slides back, then it is away
        self.assertEqual(p["close"][:2], [True, ""])
        self.assertEqual(p["close"][2:], [True, True], "the panel was cut off instead of sliding away")
        # the slide is over: it is parked, off the screen and still in the scene (test_a_closed_panel_stays_in_the_scene...), then it goes
        self.assertEqual(p["parked"], {"leaving": False, "progress": 0, "visible": True, "presented": False, "held": True, "parked": True})
        self.assertEqual(p["gone"], {"leaving": False, "progress": 0, "visible": False, "presented": False, "held": False})

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
        self.assertEqual(p["early"]["x"], 1320, "the sheet travelled in reduced motion")
        self.assertEqual(p["shown"], {"x": 1320, "progress": 1})

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
        # the slide is at most 110 ms, and nothing that rebinds the rows is asked for while it runs (the answers came mid-slide). The gate counts
        # a frame within 50 ms of the burst's last as part of it and the burst can last to 177 ms, so the readings, whose answers rebind
        # the rows and tiles, wait for 310 ms (r14 over HDMI and the film: picker burst ends of 206 and 301 ms at the 170 ms they were asked
        # for before; r15 over the film: a quick burst to 370 ms).
        asked = "root.lunaCalls.filter(function(c) { return c.method === '/getAllInputStatus' }).length"
        for kind, find, quiet in (("quick", "footFocus", 230), ("picker", "soundLine", 230)):
            rc, out = self.host(
                *probe(READY + 500, "before", "var b = bus(); var n = %s; b.open('%s'); return n;" % (asked, kind)),
                *probe(READY + 560, "during", "return %s;" % asked),
                *probe(READY + 700, "done", "var s = find(root, '%s'); return {n: %s, progress: s.progress};" % (find, asked)),
                *probe(READY + 500 + max(quiet, 220), "quiet", "return %s;" % asked),
                *probe(READY + 1000, "after", "return %s;" % asked),
                "--wait", str(READY + 1400))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertEqual(p["during"], p["before"], "the %s asked for a reading while it was sliding in" % kind)
            self.assertEqual(p["done"]["progress"], 1, "the %s slide was not over 200 ms after it opened" % kind)
            if quiet:
                self.assertEqual(p["quiet"], p["before"], "the %s asked for a reading %d ms after it opened, in the burst the key started" % (kind, quiet))
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

    def test_nothing_fades_while_a_panel_slides(self):
        # Each change of an item's opacity made Qt's renderer batch what it covers again, every frame of the slide: the scrim all of Home
        # (round 11 over Home: quick p90 3.70 ms), the sheet itself, and the frame where it passed 1 the whole render list (round 12:
        # 3.1-4.5 ms frames). The scrim is whole from the first frame and the sheet only moves.
        layers = ("var c = s.children, f = [], full = 0; for (var i = 0; i < c.length; i++) if (c[i].visible) { f.push(c[i].opacity); "
                  "if (c[i].width >= root.width && c[i].height >= root.height) full++ } return {drawn: s.visible, progress: s.progress, ops: f, full: full};")
        for kind, find in (("quick", "footFocus"), ("picker", "soundLine")):
            # The software renderer's first frame of a panel can outlast the slide; a probe before the slide has ended is enough (the faded
            # scrim drew at the opacity of the progress, 0 at the start).
            steps = list(probe(READY + 500, "open", "return bus().open('%s');" % kind))
            for k in range(16):
                steps += probe(READY + 510 + 25 * k, "mid%d" % k, "var s = find(root, '%s'); " % find + layers)
            rc, out = self.host(*steps, "--wait", str(READY + 1000))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertTrue(p["open"])
            drawn = [m for m in (p["mid%d" % k] for k in range(16)) if m["drawn"]]
            self.assertTrue(any(m["progress"] < 1 for m in drawn), "no probe saw the %s before its slide ended: %s" % (kind, drawn))
            for m in drawn:
                self.assertEqual(m["full"], 1, "the %s has no full-screen scrim" % kind)
                self.assertEqual(set(m["ops"]), {1}, "a layer of the %s faded at progress %s: %s" % (kind, m["progress"], m["ops"]))

    def test_a_closed_panel_stays_in_the_scene_off_the_screen_for_a_while_and_then_goes(self):
        # Nodes that join the scene or leave it make Qt's renderer build the render list of the whole scene again: the slow frame of every
        # opening and closing on the set (round 14: 3.2-3.4 ms where a panel's nodes joined Home's, 4.0-4.4 ms where they left, against about
        # 2 ms). After its slide a panel is parked: what it draws is moved off the screen, it stays visible (in the scene) for `lingerMs`, and an
        # opening in that time joins nothing.
        for kind, find in (("quick", "footFocus"), ("picker", "soundLine")):
            state = ("var s = find(root, '%s'); return {visible: s.visible, held: s.held, parked: s.parked, progress: s.progress, "
                     "drawn: onscreen(s), nodes: nodes(s)};" % find)
            shut = "return bus().open('quick');" if kind == "quick" else "root.closeLayer(); return true;"      # the Input key again goes on to the next input
            rc, out = self.host(
                *probe(READY + 300, "linger", "var s = find(root, '%s'); s.lingerMs = 1500; return s.lingerMs;" % find),
                *probe(READY + 400, "built", state),
                *probe(READY + 500, "open", "return bus().open('%s');" % kind),
                *probe(READY + 1300, "up", state),
                *probe(READY + 1400, "close", shut),
                *probe(READY + 2100, "parked", state),                  # the slide is over, 800 ms of the linger are left
                *probe(READY + 2200, "again", "return bus().open('%s');" % kind),
                *probe(READY + 2300, "reopened", state),                # it never stopped being visible: nothing joined
                *probe(READY + 3000, "up2", state),
                *probe(READY + 3100, "close2", shut),
                *probe(READY + 3600, "parked2", state),
                *probe(READY + 5300, "gone", state),                    # the linger ran out at 4600
                "--wait", str(READY + 5700))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertEqual((p["linger"], p["open"], p["again"], p["close"], p["close2"]), (1500, True, True, True, True))
            built, up = p["built"], p["up"]
            self.assertEqual((built["visible"], built["held"], built["parked"], built["drawn"]), (False, False, True, 0), "%s: built, never shown: %s" % (kind, built))
            self.assertEqual((up["visible"], up["held"], up["parked"], up["progress"], up["drawn"]), (True, False, False, 1, 2), "%s: shown: %s" % (kind, up))
            for name in ("parked", "parked2"):
                parked = p[name]
                self.assertEqual((parked["visible"], parked["held"], parked["parked"], parked["progress"], parked["drawn"]), (True, True, True, 0, 0),
                                 "%s %s: the closed panel must be in the scene and off the screen: %s" % (kind, name, parked))
                self.assertEqual(parked["nodes"], built["nodes"], "the %s built or lost something while parked" % kind)
            self.assertEqual((p["reopened"]["visible"], p["reopened"]["held"], p["reopened"]["drawn"]), (True, False, 2), "%s: opened again while parked: %s" % (kind, p["reopened"]))
            self.assertEqual(p["up2"], up)
            self.assertEqual((p["gone"]["visible"], p["gone"]["held"], p["gone"]["drawn"], p["gone"]["nodes"]), (False, False, 0, built["nodes"]))

    def test_the_gears_press_joins_the_quick_panel_to_the_scene_before_its_tap_opens_it(self):
        # The key filter's first entry sees the gear go down, and LG replays the tap on the release: the panel joins the scene, off the screen, at
        # the press (and moves the sheet once, which makes it a batch root there and not in the slide), so the opening joins and rebuilds nothing.
        # Nothing is shown; a hold becomes the drawer, which lets the panel go again; something that is up leaves it alone.
        state = "var s = find(root, 'footFocus'); return {visible: s.visible, held: s.held, parked: s.parked, drawn: onscreen(s), x: s.slideX, nodes: nodes(s)};"
        rc, out = self.host(
            *probe(READY + 400, "warm", "var r = bus().warm('quick'); return [r, find(root, 'footFocus').held, find(root, 'footFocus').slideX];"),
            *probe(READY + 1100, "warmed", state),                  # the sheet's move is a timer of 30 ms, which a frame of the software renderer can hold up
            *probe(READY + 1200, "open", "return bus().open('quick');"),
            *probe(READY + 2000, "up", state),
            *probe(READY + 2100, "close", "return bus().open('quick');"),
            *probe(READY + 2300, "hold", "root.closeAllLayers(); var w = bus().warm('quick'); root.openLayer('drawer'); var s = find(root, 'footFocus'); return [w, s.held, s.visible, root.layerOpen];"),
            *probe(READY + 2800, "busy", "root.closeAllLayers(); root.openSettings(); return [bus().warm('quick'), root.settingsOpen];"),
            *probe(READY + 2900, "picker", "root.closeSettings(); return [bus().warm('picker'), bus().ready('picker')];"),
            "--wait", str(READY + 3300))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["warm"], [True, True, 2320], "the gear's press must join the panel at once, parked")
        warmed = p["warmed"]
        self.assertEqual((warmed["visible"], warmed["held"], warmed["parked"], warmed["drawn"]), (True, True, True, 0), "warmed, nothing is shown: %s" % warmed)
        # parked off the right edge (1920 + 400), one px further once the nudge's 16 ms timer has run; a frame of the software renderer on a busy machine can hold that timer
        # up past the probe, so both are the sheet having moved at most once
        self.assertIn(warmed["x"], (2320, 2321), "the sheet did not move once after it joined the scene")
        self.assertEqual(p["open"], True)
        up = p["up"]
        self.assertEqual((up["visible"], up["held"], up["parked"], up["drawn"], up["x"]), (True, False, False, 2, 1320), "opened after a warm-up: %s" % up)
        self.assertEqual(up["nodes"], warmed["nodes"], "the opening built or lost something")
        self.assertEqual(p["hold"], [True, False, False, "drawer"], "the drawer opened and the warmed panel stayed")
        self.assertEqual(p["busy"], [False, True], "a panel was warmed under Settings")
        self.assertEqual(p["picker"][0], False, "only the quick panel is warmed")

    def test_the_renderer_builds_nothing_of_the_scene_again_when_the_tap_opens_a_panel_that_the_press_joined(self):
        # tests/qmlhost/renderstats.py: the press' warm-up takes the join (and the sheet becoming a batch root) out of the frames of the opening
        t = READY + 7000
        steps = [*probe(t, "warm", "return bus().warm('quick');"), *probe(t + 1500, "open", "return bus().open('quick');"),
                 *probe(t + 2400, "close", "return bus().open('quick');"), *probe(t + 3200, "end", "return 1;")]
        with mock.patch.dict(os.environ, {"QSG_RENDERER_DEBUG": "render"}):
            rc, out = self.host(*steps, "--wait", str(t + 3700))
        self.assertEqual(rc, 0, out[-1500:])
        seg = {m["marker"]: m["frames"] for m in renderstats.parse(out)}
        report = "\n".join("%s %s" % (name, renderstats.kinds(seg[name])) for name in ("warm", "open", "close"))
        self.assertGreaterEqual(renderstats.whole(seg["warm"]), 1, "the panel did not join the scene when it was warmed\n%s" % report)
        self.assertEqual(renderstats.whole(seg["open"][:6]), 0, "the opening of a warmed panel rebuilt the scene\n%s" % report)
        self.assertEqual(renderstats.whole(seg["close"]), 0, "its closing rebuilt the scene\n%s" % report)

    def test_a_surface_that_opens_takes_the_parked_panels_out_of_the_scene_with_it(self):
        # Whatever opens joins nodes to the scene and rebuilds its render list anyway: the panels parked for their next opening go in that frame,
        # not in one of their own a few seconds later (the linger here is the default, 4 s: it is still running when the next surface opens)
        rc, out = self.host(
            *probe(READY + 400, "open", "return bus().open('quick');"),
            *probe(READY + 1200, "close", "return bus().open('quick');"),
            *probe(READY + 1800, "parked", "var s = find(root, 'footFocus'); return [s.held, s.visible];"),
            *probe(READY + 1900, "picker", "return bus().open('picker');"),
            *probe(READY + 2300, "after", "var s = find(root, 'footFocus'); var q = find(root, 'soundLine'); return [s.held, s.visible, q.visible, root.layerOpen];"),
            *probe(READY + 2400, "shut", "root.closeLayer(); return root.layerOpen;"),
            *probe(READY + 3000, "held", "var q = find(root, 'soundLine'); return [q.held, q.visible];"),
            *probe(READY + 3100, "settings", "root.openSettings(); return root.settingsOpen;"),
            *probe(READY + 3700, "later", "var q = find(root, 'soundLine'); return [q.held, q.visible, root.settingsOpen];"),
            "--wait", str(READY + 4100))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["parked"], [True, True])
        self.assertEqual(p["after"], [False, False, True, "picker"], "the picker opened and the quick panel stayed parked")
        self.assertEqual((p["shut"], p["held"]), ("", [True, True]))
        self.assertEqual(p["later"], [False, False, True], "Settings opened and the picker stayed parked")

    def test_the_renderer_builds_nothing_of_the_scene_again_for_a_panel_that_is_parked(self):
        # tests/qmlhost/renderstats.py: what Qt's batch renderer says it rebuilt in each frame ("full": the render list and the batches of the whole
        # scene). With no linger a panel's nodes join the scene at its opening and leave at its closing, and each is a full rebuild. With one the
        # first opening still joins, and the closing, an opening again and its closing rebuild nothing of the scene. The steps are placed between
        # Home's own animations (11-12.6 s after the host starts and every 10 s on: a full rebuild of theirs would be counted as the panel's).
        t = READY + 7000
        for kind, find in (("quick", "footFocus"), ("picker", "soundLine")):
            set_linger = lambda at, label, ms: probe(at, label, "find(root, '%s').lingerMs = %d; return %d;" % (find, ms, ms))
            go = lambda at, label: probe(at, label, "return bus().open('%s');" % kind if label.startswith("open") or kind == "quick" else "root.closeLayer(); return true;")
            nodes_now = lambda at, label: probe(at, label, "return nodes(find(root, '%s'));" % find)
            steps = [*set_linger(t - 300, "none", 0), *go(t, "open0"), *go(t + 900, "close0"), *set_linger(t + 1600, "some", 1500),
                     *go(t + 1800, "open1"), *go(t + 2700, "close1"), *go(t + 3400, "open2"), *go(t + 4300, "close2"),
                     *nodes_now(t + 5000, "parked"), *nodes_now(t + 6500, "gone")]
            with mock.patch.dict(os.environ, {"QSG_RENDERER_DEBUG": "render"}):
                rc, out = self.host(*steps, "--wait", str(t + 7000))
            self.assertEqual(rc, 0, out[-1500:])
            seg = {m["marker"]: m["frames"] for m in renderstats.parse(out)}
            report = "\n".join("%s %s" % (name, renderstats.kinds(seg[name])) for name in ("open0", "close0", "open1", "close1", "open2", "close2", "parked", "gone"))
            self.assertGreaterEqual(renderstats.whole(seg["open0"][:5]), 1, "%s: without a linger the opening joins the scene, and that is a full rebuild\n%s" % (kind, report))
            self.assertGreaterEqual(renderstats.whole(seg["close0"]), 1, "%s: without a linger the closing leaves it, and that is another\n%s" % (kind, report))
            self.assertEqual(renderstats.whole(seg["close1"]), 0, "%s: the closing of a panel that stays in the scene rebuilt it\n%s" % (kind, report))
            self.assertEqual(renderstats.whole(seg["open2"][:5]), 0, "%s: an opening of a parked panel rebuilt the scene\n%s" % (kind, report))
            self.assertEqual(renderstats.whole(seg["close2"]), 0, "%s: the closing after it rebuilt the scene\n%s" % (kind, report))
            # and what it left behind when it went is Home's own scene again: the panel's nodes (the scrim, the glass and the sheet's cached
            # texture) are no longer drawn in every frame
            shown = max(f["nodes"] for f in seg["parked"])
            home = max(f["nodes"] for f in seg["gone"])
            self.assertGreaterEqual(shown - home, 3, "%s: the scene still drew %d nodes after the linger (%d while parked)\n%s" % (kind, home, shown, report))

    def test_a_panel_sheet_is_one_cached_texture_and_takes_a_focus_change_at_once_while_it_moves(self):
        # The sheet's content is drawn once into a texture (LuPane.cacheContent): the scene with the quick panel up drew 101 batches, with
        # the texture 72. A halo or a lift animating inside it draws the texture again in every frame, so while the panel moves a focus
        # change lands at once; once it is up a focus change animates as everywhere else.
        halos = ("function halos(p) { var out = []; (function walk(n) { for (var i = 0; i < n.children.length; i++) { var c = n.children[i]; "
                 "if (c.strength !== undefined && c.tint !== undefined && c.on) out.push(Math.round(c.opacity * 100) / 100); walk(c) } })(p); return out }")
        for kind, find, steps in (("quick", "footFocus", ("down", "down")), ("picker", "soundLine", ("right", "left"))):
            act = lambda step: ("var p = find(root, '%s'); %s p.key('%s'); return [find(p, 'cacheContent').cacheContent, p.progress < 1, halos(p)];"
                                % (find, halos, step))
            rc, out = self.host(
                *probe(READY + 400, "open", "return bus().open('%s');" % kind),
                *probe(READY + 430, "moving", act(steps[0])),
                *probe(READY + 1400, "up", act(steps[1])),
                "--wait", str(READY + 1800))
            self.assertEqual(rc, 0, out[-2500:])
            p = probes(out)
            self.assertEqual(p["moving"], [True, True, [0.3]], "%s: the halo of the tile focused while the panel slid did not land at once" % kind)
            self.assertEqual(p["up"][:2], [True, False])
            self.assertLess(p["up"][2][0], 0.3, "%s: a focus change on a panel at rest did not animate" % kind)

    def test_each_reading_of_the_inputs_lets_the_last_subscription_go(self):
        # LG's Service keeps a subscription open until it is cancelled; the picker and the quick panel read the inputs at every opening
        live = ("JSON.stringify(['luna://com.webos.service.eim', 'luna://com.webos.service.applicationmanager'].map(function(u) { "
                "var mine = root.lunaCalls.filter(function(c) { return c.uri === u }); "
                "return [mine.filter(function(c) { return c.method !== 'cancel' }).length, mine.filter(function(c) { return c.method === 'cancel' }).length] }))")
        steps = []
        for k, kind in enumerate(("picker", "quick", "picker")):
            steps += probe(READY + 300 + 700 * k, "open%d" % k, "return bus().open('%s');" % kind)
            steps += probe(READY + 700 + 700 * k, "close%d" % k, "return bus().open('%s');" % kind)
        rc, out = self.host(*steps, "--eval", "%d:%s" % (READY + 2600, live), "--wait", str(READY + 2900))
        self.assertEqual(rc, 0, out[-2500:])
        for (asked, cancelled), what in zip(json.loads(self.values(out)[-1]), ("input status", "foreground app")):
            self.assertGreaterEqual(asked, 4, "the %s was not read at each opening" % what)
            self.assertEqual(asked - cancelled, 1, "%d subscriptions to the %s left open" % (asked - cancelled, what))

    def test_the_picker_opening_again_does_not_start_its_tiles_lift_over(self):
        # pickFront used to pass the focus through the first tile, which restarted the front tile's lift and halo at every opening and again
        # when the readings came: 180 ms more of frames after the slide, each one re-batching the scene
        scan = ("var s = find(root, 'soundLine'); var out = []; function walk(p) { for (var i = 0; i < p.children.length; i++) { var c = p.children[i]; "
                "if (c.scale !== 1) out.push(Math.round(c.scale * 1000) / 1000); walk(c) } } walk(s); return out;")
        steps = list(probe(READY + 300, "first", "return bus().open('picker');"))
        steps += probe(READY + 1200, "closed", "root.closeLayer(); return root.layerOpen;")
        steps += probe(READY + 1700, "again", "return bus().open('picker');")
        for j, dt in enumerate((5, 60, 120, 200, 300, 450, 700)):
            steps += probe(READY + 1700 + dt, "s%d" % j, scan)
        rc, out = self.host(*steps, "--wait", str(READY + 2600))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual((p["first"], p["closed"], p["again"]), (True, "", True))
        seen = [p["s%d" % j] for j in range(7)]
        self.assertTrue(all(len(s) == 1 for s in seen), "one lifted plate, the focused tile's: %s" % seen)
        self.assertEqual(len({s[0] for s in seen}), 1, "the focused tile's lift moved while the picker opened again: %s" % seen)

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

    def test_the_one_button_under_the_list_is_all_settings(self):
        go = ("var s = find(root, 'footFocus'); var n = 0; while (s.focusId !== 'ai' && n++ < 12) s.key('down'); ")
        rc, out = self.host(
            *probe(READY + 300, "open", "var b = bus(); return b.open('quick');"),
            *probe(READY + 1000, "foot", go + "s.key('down'); var a = s.footFocus; s.key('right'); var b2 = s.footFocus; s.key('up'); return [a, b2, s.footFocus];"),
            "--wait", str(READY + 1500))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["foot"], [True, True, False], "Down past the last tile lands on All settings, Right stays on it, Up goes back to the tiles")

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
            (1260, "linger", panel + "s.lingerMs = 300; return s.lingerMs;"),
            (1300, "open", "return [overlay.panel('quick'), overlay.surface, overlay.wants, overlay.up];"),
            # the software renderer's first showing of the overlay's panel compiles its programs; the slide counts frames, not time
            (3900, "shown", panel + "return {progress: s.progress, x: s.slideX, y: s.slideY, w: s.sheetW, h: s.sheetH, presented: s.presented, nodes: nodes(s)};"),
            (4000, "down", panel + "overlay.key(Qt.Key_Down, true, false, 0); overlay.key(Qt.Key_Down, false, false, 0); return s.focusId;"),
            (4100, "back", panel + "overlay.key(Qt.Key_Back, true, false, 0); overlay.key(Qt.Key_Back, false, false, 0); return [overlay.surface, s.leaving, s.visible, overlay.visible, overlay.panelLeaving];"),
            (5600, "gone", panel + "return [overlay.surface, s.leaving, s.visible, overlay.visible, overlay.panelLeaving];"),
        ], 6000)
        self.assertEqual(p["idle"], [False, False], "nothing over Home: Home draws its own")
        self.assertEqual(p["ready"], [True, True, True], out[-2000:])
        self.assertEqual(p["open"], [True, "quick", True, True])
        s = p["shown"]
        self.assertGreater(s["progress"], 0.999, "the slide is over")
        self.assertAlmostEqual(s["x"], 1320, delta=1)
        self.assertEqual((s["y"], s["w"], s["h"], s["presented"]), (56, 560, 968, True))
        self.assertEqual(s["nodes"], p["nodes0"], "the panel built something when it opened")
        self.assertNotEqual(p["down"], "", "the D-pad did not reach the panel")
        self.assertEqual(p["back"], ["", True, True, True, True], "Back must start the slide, not cut it")
        self.assertEqual(p["gone"], ["", False, False, False, False])

    def test_a_closed_panel_over_an_app_stays_in_the_scene_for_a_while_and_goes_with_the_app(self):
        panel = "var s = find(overlay, 'footFocus');"
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "linger", panel + "s.lingerMs = 1500; return s.lingerMs;"),
            (1300, "open", "return [overlay.panel('quick'), overlay.surface];"),
            # the software renderer's first showing of the overlay's panel compiles its programs; the slide counts frames, not time, and a busy
            # machine draws them slowly: the slide is read 2.6 s after the opening, and the rest follows it
            (3900, "shown", panel + "return [s.progress, onscreen(s), s.held];"),
            (4000, "close", "return [overlay.panel('quick'), overlay.surface];"),
            (4600, "parked", panel + "return [s.held, s.visible, onscreen(s), overlay.visible, overlay.panelHeld, overlay.up];"),
            (4700, "again", "return [overlay.panel('quick'), overlay.surface];"),
            (4800, "reopened", panel + "return [s.held, s.visible, overlay.visible];"),
            (4900, "close2", "return [overlay.panel('quick'), overlay.surface];"),
            (5200, "away", "overlay.appFront = false; return overlay.appFront;"),
            (5600, "after", panel + "return [s.held, s.visible, overlay.visible];"),
        ], 6000)
        # the slide's progress is an eased value, a hair under 1 until its last frame is drawn
        self.assertEqual((p["linger"], p["open"], p["shown"][1:]), (1500, [True, "quick"], [2, False]))
        self.assertGreater(p["shown"][0], 0.999, "the slide is over")
        self.assertEqual(p["close"], [True, ""])
        self.assertEqual(p["parked"], [True, True, 0, True, True, False], "over an app the closed panel is in the scene, off the screen, and the layer with it")
        self.assertEqual((p["again"], p["reopened"]), ([True, "quick"], [False, True, True]))
        self.assertEqual(p["close2"], [True, ""])
        self.assertEqual((p["away"], p["after"]), (False, [False, False, False]), "the app went and the parked panel stayed")

    def test_the_gears_press_over_an_app_joins_the_quick_panel_to_the_scene_and_a_surface_that_is_up_leaves_it_alone(self):
        panel = "var s = find(overlay, 'footFocus');"
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1300, "warm", "return [overlay.warmPanel('quick'), overlay.warmPanel('picker'), overlay.visible];"),
            # the software renderer's first showing of the overlay's panel compiles its programs, and the sheet's move is a timer of 30 ms
            (2600, "warmed", panel + "return [s.held, s.visible, onscreen(s), overlay.visible, overlay.up, s.slideX];"),
            (2700, "open", "return [overlay.panel('quick'), overlay.surface];"),
            (5200, "up", panel + "return [s.held, s.visible, onscreen(s), s.progress];"),     # (the slide counts frames: read late on a busy machine)
            (5300, "again", "return [overlay.warmPanel('quick'), overlay.surface];"),
        ], 5700)
        self.assertEqual(p["warm"], [True, False, True], "the quick panel is the one warmed, and the layer is in the scene with it")
        self.assertIn(p["warmed"], ([True, True, 0, True, False, 2320], [True, True, 0, True, False, 2321]), "warmed over an app: in the scene, off the screen, the sheet moved once")
        self.assertEqual(p["open"], [True, "quick"])
        self.assertEqual(p["up"][:3], [False, True, 2])
        self.assertGreater(p["up"][3], 0.999, "the slide is over")
        self.assertEqual(p["again"], [False, "quick"], "a panel that is up was warmed")

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
            # the software renderer's first showing of the overlay's picker compiles its programs; the slide counts frames, not time
            (3900, "at", pick + "return {index: s.index, n: s.tiles.length, y: s.sheetY, progress: s.progress};"),
            (4000, "next", pick + "var i = s.index; overlay.panel('picker'); return [i, s.index, overlay.surface];"),
            (4100, "ok", pick + "var t = s.tiles[s.index]; overlay.key(Qt.Key_Return, true, false, 0); return [t.appId, overlay.surface];"),
            (4200, "calls", "return JSON.parse(%s);" % CALLS),
        ], 4900)
        self.assertEqual(p["open"], [True, "picker"])
        self.assertGreater(p["at"]["progress"], 0.999, "the slide is over")
        self.assertAlmostEqual(p["at"]["y"], 612, delta=1)
        self.assertEqual(p["next"][1], (p["next"][0] + 1) % p["at"]["n"])
        app, surface = p["ok"]
        self.assertEqual(surface, "", "the picker stays up after an input was picked")
        got = [c for c in p["calls"] if c["u"].endswith("applicationManager") and c["m"] == "/launch"]
        self.assertEqual(got[-1]["p"], {"id": app}, out[-2000:])

    def test_all_settings_over_an_app_takes_the_tv_home_and_Home_opens_Settings(self):
        p, out = self.overlay_host([
            (1000, "front", "overlay.appFront = true; return 1;"),
            (1200, "open", "overlay.panel('quick'); var s = find(overlay, 'footFocus'); var n = 0; while (s.focusId !== 'ai' && n++ < 12) s.key('down'); s.key('down'); s.key('ok'); return [overlay.surface, s.footFocus];"),
            (1400, "waiting", "return bus().take();"),
            (1500, "calls", "return JSON.parse(%s);" % CALLS),
        ], 2000)
        self.assertEqual(p["open"], ["", True])
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

    def test_a_home_return_cancelled_by_an_app_keeps_the_pending_settings_request(self):
        p, out = self.overlay_host([
            (1000, 'front', 'overlay.appFront = true; return 1;'),
            (1200, 'request', 'root.hostActive = true; bus().want("network"); root.hostActive = false; return root.hostActive;'),
            (2200, 'waiting', 'return bus().take();'),
        ], 2600)
        self.assertFalse(p['request'])
        self.assertEqual(p['waiting'], 'network', out[-2000:])

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
