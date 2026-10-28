"""Run with: python3 -m unittest discover -s tests -p 'test_switcher.py'   (needs Xvfb and PyQt5; skipped without them)

The app switcher's surface (payload/compositor/lumatv/panels/Switcher.qml, held by LumaOverlay.qml) through the offline host with the overlay
beside Home (`--overlay`): the list the compositor feeds it, the places it sends the compositor for the cards' pictures, the D-pad (Left and
Right between the cards, Down to a card's Quit, Up to Quit all, OK, Back), what each choice asks of the compositor, Quit all's question, and the
same surface over LG's compositor (no feed: LG's recents list and the app manager's own calls). The compositor's half is
`compositor/crates/lumatv-compositor/tests/switcher.rs`; what it looks like is `tests/qmlhost/scenes_switcher.py`. Nothing here measures time.
"""
import json
import re
import unittest

import os
import subprocess
import sys

from test_settings import HOST, HostCase
from xdisplay import DISPLAY, HAVE_QT

PLAIN = '{"perf":{"syncResidents":true},"remote":{"appSwitcher":true}}'
RECENTS_ONLY = '{"perf":{"syncResidents":true},"remote":{"recents":true}}'
APPS = [{"id": "plex", "title": "Plex", "picture": "live", "state": ""}, {"id": "youtube.leanback.v4", "title": "YouTube", "picture": "live", "state": ""},
        {"id": "com.webos.app.hdmi2", "title": "HDMI 2", "picture": "video", "state": ""}, {"id": "netflix", "title": "Netflix", "picture": "live", "state": ""}]
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { u: c.uri, m: c.method, p: p } }))"
# the overlay's requests to the compositor: a listener on its `switcherPost` logs each as `[rec] <json>`, which `posts()` reads back from the host's output
RECORDER = "(function(){ overlay.switcherPost.connect(function(op) { console.info('[rec] ' + JSON.stringify(op)) }); return 1 })()"
STATE = "[overlay.switcherUp, overlay.wants, overlay.switcherItem ? overlay.switcherItem.level + '/' + overlay.switcherItem.index + '/' + overlay.switcherItem.first + '/' + overlay.switcherItem.asking : 'none'].join(' ')"


LATE = 600                     # every step waits this much longer than it says: the overlay's build is not ready at once on a loaded machine


def feed(apps, is_open, n=1, frm=""):
    return "overlay.switcherFeed = %s" % json.dumps({"apps": apps, "open": is_open, "from": frm, "n": n})


def do(code):
    return "(function(){ %s; return 1 })()" % code


