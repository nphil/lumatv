"""Run with: python3 -m unittest tests.test_inputcommon_stage

tools/inputcommon.sh, the staging and the overlay mount of the patched input banners: against a copy of LG's tree in a
temporary directory, with `mount` and `umount` stood in for by scripts that record what they were asked and edit a fake
/proc/mounts. What it must do: stage only the files the patch rewrote, leave LG's tree and the last good stage alone when the
patch refuses, mount nothing from a failed stage, put the unmount before the stage (a copy of an already-patched tree is
refused), and drop the process snapshot on both `on` and `off` and only the one that is inputcommon's.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, "tools", "inputcommon.sh")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/inputcommon/qml", "/tmp/tvsrc/inputcommon/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/inputcommon/qml")
BANNER = "UserInterfaceLayer/Containers/MainViewLayer/"
STYLE = "UserInterfaceLayer/GlobalStyle.qml"
TOUCHED = sorted(["UserInterfaceLayer/GlobalStyle.qml", BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml",
                  BANNER + "InputBanner/DetailInfoBanner.qml", BANNER + "BannerItems/OptionButton.qml", BANNER + "BannerItems/BannerIcon.qml",
                  BANNER + "BannerItems/NsuProgressBar.qml"])

FAKE_MOUNT = """#!/bin/sh
# mount -t overlay inputcommon -o OPTIONS TARGET
echo "mount $*" >> "$FAKE_LOG"
[ -n "${FAKE_MOUNT_FAILS:-}" ] && exit 1
for last; do :; done
echo "inputcommon $last overlay rw,relatime 0 0" >> "$PROC_MOUNTS"
"""
FAKE_UMOUNT = """#!/bin/sh
echo "umount $*" >> "$FAKE_LOG"
grep -v " $1 overlay " "$PROC_MOUNTS" > "$PROC_MOUNTS.tmp"
mv "$PROC_MOUNTS.tmp" "$PROC_MOUNTS"
"""


@unittest.skipUnless(os.path.isdir(CENSUS) and shutil.which("python3"), "needs the census copy of LG's inputcommon tree")
class Staging(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.qml = os.path.join(t, "lg", "qml")
        shutil.copytree(CENSUS, self.qml)
        self.state = os.path.join(t, "state")
        self.checkpoints = os.path.join(t, "checkpoint")
        for d in ("com.webos.app.inputcommon", "com.webos.app.home"):
            os.makedirs(os.path.join(self.checkpoints, d))
        self.mounts = os.path.join(t, "mounts")
        open(self.mounts, "w").close()
        self.log = os.path.join(t, "calls")
        for name, body in (("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT)):
            path = os.path.join(t, name)
            with open(path, "w") as f:
                f.write(body)
            os.chmod(path, 0o755)
        self.env = dict(os.environ, APPDIR=ROOT, STATE=self.state, INPUTCOMMON_QML=self.qml, THEME_SRC=os.path.join(ROOT, "payload", "theme"),
                        THEME_DIR=os.path.join(t, "theme"), FONTS_DIR=os.path.join(ROOT, "payload", "fonts"), CHECKPOINTS=self.checkpoints,
                        PROC_MOUNTS=self.mounts, MOUNT=os.path.join(t, "mount"), UMOUNT=os.path.join(t, "umount"), FAKE_LOG=self.log)

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, verb, **env):
        r = subprocess.run(["sh", SCRIPT, verb], env=dict(self.env, **env), capture_output=True, text=True)
        out = json.loads(r.stdout.strip().splitlines()[-1])
        return r.returncode, out

    def calls(self):
        try:
            with open(self.log) as f:
                return f.read().splitlines()
        except OSError:
            return []

    def staged(self):
        up = os.path.join(self.state, "inputcommon", "upper")
        return sorted(os.path.relpath(os.path.join(b, n), up) for b, _, ns in os.walk(up) for n in ns) if os.path.isdir(up) else []

    def upper(self, rel):
        with open(os.path.join(self.state, "inputcommon", "upper", rel)) as f:
            return f.read()

    def snapshot_marked(self, name):
        return os.path.exists(os.path.join(self.checkpoints, name, "invalid_img"))

    def test_stage_keeps_only_the_files_the_patch_rewrote_and_leaves_LGs_tree_alone(self):
        rc, out = self.run_script("stage")
        self.assertEqual((rc, out["ok"], out["staged"]), (0, True, True), out)
        self.assertEqual(self.staged(), TOUCHED)
        self.assertTrue(self.upper(STYLE).startswith("// Luma: patched"))
        # LG's directory is as it was, and nothing was mounted or unmounted
        r = subprocess.run(["diff", "-r", CENSUS, self.qml], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout[:400])
        self.assertEqual(self.calls(), [])
        self.assertFalse(os.path.exists(os.path.join(self.state, "inputcommon", "full.new")))

    def test_a_tree_LG_changed_is_refused_and_the_last_good_stage_stays(self):
        self.assertEqual(self.run_script("stage")[0], 0)
        before = self.upper(STYLE)
        path = os.path.join(self.qml, BANNER + "BannerItems/NsuProgressBar.qml")
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("target: bg", "target: backdrop"))
        rc, out = self.run_script("stage")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("LG's banners are in place", out["error"])
        self.assertEqual(self.staged(), TOUCHED)
        self.assertEqual(self.upper(STYLE), before)
        with open(os.path.join(self.state, "log")) as f:
            self.assertIn("NsuProgressBar.qml: anchor found 0 times", f.read())

    def test_on_unmounts_first_stages_from_LGs_tree_mounts_and_drops_only_inputcommons_snapshot(self):
        with open(self.mounts, "w") as f:
            f.write("inputcommon %s overlay rw,relatime 0 0\n" % self.qml)       # a mount left by the last run
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["inputcommon"], out["snapshotsDropped"]), (0, True, "on", 1), out)
        calls = self.calls()
        self.assertEqual([c.split()[0] for c in calls], ["umount", "mount"])
        self.assertIn("lowerdir=%s,upperdir=%s,workdir=%s" % (self.qml, os.path.join(self.state, "inputcommon", "upper"), os.path.join(self.state, "inputcommon", "work")), calls[1])
        self.assertTrue(calls[1].endswith(" " + self.qml))
        self.assertIn("-t overlay", calls[1])
        self.assertEqual(self.staged(), TOUCHED)
        self.assertTrue(self.snapshot_marked("com.webos.app.inputcommon"))
        self.assertFalse(self.snapshot_marked("com.webos.app.home"))
        rc, out = self.run_script("status")
        self.assertEqual((out["staged"], out["mounted"], out["qml"]), (True, True, self.qml))

    def test_a_stage_the_patch_refuses_mounts_nothing_and_leaves_the_snapshot(self):
        path = os.path.join(self.qml, STYLE)
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("property int inputSimpleMinWidth: 582", "property int inputSimpleMinWidth: 600"))
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertEqual([c.split()[0] for c in self.calls()], [])
        self.assertFalse(self.snapshot_marked("com.webos.app.inputcommon"))
        self.assertEqual(self.staged(), [])

    def test_a_mount_that_fails_leaves_LGs_banners_and_the_snapshot(self):
        rc, out = self.run_script("on", FAKE_MOUNT_FAILS="1")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("LG's banners are in place", out["error"])
        self.assertFalse(self.snapshot_marked("com.webos.app.inputcommon"))
        self.assertEqual(self.run_script("status")[1]["mounted"], False)

    def test_an_overlay_that_will_not_unmount_stops_everything(self):
        with open(self.mounts, "w") as f:
            f.write("inputcommon %s overlay rw,relatime 0 0\n" % self.qml)
        with open(self.env["UMOUNT"], "w") as f:
            f.write("#!/bin/sh\necho \"umount $*\" >> \"$FAKE_LOG\"\nexit 1\n")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("could not unmount", out["error"])
        self.assertEqual([c.split()[0] for c in self.calls()], ["umount"])
        self.assertEqual(self.staged(), [])

    def test_off_unmounts_and_drops_the_snapshot_and_stages_nothing(self):
        self.assertEqual(self.run_script("on")[0], 0)
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"], out["inputcommon"]), (0, True, "off"), out)
        self.assertEqual(self.run_script("status")[1]["mounted"], False)
        self.assertEqual([c.split()[0] for c in self.calls()], ["mount", "umount"])

    def test_a_stage_while_the_overlay_is_mounted_is_refused(self):
        with open(self.mounts, "w") as f:
            f.write("inputcommon %s overlay rw,relatime 0 0\n" % self.qml)
        rc, out = self.run_script("stage")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("unmount it first", out["error"])

    def test_no_qml_directory_is_an_error_not_a_guess(self):
        rc, out = self.run_script("on", INPUTCOMMON_QML=os.path.join(self.tmp.name, "nowhere"))
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("was not found", out["error"])
        self.assertEqual(self.calls(), [])

    def test_usage(self):
        rc, out = self.run_script("bogus")
        self.assertEqual((rc, out["ok"]), (1, False))


if __name__ == "__main__":
    unittest.main()
