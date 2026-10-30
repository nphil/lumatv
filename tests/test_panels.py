"""Run with: cd tests && python3 -m unittest test_panels   (needs Xvfb and PyQt5; skipped without them)

The quick panel, the input picker and the receiver's readings, through the offline QML host against the fake helper:
a receiver value is shown while it is in date and gone 6 s after the helper's last answer on the quick panel's tile and
on Settings' receiver page (the OSD's own cases are test_volume_osd), a plugin's tile is reached with the D-pad and its
OK is a POST to the plugin's action route (a tile that is not available is not reached at all), the plugin tiles that come
and go do not move the focus, and the input picker draws the TV's inputs by the owner's choices and launches the one picked.
"""
import json
import re
import unittest

from test_settings import HostCase, READY, RAIL
from xdisplay import DISPLAY, HAVE_QT

FIND = """function find(p, name) {
    if (p[name] !== undefined && p.children !== undefined && p.visible !== undefined) return p;
    for (var i=0; i<p.children.length; i++) { var f=find(p.children[i], name); if (f) return f; }
    return null;
}"""
PUBLISH = """var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/receiver');
x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(%s));"""
POSTS = """var x=new XMLHttpRequest(); x.onreadystatechange=function(){if(x.readyState===4) console.info('[posts] '+x.responseText)};
x.open('GET','http://127.0.0.1:18731/__posts'); x.send();"""

RX = {"enabled": True, "host": "192.168.1.93", "polling": True, "reachable": True, "fresh": True, "power": "on", "input": "TV Audio",
      "volumeDb": -34.5, "volumeAbs": 45.5, "volumeDisplay": "relative", "volumeText": "-34.5 dB", "mute": False, "model": "Denon AVR-S760H",
      "soundMode": "Dolby Surround"}
ROSTER = {"plugins": [{"id": "ambient", "name": "Ambient light", "state": "connected"}, {"id": "lock", "name": "Front door", "state": "connected"}]}
TILES = {"ambient": {"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle", "hint": "Follows the picture"}]},
         "lock": {"items": [{"id": "door", "label": "Front door", "icon": "base.icons/lock", "state": {"type": "unavailable", "reason": "offline"}, "action": "lock"},
                            {"id": "scene", "label": "Movie scene", "icon": "base.icons/home", "state": {"type": "unknown"}, "action": "run", "target": "movie"}]}}


def publish_args(tiles=TILES, at=800):
    out = ["--publish", "%d:plugins:%s" % (at, json.dumps(ROSTER))]
    for pid, doc in tiles.items():
        out += ["--publish", "%d:plugin.%s.tiles:%s" % (at, pid, json.dumps(doc))]
    return out


def probe(at, label, find, body):
    """An eval at `at` ms that logs `[P:label] <json>`; `find` is the property that names the surface, `s` is the surface."""
    js = "(function(){%s var s=find(root,%s); if(!s) throw Error('no %s'); var r=(function(){%s})(); console.info('[P:%s] '+JSON.stringify(r)); return 1;})()" % (FIND, json.dumps(find), find, body, label)
    return ["--eval", "%d:%s" % (at, js)]


