"""Run with: python3 -m unittest tests.test_gate_surfaces   (the host tests need Xvfb and PyQt5; they skip without them)

The frame gate's screen walks and launches (docs/research/perf/gate.py: settings-scroll, apps-grid, home-widgets, switcher, transitions-*,
keyboard-*): the table's numbers and tokens, the settings a surface turns on and puts back, and where the keys take the screens. Nothing here
measures time: the set's gate does. The walks are replayed on the real QML through the offline host, key by key as kb.py would send them (taps,
held keys with their repeats), so that a walk that opens the wrong screen, presses an OK that launches an app or changes a setting, or ends
away from Home shows up here and not on the TV.

A host run takes about fifteen seconds, so each test is one run that replays one or two cycles.
"""
import ast
import json
import os
import re
import sys
import unittest
from unittest import mock
from unittest.mock import Mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "docs", "research", "perf"))
import gate  # noqa: E402
import test_keyboard_own as keyboard_host  # noqa: E402
import test_switcher as switcher_host  # noqa: E402
from test_settings import HostCase  # noqa: E402
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

NEW = ["settings-scroll", "apps-grid", "home-widgets", "switcher", "transitions-netflix", "transitions-settings", "keyboard-luma", "keyboard-lg"]
WALKS = ["settings-scroll", "apps-grid", "home-widgets", "switcher", "keyboard-luma", "keyboard-lg"]
LAUNCHES = ["transitions-netflix", "transitions-settings"]
VALUES = {"INPUT": "241", "HDMI": "3"}


def surface(name):
    return gate.find([name], values=VALUES)[0]


def key_names(tokens):
    """The keys a token list presses, in order, once for each token (a marker, a sleep, a luna call and a number key press none of them)."""
    return [re.match(r"[A-Z]+", t).group(0) for t in tokens if t[0] not in "@!{0123456789" and not re.fullmatch(r"s[\d.]+", t)]


def walk_of(s):
    """What a walk does after the key that enters its screen."""
    return gate.open_tokens(s)[4:] if s["act"] == "release" else s["open"].split()[2 if s["tile"] else 1:]


class Table(unittest.TestCase):
    def test_the_new_surfaces_are_in_the_table_and_run_when_named(self):
        self.assertTrue(gate.validate(gate.SURFACES))
        defaults = [e["name"] for e in gate.SURFACES if gate.surface(e)["default"]]
        for n in NEW:
            self.assertNotIn(n, defaults)
        self.assertEqual([s["name"] for s in gate.find(NEW, values=VALUES)], NEW)

    def test_a_cycle_fits_the_trace_ring_and_the_plan_covers_the_measured_cycles(self):
        for n in NEW:
            s = surface(n)
            self.assertLessEqual(gate.cycle_seconds(s), gate.FULL_CAPTURE_MAX_S, n)
            self.assertEqual(sum(gate.plan_full(s, gate.MEASURE_CYCLES)), gate.MEASURE_CYCLES, n)

    def test_the_bars_are_the_gates_and_only_the_checks_a_key_cannot_answer_are_off(self):
        for n in NEW:
            th = surface(n)["thresholds"]
            for k in ("p90_render_ms", "p90_window_ms", "dropped", "burst_over20"):
                self.assertEqual(th[k], gate.THRESHOLDS[k], (n, k))
            self.assertIsNone(th["burst_end_ms"], n)         # a walk's burst has no end of its own, and an app's launch is the app's
            if n in LAUNCHES:
                self.assertIsNone(th["first_frame_ms"], n)   # the app's resume and SAM's round trip come first
            else:
                self.assertEqual(th["first_frame_ms"], gate.THRESHOLDS["first_frame_ms"], n)

    def test_the_walks_are_workloads_on_a_screen_that_stays_up_and_the_launches_are_transitions(self):
        for n in WALKS:
            self.assertFalse(surface(n)["transition"], n)
            self.assertTrue(surface(n)["still"], n)          # the wallpaper's tick is off, as for every surface
        for n in LAUNCHES:
            s = surface(n)
            self.assertTrue(s["transition"] and s["ctl_transitions"] and s["app_retries"] == 0, n)
            self.assertIs(s["switches"]["transitions"], True)

    def test_a_walk_presses_no_ok_inside_a_screen_and_no_key_that_changes_a_value(self):
        # OK in the library launches an app, in Settings it flips a row, in the switcher it opens a card and Down is a card's Quit; Left and Right
        # inside a Settings page move a slider; Back on Home's own top opens the recents. The only OKs are the ones that enter a screen.
        for n in WALKS:
            self.assertNotIn("OK", key_names(walk_of(surface(n))), n)
        self.assertNotIn("BACK", key_names(walk_of(surface("home-widgets"))))
        self.assertEqual(set(key_names(walk_of(surface("switcher")))), {"LEFT", "RIGHT", "BACK"})
        self.assertEqual(set(key_names(walk_of(surface("keyboard-luma")))), {"UP", "DOWN", "LEFT", "RIGHT"})
        settings = walk_of(surface("settings-scroll"))
        self.assertEqual(key_names(settings).count("RIGHT"), 2)                      # the two entries into a page, from the rail
        self.assertEqual(key_names(gate.SETTINGS_PAGE.split()), ["RIGHT", "DOWN", "UP", "BACK"])

    def test_a_screen_that_was_entered_is_left_by_the_walk(self):
        for n in ("settings-scroll", "apps-grid", "switcher"):
            self.assertEqual(key_names(walk_of(surface(n)))[-1], "BACK", n)
        for n in ("settings-scroll", "switcher"):
            self.assertEqual(surface(n)["act"], "release", n)               # entered by a key acted on at its release
        self.assertEqual(surface("apps-grid")["act"], "press")             # a number key is acted on at its press

    def test_prep_is_a_window_of_its_own_and_comes_before_the_cycle(self):
        toks = gate.cycle_tokens(surface("settings-scroll"))
        self.assertEqual(toks[0], "@settings-scroll-prep")
        self.assertLess(toks.index("@settings-scroll-prep"), toks.index("@settings-scroll-open"))
        self.assertLess(toks.index("@settings-scroll-open"), toks.index("@settings-scroll-close"))
        self.assertTrue(all(not t.startswith("@") for t in surface("settings-scroll")["prep"].split()))
        self.assertNotIn("@home-dpad-prep", gate.cycle_tokens(gate.find(["home-dpad"])[0]))          # a surface without one adds no marker
        with self.assertRaises(ValueError):
            gate.validate([{"name": "x", "prep": "@x-prep UP", "open": "@x-open UP", "close": "@x-close"}])

    def test_the_release_entry_keeps_its_walk_inside_the_open_window(self):
        s = surface("settings-scroll")
        toks = gate.open_tokens(s)
        self.assertEqual(toks[:4], ["OK+", "s0.05", "@settings-scroll-open", "OK-"])
        self.assertEqual(toks[4:], s["open"].split()[2:])
        for bad in ("@x-open", "@x-open RIGHTx2 UP", "@x-open s1 UP"):
            with self.assertRaises(ValueError):
                gate.validate([{"name": "x", "act": "release", "open": bad, "close": "@x-close"}])
        self.assertTrue(gate.validate([{"name": "x", "act": "release", "open": "@x-open OK s1 RIGHT", "close": "@x-close"}]))

    def test_nothing_counts_on_where_the_owner_put_a_card_a_status_bar_item_or_a_tile(self):
        # the keys that start a cycle: Home's top (Up until the status bar, Left to its first item: it clamps there), the gear's quick panel (Down
        # to its last button, which Down does not leave), and a dock tile's number; no count of Right or Down hops over Home's rows
        self.assertEqual(gate.HOME_TOP.split(), ["UPx8@0.1", "LEFTx6@0.06"])
        self.assertEqual(key_names(gate.QUICK_PANEL_FOOT.split()), ["MENU", "DOWN"])
        self.assertEqual(surface("settings-scroll")["prep"], gate.QUICK_PANEL_FOOT)
        self.assertEqual(surface("transitions-settings")["prep"], gate.QUICK_PANEL_FOOT + " OK s1.0 DOWNx12@0.04 s0.3")
        for n in ("apps-grid", "transitions-netflix"):
            self.assertEqual(surface(n)["prep"], "", n)
        for n in ("settings-scroll", "apps-grid", "transitions-netflix", "transitions-settings"):
            self.assertNotIn("home_layout", surface(n), n)

    def test_a_dock_tile_is_opened_by_its_number_among_the_tiles_that_are_shown(self):
        # the evdev codes: KEY_1 = 2 ... KEY_9 = 10, KEY_0 = 11
        points = [{"id": "a"}, {"id": "hidden", "hidden": True}] + [{"id": "p%d" % i} for i in range(20)] + [{"id": "netflix"}]
        self.assertEqual(gate.tile_digit(points, "a"), (2, 1))
        self.assertEqual(gate.tile_digit(points, "p0"), (3, 2))                            # the hidden one takes no place
        self.assertEqual(gate.tile_digit(points, "p7"), (10, 9))
        self.assertEqual(gate.tile_digit(points, "p8"), (11, 10))                          # the tenth is the 0 key
        self.assertIsNone(gate.tile_digit(points, "p9"))                                   # the eleventh has no number
        self.assertIsNone(gate.tile_digit(points, "netflix"))
        self.assertIsNone(gate.tile_digit(points, "missing"))

    def test_a_launch_surface_has_the_digit_token_its_tile_needs_and_the_two_go_together(self):
        for n in ("apps-grid", "transitions-netflix"):
            s = surface(n)
            self.assertIn("{DIGIT}", s["open"].split())
            self.assertEqual(s["switches"]["number-keys"], True)
            self.assertEqual(s["switches"]["quick-access"], False)
        self.assertEqual(surface("apps-grid")["tile"], {"app": "com.webos.app.discovery"})
        self.assertNotIn("tile", [k for k, v in surface("transitions-settings").items() if v])
        with self.assertRaises(ValueError):
            gate.validate([{"name": "x", "open": "@x-open {DIGIT}", "close": "@x-close"}])
        with self.assertRaises(ValueError):
            gate.validate([{"name": "x", "tile": {"app": "a"}, "open": "@x-open UP", "close": "@x-close"}])
        with self.assertRaises(ValueError):
            gate.validate([{"name": "x", "switches": {"nothing": 1}, "open": "@x-open UP", "close": "@x-close"}])


