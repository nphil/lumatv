"""Run with: python3 -m unittest tests.test_settings   (needs Xvfb and PyQt5; skipped without them)

LumaTV's Settings (payload/compositor/lumatv/Settings.qml and settings/) through the offline host: Home builds it
in the background once it has presented, opening it only flips it on, the D-pad moves through the rail and the
pages, a switch writes the theme through Home's own patch and persist (the user's theme.json in the data
directory) or the TV's settings service (the host's fake one, read back through the luna calls it recorded), Back
leaves it, and the pages draw the state of the helper (a fake one serves the topics). Timing and Qt 5.12 are
the set's to say.
"""
import ast
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")
ROOT = os.path.dirname(HERE)

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


class HostCase(unittest.TestCase):
    CODES = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back"}

    def keys(self, at, *names):
        """The D-pad as Home gets it from the key filter (`key(code, pressed, autoRepeat, device)`), one press and release each."""
        body = "".join("root.key(%s,true,false,0);root.key(%s,false,false,0);" % (self.CODES[n], self.CODES[n]) for n in names)
        return ["--eval", "%d:(function(){%s return 1})()" % (at, body)]

    def host(self, *args, data=None, surface="LumaHome.qml"):
        cmd = [sys.executable, HOST, surface, "--fake-helper", "--prop", "hostActive=true", "--theme", '{"perf":{"syncResidents":true}}', *args]
        if data:
            cmd += ["--data", data]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=180)
        return r.returncode, r.stdout + r.stderr

    def docs(self, out):
        """Each `--eval` that returned a JSON string, parsed (the host prints `[eval] ('<text>', True)`)."""
        found = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m:
                v = ast.literal_eval(m.group(1))
                if isinstance(v, str) and v[:1] in "[{":
                    found.append(json.loads(v))
        return found

    def tagged(self, out, tag):
        """What an `--eval` script logged as `[tag] <json>`."""
        return [json.loads(m.group(1)) for m in re.finditer(r"\[%s\] (\{.*\}|\[.*\])" % tag, out)]

    def values(self, out):
        vals = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m:
                v = m.group(1)
                vals.append((v[1:-1] if v[:1] in "'\"" else v).lower() if v in ("True", "False") else (v[1:-1] if v[:1] in "'\"" else v))
        return vals


