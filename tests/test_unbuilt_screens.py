"""Run with: python3 -m unittest discover -s tests -p 'test_unbuilt_screens.py'   (needs Xvfb and PyQt5; skipped without them)

The screens that were not built and now are, behind their switches (off until the TV check): Quick Access for the number keys (the owner's own
choice for a digit, the question for a digit with nothing behind it) and the recent-apps switcher that Back on Home opens. Each is one run of
the offline host that walks through what happens on a remote; nothing here measures time (the set's gate does) and what they look like is
`tests/qmlhost/scenes_unbuilt.py`.

The patches that bring LG's startup cover, launch splash and launch spinner into LumaTV's look, and the `standbyscreen` switch, are in
tests/test_cover_splash_patch.py.
"""
import json
import unittest

import re

from test_settings import HostCase
from test_quick_panels import probe, probes
from test_shell_host import CALLS, READY, STATE, do, key, launches
from xdisplay import DISPLAY, HAVE_QT

QA = "root.patchTheme({ remote: { quickAccess: { on: true, slots: %s } } })"
SLOTS = "JSON.stringify(root.theme.remote.quickAccess)"


def calls(got, method):
    return [c["p"] for c in json.loads(got) if c["m"] == method]


class Walk(HostCase):
    """A run of the offline host that walks through a remote's keys (test_shell_host.KeyPath.run_host, without that class's own tests)."""

    def run_host(self, steps, wait, *args):
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


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class QuickAccess(Walk):
    def test_off_leaves_the_number_keys_as_they_were_and_on_puts_the_owners_choice_before_the_dock(self):
        got, _ = self.run_host([
            # off (the default): 2 is the dock's second tile, as before; nothing else is said or asked
            (0, key("Qt.Key_2"), "press"), (20, key("Qt.Key_2", "false"), "release"), (400, CALLS, "calls_off"), (450, "root.quickAccessOn", "on_off"),
            # on, with a choice for 2: that app opens and the dock's tile does not
            (600, do(QA % '{ "2": "com.crunchyroll.webapp" }'), "patched"), (700, "root.quickAccessOn", "on_on"),
            (800, key("Qt.Key_2"), "press_own"), (820, key("Qt.Key_2", "false"), "release_own"), (1400, CALLS, "calls_own"), (1450, STATE, "state_own"),
        ], 2200)
        self.assertEqual((got["press"], got["release"]), ("true", "true"))
        self.assertEqual(launches(got["calls_off"]), ["netflix"], "off: the dock's second tile")
        self.assertEqual((got["on_off"], got["on_on"]), ("false", "true"))
        self.assertEqual((got["press_own"], got["release_own"]), ("true", "true"))
        self.assertEqual(launches(got["calls_own"]), ["netflix", "com.crunchyroll.webapp"], "the owner's own choice, not the dock's tile, and Netflix only once")

    def test_a_number_with_nothing_behind_it_asks_and_the_answer_is_kept(self):
        got, _ = self.run_host([
            (0, do(QA % "{}"), "patched"),
            # the question for 4: Not now (down to it, OK) leaves everything as it was
            (100, do("root.accessAsk('4')"), "asked"), (500, STATE, "state_asked"),
            (600, key("Qt.Key_3"), "digit_in_question"),                        # a digit is not an answer, and is not LG's either
            (700, "root.key(Qt.Key_Down, true, false, 0)", "down"), (720, "root.key(Qt.Key_Down, false, false, 0)", "down_up"),
            (800, "root.key(Qt.Key_Return, true, false, 0)", "ok"), (820, "root.key(Qt.Key_Return, false, false, 0)", "ok_up"),
            (1200, STATE, "state_not_now"), (1250, SLOTS, "slots_not_now"), (1300, CALLS, "calls_not_now"),
            # the question again: the first pick is the first app that no number opens yet; OK keeps it for 4 and opens it
            (1500, do("root.accessAsk('4')"), "asked_again"),
            (1700, "root.key(Qt.Key_Return, true, false, 0)", "pick"), (1720, "root.key(Qt.Key_Return, false, false, 0)", "pick_up"),
            (2400, STATE, "state_picked"), (2450, SLOTS, "slots_picked"), (2500, CALLS, "calls_picked"),
        ], 3200)
        self.assertEqual(got["state_asked"], "access/dock/0/0/", "the question is the surface with the keys")
        self.assertEqual(got["digit_in_question"], "true", "Home takes the digit and does nothing with it")
        self.assertEqual(got["state_not_now"].split("/")[0], "", "Not now closes it")
        self.assertEqual(json.loads(got["slots_not_now"]), {"on": True, "slots": {}})
        self.assertEqual(launches(got["calls_not_now"]), [])
        self.assertEqual(got["state_picked"].split("/")[0], "")
        slots = json.loads(got["slots_picked"])["slots"]
        self.assertEqual(list(slots), ["4"])
        self.assertEqual(launches(got["calls_picked"]), [slots["4"]], "what was picked opens, and is kept for 4")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Recents(Walk):
    ON = "root.patchTheme({ remote: { recents: true } })"

    def test_back_on_home_opens_the_switcher_only_when_the_owner_turned_it_on_and_ok_switches_to_the_frame(self):
        got, _ = self.run_host([
            # off (the default): Back from the dock goes up to the cards, Back again is LG's (the key is not taken)
            (0, key("Qt.Key_Back"), "back1"), (20, key("Qt.Key_Back", "false"), "back1_up"), (200, key("Qt.Key_Back"), "back2"), (220, key("Qt.Key_Back", "false"), "back2_up"),
            (500, STATE, "state_off"), (550, CALLS, "calls_off"),
            # on: the same second Back opens the switcher, with the keys
            (700, do(self.ON), "patched"), (800, key("Qt.Key_Back"), "back3"), (820, key("Qt.Key_Back", "false"), "back3_up"), (1300, STATE, "state_on"), (1350, CALLS, "calls_on"),
            # Right moves to the second frame and OK switches to it (PlxNative), which closes the switcher
            (1400, "root.key(Qt.Key_Right, true, false, 0)", "right"), (1420, "root.key(Qt.Key_Right, false, false, 0)", "right_up"),
            (1500, key("Qt.Key_Return"), "ok"), (1520, key("Qt.Key_Return", "false"), "ok_up"), (2200, STATE, "state_after_ok"), (2250, CALLS, "calls"),
        ], 3000)
        self.assertEqual((got["back1"], got["back2"]), ("true", "false"), "off: the second Back leaves Home as it always did")
        self.assertEqual(got["state_off"].split("/")[0], "")
        self.assertEqual(calls(got["calls_off"], "/listRecents"), [], "off: LG's list is never asked for, not at start and not as Home comes to the front")
        self.assertTrue(calls(got["calls_on"], "/listRecents"), "on: the list is read as the switch turns on")
        self.assertEqual(got["state_on"].split("/")[0], "recents")
        self.assertEqual(got["state_after_ok"].split("/")[0], "")
        self.assertEqual(launches(got["calls"]), ["com.beb.plxnative"], "the second frame, once")

    def test_a_held_ok_closes_the_app_of_the_frame_then_its_entry_and_back_returns_to_home(self):
        got, _ = self.run_host([
            (0, do(self.ON), "patched"),
            (100, key("Qt.Key_Back"), "back1"), (120, key("Qt.Key_Back", "false"), "back1_up"), (300, key("Qt.Key_Back"), "back2"), (320, key("Qt.Key_Back", "false"), "back2_up"),
            (800, STATE, "state_open"),
            (900, key("Qt.Key_Return"), "down"), (1900, key("Qt.Key_Return", "false"), "up"), (2300, CALLS, "calls_held"),
            (2400, key("Qt.Key_Back"), "back3"), (2420, key("Qt.Key_Back", "false"), "back3_up"), (2800, STATE, "state_back"),
        ], 3400)
        self.assertEqual(got["state_open"].split("/")[0], "recents")
        closed = calls(got["calls_held"], "/closeByAppId")
        removed = calls(got["calls_held"], "/removeRecent")
        self.assertEqual(closed, [{"id": "netflix"}], "the app is closed first")
        self.assertEqual(removed, [{"appId": "netflix"}], "then its card, since removing the card alone leaves the app running")
        self.assertEqual(launches(got["calls_held"]), [], "a held OK does not switch to the app")
        self.assertEqual(got["state_back"].split("/")[0], "", "Back closes the switcher")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class QuickPanelEdit(HostCase):
    """The quick panel's popups and edit mode (docs/mocks/29-qp-*), asked through the panel's own keys over Home."""

    def test_ok_on_a_stepper_opens_its_choices_beside_the_panel_and_choosing_one_sets_it(self):
        go = "var s = find(root, 'popupOpen'); var n = 0; while (s.focusId !== 'sleep' && n++ < 12) s.key('down'); "
        rc, out = self.host(
            *probe(READY + 300, "open", "root.patchTheme({ remote: { quickPanelEdit: true } }); var b = bus(); return b.open('quick');"),
            *probe(READY + 1000, "ok", go + "s.key('ok'); return [s.focusId, s.popupOpen];"),
            *probe(READY + 1300, "moved", "var s = find(root, 'popupOpen'); s.key('down'); s.key('down'); s.key('ok'); return [s.popupOpen, s.presented];"),
            *probe(READY + 1800, "again", "var s = find(root, 'popupOpen'); s.key('ok'); s.key('back'); return [s.popupOpen, s.presented];"),
            "--eval", "%d:%s" % (READY + 2200, "JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/timer/sleep/add' }).map(function(c) { return JSON.parse(c.payload) }))"),
            "--wait", str(READY + 2600))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        self.assertEqual(p["ok"], ["sleep", True], "OK on the sleep timer opens its choices")
        self.assertEqual(p["moved"], [False, True], "two Downs and OK choose the second choice after Off and close the card; the panel stays")
        self.assertEqual(p["again"], [False, True], "Back closes only the card")
        self.assertIn('[{"sleepTimeMinute":"20"}]', "".join(l for l in out.splitlines() if "sleepTimeMinute" in l).replace(" ", ""), out[-1500:])

    def test_with_the_switch_off_ok_on_a_stepper_does_nothing_and_a_held_ok_is_an_ordinary_one(self):
        go = "var s = find(root, 'popupOpen'); var n = 0; while (s.focusId !== 'sleep' && n++ < 12) s.key('down'); "
        rc, out = self.host(
            *probe(READY + 300, "open", "var b = bus(); return b.open('quick');"),
            *probe(READY + 1000, "ok", go + "s.key('ok'); s.hold(); return [s.popupOpen, s.editing, s.popupsOn];"),
            "--wait", str(READY + 1500))
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(probes(out)["ok"], [False, False, False], "off (the default until the TV check): the panel is as it was")

    def test_a_held_ok_edits_the_tiles_and_what_the_owner_arranged_is_the_themes_and_the_panels(self):
        ids = "s.arranged().map(function(r) { return r.id })"
        go = "var s = find(root, 'popupOpen'); var n = 0; while (s.focusId !== 'sleep' && n++ < 12) s.key('down'); "
        rc, out = self.host(
            *probe(READY + 300, "open", "root.patchTheme({ remote: { quickPanelEdit: true } }); var b = bus(); return b.open('quick');"),
            *probe(READY + 1000, "start", go + "return [" + ids + ", s.editing];"),
            # held OK on Sleep timer: picked up in edit mode; Up moves it above the receiver; OK drops it
            *probe(READY + 1400, "held", "var s = find(root, 'popupOpen'); s.hold(); var a = s.editing; s.key('up'); s.key('ok'); return [a, " + ids + ", s.editing, root.theme.quickPanel.tiles];"),
            # Down to the Wi-Fi tile, Right to its cross, OK takes it off
            *probe(READY + 1800, "removed", "var s = find(root, 'popupOpen'); s.key('down'); s.key('down'); s.key('right'); s.key('ok'); return " + ids + ";"),
            # Add a tile: the list offers what is not shown; OK adds the first at the bottom
            *probe(READY + 2200, "add", "var s = find(root, 'popupOpen'); var n = 0; while (n++ < 12) s.key('down'); s.key('ok'); var open = s.popupOpen; s.key('ok'); return [open, s.popupOpen, " + ids + "];"),
            *probe(READY + 2600, "done", "var s = find(root, 'popupOpen'); s.key('back'); return [s.editing, s.presented];"),
            "--wait", str(READY + 3000))
        self.assertEqual(rc, 0, out[-2500:])
        p = probes(out)
        base = ["picture", "receiver", "sleep", "wifi", "ai"]
        self.assertEqual(p["start"], [base, False])
        held = p["held"]
        self.assertEqual(held[0], True, "a held OK on a tile edits")
        self.assertEqual(held[1], ["picture", "sleep", "receiver", "wifi", "ai"], "Up moved the picked tile one place")
        self.assertEqual(held[2], True, "dropping a tile leaves edit mode on")
        self.assertEqual(held[3], held[1], "and what is on the panel is what the theme says")
        self.assertEqual(p["removed"], ["picture", "sleep", "receiver", "ai"], "the cross took the Wi-Fi tile off")
        self.assertEqual(p["add"], [True, False, ["picture", "sleep", "receiver", "ai", "wifi"]],
                         "Add a tile opens the list of what is not shown (the Wi-Fi tile that was taken off comes first) and OK puts the first at the bottom")
        self.assertEqual(p["done"], [False, True], "Back leaves edit mode and the panel stays")


if __name__ == "__main__":
    unittest.main()
