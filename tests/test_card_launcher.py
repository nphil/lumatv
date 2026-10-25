"""Run with: python3 -m unittest tests.test_card_launcher

The card app's 32-bit launcher (rustui/app/stub/lumatv-card.c, SAM's `main` for the app), run on the host against a stand-in for LG's
libluna-service2 (stub/fakels2.c: the real glib's loop, SAM played from a script) and a stand-in for the app (stub/fakeapp.sh). It
shows what the launcher owes SAM and the app: the registration, SAM's relaunch and close passed on, a refused name or call told to the
app, a preload started hidden, a close the app ignores turned into SIGTERM and SIGKILL, the app ended when the launcher is, and the
launcher's own exit following the app's. Nothing here says the set's hub accepts the registration: that is the TV step list's.
"""
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
STUB = os.path.join(ROOT, "rustui", "app", "stub")
APP = "org.nphil.lumatv.card"
LAUNCH = json.dumps({"@system_native_app": True, "reason": "undefined", "interfaceVersion": 2, "event": "launch",
                     "interfaceMethod": "registerApp", "appId": APP, "parameters": {}})

REGISTERED = '{"event":"registered","returnValue":true}'
RELAUNCH = '{"event":"relaunch","reason":"undefined","appId":"%s","parameters":{"launchType":"homeKey"},"returnValue":true}' % APP
CLOSE = '{"event":"close","reason":"undefined","returnValue":true}'


def have_toolchain():
    return shutil.which("zig") is not None and os.path.exists("/usr/lib/x86_64-linux-gnu/libglib-2.0.so.0")