class Switches(unittest.TestCase):
    """The settings a surface turns on for its run are the owner's again afterwards, read back."""

    class Set:
        """The tool on the set: theme.json, `transitions` and `keyboard`, as `lumatv` answers them."""

        def __init__(self, theme=None, transitions=False, keyboard="lg", fail=()):
            self.theme, self.transitions, self.keyboard, self.fail, self.log = theme if theme is not None else {}, transitions, keyboard, fail, []

        def sh(self, cmd, timeout=120):
            if gate.TOOL not in cmd:
                return 0, "", ""
            self.log.append(cmd)
            args = cmd.split(gate.TOOL + " ", 1)[1]
            if args in self.fail:
                return 1, "", "refused"
            if args == "theme get":
                return 0, json.dumps({"ok": True, "theme": self.theme}), ""
            if args.startswith("theme set "):
                self.merge(self.theme, json.loads(args[len("theme set "):].strip("'")))      # tools/theme.py merges objects key by key
                return 0, "{}", ""
            if args == "transitions status":
                return 0, json.dumps({"ok": True, "transitions": {"on": self.transitions, "compositor": "own"}}), ""
            if args in ("transitions on", "transitions off"):
                self.transitions = args.endswith("on")
                return 0, "{}", ""
            if args == "keyboard status":
                return 0, json.dumps({"ok": True, "on": False, "patched": False, "keyboard": self.keyboard}), ""
            if args in ("keyboard luma", "keyboard lg"):
                self.keyboard = args.split()[1]
                return 0, "{}", ""
            raise AssertionError(cmd)

        @classmethod
        def merge(cls, base, over):
            for k, v in over.items():
                if isinstance(v, dict) and isinstance(base.get(k), dict):
                    cls.merge(base[k], v)
                else:
                    base[k] = v

    def rig(self, **kw):
        rig = Mock()
        rig.set = self.Set(**kw)
        rig.sh = rig.set.sh
        rig.compositor_pid.return_value = 17
        rig.save_ring.return_value = None
        return rig

    def during_and_after(self, rig, wanted):
        saved = []
        with mock.patch.object(gate.time, "sleep"):
            gate.set_switches(rig, wanted, saved)
            during = (rig.set.theme.get("remote", {}).get("appSwitcher"), rig.set.transitions, rig.set.keyboard)
            return saved, during, gate.restore_switches(rig, saved)

    def test_the_switcher_is_on_for_the_run_and_off_again(self):
        rig = self.rig()
        saved, during, problems = self.during_and_after(rig, {"app-switcher": True})
        self.assertEqual((saved, during[0], problems), ([("app-switcher", False)], True, ""))
        self.assertIs(rig.set.theme["remote"]["appSwitcher"], False)

    def test_the_number_keys_are_on_and_quick_access_off_for_the_run_and_the_owners_own_choices_are_kept(self):
        own = {"remote": {"numberKeys": False, "quickAccess": {"on": True, "slots": {"4": "com.example.app"}}}}
        rig = self.rig(theme=json.loads(json.dumps(own)))
        saved = []
        with mock.patch.object(gate.time, "sleep"):
            gate.set_switches(rig, gate.NUMBER_KEYS, saved)
            during = json.loads(json.dumps(rig.set.theme))
            problems = gate.restore_switches(rig, saved)
        self.assertEqual(during, {"remote": {"numberKeys": True, "quickAccess": {"on": False, "slots": {"4": "com.example.app"}}}})
        self.assertEqual((problems, rig.set.theme), ("", own), "back as the owner had them, the slot included")
        # the shipped defaults (nothing in the file) need no write: the keys are on and Quick Access is off
        rig = self.rig(theme={})
        saved, during, problems = self.during_and_after(rig, gate.NUMBER_KEYS)
        self.assertEqual((rig.set.theme, problems), ({}, ""))
        self.assertEqual([c for c in rig.set.log if "theme set" in c], [])

    def test_what_the_owner_had_on_stays_on_and_is_not_written(self):
        rig = self.rig(theme={"remote": {"appSwitcher": True}}, transitions=True, keyboard="luma")
        saved, during, problems = self.during_and_after(rig, {"app-switcher": True, "transitions": True, "keyboard": "luma"})
        self.assertEqual(problems, "")
        self.assertEqual([c for c in rig.set.log if " set " in c or c.endswith((" on", " off", " luma", " lg"))], [])
        self.assertEqual(rig.set.theme, {"remote": {"appSwitcher": True}})

    def test_transitions_and_both_keyboards_come_back_as_found(self):
        for was in (False, True):
            rig = self.rig(transitions=was)
            saved, during, problems = self.during_and_after(rig, {"transitions": not was})
            self.assertEqual((during[1], rig.set.transitions, problems), (not was, was, ""))
        for was, run in (("lg", "luma"), ("luma", "lg"), ("lg", "lg")):
            rig = self.rig(keyboard=was)
            saved, during, problems = self.during_and_after(rig, {"keyboard": run})
            self.assertEqual((during[2], rig.set.keyboard, problems), (run, was, ""))

    def test_a_value_that_does_not_come_back_is_reported(self):
        rig = self.rig(transitions=False)
        saved = []
        with mock.patch.object(gate.time, "sleep"):
            gate.set_switches(rig, {"transitions": True}, saved)
            rig.set.fail = ("transitions off",)
            problems = gate.restore_switches(rig, saved)
        self.assertIn("transitions", problems)
        self.assertTrue(rig.set.transitions)

    def test_a_tool_that_will_not_answer_stops_the_surface_and_leaves_what_it_touched_to_be_put_back(self):
        rig = self.rig(fail=("keyboard status",))
        saved = []
        with mock.patch.object(gate.time, "sleep"), self.assertRaisesRegex(RuntimeError, "keyboard status failed"):
            gate.set_switches(rig, {"transitions": True, "keyboard": "luma"}, saved)
        self.assertEqual(saved, [("transitions", False)])
        self.assertTrue(rig.set.transitions)
        rig.set.fail = ()
        with mock.patch.object(gate.time, "sleep"):
            self.assertEqual(gate.restore_switches(rig, saved), "")
        self.assertFalse(rig.set.transitions)

    def test_a_surface_that_fails_gives_the_owners_values_back(self):
        rig = self.rig(keyboard="lg")
        rig.capture.side_effect = RuntimeError("capture failed")
        with mock.patch.object(gate.time, "sleep"):
            res = gate.run_surface(rig, surface("keyboard-luma"), "t", 0, Mock(since=Mock(return_value=([], True))))
        by = {c["name"]: c for c in res["checks"]}
        self.assertFalse(by["run"]["ok"])
        self.assertTrue(by["switches_restored"]["ok"])
        self.assertEqual(rig.set.keyboard, "lg")
        # the setup ran after the switch and the teardown before it was put back
        commands = [c for c in rig.set.log]
        self.assertLess(commands.index(gate.TOOL + " keyboard luma"), commands.index(gate.TOOL + " keyboard lg"))

    def test_a_value_that_will_not_come_back_fails_the_surface(self):
        rig = self.rig()
        orig = rig.set.sh
        rig.sh = lambda cmd, timeout=120: (1, "", "refused") if cmd.endswith("transitions off") else (
            (0, json.dumps({"launchPoints": [{"id": "netflix"}]}), "") if "listLaunchPoints" in cmd else orig(cmd, timeout))
        rig.capture.side_effect = RuntimeError("capture failed")
        rig.transitions_stats.return_value = {"launches": 0, "leaves": 0, "skipped": 0, "lateGaps": 0, "maxGapMs": 0, "enabled": True}
        with mock.patch.object(gate.time, "sleep"):
            res = gate.run_surface(rig, surface("transitions-netflix"), "t", 0, Mock(since=Mock(return_value=([], True))))
        by = {c["name"]: c for c in res["checks"]}
        self.assertFalse(by["switches_restored"]["ok"])
        self.assertIn("transitions", by["switches_restored"]["detail"])
        self.assertIn("launch_path", by)

    def test_a_surface_without_switches_adds_no_check_and_touches_no_setting(self):
        rig = self.rig()
        rig.capture.side_effect = RuntimeError("capture failed")
        res = gate.run_surface(rig, surface("home-widgets"), "t", 0, Mock(since=Mock(return_value=([], True))))
        self.assertNotIn("switches_restored", [c["name"] for c in res["checks"]])
        self.assertEqual(rig.set.log, [])


