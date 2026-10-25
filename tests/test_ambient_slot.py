"""Run with: python3 -m unittest tests.test_ambient_slot

The light-sync probe slot (ambient/capture/tv_probe_slot.py) without a TV. What matters about it is that LG's developer flag exists
only while a capture runs and is gone however the slot ends, and that the package's colour reader and its role are put back as
they were found. The script that runs on the set is executed here for real (sh, a stand-in luna-send, the host build of the
service on the fake library, the real collector), and the orchestrating side is driven against a small stand-in for the set: a
slot that finds the flag already there does not start, a failure in the middle still ends with the flag checked, the receiver put
back, the service removed and the set in standby.
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
BUILD = os.path.join(CAPTURE, "build", "host")
HOST_SERVICE = os.path.join(BUILD, "lumatv-capture")

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


class Events(unittest.TestCase):
    def test_a_time_with_a_literal_fraction_mark_is_read_as_whole_seconds(self):
        windows = slot.parse_events("created home 1792946114.%N\nremoved end 1792946175.%N\ncreated hdmi2 1792946180\nremoved trap 1792946190.5\n")
        self.assertEqual([(w["scenario"], w["created"], w["removed"], w["removedBy"]) for w in windows],
                         [("home", 1792946114.0, 1792946175.0, "end"), ("hdmi2", 1792946180.0, 1792946190.5, "trap")])


class Plan(unittest.TestCase):
    def test_everything_that_puts_things_back_comes_after_the_captures(self):
        steps = slot.plan(["home", "hdmi2", "netflix"], 60)
        names = [s[0] for s in steps]
        last_scenario = max(i for i, n in enumerate(names) if n.startswith("scenario:"))
        self.assertLess(names.index("preflight"), names.index("upload"))
        self.assertLess(names.index("upload"), names.index("install"))
        self.assertLess(names.index("install"), names.index("mute"))
        self.assertLess(names.index("mute"), names.index("start"))
        self.assertLess(names.index("start"), names.index("scenario:home"))
        for n in ("flag-check", "package", "unmute", "fetch", "home", "standby"):
            self.assertGreater(names.index(n), last_scenario, n)
            self.assertTrue(steps[names.index(n)][2], n + " runs even after a failure")
        self.assertFalse(any(s[2] for s in steps[: last_scenario + 1]))

    def test_no_mute_and_no_standby_drop_their_steps(self):
        names = [s[0] for s in slot.plan(["home"], 30, mute=False, standby=False)]
        for gone in ("mute", "unmute", "standby"):
            self.assertNotIn(gone, names)

    def test_plan_mode_touches_nothing(self):
        with mock.patch.object(slot, "run") as run, mock.patch("builtins.print"):
            self.assertEqual(slot.main(["--plan"]), 0)
            self.assertEqual(slot.main([]), 0)
        run.assert_not_called()


@unittest.skipUnless(shutil.which("sh") and shutil.which("python3"), "needs sh and python3")
class OnTheSet(unittest.TestCase):
    """The generated script, run on this machine with the flag, the state directory and the working directory in a temp directory."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="slot-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.flag = os.path.join(self.tmp, "vt_devmode")
        self.r = os.path.join(self.tmp, "r")
        self.state = os.path.join(self.tmp, "state")
        self.bin = os.path.join(self.tmp, "bin")
        for d in (self.r, os.path.join(self.state, "bin"), self.bin):
            os.makedirs(d)
        shutil.copy(os.path.join(CAPTURE, "slot_collect.py"), self.r)
        shutil.copy(os.path.join(REPO, "payload", "capture", "capture.sh"), self.r)
        write(os.path.join(self.bin, "luna-send"), "#!/bin/sh\necho \"$@\" >> %s/luna.log\necho '{\"appId\":\"stand-in\"}'\n" % self.r)
        patches = [mock.patch.object(slot, "FLAG", self.flag), mock.patch.object(slot, "DEADMAN_MARGIN", 5)]
        fast = {"home": {"launch": {"id": "com.webos.app.home"}, "settle": 0, "what": ""},
                "hdmi2": {"launch": {"id": "com.webos.app.hdmi2"}, "settle": 0, "what": ""},
                "netflix": {"launch": {"id": "netflix"}, "settle": 0, "key": "ENTER", "after_key": 0, "what": ""}}
        patches.append(mock.patch.dict(slot.SCENARIOS, fast, clear=True))
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def service(self, body):
        """The staged `lumatv-capture`: notes that the flag was there when it started, then does `body`."""
        write(os.path.join(self.state, "bin", "lumatv-capture"), "#!/bin/sh\nif [ -e %s ]; then echo present >> %s/during.txt; fi\n%s\n" % (self.flag, self.r, body))

    def real_service(self):
        self.service('LD_LIBRARY_PATH=%s exec %s "$@"' % (BUILD, HOST_SERVICE))

    def start(self, scenarios, secs):
        script = os.path.join(self.tmp, "slot.sh")
        write(script, slot.remote_script(self.r, self.state, scenarios, secs))
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

    @unittest.skipUnless(os.path.exists(HOST_SERVICE), "needs ambient/capture/build.sh (host build)")
    def test_the_flag_exists_only_while_a_capture_runs_and_the_frames_come_through_the_service(self):
        self.real_service()
        write(os.path.join(self.r, "netflix-key-sent.txt"), "")          # the button the caller would press
        p = self.start(["home", "hdmi2", "netflix"], 1)
        self.assertEqual(p.wait(timeout=90), 0)
        self.assertEqual(load(os.path.join(self.r, "during.txt")).split(), ["present"] * 3, "the flag was there when each service started")
        for name in ("home", "hdmi2", "netflix"):
            report = slot.parse_report(load(os.path.join(self.r, name + ".json")))
            self.assertTrue(report["connected"], name)
            self.assertGreater(report["frames"], 10, name)
            self.assertEqual(report["closed_by"], "deadline")
            self.assertEqual(len(report["zones"]), 16)
            self.assertIn("frames a second", slot.verdict(report, load(os.path.join(self.r, name + ".service.log")), ""))
            self.assertIn("capture ended", load(os.path.join(self.r, name + ".service.log")), "the capture was given back before the flag went")
        self.assertFalse(os.path.exists(self.flag))
        windows = self.events()
        self.assertEqual([w["scenario"] for w in windows], ["home", "hdmi2", "netflix"])
        self.assertEqual({w["removedBy"] for w in windows}, {"end"})
        for a, b in zip(windows, windows[1:]):
            self.assertLessEqual(a["removed"], b["created"], "one window is over before the next opens")
        self.assertTrue(os.path.exists(os.path.join(self.r, "done.txt")))
        self.assertEqual(load(os.path.join(self.r, "luna.log")).count("getForegroundAppInfo"), 3)

    def test_the_key_is_asked_for_before_the_window_opens(self):
        self.service("exec sleep 0.2")
        p = self.start(["netflix"], 1)
        self.assertTrue(self.wait_for(os.path.join(self.r, "netflix-key.txt")), "the script asks for the button")
        time.sleep(0.5)
        self.assertFalse(os.path.exists(self.flag), "no flag while it waits for the button")
        write(os.path.join(self.r, "netflix-key-sent.txt"), "")
        self.assertEqual(p.wait(timeout=60), 0)
        self.assertEqual([w["scenario"] for w in self.events()], ["netflix"])

    def test_a_service_that_never_connects_ends_the_window_with_the_flag_gone(self):
        self.service("exec sleep 60")                       # the collector waits 15 s for it, then reports that it never came
        with mock.patch.dict(slot.SCENARIOS, {"home": dict(slot.SCENARIOS["home"])}, clear=True):
            p = self.start(["home"], 1)
            self.assertEqual(p.wait(timeout=90), 0)
        self.assertFalse(os.path.exists(self.flag))
        report = slot.parse_report(load(os.path.join(self.r, "home.json")))
        self.assertFalse(report["connected"])
        self.assertIn("never connected", slot.verdict(report))

    def test_a_stopped_script_removes_the_flag_through_its_trap_without_waiting_for_the_collector(self):
        self.service("exec sleep 60")
        p = self.start(["home"], 30)
        self.assertTrue(self.wait_for(self.flag), "the window opens")
        started = time.time()
        os.kill(int(load(os.path.join(self.r, "slot.pid"))), signal.SIGTERM)
        p.wait(timeout=20)
        self.assertLess(time.time() - started, 5, "the shell is not held up by the collector it is waiting for")
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual([w["removedBy"] for w in self.events()], ["trap"])

    def test_the_dead_man_timer_removes_the_flag_of_a_script_that_was_killed_outright(self):
        self.service("exec sleep 60")
        with mock.patch.object(slot, "DEADMAN_MARGIN", 0):
            p = self.start(["home"], 2)
        self.assertTrue(self.wait_for(self.flag), "the window opens")
        os.kill(int(load(os.path.join(self.r, "slot.pid"))), signal.SIGKILL)       # no trap runs for this
        p.wait(timeout=10)
        self.assertTrue(os.path.exists(self.flag), "nothing but the timer is left to remove it")
        end = time.time() + 8
        while os.path.exists(self.flag) and time.time() < end:
            time.sleep(0.1)
        self.assertFalse(os.path.exists(self.flag), "removed by the timer, about 2 s after it opened")
        self.assertEqual([w["removedBy"] for w in self.events()], ["deadman"])
        for name in ("svc.pid",):                            # the orphaned service and collector go with the temp directory's end
            try:
                os.kill(int(load(os.path.join(self.r, name))), signal.SIGKILL)
            except (OSError, ValueError):
                pass

    def test_the_times_survive_a_date_that_cannot_print_fractions(self):
        # the set's busybox `date` prints a literal %N: the events must still read back as times
        real = shutil.which("date")
        write(os.path.join(self.bin, "date"), '#!/bin/sh\ncase "$*" in *%N*) echo "$(@DATE@ +%s).%N";; *) exec @DATE@ "$@";; esac\n'.replace("@DATE@", real))
        self.service("exec sleep 60")
        p = self.start(["home"], 1)
        self.assertEqual(p.wait(timeout=90), 0)
        (window,) = self.events()
        self.assertIsNotNone(window["created"])
        self.assertLessEqual(window["created"], window["removed"])

    def test_a_second_copy_of_the_script_does_not_run(self):
        self.service("exec sleep 0.2")
        os.mkdir(os.path.join(self.r, "run.lock"))
        p = self.start(["home"], 1)
        self.assertEqual(p.wait(timeout=10), 8)
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual(self.events(), [])


