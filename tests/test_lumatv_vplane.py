"""Run with: python3 -m unittest tests.test_lumatv_vplane

How tools/lumatv drives the art saver's video-plane player (payload/vplane/vplane.sh, vplane/): the `vplane` verb, apply's start of a player that is
installed (the boot's apply too), revert's removal, and the rule that nothing but the owner's own `lumatv vplane install` writes the player's LS2
role, a permission for a new program. The wiring runs against a stand-in vplane.sh that records what it is asked; the contract between tool and
script (argument order, the state it reads) runs against the real script in a temporary state and role directory with a stand-in player.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")
TOOL = os.path.join(TOOLS, "lumatv")
REAL_SH = os.path.join(ROOT, "payload", "vplane", "vplane.sh")
REAL_ROLE = os.path.join(ROOT, "payload", "vplane", "role.json")
REAL_CLIPS = os.path.join(ROOT, "payload", "vplane", "clips")
SAVER = os.path.join(ROOT, "payload", "compositor", "lumatv", "panels", "SaverVideo.qml")
with open(os.path.join(ROOT, "brand.json")) as _f:
    BRAND = json.load(_f)
MAILBOX = "/tmp/%s-vplane" % BRAND["shortId"]
APPID = "%s.vplane" % BRAND["appId"]


def text():
    with open(TOOL) as f:
        return f.read()


def function(name):
    """One shell function of tools/lumatv, from its `name() {` line to the closing brace in column 0 (a one-line function is that line)."""
    out, on = [], False
    for line in text().splitlines(True):
        if line.startswith(name + "() {"):
            if line.rstrip().endswith("}"):
                return line
            on = True
        if on:
            out.append(line)
            if line.rstrip() == "}":
                return "".join(out)
    raise AssertionError(name + " not found")


def constants():
    return [l for l in text().split("\n") if re.match(r"VPLANE_[A-Z]+=", l)]


FUNCTIONS = ("json_escape", "emit_error", "vplane_json", "vplane_installed", "vplane_install", "vplane_start", "vplane_stop", "vplane_remove", "verb_vplane")

# What the verb is asked to run, recording each call: the state is files under $FAKE.
FAKE_SH = """#!/bin/sh
echo "$*" >> "$FAKE/calls"
case "$1" in
  status) [ -e "$FAKE/silent" ] && exit 0
          i=false; r=false
          [ -e "$FAKE/installed" ] && i=true
          [ -e "$FAKE/role" ] && r=true
          echo '{"installed":'$i',"role":'$r',"clips":6,"running":false,"pid":0}' ;;
  install) if [ -e "$FAKE/install-fails" ]; then echo "vplane: cannot write under /var/luna-service2-dev" >&2; exit 1; fi
           : > "$FAKE/installed"; : > "$FAKE/role" ;;
  start) if [ -e "$FAKE/start-fails" ]; then echo "vplane: not installed" >&2; exit 1; fi ;;
  stop) : ;;
  remove) rm -f "$FAKE/installed" "$FAKE/role" ;;
