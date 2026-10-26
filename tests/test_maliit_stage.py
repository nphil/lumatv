"""Run with: python3 -m unittest tests.test_maliit_stage

tools/maliit.sh, the staging and the overlay mount of the patched keyboard keys and the restart of the server that makes it read them.

It runs against a copy of LG's maliit tree in a temporary directory (the census copy, else the archive in the repository), with `mount`,
`umount` and `systemctl` stood in for by scripts that record what they were asked and edit a fake /proc/mounts, a unit that is active or
not, and a system log the server writes to. What it must do: stage only the files the patch rewrote, leave LG's tree and the last good stage
alone when the patch refuses, mount nothing from a failed stage, unmount before the stage (a copy of an already-patched tree is refused),
restart `maliit-server.service` and nothing else, only when it is running, and take the mount away again, with the server restarted on LG's
QML, when the server does not come back or logs a file of the keys it could not load. It never speaks to SAM (`luna-send`) or to any other unit.
"""
import atexit
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, "tools", "maliit.sh")
ARCHIVE = os.path.join(ROOT, "docs", "research", "round9", "r11-2026-10-26", "maliit-qml.tgz")
UNIT = "maliit-server.service"


def census():
    p = "/data/home/lgtv/uiinv/maliit/qml"
    if os.path.isdir(p):
        return p
    if not os.path.isfile(ARCHIVE):
        return None
    d = tempfile.mkdtemp(prefix="maliit-census-")
    atexit.register(shutil.rmtree, d, True)
    with tarfile.open(ARCHIVE) as t:
        t.extractall(d)
    return os.path.join(d, "qml")


CENSUS = census()

FAKE_MOUNT = """#!/bin/sh
# mount -t overlay maliit -o OPTIONS TARGET
echo "mount $*" >> "$FAKE_LOG"
[ -e "$FAKE/mount-fails" ] && exit 1
for last; do :; done
echo "maliit $last overlay rw,relatime 0 0" >> "$PROC_MOUNTS"
"""
FAKE_UMOUNT = """#!/bin/sh
echo "umount $*" >> "$FAKE_LOG"
[ -e "$FAKE/umount-fails" ] && exit 1
grep -v " $1 overlay " "$PROC_MOUNTS" > "$PROC_MOUNTS.tmp"
mv "$PROC_MOUNTS.tmp" "$PROC_MOUNTS"
"""
# systemctl, as the script calls it: `is-active UNIT`, `try-restart UNIT` and, to bring up a unit that a rollback finds failed, `reset-failed` and `restart`. The unit is active when $FAKE/unit says so; a restart records
# itself and, as the files in $FAKE ask, leaves the unit failed, writes lines to the log the server writes to, or fails.
FAKE_SYSTEMCTL = """#!/bin/sh
echo "systemctl $*" >> "$FAKE_LOG"
case "$1" in
  is-active) s=$(cat "$FAKE/unit" 2>/dev/null); echo "${s:-inactive}"; [ "$s" = active ] ;;
  try-restart)
    [ "$2" = "$UNIT" ] || exit 5
    [ -e "$FAKE/restart-fails" ] && exit 1
    [ "$(cat "$FAKE/unit" 2>/dev/null)" = active ] || exit 0
    echo restarted >> "$FAKE/restarts"
    if [ -e "$FAKE/dies-on-patched" ] && grep -q " overlay " "$PROC_MOUNTS"; then echo failed > "$FAKE/unit"; fi
    if [ -s "$FAKE/server-says" ] && grep -q " overlay " "$PROC_MOUNTS"; then cat "$FAKE/server-says" >> "$MALIIT_MESSAGES"; fi
    ;;
  reset-failed) [ "$2" = "$UNIT" ] ;;
  restart)
    [ "$2" = "$UNIT" ] || exit 5
    echo restarted >> "$FAKE/restarts"
    echo active > "$FAKE/unit"
    if [ -e "$FAKE/dies-on-patched" ] && grep -q " overlay " "$PROC_MOUNTS"; then echo failed > "$FAKE/unit"; fi
    ;;
  *) exit 5 ;;
esac
"""
FAKE_LUNA = "#!/bin/sh\necho \"luna-send $*\" >> \"$FAKE_LOG\"\n"
STOCK_NOISE = ("2026-10-26T08:05:21Z [1.0] user.warning MaliitServer [] IME default file:///usr/lib/maliit/plugins/qml/keyboard-common/ButtonBase.qml:30:5: "
               "Unable to assign [undefined] to int\n"
               "2026-10-26T08:05:21Z [1.0] user.warning MaliitServer [] IME default file:///usr/lib/maliit/plugins/qml/keyboard-common/PredictionPanel.qml:128: "
               "ReferenceError: fontfamily is not defined\n")