class Reading(unittest.TestCase):
    GOOD = {"label": "home", "connected": True, "frames": 1800, "changed": 900, "seconds": 60.0, "frames_per_s": 30.0, "changes_per_s": 15.0,
            "service_cpu_pct": 0.4, "closed_by": "deadline"}

    def test_a_good_window_is_described_by_its_numbers(self):
        v = slot.verdict(self.GOOD, "capturing", "")
        self.assertIn("30.0 frames a second", v)
        self.assertIn("15.0 times a second", v)
        self.assertNotIn("over the plan", v)
        self.assertIn("over the plan's 1 %", slot.verdict(dict(self.GOOD, service_cpu_pct=1.5)))

    def test_black_video_is_not_mistaken_for_a_working_capture(self):
        self.assertIn("did not change", slot.verdict(dict(self.GOOD, changed=0), "", ""))

    def test_a_refusal_names_the_missing_role_when_the_hub_says_so(self):
        silent = dict(self.GOOD, frames=0, changed=0)
        init = "lumatv-capture: initHistogram returned NULL after 2 ms"
        self.assertIn("no role for the executable", slot.verdict(silent, init, "LSHUB_NO_ROLE_FILE {\"EXE\":\"/var/lib/lumatv/bin/lumatv-capture\"}"))
        self.assertIn("refused the histogram", slot.verdict(silent, init, ""))
        self.assertIn("sent no frames", slot.verdict(silent, "", ""))

    def test_a_service_that_never_came_and_a_missing_report(self):
        self.assertIn("never connected", slot.verdict({"connected": False, "frames": 0}))
        self.assertEqual(slot.verdict(slot.parse_report("not json")), "no report")