@unittest.skipUnless(have_toolchain(), "needs zig and the host's libglib-2.0.so.0")
class Launcher(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="card-launcher-")
        built = subprocess.run(["sh", os.path.join(STUB, "build.sh"), os.path.join(cls.tmp, "build")], capture_output=True, text=True)
        assert built.returncode == 0, built.stdout + built.stderr
        cls.host = os.path.join(cls.tmp, "build", "host")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def setUp(self):
        self.dir = tempfile.mkdtemp(dir=self.tmp)
        self.paths = {k: os.path.join(self.dir, k) for k in ("ls", "app", "log", "script")}

    def start(self, launch=LAUNCH, events=(), env=None, child=None):
        """Starts the launcher with SAM's script `events` ([(ms, json)]); returns the process."""
        with open(self.paths["script"], "w") as f:
            for ms, payload in events:
                f.write("%d %s\n" % (ms, payload))
        e = dict(os.environ)
        e.update({
            "LD_LIBRARY_PATH": self.host,
            "FAKE_LS_LOG": self.paths["ls"],
            "FAKE_SAM_EVENTS": self.paths["script"],
            "FAKE_APP_LOG": self.paths["app"],
            "LUMATV_CARD_LOG": self.paths["log"],
            "LUMATV_CARD_CHILD": child or os.path.join(self.host, "fakeapp.sh"),
            "LUMATV_CARD_HOME": self.dir,
        })
        for k in ("XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"):
            e.pop(k, None)
        e.update(env or {})
        args = [os.path.join(self.host, "lumatv-card")] + ([launch] if launch is not None else [])
        return subprocess.Popen(args, env=e, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def text(self, name):
        try:
            with open(self.paths[name]) as f:
                return f.read()
        except FileNotFoundError:
            return ""

    def wait_for(self, name, needle, timeout=10.0):
        end = time.time() + timeout
        while time.time() < end:
            if needle in self.text(name):
                return True
            time.sleep(0.02)
        return needle in self.text(name)

    def finish(self, proc, timeout=15):
        try:
            return proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
            self.fail("the launcher did not end\n" + self.text("log") + self.text("ls") + self.text("app"))

    def app_lines(self):
        return [l[5:] for l in self.text("app").splitlines() if l.startswith("got: ")]

    def test_registers_passes_sams_relaunch_and_close_on_and_ends_with_the_app(self):
        proc = self.start(events=[(50, REGISTERED), (300, RELAUNCH), (700, CLOSE)])
        self.assertEqual(self.finish(proc), 0, self.text("log"))
        ls = self.text("ls")
        self.assertIn("register " + APP, ls)
        self.assertIn('call luna://com.webos.applicationManager/registerApp {"appId":"%s","subscribe":true}' % APP, ls)
        self.assertLess(ls.index("cancel"), ls.index("unregister"), "the subscription is cancelled, then the name given back")
        got = self.app_lines()
        self.assertEqual(got, [
            '{"kind":"sam","registered":true}',
            '{"kind":"relaunch","reason":"undefined","parameters":{"launchType":"homeKey"}}',
            '{"kind":"close","reason":"undefined"}',
        ])
        argv = self.text("app").splitlines()[0]
        for want in ("--app-id " + APP, "--reason undefined", "--channel-fd 3", "--assets " + os.path.join(self.dir, "assets")):
            self.assertIn(want, argv)
        self.assertNotIn("--hidden", argv)
        self.assertIn("XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0", self.text("app"), "LG's socket by default")
        log = self.text("log")
        self.assertIn('app: {"kind":"ready"}', log)
        self.assertIn("SAM: registered", log)
        self.assertIn("SAM: close (undefined)", log)

    def test_a_name_the_hub_refuses_leaves_the_app_running_and_told(self):
        proc = self.start(env={"FAKE_LS_REFUSE": "1", "FAKE_APP_EXIT": "1:0"})
        self.assertEqual(self.finish(proc), 0)
        self.assertEqual(self.app_lines(), ['{"kind":"sam","registered":false}'])
        self.assertNotIn("call ", self.text("ls"), "nothing is asked of SAM without a name")
        self.assertIn("LSRegister " + APP, self.text("log"))

    def test_a_call_sam_refuses_is_told_too_and_the_launcher_returns_the_apps_status(self):
        proc = self.start(events=[(50, '{"returnValue":false,"errorText":"Denied method call"}')], env={"FAKE_APP_EXIT": "1:3"})
        self.assertEqual(self.finish(proc), 3)
        self.assertEqual(self.app_lines(), ['{"kind":"sam","registered":false}'])
        self.assertIn("SAM refused registerApp", self.text("log"))

    def test_a_preload_starts_the_app_hidden(self):
        for launch in (LAUNCH.replace('"undefined"', '"preload"'), LAUNCH.replace('"parameters": {}', '"parameters": {"snapshot": true}')):
            self.setUp()
            proc = self.start(launch=launch, env={"FAKE_APP_EXIT": "1:0"})
            self.assertEqual(self.finish(proc), 0)
            self.assertIn("--hidden", self.text("app").splitlines()[0], launch)

    def test_parameters_reach_the_app_as_one_argument(self):
        launch = LAUNCH.replace('"parameters": {}', '"parameters": {"contentTarget": "a b \\"c\\"", "n": [1, 2]}')
        proc = self.start(launch=launch, env={"FAKE_APP_EXIT": "1:0"})
        self.assertEqual(self.finish(proc), 0)
        self.assertIn('--params {"contentTarget": "a b \\"c\\"", "n": [1, 2]} --channel-fd', self.text("app").splitlines()[0])

    def test_the_apps_picture_files_are_cleared_when_it_is_gone(self):
        shm = os.path.join(self.dir, "shm")
        os.makedirs(shm)
        for name in ("layer-1", "layer-2"):
            with open(os.path.join(shm, name), "w") as f:
                f.write("pixels")
        proc = self.start(env={"FAKE_APP_EXIT": "0.5:0", "LUMATV_CARD_SHM": shm})
        self.assertEqual(self.finish(proc), 0)
        self.assertIn("--shm " + shm, self.text("app").splitlines()[0])
        self.assertFalse(os.path.exists(shm), "the directory is gone with the files in it")

    def test_a_launch_without_json_still_starts_the_app_under_the_default_id(self):
        proc = self.start(launch=None, env={"FAKE_APP_EXIT": "1:0"})
        self.assertEqual(self.finish(proc), 0)
        self.assertIn("register " + APP, self.text("ls"))

    def test_a_close_the_app_ignores_becomes_sigterm_and_then_sigkill(self):
        started = time.time()
        proc = self.start(events=[(100, REGISTERED), (200, CLOSE)], env={"FAKE_APP_IGNORE_CLOSE": "1", "LUMATV_CARD_KILL_MS": "400"})
        code = self.finish(proc)
        self.assertEqual(code, 128 + signal.SIGKILL)
        self.assertGreater(time.time() - started, 2.0, "SIGKILL comes 2 s after SIGTERM")
        self.assertIn("did not end in 400 ms: SIGTERM", self.text("log"))
        self.assertIn("SIGKILL", self.text("log"))

    def test_an_app_that_answers_sigterm_ends_without_the_kill(self):
        proc = self.start(events=[(100, REGISTERED), (200, CLOSE)], env={"FAKE_APP_IGNORE_CLOSE": "1", "FAKE_APP_HONOUR_TERM": "1", "LUMATV_CARD_KILL_MS": "300"})
        self.assertEqual(self.finish(proc), 0)
        self.assertIn("got TERM", self.text("app"))
        self.assertNotIn("SIGKILL", self.text("log"))

    def test_sigterm_to_the_launcher_stops_the_app(self):
        proc = self.start(events=[(100, REGISTERED)], env={"FAKE_APP_HONOUR_TERM": "1", "FAKE_APP_IGNORE_CLOSE": "1"})
        self.assertTrue(self.wait_for("app", "got: {\"kind\":\"sam\""), self.text("log"))
        proc.send_signal(signal.SIGTERM)
        self.assertEqual(self.finish(proc), 0)
        self.assertIn("got TERM", self.text("app"))
        self.assertIn("SIGTERM: stopping the app", self.text("log"))
        self.assertIn("unregister", self.text("ls"))

    def test_an_app_that_ends_first_takes_the_launcher_with_it_and_gives_the_name_back(self):
        proc = self.start(events=[(50, REGISTERED)], env={"FAKE_APP_EXIT": "0.5:0"})
        self.assertEqual(self.finish(proc), 0)
        ls = self.text("ls")
        self.assertIn("cancel", ls)
        self.assertIn("unregister", ls)

    def test_the_app_ends_when_the_launcher_is_killed(self):
        proc = self.start(events=[(50, REGISTERED)], env={"FAKE_APP_IGNORE_CLOSE": "1"})
        self.assertTrue(self.wait_for("app", "got: {\"kind\":\"sam\""), self.text("log"))
        proc.kill()
        proc.wait()
        self.assertTrue(self.wait_for("app", "channel closed", 5), "the app saw its channel close: " + self.text("app"))

    def test_an_app_that_cannot_start_is_the_launchers_exit_status(self):
        proc = self.start(child="/nonexistent/app")
        self.assertEqual(self.finish(proc), 127)
        self.assertIn("exec /nonexistent/app", self.text("log"))


if __name__ == "__main__":
    unittest.main()