class Transitions(unittest.TestCase):
    CTL = json.dumps({"ok": True, "enabled": True, "stats": {"launches": 4, "leaves": 3, "switches": 0, "interrupted": 0, "skipped": 1, "frames": 90,
                                                              "maxGapMs": 17, "lateGaps": 2, "veilCaps": 0}})
    BEFORE = {"launches": 4, "leaves": 3, "skipped": 1, "lateGaps": 2, "maxGapMs": 17, "enabled": True}

    def test_the_counters_are_read_off_ctl(self):
        self.assertEqual(gate.parse_transitions("noise\n" + self.CTL), self.BEFORE)
        with self.assertRaises(ValueError):
            gate.parse_transitions("no answer")

    def test_a_cycle_that_cut_instead_of_moving_or_dropped_a_frame_of_one_fails(self):
        good = dict(self.BEFORE, launches=7, leaves=6)
        c = {x["name"]: x for x in gate.transitions_checks(self.BEFORE, good, 3)}
        self.assertTrue(c["transitions_ran"]["ok"] and c["transitions_late_gaps"]["ok"])
        cut = dict(self.BEFORE, launches=7, leaves=5, enabled=False)
        c = {x["name"]: x for x in gate.transitions_checks(self.BEFORE, cut, 3)}
        self.assertFalse(c["transitions_ran"]["ok"])
        self.assertIn("OFF", c["transitions_ran"]["detail"])
        late = dict(good, lateGaps=3, maxGapMs=41)
        c = {x["name"]: x for x in gate.transitions_checks(self.BEFORE, late, 3)}
        self.assertFalse(c["transitions_late_gaps"]["ok"])
        self.assertIn("41", c["transitions_late_gaps"]["detail"])

    def test_the_surface_reads_the_counters_around_its_measured_cycles(self):
        s = dict(surface("transitions-netflix"), switches={}, soak=False)
        rig = Mock()
        rig.compositor_pid.return_value = 17
        rig.save_ring.return_value = None
        rig.lg_compositor.return_value = False
        rig.sh.side_effect = lambda cmd, timeout=120: (0, json.dumps({"launchPoints": [{"id": "netflix"}]}), "")
        rig.stop.return_value = ("/dev/null", "/dev/null")
        one = {"trace": "t", "windows": [{"name": "transitions-netflix-%s" % k, "first_frame_ms": 30.0, "burst_end_ms": 100.0, "burst_over20": 0, "dropped": 0,
                            "burst_anim_p90_ms": 2.0, "anim_render_ms": None, "burst_app_jobs": 3} for k in ("open", "close")]}
        rig.analyse.side_effect = lambda name: json.loads(json.dumps(one))
        rig.transitions_stats.side_effect = [dict(self.BEFORE), dict(self.BEFORE, launches=7, leaves=6)]
        res = gate.run_surface(rig, s, "t", 0, Mock(since=Mock(return_value=([], True))))
        by = {c["name"]: c for c in res["checks"]}
        self.assertEqual(rig.transitions_stats.call_count, 2)
        self.assertTrue(by["transitions_ran"]["ok"] and by["transitions_late_gaps"]["ok"], by)
        self.assertEqual(len(res["traces"]), len(gate.plan_full(s, gate.MEASURE_CYCLES)), "a launch is the app drawing: a capture whose burst held the app's jobs is not measured again")
        self.assertEqual(res["discarded"], [])
        self.assertEqual(res["transitions"]["after"]["launches"], 7)
        # a compositor that does not answer fails the surface and says why
        rig.transitions_stats.side_effect = RuntimeError("ctl transitions failed: no such file")
        by = {c["name"]: c for c in gate.run_surface(rig, s, "t", 0, Mock(since=Mock(return_value=([], True))))["checks"]}
        self.assertIn("ctl transitions failed", by["run"]["detail"])

    def test_the_tile_is_resolved_on_the_set_to_its_number_key(self):
        s = surface("transitions-netflix")
        rig = Mock()
        rig.sh.return_value = (0, json.dumps({"launchPoints": [{"id": "com.webos.app.discovery"}, {"id": "netflix"}, {"id": "com.palm.app.settings"}]}), "")
        got, how = gate.resolve_tile(rig, s)
        self.assertEqual(got["open"], "@transitions-netflix-open 3")
        self.assertIn("tile 2", how)
        self.assertEqual(gate.resolve_tile(rig, surface("apps-grid"))[0]["open"].split()[1], "2")            # the Apps tile: KEY_1
        self.assertNotIn("{DIGIT}", gate.cycle_command(got, 1))
        # the owner moved Netflix to the tenth place: the 0 key; to the eleventh: it has no key, the surface stops before it presses one
        points = [{"id": "p%d" % i} for i in range(9)] + [{"id": "netflix"}, {"id": "x"}]
        rig.sh.return_value = (0, json.dumps({"launchPoints": points}), "")
        self.assertEqual(gate.resolve_tile(rig, s)[0]["open"], "@transitions-netflix-open 11")
        rig.sh.return_value = (0, json.dumps({"launchPoints": points[:9] + [{"id": "x"}, {"id": "netflix"}]}), "")
        with self.assertRaisesRegex(RuntimeError, "no tile among the dock's first 10.*nothing was pressed"):
            gate.resolve_tile(rig, s)
        rig.sh.return_value = (1, "", "luna down")
        with self.assertRaisesRegex(RuntimeError, "launch points could not be read"):
            gate.resolve_tile(rig, s)
        rig.sh.return_value = (0, "garbage", "")
        with self.assertRaisesRegex(RuntimeError, "answered"):
            gate.resolve_tile(rig, s)
        self.assertEqual(gate.resolve_tile(rig, surface("home-widgets")), (surface("home-widgets"), ""))

    def test_a_surface_whose_tile_has_no_number_presses_nothing_and_changes_nothing(self):
        rig = Mock()
        rig.compositor_pid.return_value = 17
        rig.sh.return_value = (0, json.dumps({"launchPoints": [{"id": "p%d" % i} for i in range(12)] + [{"id": "netflix"}]}), "")
        res = gate.run_surface(rig, surface("transitions-netflix"), "t", 0, Mock(since=Mock(return_value=([], True))))
        by = {c["name"]: c for c in res["checks"]}
        self.assertIn("no tile among the dock's first 10", by["run"]["detail"])
        commands = [c.args[0] for c in rig.sh.call_args_list]
        self.assertEqual(commands[0], "luna-send -n 1 luna://com.webos.service.homelaunchpoints/listLaunchPoints '{}'")
        self.assertFalse([c for c in commands if "theme set" in c or "transitions" in c or "kb.py" in c], commands)    # no setting changed, no key pressed
        rig.capture.assert_not_called()


