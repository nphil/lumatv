"""Run with: python3 -m unittest discover -s tests -p 'test_shell_host.py'   (needs Xvfb and PyQt5; skipped without them)

The shell's key path as LumaHome answers it through the offline host: the number keys that open the dock's tiles and the setting that
turns them off, the gear key's hold that opens the notification drawer, and the heartbeat Home writes for the Home stub's guard (once at
load and then every second, whatever the panel does). The key filter's own side of the same paths is `tests/keyfilter.test.js`. Nothing
here measures time: the set's gate does.

A host run costs about twenty seconds, so each test is one run that walks through a sequence of what happens on a remote.
"""
import json
import re
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

READY = 7000                                   # Home has presented and built the chain
# where Home is: the layer with the keys, Home's own focus layer, the dock's row and column, the surfaces under the open one
STATE = "[root.layerOpen, root.focusLayer, root.dockRow, root.dockCol, root.layerStack.join('+')].join('/')"
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { m: c.method, p: p } }))"
OFF = '{"perf":{"syncResidents":true},"remote":{"numberKeys":false,"gearHold":false}}'


def launches(got):
    """The ids of the apps the TV was asked to launch, in order."""
    return [c["p"].get("id") for c in json.loads(got) if c["m"] == "/launch"]


def key(code, pressed="true", repeat="false"):
    """`LumaHome.key(code, pressed, autoRepeat, device)`: what the key filter hands Home."""
    return "root.key(%s, %s, %s, 0)" % (code, pressed, repeat)


