"""Run with: cd tests && python3 -m unittest test_vplane_install   (the end-to-end cases need `sh vplane/build.sh`)

payload/vplane/vplane.sh, what `tools/lumatv` runs for the video-plane player at apply and revert: the player is staged at a path of its own
with the clips, its LS2 role is written for exactly that path, the hub is asked to rescan only when the role changed, a supervisor keeps the player
up and gives up on one that will not stay, and remove takes it all away. Run against a temporary state directory and role directory, with a
stand-in `ls-control` that records what it was asked. What the decoder library registers with is the TV's to say
(docs/research/videoplane-saver-result.md, the TV steps).
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
REPO = os.path.dirname(HERE)
SH = os.path.join(REPO, "payload", "vplane", "vplane.sh")
ROLE = os.path.join(REPO, "payload", "vplane", "role.json")
CLIPS = os.path.join(REPO, "payload", "vplane", "clips")
HOST_BIN = os.path.join(REPO, "vplane", "build", "host", "lumatv-vplane")
NDL = os.path.join(REPO, "vplane", "build", "host")

# What LG's media library registers on the set (2026-10-25 23:3x, docs/research/videoplane-saver-result.md): the resource-manager and
# media clients, the pipeline and AV-mute clients, the AV connector, the video-texture client and one handle with no name at all
# (`LSHUB_NO_NAME_PERMS ... register name: "(null)"` without the empty entry), the names Moonlight's and PlxNative's roles carry; and
# what each may call.
CLIENT_NAMES = {"com.webos.rm.client.*", "com.webos.media.client.*", "com.webos.pipeline.*", "com.webos.pipe.avmuteclient.*",
                "com.webos.avconnector.*", "com.webos.vt.*", ""}
OUTBOUND = {"com.webos.rm.client.*": ["com.webos.media"], "com.webos.media.client.*": ["com.webos.media"],
            "com.webos.pipeline.*": ["com.webos.pipelinectrl.*"], "com.webos.pipe.avmuteclient.*": ["com.webos.service.avmutemanager"],
            "com.webos.avconnector.*": ["com.webos.service.videooutput", "com.webos.service.audio", "com.webos.service.multiviewcontroller",
                                        "com.webos.service.utp.extinputs", "com.webos.service.config"],
            "com.webos.vt.*": ["*"]}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="vplane-install-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = os.path.join(self.tmp, "state")
        self.mailbox = os.path.join(self.tmp, "mailbox")
        self.ls2 = os.path.join(self.tmp, "ls2")
        self.bin = os.path.join(self.tmp, "bin")
        self.calls = os.path.join(self.tmp, "ls-control.calls")
        os.mkdir(self.bin)
        with open(os.path.join(self.bin, "ls-control"), "w") as f:
            f.write('#!/bin/sh\necho "$@" >> %s\n' % self.calls)
        os.chmod(os.path.join(self.bin, "ls-control"), 0o755)
        self.package = self.script("#!/bin/sh\nexec sleep 30\n", "lumatv-vplane")
        self.env = dict(os.environ, LUMATV_LS2_DIR=self.ls2, PATH=self.bin + os.pathsep + os.environ["PATH"], LD_LIBRARY_PATH=NDL,
                        LUMATV_VPLANE_RESTART_SLEEP="0.05")
        self.addCleanup(self.run_sh, "stop", self.state)

    def script(self, text, name):
        path = os.path.join(self.tmp, name)
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, 0o755)
        return path

    def run_sh(self, *args):
        return subprocess.run(["sh", SH, *args], env=self.env, capture_output=True, text=True, timeout=30)

    def install(self, package=None, clips=None):
        return self.run_sh("install", package or self.package, ROLE, clips or CLIPS, self.state)

    def ls_calls(self):
        try:
            with open(self.calls) as f:
                return f.read().split("\n")[:-1]
        except FileNotFoundError:
            return []

    def role(self):
        with open(os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")) as f:
            return json.load(f)

    def status(self):
        return json.loads(self.run_sh("status", self.state).stdout)

    def wait(self, pred, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if pred():
                return True
            time.sleep(0.02)
        return False

    def log(self):
        try:
            with open(os.path.join(self.state, "vplane.log")) as f:
                return f.read()
        except FileNotFoundError:
            return ""


class Install(Base):
    def test_the_role_is_written_for_the_staged_path_and_allows_only_the_media_clients_calls(self):
        r = self.install()
        self.assertEqual(r.returncode, 0, r.stderr)
        exe = os.path.join(self.state, "bin", "lumatv-vplane")
        self.assertTrue(os.access(exe, os.X_OK))
        role = self.role()
        self.assertEqual(role["exeName"], exe, "ls-hubd matches a role by the executable's path")
        self.assertEqual(role["type"], "regular")
        self.assertEqual(set(role["allowedNames"]), CLIENT_NAMES, "the names LG's media library registers, and no other")
        self.assertEqual({r["service"] for r in role["permissions"]}, CLIENT_NAMES - {""}, "the nameless handle has no rule of its own")
        for rule in role["permissions"]:
            self.assertEqual(rule["outbound"], OUTBOUND[rule["service"]], "each client calls only what LG's own roles let it call")
            self.assertIn(rule["service"], CLIENT_NAMES)
        with open(os.path.join(self.ls2, "manifests.d", "org.nphil.lumatv.vplane.json")) as f:
            m = json.load(f)
        self.assertEqual(m["id"], "org.nphil.lumatv.vplane")
        self.assertEqual(m["roleFiles"], [os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")], "the hub reads a volatile role through its manifest")
        self.assertEqual(self.ls_calls(), ["scan-volatile-dirs", "scan-services"], "the hub is asked to read it")

    def test_the_clips_are_the_packages_whole_and_a_stale_one_is_gone(self):
        self.install()
        staged = os.path.join(self.state, "vplane", "clips")
        self.assertEqual(sorted(os.listdir(staged)), sorted(os.listdir(CLIPS)))
        for n in os.listdir(CLIPS):
            with open(os.path.join(CLIPS, n), "rb") as a, open(os.path.join(staged, n), "rb") as b:
                self.assertEqual(a.read(), b.read(), n)
        with open(os.path.join(staged, "left-over.vpc"), "wb") as f:
            f.write(b"an earlier package's print")
        self.assertEqual(self.install().returncode, 0)
        self.assertNotIn("left-over.vpc", os.listdir(staged), "what the package no longer has is no longer there")
        self.assertFalse([n for n in os.listdir(os.path.join(self.state, "vplane")) if n != "clips"], "no half-staged directory is left")

    def test_installing_again_does_not_ask_the_hub_to_rescan_and_a_new_binary_is_swapped_in(self):
        self.install()
        self.assertEqual(len(self.ls_calls()), 2)
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(len(self.ls_calls()), 2, "the role did not change, so every boot does not rescan")
        newer = self.script("#!/bin/sh\nexec sleep 31\n", "newer")
        self.assertEqual(self.install(newer).returncode, 0)
        with open(os.path.join(self.state, "bin", "lumatv-vplane")) as f:
            self.assertIn("sleep 31", f.read())
        self.assertEqual(len(self.ls_calls()), 2, "the same path, the same role")

    def test_a_package_without_the_binary_or_the_clips_installs_nothing(self):
        r = self.install(os.path.join(self.tmp, "no-such-binary"))
        self.assertNotEqual(r.returncode, 0)
        r = self.install(clips=os.path.join(self.tmp, "no-clips"))
        self.assertNotEqual(r.returncode, 0)
        empty = os.path.join(self.tmp, "empty")
        os.mkdir(empty)
        self.assertNotEqual(self.install(clips=empty).returncode, 0, "an empty directory is not a set of prints")
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")))
        self.assertEqual(self.ls_calls(), [])

    def test_the_status_says_what_is_there(self):
        self.assertEqual(self.status(), {"installed": False, "role": False, "clips": 0, "running": False, "pid": 0})
        self.install()
        s = self.status()
        self.assertEqual((s["installed"], s["role"], s["clips"], s["running"]), (True, True, len([n for n in os.listdir(CLIPS) if n.endswith(".vpc")]), False))

    def test_remove_takes_the_role_the_binary_and_the_clips_away_and_asks_for_a_rescan_once(self):
        self.install()
        self.assertEqual(self.run_sh("remove", self.state).returncode, 0)
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")))
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "manifests.d", "org.nphil.lumatv.vplane.json")))
        self.assertFalse(os.path.exists(os.path.join(self.state, "bin", "lumatv-vplane")))
        self.assertFalse(os.path.exists(os.path.join(self.state, "vplane", "clips")))
        self.assertEqual(self.status(), {"installed": False, "role": False, "clips": 0, "running": False, "pid": 0})
        self.assertEqual(self.ls_calls()[2:], ["scan-volatile-dirs", "scan-services"])
        self.run_sh("remove", self.state)
        self.assertEqual(len(self.ls_calls()), 4, "removing what is not there asks nothing")

    def test_it_will_not_start_what_is_not_installed_or_has_nowhere_to_listen(self):
        self.assertNotEqual(self.run_sh("start", self.state, self.mailbox).returncode, 0)
        self.install()
        self.assertNotEqual(self.run_sh("start", self.state, "").returncode, 0)


class Supervisor(Base):
    """The supervisor with stand-in players that say how they end, and with the real one."""

    def fake(self, status):
        log = os.path.join(self.tmp, "runs")
        return self.script('#!/bin/sh\necho "$@" >> %s\nexit %s\n' % (log, status), "fake-%s" % status), log

    def runs(self, log):
        try:
            with open(log) as f:
                return f.read().splitlines()
        except FileNotFoundError:
            return []

    def test_a_player_that_will_not_stay_is_started_five_more_times_and_then_left(self):
        fake, log = self.fake(1)
        self.install(fake)
        self.assertEqual(self.run_sh("start", self.state, self.mailbox).returncode, 0)
        self.assertTrue(self.wait(lambda: "not started again" in self.log(), 8), self.log())
        self.assertEqual(len(self.runs(log)), 6, "the first start and five more")
        self.assertTrue(self.wait(lambda: not os.path.exists(os.path.join(self.state, "vplane.pid")), 3), "and the supervisor is gone")

    def test_a_hung_teardown_is_started_again_but_a_clean_end_or_a_refusal_is_not(self):
        for status, want in ((3, 6), (0, 1), (2, 1)):
            shutil.rmtree(self.state, ignore_errors=True)
            fake, log = self.fake(status)
            if os.path.exists(log):
                os.unlink(log)
            self.install(fake)
            self.run_sh("start", self.state, self.mailbox)
            self.assertTrue(self.wait(lambda: not os.path.exists(os.path.join(self.state, "vplane.pid")), 8), (status, self.log()))
            self.assertEqual(len(self.runs(log)), want, status)
            self.run_sh("stop", self.state)

    def test_the_players_command_line_is_the_mailbox_the_staged_clips_the_app_id_and_the_args_file(self):
        fake, log = self.fake(0)
        self.install(fake)
        os.makedirs(os.path.join(self.state, "vplane"), exist_ok=True)
        with open(os.path.join(self.state, "vplane", "args"), "w") as f:
            f.write("-w card -e after\n")
        self.run_sh("start", self.state, self.mailbox, "org.example.vplane")
        self.assertTrue(self.wait(lambda: self.runs(log), 5))
        self.assertEqual(self.runs(log)[0], "-d %s -c %s -a org.example.vplane -w card -e after" % (self.mailbox, os.path.join(self.state, "vplane", "clips")))

    @unittest.skipUnless(os.path.exists(HOST_BIN), "needs vplane/build.sh (the host build)")
    def test_the_real_player_is_started_once_listens_on_the_mailbox_and_stop_ends_it(self):
        self.install(HOST_BIN)
        self.assertEqual(self.run_sh("start", self.state, self.mailbox).returncode, 0)
        self.assertTrue(self.wait(lambda: os.path.exists(os.path.join(self.mailbox, "status.json")), 5), self.log())
        with open(os.path.join(self.mailbox, "status.json")) as f:
            st = json.load(f)
        self.assertEqual(st["state"], "idle")
        self.assertGreaterEqual(st["clips"], 6)
        self.assertTrue(self.wait(lambda: self.status()["running"], 3))
        first = self.status()["pid"]
        self.run_sh("start", self.state, self.mailbox)
        self.assertEqual(self.status()["pid"], first, "started once, not twice")
        self.assertEqual(self.status()["pid"], st["pid"], "it is the staged executable that runs: that is what the role names")
        self.assertEqual(self.run_sh("stop", self.state).returncode, 0)
        self.assertFalse(self.status()["running"])
        self.assertTrue(self.wait(lambda: not os.path.exists("/proc/%d" % first) or open("/proc/%d/stat" % first).read().split()[2] == "Z", 3), "and it is gone")

    @unittest.skipUnless(os.path.exists(HOST_BIN), "needs vplane/build.sh (the host build)")
    def test_a_player_that_is_killed_comes_back(self):
        self.install(HOST_BIN)
        self.run_sh("start", self.state, self.mailbox)
        self.assertTrue(self.wait(lambda: self.status()["running"], 5), self.log())
        first = self.status()["pid"]
        os.kill(first, signal.SIGKILL)
        self.assertTrue(self.wait(lambda: self.status()["running"] and self.status()["pid"] != first, 5), self.log())
        self.assertIn("starting it again", self.log())


if __name__ == "__main__":
    unittest.main()
