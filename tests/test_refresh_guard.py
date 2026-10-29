"""Run with: python3 -m unittest tests.test_refresh_guard

compositor/tools/refresh_guard.py: the test tooling does not wake the set while LG's panel refresh (off-RS) may be running, and
tv_gate.wake() uses it. The set is faked: its luna replies are the shapes the helper's refreshguard.rs also parses.
"""
import importlib.util
import json
import os
import sys
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(os.path.dirname(HERE), "compositor", "tools")


def load(name, extra=()):
    mocks = {"tv": mock.MagicMock(), "paramiko": mock.MagicMock()}
    mocks.update({m: mock.MagicMock() for m in extra})
    with mock.patch.dict(sys.modules, mocks), mock.patch.object(sys, "path", [TOOLS] + sys.path):
        spec = importlib.util.spec_from_file_location(name + "_rg_under_test", os.path.join(TOOLS, name + ".py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[name + "_rg_under_test"] = mod
        spec.loader.exec_module(mod)
    return mod


rg = load("refresh_guard")
MIN = 60


def refresher(count=12, last=1700, interval=4, nested=True):
    body = {"offrsCount": count, "offrsLastTime": last, "offrsInterval": interval}
    return json.dumps({"returnValue": True, **({"pixelRefresherInfoList": [body]} if nested else body)})


def usage(ticks):
    return json.dumps({"returnValue": True, "panelUsageTime": ticks})


class Parse(unittest.TestCase):
    def test_nested_flat_text_numbers_and_a_pty_s_carriage_returns(self):
        i = rg.parse_info(refresher(), usage(10230))
        self.assertEqual((i["count"], i["last"], i["interval"], i["usage"]), (12, 1700, 4, 1705.0))
        self.assertTrue(rg.due(i))
        text = json.dumps({"returnValue": True, "offrsCount": "3", "offrsLastTime": "20", "offrsInterval": "4"}, indent=2)
        j = rg.parse_info("\r\n" + text.replace("\n", "\r\n"), usage("138"))
        self.assertEqual((j["count"], j["usage"]), (3, 23.0))
        self.assertFalse(rg.due(j), "23 h is under 20 + 4")

    def test_a_wrong_reply_says_which_call(self):
        bad = [(json.dumps({"returnValue": False, "errorText": "Denied method call"}), usage(1), "getPixelRefresherInfoList: Denied method call"),
               (json.dumps({"returnValue": True}), usage(1), "no offrs values"),
               (json.dumps({"offrsCount": 1, "offrsLastTime": 2}), usage(1), "no offrsInterval"),
               (refresher(), json.dumps({"returnValue": True}), "no panelUsageTime"),
               (refresher(), "no JSON here", "no JSON reply"),
               ("", usage(1), "no JSON reply")]
        for r, u, want in bad:
            with self.subTest(want):
                with self.assertRaises(ValueError) as e:
                    rg.parse_info(r, u)
                self.assertIn(want, str(e.exception))


class Payload(unittest.TestCase):
    def test_the_usage_call_carries_subscribe_false_and_an_empty_payload_is_what_the_tv_refuses(self):
        fake = Fake([(5, 100, 4, 104)])
        info = rg.read_info(fake.run)
        self.assertEqual(info["usage"], 104.0)
        usage_cmds = [c for c in fake.cmds if "getPanelUsageTime" in c]
        self.assertEqual(len(usage_cmds), 1)
        self.assertIn("'{\"subscribe\":false}'", usage_cmds[0])
        with self.assertRaises(ValueError) as e:
            rg.read_info(lambda cmd, timeout=20: fake.run(cmd.replace(rg.USAGE_PAYLOAD, "{}"), timeout))
        self.assertIn("Failed to validate against schema", str(e.exception))


class Rule(unittest.TestCase):
    def info(self, count=5, last=100.0, usage=104.0):
        return {"count": count, "last": last, "interval": 4.0, "usage": usage}

    def test_the_window_is_the_first_15_minutes_of_a_standby_that_began_due(self):
        i = self.info()
        self.assertEqual(rg.verdict(i, 0), 15 * MIN)
        self.assertEqual(rg.verdict(i, 3 * MIN + 1), 12 * MIN - 1)
        self.assertEqual(rg.verdict(i, 15 * MIN - 1), 1)
        self.assertEqual(rg.verdict(i, 15 * MIN), 0)

    def test_nothing_to_wait_for_when_not_due_finished_or_started_not_due(self):
        self.assertEqual(rg.verdict(self.info(usage=103.9), 60), 0)
        self.assertEqual(rg.verdict(self.info(count=6), 60, due_at_start=True, count_at_start=5), 0, "the run finished")
        self.assertEqual(rg.verdict(self.info(), 60, due_at_start=False), 0)
        self.assertEqual(rg.verdict(self.info(usage=103.0), 60, due_at_start=True), 14 * MIN, "the start's word beats the reading now")


class Fake:
    """The set's shell: answers luna-send, the helper's topic and the clock; `infos` is (count, last, interval, panel hours) for the next reads (the last repeats)."""

    def __init__(self, infos, standby=None, tv_now=1_000_000):
        self.infos, self.standby, self.tv_now = list(infos), standby, tv_now
        self.reads = 0
        self.cmds = []

    def run(self, cmd, timeout=20):
        self.cmds.append(cmd)
        if "getPixelRefresherInfoList" in cmd:
            self.reads += 1
            c, l, i, _ = self.infos[min(self.reads - 1, len(self.infos) - 1)]
            return refresher(c, l, i)
        if "getPanelUsageTime" in cmd:
            if "'%s'" % rg.USAGE_PAYLOAD not in cmd:    # the TV validates the payload: {} is refused
                return json.dumps({"returnValue": False, "errorCode": 3, "errorText": "Failed to validate against schema"})
            _, _, _, u = self.infos[min(self.reads - 1, len(self.infos) - 1)]
            return usage(round(u * 6))
        if "18731/v1/state/panel" in cmd:
            if self.standby is None:
                raise RuntimeError("curl: (7) connection refused")
            return json.dumps({"ok": True, "topic": "panel", "value": {"standby": self.standby}}) + "\n%d\n" % self.tv_now
        raise AssertionError(cmd)


class Clock:
    def __init__(self):
        self.t = 5000.0
        self.slept = []

    def now(self):
        return self.t

    def sleep(self, s):
        self.slept.append(s)
        self.t += s


class Wait(unittest.TestCase):
    def go(self, fake, **kw):
        clock, said = Clock(), []
        waited = rg.wait_out_refresh(fake.run, now=clock.now, sleep=clock.sleep, say=said.append, **kw)
        return waited, clock, said

    def test_a_due_run_holds_the_wake_until_15_minutes_of_standby_and_says_why(self):
        fake = Fake([(5, 100, 4, 104)])
        clock = Clock()
        said = []
        waited = rg.wait_out_refresh(fake.run, standby_since=clock.t - 3 * MIN, now=clock.now, sleep=clock.sleep, say=said.append)
        self.assertTrue(waited)
        self.assertAlmostEqual(sum(clock.slept), 12 * MIN, delta=1)
        self.assertTrue(all(s <= rg.POLL_S for s in clock.slept))
        self.assertIn("panel refresh is due", said[0])
        self.assertIn("12 min to go", said[0])
        self.assertIn("--force-wake", said[0])
        self.assertIn("15 minutes of standby have passed", said[-1])

    def test_nothing_waited_when_the_run_is_not_due_or_the_standby_is_long(self):
        waited, clock, said = self.go(Fake([(5, 100, 4, 104)]), standby_since=5000 - 16 * MIN)
        self.assertEqual((waited, clock.slept, said), (False, [], []), "16 minutes of standby")
        waited, clock, said = self.go(Fake([(5, 100, 4, 103)]), standby_since=5000 - 60)
        self.assertEqual((waited, clock.slept, said), (False, [], []), "the panel is at 103 h, a run needs 104")

    def test_a_run_that_finishes_ends_the_wait_early(self):
        fake = Fake([(5, 100, 4, 104), (5, 100, 4, 104), (6, 104, 4, 104)])
        waited, clock, said = self.go(fake, standby_since=5000 - 60)
        self.assertTrue(waited)
        self.assertLess(sum(clock.slept), 4 * MIN)
        self.assertIn("panel refresh finished (run 6)", said[-1])

    def test_force_wake_reads_nothing(self):
        fake = Fake([(5, 100, 4, 104)])
        waited, clock, said = self.go(fake, force=True, standby_since=5000)
        self.assertEqual((waited, clock.slept, fake.cmds), (False, [], []))
        self.assertIn("--force-wake", said[0])

    def test_an_unreadable_state_wakes_and_says_so(self):
        class Dead(Fake):
            def run(self, cmd, timeout=20):
                raise OSError("connection reset")
        waited, clock, said = self.go(Dead([]), standby_since=5000)
        self.assertEqual((waited, clock.slept), (False, []))
        self.assertIn("could not be read", said[0])
        self.assertIn("connection reset", said[0])

    def test_the_helpers_standby_is_used_when_the_caller_does_not_know_it(self):
        since = 1_000_000 - 5 * MIN
        fake = Fake([(5, 100, 4, 104)], standby={"since": since * 1000, "dueAtStart": True, "countAtStart": 5})
        waited, clock, said = self.go(fake)
        self.assertTrue(waited)
        self.assertAlmostEqual(sum(clock.slept), 10 * MIN, delta=1)
        self.assertIn("from the helper", said[0])
        fake = Fake([(5, 100, 4, 104)], standby={"since": since * 1000, "dueAtStart": False, "countAtStart": 5})
        self.assertEqual(self.go(fake)[0], False, "the helper saw that no run was due when the standby began")

    def test_without_the_caller_or_the_helper_the_standby_is_taken_as_beginning_now(self):
        fake = Fake([(5, 100, 4, 104)])
        waited, clock, said = self.go(fake)
        self.assertTrue(waited)
        self.assertAlmostEqual(sum(clock.slept), 15 * MIN, delta=1)
        self.assertIn("not known", said[0])


class Wake(unittest.TestCase):
    """tv_gate.wake(): the guard runs for a set in standby, never for one that is lit, and --force-wake reaches it."""

    def setUp(self):
        self.gate = load("tv_gate")
        g = self.gate
        self.calls = []
        g.wait_ssh = lambda s: True
        g.pty = lambda cmd, timeout=600: self.calls.append(("pty", cmd)) or ""
        g.time = mock.MagicMock()
        g.refresh_guard = mock.MagicMock(LIT=("active", "screen saver"), wait_out_refresh=lambda *a, **k: self.calls.append(("guard", k)))

    def wake(self, state, **kw):
        self.gate.power_state = lambda: state
        with mock.patch("builtins.print"):
            return self.gate.wake(**kw)

    def guard_calls(self):
        return [c[1] for c in self.calls if c[0] == "guard"]

    def test_a_set_in_standby_is_checked_before_powerOn(self):
        self.assertTrue(self.wake("active standby", standby_since=123.0))
        kinds = [c[0] for c in self.calls]
        self.assertEqual(kinds[0], "guard")
        self.assertIn("powerOn", self.calls[1][1])
        self.assertEqual(self.guard_calls(), [{"standby_since": 123.0, "force": False}])

    def test_a_lit_set_is_not_checked(self):
        for state in ("active", "screen saver"):
            self.calls.clear()
            self.wake(state)
            self.assertEqual(self.guard_calls(), [], state)

    def test_force_wake_is_passed_on(self):
        self.gate.FORCE_WAKE = True
        self.wake("suspend")
        self.assertEqual(self.guard_calls(), [{"standby_since": None, "force": True}])


if __name__ == "__main__":
    unittest.main()
