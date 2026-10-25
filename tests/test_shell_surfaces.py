"""Run with: python3 -m unittest discover -s tests -p 'test_shell_surfaces.py'   (needs Xvfb and PyQt5; skipped without them)

The surfaces the shell keeps built (Layers.qml), through the offline host: the library behind the Apps tile, search and its
keyboard, the voice bar and the banners, and the real openers of the small panels (an app's menu, the uninstall question, the
on-screen remote). Each one opens the way Home opens it, has the keys while it is up, Back closes it, and Home's focus is
where it was. Nothing here measures time: the set's gate does.
"""
import json
import os
import re
import tempfile
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

READY = 7000                                   # Home has presented and built the chain
FIND = ("function find(p, k, b) { if (p[k] !== undefined && (!b || p.built !== undefined)) return p; var c = p.children; "
        "for (var i = 0; i < c.length; i++) { var f = find(c[i], k, b); if (f) return f } return null } ")
# where Home is: the layer with the keys, Home's own focus layer, the dock's row and column, the surfaces under the open one
STATE = "[root.layerOpen, root.focusLayer, root.dockRow, root.dockCol, root.layerStack.join('+')].join('/')"
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { m: c.method, p: p } }))"
SCREEN_REMOTE = 16777315                        # the host's stand-in for WebOS.Key_webOS_ScreenRemote (stubs/WebOS/Global)
KEYS = dict(HostCase.CODES, menu="Qt.Key_Menu")


def probe(find_key, expr, panel=True):
    """A JSON string of `expr` over the surface that has the property `find_key` (a panel: it says it is built); 'absent' when it is not built."""
    return "(function(){ " + FIND + "var s = find(root, '" + find_key + "', " + ("true" if panel else "false") + "); return s ? JSON.stringify(" + expr + ") : 'absent' })()"


