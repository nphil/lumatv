"""Run with: python3 -m unittest discover -s tests -p 'test_profiles_host.py'   (needs Xvfb and PyQt5; skipped without them)

What profiles change in the QML that is still there (the screens themselves move to the Rust shell; the logic is the helper's, `helper/app/src/profiles.rs`,
with its own host tests): the status bar's profile item is the person in front of the TV and LG's account is not read; its popover offers the other
profiles; a person's own dock (theme `apps.dockOrder` and `apps.dockHide`); and a switch announced on the helper's `profiles` topic is read and
applied at once (`LumaHome.profilesChanged`), which this measures on the offline host (software GL, Qt 5.15: a ceiling for the logic, not the set's timing).
"""
import json
import os
import re
import tempfile
import unittest

from test_settings import HostCase
from test_shell_surfaces import KEYS, READY, STATE, calls, probe, CALLS
from xdisplay import DISPLAY, HAVE_QT

NOW = 1730000000000


def topic(active="nitin", can_admin=True, locked=("isabel",)):
    people = [("nitin", "Nitin", "#A79BF5", True), ("isabel", "Isabel", "#EB6F92", False), ("mia", "Mia", "#F6C177", False)]
    return {"ready": True, "active": active, "owner": "nitin", "canAdmin": can_admin, "pickerOnWake": False, "startWith": "last", "max": 6, "switchedAt": NOW,
            "profiles": [{"id": i, "name": n, "initial": n[0], "color": c, "avatar": {"kind": "initial"}, "avatarFile": None, "owner": o, "active": i == active,
                          "locked": i in locked, "firstRun": False} for i, n, c, o in people]}


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class ProfilesHost(HostCase):
    CODES = KEYS

    def run_host(self, steps, wait, data=None, theme=None, publish=()):
        args = []
        for st in steps:
            ms, kind, what = st[:3]
            if kind == "keys":
                args += self.keys(READY + ms, *what.split())
            else:
                args += ["--eval", "%d:(function(){ return %s + (%s) })()" % (READY + ms, json.dumps(st[3] + "="), what)]
        for ms, name, doc in publish:
            args += ["--publish", "%d:%s:%s" % (READY + ms, name, json.dumps(doc))]
        if theme is not None:
            args += ["--theme", json.dumps(dict(theme, perf={"syncResidents": True}))]
        rc, out = self.host(*args, "--wait", str(READY + wait), data=data)
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got, out

    def test_the_status_bar_shows_the_person_in_front_of_the_tv_and_never_lgs_account(self):
        bar = probe("profileName", "{ name: s.profileName, initial: s.profileInitial, color: String(s.profileColor), rows: s.profileRows().map(function(r) { return r.label + '/' + r.action + '/' + r.id }) }", panel=False)
        locked_off = probe("profileName", "s.profileRows().map(function(r) { return r.id })", panel=False)
        got, out = self.run_host([(100, "eval", bar, "bar"), (200, "eval", CALLS, "calls")], 1500, publish=[(0, "profiles", topic("nitin"))])
        want = json.loads(got["bar"])
        self.assertEqual((want["name"], want["initial"]), ("Nitin", "N"))
        self.assertEqual(want["color"].lower(), "#a79bf5")
        # the owner is active: both other profiles are offered, the one with a PIN marked
        self.assertEqual(want["rows"], ["Isabel  \u00b7 PIN/switch/isabel", "Mia/switch/mia"])
        # LG's account is not asked for by Home any more
        self.assertEqual(calls("/getLoginUserData")(got["calls"]), [])
        # someone who is not the owner is not offered a profile that has a PIN (this popover has no pad to ask it with)
        got2, _ = self.run_host([(100, "eval", locked_off, "ids")], 800, publish=[(0, "profiles", topic("mia", can_admin=False))])
        self.assertEqual(json.loads(got2["ids"]), ["nitin"])
        got3, _ = self.run_host([(100, "eval", locked_off, "ids")], 800, publish=[(0, "profiles", topic("isabel", can_admin=False, locked=("nitin", "isabel")))])
        self.assertEqual(json.loads(got3["ids"]), ["mia"], "Nitin has a PIN: only the owner is let into a profile with one without asking")

    def test_the_status_bar_has_no_name_and_a_question_mark_until_the_helper_has_made_the_profiles(self):
        bar = probe("profileName", "{ name: s.profileName, initial: s.profileInitial, rows: s.profileRows().length }", panel=False)
        got, _ = self.run_host([(100, "eval", bar, "bar")], 800)
        self.assertEqual(json.loads(got["bar"]), {"name": "", "initial": "", "rows": 0})
        got, _ = self.run_host([(100, "eval", bar, "bar")], 800, publish=[(0, "profiles", {"ready": False, "canAdmin": True})])
        self.assertEqual(json.loads(got["bar"]), {"name": "", "initial": "", "rows": 0})

    def test_a_dock_is_the_persons_own_lgs_list_is_what_is_installed(self):
        ids = probe("appsTileId", "s.points.map(function(p) { return p.id })", panel=False)
        got, _ = self.run_host([(100, "eval", ids, "default")], 800)
        default = json.loads(got["default"])
        self.assertGreater(len(default), 4)
        apps_tile = default[0]
        hidden = default[3]
        first_moved = default[4]
        mine = {"apps": {"dockHide": [hidden], "dockOrder": [first_moved, default[2]]}}
        got, _ = self.run_host([(100, "eval", ids, "mine")], 800, theme=mine)
        arranged = json.loads(got["mine"])
        self.assertEqual(arranged[0], apps_tile, "LG's own first tile stays first")
        self.assertEqual(arranged[1:3], [first_moved, default[2]], "the tiles the person placed come first, in their order")
        self.assertNotIn(hidden, arranged)
        rest = [i for i in default if i not in (apps_tile, hidden, first_moved, default[2])]
        self.assertEqual(arranged[3:], rest, "the others keep LG's order after them")
        self.assertEqual(len(arranged), len(default) - 1)

    # What the helper does at a switch, as a file the host can write at once: the other person's keys in theme.json (a TextFile write is synchronous;
    # an XMLHttpRequest PUT, which Home itself saves with, waits in the network queue behind the helper's long poll for seconds on the host).
    WRITE = ("(function(){ var f = Qt.createQmlObject('import TextFile 1.0; TextFile {}', root); f.name = root.themeDirUrl + 'theme.json'; "
             "f.write(JSON.stringify({ accent: '#FF0000', material: 'light', perf: { syncResidents: true } })); return Date.now() })()")

    def test_a_switch_on_the_topic_is_read_at_once_and_what_was_open_is_closed(self):
        steps = [(200, "eval", "root.openSettings()", "open"), (900, "eval", self.WRITE, "mark")]
        times = list(range(900, 1700, 25))
        steps += [(t, "eval", "Date.now() + ':' + root.theme.accent + ':' + root.isLight", "s%04d" % t) for t in times]
        steps += [(1800, "eval", "root.settingsOpen", "settings_after")]
        # the topic's first answers (nothing to reload), then the change of person 20 ms after the file was written
        pub = [(0, "profiles", topic("nitin", locked=())), (100, "profiles", topic("nitin", locked=())), (920, "profiles", topic("isabel", can_admin=False, locked=()))]
        got, out = self.run_host(steps, 2200, publish=pub)
        mark = int(got["mark"])
        seen = [(int(got["s%04d" % t].split(":")[0]), got["s%04d" % t].split(":")[1:]) for t in times if "s%04d" % t in got]
        self.assertTrue(seen, out[-800:])
        applied = [(at, v) for at, v in seen if v[0] == "#FF0000"]
        self.assertTrue(applied, "the switch was never applied: %s" % seen)
        took = applied[0][0] - (mark + 20)
        self.assertEqual(applied[0][1], ["#FF0000", "true"], "the theme is the other person's whole: the accent and the light glass")
        self.assertLess(took, 500, "a switch took %d ms to reach Home's data on the host (%s)" % (took, seen))
        self.assertEqual(got["settings_after"], "false", "Settings was open for the last person: the switch closes it")
        print("\n[profiles] the change on the topic reached Home's theme %d ms after it was published (host: software GL, a loaded machine; the sample that first saw it)" % took)

    def test_only_a_change_of_person_drops_what_is_open_a_name_or_a_first_answer_does_not(self):
        steps = [(100, "eval", "root.openApps()", "opened"), (200, "eval", "root.layerOpen", "before"), (700, "eval", "root.layerOpen", "after_rename"),
                 (1200, "eval", "root.layerOpen", "after_switch")]
        pub = [(0, "profiles", topic("nitin")), (300, "profiles", dict(topic("nitin"), pickerOnWake=True)),
               (500, "profiles", dict(topic("nitin"), profiles=[dict(p, name=p["name"] + "x") for p in topic("nitin")["profiles"]])),
               (900, "profiles", topic("isabel", can_admin=False))]
        got, _ = self.run_host(steps, 1600, publish=pub)
        self.assertEqual((got["before"], got["after_rename"]), ("apps", "apps"), "a change that is not a switch left the library alone")
        self.assertEqual(got["after_switch"], "", "the change of person closed it")


if __name__ == "__main__":
    unittest.main()