def do(js):
    """A statement (or several) as the expression the host evaluates."""
    return "(function(){ %s; return 1 })()" % js


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class KeyPath(HostCase):
    def run_host(self, steps, wait, *args):
        """steps: (ms after READY, js, label). Returns ({label: value}, the host's whole output)."""
        argv = []
        for ms, expr, label in steps:
            argv += ["--eval", "%d:(function(){ return %s + (%s) })()" % (READY + ms, json.dumps(label + "="), expr)]
        rc, out = self.host(*argv, *args, "--wait", str(READY + wait))
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got, out

    def test_number_keys_open_the_docks_tiles_and_only_where_a_tile_is_the_answer(self):
        got, _ = self.run_host([
            # 2 is the second tile (Netflix): the press acts, its repeats and release go with it, a release of a press Home never took does not
            (0, key("Qt.Key_2"), "press"), (20, key("Qt.Key_2", "true", "true"), "repeat"), (40, key("Qt.Key_2", "false"), "release"),
            (60, key("Qt.Key_2", "false"), "stray_release"), (500, CALLS, "calls_two"), (500, STATE, "state_two"),
            # 9 is the ninth and 0 the tenth
            (600, key("Qt.Key_9"), "nine"), (620, key("Qt.Key_9", "false"), "nine_up"), (700, key("Qt.Key_0"), "zero"), (720, key("Qt.Key_0", "false"), "zero_up"),
            (1000, CALLS, "calls_all"),
            # 1 is the Apps tile: LumaTV's library, not LG's store
            (1100, key("Qt.Key_1"), "one"), (1120, key("Qt.Key_1", "false"), "one_up"), (1700, STATE, "state_one"),
            # with the library open a digit is not Home's; nor while a tile is being moved; nor when Home is not in front
            (1800, key("Qt.Key_3"), "in_library"), (1900, do("root.closeAllLayers()"), "closed"),
            (2000, do("root.moveOnDock('netflix')"), "moving"), (2100, key("Qt.Key_3"), "while_moving"),
        ], 2600)
        self.assertEqual((got["press"], got["repeat"], got["release"]), ("true", "true", "true"), "the press, its repeats and its release are Home's")
        self.assertEqual(got["stray_release"], "false", "a release of a press Home never took is not Home's")
        self.assertEqual(launches(got["calls_two"]), ["netflix"], "2 is the second tile of the dock, once")
        self.assertEqual(got["state_two"], "/dock/0/1/", "the tile keeps Home's focus, as if OK had been pressed on it")
        self.assertEqual((got["nine"], got["nine_up"], got["zero"], got["zero_up"]), ("true",) * 4)
        self.assertEqual(launches(got["calls_all"]), ["netflix", "com.webos.app.browser", "com.lighting.huesync"], "9 is the ninth tile, 0 the tenth")
        self.assertEqual((got["one"], got["one_up"]), ("true", "true"))
        self.assertEqual(got["state_one"], "apps/dock/0/0/")
        self.assertNotIn("com.webos.app.discovery", launches(got["calls_all"]))
        self.assertEqual((got["in_library"], got["while_moving"]), ("false", "false"))

    def test_a_digit_whose_tile_left_home_still_ends_with_its_release(self):
        got, _ = self.run_host([(0, key("Qt.Key_5"), "press"), (100, do("root.hostActive = false"), "away"), (150, key("Qt.Key_5", "false"), "release"),
                                (200, key("Qt.Key_5", "false"), "second"), (250, key("Qt.Key_5"), "not_home"), (300, "root.gearReady()", "gear_ready"),
                                (350, "root.gearHold()", "gear_held"), (700, CALLS, "calls")], 900)
        self.assertEqual((got["press"], got["release"], got["second"]), ("true", "true", "false"))
        self.assertEqual(got["not_home"], "false", "Home is not in front: LG's number key handling keeps the key")
        self.assertEqual((got["gear_ready"], got["gear_held"]), ("false", "false"), "nor does Home answer the gear's hold then")
        self.assertEqual(launches(got["calls"]), ["com.geforcenow.play"])

    def test_the_owner_turns_the_number_keys_and_the_gear_hold_off_and_the_keys_are_LGs_again(self):
        patch = "root.patchTheme({ remote: { numberKeys: true } })"
        got, _ = self.run_host([(0, key("Qt.Key_2"), "press"), (20, key("Qt.Key_2", "false"), "release"), (50, "root.gearReady()", "gear_ready"),
                                (60, "root.gearHold()", "gear_held"), (500, STATE, "state"), (600, CALLS, "calls_off"),
                                # the switch is read live: back on, the same key opens the tile
                                (700, do(patch), "on"), (800, key("Qt.Key_3"), "press_on"), (820, key("Qt.Key_3", "false"), "release_on"), (1300, CALLS, "calls_on")],
                               1700, "--theme", OFF)
        self.assertEqual((got["press"], got["release"]), ("false", "false"), "with remote.numberKeys off no digit is taken")
        self.assertEqual((got["gear_ready"], got["gear_held"]), ("false", "false"), "with remote.gearHold off the gear's hold is LG's")
        self.assertEqual(got["state"], "/dock/0/0/")
        self.assertEqual(launches(got["calls_off"]), [])
        self.assertEqual((got["press_on"], got["release_on"]), ("true", "true"))
        self.assertEqual(launches(got["calls_on"]), ["com.beb.plxnative"])

    def test_the_gear_hold_opens_the_notification_drawer_from_wherever_home_is_and_a_second_hold_closes_it(self):
        got, _ = self.run_host([
            (0, "root.gearReady()", "ready"), (50, "root.gearHold()", "held"), (900, STATE, "open"), (1000, "root.gearHold()", "again"), (1400, STATE, "closed"),
            # from Settings: Settings is closed and the drawer is what has the keys
            (1500, do("root.openSettings()"), "settings"), (2200, "root.settingsOpen", "settings_open"), (2300, "root.gearHold()", "held_settings"),
            (3000, "[root.settingsOpen, root.layerOpen].join('/')", "state_settings"),
            # from the library: the library is gone and the drawer is what has the keys (Home's focus is where closing Settings left it: the bar)
            (3100, do("root.closeAllLayers(); root.openApps()"), "apps"), (3200, "root.gearHold()", "held_apps"), (3900, STATE, "state_apps"),
        ], 4300)
        self.assertEqual((got["ready"], got["held"], got["again"]), ("true", "true", "true"))
        self.assertEqual(got["open"], "drawer/dock/0/0/")
        self.assertEqual(got["closed"], "/dock/0/0/")
        self.assertEqual(got["settings_open"], "true")
        self.assertEqual(got["state_settings"], "false/drawer")
        self.assertEqual(got["state_apps"], "drawer/statusbar/0/0/")

    # ---- the remote's app keys: what LumaHome says to the key filter (settings/AppKeys.js has the shape of remote.appKeys)

    def test_the_owners_app_keys_open_the_app_he_chose_or_nothing_and_an_unset_key_is_LGs_where_its_app_is_installed(self):
        chosen = {"perf": {"syncResidents": True}, "remote": {"appKeys": {"netflix": "com.beb.plxnative", "amazon": "", "disneyplus": "com.not.installed"}}}
        unset = "root.patchTheme({ remote: { appKeys: {} } })"
        got, _ = self.run_host([
            (0, "root.appKey('netflix', '')", "netflix"), (100, "root.appKey('netflix', 'com.beb.plxnative')", "already"),
            (200, "root.appKey('amazon', '')", "nothing"), (300, "root.appKey('disneyplus', '')", "missing"),
            (400, "root.appKey('lgchannels', '')", "unset_installed"), (500, CALLS, "calls"),
            # every key unset: LG's own target where that app is installed (Netflix is), nothing where it is not (Disney+ is not)
            (600, do(unset), "reset"), (700, "root.appKey('netflix', '')", "default_netflix"), (800, "root.appKey('disneyplus', '')", "default_disney"),
            (900, CALLS, "calls_default"),
            # not from Home: no splash, the same launch
            (1000, do("root.hostActive = false"), "away"), (1100, "root.patchTheme({ remote: { appKeys: { amazon: 'com.apple.appletv' } } })", "set"),
            (1200, "root.appKey('amazon', 'netflix')", "over_app"), (1500, CALLS, "calls_over"),
        ], 2000, "--theme", json.dumps(chosen))
        self.assertEqual(got["netflix"], "true", "the key is taken: it opens Plex, not Netflix")
        self.assertEqual((got["already"], got["nothing"], got["missing"]), ("true", "true", "true"),
                         "over the app it opens, set to nothing, or set to an app that is gone: taken, nothing opens")
        self.assertEqual(got["unset_installed"], "false", "LG Channels is installed and never chosen: LG's key")
        self.assertEqual(launches(got["calls"]), ["com.beb.plxnative"])
        self.assertEqual((got["default_netflix"], got["default_disney"]), ("false", "true"))
        self.assertEqual(launches(got["calls_default"]), ["com.beb.plxnative"], "an unset key never launches what LG would auto-install")
        self.assertEqual(launches(got["calls_over"]), ["com.beb.plxnative", "com.apple.appletv"])

    def test_home_writes_a_beat_at_load_and_goes_on_beating_with_the_panel_off(self):
        # hostActive is what the container says when Home is not on screen (standby, an app in front); the beat never reads it
        got, out = self.run_host([(-6900, "root.beatSeq", "at_load"), (-2500, "root.beatSeq", "later"), (-1000, "root.beatSeq", "a"), (1500, "root.beatSeq", "b"),
                                  (1600, do("root.lastBeatMs = Date.now() - 9000"), "quiet")], 3600, "--prop", "hostActive=false")
        self.assertEqual(got["at_load"], "1", "the first beat is written at load, before the timer's first tick")
        self.assertIn("[LumaHome] loaded, first beat written", out)
        self.assertGreaterEqual(int(got["later"]), 4, "the timer beats once a second with Home hidden")
        self.assertGreaterEqual(int(got["b"]) - int(got["a"]), 2)
        self.assertRegex(out, r"\[LumaHome\] the beat was silent for (8|9|10) s", "a beat that went quiet says so when it starts again")


if __name__ == "__main__":
    unittest.main()