class Setups(unittest.TestCase):
    def test_the_search_app_is_opened_with_its_keyboard_up_and_closed_after(self):
        for n in ("keyboard-luma", "keyboard-lg"):
            s = surface(n)
            self.assertEqual(s["setup"], gate.VOICE_SETUP[:4], n)
            self.assertNotIn("kb.py", " ".join(s["setup"]), n)            # Back would hide the keyboard
            self.assertEqual(s["teardown"], gate.VOICE_TEARDOWN, n)
        self.assertEqual(surface("keyboard-luma")["switches"], {"keyboard": "luma"})
        self.assertEqual(surface("keyboard-lg")["switches"], {"keyboard": "lg"})

    def test_both_keyboards_get_the_same_walk_and_it_is_balanced(self):
        a, b = walk_of(surface("keyboard-luma")), walk_of(surface("keyboard-lg"))
        self.assertEqual(a, b)
        ups = [t for t in a if t.startswith(("LEFT", "UP"))]
        downs = [t for t in a if t.startswith(("RIGHT", "DOWN"))]
        self.assertEqual(sorted(t.replace("LEFT", "RIGHT").replace("UP", "DOWN") for t in ups), sorted(downs))

    def test_the_switcher_has_apps_running_with_home_in_front_and_closes_them_all(self):
        s = surface("switcher")
        launched = [c for c in s["setup"] if "/launch" in c and gate.HOME_ID not in c]
        self.assertEqual(len(launched), 3)
        self.assertIn("netflix", launched[0])
        self.assertIn(gate.HOME_ID, s["setup"][-2])
        for app, _ in gate.SWITCHER_APPS:
            self.assertTrue(any("closeByAppId" in c and app in c for c in s["teardown"]), app)
        self.assertIn(gate.HOME_ID, s["teardown"][-2])
        self.assertEqual(s["switches"], {"app-switcher": True})

    def test_the_switcher_opens_on_the_release_of_home_and_is_named_for_the_compositors_marker(self):
        s = surface("switcher")
        self.assertEqual(gate.open_tokens(s)[:4], ["HOME+", "s0.05", "@switcher-open", "HOME-"])
        with open(os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "switcher.rs")) as f:
            self.assertIn('"Q:switcher-show"', f.read())            # annotate_shows() reads `Q:<name>-show`
        trace = "\n".join(["  <...>-1 [001] .... 10.000: tracing_mark_write: P:switcher-open",
                           "  <...>-1 [001] .... 10.040: tracing_mark_write: Q:switcher-show", ""])
        rep = {"windows": [{"name": "switcher-open"}]}
        gate.annotate_shows(rep, trace, "switcher")
        self.assertEqual(rep["windows"][0]["show_ms"], 40.0)

    def test_a_launch_is_from_a_running_app_and_ends_with_it_gone_and_home_in_front(self):
        for n, app in (("transitions-netflix", "netflix"), ("transitions-settings", "com.palm.app.settings")):
            s = surface(n)
            self.assertIn("/launch", s["setup"][2])
            self.assertIn(app, s["setup"][2])
            self.assertIn(gate.HOME_ID, s["setup"][4])               # then Home over it: the launch from the tile is a resume
            self.assertIn("closeByAppId", s["teardown"][0])
            self.assertIn(app, s["teardown"][0])
            self.assertIn(gate.HOME_ID, s["teardown"][-2])
        self.assertEqual(key_names(surface("transitions-netflix")["close"].split()), ["HOME"])
        self.assertEqual(key_names(surface("transitions-settings")["close"].split()), ["BACK"])

    def test_the_owners_motion_switches_are_read_from_the_theme_get_answer(self):
        rig = gate.Rig.__new__(gate.Rig)
        for theme, want in (({"wallpaperMotion": {"enabled": False}}, (False, None)), ({"wallpaperMotion": {"enabled": True}, "weather": {"animated": False}}, (True, False)),
                            ({}, (None, None))):
            rig.sh = lambda cmd, timeout=120, theme=theme: (0, json.dumps({"ok": True, "theme": theme}), "")
            state = rig.motion_state()
            self.assertEqual((state["wallpaperMotion.enabled"], state["weather.animated"]), want)
        rig.sh = lambda cmd, timeout=120: (1, "", "no tool")
        self.assertEqual(set(rig.motion_state().values()), {None})

    def test_a_motion_change_is_one_theme_set_with_the_paths_nested(self):
        rig = gate.Rig.__new__(gate.Rig)
        sent = []
        rig.sh = lambda cmd, timeout=120: (sent.append(cmd), (0, "{}", ""))[1]
        with mock.patch.object(gate.time, "sleep"):
            rig.set_motion({"wallpaperMotion.enabled": False, "weather.animated": False})
        self.assertEqual(len(sent), 1)
        self.assertIn("theme set", sent[0])
        self.assertEqual(json.loads(sent[0].split("theme set ", 1)[1].strip("'")), {"wallpaperMotion": {"enabled": False}, "weather": {"animated": False}})


