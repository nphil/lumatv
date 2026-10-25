"""Run with: python3 -m unittest tests.test_ambient_slot

The light-sync probe slot (ambient/capture/tv_probe_slot.py) without a TV. What matters about it is that LG's developer
flag exists only while a capture runs and is gone however the slot ends. The script that runs on the set is executed here
for real (sh, against a stand-in luna-send and the host build of the probe on its fake library), and the orchestrating
side is driven against a small stand-in for the set: a slot that finds the flag already there does not start, a failure in
the middle still ends with the flag checked, the receiver put back and the set in standby.
"""
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
CAPTURE = os.path.join(REPO, "ambient", "capture")
HOST_PROBE = os.path.join(CAPTURE, "build", "host", "ambient-probe")

spec = importlib.util.spec_from_file_location("tv_probe_slot", os.path.join(CAPTURE, "tv_probe_slot.py"))
slot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(slot)


class Resp:
    """What urlopen returns, as the context manager the slot uses."""

    def __init__(self, payload):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def read(self):
        return json.dumps(self.payload).encode()


def load(path):
    with open(path) as f:
        return f.read()


def write(path, text, mode=0o755):
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


class Plan(unittest.TestCase):
    def test_everything_that_puts_things_back_comes_after_the_captures(self):
        steps = slot.plan(["home", "hdmi2", "netflix"], 60)
        names = [s[0] for s in steps]
        last_scenario = max(i for i, n in enumerate(names) if n.startswith("scenario:"))
        self.assertLess(names.index("preflight"), names.index("mute"))
        self.assertLess(names.index("mute"), names.index("start"))
        self.assertLess(names.index("start"), names.index("scenario:home"))
        for n in ("flag-check", "unmute", "fetch", "home", "standby"):
            self.assertGreater(names.index(n), last_scenario, n)
            self.assertTrue(steps[names.index(n)][2], n + " runs even after a failure")
        self.assertFalse(any(s[2] for s in steps[: last_scenario + 1]))

    def test_no_mute_and_no_standby_drop_their_steps(self):
        names = [s[0] for s in slot.plan(["home"], 30, mute=False, standby=False)]
        self.assertNotIn("mute", names)
        self.assertNotIn("unmute", names)
        self.assertNotIn("standby", names)

    def test_plan_mode_touches_nothing(self):
        with mock.patch.object(slot, "run") as run, mock.patch("builtins.print"):
            self.assertEqual(slot.main(["--plan"]), 0)
            self.assertEqual(slot.main([]), 0)
        run.assert_not_called()