LOAD_ERROR = ("2026-10-26T08:05:21Z [1.0] user.warning MaliitServer [] IME default file:///usr/lib/maliit/plugins/qml/view-global/main.qml:189:5: "
              "Type NormalKeyboard unavailable\n")


def tree_state(root):
    out = {}
    for base, dirs, files in os.walk(root):
        for n in dirs + files:
            p = os.path.join(base, n)
            rel = os.path.relpath(p, root)
            if os.path.isdir(p):
                out[rel] = ("dir",)
            else:
                with open(p, "rb") as f:
                    out[rel] = ("file", hash(f.read()))
    return out


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.t = t
        self.qml = os.path.join(t, "lg", "plugins", "qml")
        self.make_qml()
        self.state = os.path.join(t, "state")
        self.fake = os.path.join(t, "fake")
        os.makedirs(self.fake)
        self.bin = os.path.join(t, "bin")
        os.makedirs(self.bin)
        self.mounts = os.path.join(t, "mounts")
        open(self.mounts, "w").close()
        self.log = os.path.join(t, "calls")
        self.messages = os.path.join(t, "messages")
        with open(self.messages, "w") as f:
            f.write("2026-10-26T08:00:00Z [0.5] user.info sam [] SAM APP_LAUNCH old line\n")
        for name, body in (("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT), ("systemctl", FAKE_SYSTEMCTL), ("luna-send", FAKE_LUNA)):
            with open(os.path.join(self.bin, name), "w") as f:
                f.write(body)
            os.chmod(os.path.join(self.bin, name), 0o755)
        self.set_unit("active")

    def tearDown(self):
        self.tmp.cleanup()

    def make_qml(self):
        shutil.copytree(CENSUS, self.qml)

    def set_unit(self, state):
        with open(os.path.join(self.fake, "unit"), "w") as f:
            f.write(state)

    def mark(self, name, text=""):
        with open(os.path.join(self.fake, name), "w") as f:
            f.write(text)

    def env(self, **extra):
        e = dict(os.environ, APPDIR=ROOT, STATE=self.state, MALIIT_QML=self.qml, FONTS_DIR=os.path.join(ROOT, "payload", "fonts"),
                 THEME_DIR=os.path.join(self.t, "no-theme"), PROC_MOUNTS=self.mounts, MOUNT=os.path.join(self.bin, "mount"),
                 UMOUNT=os.path.join(self.bin, "umount"), SYSTEMCTL=os.path.join(self.bin, "systemctl"), MALIIT_MESSAGES=self.messages,
                 MALIIT_SETTLE="1", MALIIT_STEP="0", FAKE=self.fake, FAKE_LOG=self.log, UNIT=UNIT, LOGFILE=os.path.join(self.state, "log"),
                 PATH=self.bin + os.pathsep + os.environ["PATH"])
        e.update(extra)
        return e

    def run_script(self, verb, **env):
        r = subprocess.run(["sh", SCRIPT, verb], env=self.env(**env), capture_output=True, text=True, timeout=120)
        out = r.stdout.strip()
        try:
            doc = json.loads(out)
        except ValueError:
            self.fail("not one JSON object: %r (stderr %r)" % (out, r.stderr[-400:]))
        return r.returncode, doc

    def calls(self):
        try:
            with open(self.log) as f:
                return f.read().splitlines()
        except FileNotFoundError:
            return []

    def upper(self):
        return os.path.join(self.state, "maliit", "upper")

    def mounted(self):
        with open(self.mounts) as f:
            return " overlay " in f.read()


@unittest.skipUnless(CENSUS and os.path.isdir(CENSUS), "needs the census copy of LG's maliit tree or its archive")
class Staging(Base):
    def test_stage_keeps_only_the_rewritten_files_and_leaves_LGs_tree_alone(self):
        before = tree_state(self.qml)
        rc, doc = self.run_script("stage")
        self.assertEqual((rc, doc["ok"], doc["staged"]), (0, True, True), doc)
        self.assertEqual(tree_state(self.qml), before)
        names = sorted(os.path.relpath(os.path.join(b, n), self.upper()) for b, _, fs in os.walk(self.upper()) for n in fs)
        self.assertEqual(len(names), 38)
        self.assertIn("keyboard-common/style/Style.qml", names)
        self.assertIn("keyboard-common/FocusPanel.qml", names)
        self.assertNotIn("view-global/main.qml", names)
        self.assertEqual(self.calls(), [])                              # nothing mounted, nothing restarted

    def test_a_stage_over_a_mounted_overlay_is_refused_because_it_would_copy_our_own_files(self):
        with open(self.mounts, "w") as f:
            f.write("maliit %s overlay rw 0 0\n" % self.qml)
        rc, doc = self.run_script("stage")
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("unmount it first", doc["error"])

    def test_a_patch_that_refuses_leaves_the_last_good_stage_nothing_mounted_and_nothing_restarted(self):
        self.assertEqual(self.run_script("stage")[0], 0)
        good = tree_state(self.upper())
        path = os.path.join(self.qml, "keyboard-common", "FocusPanel.qml")
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("layer.enabled: true", "layer.enabled: yes"))
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False), doc)
        self.assertIn("could not stage", doc["error"])
        self.assertEqual(tree_state(self.upper()), good)
        self.assertFalse(self.mounted())
        self.assertEqual([c for c in self.calls() if c.startswith(("mount", "systemctl try-restart"))], [])

    def test_on_unmounts_then_stages_then_mounts_then_restarts_the_one_unit_and_says_so(self):
        with open(self.mounts, "w") as f:
            f.write("maliit %s overlay rw 0 0\n" % self.qml)          # a stale layer of ours, as after a crashed run
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc), (0, {"ok": True, "keys": "on", "restarted": True, "unit": "active"}))
        calls = self.calls()
        kinds = [c.split()[0] + (" " + c.split()[1] if c.startswith("systemctl") else "") for c in calls]
        self.assertEqual(kinds[0], "umount")
        self.assertEqual(kinds[1], "mount")
        self.assertEqual([c for c in calls if c.startswith("systemctl try-restart")], ["systemctl try-restart " + UNIT])
        self.assertLess(kinds.index("mount"), kinds.index("systemctl try-restart"))
        self.assertTrue(self.mounted())
        self.assertTrue(os.path.exists(os.path.join(self.upper(), "keyboard-common", "style", "Style.qml")))

    def test_the_only_thing_it_ever_restarts_is_the_keyboards_unit_and_it_never_speaks_to_SAM(self):
        self.run_script("on")
        self.run_script("off")
        for c in self.calls():
            self.assertRegex(c, r"^(mount|umount|systemctl (is-active|try-restart|reset-failed|restart) %s)" % UNIT.replace(".", r"\."), c)
        self.assertFalse([c for c in self.calls() if c.startswith("luna-send")])
        with open(SCRIPT) as f:
            code = "\n".join(l for l in f.read().splitlines() if not l.lstrip().startswith("#"))
        for word in ("luna-send", "closeByAppId", "sam.service", "kill ", "pkill", "killall", "systemctl stop", "systemctl start", "daemon-reload"):
            self.assertNotIn(word, code)

    def test_a_unit_that_is_not_running_is_left_not_running_and_the_server_reads_the_mount_when_it_starts(self):
        self.set_unit("inactive")
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc), (0, {"ok": True, "keys": "on", "restarted": False, "unit": "inactive"}))
        self.assertEqual([c for c in self.calls() if "try-restart" in c], [])
        self.assertTrue(self.mounted())

    def test_a_failed_stage_or_mount_restarts_nothing_and_a_mount_that_will_not_come_off_stops_everything(self):
        self.mark("mount-fails")
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("could not mount", doc["error"])
        self.assertEqual([c for c in self.calls() if "try-restart" in c], [])
        os.unlink(os.path.join(self.fake, "mount-fails"))
        with open(self.mounts, "w") as f:
            f.write("maliit %s overlay rw 0 0\n" % self.qml)
        self.mark("umount-fails")
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("could not unmount", doc["error"])
        self.assertEqual([c for c in self.calls() if "try-restart" in c], [])

    def test_a_server_that_does_not_come_back_gets_the_mount_taken_away_and_is_restarted_on_LGs_QML(self):
        self.mark("dies-on-patched")
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False), doc)
        self.assertIn("is failed", doc["error"])
        self.assertIn("brought up on LG's", doc["error"])
        self.assertFalse(self.mounted())
        calls = self.calls()
        # the unit is failed, which a try-restart would leave alone: reset and restart, and only after the mount is gone
        self.assertEqual([c for c in calls if c.startswith("systemctl try-restart")], ["systemctl try-restart " + UNIT])
        self.assertEqual(calls[-2:], ["systemctl reset-failed " + UNIT, "systemctl restart " + UNIT])
        self.assertLess(calls.index("umount " + self.qml), calls.index("systemctl reset-failed " + UNIT))
        with open(os.path.join(self.fake, "unit")) as f:
            self.assertEqual(f.read().strip(), "active")

    def test_a_server_that_logs_a_keyboard_file_it_could_not_load_gets_the_same(self):
        self.mark("server-says", LOAD_ERROR)
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False), doc)
        self.assertIn("could not load a QML file", doc["error"])
        self.assertIn("Type NormalKeyboard unavailable", doc["error"])
        self.assertIn("brought up on LG's", doc["error"])
        self.assertFalse(self.mounted())
        self.assertEqual(len([c for c in self.calls() if c.startswith("systemctl try-restart")]), 2)

    def test_LGs_own_warnings_in_the_log_are_not_a_failure_and_neither_are_lines_from_before_the_restart(self):
        with open(self.messages, "a") as f:
            f.write(LOAD_ERROR)                                         # from an earlier run of the stock server: before the restart
        self.mark("server-says", STOCK_NOISE)
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (0, True), doc)
        self.assertTrue(self.mounted())

    def test_a_restart_that_fails_is_a_failure_too(self):
        self.mark("restart-fails")
        rc, doc = self.run_script("on")
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("try-restart", doc["error"])
        self.assertFalse(self.mounted())

    def test_off_unmounts_and_restarts_only_when_there_was_something_to_undo(self):
        self.assertEqual(self.run_script("on")[0], 0)
        open(self.log, "w").close()
        rc, doc = self.run_script("off")
        self.assertEqual((rc, doc), (0, {"ok": True, "keys": "off", "restarted": True, "unit": "active"}))
        self.assertFalse(self.mounted())
        self.assertEqual([c for c in self.calls() if "try-restart" in c], ["systemctl try-restart " + UNIT])
        open(self.log, "w").close()
        rc, doc = self.run_script("off")
        self.assertEqual((rc, doc), (0, {"ok": True, "keys": "off", "restarted": False, "unit": "active"}))
        self.assertEqual([c for c in self.calls() if "try-restart" in c], [])

    def test_status_says_what_is_staged_mounted_and_running(self):
        rc, doc = self.run_script("status")
        self.assertEqual((rc, doc["staged"], doc["mounted"], doc["unit"], doc["qml"]), (0, False, False, "active", self.qml))
        self.run_script("on")
        rc, doc = self.run_script("status")
        self.assertEqual((doc["staged"], doc["mounted"]), (True, True))
        self.set_unit("inactive")
        self.assertEqual(self.run_script("status")[1]["unit"], "inactive")

    def test_no_qml_directory_and_a_wrong_word_are_errors(self):
        shutil.rmtree(self.qml)
        for verb in ("on", "off", "stage"):
            rc, doc = self.run_script(verb)
            self.assertEqual((rc, doc["ok"]), (1, False), verb)
            self.assertIn("MALIIT_QML", doc["error"])
        rc, doc = self.run_script("sideways")
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("usage", doc["error"])


@unittest.skipUnless(CENSUS and os.path.isdir(CENSUS), "needs the census copy of LG's maliit tree or its archive")
class Candidates(Base):
    """Where LG's QML is looked for when nothing overrides it."""

    def test_the_default_place_is_the_one_copied_from_the_set(self):
        with open(SCRIPT) as f:
            text = f.read()
        self.assertIn('MT_CANDIDATES="/usr/lib/maliit/plugins/qml"', text)
        self.assertIn("MT_ROOT_FILE=view-global/main.qml", text)


if __name__ == "__main__":
    unittest.main()