esac
"""


class Base(unittest.TestCase):
    """A package directory laid out as the ipk lays it out (payload/vplane/...), a state directory and the tool's own functions and constants."""

    real = False

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)                  # registered first, so it runs last: the stop below still finds the script
        self.d = self.tmp.name
        self.app = os.path.join(self.d, "app")
        self.state = os.path.join(self.d, "state")
        pv = os.path.join(self.app, "payload", "vplane")
        os.makedirs(os.path.join(pv, "clips"))
        if self.real:
            shutil.copy(REAL_SH, os.path.join(pv, "vplane.sh"))
            shutil.copy(REAL_ROLE, os.path.join(pv, "role.json"))
            for f in os.listdir(REAL_CLIPS):
                shutil.copy(os.path.join(REAL_CLIPS, f), os.path.join(pv, "clips", f))
        else:
            self.write(os.path.join(pv, "vplane.sh"), FAKE_SH)
            self.write(os.path.join(pv, "role.json"), "{}\n")
            self.write(os.path.join(pv, "clips", "a.vpc"), "x")
        # the program: a stand-in that records the arguments it is started with
        self.player = os.path.join(pv, "lumatv-vplane")
        self.write(self.player, '#!/bin/sh\necho "$*" > "%s/player-args"\nexit 0\n' % self.d, 0o755)
        self.pv = pv
        self.ls2 = os.path.join(self.d, "ls2")
        self.bin = os.path.join(self.d, "bin")
        os.mkdir(self.bin)
        self.write(os.path.join(self.bin, "ls-control"), '#!/bin/sh\necho "$@" >> "%s/ls-control"\n' % self.d, 0o755)
        self.env = dict(os.environ, FAKE=self.d, LUMATV_LS2_DIR=self.ls2, LUMATV_VPLANE_RESTART_SLEEP="0.05", PATH=self.bin + os.pathsep + os.environ["PATH"])
        if self.real:
            self.addCleanup(subprocess.run, ["sh", os.path.join(pv, "vplane.sh"), "stop", self.state], env=self.env, capture_output=True, timeout=30)

    def write(self, path, body, mode=0o644):
        with open(path, "w") as f:
            f.write(body)
        os.chmod(path, mode)

    def mark(self, name):
        open(os.path.join(self.d, name), "w").close()

    def sh(self, body, lock=True, functions=FUNCTIONS):
        script = (f'APPDIR="{self.app}"\nSTATE="{self.state}"\nSHORT_ID="{BRAND["shortId"]}"\nAPP_ID="{BRAND["appId"]}"\nLOGFILE="{self.d}/log"\n'
                  + "\n".join(constants()) + "\n"
                  + f'take_mutation_lock() {{ {"return 0" if lock else "return 1"}; }}\n'
                  + f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + "".join(function(n) for n in functions) + body + "\n")
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60, env=self.env)
        return r.returncode, r.stdout.strip()

    def verb(self, word, lock=True):
        return self.sh('verb_vplane "%s"' % word, lock=lock)

    def calls(self):
        return self.lines("calls")

    def loglines(self):
        return self.lines("loglines")

    def lines(self, name):
        p = os.path.join(self.d, name)
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return f.read().split("\n")[:-1]

    def boot_line(self):
        """The line of verb_apply that starts the player, run on its own: the exit status it leaves is apply's to go on with."""
        lines = [l.strip() for l in function("verb_apply").splitlines() if "vplane_installed" in l]
        self.assertEqual(len(lines), 1, lines)
        return self.sh(lines[0] + "\necho rc=$?")

    def wait_for(self, pred, timeout=8.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if pred():
                return True
            time.sleep(0.02)
        return False


class Verb(Base):
    """`lumatv vplane ...` over a stand-in script: what it passes, what it answers, what it refuses."""

    def test_status_is_the_scripts_object_under_vplane(self):
        rc, out = self.verb("status")
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out), {"ok": True, "vplane": {"installed": False, "role": False, "clips": 6, "running": False, "pid": 0}})
        self.assertEqual(self.calls(), ["status " + self.state])

    def test_no_word_is_status(self):
        self.assertEqual(self.sh("verb_vplane")[0], 0)
        self.assertEqual(self.calls(), ["status " + self.state])

    def test_a_package_without_the_player_is_not_installed_and_nothing_is_run(self):
        os.remove(os.path.join(self.pv, "vplane.sh"))
        rc, out = self.verb("status")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "vplane": {"installed": False, "role": False, "clips": 0, "running": False, "pid": 0}}))
        for word in ("install", "start"):
            rc, out = self.verb(word)
            self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "vplane did not %s: this package has no video-plane player" % word}))
        self.assertEqual(self.calls(), [])

    def test_install_gives_the_script_the_package_files_and_the_state_and_answers_with_the_status(self):
        rc, out = self.verb("install")
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out), {"ok": True, "vplane": {"installed": True, "role": True, "clips": 6, "running": False, "pid": 0}})
        self.assertEqual(self.calls(), ["install %s %s %s %s" % (self.player, os.path.join(self.pv, "role.json"), os.path.join(self.pv, "clips"), self.state),
                                        "status " + self.state])

    def test_install_and_remove_wait_for_apply_and_revert_and_touch_nothing_meanwhile(self):
        for word in ("install", "remove"):
            rc, out = self.verb(word, lock=False)
            self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "another apply or revert is still running"}))
        self.assertEqual(self.calls(), [])

    def test_a_refused_install_says_why_in_the_answer_and_the_log(self):
        self.mark("install-fails")
        rc, out = self.verb("install")
        self.assertEqual(rc, 1)
        j = json.loads(out)
        self.assertFalse(j["ok"])
        self.assertEqual(j["error"], "vplane did not install: vplane: cannot write under /var/luna-service2-dev")
        self.assertEqual(self.loglines(), ["vplane install: vplane: cannot write under /var/luna-service2-dev"])

    def test_install_without_the_program_in_the_package_runs_nothing(self):
        os.remove(self.player)
        rc, out = self.verb("install")
        self.assertEqual((rc, json.loads(out)["error"]), (1, "vplane did not install: this package has no video-plane player"))
        self.assertEqual(self.calls(), [])

    def test_start_gives_the_script_the_state_the_mailbox_and_the_app_id(self):
        rc, out = self.verb("start")
        self.assertEqual(rc, 0)
        self.assertTrue(json.loads(out)["ok"])
        self.assertEqual(self.calls(), ["start %s %s %s" % (self.state, MAILBOX, APPID), "status " + self.state])
        self.assertEqual((MAILBOX, APPID), ("/tmp/lumatv-vplane", "org.nphil.lumatv.vplane"))

    def test_a_start_that_fails_says_why_and_is_logged(self):
        self.mark("start-fails")
        rc, out = self.verb("start")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "vplane did not start: vplane: not installed"}))
        self.assertEqual(self.loglines(), ["vplane: vplane: not installed"])

    def test_stop_and_remove_run_the_script_and_answer_with_the_status(self):
        self.mark("installed")
        self.mark("role")
        rc, out = self.verb("stop")
        self.assertEqual((rc, json.loads(out)["vplane"]["installed"]), (0, True))
        rc, out = self.verb("remove")
        self.assertEqual((rc, json.loads(out)["vplane"]["installed"]), (0, False))
        self.assertEqual(self.calls(), ["stop " + self.state, "status " + self.state, "remove " + self.state, "status " + self.state])

    def test_a_wrong_word_is_an_error_that_runs_nothing(self):
        rc, out = self.verb("sideways")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "vplane: install, start, stop, remove or status"}))
        self.assertEqual(self.calls(), [])

    def test_a_script_that_says_nothing_is_not_installed(self):
        self.mark("silent")
        self.mark("installed")
        self.mark("role")
        self.assertEqual(self.sh("vplane_installed")[0], 1)