# ------------------------------------------------------------------------------------------------------------ the host

KEYS = {"UP": "Qt.Key_Up", "DOWN": "Qt.Key_Down", "LEFT": "Qt.Key_Left", "RIGHT": "Qt.Key_Right", "OK": "Qt.Key_Return", "BACK": "Qt.Key_Back",
        # the number keys by evdev code (KEY_1 = 2 ... KEY_9 = 10, KEY_0 = 11), as kb.py takes them
        **{str(code): "Qt.Key_%d" % (code - 1 if code < 11 else 0) for code in range(2, 12)}}
STATE = ("JSON.stringify({fl: root.focusLayer, si: root.statusFocusIndex, so: root.settingsOpen, po: root.popoverOpen, wf: root.widgetFocusId,"
         " dr: root.dockRow, dc: root.dockCol, lo: root.layerOpen, ed: root.editing})")
# the calls of the luna stand-in that are not reads: a launch, a close, a write of a setting or of the dock's order, a reset or a shutdown
CALLS = ("JSON.stringify(root.lunaCalls.filter(function(c) { return /^\\/(set[A-Z]|launch|close|move|remove|delete|reset|reboot|turn|create|install|uninstall|shutdown)/.test(c.method) })"
         ".map(function(c) { return c.method + ' ' + (c.payload || '').slice(0, 60) }))")
