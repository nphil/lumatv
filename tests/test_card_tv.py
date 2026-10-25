"""Run with: python3 -m unittest tests.test_card_tv

rustui/tools/tv-card.py, the card app's TV script, run against a simulated set: SAM's foreground, running list and launch, the card's
processes and log, the remote's keys. It says the script's flow and its parsing agree with one another and with the log lines the launcher
and the app write (the strings it looks for are the ones tests.test_card_launcher and the app's own log produce). It says nothing about
the TV: the simulation is written from the same reading of the capture the script is.
"""
import importlib.util
import json
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
APP = "org.nphil.lumatv.card"
HOME = "com.webos.app.home"


def load():
    spec = importlib.util.spec_from_file_location("tv_card", os.path.join(ROOT, "rustui", "tools", "tv-card.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    class Clock:  # the script's waits cost nothing here
        @staticmethod
        def sleep(_):
            pass

        time = staticmethod(__import__("time").time)
        strftime = staticmethod(__import__("time").strftime)

    mod.time = Clock
    return mod


class FakeSet:
    """Just enough of a TV: the card's life as SAM and the compositor lead it, the keys, and what the script reads back."""

    def __init__(self, registers=True, minimizes=True, seats=3):
        self.registers = registers
        # how many wl_seat globals the compositor advertises (LG's: one per input device, plus a spare); 0 is an app that bound none
        self.seats = seats
        self.minimizes = minimizes
        self.installed = False
        self.running = False
        self.fg = HOME
        self.page = "status"
        self.log = ""
        self.puts = []
        self.ticks = 100
        self.cleared = True

    def say(self, line):
        self.log += line + "\n"

    def start(self):
        self.running, self.fg, self.page, self.cleared = True, APP, "status", False
        self.say('[lumatv-card 1.000] launch: app %s reason undefined hidden 0 parameters {}, home %s/%s, uid 0 gid 0' % (APP, "/x", APP))
        self.say("[lumatv-card 1.001] the app started: pid 4101")
        self.say("card[1.5]: start: %s reason undefined hidden false" % APP)
        self.say("gfx64: 64-bit path: /x/lib/libmali.so")
        for i in range(self.seats):
            self.say("window: wl_seat %d bound at version 4 (%d seats)" % (6 + i, i + 1))
        self.say("card[1.55]: seats: %d bound, %d with a keyboard" % (self.seats, self.seats))
        if self.seats:
            self.say("window: keyboard focus on seat 6")
        self.say("card[1.6]: input State(3)")
        self.say('[lumatv-card 1.7] app: {"kind":"ready"}')
        self.say("[lumatv-card 1.8] " + ("SAM: registered" if self.registers else "LSRegister %s: Denied" % APP))

    def end(self, how):
        self.running, self.fg, self.cleared = False, HOME, True
        self.say("[lumatv-card 9.0] the app ended with 0")
        self.say("[lumatv-card 9.1] done: status 0")

    def tv(self, cmd, timeout=60):
        if "getPowerState" in cmd:
            return '{"returnValue": true, "state": "Active"}'
        if "tv-card-probe.sh facts" in cmd:
            return "id: uid=0(root)\nmachine: aarch64\ncompositor: LG\n"
        if "tv-card-probe.sh pids" in cmd:
            if not self.running:
                return ""
            return ("stub 4100 0 /x/bin/lumatv-card {...}\nshell 4101 0 sh /x/gfx64/launch.sh -- /x/bin64/lumatv-card-app\n"
                    "app 4102 0 /x/ld-linux-aarch64.so.1 --library-path /x /x/bin64/lumatv-card-app --app-id %s\n" % APP)
        if "tv-card-probe.sh ticks" in cmd:
            return "%d\n" % self.ticks
        if "tv-card-probe.sh rss" in cmd:
            return "152000\n"
        if "listApps" in cmd:
            apps = [{"id": APP, "type": "native", "main": "bin/lumatv-card", "version": "0.1.0", "nativeLifeCycleInterfaceVersion": 2, "visible": True}] if self.installed else []
            return json.dumps({"returnValue": True, "apps": apps + [{"id": "netflix", "type": "native", "systemApp": False}]})
        if "listLaunchPoints" in cmd:
            return json.dumps({"launchPoints": [{"id": APP}] if self.installed else []})
        if "applicationManager/running" in cmd:
            return json.dumps({"running": [{"id": APP}] if self.running else []})
        if "getForegroundAppInfo" in cmd:
            item = {"appId": self.fg, "windowType": "_WEBOS_WINDOW_TYPE_CARD", "processId": "4102" if self.fg == APP else "900"}
            return json.dumps({"returnValue": True, "foregroundAppInfo": [item]})
        if "applicationManager/launch" in cmd:
            if not self.running:
                self.start()
            else:
                self.fg = APP
                self.say("[lumatv-card 5.0] SAM: relaunch (undefined) {}")
                self.say("card[5.1]: input Relaunch { reason: \"undefined\", parameters: \"{}\" }")
                self.say("card[5.2]: asking the compositor for state 3")
                self.say("card[5.3]: input State(3)")
            return '{"returnValue": true}'
        if "closeByAppId" in cmd:
            if self.running:
                self.say("[lumatv-card 8.0] SAM: close (undefined)")
                self.end("close")
            return '{"returnValue": true}'
        if "kb.py HOME" in cmd:
            self.fg = HOME
            if self.minimizes:
                self.say("card[3.0]: input State(1)")
                self.say('[lumatv-card 3.1] app: {"kind":"state","state":"minimized"}')
            return ""
        if "kb.py BACK:1300" in cmd:
            if self.running:
                self.say("[lumatv-card 8.5] SAM: close (undefined)")
                self.end("exit")
            return ""
        if "kb.py OK" in cmd:
            self.say("card[2.0]: input Key { code: 28, pressed: true }")
            self.page = "keys"
            self.say('[lumatv-card 2.1] app: {"kind":"page","page":"keys"}')
            return ""
        if "kb.py BACK" in cmd:
            self.say("card[2.5]: input Key { code: 412, pressed: true }")
            if self.page == "keys":
                self.page = "status"
                self.say('[lumatv-card 2.6] app: {"kind":"page","page":"status"}')
            else:
                self.end("back")
            return ""
        if "listRecents" in cmd:
            return json.dumps({"recentsAppList": [{"appId": APP}] if self.running else []})
        if "cat /tmp/lumatv-card.log" in cmd:
            return self.log
        if "lumatv-card-shm" in cmd:
            return "gone\n" if self.cleared else "there\n"
        if "card_install" in cmd:
            self.installed = True
            return "card app 0.1.0 installed\nrc=0\n{...}\n"
        if "card_remove" in cmd:
            self.installed = False
            return "card app removed\nrc=0\n"
        if "ls -l" in cmd and "bin/lumatv-card" in cmd:
            return "-rwxr-xr-x 1 root root 15688 Oct 25 13:43 /media/developer/apps/usr/palm/applications/%s/bin/lumatv-card\n" % APP
        if cmd.startswith("rm -f"):
            self.log = ""
            return ""
        if cmd.startswith("wc -c <"):
            return "4096\n"
        return ""

    def put(self, local, remote):
        self.puts.append((os.path.basename(local), remote))

    def get(self, remote, local):
        with open(local, "wb") as f:
            f.write(b"png")


class CardTv(unittest.TestCase):
    def setUp(self):
        self.mod = load()
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, fake, *args):
        run = self.mod.Run(fake, os.path.join(self.tmp.name, "out"))
        self.mod.preflight(run)
        for a in args:
            getattr(self.mod, a)(run)
        return run

    def test_a_card_that_behaves_passes_every_step(self):
        fake = FakeSet()
        fake.installed = True
        run = self.run_script(fake, "scenario")
        fails = [l for l in run.lines if l.startswith("FAIL")]
        self.assertEqual(fails, [], "\n".join(run.lines))
        self.assertGreaterEqual(len([l for l in run.lines if l.startswith("PASS")]), 25)
        self.assertEqual(run.failed, 0)
        self.assertFalse(fake.running)
        self.assertEqual(fake.fg, HOME)
        names = [l.split(":")[0] for l in run.lines]
        for want in ("PASS launch", "PASS keys", "PASS home", "PASS relaunch", "PASS back", "PASS close", "PASS exit"):
            self.assertTrue(any(n.startswith(want) for n in names), want)
        self.assertTrue(os.path.exists(os.path.join(self.tmp.name, "out", "status.png")), "a screenshot is fetched")

    def test_the_install_step_puts_the_ipk_and_the_helper_on_the_set_and_checks_the_result(self):
        fake = FakeSet()
        out = os.path.join(ROOT, "build", "card-ipk", APP + ".ipk")
        made = not os.path.exists(out)
        if made:
            os.makedirs(os.path.dirname(out), exist_ok=True)
            open(out, "wb").close()
        try:
            run = self.run_script(fake, "install")
        finally:
            if made:
                os.remove(out)
        self.assertEqual(run.failed, 0, "\n".join(run.lines))
        self.assertIn((APP + ".ipk", "/tmp/%s.ipk" % APP), fake.puts)
        self.assertIn(("cardapp.sh", "/tmp/cardapp.sh"), fake.puts)
        self.assertTrue(fake.installed)

    def test_a_launcher_the_hub_refused_is_a_failure_that_names_the_step(self):
        fake = FakeSet(registers=False)
        fake.installed = True
        run = self.run_script(fake, "scenario")
        fails = [l for l in run.lines if l.startswith("FAIL")]
        self.assertTrue(any("the launcher registered with SAM" in l for l in fails), "\n".join(run.lines))
        self.assertGreater(run.failed, 0)

    def test_a_card_that_is_not_told_it_is_covered_is_a_failure(self):
        fake = FakeSet(minimizes=False)
        fake.installed = True
        run = self.run_script(fake, "scenario")
        self.assertTrue(any(l.startswith("FAIL") and "told it is minimized" in l for l in run.lines), "\n".join(run.lines))

    def test_an_app_that_bound_no_seat_is_a_failure_that_names_the_keys_step(self):
        fake = FakeSet(seats=0)
        fake.installed = True
        run = self.run_script(fake, "scenario")
        self.assertTrue(any(l.startswith("FAIL") and "bound the compositor's seats" in l for l in run.lines), "\n".join(run.lines))

    def test_a_busy_covered_card_is_a_failure(self):
        fake = FakeSet()
        fake.installed = True
        readings = iter([100, 160])
        original = fake.tv

        def tv(cmd, timeout=60):
            if "tv-card-probe.sh ticks" in cmd:
                return "%d\n" % next(readings, 160)
            return original(cmd, timeout)

        fake.tv = tv
        run = self.run_script(fake, "scenario")
        self.assertTrue(any(l.startswith("FAIL") and "covered card is idle" in l for l in run.lines), "\n".join(run.lines))

    def test_installing_and_removing_need_the_approval_flag(self):
        for verb in ("install", "remove", "all"):
            with self.assertRaises(SystemExit) as caught:
                self.mod.main([verb], tv=FakeSet())
            self.assertIn("Nitin's go first", str(caught.exception))
        with self.assertRaises(SystemExit) as caught:
            self.mod.main([], tv=FakeSet())
        self.assertIn("tv-card.py facts", str(caught.exception))

    def test_it_refuses_a_set_that_is_not_awake(self):
        fake = FakeSet()
        original = fake.tv
        fake.tv = lambda cmd, timeout=60: '{"state": "Active Standby"}' if "getPowerState" in cmd else original(cmd, timeout)
        with self.assertRaises(SystemExit) as caught:
            self.run_script(fake, "scenario")
        self.assertIn("not Active", str(caught.exception))

    def test_it_will_not_start_over_something_else(self):
        fake = FakeSet()
        fake.installed = True
        fake.fg = "netflix"
        with self.assertRaises(SystemExit) as caught:
            self.run_script(fake, "scenario")
        self.assertIn("Home is not in front", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
