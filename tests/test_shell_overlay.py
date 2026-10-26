"""Run with: python3 -m unittest discover -s tests -p 'test_shell_overlay.py'   (needs Xvfb and PyQt5; skipped without them)

What Home draws over an app (payload/compositor/lumatv/LumaOverlay.qml), through the offline host with the overlay beside Home as the
fullscreen container loads it (`--overlay`): the input pill and the other banners, their buttons and the keys that move them, the
notification drawer a held gear opens, and that nothing of it takes a key or draws while no app is in front. The key filter's side is
`tests/keyfilter.test.js` and `tests/bus.test.js`. Nothing here measures time: the set's gate does.
"""
import json
import os
import re
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SHORT_ID = json.load(open(os.path.join(ROOT, "brand.json")))["shortId"]
DOOR = "/tmp/%s-overlay" % SHORT_ID
PLAIN = '{"perf":{"syncResidents":true}}'
OWN = '{"perf":{"syncResidents":true},"banners":{"own":true}}'
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { u: c.uri, m: c.method, p: p } }))"
STATUS = "[overlay.up, overlay.wants, overlay.surface].join('/')"
PILL = {"kind": "input", "icon": "gamepad-2", "name": "HDMI 3 \u00b7 PlayStation 5", "facts": "4K", "holdMs": 60000}
BAR = dict(PILL, buttons=True, actions=["inputs", "picture"])


# The make-believe TV answers `Inputs` when its singleton first lives, which is when Home's build first reaches a page that uses it; a test that
# needs the answer to be in before it starts asks at once.
PRIME_INPUTS = "Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\nItem { Component.onCompleted: Inputs.refresh() }', overlay, 'prime')"


def js(v):
    return json.dumps(v)


def do(code):
    return "(function(){ %s; return 1 })()" % code


def key(code, pressed="true"):
    return "overlay.key(%s, %s, false, 0)" % (code, pressed)