def calls(method):
    """The payloads of the luna calls of one method the surface made, from the recorded list."""
    return lambda got: [c["p"] for c in json.loads(got) if c["m"] == method]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Shell(HostCase):
    CODES = KEYS

    def run_host(self, steps, wait, data=None, warns=None):
        """steps: (ms, 'keys', 'ok down') or (ms, 'eval', js, label), ms after READY. Returns {label: the eval's value}.
        `warns` names the one warning the run is meant to make (a refused layer), the only one allowed."""
        args = []
        for st in steps:
            ms, kind, what = st[:3]
            if kind == "keys":
                args += self.keys(READY + ms, *what.split())
            else:
                args += ["--eval", "%d:(function(){ return %s + (%s) })()" % (READY + ms, json.dumps(st[3] + "="), what)]
        rc, out = self.host(*args, "--wait", str(READY + wait), data=data)
        if warns:
            stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and warns not in ln and "Implicitly defined onFoo" not in ln]
            self.assertEqual(stray, [], out[-2000:])
            self.assertIn(warns, out)
        else:
            stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
            self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got

    def test_the_apps_tile_opens_the_library_back_closes_it_and_focus_is_where_it_was(self):
        titles = probe("cellIndex", "{ n: s.apps.length, titles: s.apps.map(function(a) { return a.title }), visible: s.visible, section: s.section }")
        got = self.run_host([(-300, "eval", STATE, "before"), (0, "keys", "ok"), (900, "eval", STATE, "opened"), (900, "eval", CALLS, "calls"),
                             (900, "eval", titles, "lib"), (1000, "keys", "back"), (1900, "eval", STATE, "after")], 2400)
        self.assertEqual(got["before"], "/dock/0/0/")
        self.assertEqual(got["opened"], "apps/dock/0/0/", "OK on the Apps tile did not open the library")
        self.assertEqual([p for p in calls("/launch")(got["calls"]) if p.get("id") == "com.webos.app.discovery"], [], "LG's store was launched instead")
        lib = json.loads(got["lib"])
        self.assertTrue(lib["visible"] and lib["section"] == "all")
        # what the TV launches, by the library's rules: the apps LG shows and Live TV; not the HDMI bookmark, not LG's hidden system apps
        self.assertEqual(lib["n"], 23)
        for want in ("LG Content Store", "Live TV", "Netflix"):
            self.assertIn(want, lib["titles"])
        for gone in ("AVR-S760H", "Camera", "Media Player", "LG Gallery+"):
            self.assertNotIn(gone, lib["titles"])
        self.assertEqual(got["after"], got["before"], "Back did not give the focus back")

    def test_the_menu_key_goes_to_the_library_from_the_widgets_and_to_the_tiles_options_on_the_dock(self):
        got = self.run_host([(-300, "eval", "(function(){ root.focusLayer = 'widgets'; return 'set' })()", "set"), (0, "keys", "menu"), (900, "eval", STATE, "widgets_menu"),
                             (1000, "keys", "back"), (1900, "eval", STATE, "widgets_back"),
                             (2000, "eval", "(function(){ root.focusLayer = 'dock'; return 'set' })()", "set2"), (2100, "keys", "right menu"), (3000, "eval", STATE, "dock_menu"),
                             (3100, "keys", "back"), (4000, "eval", STATE, "dock_back")], 4500)
        self.assertEqual(got["widgets_menu"], "apps/widgets/0/0/")
        self.assertEqual(got["widgets_back"], "/widgets/0/0/")
        self.assertEqual(got["dock_menu"], "menu/dock/0/1/", "the Menu key on a dock tile did not open that tile's options")
        self.assertEqual(got["dock_back"], "/dock/0/1/", "Back from the menu moved the dock's focus")

    def test_a_dock_tiles_menu_moves_it_and_the_move_is_written_back_to_the_dock_list(self):
        dock = probe("appsTileId", "{ moving: s.moving, col: s.col }", panel=False)
        got = self.run_host([(0, "keys", "right menu"), (900, "keys", "down ok"), (1700, "eval", STATE, "picked"), (1700, "eval", dock, "dock"),
                             (1800, "keys", "right ok"), (2800, "eval", CALLS, "calls")], 3300)
        self.assertEqual(got["picked"], "/dock/0/1/", "choosing Move closes the menu and leaves the tile picked up")
        self.assertEqual(json.loads(got["dock"]), {"moving": True, "col": 1})
        self.assertEqual(calls("/moveLaunchPoint")(got["calls"]), [{"launchPointId": "netflix_default", "position": 2}])

    def test_hide_and_rename_are_the_owners_and_are_written_to_his_theme_file(self):
        titles = probe("cellIndex", "{ shown: s.shown.map(function(a) { return a.title }), hidden: s.apps.filter(function(a) { return a.hidden }).map(function(a) { return a.title }) }")
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            got = self.run_host([(0, "keys", "ok"), (700, "keys", "down menu"),                                           # Crunchyroll's options
                                 (1800, "keys", "down down ok"), (2800, "eval", STATE, "hid"), (2800, "eval", titles, "hid_lib"),       # Hide
                                 (2900, "keys", "menu"), (3900, "keys", "down down down ok"), (4900, "eval", STATE, "renaming"),   # Rename what took its place
                                 (5000, "keys", "up ok down down down down right ok"), (6000, "eval", STATE, "renamed"), (6000, "eval", titles, "ren_lib")], 6600, data=data)
            self.assertEqual(got["hid"], "apps/dock/0/0/", "hiding closes the menu and leaves the library open")
            hid = json.loads(got["hid_lib"])
            self.assertNotIn("Crunchyroll", hid["shown"])
            self.assertEqual(hid["hidden"], ["Crunchyroll"])
            self.assertEqual(got["renaming"], "search/dock/0/0/apps", "Rename opens the keyboard over the library")
            self.assertEqual(got["renamed"], "apps/dock/0/0/", "Done closes the keyboard and the library is back")
            with open(os.path.join(data, "theme", "theme.json")) as f:
                saved = json.load(f)
            self.assertEqual(saved["apps"]["hidden"], ["com.crunchyroll.webapp"])
            self.assertEqual(saved["apps"]["names"], {"io.unom.punktfunk.client-webos": "Punktfunk5"})
            self.assertIn("Punktfunk5", json.loads(got["ren_lib"])["shown"])

    def test_uninstall_asks_first_keep_it_is_the_safe_answer_and_a_protected_app_has_no_uninstall(self):
        items = probe("bodyH", "s.items.map(function(i) { return i.id })")
        got = self.run_host([(0, "keys", "ok"), (700, "keys", "menu"), (1700, "eval", items, "store_items"), (1800, "keys", "back"),          # the store: no Uninstall
                             (2400, "keys", "down menu"), (3400, "keys", "down down down down ok"), (4400, "eval", STATE, "asked"),        # Crunchyroll: the question
                             (4500, "keys", "ok"), (5400, "eval", STATE, "kept"), (5400, "eval", CALLS, "kept_calls"),                       # Keep it
                             (5500, "keys", "menu"), (6500, "keys", "down down down down ok"), (7400, "keys", "right ok"),                  # again, now Uninstall
                             (8400, "eval", STATE, "done"), (8400, "eval", CALLS, "done_calls")], 8900)
        self.assertEqual(json.loads(got["store_items"]), ["open", "hide", "rename"], "LG's store offers no Uninstall and no Move")
        self.assertEqual(got["asked"], "confirm/dock/0/0/apps")
        self.assertEqual(got["kept"], "apps/dock/0/0/")
        self.assertEqual(calls("/remove")(got["kept_calls"]), [], "Keep it uninstalled something")
        self.assertEqual(got["done"], "apps/dock/0/0/")
        self.assertEqual(calls("/remove")(got["done_calls"]), [{"id": "com.crunchyroll.webapp", "subscribe": False}])

    def test_search_opens_from_the_status_bar_types_picks_and_back_returns_to_the_bar(self):
        bar = "(function(){ root.focusLayer = 'statusbar'; root.statusFocusIndex = root.statusItems.length - 1; return root.statusItems[root.statusFocusIndex].type })()"
        got = self.run_host([(-300, "eval", bar, "item"), (0, "keys", "ok"), (900, "eval", STATE, "opened"), (1000, "keys", "back"), (1900, "eval", STATE, "closed"),
                             (2000, "keys", "ok"), (2900, "keys", "down down left left ok up up up up ok"), (4000, "eval", STATE, "picked"), (4000, "eval", CALLS, "calls")], 4500)
        self.assertEqual(got["item"], "search")
        self.assertEqual(got["opened"], "search/statusbar/0/0/", "OK on the bar's search did not open search")
        self.assertEqual(got["closed"], "/statusbar/0/0/", "Back from search must return to the bar")
        # x starts no app or page: the one answer is the web search, and picking it opens the browser with the text
        self.assertEqual(got["picked"], "/statusbar/0/0/")
        launch = calls("/launch")(got["calls"])[-1]
        self.assertEqual(launch["id"], "com.webos.app.browser")
        self.assertEqual(launch["params"]["target"], "https://duckduckgo.com/?q=x")

    def test_the_bell_opens_the_drawer_back_closes_it_and_the_bar_keeps_the_focus(self):
        bell = ("(function(){ var i = root.statusItems.map(function(s) { return s.type }).indexOf('notifications'); "
                "root.focusLayer = 'statusbar'; root.statusFocusIndex = i; return i })()")
        drawer = probe("sheetWidth", "{ visible: s.visible, width: s.sheetWidth }")
        got = self.run_host([(-300, "eval", bell, "bell"), (0, "keys", "ok"), (900, "eval", STATE, "opened"), (900, "eval", drawer, "drawer"),
                             (1000, "keys", "down"), (1500, "keys", "back"), (2400, "eval", STATE, "closed"), (2400, "eval", drawer, "after")], 2900)
        self.assertGreaterEqual(int(got["bell"]), 0, "the bar has no bell")
        self.assertEqual(got["opened"], "drawer/statusbar/0/0/", "OK on the bell did not open the drawer")
        self.assertEqual(json.loads(got["drawer"]), {"visible": True, "width": 640})
        self.assertEqual(got["closed"], "/statusbar/0/0/", "Back from the drawer must leave the focus on the bell")
        self.assertFalse(json.loads(got["after"])["visible"])

    def test_the_screensaver_takes_every_key_and_wakes_the_tv_and_gives_the_keys_back_when_it_is_gone(self):
        def saver(on):
            return "(function(){ " + FIND + "var s = find(root, 'moveEveryMs', true); s.forced = %s; return s.active })()" % on
        up = "(function(){ " + FIND + "return find(root, 'saverUp', false).saverUp })()"
        got = self.run_host([(-300, "eval", saver("true"), "forced"), (0, "keys", "right ok"), (900, "eval", STATE, "taken"), (900, "eval", up, "up"), (900, "eval", CALLS, "calls"),
                             (1000, "eval", saver("false"), "gone"), (1100, "eval", up, "down"), (1200, "keys", "right"), (2000, "eval", STATE, "back")], 2500)
        self.assertEqual(got["forced"], "true")
        self.assertEqual(got["up"], "true")
        self.assertEqual(got["taken"], "/dock/0/0/", "a key reached Home while the screensaver was up")
        self.assertEqual(len(calls("/power/turnOnScreen")(got["calls"])), 2, "each key press asks the TV to turn the screen on")
        self.assertEqual(got["down"], "false")
        self.assertEqual(got["back"], "/dock/0/1/", "the keys did not come back to Home")

    def test_the_voice_bar_listens_and_back_leaves_it_and_what_was_said_has_no_way_to_open_anything(self):
        voice = probe("micOpen", "{ state: s.state, mic: s.micOpen, pick: typeof s.picked, options: typeof s.options }")
        got = self.run_host([(0, "eval", "(function(){ root.voiceStart(); return root.layerOpen })()", "listening"), (500, "eval", voice, "bar"),
                             (800, "keys", "back"), (1400, "eval", STATE, "left"), (1400, "eval", voice, "after"),
                             (1500, "eval", "typeof root.voiceHeard + typeof root.voicePicked", "gone"), (1500, "eval", CALLS, "calls")], 2000)
        self.assertEqual(got["listening"], "voice")
        self.assertEqual(json.loads(got["bar"]), {"state": "listening", "mic": True, "pick": "undefined", "options": "undefined"}, "the bar has nothing to pick")
        self.assertEqual(got["left"], "/dock/0/0/", "Back leaves the voice bar")
        self.assertEqual(json.loads(got["after"])["mic"], False)
        self.assertEqual(got["gone"], "undefinedundefined", "Home has no call that acts on what was said")
        self.assertEqual(calls("/launch")(got["calls"]), [])

    def test_a_banner_with_buttons_takes_the_keys_and_one_without_leaves_them_to_home(self):
        nosignal = "{kind:'nosignal', icon:'unplug', port:'HDMI 4', title:'Nothing is connected here', body:'x'}"
        pill = "{kind:'input', icon:'gamepad-2', name:'HDMI 3', facts:'4K', holdMs: 60000}"
        bar = "{kind:'input', icon:'gamepad-2', name:'HDMI 3', facts:'4K', buttons: true, holdMs: 60000}"
        got = self.run_host([(0, "eval", "(function(){ root.showBanner(%s); return root.layerOpen })()" % nosignal, "nosignal"), (400, "keys", "right ok"),
                             (1400, "eval", STATE, "home"), (1400, "eval", CALLS, "home_calls"),
                             (1500, "eval", "(function(){ root.showBanner(%s); return root.layerOpen })()" % pill, "pill"), (1600, "keys", "right"), (2300, "eval", STATE, "pill_right"),
                             (2400, "eval", "(function(){ root.hideBanner(); root.showBanner(%s); return root.layerOpen })()" % bar, "bar"),
                             (2500, "keys", "right right ok"), (3500, "eval", "root.settingsOpen", "settings")], 4000)
        self.assertEqual(got["nosignal"], "banner", "no signal has buttons: it takes the keys")
        self.assertEqual(got["home"], "/dock/0/0/")
        self.assertEqual(calls("/launchDefaultApp")(got["home_calls"]), [{"category": "home"}])
        self.assertEqual(got["pill"], "", "a pill is not modal")
        self.assertEqual(got["pill_right"], "/dock/0/1/", "with a pill up the D-pad is still Home's")
        self.assertEqual(got["bar"], "banner")
        self.assertEqual(got["settings"], "true", "Picture on the bar opens Settings")

    def test_the_on_screen_remote_key_opens_it_and_a_key_on_it_is_the_tvs_key(self):
        press = "(function(){ root.key(%d, true, false, 0); root.key(%d, false, false, 0); return root.layerOpen })()" % (SCREEN_REMOTE, SCREEN_REMOTE)
        got = self.run_host([(0, "eval", press, "opened"), (900, "keys", "ok"), (1500, "eval", CALLS, "calls"), (1600, "keys", "back"), (2400, "eval", STATE, "closed")], 2900)
        self.assertEqual(got["opened"], "remote")
        self.assertEqual(calls("/sendKeycode")(got["calls"]), [{"keycode": 143}], "OK on the first key (rewind) presses it through the micom service")
        self.assertEqual(got["closed"], "/dock/0/0/")

    def test_a_layer_that_is_no_surface_owns_no_key_and_cannot_be_opened(self):
        got = self.run_host([(0, "eval", "(function(){ root.openLayer('nonsense'); return root.layerOpen })()", "refused"),
                             (100, "eval", "(function(){ root.layerOpen = 'bogus'; return root.layerOpen })()", "forced"), (200, "keys", "right"), (1000, "eval", STATE, "after")], 1500,
                            warns="no surface is called nonsense")
        self.assertEqual(got["refused"], "")
        self.assertEqual(got["forced"], "bogus")
        self.assertEqual(got["after"], "/dock/0/1/", "a name that is no surface swallowed the key")


if __name__ == "__main__":
    unittest.main()