def key(name):
    code = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back"}[name]
    return "overlay.key(%s, true, false, 0)" % code


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Switcher(HostCase):
    def host(self, *args, data=None, surface="LumaHome.qml"):
        """The offline host without the default `hostActive=true` of HostCase: Home builds its whole chain when it starts active, and while it does the
        surface under test answers a timer late (the places of the cards went out a second after the opening)."""
        cmd = [sys.executable, HOST, surface, "--fake-helper", "--theme", '{"perf":{"syncResidents":true}}', *args]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "180")))
        return r.returncode, r.stdout + r.stderr

    def run_overlay(self, steps, wait, theme=PLAIN):
        """steps: (ms, js, label). Home hidden, the overlay built at once, a recorder on its requests to the compositor from 150 ms."""
        argv = ["--eval", "100:" + do("overlay.warm = true; overlay.warmSwitcher = true"), "--eval", "150:" + RECORDER]
        for ms, expr, label in steps:
            argv += ["--eval", "%d:(function(){ return %s + (%s) })()" % (ms + LATE, json.dumps(label + "="), expr)]
        rc, out = self.host(*argv, "--overlay", "--prop", "hostActive=false", "--theme", theme, "--wait", str(wait + LATE))
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got, out

    def posts(self, out, upto=None):
        """What the overlay asked of the compositor before the eval labelled `upto` (all of it when none)."""
        found = []
        for line in out.splitlines():
            m = re.search(r"\[rec\] (\{.*\})$", line)
            if m:
                found.append(json.loads(m.group(1)))
            elif upto and re.match(r"^\[eval\] \('%s=" % re.escape(upto), line):
                break
        return found

    def test_it_is_offered_only_while_the_switch_is_on_and_the_compositor_feeds_it(self):
        got, _ = self.run_overlay([
            (600, "overlay.panelReady('switcher')", "no_feed"),
            (700, feed(APPS, False), "fed"), (800, "overlay.panelReady('switcher')", "ready"), (850, "overlay.switcherCount()", "count"),
            (900, STATE, "state_closed"),
        ], 1300)
        self.assertEqual(got["no_feed"], "false", "under LG's compositor there is no second press of Home to offer")
        self.assertEqual(got["ready"], "true", "the shell tells the compositor it can show it: the second press is then the compositor's to take")
        self.assertEqual(got["count"], "4")
        self.assertEqual(got["state_closed"].split(" ")[:2], ["false", "false"], "closed it takes no key")
        got, _ = self.run_overlay([(700, feed(APPS, False), "fed"), (800, "overlay.panelReady('switcher')", "ready")], 1200, '{"perf":{"syncResidents":true}}')
        self.assertEqual(got["ready"], "false", "with the owner's switches off the shell does not say it can: Home is Home alone")
        got, _ = self.run_overlay([(700, feed(APPS, False), "fed"), (800, "overlay.panelReady('switcher')", "ready"), (850, "overlay.switcherCount()", "count"), (900, "overlay.openSwitcher()", "back_door")],
                                  1300, RECENTS_ONLY)
        self.assertEqual((got["ready"], got["count"], got["back_door"]), ("false", "4", "true"),
                         "Back on Home alone does not give the second press of Home to the compositor, but opens the same surface")

    def test_when_the_compositor_opens_it_the_cards_places_are_sent_once_and_the_dpad_walks_them(self):
        got, out = self.run_overlay([
            (700, feed(APPS, True), "opened"), (1000, STATE, "state_open"), (1100, key("right"), "right"), (1300, STATE, "state_right"), (1400, key("down"), "down"), (1500, STATE, "state_quit"),
            (1600, key("up"), "up1"), (1650, key("up"), "up2"), (1700, STATE, "state_all"),
            (1800, key("down"), "down2"), (1900, STATE, "state_back_to_card"),
        ], 2300)
        self.assertEqual(got["state_open"], "true true card/0/0/false", "it has the keys and its first card in focus")
        ops = self.posts(out, "state_open")
        self.assertEqual([o["op"] for o in ops], ["layout"], "one message with the places, nothing else asked yet")
        lay = ops[0]
        self.assertEqual([c["id"] for c in lay["cards"]], ["plex", "youtube.leanback.v4", "netflix"], "only the apps whose picture the compositor draws; the one on the video plane has a tile of its own")
        # four cards of 400x225 with 32 between, centred: the first at x 112; the third (the tile) takes its slot, so Netflix is in the fourth, at 1408
        self.assertEqual([(c["x"], c["y"], c["w"], c["h"], c["r"]) for c in lay["cards"]], [(112, 340, 400, 225, 22), (544, 340, 400, 225, 22), (1408, 340, 400, 225, 22)])
        self.assertEqual(lay["ms"], 200)
        self.assertAlmostEqual(lay["scrim"]["a"], 0.973, places=2)
        self.assertFalse(lay["reduced"])
        self.assertEqual(got["state_right"], "true true card/1/0/false")
        self.assertEqual(len(self.posts(out, "state_right")), 1, "moving the focus inside the window changes no place: nothing is sent")
        self.assertEqual(got["state_quit"], "true true quit/1/0/false", "Down goes to the card's own Quit")
        self.assertEqual(got["state_all"], "true true all/1/0/false", "and Up from a card goes to the header's Quit all")
        self.assertEqual(got["state_back_to_card"], "true true card/1/0/false")

    def test_ok_opens_the_card_quit_quits_it_and_back_closes(self):
        got, out = self.run_overlay([
            (700, feed(APPS, True), "opened"),
            (1000, key("right"), "r"), (1050, key("ok"), "ok"), (1300, key("down"), "d"), (1350, key("ok"), "quit"), (1550, STATE, "state_after_quit"),
            (1600, key("back"), "back"), ], 2100)
        posted = [o for o in self.posts(out) if o["op"] != "layout"]
        self.assertEqual(posted, [{"op": "activate", "id": "youtube.leanback.v4"}, {"op": "quit", "id": "youtube.leanback.v4"}, {"op": "close"}])
        self.assertEqual(got["state_after_quit"], "true true card/1/0/false", "after a Quit the focus is on the card again")

    def test_quit_all_asks_first_and_cancel_is_where_the_focus_starts(self):
        got, out = self.run_overlay([
            (700, feed(APPS, True), "opened"),
            (1000, key("up"), "up"), (1050, key("ok"), "ask"), (1100, STATE, "state_ask"), (1200, key("ok"), "cancel"), (1250, STATE, "state_cancelled"),
            (1300, key("ok"), "ask_again"), (1350, key("right"), "yes"), (1400, key("ok"), "quit_all"), (1600, key("up"), "up2"), (1650, key("ok"), "ask3"), (1700, key("back"), "back_cancels"), (1750, STATE, "state_after_back"),
        ], 2100)
        self.assertEqual(got["state_ask"], "true true all/0/0/true")
        self.assertEqual([o["op"] for o in self.posts(out, "state_ask")], ["layout"], "nothing is quit by asking")
        self.assertEqual(got["state_cancelled"], "true true all/0/0/false", "OK on Cancel leaves it as it was")
        self.assertEqual([o["op"] for o in self.posts(out, "quit_all")], ["layout", "quitall"], "and only the second yes quits them all")
        self.assertEqual(got["state_after_back"], "true true all/0/0/false", "Back on the question is Cancel, not the switcher's own Back")

    def test_more_than_four_apps_are_a_window_of_four_that_moves_a_card_at_a_time(self):
        many = [{"id": "app%d" % i, "title": "App %d" % i, "picture": "live", "state": ""} for i in range(7)]
        got, out = self.run_overlay([
            (700, feed(many, True), "opened"), (1100, key("right"), "r1"), (1120, key("right"), "r2"), (1140, key("right"), "r3"), (1160, key("right"), "r4"), (1400, STATE, "state_moved"), ], 1900)
        first = self.posts(out, "r1")[0]["cards"]
        self.assertEqual([c["id"] for c in first], ["app0", "app1", "app2", "app3"])
        self.assertEqual(got["state_moved"], "true true card/4/1/false", "the fifth card is in focus and the window moved by one")
        last = self.posts(out)[-1]["cards"]
        self.assertEqual([c["id"] for c in last], ["app1", "app2", "app3", "app4"])
        self.assertEqual([c["x"] for c in last], [112, 544, 976, 1408], "the row moved a card to the left")

    def test_the_quit_of_a_card_that_is_closing_is_not_asked_again_and_a_stuck_one_can_be_tried_again(self):
        closing = [dict(APPS[0], state="quitting")] + APPS[1:2]
        stuck = [dict(APPS[0], state="stuck")] + APPS[1:2]
        got, out = self.run_overlay([
            (700, feed(closing, True), "opened"), (1000, key("down"), "d"), (1050, key("ok"), "quit"), (1150, key("back"), "esc"), (1300, feed(stuck, True, 2), "stuck"), (1500, key("down"), "d2"), (1550, key("ok"), "retry"), ], 2000)
        self.assertNotIn({"op": "quit", "id": "plex"}, self.posts(out, "esc"), "an app that is closing is not asked to close again")
        self.assertIn({"op": "quit", "id": "plex"}, self.posts(out), "one that did not close can be asked again")

    def test_reduced_motion_is_said_to_the_compositor(self):
        got, out = self.run_overlay([(700, feed(APPS, True), "opened")], 1400, '{"perf":{"syncResidents":true},"remote":{"appSwitcher":true},"motion":{"reduced":true}}')
        lay = self.posts(out)[0]
        self.assertEqual((lay["reduced"], lay["ms"]), (True, 120))

    def test_over_lgs_compositor_back_on_home_opens_the_same_surface_with_lgs_list_and_the_app_managers_calls(self):
        got, _ = self.run_overlay([
            (900, do("overlay.switcherItem.readLg()"), "read"), (1200, "overlay.switcherCount()", "count"),
            (1300, "overlay.openSwitcher()", "opened"), (1400, STATE, "state_open"),
            (1500, key("right"), "r"), (1550, key("ok"), "ok"), (1700, STATE, "state_after_ok"), (1750, CALLS, "calls_ok"),
            (1800, "overlay.openSwitcher()", "again"), (1950, key("down"), "d"), (2000, key("ok"), "quit"), (2400, CALLS, "calls_quit"),
            (2450, key("back"), "back"), (2600, STATE, "state_back"),
        ], 3000, RECENTS_ONLY)
        self.assertEqual(got["count"], "4", "LG's list, without LumaTV's own card")
        self.assertEqual(got["state_open"].split(" ")[:2], ["true", "true"])
        self.assertEqual(got["state_after_ok"].split(" ")[0], "false")
        launches = [c for c in json.loads(got["calls_ok"]) if c["m"] == "/launch"]
        self.assertEqual([c["p"] for c in launches], [{"id": "com.beb.plxnative"}], "the second frame, once")
        calls = json.loads(got["calls_quit"])
        closed = [c["p"] for c in calls if c["m"] == "/closeByAppId"]
        removed = [c["p"] for c in calls if c["m"] == "/removeRecent"]
        self.assertEqual(closed, [{"id": "netflix", "reason": "windowPolicy"}], "the app is closed first")
        self.assertEqual(removed, [{"appId": "netflix"}], "then its entry, since removing the entry alone leaves the app running")
        self.assertEqual(got["state_back"].split(" ")[0], "false", "Back closes it")


if __name__ == "__main__":
    unittest.main()