READY = 6000
STEP = 150


def repeats_of(hold_ms):
    """The auto-repeats kb.py sends for a held key: it sleeps 100 ms and tests `0.3 < elapsed < hold`, each pass a little over its 100 ms, so
    the first repeat is at 300 ms and then there is one every 100 ms."""
    return sum(1 for i in range(1, 1000) if 0.3 < i * 0.1 + 0.0005 < hold_ms / 1000.0)


def events(token):
    """A key token as the events kb.py sends: [(key, pressed, repeat)]; [] for a marker, a sleep or a luna call."""
    if token[0] in "@!" or re.fullmatch(r"s[\d.]+", token):
        return []
    if token.endswith("+"):
        return [(token[:-1], True, False)]
    if token.endswith("-"):
        return [(token[:-1], False, False)]
    if "x" in token:
        name, rest = token.split("x", 1)
        return [(name, True, False), (name, False, False)] * int(rest.partition("@")[0])
    if ":" in token:
        name, ms = token.split(":")
        return [(name, True, False)] + [(name, True, True)] * repeats_of(int(ms)) + [(name, False, False)]
    return [(token, True, False), (token, False, False)]


def home_js(token):
    """The token as calls of Home's key handler (the key filter's `key(code, pressed, autoRepeat, device)`). The gear is not a key of Home's under LumaTV's
    compositor: its tap is the compositor's, which asks the shell for the quick panel (Shell.show, Bus.panel); over Home that is HomePanels.open."""
    if token == "MENU":
        return "root.openLayer('quick');"
    return "".join("root.key(%s,%s,%s,0);" % (KEYS[k], str(p).lower(), str(r).lower()) for k, p, r in events(token))


def settle(token):
    return re.fullmatch(r"s[\d.]+", token) is not None