def probes(out):
    return {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[P:(\w+)\] (.*)", out)}


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Receiver(HostCase):
    def reading(self, at):
        """An eval that publishes the receiver as read right now."""
        doc = json.dumps(RX)
        return ["--eval", "%d:(function(){var d=%s; d.updatedAt=Date.now(); %s return 1})()" % (at, doc, PUBLISH % "d")]

    def test_the_quick_panels_receiver_tile_hides_a_reading_six_seconds_after_the_last_and_shows_the_next(self):
        tile = "var t=s.tiles()[1]; return {answers:s.rcAnswers, value:t.value(), detail:t.detail(), enabled:t.enabled()};"
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400), *self.reading(READY + 600),
                            *probe(READY + 1200, "fresh", "footFocus", tile),
                            *probe(READY + 7000, "old", "footFocus", tile),            # 6.4 s after the reading, nothing newer
                            *self.reading(READY + 7200), *probe(READY + 7700, "again", "footFocus", tile),
                            "--wait", str(READY + 8200))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual((p["fresh"]["answers"], p["fresh"]["value"], p["fresh"]["detail"]), (True, "-34.5 dB", "TV Audio \u00b7 Dolby Surround"), out[-2500:])
        self.assertEqual((p["old"]["answers"], p["old"]["value"], p["old"]["detail"]), (False, "\u2013", "Not answering"))
        self.assertEqual((p["again"]["answers"], p["again"]["value"]), (True, "-34.5 dB"))

    def test_settings_receiver_page_says_not_answering_six_seconds_after_the_last_reading(self):
        page = "return {answers:s.answers, chip:s.chip, summary:s.summary, volume:s.rc.volumeText || null};"
        rc, out = self.host("--eval", "%d:root.openSettings()" % 1500, *self.keys(READY + 600, *(["down"] * RAIL["sound"]), "right", "down", "ok"),
                            *self.reading(READY + 1500), *probe(READY + 2200, "fresh", "ctl", page),
                            *probe(READY + 8000, "old", "ctl", page),
                            *self.reading(READY + 8200), *probe(READY + 8700, "again", "ctl", page),
                            "--wait", str(READY + 9200))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual((p["fresh"]["answers"], p["fresh"]["chip"], p["fresh"]["volume"]), (True, "Answering", "-34.5 dB"), out[-2500:])
        self.assertEqual((p["old"]["answers"], p["old"]["chip"], p["old"]["volume"]), (False, "Not answering", None))
        self.assertEqual((p["again"]["answers"], p["again"]["volume"]), (True, "-34.5 dB"))

    def test_a_reading_the_helper_still_calls_fresh_is_not_shown_once_its_own_time_is_old(self):
        # the helper that has stopped leaves `fresh: true` in the topic for ever; the panel ages it by `updatedAt`
        stale = ["--eval", "%d:(function(){var d=%s; d.updatedAt=Date.now()-7000; %s return 1})()" % (READY + 600, json.dumps(RX), PUBLISH % "d")]
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400), *stale,
                            *probe(READY + 1200, "old", "footFocus", "return {answers:s.rcAnswers, value:s.tiles()[1].value()};"), "--wait", str(READY + 1600))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["old"], {"answers": False, "value": "\u2013"}, out[-2500:])

    def test_a_stamp_the_helper_moves_quietly_keeps_the_reading_and_a_silent_helper_loses_it(self):
        # The helper moves `updatedAt` without an event while nothing changes. The copy the panel got from the one event
        # ages out at 6 s; the panel asks the helper and takes the newer stamp, so the tile stays up while the helper keeps
        # reading, and goes when the stamps stop (the last one was at +8.6 s, so it is gone by +14.6 s).
        quiet = lambda at: ["--eval", "%d:(function(){var d=%s; d.updatedAt=Date.now(); var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__quiet/receiver');"
                            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % (at, json.dumps(RX))]
        tile = "return {answers:s.rcAnswers, value:s.tiles()[1].value()};"
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400), *self.reading(READY + 600),
                            *quiet(READY + 4600), *quiet(READY + 8600),
                            *probe(READY + 9600, "a", "footFocus", tile), *probe(READY + 12000, "b", "footFocus", tile),
                            *probe(READY + 15600, "c", "footFocus", tile), "--wait", str(READY + 16000))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual((p["a"]["answers"], p["a"]["value"]), (True, "-34.5 dB"), out[-2500:])
        self.assertEqual((p["b"]["answers"], p["b"]["value"]), (True, "-34.5 dB"), out[-2500:])
        self.assertEqual((p["c"]["answers"], p["c"]["value"]), (False, "\u2013"), out[-2500:])

    def test_the_receivers_answer_changes_no_rows_height(self):
        # the answer arrives after the panel is up (the helper's reading ages out between openings): "Not answering" and the answer each take
        # the same two lines, so the rows below do not move while the panel is sliding in
        tall = "var l = find(root, 'itemFor'); return [l.itemFor(1).height, l.itemFor(2).y];"
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400),
                            *probe(READY + 1000, "silent", "itemFor", tall),
                            *self.reading(READY + 1200),
                            *probe(READY + 1900, "answered", "itemFor", tall),
                            "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["silent"], p["answered"], out[-2500:])

    def test_the_tile_moves_the_volume_through_the_tv_and_asks_the_helper_to_read_now(self):
        go = "var n=0; while (s.focusId !== 'receiver' && n++ < 8) s.key('down'); s.key('right'); return s.focusId;"
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400), *self.reading(READY + 600),
                            *probe(READY + 1000, "went", "footFocus", go),
                            "--eval", "%d:(function(){%s return 1})()" % (READY + 1400, POSTS),
                            "--eval", "%d:JSON.stringify(root.lunaCalls.filter(function(c){return c.uri==='luna://com.webos.service.audio'}).map(function(c){return c.method}))" % (READY + 1400),
                            "--wait", str(READY + 2000))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["went"], "receiver")
        self.assertEqual(json.loads(self.values(out)[-1]), ["/master/volumeUp"])
        self.assertIn(["/v1/receiver/nudge", {}], json.loads(re.findall(r"\[posts\] (.*)", out)[-1])["posts"])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class QuickPanel(HostCase):
    def walk(self, until, limit=10):
        """JS that presses Down until the focus is on `until`, recording each stop in order."""
        return "var seen=[s.focusId], n=0; while (s.focusId !== %s && n++ < %d) { s.key('down'); seen.push(s.focusId); } return seen;" % (json.dumps(until), limit)

    def test_a_plugins_tile_is_reached_and_ok_posts_its_action_and_nothing_else_does(self):
        rc, out = self.host(*publish_args(), "--eval", "%d:root.openLayer('quick')" % (READY + 400),
                            *probe(READY + 1000, "walk", "footFocus", self.walk("ai") + ""),
                            *probe(READY + 1200, "back", "footFocus", "s.key('up'); return s.focusId;"),       # up from AI access: the movie scene tile
                            *probe(READY + 1400, "press", "footFocus", "s.key('ok'); return s.focusId;"),
                            *probe(READY + 1600, "up", "footFocus", "s.key('up'); return s.focusId;"),         # the ambient tile (a toggle)
                            *probe(READY + 1800, "toggle", "footFocus", "s.key('ok'); return s.focusId;"),
                            "--eval", "%d:(function(){%s return 1})()" % (READY + 2400, POSTS), "--wait", str(READY + 2800))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        # Down visits the tiles in order; the front door tile is not available and never takes the focus
        self.assertEqual(p["walk"][-1], "ai", out[-2500:])
        self.assertNotIn("plugin.lock.door", p["walk"])
        self.assertIn("plugin.ambient.ambient", p["walk"])
        self.assertEqual((p["back"], p["press"], p["up"], p["toggle"]), ("plugin.lock.scene", "plugin.lock.scene", "plugin.ambient.ambient", "plugin.ambient.ambient"))
        posts = json.loads(re.findall(r"\[posts\] (.*)", out)[-1])["posts"]
        self.assertEqual([x for x in posts if x[0].startswith("/v1/plugins")],
                         [["/v1/plugins/lock/actions/run", {"target": "movie"}], ["/v1/plugins/ambient/actions/toggle", {}]], out[-2500:])

    def test_tiles_that_come_and_go_keep_the_focus_on_the_tile_it_was_on(self):
        more = {"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle"},
                          {"id": "mood", "label": "Mood", "icon": "base.icons/light", "state": {"type": "off"}, "action": "toggle"}]}
        add = ["--eval", "%d:(function(){var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/plugin.ambient.tiles');"
               "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(%s)); return 1})()" % (READY + 1200, json.dumps(more))]
        rc, out = self.host(*publish_args(), "--eval", "%d:root.openLayer('quick')" % (READY + 400),
                            *probe(READY + 1000, "before", "footFocus", self.walk("ai")),
                            *add,
                            *probe(READY + 1800, "after", "footFocus",
                                   "var l=find(s,'tabs'); return {id:s.focusId, row:l.rows[l.stop].id, ids:l.rows.map(function(r){return r.id})};"),
                            "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["before"][-1], "ai", out[-2500:])
        self.assertEqual((p["after"]["id"], p["after"]["row"]), ("ai", "ai"), out[-2500:])
        self.assertIn("plugin.ambient.mood", p["after"]["ids"])

    def test_down_past_the_last_tile_is_the_all_settings_button_and_ok_opens_settings(self):
        rc, out = self.host("--eval", "%d:root.openLayer('quick')" % (READY + 400),
                            *probe(READY + 1000, "foot", "footFocus", self.walk("ai") + ""),
                            *probe(READY + 1200, "on", "footFocus", "s.key('down'); return [s.footFocus, s.zone];"),
                            *probe(READY + 1400, "off", "footFocus", "s.key('up'); return [s.footFocus, s.zone, s.focusId];"),
                            *probe(READY + 1600, "on2", "footFocus", "s.key('down'); s.key('ok'); return s.footFocus;"),
                            "--eval", "%d:root.settingsOpen" % (READY + 2600), "--wait", str(READY + 3000))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["on"], [True, "foot"])
        self.assertEqual(p["off"], [False, "page", "ai"])
        self.assertEqual(self.values(out)[-1], "true", "OK on All settings did not open Settings")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class InputPicker(HostCase):
    TILES = "return s.tiles.map(function(t){return [t.port, t.icon, t.name, t.connected, !!t.front, !!t.sound]});"

    def run_picker(self, theme, *extra):
        return self.host("--theme", json.dumps(dict(theme, perf={"syncResidents": True})), "--eval", "%d:root.openLayer('picker')" % (READY + 400),
                         *probe(READY + 1000, "tiles", "soundLine", self.TILES), *extra, "--wait", str(READY + 2400))

    def test_tiles_are_the_tvs_inputs_with_the_icon_of_what_is_on_them(self):
        rc, out = self.run_picker({})
        self.assertEqual(rc, 0, out[-2500:])
        tiles = probes(out)["tiles"]
        # Auto: Live TV and the three connected ports; the empty HDMI 4 is not there
        self.assertEqual(tiles, [["LIVE TV", "antenna", "Antenna", True, False, False],
                                 ["HDMI 1", "tv-minimal-play", "Apple TV 4K", True, False, False],
                                 ["HDMI 2 \u00b7 eARC", "speaker", "Denon AVR-S760H", True, True, True],
                                 ["HDMI 3", "gamepad-2", "PlayStation 5", True, False, False]], out[-2500:])

    def test_always_shows_an_empty_port_and_hidden_removes_one(self):
        rc, out = self.run_picker({"inputs": {"hdmi4": "always", "hdmi3": "hidden", "livetv": "hidden"}})
        self.assertEqual(rc, 0, out[-2500:])
        tiles = probes(out)["tiles"]
        self.assertEqual([t[0] for t in tiles], ["HDMI 1", "HDMI 2 \u00b7 eARC", "HDMI 4"])
        self.assertEqual(tiles[2][1:4], ["unplug", "Nothing connected", False])

    def test_it_opens_on_the_input_in_front_and_ok_launches_the_one_picked(self):
        go = "s.key('right'); s.key('ok'); return s.index;"
        rc, out = self.run_picker({}, *probe(READY + 1300, "pick", "soundLine", go),
                                  "--eval", "%d:JSON.stringify(root.lunaCalls.filter(function(c){return c.method==='/launch'}).map(function(c){return JSON.parse(c.payload).id}))" % (READY + 1800))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["pick"], 3)                   # the front input is HDMI 2 (index 2); Right is HDMI 3
        self.assertEqual(json.loads(self.values(out)[-1]), ["com.webos.app.hdmi3"])

    def test_the_sound_line_names_the_receiver_and_the_way_out(self):
        rc, out = self.run_picker({}, *probe(READY + 1300, "line", "soundLine", "return s.soundLine;"))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["line"], "Sound: Denon (HDMI ARC)")


if __name__ == "__main__":
    unittest.main()