# The rail, in order: picture, sound, inputs, network, ambient, look, remote, ai, system, then LG's Settings.
RAIL = {"picture": 0, "sound": 1, "inputs": 2, "network": 3, "ambient": 4, "look": 5, "remote": 6, "ai": 7, "system": 8, "lg": 9}
OPEN_AT = 1500          # Home asks for Settings
READY = 6000            # Home has presented, built the chain (Settings and its pages) and warmed up
LUNA = "JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/setSystemSettings' }).map(function(c) { return JSON.parse(c.payload) }))"


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Settings(HostCase):
    def go(self, section, *more):
        """Keys that open Settings and take the D-pad to `section`'s page."""
        return [*self.keys(READY + 600, *(["down"] * RAIL[section]))] + ([*self.keys(READY + 1500, *more)] if more else [])

    def test_home_opens_settings_and_it_loads_without_a_warning(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, "--eval", "%d:root.settingsOpen" % (READY + 400), "--wait", str(READY + 1200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true")
        self.assertNotIn("did not load", out)

    def test_every_page_is_built_with_it_and_the_rail_shows_each_pages_own_summary(self):
        # Settings on its own, the pages built in the background: every page loaded and each rail section has the line its page keeps
        rc, out = self.host("--eval", "%d:JSON.stringify({loaded: root.loaded, pages: root.pages.length, built: root.built, summaries: root.summaries})" % 5000,
                            "--wait", "5600", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertNotIn("did not load", out)
        doc = self.docs(out)[-1]
        self.assertTrue(doc["built"])
        self.assertEqual(doc["loaded"], doc["pages"])
        s = doc["summaries"]
        self.assertEqual(s["picture"], "Filmmaker \u00b7 Brightness 80")                 # the fake TV's picture mode and OLED pixel brightness
        self.assertEqual(s["sound"], "Receiver \u00b7 eARC")
        self.assertEqual(s["network"], "Nate's Home \u00b7 Strong")
        self.assertEqual(s["ambient"], "Off")                                           # the helper's base.ambient default
        self.assertEqual(s["remote"], "Pointer on \u00b7 Normal speed")
        self.assertEqual(s["system"], "webOS \u00b7 Living Room TV")
        for section in ("inputs", "look", "ai"):
            self.assertTrue(s.get(section), section)

    def test_a_switch_changes_the_theme_and_is_written_to_the_user_file(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # Home & look: the wallpaper row only shows, so the first stop is Dynamic tint; RIGHT enters the page, OK flips it
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "ok"),
                                "--eval", "%d:root.theme.accent" % (READY + 3000), "--wait", str(READY + 3600), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "wallpaper")
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f).get("accent"), "wallpaper")

    def test_down_and_left_move_inside_a_segmented_row_and_ok_picks(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # page, down to Appearance (Dark | Light | Auto), Auto is current, Left to Light, OK
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "down", "left", "ok"),
                                "--eval", "%d:root.theme.material" % (READY + 3000), "--wait", str(READY + 3600), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "light")

    def test_the_tabs_switch_without_building_anything(self):
        # Home & look has four tabs; Up from the first row is the tab bar, Right on it the next tab, Down the first row of that tab
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "up", "right", "right", "down", "right", "ok"),
                            "--eval", "%d:JSON.stringify(root.theme.screensaver || {})" % (READY + 3200), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        # Screensaver tab, first row (style Clock | Art | Off): Right moves to Art, OK picks it
        self.assertNotIn("did not load", out)
        self.assertEqual(self.docs(out)[-1]["style"], "art")

    def test_a_slider_writes_the_tvs_setting_with_the_dimension_it_was_read_for(self):
        # Picture, Mode tab: the first stop is the picture mode cards (one scrolling row), then OLED pixel brightness (80 in the fake service)
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("picture", "right", "down", "right", "right"),
                            "--eval", "%d:%s" % (READY + 3200, LUNA), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        calls = json.loads(re.findall(r"\[eval\] \('(\[.*\])'", out)[-1])
        sent = [c for c in calls if c["category"] == "picture" and "backlight" in c["settings"]]
        self.assertEqual([c["settings"]["backlight"] for c in sent], ["81", "82"])
        self.assertEqual(sent[0]["dimension"]["pictureMode"], "filmMaker")

    def test_a_row_the_service_does_not_answer_is_dimmed_and_does_not_take_focus(self):
        # Sound: Receiver volume and Sound mode only show, so Down from Sound out lands on Match screen and sound, then on Digital
        # sound output (Pass through now): Left picks PCM
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("sound", "right", "down", "down", "left", "ok"),
                            "--eval", "%d:%s" % (READY + 3200, LUNA), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        calls = json.loads(re.findall(r"\[eval\] \('(\[.*\])'", out)[-1])
        self.assertEqual([c["settings"] for c in calls if c["category"] == "sound"], [{"soundOutputDigital": "pcm"}])

    def test_back_from_the_rail_closes_it_and_back_from_the_page_returns_to_the_rail(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, "right", "back"),
                            "--eval", "%d:root.settingsOpen" % (READY + 1600), "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true")      # still open: Back only left the page
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, "back"),
                            "--eval", "%d:root.settingsOpen" % (READY + 1600), "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "false")

    def test_keys_go_to_settings_and_not_to_the_home_behind_it(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, "--eval", "%d:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')" % (READY + 400),
                            *self.keys(READY + 800, "down", "down", "left", "up"),
                            "--eval", "%d:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')" % (READY + 2000), "--wait", str(READY + 2600))
        self.assertEqual(rc, 0, out[-1500:])
        vals = self.values(out)                 # openSettings, before, the keys, after
        self.assertEqual(vals[1], vals[3], out[-800:])

    def test_the_rail_ends_with_the_way_to_LGs_settings(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, *(["down"] * RAIL["lg"]), "ok"),
                            "--eval", "%d:root.settingsOpen" % (READY + 3000), "--wait", str(READY + 3600))
        self.assertEqual(rc, 0, out[-1500:])
        # the last row of the rail launches LG's app and closes Settings
        self.assertEqual(self.values(out)[-1], "false")

    def test_art_options_carry_the_whole_art_block_and_show_only_with_the_art_style(self):
        # Screensaver tab: style Art, then (afterMin, dim) down to "Change the art every" (15 min now): Right picks 1 hour
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT,
                            *self.go("look", "right", "up", "right", "right", "down", "right", "ok"),
                            *self.keys(READY + 3000, "down", "down", "down", "right", "ok"),
                            "--eval", "%d:JSON.stringify(root.theme.screensaver)" % (READY + 4200), "--wait", str(READY + 4800))
        self.assertEqual(rc, 0, out[-1500:])
        s = self.docs(out)[-1]
        self.assertEqual(s["style"], "art")
        self.assertEqual(s["art"], {"everyMin": 60, "order": "shuffle", "caption": True})      # one change, the other two kept

    def test_the_power_on_and_off_logos_are_chosen_among_three_and_kept_in_the_theme_file(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # Standby tab: Power-on logo (Panes now), Right to Arc, OK; Down to Power-off logo, Right twice to Name only, OK
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT,
                                *self.go("look", "right", "up", "right", "right", "right", "down", "right", "ok"),
                                *self.keys(READY + 3000, "down", "right", "right", "ok"),
                                "--eval", "%d:JSON.stringify(root.theme.logo)" % (READY + 4000), "--wait", str(READY + 4600), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.docs(out)[-1], {"on": 2, "off": 3})
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f)["logo"], {"on": 2, "off": 3})

    def test_number_keys_are_a_theme_switch_and_hold_home_only_shows(self):
        # Remote & pointer: pointer, speed, size, then "Hold Home" only shows, so the fourth stop is the number keys switch
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("remote", "right", "down", "down", "down", "ok"),
                            "--eval", "%d:JSON.stringify(root.theme.remote)" % (READY + 3200), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.docs(out)[-1], {"numberKeys": False})

    # ---- the remote's app keys (settings/AppKeys.js, PageAppKey): the answer is `remote.appKeys` in the user's theme.json
    def probe(self, data, js):
        """JS for `--eval` that runs `js` with `o.power`, `o.lu` and `o.apps` the app's own singletons (kit/Power, Lu, Apps): the host's
        expressions see `root` and the module's types, not the names a QML file imports, so the probe imports the staged kit directory itself
        (`data` is the host's --data directory, where the module is staged)."""
        base = "file://%s/qml/WebOSCompositor/lumatv" % data
        return ("(function(){var o=Qt.createQmlObject('import QtQuick 2.4; import \"%s/kit\"; QtObject { property var power: Power; property var lu: Lu; property var apps: Apps }', "
                "root, '%s/probe.qml');%s})()" % (base, base, js))

    def run_probe(self, js, *args, surface="Settings.qml", wait="5800", at=5000):
        """Runs `js` through `probe` on `surface`; returns (the parsed JSON documents the eval printed, the host's output)."""
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            rc, out = self.host("--eval", "%d:%s" % (at, self.probe(data, js)), *args, "--wait", wait, data=data, surface=surface)
        self.assertEqual(rc, 0, out[-1500:])
        return self.docs(out), out

    CHOOSER = ("root.openPage('appkey',{key:'%s'});var p=root.pageItem('appkey');"
               "return JSON.stringify({title:p.title,back:p.back,stop:p.stop,apps:o.apps.list.length,"
               "rows:p.rows.map(function(r){return {label:p.ev(r.label)||p.ev(r.text)||'',selected:p.ev(r.selected),detail:p.ev(r.detail)||''}})})")
    KEY_ROWS = ("var p=root.pageItem('remote');return JSON.stringify(p.rows.filter(function(r){return /key$/.test(p.ev(r.label)||'')&&r.kind==='value'}).map(function(r){"
                "return {label:p.ev(r.label),value:p.ev(r.value),detail:p.ev(r.detail)}}))")

    def test_the_app_key_chooser_offers_lgs_default_nothing_and_every_app_and_marks_what_is_in_force(self):
        docs, out = self.run_probe(self.CHOOSER % "amazon")
        prime = docs[-1]
        self.assertEqual((prime["title"], prime["back"]), ("Prime Video key", "Remote & pointer"))
        labels = [r["label"] for r in prime["rows"]]
        self.assertEqual(labels[:3], ["LG's default", "Does nothing", "Open an app"])
        self.assertEqual(len(labels) - 3, prime["apps"])                                   # every app the library lists, once
        self.assertIn("Netflix", labels[3:])
        self.assertEqual([r["selected"] for r in prime["rows"][:2]], [True, False])           # nothing was chosen: LG's default is in force
        self.assertEqual(prime["stop"], 0)                                                 # and the focus starts on it
        self.assertIn("not installed", prime["rows"][0]["detail"])                         # Prime Video is not on the fake TV: the key would do nothing
        docs, out = self.run_probe(self.CHOOSER % "netflix")
        self.assertEqual(docs[-1]["rows"][0]["detail"], "Opens Netflix.")

    def test_the_remote_page_says_what_each_app_key_does(self):
        # LG Channels is a hidden system app: the library does not list it, but it is installed, so LG's default holds
        rows = self.run_probe(self.KEY_ROWS)[0][-1]
        self.assertEqual([(r["label"], r["value"]) for r in rows],
                         [("Netflix key", "Netflix"), ("Prime Video key", "Nothing"), ("Disney+ key", "Nothing"), ("LG Channels key", "LG Channels")])
        self.assertEqual(rows[0]["detail"], "LG's choice.")
        self.assertIn("not installed", rows[1]["detail"])

    def test_a_configured_key_shows_its_app_or_nothing_and_an_app_that_is_gone_does_nothing(self):
        theme = {"remote": {"appKeys": {"netflix": "", "amazon": "youtube.leanback.v4", "disneyplus": "com.gone.app"}}}
        rows = self.run_probe("o.lu.theme = %s;" % json.dumps(theme) + self.KEY_ROWS)[0][-1]
        self.assertEqual([(r["label"], r["value"]) for r in rows],
                         [("Netflix key", "Nothing"), ("Prime Video key", "YouTube"), ("Disney+ key", "Nothing"), ("LG Channels key", "LG Channels")])
        self.assertEqual(rows[0]["detail"], "The key does nothing.")
        self.assertEqual(rows[1]["detail"], "Opens the app chosen here.")
        self.assertIn("no longer installed", rows[2]["detail"])

    def test_choosing_writes_one_entry_each_and_leaving_the_default_removes_it(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # Remote page: four stops (pointer, speed, size, number keys), then Netflix. Right enters, Down x4 lands on Netflix, OK opens the chooser
            # on LG's default; Down + OK is "Does nothing" and returns to the page; Down is Prime Video, the same; Up and OK reopens Netflix on
            # "Does nothing" (the one in force), Up + OK is LG's default again, which removes the entry. One key every 350 ms: each choice is
            # written to the file before the next, as a person's presses are.
            steps = ["right"] + ["down"] * 4 + ["ok", "down", "ok", "down", "ok", "down", "ok", "up", "ok", "up", "ok"]
            presses = [a for i, s in enumerate(steps) for a in self.keys(READY + 1500 + 350 * i, s)]
            end = READY + 1500 + 350 * len(steps)
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, *(["down"] * RAIL["remote"])), *presses,
                                "--eval", "%d:JSON.stringify({remote: root.theme.remote, open: root.settingsOpen})" % (end + 600), "--wait", str(end + 1200), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            doc = self.docs(out)[-1]
            self.assertEqual(doc["remote"], {"numberKeys": True, "appKeys": {"amazon": ""}})     # Netflix is back on LG's default
            self.assertTrue(doc["open"])
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f)["remote"]["appKeys"], {"amazon": ""})

    def test_an_app_chosen_for_a_key_is_that_apps_id(self):
        # the first app row: Down x2 from LG's default (past Does nothing and the heading), then OK
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("remote", "right", *["down"] * 4, "ok", "down", "down", "ok"),
                                "--eval", "%d:JSON.stringify(root.theme.remote.appKeys)" % (READY + 3200), "--wait", str(READY + 3800), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.docs(out)[-1], {"netflix": "com.webos.app.discovery"})        # the dock's first tile: the all-apps store

    # ---- Glass depth (Lucent token theme.depth, read by Glass)
    def glass(self, level=None):
        theme = {"perf": {"syncResidents": True}}
        if level:
            theme["depth"] = {"level": level}
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            js = ("var g=Qt.createComponent('file://%s/qml/WebOSCompositor/lumatv/Glass.qml').createObject(root);g.isLight=false;"
                  "return JSON.stringify({level:o.lu.depthLevel,depth:o.lu.depth,margin:g.margin,shadow:[g.shadow.x,g.shadow.y,g.shadow.z],edge:g.edge.w})" % data)
            rc, out = self.host("--theme", json.dumps(theme), "--eval", "%d:%s" % (READY + 500, self.probe(data, js)), "--wait", str(READY + 1200), data=data)
        self.assertEqual(rc, 0, out[-1500:])
        return self.docs(out)[-1]

    def test_glass_depth_scales_the_shadow_and_the_margin_that_holds_it(self):
        normal, flat, deep = self.glass(), self.glass("flat"), self.glass("deep")
        self.assertEqual((normal["level"], normal["depth"], normal["margin"]), ("normal", 1, 60))
        self.assertAlmostEqual(normal["shadow"][0], 0.28, places=5)                          # the material as designed
        self.assertEqual(normal["shadow"][1:], [24, 30])
        self.assertEqual((flat["depth"], flat["margin"]), (0, 60))
        self.assertEqual(flat["shadow"][0], 0)                                               # no shadow, and a blur the shader can divide by
        self.assertGreaterEqual(flat["shadow"][2], 1)
        self.assertLess(flat["edge"], normal["edge"])                                        # the edge is lighter, not gone
        self.assertGreater(flat["edge"], 0.5 * normal["edge"])
        self.assertEqual((deep["depth"], deep["margin"]), (1.5, 90))                         # the quad grows with the shadow it has to hold
        self.assertAlmostEqual(deep["shadow"][0], 0.42, places=5)
        self.assertEqual(deep["shadow"][1:], [36, 45])

    def test_the_glass_depth_row_writes_the_level_to_the_theme_file(self):
        # Home & look, Look tab: Dynamic tint, Appearance, Clock, Wallpaper motion, then Glass depth (Normal now): Right is Deep
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", *["down"] * 4, "right", "ok"),
                                "--eval", "%d:JSON.stringify(root.theme.depth)" % (READY + 3200), "--wait", str(READY + 3800), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.docs(out)[-1]["level"], "deep")
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f)["depth"], {"level": "deep"})

    # ---- sleep timer and panel hours (kit/Power: LG's tvReservationAgent and panelcontroller)
    def test_the_sleep_timer_is_set_in_minutes_and_shows_what_is_left(self):
        # System, Power tab (the tab bar, Right, back down): the sleep timer is the first row and a stepper (Off now): Right is 10 min
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            read = ("var c=root.lunaCalls.filter(function(c) { return c.method === '/timer/sleep/add' }).map(function(c) { return c.payload });"
                    "return JSON.stringify({calls: c, left: o.power.sleepLeft, index: o.power.sleepIndex})")
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("system", "right", "up", "right", "down", "right"),
                                "--eval", "%d:%s" % (READY + 3600, self.probe(data, read)), "--wait", str(READY + 4200), data=data)
        self.assertEqual(rc, 0, out[-1500:])
        doc = self.docs(out)[-1]
        self.assertEqual(doc["calls"], ['{"sleepTimeMinute":"10"}'])
        self.assertEqual((doc["left"], doc["index"]), ("Off in 10 min", 1))                  # read back from the service a moment later

    def test_the_sleep_timer_rounds_up_to_LGs_choices_the_way_LGs_own_row_does(self):
        def at(doc):
            return "o.power.parseSleep(%s);out.push([o.power.sleepIndex, o.power.sleepLeft]);" % json.dumps(json.dumps(doc))
        js = ("var out=[];" + at({"returnValue": True, "sleepTimeMinute": 25, "sleepTimeSecond": 10}) + at({"returnValue": True, "sleepTimeSecond": 40})
              + at({"returnValue": True, "sleepTimeMinute": 61, "sleepTimeSecond": 0}) + at({"returnValue": True, "sleepTimeSecond": 0})
              + at({"returnValue": True, "sleepTimeMinute": 300}) + "return JSON.stringify(out)")
        self.assertEqual(self.run_probe(js)[0][-1], [[3, "Off in 25 min"], [1, "Off in under a minute"], [5, "Off in 61 min"], [0, ""], [8, "Off in 300 min"]])

    def test_panel_hours_are_LGs_counter_in_ten_minute_ticks_and_only_where_LG_shows_them(self):
        row = ("var p=root.pageItem('system');var r=null;for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].label)==='Panel hours')r=p.rows[i];")
        js = (row + "var on={hours:o.power.panelHours,shown:o.power.panelShown,value:p.ev(r.value),visible:p.ev(r.visible)};"
              "o.power.parseConfig(JSON.stringify({returnValue:true,configs:{'tv.conti.supportUsedTime':false}}));"
              "return JSON.stringify([on,{shown:o.power.panelShown,visible:p.ev(r.visible)}])")
        on, gone = self.run_probe(js)[0][-1]
        self.assertEqual(on, {"hours": "1,705 h", "shown": True, "value": "1,705 h", "visible": True})     # the fake TV's 10230 ticks
        self.assertEqual(gone, {"shown": False, "visible": False})

    def test_the_sound_delay_is_the_key_of_the_way_out_and_takes_its_limits_from_the_service(self):
        read = ("(function(){var p=root.pageItem('sound');var r=null;for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].label)==='Match screen and sound')r=p.rows[i];"
                "return JSON.stringify({ds:p.delayDs,min:p.ev(r.min),max:p.ev(r.max),step:p.ev(r.step),enabled:p.ev(r.enabled)})})()")
        rc, out = self.host("--eval", "5000:" + read, "--eval", "5200:" + self.SET_OUTPUT, "--eval", "5700:" + read, "--wait", "6300", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        earc, speakers = self.docs(out)[-2:]
        self.assertEqual(earc, {"ds": "sound.avSyncSpdif", "min": 0, "max": 400, "step": 10, "enabled": True})
        self.assertEqual(speakers["ds"], "sound.avSyncSpeaker")

    def test_the_rows_that_wrote_nothing_the_tv_has_are_gone(self):
        # sound.avSync is a switch on the set ("on"), not a delay in ms; the Wallpaper row only said a slideshow is not built
        rc, out = self.host("--eval", "5000:" + self.WALK, "--eval", "5600:" + self.CALLS, "--eval",
                            "5800:JSON.stringify(['sound','look'].map(function(id){var p=root.pageItem(id);return p.rows.map(function(r){return p.ev(r.label)||''})}))",
                            "--wait", "6400", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        sound, look = self.docs(out)[-1]
        calls = self.docs(out)[-2]
        self.assertNotIn("Wallpaper", look)
        self.assertIn("Glass depth", look)
        self.assertIn("Match screen and sound", sound)
        self.assertNotIn("avSync", {k for c in calls for k in c["payload"]["settings"]})

    def test_the_aspect_ratio_is_written_for_the_input_in_front(self):
        # Picture, Mode tab: cards, OLED pixel brightness, Filmmaker ambient light, AI Picture Pro, Aspect ratio (Original now): Left is 16:9
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("picture", "right", "down", "down", "down", "down", "left", "ok"),
                            "--eval", "%d:%s" % (READY + 3600, LUNA), "--wait", str(READY + 4200))
        self.assertEqual(rc, 0, out[-1500:])
        calls = self.docs(out)[-1]
        self.assertEqual([c for c in calls if c["category"] == "aspectRatio"],
                         [{"category": "aspectRatio", "settings": {"arcPerApp": "16x9"}, "app_id": "com.webos.app.hdmi2"}])

    def ambient_posts(self, *keys):
        """Home opens Settings, the rail goes down to Ambient light and the D-pad does `keys`; what the helper was asked to change or do.
        Later than the other tests press: the page is built last and asks the helper for its values when it is."""
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 3000, *(["down"] * RAIL["ambient"])), *self.keys(READY + 4500, "right", *keys),
                            "--eval", "%d:%s" % (READY + 5800, POSTS), "--wait", str(READY + 6400))
        self.assertEqual(rc, 0, out[-1500:])
        return self.tagged(out, "posts")[-1]["posts"]

    def test_the_ambient_light_switch_is_a_helper_setting_the_helper_checks(self):
        posts = [p for p in self.ambient_posts("ok", "down", "right", "right") if p[0] == "/v1/settings/base.ambient"]
        self.assertEqual([p[1]["changes"][0] for p in posts],
                         [{"key": "enabled", "value": {"type": "bool", "value": True}},               # in the order made, one at a time,
                          {"key": "brightness", "value": {"type": "integer", "value": 80}}])         # and a held slider sends the value it ended on

    def test_the_test_pattern_button_starts_and_the_helpers_state_turns_it_into_stop(self):
        # toggle, brightness, mode, then the Run button
        self.assertIn(["/v1/ambient/test", {"run": True}], self.ambient_posts("down", "down", "down", "ok"))

    def test_the_layout_tab_writes_one_edge_at_a_time(self):
        # up to the tab bar, Right to Layout (the focus stays on the bar), Down to Top, Right adds a section
        posts = [p for p in self.ambient_posts("up", "right", "down", "right") if p[0] == "/v1/settings/base.ambient"]
        self.assertEqual([p[1]["changes"] for p in posts], [[{"key": "top", "value": {"type": "integer", "value": 19}}]])

    def test_the_tab_bar_keeps_the_focus_so_right_walks_through_the_tabs(self):
        # two Rights on the tab bar reach Colour (the third tab); Down then focuses its first row (Still colour: Warm now), Right picks Cool
        posts = [p for p in self.ambient_posts("up", "right", "right", "down", "right", "ok") if p[0] == "/v1/settings/base.ambient"]
        self.assertEqual([p[1]["changes"] for p in posts], [[{"key": "still_colour", "value": {"type": "choice", "value": "cool"}}]])

    def test_an_ambient_value_the_helper_refuses_goes_back_to_what_is_in_force(self):
        # 73 % is off the 5 % step the schema allows: the fake enforces the helper's own schema file, answers 400, and the page re-reads
        write = "(function(){var s=root.pageItem('ambient').settings;s.write('brightness', 73);return s.get('brightness')})()"
        read = "(function(){var s=root.pageItem('ambient').settings;return JSON.stringify({brightness: s.get('brightness'), problem: s.problem})})()"
        rc, out = self.host("--eval", "5000:" + write, "--eval", "6000:" + read, "--wait", "6600", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(float(self.values(out)[0]), 73)               # shown at once ...
        doc = self.docs(out)[-1]
        self.assertEqual(doc["brightness"], 70)                       # ... and back to the helper's value once it said no
        self.assertTrue(doc["problem"])

    # ---- every row against the service's own rules
    WALK = ("(function(){var n=0;for(var i=0;i<root.pages.length;i++){var p=root.pageItem(root.pages[i].id);if(!p||!p.rows)continue;"
            "for(var r=0;r<p.rows.length;r++){var row=p.rows[r];if(!row.act||row.info===true||p.ev(row.enabled)===false)continue;var k=row.kind;"
            "if(k==='toggle'){row.act(true);row.act(false);n+=2}"
            "else if(k==='seg'||k==='cards'){var c=p.count(r);for(var j=0;j<c;j++){row.act(j);n++}}"
            "else if(k==='slider'){row.act(p.ev(row.min));row.act(p.ev(row.max));n+=2}}}return n})()")
    SET_OUTPUT = ("(function(){var o=Qt.createQmlObject('import QtQuick 2.4; import WebOSServices 1.0; QtObject { function set(c, s) { LS.adhoc.call(\"luna://com.webos.service.settings\", \"/setSystemSettings\", JSON.stringify({category: c, settings: s})) } }', root);"
                  "o.set('sound', {soundOutput: 'tv_speaker'});root.pageItem('sound').refresh();return 1})()")
    CALLS = ("(function(){var o=Qt.createQmlObject('import QtQuick 2.4; import WebOSServices 1.0; QtObject { property var calls: LS.calls }', root);"
             "return JSON.stringify(o.calls.filter(function(c){return c.method==='/setSystemSettings'}).map(function(c){return {payload: JSON.parse(c.payload), error: c.error}}))})()")

    def test_no_row_writes_a_setting_or_value_the_service_refuses(self):
        # Every switch both ways, every choice, every slider at both ends, against the stub that refuses what LG's own service has
        # no key for (keys and value tables taken from LG's Settings app). Sound's modes only answer while the TV speakers play.
        rc, out = self.host("--eval", "5000:" + self.WALK, "--eval", "5200:" + self.SET_OUTPUT, "--eval", "6000:" + self.WALK, "--eval", "6400:" + self.CALLS,
                            "--wait", "7000", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        calls = self.docs(out)[-1]
        walked = [v for v in self.values(out) if str(v).isdigit()]
        self.assertGreater(int(walked[0]), 40, "the walk found too few rows")
        self.assertGreater(len(calls), 40)
        self.assertEqual([c for c in calls if c["error"]], [])
        written = {k for c in calls for k in c["payload"]["settings"]}
        self.assertTrue({"soundMode", "pictureMode", "ai_Picture", "noOperationAutoOff", "pointerSpeed", "simplinkEnable", "wolwowlOnOff"} <= written, written)

    def test_the_stub_refuses_what_the_real_service_would(self):
        # the check above means something only if the stub says no: a key LG has no such name for, and a value outside a key's table
        rc, out = self.host("--eval", "5000:(function(){var o=Qt.createQmlObject('import QtQuick 2.4; import WebOSServices 1.0; QtObject { function set(c, s) { LS.adhoc.call(\"u\", \"/setSystemSettings\", JSON.stringify({category: c, settings: s})) } property var calls: LS.calls }', root);"
                            "o.set('general', {pointerSpeed: 'fast'});o.set('sound', {soundMode: 'cinema'});o.set('picture', {backlight: '101'});o.set('picture', {backlight: '70'});"
                            "return JSON.stringify(o.calls.filter(function(c){return c.method==='/setSystemSettings'}).map(function(c){return c.error !== ''}))})()", "--wait", "5600", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.docs(out)[-1], [True, True, True, False])

    def test_every_row_that_leaves_for_LG_names_a_screen_LG_has(self):
        collect = ("(function(){var out=[];for(var i=0;i<root.pages.length;i++){var p=root.pageItem(root.pages[i].id);if(!p||!p.rows)continue;"
                   "for(var r=0;r<p.rows.length;r++){var row=p.rows[r];if(row.app!==undefined)out.push({page:root.pages[i].id,label:p.ev(row.label),target:row.target,app:row.app})}}"
                   "return JSON.stringify(out)})()")
        rc, out = self.host("--eval", "5000:" + collect, "--wait", "5600", surface="Settings.qml")
        self.assertEqual(rc, 0, out[-1500:])
        rows = self.docs(out)[-1]
        with open(os.path.join(HERE, "qmlhost", "stubs", "WebOSServices", "LgSettings.js")) as f:
            js = f.read()
        known = set(re.findall(r'"([A-Za-z0-9_]+)"', js[js.index("var targets"):js.index("function domain")]))
        self.assertGreater(len(rows), 12)
        for r in rows:
            if r["app"] == "com.palm.app.settings" and r["target"]:
                self.assertIn(r["target"], known, r)
        # the ones that matter, by page: each page's escape lands where its own text says
        by = {(r["page"], r["label"], r["target"]) for r in rows}
        self.assertIn(("picture", "More in LG settings", "Brightness"), by)
        self.assertIn(("system", "Software update", "SoftwareUpdate"), by)
        self.assertIn(("inputs", "Game Optimizer", ""), by)


AI_BASE = {"listener": {"enabled": False, "bound": False, "addr": None, "port": 18732, "since": None, "error": None}, "pairing": None,
           "assistants": [{"id": "cody", "name": "Cody", "createdAt": 1, "lastSeenAt": 0, "paused": False, "calls": 12,
                           "levels": {"state": "always", "playback": "always", "launch": "ask", "power": "ask"}}],
           "limits": {"maxVolume": 60},
           "groups": [{"id": "state", "title": "See what is playing", "default": "always"}, {"id": "playback", "title": "Play, pause, volume, mute", "default": "always"},
                      {"id": "launch", "title": "Open apps, switch inputs", "default": "ask"}, {"id": "power", "title": "Turn the TV off", "default": "ask"}],
           "tools": []}

POSTS = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[posts] "+x.responseText)};'
         'x.open("GET","http://127.0.0.1:18731/__posts");x.send();return 1})()')
REQUESTS = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[req] "+x.responseText)};'
            'x.open("GET","http://127.0.0.1:18731/__requests");x.send();return 1})()')