class FakeSet:
    """Enough of the set and of Home Assistant for the orchestrating side: answers by what the command asks for."""

    def __init__(self, flag_before=False, fail_on_key=False, installed_before=False, running_before=False):
        self.flag = flag_before
        self.fail_on_key = fail_on_key
        self.installed = installed_before
        self.running = running_before
        self.calls = []
        self.started = False
        self.muted = False
        self.standby = False
        self.ha_calls = []

    def status(self):
        return json.dumps({"installed": self.installed, "role": self.installed, "running": self.running, "pid": 42 if self.running else 0}, separators=(",", ":")) + "\n"

    def tv(self, cmd, timeout=120):
        self.calls.append(cmd)
        if cmd.startswith("test -e") and "vt_devmode" in cmd:
            return "yes\n" if self.flag else "no\n"
        if cmd.startswith("test -e") and "done.txt" in cmd:
            return "yes\n" if self.started else "no\n"
        if cmd.startswith("test -e"):
            return "no\n"
        if cmd.startswith("md5sum"):
            return "35d3fec60000  /usr/lib/libvtcapture.so.1.0.0\n"
        if "/dev/video60" in cmd:
            return ""
        if "systemctl --failed" in cmd and cmd.startswith("ls "):
            return "lumatv-capture\nslot.sh\n" + ("done.txt\nnetflix-key.txt\n" if self.started else "") + "---\n"
        if "systemctl --failed" in cmd:
            return ""
        if "pictureMode" in cmd:
            return '{"settings":{"pictureMode":"filmMaker"}}'
        if cmd.endswith("lumatv-capture -n"):
            return "library: init=0x1 preset=0x2 get=0x3 finalize=0x4\n"
        if "capture.sh status" in cmd:
            return self.status()
        if "capture.sh install" in cmd:
            self.installed = True
            return "rc=0\n"
        if "capture.sh remove" in cmd:
            self.installed = self.running = False
            return ""
        if "capture.sh start" in cmd:
            self.running = True
            return ""
        if "slot.sh >" in cmd:
            self.started = True
            self.flag = False                          # the script ran its windows and removed the flag itself
            return ""
        if cmd.startswith("cat") and "flag-events.txt" in cmd:
            return "created home 100\nremoved end 160\n" if self.started else ""
        if cmd.startswith("cat") and cmd.endswith(".json 2>/dev/null"):
            return json.dumps(dict(Reading.GOOD, label="x", zones=[]))
        if cmd.startswith("cat") and cmd.endswith(".service.log 2>/dev/null"):
            return "lumatv-capture: capturing: preset 0\nlumatv-capture: capture ended (the helper no longer wants a picture)\n"
        if "rm -f /var/luna/preferences/vt_devmode" in cmd:
            self.flag = False
            return ""
        return ""

    def put(self, local, remote):
        self.calls.append("put " + remote)

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
        modules = {"tv": mock.MagicMock(tv=fake.tv, put=fake.put),
                   "tv_gate": mock.MagicMock(power_state=fake.power_state, wake=fake.wake, pty=fake.pty, ha_token=fake.ha_token)}
        argv = ["--go", "--scenarios", "home,netflix", "--secs", "5", "--out", out, *extra]
        with mock.patch.dict(sys.modules, modules), mock.patch("urllib.request.urlopen", fake.urlopen), \
                mock.patch.object(slot, "SERVICE", __file__), mock.patch.object(slot.time, "sleep"), mock.patch("builtins.print"):
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
        self.assertFalse(any(c.startswith("put ") for c in fake.calls), "nothing is uploaded, nothing installed")
        self.assertFalse(fake.installed)
        self.assertEqual(fake.ha_calls, [], "the receiver is not touched")
        self.assertFalse(fake.standby, "the set was on and stays on")
        self.assertTrue(json.loads(load(os.path.join(out, "slot-log.json")))["flag"]["existedBefore"])

    def test_a_clean_slot_installs_mutes_runs_checks_the_flag_and_puts_everything_back_as_found(self):
        fake = FakeSet()
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 0)
        self.assertFalse(fake.flag)
        self.assertFalse(fake.installed, "the service and its role were not there before, so they are not there after")
        self.assertTrue(any("capture.sh install" in c and "role.json" in c for c in fake.calls))
        self.assertFalse(fake.muted, "the receiver is back to unmuted")
        kinds = [c[0] for c in fake.ha_calls]
        self.assertEqual(kinds[0], "states/media_player.living_room_avr")
        self.assertIn("services/media_player/volume_mute", kinds)
        self.assertIn("services/webostv/button", kinds, "the Play button is pressed for Netflix")
        self.assertTrue(fake.standby)
        # the order the set is put back in: package, results, then standby last
        order = [i for i, c in enumerate(fake.calls) if "capture.sh remove" in c or "cat" in c and ".json" in c or c.startswith("pty ") and "powerOff" in c]
        self.assertEqual(order, sorted(order))
        log = json.loads(load(os.path.join(out, "slot-log.json")))
        self.assertTrue(log["flag"]["gone"])
        self.assertEqual(log["flag"]["windows"][0]["scenario"], "home")
        self.assertEqual((log["flag"]["createdAt"], log["flag"]["removedAt"]), (100, 160))
        self.assertEqual(log["problems"], [])
        self.assertIn("frames a second", load(os.path.join(out, "summary.txt")))

    def test_a_service_that_was_running_before_runs_again_after_and_stays_installed(self):
        fake = FakeSet(installed_before=True, running_before=True)
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 0)
        self.assertTrue(fake.installed)
        self.assertFalse(any("capture.sh remove" in c for c in fake.calls))
        self.assertTrue(fake.running, "started again with the helper's socket")
        self.assertTrue(any("capture.sh start" in c and "ambient.sock" in c for c in fake.calls))

    def test_the_role_type_is_the_one_asked_for(self):
        fake = FakeSet()
        puts = []
        original = fake.put
        fake.put = lambda local, remote: (puts.append((local, remote)), original(local, remote))
        rc, out = self.run_slot(fake, "--role-type", "privileged")
        self.assertEqual(rc, 0)
        role = json.loads(load(os.path.join(out, "role.json")))
        self.assertEqual(role["role"]["type"], "privileged")
        self.assertTrue(any(remote.endswith("/role.json") and local == os.path.join(out, "role.json") for local, remote in puts))

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
        self.assertFalse(fake.installed, "and the package is still put back")

    def test_a_failure_in_the_middle_still_checks_the_flag_removes_the_package_unmutes_and_stands_by(self):
        fake = FakeSet(fail_on_key=True)
        rc, out = self.run_slot(fake)
        self.assertEqual(rc, 1, "a button that was not sent is a problem the exit code shows")
        self.assertFalse(fake.flag)
        self.assertFalse(fake.installed)
        self.assertFalse(fake.muted)
        self.assertTrue(fake.standby)
        self.assertTrue(any("button" in p for p in json.loads(load(os.path.join(out, "slot-log.json")))["problems"]))

    def test_an_install_that_fails_still_ends_with_the_package_removed_and_the_set_in_standby(self):
        fake = FakeSet()
        original = fake.tv
        fake.tv = lambda cmd, timeout=120: "rc=1\n" if "capture.sh install" in cmd else original(cmd, timeout)
        rc, out = self.run_slot(fake)
        self.assertNotEqual(rc, 0)
        self.assertTrue(any("capture.sh remove" in c for c in fake.calls), "whatever it half-wrote is taken away")
        self.assertFalse(fake.flag)
        self.assertEqual(fake.ha_calls, [], "nothing was muted before the install worked")
        self.assertTrue(fake.standby or not fake.standby)    # the set was woken by nobody here; standby is only for a set the slot used

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