class Home(HostCase):
    """LumaHome through the offline host, the way a remote presses it."""

    def replay(self, tokens):
        """Runs the tokens, one eval each; the state after each token, in order, and the luna calls that launch or write, at the end."""
        steps = []
        t = READY
        for tok in tokens:
            steps += ["--eval", "%d:(function(){%s return %s})()" % (t, home_js(tok), STATE)]
            t += 350 if settle(tok) else STEP
        steps += ["--eval", "%d:%s" % (t, CALLS)]
        rc, out = self.host(*steps, "--wait", str(t + 600))
        self.assertEqual(rc, 0, out[-1500:])
        vals = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m and m.group(1)[:1] in "'\"":
                vals.append(ast.literal_eval(m.group(1)))          # the host prints a Python repr
        self.assertEqual(len(vals), len(tokens) + 1, out[-2500:])
        return [json.loads(v) for v in vals[:-1]], json.loads(vals[-1])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class HomeKeys(Home):
    def after(self, tokens, states, token, nth=0):
        """The state after the nth occurrence of `token`."""
        return states[[i for i, t in enumerate(tokens) if t == token][nth]]

    @staticmethod
    def fixture_points():
        """The launch points the offline host's make-believe TV (a capture of the owner's) lists, in order."""
        with open(os.path.join(ROOT, "tests", "qmlhost", "stubs", "WebOSServices", "FakeApps.js")) as f:
            text = f.read()
        rows = re.findall(r'^    \["([^"]+)", "[^"]*", "[^"]*", \{([^}]*)\}\]', text, re.M)
        return [{"id": i, "hidden": "hidden: true" in o} for i, o in rows] + [{"id": "com.webos.app.hdmi2"}]

    def resolved(self, name):
        """The surface with its tile's number key worked out from the host's launch points, as the gate does on the set."""
        rig = Mock()
        rig.sh.return_value = (0, json.dumps({"launchPoints": self.fixture_points()}), "")
        return gate.resolve_tile(rig, surface(name))[0]

    def test_home_top_finds_the_status_bars_first_item_from_anywhere_on_home(self):
        tokens, marks = [], []
        for start in ("RIGHTx4@0.1 DOWN", "UPx2@0.1 RIGHTx2@0.1", "DOWNx3@0.1 LEFTx2@0.1", "DOWNx2@0.1"):
            tokens += start.split() + gate.HOME_TOP.split()
            marks.append(len(tokens) - 1)
        states, calls = self.replay(tokens)
        for i in marks:
            self.assertEqual((states[i]["fl"], states[i]["si"]), ("statusbar", 0), (i, states[i]))
        self.assertEqual(calls, [])

    def test_settings_opens_from_the_quick_panels_last_button_walks_and_closes_again_twice(self):
        s = surface("settings-scroll")
        tokens = gate.cycle_tokens(s) * 2
        states, calls = self.replay(tokens)
        for n in (0, 1):
            panel = self.after(tokens, states, "s0.2", n)                  # the end of prep
            self.assertEqual((panel["lo"], panel["so"]), ("quick", False), (n, panel))
            opened = self.after(tokens, states, "OK-", n)
            self.assertEqual((opened["so"], opened["lo"]), (True, ""), "the release of OK on the button opens Settings and the panel goes: the marker is on its edge")
            end = self.after(tokens, states, "@settings-scroll-close", n)
            self.assertEqual((end["so"], end["po"], end["lo"], end["ed"]), (False, False, "", False), "the walk ends on Home")
        self.assertEqual(calls, [], "no app launched, no setting written")

    def test_the_library_opens_from_the_apps_tiles_number_and_closes_with_nothing_launched_twice(self):
        s = self.resolved("apps-grid")
        self.assertEqual(s["open"].split()[1], "2")                          # the Apps tile is the first: KEY_1
        tokens = gate.cycle_tokens(s) * 2
        states, calls = self.replay(tokens)
        for n in (0, 1):
            opened = self.after(tokens, states, "2", n)
            self.assertEqual((opened["lo"], opened["fl"]), ("apps", "dock"), (n, opened))
            end = self.after(tokens, states, "@apps-grid-close", n)
            self.assertEqual((end["lo"], end["fl"], end["dr"], end["dc"]), ("", "dock", 0, 0), "Back from the library: the dock's first tile, which has the focus")
        self.assertEqual(calls, [])

    def test_the_library_opens_wherever_the_focus_is_and_whichever_place_the_apps_tile_has(self):
        # the number key is the dock's, whatever Home's focus: after the status bar and after a card, and with the Apps tile moved to the fourth place
        points = self.fixture_points()
        moved = [points[1], points[2], points[3], points[0]] + points[4:]
        rig = Mock()
        rig.sh.return_value = (0, json.dumps({"launchPoints": moved}), "")
        number = gate.resolve_tile(rig, surface("apps-grid"))[0]["open"].split()[1]
        self.assertEqual(number, "5")                                          # KEY_4
        tokens = ["UPx8@0.1", number, "BACK", "DOWN", "DOWN", "RIGHT", number, "BACK"]
        # the host's own list has the Apps tile first, so it is the first tile's key that opens it there: this walk is about the focus
        tokens = [t if t != number else "2" for t in tokens]
        states, calls = self.replay(tokens)
        self.assertEqual([st["lo"] for i, st in enumerate(states) if tokens[i] == "2"], ["apps", "apps"])
        self.assertEqual(calls, [])

    def test_the_home_walk_visits_every_row_and_opens_nothing_twice(self):
        s = surface("home-widgets")
        tokens = gate.cycle_tokens(s) * 2
        states, calls = self.replay(tokens)
        first = [st for tok, st in zip(tokens, states)][:len(tokens) // 2]
        self.assertEqual({st["fl"] for st in first}, {"statusbar", "widgets", "dock"})
        self.assertEqual([st["si"] for st in first if st["fl"] == "statusbar"][:3], [0, 0, 0])
        self.assertEqual(max(st["si"] for st in first), 3, "all four items of the status bar")
        self.assertEqual({st["wf"] for st in first if st["fl"] == "widgets"}, {"clock", "home", "news", "weather"}, "every card")
        self.assertEqual({st["dr"] for st in first if st["fl"] == "dock"}, {0, 1}, "the dock's rows")
        self.assertTrue(all(not st["so"] and not st["po"] and st["lo"] == "" and not st["ed"] for st in states))
        self.assertEqual((states[len(tokens) // 2 - 1]["fl"], states[-1]["fl"]), ("statusbar", "statusbar"), "each cycle ends where it can start again")
        self.assertEqual(calls, [])

    def test_a_tile_found_by_its_number_is_the_one_that_opens(self):
        points = self.fixture_points()
        tokens, apps = [], []
        for app in ("netflix", "com.webos.app.browser", "com.lighting.huesync"):      # the second tile, the ninth, the tenth (the 0 key)
            code, number = gate.tile_digit(points, app)
            tokens += [str(code), "UPx8@0.1"]
            apps.append((app, number))
        self.assertEqual(apps, [("netflix", 2), ("com.webos.app.browser", 9), ("com.lighting.huesync", 10)])
        states, calls = self.replay(tokens)
        ids = [re.search(r'"id":\s*"([^"]+)"', c).group(1) for c in calls if c.startswith("/launch ")]
        self.assertEqual(ids, [a for a, _ in apps], calls)
        self.assertEqual([states[i]["dc"] for i in (0, 2, 4)], [1, 8, 9], "the dock's cursor is on the tile that opened")
        self.assertFalse([c for c in calls if "setSystemSettings" in c or "moveLaunchPoint" in c])

    def test_lgs_settings_is_the_last_row_of_lumatvs_settings_rail_reached_through_the_quick_panel(self):
        s = surface("transitions-settings")
        tokens = gate.cycle_tokens(s) * 2
        states, calls = self.replay(tokens)
        launched = [c for c in calls if c.startswith("/launch ")]
        self.assertEqual(len(launched), 2, calls)
        self.assertTrue(all('"com.palm.app.settings"' in c for c in launched), launched)
        for n in (0, 1):
            before = self.after(tokens, states, "@transitions-settings-open", n)
            self.assertEqual((before["so"], before["lo"]), (True, ""), "Settings is up with its rail on the last row when OK is pressed")
            after = self.after(tokens, states, "OK-", n)
            self.assertEqual((after["so"], after["lo"]), (False, ""), "and the launch closes it: Home is under the app that comes")
        self.assertFalse([c for c in calls if "setSystemSettings" in c])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class ScreenKeys(HostCase):
    """The screens' own handlers, standalone, for what Home's state does not show."""

    def run_screen(self, surface_file, init, tokens, state_js, dir_of, wait_extra=400):
        steps = [(5000, init)]
        t = 5300
        for tok in tokens:
            js = "".join(dir_of(k, p, r) for k, p, r in events(tok))
            steps.append((t, "(function(){%s return %s})()" % (js, state_js)))
            t += 200
        args = []
        for ms, e in steps:
            args += ["--eval", "%d:%s" % (ms, e)]
        rc, out = self.host(*args, "--wait", str(t + wait_extra), surface=surface_file)
        self.assertEqual(rc, 0, out[-1500:])
        vals = [json.loads(m.group(1).replace("\\'", "'")) for m in re.finditer(r"\[eval\] \('(\{.*\})', (?:True|False)\)$", out, re.M)]
        self.assertEqual(len(vals), len(tokens), out[-2000:])
        return vals, [ln.split("[sig] ", 1)[1] for ln in out.splitlines() if "[sig] " in ln]

    @staticmethod
    def press_dir(k, pressed, repeat):
        return 'root.key("%s");' % k.lower() if pressed else ""

    def test_settings_moves_its_rail_and_pages_and_changes_nothing(self):
        s = surface("settings-scroll")
        toks = gate.cycle_tokens(s)
        walk = toks[toks.index("@settings-scroll-open") + 2:toks.index("@settings-scroll-close")]
        init = ("(function(){root.closeRequested.connect(function(){console.log('[sig] close')});root.themePatch.connect(function(p){console.log('[sig] patch')});"
                "root.themePersist.connect(function(p){console.log('[sig] persist')});root.launchApp.connect(function(a){console.log('[sig] launch ' + a)});return 1})()")
        states, signals = self.run_screen("Settings.qml", init, walk, "JSON.stringify({z: root.zone, r: root.railIndex, c: root.current})", self.press_dir)

        def at(token, nth=0):
            return states[[i for i, t in enumerate(walk) if t == token][nth]]
        self.assertEqual(at("RIGHT", 0), {"z": "page", "r": 0, "c": "picture"})
        self.assertEqual(at("BACK", 0), {"z": "rail", "r": 0, "c": "picture"}, "Back leaves a page for the rail; Left would have moved a slider")
        self.assertEqual(at("DOWNx8@0.06"), {"z": "rail", "r": 8, "c": "system"}, "eight sections down is System, each switching the page beside it")
        self.assertEqual(at("RIGHT", 1), {"z": "page", "r": 8, "c": "system"})
        self.assertEqual(at("UPx9@0.06"), {"z": "rail", "r": 0, "c": "picture"}, "and back on the section the next open starts from")
        self.assertEqual(signals, ["close"], "Back on the rail closes it, and nothing was patched, written or launched")

    def test_the_library_moves_across_down_and_into_its_rail_and_launches_nothing(self):
        s = surface("apps-grid")
        toks = gate.cycle_tokens(s)
        walk = toks[toks.index("@apps-grid-open") + 2:toks.index("@apps-grid-close")]
        init = ("(function(){root.launched.connect(function(a){console.log('[sig] launched ' + a.id)});root.closeRequested.connect(function(){console.log('[sig] close')});"
                "root.menuRequested.connect(function(){console.log('[sig] menu')});root.searchRequested.connect(function(){console.log('[sig] search')});return 1})()")
        state = "JSON.stringify({c: root.cellIndex, g: root.inGrid, s: root.searchFocused, r: root.railIndex, n: root.list.length, row: root.scrollRow})"
        states, signals = self.run_screen("panels/AppsGrid.qml", init, walk, state, self.press_dir)

        def at(token, nth=0):
            return states[[i for i, t in enumerate(walk) if t == token][nth]]
        self.assertGreaterEqual(at("RIGHTx4@0.12")["n"], 15, "the host's TV lists 23 apps: five rows")
        self.assertEqual(at("RIGHTx4@0.12")["c"], 4)
        down = at("DOWN:600")
        self.assertTrue(down["g"] and not down["s"] and down["row"] > 0, "the held key walks down the grid and scrolls it")
        up = at("UP:600")
        self.assertTrue(up["g"] and not up["s"] and up["row"] == 0, "and back to the first row without reaching the search button")
        out = at("LEFTx5@0.12")
        self.assertEqual((out["g"], out["s"], out["r"]), (False, False, 0), "four moves along, the fifth leaves the grid for the rail")
        self.assertEqual(at("DOWNx3@0.12")["r"], 3, "the rail's sections")
        self.assertEqual(at("UPx3@0.12")["r"], 0)
        self.assertTrue(at("RIGHT")["g"])
        self.assertEqual(signals, ["close"], "no app launched, no menu, no search")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class SwitcherKeys(unittest.TestCase):
    def test_the_walk_moves_between_the_cards_quits_nothing_and_closes(self):
        helper = switcher_host.Switcher("test_reduced_motion_is_said_to_the_compositor")
        apps = [{"id": "netflix", "title": "Netflix", "picture": "video", "state": ""},
                {"id": "com.webos.app.discovery", "title": "LG Content Store", "picture": "live", "state": ""},
                {"id": "com.webos.app.browser", "title": "Web Browser", "picture": "live", "state": ""}]
        walk = walk_of(surface("switcher"))
        steps = [(700, switcher_host.feed(apps, True), "opened")]
        at = 1000
        labels = []
        for i, tok in enumerate(walk):
            for k, pressed, repeat in events(tok):
                if pressed:
                    steps.append((at, "overlay.key(%s, true, %s, 0)" % (KEYS[k], str(repeat).lower()), "k%d" % len(steps)))
                    at += 40
            if events(tok):
                steps.append((at, switcher_host.STATE, "after%d" % i))
                labels.append((i, tok))
                at += 150
        got, out = helper.run_overlay(steps, at + 400)
        self.assertEqual(got["after%d" % labels[0][0]].split()[0:2], ["true", "true"], "the walk starts with the switcher up and the keys in it")
        by = {tok: got["after%d" % i] for i, tok in labels}
        self.assertTrue(by["RIGHTx2@0.2"].startswith("true true card/2/"), "two cards along: %s" % by)
        self.assertTrue(by["LEFTx2@0.2"].startswith("true true card/0/"), "left and right come back to the first card: %s" % by)
        self.assertTrue(by["LEFT:700"].startswith("true true card/0/"), by)
        ops = [o["op"] for o in helper.posts(out)]
        self.assertEqual(ops[0], "layout")
        self.assertNotIn("quit", ops)
        self.assertNotIn("quitall", ops)
        self.assertNotIn("activate", ops)
        self.assertEqual(ops[-1], "close", "Back asks the compositor to close it (the overlay stays up until the compositor's feed says it is closed)")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class KeyboardKeys(unittest.TestCase):
    def test_the_walk_across_the_keys_types_nothing_and_leaves_the_keyboard_up(self):
        helper = keyboard_host.OwnKeyboard("test_hidden_the_keyboard_runs_nothing_and_the_system_layer_draws_no_frame")
        s = keyboard_host.Script(keyboard_host.READY + 1000, gap=120)
        for tok in walk_of(surface("keyboard-luma")):
            if settle(tok):
                s.at += int(float(tok[1:]) * 1000)
                continue
            for k, pressed, repeat in events(tok):
                if pressed:
                    s.args += keyboard_host.push(s.at, "imekey", {"k": k.lower(), "d": True, "r": repeat}) + \
                        keyboard_host.push(s.at + 40, "imekey", {"k": k.lower(), "d": False, "r": False})
                    s.at += 100 if repeat else 120
            s.read(tok)
        end = s.at + 600
        rc, out = helper.host("--push", "%d:ime:%s" % (keyboard_host.READY, json.dumps(keyboard_host.FIELD)), *s.args, wait=end)
        helper.clean(rc, out)
        posts = helper.lines(out, "ime")
        self.assertEqual([p["op"] for p in posts if p["op"] != "shown"], [], "no letter typed, nothing entered, the keyboard not put away")
        got = helper.evals(out)
        self.assertTrue(all(k["up"] for k in got.values()), "the keyboard is up after every step")
        self.assertGreaterEqual(len({json.dumps(k.get("key")) for k in got.values()}), 2, "the focus moved over the keys")


if __name__ == "__main__":
    unittest.main()