def sysui_calls(got):
    """The systemUIManager launches the overlay made: the app ids."""
    return [c["p"].get("appId") for c in json.loads(got) if c["u"].endswith("systemUIManager") and c["m"] == "/launch"]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Overlay(HostCase):
    def run_overlay(self, steps, wait, theme=PLAIN, *args):
        """steps: (ms, js, label), Home hidden and the overlay built at once; `theme` is the owner's theme.json (the banners of their own are off
        unless OWN: with them on, the host's make-believe TV, whose foreground arrives whenever Home's build reaches its Inputs, can put a pill
        over a banner a step is about to key). Returns ({label: value}, the host's whole output)."""
        argv = ["--eval", "100:" + do("overlay.warm = true")]
        for ms, expr, label in steps:
            argv += ["--eval", "%d:(function(){ return %s + (%s) })()" % (ms, json.dumps(label + "="), expr)]
        rc, out = self.host(*argv, *args, "--overlay", "--prop", "hostActive=false", "--theme", theme, "--wait", str(wait))
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got, out

    def test_the_overlay_is_idle_until_an_app_is_in_front_and_leaves_with_it(self):
        got, _ = self.run_overlay([
            (1000, STATUS, "idle"), (1050, "overlay.gearReady()", "ready_home"), (1100, "overlay.showBanner(%s)" % js(PILL), "shown_home"),
            (1150, key("Qt.Key_Right"), "key_home"),
            (1500, do("overlay.appFront = true"), "front"), (1600, "overlay.flatGlass", "flat"), (1650, "overlay.gearReady()", "ready_app"),
            (1700, "overlay.showBanner(%s)" % js(BAR), "bar"), (1800, STATUS, "bar_state"),
            (1900, do("overlay.appFront = false"), "left"), (2000, STATUS, "after_left"), (2050, "overlay.bannerUp", "banner_gone"),
            (2100, key("Qt.Key_Right"), "key_left"),
            # the glass is Home's again the moment Home is shown
            (2200, do("root.hostActive = true"), "home"), (2500, "overlay.flatGlass", "flat_home"),
        ], 3200)
        self.assertEqual(got["idle"], "false/false/")
        self.assertEqual((got["ready_home"], got["shown_home"], got["key_home"]), ("false", "false", "false"), "nothing over Home: Home draws its own")
        self.assertEqual((got["flat"], got["ready_app"]), ("true", "true"), "over an app the glass samples a flat colour, and the gear may open the drawer")
        self.assertEqual(got["bar_state"], "true/true/banner")
        self.assertEqual(got["after_left"], "false/false/", "the app left the front: everything closes")
        self.assertEqual(got["banner_gone"], "false")
        self.assertEqual(got["key_left"], "false")
        self.assertEqual(got["flat_home"], "false", "Home takes its own backdrop back")

    def test_a_banner_with_buttons_takes_the_dpad_and_its_buttons_open_lgs_windows_over_the_app(self):
        got, _ = self.run_overlay([
            (1000, do("overlay.appFront = true"), "front"),
            # a pill that only shows takes no key and hides itself on request
            (1100, "overlay.showBanner(%s)" % js(PILL), "pill"), (1200, STATUS, "pill_state"), (1250, key("Qt.Key_Right"), "pill_right"), (1300, do("overlay.hideBanner()"), "hide"),
            (1400, "overlay.bannerUp", "pill_up"),
            # with buttons: Right moves to Picture, OK opens LG's Quick Settings and the banner is gone; the release of the same key is the filter's pair
            (1500, "overlay.showBanner(%s)" % js(BAR), "bar"), (1600, STATUS, "bar_state"), (1650, key("Qt.Key_Right"), "right"), (1660, key("Qt.Key_Right", "false"), "right_up"),
            (1700, key("Qt.Key_Return"), "ok"), (1710, key("Qt.Key_Return", "false"), "ok_up"), (2000, STATUS, "after_ok"), (2000, CALLS, "calls"),
            # Back leaves a banner; Inputs opens LG's input picker
            (2100, "overlay.showBanner(%s)" % js(BAR), "again"), (2200, key("Qt.Key_Back"), "back"), (2400, STATUS, "after_back"),
            (2500, "overlay.showBanner(%s)" % js(BAR), "third"), (2600, key("Qt.Key_Return"), "inputs"), (2900, CALLS, "calls2"),
            # a key that is not the D-pad is not the overlay's, and no signal is a banner with buttons too
            (3000, "overlay.showBanner(%s)" % js({"kind": "nosignal", "icon": "unplug", "port": "HDMI 4", "title": "Nothing is connected here", "body": "Plug a device into HDMI 4."}), "nosignal"),
            (3100, STATUS, "nosignal_state"), (3150, key("Qt.Key_VolumeUp"), "volume"),
        ], 3600)
        self.assertEqual(got["pill_state"], "true/false/", "a pill without buttons shows and takes no key")
        self.assertEqual(got["pill_right"], "false")
        self.assertEqual(got["pill_up"], "false")
        self.assertEqual(got["bar_state"], "true/true/banner")
        # the release of OK comes after the banner has gone: the filter pairs it to the overlay and does not read the answer
        self.assertEqual((got["right"], got["right_up"], got["ok"]), ("true",) * 3)
        self.assertEqual(got["after_ok"], "false/false/")
        self.assertEqual(sysui_calls(got["calls"]), ["com.webos.app.quicksettings"], "Picture opens Quick Settings, a system window that LG draws over any app")
        self.assertEqual((got["back"], got["after_back"]), ("true", "false/false/"))
        self.assertEqual(sysui_calls(got["calls2"]), ["com.webos.app.quicksettings", "com.webos.app.quickinputpicker"])
        self.assertEqual(got["nosignal_state"], "true/true/banner")
        self.assertEqual(got["volume"], "false", "the volume keys are the app's")

    def test_with_the_key_filters_client_the_buttons_open_the_system_windows_through_it_and_not_through_ls_adhoc(self):
        # the filter registers a launcher on the bus (LumaKeyBridge.qml does, with LG's key filters' own luna client); a probe stands in for it
        probe = ("Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'LumaBus.js\" as Bus\\n"
                 "Item { objectName: \"launcherProbe\"; property var seen: []; Component.onCompleted: Bus.setSystemLauncher(function(a, p) { seen.push(a); return true }) }', overlay, 'probe')")
        seen = "(function(){ for (var i = 0; i < overlay.children.length; i++) if (overlay.children[i].objectName === 'launcherProbe') return JSON.stringify(overlay.children[i].seen); return 'no probe' })()"
        got, out = self.run_overlay([
            (900, "(function(){ " + probe + "; return 1 })()", "probe"), (1000, do("overlay.appFront = true"), "front"),
            (1100, "overlay.showBanner(%s)" % js(BAR), "bar"), (1200, key("Qt.Key_Right"), "right"), (1210, key("Qt.Key_Return"), "picture"),
            (1500, "overlay.showBanner(%s)" % js(BAR), "again"), (1600, key("Qt.Key_Return"), "inputs"), (1900, seen, "seen"), (1900, CALLS, "calls"),
        ], 2300)
        self.assertEqual((got["picture"], got["inputs"]), ("true", "true"))
        self.assertEqual(json.loads(got["seen"]), ["com.webos.app.quicksettings", "com.webos.app.quickinputpicker"])
        self.assertEqual(sysui_calls(got["calls"]), [], "a system window asked for twice (Quick Settings toggles) or through a client that may not be allowed")
        self.assertIn("[LumaOverlay] com.webos.app.quicksettings asked for through the key filter's client", out)

    def test_the_gear_hold_opens_the_drawer_over_an_app_and_a_second_hold_or_back_closes_it(self):
        got, _ = self.run_overlay([
            (1000, "overlay.gearHold()", "no_app"), (1100, do("overlay.appFront = true"), "front"),
            (1200, "overlay.gearHold()", "held"), (1300, STATUS, "open"), (1350, key("Qt.Key_Down"), "down"), (1360, key("Qt.Key_Down", "false"), "down_up"),
            (1400, "overlay.showBanner(%s)" % js(PILL), "banner_over_drawer"), (1450, key("Qt.Key_Back"), "back"), (1700, STATUS, "after_back"),
            (1800, "overlay.gearHold()", "held_again"), (1900, STATUS, "open_again"), (2000, "overlay.gearHold()", "toggled"), (2300, STATUS, "after_toggle"),
            # the drawer asked for while a banner with buttons is up replaces it
            (2400, "overlay.showBanner(%s)" % js(BAR), "bar"), (2500, "overlay.gearHold()", "over_bar"), (2600, STATUS, "over_bar_state"),
        ], 3000)
        self.assertEqual(got["no_app"], "false", "no app in front: the gear is not the overlay's")
        self.assertEqual(got["held"], "true")
        self.assertEqual(got["open"], "true/true/drawer")
        self.assertEqual((got["down"], got["down_up"]), ("true", "true"))
        self.assertEqual(got["banner_over_drawer"], "false", "a banner does not cover the drawer the person asked for")
        self.assertEqual((got["back"], got["after_back"]), ("true", "false/false/"))
        self.assertEqual((got["held_again"], got["open_again"]), ("true", "true/true/drawer"))
        self.assertEqual((got["toggled"], got["after_toggle"]), ("true", "false/false/"))
        self.assertEqual(got["over_bar_state"], "true/true/drawer")

    def test_the_drawer_with_nothing_in_it_is_as_tall_as_its_words(self):
        # round 7 (docs/research/round7-2026-10-25/slot7/door-drawer.png): the card stayed 240 high and the caption under "Nothing new" ran out of it
        by_name = "function byName(p, n) { if (p.objectName === n) return p; var c = p.children; for (var i = 0; i < c.length; i++) { var f = byName(c[i], n); if (f) return f } return null } "
        geometry = ("(function(){ " + by_name + "var s = byName(overlay, 'drawerSheet'), e = byName(overlay, 'drawerEmpty'); "
                    "return JSON.stringify({ sheet: s.height, count: s.bodyHeight, emptyY: e.y, emptyBottom: e.y + e.height, visible: e.visible }) })()")
        got, _ = self.run_overlay([(1000, do("overlay.appFront = true"), "front"), (1100, "overlay.gearHold()", "held"), (1900, geometry, "geometry")], 2300)
        g = json.loads(got["geometry"])
        self.assertTrue(g["visible"], "the drawer has something in it: the host's helper has no audit and the log is empty")
        self.assertGreater(g["emptyBottom"], 240 - 16, "the words fit in the old 240: this test no longer covers the overflow")
        self.assertGreaterEqual(g["sheet"], g["emptyBottom"] + 16 - 0.5, "the words run out of the card: %r" % g)
        self.assertLessEqual(g["sheet"], 1080 - 2 * 56)

    def test_the_owner_hands_the_gear_hold_back_to_LG(self):
        got, _ = self.run_overlay([(1000, do("overlay.appFront = true"), "front"), (1100, "overlay.gearReady()", "ready"), (1150, "overlay.gearHold()", "held"),
                                   (1300, STATUS, "state")], 1700, '{"perf":{"syncResidents":true},"remote":{"gearHold":false}}')
        self.assertEqual((got["ready"], got["held"]), ("false", "false"))
        self.assertEqual(got["state"], "false/false/")

    def test_an_input_coming_to_the_front_shows_its_pill_when_the_owner_asked_for_banners(self):
        got, _ = self.run_overlay([
            # the fake TV has HDMI 2 in front from the start, so a pill for it is waiting from load: it is cleared, so what follows is what each step asks for
            (200, do(PRIME_INPUTS), "prime"), (900, do("overlay.pending = null"), "clear_pending"), (1000, do("overlay.appFront = true"), "front"), (1060, do("overlay.hideBanner()"), "clear"),
            (1100, do("overlay.inputSwitched('netflix')"), "app"), (1200, "overlay.bannerUp", "app_banner"),
            (1300, do("overlay.inputSwitched('com.webos.app.hdmi2')"), "hdmi2"), (1400, "overlay.bannerUp", "hdmi2_banner"),
            (1450, "JSON.stringify(overlay.inputSpec('com.webos.app.hdmi2'))", "spec"), (1500, STATUS, "pill_state"), (1550, do("overlay.hideBanner()"), "hide"),
            # SAM names the app a little before its window maps: the pill waits for the app to be on screen
            (1600, do("overlay.appFront = false"), "away"), (1700, do("overlay.inputSwitched('com.webos.app.hdmi3')"), "hdmi3"), (1800, "overlay.bannerUp", "hdmi3_early"),
            (1900, do("overlay.appFront = true"), "mapped"), (2000, "overlay.bannerUp", "hdmi3_banner"), (2050, do("overlay.hideBanner()"), "hide3"),
            (2100, do("overlay.inputSwitched('com.webos.app.livetv')"), "tv"), (2200, "JSON.stringify(overlay.inputSpec('com.webos.app.livetv'))", "tv_spec"),
        ], 2600, OWN)
        self.assertEqual(got["app_banner"], "false", "an app that is not an input has no pill")
        self.assertEqual(got["hdmi2_banner"], "true")
        spec = json.loads(got["spec"])
        self.assertEqual((spec["kind"], spec["name"].split(" \u00b7 ")[0]), ("input", "HDMI 2"), "the pill names the port, then the device that named itself")
        self.assertEqual(got["pill_state"], "true/false/", "the pill takes no key")
        self.assertEqual((got["hdmi3_early"], got["hdmi3_banner"]), ("false", "true"))
        self.assertEqual(json.loads(got["tv_spec"])["name"], "Live TV")

    def test_banners_are_the_owners_choice_and_off_unless_asked_for(self):
        got, _ = self.run_overlay([(1000, do("overlay.appFront = true"), "front"), (1100, do("overlay.inputSwitched('com.webos.app.hdmi2')"), "hdmi2"),
                                   (1200, "overlay.bannerUp", "banner"), (1300, "overlay.showBanner(%s)" % js(PILL), "api"), (1400, "overlay.bannerUp", "api_banner")],
                                  1800, PLAIN)
        self.assertEqual(got["banner"], "false", "LG's own banners are the default; banners.own is the owner's switch")
        self.assertEqual((got["api"], got["api_banner"]), ("true", "true"), "a surface that asks for a banner is not the owner's switch")

    def test_the_door_shows_a_banner_written_to_it(self):
        with open(DOOR, "w") as f:
            json.dump(dict(PILL, name="HDMI 4 \u00b7 door"), f)
        try:
            got, _ = self.run_overlay([(1000, do("overlay.appFront = true"), "front"), (2600, "overlay.bannerUp", "banner")], 3000)
        finally:
            os.remove(DOOR)
        self.assertEqual(got["banner"], "true")


if __name__ == "__main__":
    unittest.main()
