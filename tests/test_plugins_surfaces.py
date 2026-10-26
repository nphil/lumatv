"""Run with: cd tests && python3 -m unittest test_plugins_surfaces   (needs Xvfb and PyQt5; skipped without them)

The Notification Centre, the plugin cards on Home and the screensaver through the offline QML host against the fake helper.
The drawer: what the helper's audit, a plugin's notifications and the TV's own log put in it, grouped; the D-pad moves down it, OK
runs a button or clears an item, Resume asks the helper (which asks on the TV), Clear all empties it, Back closes, and opening it
is what makes the bell's count zero. Home's At-home lane: plugin cards are chips, OK enters, OK on a chip runs its first action,
and a Home without plugin cards says Home Assistant is not linked yet (nothing invented), OK on it opens Settings. The saver: it comes up on the TV's `Screen Saver` state and only while shown,
no timer runs while it is not, the clock block moves only while it is faded out, any key turns the screen on, the art swaps on its
timetable and skips a print that cannot be loaded. The art on the set is public domain and complete (tools/screensaver_art.py).
Timings are the set's to say (docs/research/plugins-slice-result.md).
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import unittest

from test_settings import HostCase, HOST, READY
from xdisplay import DISPLAY, HAVE_QT

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NOW = int(time.time() * 1000)
SECS = NOW // 1000

BYNAME = "function byName(p, n) { if (p.objectName === n) return p; var k = p.children; for (var i = 0; i < k.length; i++) { var f = byName(k[i], n); if (f) return f } return null }"
BYPROP = "function byProp(p, n) { if (p[n] !== undefined && p.children !== undefined && p.visible !== undefined) return p; var k = p.children; for (var i = 0; i < k.length; i++) { var f = byProp(k[i], n); if (f) return f } return null }"
NOTELOG = ("function noteLog() { return Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\n"
           "QtObject { property int unread: NoteLog.unread; property int n: NoteLog.items.length }', root) }")
GET = ("(function(){var x=new XMLHttpRequest(); x.onreadystatechange=function(){if(x.readyState===4) console.info('[%s] '+x.responseText)};"
       "x.open('GET','http://127.0.0.1:18731/%s'); x.send()})();")


def js(at, body):
    return ["--eval", "%d:(function(){%s return 1})()" % (at, body)]


def probe(at, label, body, find="byName(root, 'under-test')"):
    """An eval at `at` ms that logs `[P:label] <json>`; `s` is the surface, `body` returns what to log."""
    src = ("(function(){%s %s %s try { var s=%s; if(!s) throw Error('no surface'); var r=(function(){%s})(); console.info('[P:%s] '+JSON.stringify(r)) }"
           " catch (err) { console.info('[P:%s] '+JSON.stringify({error: String(err)})) } return 1})()") % (BYNAME, BYPROP, NOTELOG, find, body, label, label)
    return ["--eval", "%d:%s" % (at, src)]


def probes(out):
    return {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[P:(\w+)\] (.*)", out)}


def build(at, path, props):
    """Builds `panels/<path>` from the module directory, the way Layers builds it."""
    return js(at, "var c = Qt.createComponent(root.moduleDirUrl + 'panels/%s'); var o = c.createObject(root, %s); if (!o) throw Error(c.errorString());" % (path, json.dumps(props)))


def plugin(id, name, first_party=False):
    return {"id": id, "name": name, "version": "1", "class": "S", "state": "connected", "registered": True, **({"firstParty": True} if first_party else {})}


def note(id, title, body, actions=()):
    return {"id": id, "channel": "events", "revision": 1, "receivedAt": SECS - 600, "expiresAt": None,
            "post": {"id": id, "channel": "events", "title": title, "body": body, "actions": list(actions)}}


def card(id, title, fact, icon, role, actions=()):
    return {"id": id, "revision": 1, "createdAt": SECS - 600, "expiresAt": None,
            "card": {"id": id, "size": "half", "priority": "new", "open": {"view": "main"}, "actions": list(actions),
                     "content": {"type": "facts", "title": title, "icon": icon, "role": role, "facts": [{"label": "State", "value": fact}]}}}


def publish(at, topic, doc):
    return ["--publish", "%d:%s:%s" % (at, topic, json.dumps(doc))]


def publish_now(at, topic, doc):
    """Publishes `doc` with every item received at that moment: Home counts what arrives after it started as unread (the bell's badge)."""
    return js(at, "var d = %s; d.items.forEach(function(i) { i.receivedAt = Math.ceil(Date.now() / 1000) });"
                  "var x = new XMLHttpRequest(); x.open('POST', 'http://127.0.0.1:18731/__publish/%s'); x.setRequestHeader('Content-Type', 'application/json'); x.send(JSON.stringify(d));"
              % (json.dumps(doc), topic))


AI = {"listener": {"enabled": True, "bound": True, "addr": "192.168.1.242", "port": 18732, "since": NOW - 3600000, "error": None}, "pairing": None,
      "assistants": [{"id": "cody", "name": "Cody", "createdAt": 1, "lastSeenAt": NOW - 120000, "paused": True, "calls": 12,
                      "levels": {"state": "always", "playback": "always", "launch": "ask", "power": "ask"}}],
      "limits": {"maxVolume": 60}, "groups": [], "tools": []}
AUDIT = [{"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_get_now_playing", "message": "what is playing", "outcome": "ok", "at": NOW - 420000},
         {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_standby", "message": "turn the TV off", "outcome": "denied", "at": NOW - 300000}]
ROSTER = {"plugins": [plugin("lock", "Front door")], "firstParty": [plugin("weather", "Weather", True)], "problem": None}
NOTES = {"items": [note("unlocked", "Door unlocked", "Someone opened it.", [{"id": "lock", "label": "Lock it", "action": "lock"}]),
                   note("battery", "Battery low", "The lock's battery is at 12 %.")]}
CARDS = {"items": [card("door", "Front door", "Locked", "base.icons/lock", "positive"),
                   card("living", "Living room", "3 on", "base.icons/lightbulb", "warning", [{"action": "off", "label": "Turn off"}]),
                   card("garage", "Garage door", "Open for 12 min", "base.icons/info", "danger", [{"action": "close", "label": "Close it"}])]}
SAVER = {"on": True, "state": "Screen Saver", "since": NOW, "source": "luna-subscribe", "error": None}
AWAKE = {"on": True, "state": "Active", "since": NOW, "source": "luna-subscribe", "error": None}


class Surfaces(HostCase):
    def run_host(self, theme, *args):
        theme = dict(theme, perf={"syncResidents": True})
        cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", json.dumps(theme), *args]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=240)
        return r.returncode, r.stdout + r.stderr

    MINE = ("Drawer.qml", "Screensaver.qml", "ArtPiece.qml", "PluginHub.qml", "NoteLog.qml", "HomeAssistantWidget.qml", "Widget.qml", "LuButton.qml")

    def own_warnings(self, out, allow=()):
        """The host fails a run for a warning of any file in the tree; what a run of these surfaces decides on is their own."""
        # (the host ignores the Qt 5.15 note about `Connections { onFoo: }`, which is how the set's Qt 5.12 is written: so does this)
        return [l for l in out.splitlines() if l.startswith("[warning]") and "Implicitly defined onFoo" not in l
                and any(f in l for f in self.MINE) and not any(a in l for a in allow)]

    def posts(self, out):
        got = re.findall(r"\[posts\] (\{.*\})", out)
        return json.loads(got[-1])["posts"] if got else []


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Drawer(Surfaces):
    def go(self, title):
        """JS: the D-pad down from the first item to the one titled `title`, one press each, as Layers sends them."""
        return "s.index = 1; for (var guard = 0; guard < 30 && s.items[s.index - 1].title !== %s; guard++) s.key('down');" % json.dumps(title)

    def test_groups_keys_actions_dismiss_clear_and_back(self):
        at = READY - 900
        tv_update = ("var nl = Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\nQtObject { Component.onCompleted: NoteLog.post("
                     "{source: \"TV\", icon: \"download\", title: \"TV update ready\", body: \"webOS 10.2.3\"}) }', root);")
        shape = "return {count: s.count, index: s.index, heads: s.entries.filter(function(e) { return e.head }).map(function(e) { return e.source + ':' + e.count }), titles: s.items.map(function(i) { return i.title })};"
        rc, out = self.run_host(
            {}, *publish(300, "ai", AI), *publish(300, "__audit", AUDIT), *publish(400, "plugins", ROSTER), *publish_now(500, "plugin.lock.notifications", NOTES),
            *js(at - 600, tv_update), *build(at, "Drawer.qml", {"z": 70, "objectName": "under-test"}),
            *probe(at + 200, "before", "var nl = noteLog(); return {unread: nl.unread, n: nl.n};"),
            *js(at + 300, "var s = byName(root, 'under-test'); s.open();" + BYNAME),
            *probe(at + 900, "opened", shape + ""),
            *probe(at + 1000, "seen", "var nl = noteLog(); return {unread: nl.unread};"),
            # a plugin's button: OK on "Door unlocked" is a POST to the plugin's action route
            *probe(at + 1200, "toward", self.go("Door unlocked") + " return s.items[s.index - 1].title;"),
            *js(at + 1300, "byName(root, 'under-test').key('ok');" + BYNAME),
            # an item without buttons: OK clears it
            *probe(at + 1500, "battery", "var before = s.count; " + self.go("Battery low") + " s.key('ok'); return {before: before, after: s.count, titles: s.items.map(function(i) { return i.title })};"),
            # a paused assistant's denial: its one button resumes it, which is a request to the helper
            *probe(at + 1800, "resume", self.go("Cody was denied") + " s.key('right'); return {title: s.items[s.index - 1].title, button: s.button, label: s.items[s.index - 1].actions[0].label};"),
            *js(at + 1900, "byName(root, 'under-test').key('ok');" + BYNAME),
            *js(at + 2200, GET % ("posts", "__posts") + GET % ("requests", "__requests")),
            # Clear all: up to the first row, OK
            *probe(at + 2700, "cleared", "s.index = 1; s.key('up'); var at = s.index; s.key('ok'); var nl = noteLog(); return {at: at, count: s.count, n: nl.n};"),
            *probe(at + 3000, "back", "var hit = 0; s.closeRequested.connect(function() { hit++ }); s.key('back'); return hit;"),
            "--wait", str(at + 3600))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["before"], {"unread": 3, "n": 1}, "the TV's update and the plugin's two notes are unread before the drawer opens")
        self.assertEqual(p["opened"]["index"], 1)
        self.assertEqual(p["opened"]["heads"], ["AI access:2", "Front door:2", "TV:1"], out[-3000:])
        self.assertEqual(p["opened"]["count"], 5)
        self.assertEqual(p["opened"]["titles"][0], "Cody was denied", "the audit is newest first")
        self.assertEqual(p["seen"], {"unread": 0}, "opening the drawer is what the bell's count is cleared by")
        self.assertEqual(p["toward"], "Door unlocked")
        self.assertEqual(p["battery"]["after"], p["battery"]["before"] - 1)
        self.assertNotIn("Battery low", p["battery"]["titles"])
        self.assertEqual((p["resume"]["title"], p["resume"]["button"], p["resume"]["label"]), ("Cody was denied", 0, "Resume Cody"))
        self.assertIn(["/v1/plugins/lock/actions/lock", {}], self.posts(out), "the plugin's own action, through the broker")
        requests = re.findall(r"\[requests\] (\{.*\})", out)
        self.assertIn(["POST", "/v1/ai/assistants/cody/resume", {}], json.loads(requests[-1])["requests"] if requests else [], "resume goes to the helper")
        self.assertEqual(p["cleared"]["at"], 0, "Up from the first item is the Clear all button")
        self.assertEqual((p["cleared"]["count"], p["cleared"]["n"]), (0, 0), "nothing is left, and the TV's own log is empty too")
        self.assertEqual(p["back"], 1)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Lane(Surfaces):
    def test_ok_enters_the_chips_and_runs_the_focused_ones_first_action(self):
        focus = "root.widgetFocusId = 'home'; root.focusLayer = 'widgets';"
        lane = "var w = byProp(root, 'pluginMode'); return {cards: w.cards.map(function(c) { return c.id }), mode: w.pluginMode, inner: w.inner, index: w.index};"
        rc, out = self.run_host(
            {}, *publish(300, "plugins", ROSTER), *publish(300, "plugin.lock.cards", CARDS),
            *publish(300, "plugin.weather.cards", {"items": [card("weather", "Weather", "46", "base.icons/moon", "info")]}),
            *js(READY - 600, focus),
            *probe(READY, "start", lane, find="root"),
            *self.keys(READY + 200, "ok"), *probe(READY + 500, "entered", lane, find="root"),
            *self.keys(READY + 700, "ok"),                      # the first chip, "Front door", has no action: nothing is sent
            *self.keys(READY + 900, "right", "ok"),             # the second one, "Living room": its first action
            *self.keys(READY + 1200, "down", "ok"),             # third (down from the second is the row below: the garage door)
            *probe(READY + 1500, "later", lane, find="root"),
            *self.keys(READY + 1600, "back"), *probe(READY + 1900, "left", lane, find="root"),
            *js(READY + 2000, GET % ("posts", "__posts")),
            "--wait", str(READY + 2600))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["start"]["cards"], ["door", "living", "garage"], "the weather card is Home's own widget, not a chip")
        self.assertTrue(p["start"]["mode"])
        self.assertFalse(p["start"]["inner"])
        self.assertTrue(p["entered"]["inner"], "OK enters the chips")
        self.assertEqual(p["entered"]["index"], 0)
        self.assertFalse(p["left"]["inner"], "Back leaves them")
        sent = [(path, body) for path, body in self.posts(out) if path.startswith("/v1/plugins/")]
        self.assertEqual(sent, [("/v1/plugins/lock/actions/off", {}), ("/v1/plugins/lock/actions/close", {})], str(sent))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class LaneEmpty(Surfaces):
    STATE = ("var w = byProp(root, 'noneTitle'); var st = byProp(root, 'railIndex');"
             "return {title: w.noneTitle, sub: w.noneSub, mode: w.pluginMode, focused: w.focused, inner: w.inner, has_key: typeof w.key === 'function',"
             " focus: root.widgetFocusId, layer: root.focusLayer, settings: root.settingsOpen, zone: st.zone, current: st.current};")

    def test_it_says_home_assistant_is_not_linked_and_ok_opens_settings(self):
        # from the clock card down to the At-home card, the way the remote does it, then OK, then Back
        rc, out = self.run_host(
            {}, *js(READY - 600, "root.widgetFocusId = 'clock'; root.focusLayer = 'widgets';"),
            *probe(READY - 100, "before", self.STATE, find="root"),
            *self.keys(READY, "down"), *probe(READY + 300, "arrived", self.STATE, find="root"),
            *self.keys(READY + 400, "ok"), *probe(READY + 900, "opened", self.STATE, find="root"),
            *self.keys(READY + 1000, "back"), *probe(READY + 1500, "closed", self.STATE, find="root"),
            *js(READY + 1800, GET % ("posts", "__posts")),
            "--wait", str(READY + 2400))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["before"]["title"], "Home Assistant isn't linked yet", "no card, no made-up house")
        self.assertEqual(p["before"]["sub"], "Link it in Settings \u203a Home Assistant", "and where to link it")
        self.assertFalse(p["before"]["mode"])
        self.assertFalse(p["before"]["has_key"], "no chips: OK does not enter an inner layer")
        self.assertEqual(p["before"]["focus"], "clock")
        self.assertEqual((p["arrived"]["focus"], p["arrived"]["layer"]), ("home", "widgets"), "the lane can be reached with Down from the clock")
        self.assertTrue(p["arrived"]["focused"])
        self.assertFalse(p["arrived"]["settings"])
        self.assertTrue(p["opened"]["settings"], "OK on it opens Settings")
        self.assertFalse(p["opened"]["inner"])
        self.assertFalse(p["closed"]["settings"], "Back closes Settings and is Home again")
        sent = [path for path, _ in self.posts(out) if path.startswith("/v1/plugins/")]
        self.assertEqual(sent, [], "nothing is asked of a plugin when there is none")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Saver(Surfaces):
    def state(self, label, at):
        return probe(at, label, "var d = JSON.parse(s.snapshot()); return {up: d.up, active: d.active, timers: d.timers, fade: d.clock.fade};")

    def test_it_comes_up_on_the_power_state_only_while_shown_and_nothing_runs_otherwise(self):
        at = READY - 900
        rc, out = self.run_host(
            {"screensaver": {"style": "clock"}}, *publish(300, "screen", AWAKE),
            *build(at, "Screensaver.qml", {"z": 95, "objectName": "under-test", "visible": False}),
            *self.state("awake", READY),
            *publish(READY + 200, "screen", SAVER), *self.state("hidden", READY + 800),         # the state says saver, nothing is shown: no timer
            *js(READY + 900, "byName(root, 'under-test').visible = true;" + BYNAME), *self.state("shown", READY + 1400),
            *publish(READY + 1500, "screen", AWAKE), *self.state("woken", READY + 2000),
            *js(READY + 2100, "byName(root, 'under-test').visible = false;" + BYNAME), *publish(READY + 2200, "screen", SAVER), *self.state("again", READY + 2700),
            "--wait", str(READY + 3200))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual((p["awake"]["up"], p["awake"]["active"], p["awake"]["timers"]), (False, False, False))
        self.assertEqual((p["hidden"]["up"], p["hidden"]["active"], p["hidden"]["timers"]), (False, True, False), "hidden: active in name only, no timer")
        self.assertEqual((p["shown"]["up"], p["shown"]["timers"]), (True, True))
        self.assertEqual((p["woken"]["up"], p["woken"]["timers"]), (False, False), "the TV woke: everything stopped")
        self.assertEqual((p["again"]["up"], p["again"]["timers"]), (False, False))

    def test_the_clock_block_moves_only_while_it_is_faded_out_and_any_key_turns_the_screen_on(self):
        at = READY - 900
        read = ("function sampler(p) { var k = p.children; for (var i = 0; i < k.length; i++) { if (k[i].visibleMoves !== undefined) return k[i]; var f = sampler(k[i]); if (f) return f } return null }"
                "var w = sampler(root); return {moves: w.moves, seen: w.seen, visibleMoves: w.visibleMoves};")
        rc, out = self.run_host(
            {"screensaver": {"style": "clock"}}, *publish(300, "screen", AWAKE),
            *build(at, "Screensaver.qml", {"z": 95, "objectName": "under-test", "moveEveryMs": 2500, "fadeMs": 600, "standbyMs": 6000}),
            *publish(at + 100, "screen", SAVER),
            *js(at + 600, BYNAME + "var w = Qt.createQmlObject('import QtQuick 2.4\\nItem { property var s; property real fx: -1; property real fy: -1; property int moves: 0; property int seen: 0; property int visibleMoves: 0\\n"
                "Timer { interval: 40; repeat: true; running: true; onTriggered: { var c = JSON.parse(s.snapshot()).clock; if (fx >= 0 && (c.fx !== fx || c.fy !== fy)) { moves++; if (c.opacity > 0.02) visibleMoves++ } fx = c.fx; fy = c.fy; seen++ } } }', root); w.s = byName(root, 'under-test');"),
            *probe(at + 9000, "moves", read, find="root"),
            *js(at + 9100, "byName(root, 'under-test').key('left');" + BYNAME),
            *js(at + 9200, "console.info('[calls] ' + JSON.stringify(root.lunaCalls.filter(function(c) { return c.uri.indexOf('tvpower') > 0 }).map(function(c) { return c.method })));"),
            "--wait", str(at + 9800))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertGreater(p["moves"]["seen"], 100, "the sampler ran")
        self.assertGreaterEqual(p["moves"]["moves"], 2, "it moved at least twice in nine seconds at one move every 2.5 s")
        self.assertEqual(p["moves"]["visibleMoves"], 0, "never a move that can be seen: it is faded out when it changes place")
        # six seconds in the saver, the TV is put in standby (once), and a key asks it to turn the screen on
        self.assertIn('[calls] ["/power/powerOff","/power/turnOnScreen"]', out, out[-1500:])

    def test_the_art_follows_its_timetable_and_skips_a_print_it_cannot_load(self):
        at = READY - 900
        with tempfile.TemporaryDirectory() as tmp:
            index = os.path.join(tmp, "index.json")
            pics = [{"id": "first", "file": "../icons/lucide/bell.png"}, {"id": "broken", "file": "no-such-print.jpg"}, {"id": "third", "file": "../icons/lucide/camera.png"}]
            with open(index, "w") as f:
                json.dump({"pictures": [dict(p, licence="Public domain", artist="A", title="T", year="1900", width=64, height=64) for p in pics]}, f)
            snap = "var d = JSON.parse(s.snapshot()); return {up: d.up, timers: d.timers, shown: d.art.shown, current: d.art.current, skipped: d.art.skipped, idle: d.art.idle};"
            rc, out = self.run_host(
                {"screensaver": {"style": "art", "art": {"order": "sequence"}}}, *publish(300, "screen", AWAKE),
                *build(at, "Screensaver.qml", {"z": 95, "objectName": "under-test", "periodMs": 2500, "leadMs": 1200, "indexUrl": "file://" + index}),
                *publish(at + 100, "screen", SAVER),
                *probe(at + 900, "first", snap),
                *probe(at + 2800, "skipping", snap),
                *[a for i in range(12) for a in probe(at + 3200 + 450 * i, "t%d" % i, snap)],
                *publish(at + 8800, "screen", AWAKE), *probe(at + 9500, "woken", snap),
                "--wait", str(at + 10000))
        # Qt warns each time it tries the print that is not there: those warnings, and no other of these surfaces
        self.assertEqual(self.own_warnings(out, allow=("no-such-print.jpg",)), [], out[-3000:])
        self.assertIn("no-such-print.jpg", out)
        p = probes(out)
        self.assertEqual((p["first"]["shown"], p["first"]["current"]), ("first", 0), "the first print in the owner's order is shown at once")
        self.assertEqual(p["skipping"]["skipped"], 1, "the print that cannot be loaded was passed over...")
        self.assertEqual(p["skipping"]["idle"], "third", "...for the one after it, not tried again")
        seen = [p["t%d" % i]["shown"] for i in range(12)]
        self.assertIn("third", seen, "it got to the third print: %s" % seen)
        self.assertNotIn("broken", seen, "the print that cannot be loaded was never shown")
        self.assertEqual(seen[seen.index("third") + 1:].count("first") > 0, True, "and the order went on round to the first again: %s" % seen)
        self.assertEqual((p["woken"]["up"], p["woken"]["timers"], p["woken"]["shown"], p["woken"]["current"]), (False, False, "", -1), "let go of everything when it goes")


class Art(unittest.TestCase):
    def test_every_print_on_the_set_is_public_domain_and_is_there(self):
        r = subprocess.run([sys.executable, os.path.join(REPO, "tools", "screensaver_art.py"), "--check"], capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