@unittest.skipUnless(shutil.which("sh"), "needs sh")
class OnTheSet(unittest.TestCase):
    """The generated script, run on this machine with the flag and the working directory in a temp directory."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="slot-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.flag = os.path.join(self.tmp, "vt_devmode")
        self.r = os.path.join(self.tmp, "r")
        os.mkdir(self.r)
        self.bin = os.path.join(self.tmp, "bin")
        os.mkdir(self.bin)
        write(os.path.join(self.bin, "luna-send"), "#!/bin/sh\necho \"$@\" >> %s/luna.log\necho '{\"appId\":\"stand-in\"}'\n" % self.r)
        patches = [mock.patch.object(slot, "FLAG", self.flag), mock.patch.object(slot, "DEADMAN_MARGIN", 5)]
        fast = {"home": {"launch": {"id": "com.webos.app.home"}, "settle": 0, "what": ""},
                "hdmi2": {"launch": {"id": "com.webos.app.hdmi2"}, "settle": 0, "what": ""},
                "netflix": {"launch": {"id": "netflix"}, "settle": 0, "key": "ENTER", "after_key": 0, "what": ""}}
        patches.append(mock.patch.dict(slot.SCENARIOS, fast, clear=True))
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def probe(self, body):
        write(os.path.join(self.r, "ambient-probe"), "#!/bin/sh\nprev=; for a in \"$@\"; do [ \"$prev\" = -L ] && L=$a; prev=$a; done\n"
              "echo $$ > %s/probe.pid\nif [ -e %s ]; then echo present > \"%s/during-$L\"; fi\n%s\n" % (self.r, self.flag, self.r, body))

    def start(self, scenarios, secs):
        script = os.path.join(self.tmp, "slot.sh")
        write(script, slot.remote_script(self.r, scenarios, secs))
        env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"])
        return subprocess.Popen(["sh", script], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)

    def wait_for(self, path, seconds=10):
        end = time.time() + seconds
        while time.time() < end:
            if os.path.exists(path):
                return True
            time.sleep(0.05)
        return False

    def events(self):
        path = os.path.join(self.r, "flag-events.txt")
        return slot.parse_events(load(path)) if os.path.exists(path) else []

    @unittest.skipUnless(os.path.exists(HOST_PROBE), "needs ambient/capture/build.sh (host build)")
    def test_the_flag_exists_only_while_a_capture_runs(self):
        self.probe('LD_LIBRARY_PATH=%s exec %s "$@"' % (os.path.dirname(HOST_PROBE), HOST_PROBE))
        write(os.path.join(self.r, "netflix-key-sent.txt"), "")          # the button the caller would press
        p = self.start(["home", "hdmi2", "netflix"], 1)
        self.assertEqual(p.wait(timeout=60), 0)
        for name in ("home", "hdmi2", "netflix"):
            self.assertTrue(os.path.exists(os.path.join(self.r, "during-" + name)), name + ": the flag was there when the probe started")
            report = slot.parse_report(load(os.path.join(self.r, name + ".txt")))
            self.assertEqual(report["exit"], 0)
            self.assertGreater(report["ok"], 0)
        self.assertFalse(os.path.exists(self.flag))
        windows = self.events()
        self.assertEqual([w["scenario"] for w in windows], ["home", "hdmi2", "netflix"])
        self.assertEqual({w["removedBy"] for w in windows}, {"end"})
        for a, b in zip(windows, windows[1:]):
            self.assertLess(a["removed"], b["created"], "one window is over before the next opens")
        self.assertTrue(os.path.exists(os.path.join(self.r, "done.txt")))
        # the launch and the foreground query went through luna-send, once each per scenario
        self.assertEqual(load(os.path.join(self.r, "luna.log")).count("getForegroundAppInfo"), 3)

    def test_the_key_is_asked_for_before_the_window_opens(self):
        self.probe("exec sleep 0.2")
        p = self.start(["netflix"], 1)
        self.assertTrue(self.wait_for(os.path.join(self.r, "netflix-key.txt")), "the script asks for the button")
        time.sleep(0.5)
        self.assertFalse(os.path.exists(self.flag), "no flag while it waits for the button")
        write(os.path.join(self.r, "netflix-key-sent.txt"), "")
        self.assertEqual(p.wait(timeout=30), 0)
        self.assertEqual([w["scenario"] for w in self.events()], ["netflix"])

    def test_a_stopped_script_removes_the_flag_through_its_trap(self):
        self.probe("exec sleep 30")
        p = self.start(["home"], 30)
        self.assertTrue(self.wait_for(self.flag), "the window opens")
        # what the slot's own cleanup does: the script, then the probe it is waiting for
        os.kill(int(load(os.path.join(self.r, "slot.pid"))), signal.SIGTERM)
        os.kill(int(load(os.path.join(self.r, "probe.pid"))), signal.SIGTERM)
        p.wait(timeout=20)
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual([w["removedBy"] for w in self.events()], ["trap"])

    def test_the_dead_man_timer_removes_the_flag_of_a_script_that_was_killed_outright(self):
        self.probe("exec sleep 30")
        with mock.patch.object(slot, "DEADMAN_MARGIN", 0):
            p = self.start(["home"], 2)
        self.assertTrue(self.wait_for(self.flag), "the window opens")
        os.kill(int(load(os.path.join(self.r, "slot.pid"))), signal.SIGKILL)       # no trap runs for this
        os.kill(int(load(os.path.join(self.r, "probe.pid"))), signal.SIGKILL)
        p.wait(timeout=10)
        self.assertTrue(os.path.exists(self.flag), "nothing but the timer is left to remove it")
        end = time.time() + 8
        while os.path.exists(self.flag) and time.time() < end:
            time.sleep(0.1)
        self.assertFalse(os.path.exists(self.flag), "removed by the timer, about 2 s after it opened")
        self.assertEqual([w["removedBy"] for w in self.events()], ["deadman"])

    def test_a_second_copy_of_the_script_does_not_run(self):
        self.probe("exec sleep 0.2")
        os.mkdir(os.path.join(self.r, "run.lock"))
        p = self.start(["home"], 1)
        self.assertEqual(p.wait(timeout=10), 8)
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual(self.events(), [])


@unittest.skipUnless(os.path.exists(HOST_PROBE), "needs ambient/capture/build.sh (host build)")
class Reading(unittest.TestCase):
    def run_probe(self, **env):
        r = subprocess.run([HOST_PROBE, "-s", "1", "-r", "30", "-L", "t"], capture_output=True, text=True, timeout=30,
                           env=dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(HOST_PROBE), **env))
        return r.stdout + "exit=%d\n" % r.returncode

    def test_the_probes_own_report_is_read(self):
        report = slot.parse_report(self.run_probe())
        self.assertEqual(report["exit"], 0)
        self.assertGreater(report["calls"], 20)
        self.assertEqual(report["ok"], report["calls"])
        self.assertEqual(report["rc"], {0: report["calls"]})
        self.assertGreater(report["changes_per_s"], 10)
        self.assertEqual(set(report["latency_us"]), {"p50", "p90", "p99", "max"})
        self.assertIn("cpu_probe_pct", report)
        self.assertIn("init_ms", report)
        self.assertIn("the zones change", slot.verdict(report))

    def test_a_refusal_is_named(self):
        report = slot.parse_report(self.run_probe(FAKE_VT_DENY="1"))
        self.assertTrue(report["init_refused"])
        self.assertEqual(report["exit"], 2)
        self.assertIn("refused", slot.verdict(report))

    def test_black_video_is_not_mistaken_for_a_working_capture(self):
        text = ("summary preset 0: wall=60.00s calls=1800 ok=1800 changed=0 (0.0/s) rc: 0 x1800\n  call latency us: p50=3 p90=4 p99=6 max=9\n"
                "  cpu: probe 0.30% of one core (0.180 s in 60.00 s), system busy 20.0% over the window\nexit=0\n"
                "dev flag /var/luna/preferences/vt_devmode: present\n")
        self.assertIn("did not change", slot.verdict(slot.parse_report(text)))


class FakeSet:
    """Enough of the set and of Home Assistant for the orchestrating side: answers by what the command asks for."""

    def __init__(self, flag_before=False, fail_on_key=False):
        self.flag = flag_before
        self.flag_before = flag_before
        self.fail_on_key = fail_on_key
        self.calls = []
        self.started = False
        self.muted = False
        self.standby = False
        self.removed_remote = False
        self.ha_calls = []

    # tv.tv
    def tv(self, cmd, timeout=120):
        self.calls.append(cmd)
        if cmd.startswith("test -e") and "vt_devmode" in cmd:
            return "yes\n" if self.flag else "no\n"
        if cmd.startswith("test -e") and "done.txt" in cmd:
            return "yes\n" if self.started else "no\n"
        if cmd.startswith("test -e") and self.started and any("/%s." % n in cmd for n in ("home", "netflix")):
            return "yes\n"
        if cmd.startswith("test -e"):
            return "no\n"
        if cmd.startswith("md5sum"):
            return "35d3fec60000  /usr/lib/libvtcapture.so.1.0.0\n"
        if "/dev/video60" in cmd:
            return ""
        if "systemctl --failed" in cmd and cmd.startswith("ls "):
            return "ambient-probe\nslot.sh\n" + ("done.txt\nnetflix-key.txt\n" if self.started else "") + "---\n"
        if "systemctl --failed" in cmd:
            return ""
        if "pictureMode" in cmd:
            return '{"settings":{"pictureMode":"filmMaker"}}'
        if cmd.endswith("ambient-probe -n"):
            return "library: init=0x1 preset=0x2 get=0x3 finalize=0x4\n"
        if "slot.sh >" in cmd:
            self.started = True
            self.flag = False                          # the script ran its windows and removed the flag itself
            return ""
        if cmd.startswith("cat") and "flag-events.txt" in cmd:
            return "created home 100.5\nremoved end 160.5\n" if self.started else ""
        if "rm -f /var/luna/preferences/vt_devmode" in cmd:
            self.flag = False
            return ""
        if "power/powerOff" in cmd or "getPowerState" in cmd:
            return ""
        return ""

    def put(self, local, remote):
        self.calls.append("put " + remote)

    def get(self, remote, local):
        self.calls.append("get " + remote)
        with open(local, "w") as f:
            f.write("summary preset 0: wall=60.00s calls=1800 ok=1800 changed=900 (15.0/s) rc: 0 x1800\nexit=0\n" if remote.endswith(".txt") else "x")

    # tv_gate
    def power_state(self):
        return "active"

    def wake(self):
        return True

    def pty(self, cmd, timeout=600):
        self.calls.append("pty " + cmd)
        if "powerOff" in cmd:
            self.standby = True
        return ""

    def ha_token(self):
        return "token"

    def urlopen(self, req, timeout=0):
        url = req.full_url
        body = json.loads(req.data) if req.data else None
        self.ha_calls.append((url.rsplit("/api/", 1)[-1], body))
        if "webostv/button" in url and self.fail_on_key:
            raise OSError("HA is not answering")
        if "volume_mute" in url:
            self.muted = body["is_volume_muted"]
        return Resp({"state": "on", "attributes": {"is_volume_muted": False, "volume_level": 0.355}} if "/api/states/" in url else [])


class Slot(unittest.TestCase):
    def run_slot(self, fake, *extra):
        out = tempfile.mkdtemp(prefix="slot-out-")
        self.addCleanup(shutil.rmtree, out, True)
        modules = {"tv": mock.MagicMock(tv=fake.tv, put=fake.put, get=fake.get),
                   "tv_gate": mock.MagicMock(power_state=fake.power_state, wake=fake.wake, pty=fake.pty, ha_token=fake.ha_token)}
        argv = ["--go", "--scenarios", "home,netflix", "--secs", "5", "--out", out, *extra]
        with mock.patch.dict(sys.modules, modules), mock.patch("urllib.request.urlopen", fake.urlopen), \
                mock.patch.object(slot, "PROBE", __file__), mock.patch.object(slot.time, "sleep"), mock.patch("builtins.print"):
            try:
                rc = slot.main(argv)
            except SystemExit as e:
                rc = e.code
        return rc, out

    def test_a_slot_that_finds_the_flag_already_there_does_not_start(self):
        fake = FakeSet(flag_before=True)
        rc, out = self.run_slot(fake)
        self.assertNotEqual(rc, 0)
        self.assertTrue(fake.flag, "somebody else's flag is left as it was")
        self.assertFalse(any(c.startswith("put ") for c in fake.calls), "nothing is uploaded")
        self.assertEqual(fake.ha_calls, [], "the receiver is not touched")
        self.assertFalse(fake.standby, "the set was on and stays on")
        self.assertTrue(json.loads(load(os.path.join(out, "slot-log.json")))["flag"]["existedBefore"])

    def test_a_clean_slot_mutes_runs_checks_the_flag_and_puts_everything_back(self):
        fake = FakeSet()
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 0)
        self.assertFalse(fake.flag)
        self.assertFalse(fake.muted, "the receiver is back to unmuted")
        kinds = [c[0] for c in fake.ha_calls]
        self.assertEqual(kinds[0], "states/media_player.living_room_avr")
        self.assertIn("services/media_player/volume_mute", kinds)
        self.assertIn("services/webostv/button", kinds, "the Play button is pressed for Netflix")
        self.assertTrue(fake.standby)
        # standby is the last thing, after the results were fetched
        last_get = max(i for i, c in enumerate(fake.calls) if c.startswith("get "))
        self.assertGreater(next(i for i, c in enumerate(fake.calls) if c.startswith("pty ") and "powerOff" in c), last_get)
        log = json.loads(load(os.path.join(out, "slot-log.json")))
        self.assertTrue(log["flag"]["gone"])
        self.assertEqual(log["flag"]["windows"][0]["scenario"], "home")
        self.assertEqual(log["flag"]["createdAt"], 100.5)
        self.assertEqual(log["flag"]["removedAt"], 160.5)
        self.assertEqual(log["problems"], [])
        self.assertIn("the zones change", load(os.path.join(out, "summary.txt")))

    def test_a_flag_left_behind_is_removed_and_reported(self):
        fake = FakeSet()
        original = fake.tv

        def sticky(cmd, timeout=120):
            r = original(cmd, timeout)
            if "slot.sh >" in cmd:
                fake.flag = True                     # the script died inside a window
            return r
        fake.tv = sticky
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 1, "the script's own removal failing is a problem even though the slot fixed it")
        self.assertFalse(fake.flag, "the slot's own check removed it")
        log = json.loads(load(os.path.join(out, "slot-log.json")))
        self.assertTrue(log["flag"]["gone"])
        self.assertTrue(any("still there" in p for p in log["problems"]))
        self.assertTrue(any("rm -f /var/luna/preferences/vt_devmode" in c for c in fake.calls))

    def test_a_failure_in_the_middle_still_checks_the_flag_unmutes_and_stands_by(self):
        fake = FakeSet(fail_on_key=True)
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 1, "a button that was not sent is a problem the exit code shows")
        self.assertFalse(fake.flag)
        self.assertFalse(fake.muted)
        self.assertTrue(fake.standby)
        self.assertTrue(any("button" in p for p in json.loads(load(os.path.join(out, "slot-log.json")))["problems"]))

    def test_the_receiver_stays_as_it_was_when_it_was_muted_before(self):
        fake = FakeSet()
        original = fake.urlopen

        def muted(req, timeout=0):
            if "/api/states/" in req.full_url:
                fake.ha_calls.append(("states", None))
                return Resp({"state": "on", "attributes": {"is_volume_muted": True, "volume_level": 0.1}})
            return original(req, timeout)
        fake.urlopen = muted
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 0)
        self.assertTrue(fake.muted, "it was muted before; it is muted after")


if __name__ == "__main__":
    unittest.main()