class Boot(Base):
    """apply starts a player that is installed, boot included, and does nothing about one that is not. It never installs."""

    def test_an_installed_player_is_started(self):
        self.mark("installed")
        self.mark("role")
        rc, out = self.boot_line()
        self.assertEqual(out, "rc=0")
        self.assertEqual(self.calls(), ["status " + self.state, "start %s %s %s" % (self.state, MAILBOX, APPID)])

    def test_a_player_that_is_not_installed_is_left_alone(self):
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertEqual(self.calls(), ["status " + self.state])

    def test_a_staged_program_without_its_role_is_not_started(self):
        self.mark("installed")
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertEqual(self.calls(), ["status " + self.state])

    def test_a_role_without_the_staged_program_is_not_started_either(self):
        self.mark("role")
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertEqual(self.calls(), ["status " + self.state])

    def test_a_package_without_the_player_asks_nothing(self):
        os.remove(os.path.join(self.pv, "vplane.sh"))
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertEqual(self.calls(), [])

    def test_a_start_that_fails_is_logged_and_apply_goes_on(self):
        self.mark("installed")
        self.mark("role")
        self.mark("start-fails")
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertEqual(self.loglines(), ["vplane: vplane: not installed"])

    def test_whatever_the_state_apply_never_runs_install(self):
        for have in ((), ("installed",), ("role",), ("installed", "role")):
            for name in ("installed", "role", "start-fails", "calls", "loglines"):
                p = os.path.join(self.d, name)
                if os.path.exists(p):
                    os.remove(p)
            for name in have:
                self.mark(name)
            self.boot_line()
            self.assertFalse([c for c in self.calls() if c.startswith("install")], have)