APPROVE = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[approval] "+x.status)};'
           'x.open("POST","http://127.0.0.1:18731/__approve");x.setRequestHeader("Content-Type","application/json");x.send("{}");return 1})()')


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class AiAccess(HostCase):
    """The AI access page and the prompt, against a fake helper that models the /v1/ai routes and
    answers a prompt only through its own test door (`/__approve`), as only the real remote does."""

    def ai(self, **over):
        doc = json.loads(json.dumps(AI_BASE))
        for k, v in over.items():
            doc[k] = v
        return doc

    def run_page(self, doc, steps, settle=1200):
        """Opens Settings on the AI page and runs `steps` (a list of (ms, kind, text): keys or an eval), returning the requests
        the helper saw as each `[req]` printed."""
        args = ["--publish", "800:ai:" + json.dumps(doc), "--eval", "%d:root.openSettings()" % OPEN_AT,
                *self.keys(READY + 600, *(["down"] * RAIL["ai"]), "right")]
        end = READY + 600
        for ms, kind, what in steps:
            ms += READY
            end = max(end, ms)
            args += self.keys(ms, *what) if kind == "keys" else ["--eval", "%d:%s" % (ms, what)]
        rc, out = self.host(*args, "--wait", str(end + settle))
        self.assertEqual(rc, 0, out[-1500:])
        reqs = [json.loads(m.group(1)) for m in re.finditer(r"\[req\] (\{.*\})", out)]
        return out, [[tuple(r[:2]) + (r[2],) for r in q["requests"] if not r[1].endswith("/audit")] for q in reqs]

    def test_turning_it_on_only_asks_and_the_open_prompt_swallows_every_key(self):
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["right", "ok"]), (2400, "eval", REQUESTS),
                                              (2800, "keys", ["ok", "ok", "back", "down", "ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": True})])
        self.assertEqual(seen[1], seen[0], "a key got past the prompt")

    def test_turning_it_off_is_done_at_once_and_pressing_off_again_sends_nothing_new(self):
        on = self.ai(listener={"enabled": True, "bound": True, "addr": "192.168.1.242:18732", "port": 18732, "since": 1, "error": None})
        out, seen = self.run_page(on, [(1600, "keys", ["left", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": False})])

    def test_removing_an_assistant_needs_a_second_press_and_lockout_too(self):
        on = self.ai(listener={"enabled": True, "bound": True, "addr": "x", "port": 1, "since": 1, "error": None})
        # listener, down to the volume limit, down to Cody, Right to Remove, OK arms it, OK again removes it
        out, seen = self.run_page(on, [(1600, "keys", ["down", "down", "right", "ok"]), (2400, "eval", REQUESTS), (2600, "keys", ["ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("DELETE", "/v1/ai/assistants/cody", {})])
        # ... and from Cody down to the pairing row and the lock-out, which asks twice as well
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["down", "down", "down", "down"]), (2000, "keys", ["ok"]), (2400, "eval", REQUESTS), (2600, "keys", ["ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("POST", "/v1/ai/lockout", {})])

    def test_a_lower_level_is_sent_whole_a_higher_one_only_asks(self):
        # listener, limit, Cody (Right twice: Remove, then across to the first group), down to "Open apps" (ask now)
        into = ["down", "down", "right", "right", "down", "down"]
        out, seen = self.run_page(self.ai(), [(1600, "keys", into + ["right", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(len(seen[0]), 1)
        method, path, body = seen[0][0]
        self.assertEqual((method, path), ("POST", "/v1/ai/assistants/cody/levels"))
        self.assertEqual(body, {"levels": {"state": "always", "playback": "always", "launch": "never", "power": "ask"}})
        out, seen = self.run_page(self.ai(), [(1600, "keys", into + ["left", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(seen[0][0][2]["levels"]["launch"], "always")      # asked; the fake helper answers 202 and opens a prompt

    def test_a_lower_volume_limit_is_done_a_higher_one_asks(self):
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["down", "left"]), (2400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/limits", {"maxVolume": 58})])

    def test_approving_at_the_remote_turns_it_on_and_nothing_else_does(self):
        state = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[ai] "+x.responseText)};'
                 'x.open("GET","http://127.0.0.1:18731/v1/ai");x.send();return 1})()')
        out, _ = self.run_page(self.ai(), [(1600, "keys", ["right", "ok"]), (2300, "eval", state), (2800, "eval", APPROVE), (3500, "eval", state)], settle=1500)
        enabled = [json.loads(m.group(1))["ai"]["listener"]["enabled"] for m in re.finditer(r"\[ai\] (\{.*\})", out)]
        self.assertEqual(enabled, [False, True])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Prompt(HostCase):
    PROMPT = {"kind": "tv_standby", "title": "Cody wants to turn off the TV", "detail": "The TV turns off now. The receiver stays as it is.",
              "assistant": {"id": "cody", "name": "Cody"}, "reason": "Nate asked me to shut everything down for the night.", "expiresAt": 4102444800000}

    DRAWN = ("(function(){var p=null,out=[];function find(o){if(o.who!==undefined&&o.secondsLeft!==undefined){p=o;return}var c=o.children;for(var i=0;i<c.length&&!p;i++)find(c[i])}"
             "function walk(o){if(o.text!==undefined&&o.visible)out.push(String(o.text));var c=o.children;for(var i=0;i<c.length;i++)walk(c[i])}"
             "find(root);if(!p)return 'null';walk(p);return JSON.stringify({visible:p.visible,up:p.up,texts:out})})()")

    def test_the_prompt_draws_who_what_and_the_assistants_own_words_apart(self):
        rc, out = self.host("--publish", "2000:ai.prompt:" + json.dumps(self.PROMPT), "--eval", "3000:" + self.DRAWN, "--wait", "3600")
        self.assertEqual(rc, 0, out[-1500:])
        doc = self.docs(out)[-1]
        self.assertTrue(doc["visible"] and doc["up"])
        texts = doc["texts"]
        self.assertIn("Cody is asking \u00b7 through AI access", texts)               # who asks
        self.assertIn("Cody wants to turn off the TV", texts)                        # the helper's own title
        self.assertIn("The TV turns off now. The receiver stays as it is.", texts)   # what will happen, the helper's words
        self.assertIn("Cody\u2019s reason, in its own words", texts)                  # the assistant's words are labelled as its own
        self.assertIn("\u201cNate asked me to shut everything down for the night.\u201d", texts)
        self.assertTrue(any(x.startswith("Closes in ") for x in texts))
        # the label of the assistant's words comes before them, and the helper's detail before both
        self.assertLess(texts.index("The TV turns off now. The receiver stays as it is."), texts.index("Cody\u2019s reason, in its own words"))

    def test_nothing_is_drawn_while_no_prompt_is_open(self):
        rc, out = self.host("--eval", "3000:" + self.DRAWN, "--wait", "3600")
        self.assertEqual(rc, 0, out[-1500:])
        doc = self.docs(out)[-1]
        self.assertFalse(doc["visible"] or doc["up"])


class Icons(unittest.TestCase):
    """Every icon a QML file names is one the generator made."""

    def test_every_named_icon_exists(self):
        with open(os.path.join(ROOT, "payload", "compositor", "lumatv", "icons", "lucide", "index.json")) as f:
            have = set(json.load(f))
        missing = []
        base = os.path.join(ROOT, "payload", "compositor", "lumatv")
        for dirpath, _, files in os.walk(base):
            if os.sep + "widgets" in dirpath or os.sep + "icons" in dirpath:       # Home's weather and Home Assistant cards name their own pictures
                continue
            for n in files:
                if not n.endswith((".qml", ".js")):
                    continue
                with open(os.path.join(dirpath, n), encoding="utf-8") as f:
                    text = f.read()
                for m in re.finditer(r'(?:\bicon: *|LuIcon *\{[^}\n]*?\bname: *)"([a-z0-9-]+)"', text):
                    if m.group(1) not in have and n != "Lu.qml":
                        missing.append((n, m.group(1)))
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