class Wiring(unittest.TestCase):
    """Where the player is started, removed and written, in the tool's own text."""

    def test_the_role_is_written_by_the_owners_verb_and_nowhere_else(self):
        uses = [l for l in text().split("\n") if re.search(r"\bvplane_install\b", l) and not l.lstrip().startswith("#")]
        self.assertEqual(len(uses), 2, uses)                        # its definition, and its one use
        self.assertTrue(uses[0].startswith("vplane_install() {"))
        self.assertIn(uses[1], function("verb_vplane"))
        runs = [l for l in text().split("\n") if '"$VPLANE_SH" install' in l]
        self.assertEqual(len(runs), 1, runs)
        self.assertIn(runs[0], function("vplane_install"))
        # no other shell script that runs on the set (the boot hook, the compositor's, the restyles') knows the player at all; the package builder does
        for name in sorted(os.listdir(TOOLS)):
            if name in ("lumatv", "build-ipk.sh") or not name.endswith(".sh"):
                continue
            with open(os.path.join(TOOLS, name), encoding="utf-8") as f:
                code = [l for l in f.read().split("\n") if not l.lstrip().startswith("#")]
            self.assertEqual([l for l in code if "vplane" in l], [], name)

    def test_apply_starts_it_beside_the_colour_reader_before_the_screen_is_touched(self):
        apply_text = function("verb_apply")
        cap, vp, screen = apply_text.index("capture_start || true"), apply_text.index("vplane_installed"), apply_text.index("# 6. Bring the change on screen")
        self.assertLess(cap, vp)
        self.assertLess(vp, screen)
        self.assertIn("if vplane_installed; then vplane_start || true; fi", apply_text)

    def test_revert_removes_it_with_the_colour_reader(self):
        self.assertRegex(function("do_revert_internal"), r"\n  capture_remove\n  vplane_remove\n")

    def test_the_verb_is_dispatched_and_documented(self):
        self.assertIn('  vplane) verb_vplane "${2:-status}"; exit $? ;;', text())
        self.assertRegex(text(), r"\n#   vplane install\|start\|stop\|remove\|status ->")

    def test_nothing_in_it_restarts_a_service_or_kills_by_name(self):
        for name in FUNCTIONS[2:]:
            self.assertNotRegex(function(name), r"systemctl|restart|pkill|killall|restart_surface", name)

    def test_the_mailbox_is_the_directory_the_saver_writes_its_requests_to(self):
        with open(SAVER) as f:
            m = re.search(r'dirUrl:\s*"file://([^"]+)"', f.read())
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1).rstrip("/").replace("{{shortId}}", BRAND["shortId"]), MAILBOX)
        self.assertIn('VPLANE_MAILBOX="/tmp/$SHORT_ID-vplane"', text())

    def test_the_constants_name_the_files_the_package_stages(self):
        self.assertEqual(constants(), [
            'VPLANE_SH="$APPDIR/payload/vplane/vplane.sh"',
            'VPLANE_SRC="$APPDIR/payload/vplane/lumatv-vplane"',
            'VPLANE_ROLE="$APPDIR/payload/vplane/role.json"',
            'VPLANE_CLIPS="$APPDIR/payload/vplane/clips"',
            'VPLANE_MAILBOX="/tmp/$SHORT_ID-vplane"',
            'VPLANE_APPID="$APP_ID.vplane"',
        ])


class RealScript(Base):
    """The tool against the real payload/vplane/vplane.sh: the arguments it passes are the ones the script takes, and what the script reports is what
    the boot line reads. A temporary state and role directory (LUMATV_LS2_DIR), a stand-in program that records its arguments and exits."""

    real = True

    def state_files(self):
        return sorted(os.listdir(self.state)) if os.path.isdir(self.state) else []

    def test_an_uninstalled_player_stays_uninstalled_through_a_boot_and_says_so(self):
        rc, out = self.verb("status")
        self.assertEqual(json.loads(out)["vplane"], {"installed": False, "role": False, "clips": 0, "running": False, "pid": 0})
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertFalse(os.path.exists(self.ls2), "a boot wrote an LS2 role")
        self.assertFalse(os.path.exists(os.path.join(self.state, "bin")), "a boot staged the program")
        self.assertFalse(os.path.exists(os.path.join(self.d, "player-args")), "a boot started the program")

    def test_install_stages_the_program_and_clips_and_writes_the_role_for_the_staged_path(self):
        rc, out = self.verb("install")
        self.assertEqual(rc, 0, out)
        vp = json.loads(out)["vplane"]
        prints = len([f for f in os.listdir(REAL_CLIPS) if f.endswith(".vpc")])
        self.assertGreater(prints, 0)
        self.assertEqual((vp["installed"], vp["role"], vp["clips"], vp["running"]), (True, True, prints, False))
        staged = os.path.join(self.state, "bin", "lumatv-vplane")
        self.assertTrue(os.access(staged, os.X_OK))
        with open(os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")) as f:
            self.assertEqual(json.load(f)["exeName"], staged)
        self.assertTrue(os.path.exists(os.path.join(self.ls2, "manifests.d", "org.nphil.lumatv.vplane.json")))
        self.assertEqual(sorted(os.listdir(os.path.join(self.state, "vplane", "clips"))), sorted(os.listdir(REAL_CLIPS)))

    def test_a_boot_then_starts_the_staged_program_with_the_mailbox_the_clips_and_the_app_id(self):
        self.assertEqual(self.verb("install")[0], 0)
        self.assertEqual(self.boot_line()[1], "rc=0")
        self.assertTrue(self.wait_for(lambda: os.path.exists(os.path.join(self.d, "player-args"))), "the program was not started")
        with open(os.path.join(self.d, "player-args")) as f:
            args = f.read().split()
        self.assertEqual(args[:6], ["-d", MAILBOX, "-c", os.path.join(self.state, "vplane", "clips"), "-a", APPID])

    def test_start_by_hand_on_a_set_that_is_not_installed_is_refused_with_the_scripts_words(self):
        rc, out = self.verb("start")
        self.assertEqual((rc, json.loads(out)["error"]), (1, "vplane did not start: vplane: not installed"))

    def test_remove_takes_the_role_the_manifest_the_program_and_the_clips_away(self):
        self.assertEqual(self.verb("install")[0], 0)
        rc, out = self.verb("remove")
        self.assertEqual(rc, 0, out)
        self.assertEqual(json.loads(out)["vplane"], {"installed": False, "role": False, "clips": 0, "running": False, "pid": 0})
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "roles.d", "lumatv-vplane.json")))
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "manifests.d", "org.nphil.lumatv.vplane.json")))
        self.assertFalse(os.path.exists(os.path.join(self.state, "bin", "lumatv-vplane")))
        self.assertFalse(os.path.exists(os.path.join(self.state, "vplane", "clips")))

    def test_revert_removes_what_the_owner_installed(self):
        self.assertEqual(self.verb("install")[0], 0)
        self.assertEqual(self.sh("vplane_remove\nvplane_json", functions=FUNCTIONS)[1], '{"installed":false,"role":false,"clips":0,"running":false,"pid":0}')


if __name__ == "__main__":
    unittest.main()
