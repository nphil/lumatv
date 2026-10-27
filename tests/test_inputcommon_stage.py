"""Run with: python3 -m unittest tests.test_inputcommon_stage

tools/inputcommon.sh, the staging and the overlay mount of the patched input banners and the cold start that makes them show.

`Staging` runs against a copy of LG's tree in a temporary directory (the census copy; skipped when it is not on this workstation),
with `mount` and `umount` stood in for by scripts that record what they were asked and edit a fake /proc/mounts. What it must do:
stage only the files the patch rewrote, leave LG's tree and the last good stage alone when the patch refuses, mount nothing from a
failed stage, put the unmount before the stage (a copy of an already-patched tree is refused), and set LG's dump aside on `on`.

`ColdStart` runs the same script with a stand-in for the patch, a CRIU working directory shaped like LG's (page files that are
symlinks into the persisted copy), a persisted copy that must come out byte for byte as it went in, and a `luna-send` that plays SAM:
LG's dump is renamed in the working directory (never deleted, the persisted copy never touched), the input apps that are running
and not on screen are closed through SAM and only after the dump is aside, the one on screen is left, `off` puts everything back.

Both run with a fake /proc, a stand-in `kill` and waits of no time (IC_STEP=0): the resident-app part of the script (the Search app's
process, ended after the close) is tests.test_voice_stage's; here the input banners' run must never signal a process.
"""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, "tools", "inputcommon.sh")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/inputcommon/qml", "/tmp/tvsrc/inputcommon/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/inputcommon/qml")
BANNER = "UserInterfaceLayer/Containers/MainViewLayer/"
STYLE = "UserInterfaceLayer/GlobalStyle.qml"
BG = "UserInterfaceLayer/Containers/BgViewLayer/"
TOUCHED = sorted(["UserInterfaceLayer/GlobalStyle.qml", BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml",
                  BANNER + "InputBanner/DetailInfoBanner.qml", BANNER + "BannerItems/OptionButton.qml", BANNER + "BannerItems/BannerIcon.qml",
                  BANNER + "BannerItems/NsuProgressBar.qml", BANNER + "ChannelBanner/ZappingBanner.qml", BANNER + "ChannelBanner/ChannelBanner.qml",
                  BANNER + "ChannelBanner/DetailInfoBanner.qml", BANNER + "BannerItems/BannerEditItem.qml", BANNER + "BannerEditView.qml",
                  "UserInterfaceLayer/Components/SimpleButton.qml", "UserInterfaceLayer/Containers/PopupLayer/AlertPopupView.qml",
                  BG + "Screensaver.qml", BG + "ScreensaverMainText.qml", BG + "ScreensaverSubText.qml",
                  "UserInterfaceLayer/Components/LumaStyle.qml"])
DUMP = "com.webos.app.inputcommon"
ASIDE = ".lumatv-aside." + DUMP

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
# kill, as tools/inputcommon.sh sends it: kill -SIGNAL PID. A process ends by its directory leaving the fake /proc and SAM drops it from
# the running list when an answer for after the kill is there; $FAKE_SAM/ignore-term makes SIGTERM do nothing, immortal every signal.
# The calls go to the SAM log, where the order of a close and a kill can be read, and to the mount log.
FAKE_KILL = """#!/bin/sh
sig=${1#-}; pid=$2
echo "kill $sig $pid" >> "$FAKE_SAM/calls"
echo "kill $sig $pid" >> "$FAKE_LOG"
[ -d "$PROC_ROOT/$pid" ] || exit 1
[ -e "$FAKE_SAM/immortal" ] && exit 0
[ "$sig" = TERM ] && [ -e "$FAKE_SAM/ignore-term" ] && exit 0
rm -rf "$PROC_ROOT/$pid"
[ -f "$FAKE_SAM/running-after-kill" ] && cp "$FAKE_SAM/running-after-kill" "$FAKE_SAM/running"
exit 0
"""
# SAM, as luna-send reaches it: luna-send -n 1 luna://com.webos.applicationManager/METHOD PAYLOAD. The answers come from files in
# $FAKE_SAM; a close records what the working directory held at that moment.
FAKE_LUNA = """#!/bin/sh
uri=$3; payload=$4
method=${uri##*/}
echo "$method $payload" >> "$FAKE_SAM/calls"
[ -e "$FAKE_SAM/hangs" ] && exec sleep 30
case "$method" in
  running) cat "$FAKE_SAM/running" 2>/dev/null ;;
  getForegroundAppInfo) printf '{\\n "returnValue": true,\\n "foregroundAppInfo": [ { "appId": "%s" } ]\\n}\\n' "$(cat "$FAKE_SAM/foreground" 2>/dev/null)" ;;
  closeByAppId)
    echo "close $payload while $CRIU_TMP held: $(ls -A "$CRIU_TMP" 2>/dev/null | tr '\\n' ' ')" >> "$FAKE_SAM/closes"
    if [ -e "$FAKE_SAM/close-fails" ]; then echo '{"returnValue":false,"errorText":"not running"}'; else echo '{"returnValue":true}'; fi ;;
esac
"""
FAKE_TOKENS = "#!/usr/bin/env python3\nprint('{}')\n"
# A stand-in for the patch: "rewrites" one file of the copy it is given and prints what it rewrote.
FAKE_PATCH = """#!/usr/bin/env python3
import sys
p = sys.argv[1] + "/UserInterfaceLayer/GlobalStyle.qml"
open(p, "a").write("// Luma: patched\\n")
print("UserInterfaceLayer/GlobalStyle.qml")
"""


def tree_state(root):
    """Every path under root with its content hash or link target: what 'untouched' means."""
    out = {}
    for base, dirs, files in os.walk(root):
        for n in dirs + files:
            p = os.path.join(base, n)
            rel = os.path.relpath(p, root)
            if os.path.islink(p):
                out[rel] = ("link", os.readlink(p))
            elif os.path.isdir(p):
                out[rel] = ("dir",)
            else:
                with open(p, "rb") as f:
                    out[rel] = ("file", hashlib.sha256(f.read()).hexdigest())
    return out


class Base(unittest.TestCase):
    """The set around the script: LG's QML directory, the mount table, CRIU's directories and SAM."""

    qml_source = None

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.t = t
        self.qml = os.path.join(t, "lg", "qml")
        self.make_qml()
        self.state = os.path.join(t, "state")
        # CRIU's working directory, as LG's census shows it: the dump of inputcommon with its page file a symlink into the persisted
        # copy, and the Home stub's dump beside it; the persisted copy is the same dumps, complete.
        self.criu = os.path.join(t, "tmp-checkpoint")
        self.persisted = os.path.join(t, "mnt-checkpoint")
        for d in (DUMP, "com.webos.app.home"):
            os.makedirs(os.path.join(self.persisted, d))
            for f, text in (("pgid", "2513\n"), ("MD5SUM", "x\n"), ("core-2513.img", "core of " + d), ("pages-1.img.ori", "pages of " + d)):
                with open(os.path.join(self.persisted, d, f), "w") as fh:
                    fh.write(text)
            os.symlink(os.path.join(self.persisted, d, "pages-1.img.ori"), os.path.join(self.persisted, d, "pages-1.img"))
            os.makedirs(os.path.join(self.criu, d))
            for f, text in (("pgid", "2513\n"), ("MD5SUM", "x\n"), ("core-2513.img", "core of " + d)):
                with open(os.path.join(self.criu, d, f), "w") as fh:
                    fh.write(text)
            os.symlink(os.path.join(self.persisted, d, "pages-1.img.ori"), os.path.join(self.criu, d, "pages-1.img"))
        os.makedirs(os.path.join(self.criu, "error"))
        self.persisted_before = tree_state(self.persisted)
        self.mounts = os.path.join(t, "mounts")
        open(self.mounts, "w").close()
        self.log = os.path.join(t, "calls")
        self.sam = os.path.join(t, "sam")
        os.makedirs(self.sam)
        self.proc = os.path.join(t, "proc")
        os.makedirs(self.proc)
        self.write("sam/foreground", "com.webos.app.home")
        self.write("sam/running", '{"running":[{"id":"com.webos.app.home","processid":"17483"},{"id":"netflix","processid":"17580"}],"returnValue":true}')
        for name, body in (("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT), ("luna-send", FAKE_LUNA), ("kill", FAKE_KILL)):
            path = os.path.join(t, name)
            with open(path, "w") as f:
                f.write(body)
            os.chmod(path, 0o755)
        self.env = dict(os.environ, APPDIR=self.appdir(), STATE=self.state, INPUTCOMMON_QML=self.qml, THEME_SRC=os.path.join(ROOT, "payload", "theme"),
                        THEME_DIR=os.path.join(t, "theme"), FONTS_DIR=os.path.join(ROOT, "payload", "fonts"),
                        CRIU_TMP=self.criu, CRIU_RESTORING=os.path.join(t, "criu_restoring"), CRIU_ENABLE=os.path.join(t, "criu_enable"), RESTORE_WAIT="1",
                        LUNA_SEND=os.path.join(t, "luna-send"), SAM_TIMEOUT="1", FAKE_SAM=self.sam,
                        PROC_MOUNTS=self.mounts, MOUNT=os.path.join(t, "mount"), UMOUNT=os.path.join(t, "umount"), FAKE_LOG=self.log,
                        PROC_ROOT=self.proc, KILL=os.path.join(t, "kill"), IC_STEP="0")

    def tearDown(self):
        self.tmp.cleanup()

    def make_qml(self):
        raise NotImplementedError

    def appdir(self):
        return ROOT

    def write(self, rel, text):
        p = os.path.join(self.t, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(text)

    def run_script(self, verb, **env):
        r = subprocess.run(["sh", SCRIPT, verb], env=dict(self.env, **env), capture_output=True, text=True)
        out = json.loads(r.stdout.strip().splitlines()[-1])
        return r.returncode, out

    def lines(self, path):
        try:
            with open(path) as f:
                return f.read().splitlines()
        except OSError:
            return []

    def calls(self):
        return self.lines(self.log)

    def sam_calls(self):
        return self.lines(os.path.join(self.sam, "calls"))

    def closes(self):
        return self.lines(os.path.join(self.sam, "closes"))

    def dump_in_place(self, name=DUMP):
        return os.path.isdir(os.path.join(self.criu, name))

    def dump_aside(self, name=DUMP):
        return os.path.isdir(os.path.join(self.criu, ".lumatv-aside." + name))

    def running(self, *apps):
        self.write("sam/running", json.dumps({"running": [{"id": a, "processid": str(2500 + i)} for i, a in enumerate(apps)], "returnValue": True}))

    def running_as(self, apps):
        """SAM's list for {app id: pid}."""
        self.write("sam/running", json.dumps({"running": [{"id": a, "processid": str(p), "appType": "native"} for a, p in apps.items()], "returnValue": True}))

    def process(self, pid, binary="/usr/bin/com.webos.app.voice", comm=None):
        """A process in the fake /proc: comm is the binary's name cut to 15 characters, cmdline is NUL-separated with the launch JSON after it."""
        d = os.path.join(self.proc, str(pid))
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "comm"), "w") as f:
            f.write((comm or os.path.basename(binary)[:15]) + "\n")
        with open(os.path.join(d, "cmdline"), "wb") as f:
            f.write(binary.encode() + b'\0{"event":"launch","appId":"x"}\0')

    def alive(self, pid):
        return os.path.isdir(os.path.join(self.proc, str(pid)))


@unittest.skipUnless(os.path.isdir(CENSUS) and shutil.which("python3"), "needs the census copy of LG's inputcommon tree")
class Staging(Base):
    def make_qml(self):
        shutil.copytree(CENSUS, self.qml)

    def staged(self):
        up = os.path.join(self.state, "inputcommon", "upper")
        return sorted(os.path.relpath(os.path.join(b, n), up) for b, _, ns in os.walk(up) for n in ns) if os.path.isdir(up) else []

    def upper(self, rel):
        with open(os.path.join(self.state, "inputcommon", "upper", rel)) as f:
            return f.read()

    def test_stage_keeps_only_the_files_the_patch_rewrote_and_leaves_LGs_tree_alone(self):
        rc, out = self.run_script("stage")
        self.assertEqual((rc, out["ok"], out["staged"]), (0, True, True), out)
        self.assertEqual(self.staged(), TOUCHED)
        self.assertTrue(self.upper(STYLE).startswith("// Luma: patched"))
        # LG's directory is as it was, and nothing was mounted, unmounted, moved or asked of SAM
        r = subprocess.run(["diff", "-r", CENSUS, self.qml], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout[:400])
        self.assertEqual(self.calls(), [])
        self.assertEqual(self.sam_calls(), [])
        self.assertTrue(self.dump_in_place())
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

    def test_on_unmounts_first_stages_from_LGs_tree_mounts_and_sets_only_inputcommons_dump_aside(self):
        with open(self.mounts, "w") as f:
            f.write("inputcommon %s overlay rw,relatime 0 0\n" % self.qml)       # a mount left by the last run
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["inputcommon"], out["checkpointAside"]), (0, True, "on", 1), out)
        calls = self.calls()
        self.assertEqual([c.split()[0] for c in calls], ["umount", "mount"])
        self.assertIn("lowerdir=%s,upperdir=%s,workdir=%s" % (self.qml, os.path.join(self.state, "inputcommon", "upper"), os.path.join(self.state, "inputcommon", "work")), calls[1])
        self.assertTrue(calls[1].endswith(" " + self.qml))
        self.assertIn("-t overlay", calls[1])
        self.assertEqual(self.staged(), TOUCHED)
        self.assertTrue(self.dump_aside() and not self.dump_in_place())
        self.assertTrue(self.dump_in_place("com.webos.app.home"))
        rc, out = self.run_script("status")
        self.assertEqual((out["staged"], out["mounted"], out["qml"], out["checkpoint"]), (True, True, self.qml, "aside"))

    def test_a_stage_the_patch_refuses_mounts_nothing_and_leaves_the_dump(self):
        path = os.path.join(self.qml, STYLE)
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("property int inputSimpleMinWidth: 582", "property int inputSimpleMinWidth: 600"))
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertEqual([c.split()[0] for c in self.calls()], [])
        self.assertTrue(self.dump_in_place() and not self.dump_aside())
        self.assertEqual(self.sam_calls(), [])
        self.assertEqual(self.staged(), [])

    def test_a_mount_that_fails_leaves_LGs_banners_and_the_dump(self):
        rc, out = self.run_script("on", FAKE_MOUNT_FAILS="1")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("LG's banners are in place", out["error"])
        self.assertTrue(self.dump_in_place() and not self.dump_aside())
        self.assertEqual(self.sam_calls(), [])
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
        self.assertTrue(self.dump_in_place())
        self.assertEqual(self.staged(), [])

    def test_off_unmounts_puts_the_dump_back_and_stages_nothing(self):
        self.assertEqual(self.run_script("on")[0], 0)
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"], out["inputcommon"], out["checkpointBack"]), (0, True, "off", 1), out)
        self.assertEqual(self.run_script("status")[1]["mounted"], False)
        self.assertEqual([c.split()[0] for c in self.calls()], ["mount", "umount"])
        self.assertTrue(self.dump_in_place() and not self.dump_aside())

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
        self.assertTrue(self.dump_in_place())

    def test_usage(self):
        rc, out = self.run_script("bogus")
        self.assertEqual((rc, out["ok"]), (1, False))


class ColdStart(Base):
    """The part that makes the banners show: LG's dump aside, the idle instances ended, all of it undone by `off`. A stand-in for the
    patch and a small QML tree, so this needs nothing of LG's."""

    def make_qml(self):
        os.makedirs(os.path.join(self.qml, "UserInterfaceLayer"))
        for rel in ("RootWindow.qml", "UserInterfaceLayer/GlobalStyle.qml"):
            with open(os.path.join(self.qml, rel), "w") as f:
                f.write("// LG's\n")

    def appdir(self):
        d = os.path.join(self.t, "app")
        for name, body in (("tokens.py", FAKE_TOKENS), ("inputcommon_patch.py", FAKE_PATCH)):
            p = os.path.join(d, "tools", name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                f.write(body)
        return d

    def test_on_renames_the_dump_in_place_and_touches_nothing_else_of_LGs(self):
        before_home = tree_state(os.path.join(self.criu, "com.webos.app.home"))
        before_dump = tree_state(os.path.join(self.criu, DUMP))
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["checkpointAside"]), (0, True, 1), out)
        self.assertFalse(self.dump_in_place())
        # the same directory, with every file and every symlink as it was (the page file still points into the persisted copy)
        self.assertEqual(tree_state(os.path.join(self.criu, ASIDE)), before_dump)
        self.assertEqual(tree_state(os.path.join(self.criu, "com.webos.app.home")), before_home)
        self.assertTrue(os.path.isdir(os.path.join(self.criu, "error")))
        self.assertEqual(tree_state(self.persisted), self.persisted_before)                # LG's persisted copy: byte for byte, no marker
        self.assertFalse(os.path.exists(os.path.join(self.persisted, DUMP, "invalid_img")))
        self.assertEqual(sorted(os.listdir(self.criu)), [ASIDE, "com.webos.app.home", "error"])

    def test_ls_of_the_working_directory_does_not_show_the_copy_and_the_glob_does_not_find_it(self):
        self.run_script("on")
        r = subprocess.run(["sh", "-c", "cd %s && ls; echo ---; for d in *inputcommon*; do echo $d; done" % self.criu], capture_output=True, text=True)
        self.assertEqual(r.stdout.split("\n"), ["com.webos.app.home", "error", "---", "*inputcommon*", ""])

    def test_the_idle_input_apps_are_closed_after_the_dump_is_aside_and_the_one_on_screen_is_left(self):
        self.running("com.webos.app.home", "com.webos.app.livetv", "com.webos.app.hdmi2", "com.webos.app.hdmi3", "netflix")
        self.write("sam/foreground", "com.webos.app.hdmi3")
        rc, out = self.run_script("on")
        self.assertEqual(out["closed"], ["com.webos.app.livetv", "com.webos.app.hdmi2"], out)
        self.assertEqual(out["onScreen"], "com.webos.app.hdmi3")
        closes = self.closes()
        self.assertEqual(len(closes), 2)
        for c in closes:
            # when SAM was asked to close, LG's dump was no longer where a launch would look: the preload manager's reload is cold
            self.assertIn(ASIDE, c)
            self.assertNotIn(" %s " % DUMP, c.split("held:")[1])
        self.assertNotIn("netflix", " ".join(self.sam_calls()))
        self.assertNotIn("com.webos.app.home", " ".join(c for c in self.sam_calls() if c.startswith("closeByAppId")))

    def test_an_app_that_only_looks_like_an_input_app_is_not_closed(self):
        self.running("com.webos.app.hdmi2x", "com.webos.app.hdmi", "xcom.webos.app.hdmi4")
        rc, out = self.run_script("on")
        self.assertEqual((out["closed"], self.closes()), ([], []))

    def test_every_app_that_runs_on_the_inputcommon_binary_is_known(self):
        apps = ["com.webos.app.inputcommon", "com.webos.app.livetv", "com.webos.app.livetvopapp", "com.webos.app.hdmi1", "com.webos.app.hdmi2", "com.webos.app.hdmi3",
                "com.webos.app.hdmi4", "com.webos.app.dp1", "com.webos.app.dp2", "com.webos.app.externalinput.av1", "com.webos.app.externalinput.av2",
                "com.webos.app.externalinput.component", "com.webos.app.externalinput.scart", "com.webos.app.usbc1", "com.webos.app.usbc2"]
        self.running(*apps)
        rc, out = self.run_script("on")
        self.assertEqual(out["closed"], apps)
        census = "/data/home/lgtv/tmp-research/census1/dc-census/listApps.json"
        if os.path.exists(census):
            with open(census) as f:
                mains = sorted(a["id"] for a in json.load(f)["apps"] if "inputcommon" in a.get("main", ""))
            self.assertEqual(mains, sorted(apps))

    def test_a_close_that_SAM_refuses_is_not_reported_as_done(self):
        self.running("com.webos.app.hdmi2")
        self.write("sam/close-fails", "")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["closed"]), (0, True, []))
        self.assertIn("SAM did not close com.webos.app.hdmi2", "\n".join(self.lines(os.path.join(self.state, "log"))))

    def test_a_SAM_that_does_not_answer_costs_one_bounded_wait_and_ends_nothing(self):
        self.running("com.webos.app.hdmi2")
        self.write("sam/hangs", "")
        t0 = time.time()
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["closed"], out["checkpointAside"]), (0, True, [], 1), out)
        self.assertLess(time.time() - t0, 6)
        self.assertTrue(self.dump_aside())

    def test_without_a_dump_there_is_nothing_to_set_aside_and_the_rest_goes_on(self):
        shutil.rmtree(self.criu)
        self.running("com.webos.app.hdmi2")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["checkpointAside"], out["closed"]), (0, True, 0, ["com.webos.app.hdmi2"]), out)
        self.assertEqual(self.run_script("status")[1]["checkpoint"], "none")
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"], out["checkpointBack"]), (0, True, 0))

    def test_a_dump_that_is_already_aside_is_kept_and_what_stands_in_its_place_is_dropped(self):
        self.run_script("on")
        os.makedirs(os.path.join(self.criu, DUMP))                    # a dump made while the overlay was mounted holds the patched app
        with open(os.path.join(self.criu, DUMP, "core-9999.img"), "w") as f:
            f.write("patched")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["checkpointAside"]), (0, 1), out)
        self.assertFalse(self.dump_in_place())
        with open(os.path.join(self.criu, ASIDE, "core-2513.img")) as f:
            self.assertEqual(f.read(), "core of " + DUMP)             # LG's own, still
        self.assertFalse(os.path.exists(os.path.join(self.criu, ASIDE, "core-9999.img")))
        self.assertEqual(tree_state(self.persisted), self.persisted_before)

    def test_off_puts_the_dump_back_exactly_and_ends_the_idle_instances_again(self):
        before = tree_state(os.path.join(self.criu, DUMP))
        self.run_script("on")
        self.running("com.webos.app.hdmi2")
        os.makedirs(os.path.join(self.criu, DUMP))                    # made meanwhile: the patched app, not to be restored
        with open(os.path.join(self.criu, DUMP, "core-9999.img"), "w") as f:
            f.write("patched")
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"], out["checkpointBack"], out["closed"]), (0, True, 1, ["com.webos.app.hdmi2"]), out)
        self.assertFalse(self.dump_aside())
        self.assertEqual(tree_state(os.path.join(self.criu, DUMP)), before)
        self.assertEqual(tree_state(self.persisted), self.persisted_before)
        self.assertIn(DUMP, self.closes()[-1].split("held:")[1])      # the dump was back before the close: the reload restores LG's
        self.assertEqual(self.run_script("status")[1]["checkpoint"], "present")

    def test_off_on_a_set_that_never_used_the_switch_changes_and_asks_nothing(self):
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"], out["checkpointBack"], out["closed"]), (0, True, 0, []), out)
        self.assertEqual(self.sam_calls(), [])
        self.assertTrue(self.dump_in_place())
        self.assertEqual(self.calls(), [])

    def test_off_leaves_the_directory_alone_when_a_copy_has_no_name(self):
        os.makedirs(os.path.join(self.criu, ".lumatv-aside."))        # nothing of ours: a name of nothing must not become `rm -rf $CRIU_TMP/`
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ok"]), (0, True))
        self.assertEqual(sorted(os.listdir(self.criu)), [".lumatv-aside.", "com.webos.app.home", DUMP, "error"])
        self.assertTrue(self.dump_in_place("com.webos.app.home"))

    def test_a_restore_that_is_still_running_is_waited_for_a_moment_and_then_not_allowed_to_stop_the_switch(self):
        open(self.env["CRIU_RESTORING"], "w").close()
        t0 = time.time()
        rc, out = self.run_script("on", RESTORE_WAIT="2")
        took = time.time() - t0
        self.assertEqual((rc, out["ok"], out["checkpointAside"]), (0, True, 1), out)
        self.assertGreaterEqual(took, 2)
        self.assertIn("a CRIU restore was still running after 2 s", "\n".join(self.lines(os.path.join(self.state, "log"))))

    def test_a_restore_that_ends_while_it_waits_lets_the_switch_through_at_once(self):
        flag = self.env["CRIU_RESTORING"]
        open(flag, "w").close()
        p = subprocess.Popen(["sh", "-c", "sleep 1; rm -f %s" % flag])
        t0 = time.time()
        rc, out = self.run_script("on", RESTORE_WAIT="5")
        p.wait()
        self.assertEqual((rc, out["ok"]), (0, True))
        self.assertLess(time.time() - t0, 4.5)
        self.assertNotIn("still running", "\n".join(self.lines(os.path.join(self.state, "log"))))

    def test_status_says_where_the_dump_is_and_whether_CRIU_is_in_use(self):
        j = self.run_script("status")[1]
        self.assertEqual((j["checkpoint"], j["criuEnabled"], j["mounted"]), ("present", False, False))
        open(self.env["CRIU_ENABLE"], "w").close()
        self.run_script("on")
        j = self.run_script("status")[1]
        self.assertEqual((j["checkpoint"], j["criuEnabled"], j["mounted"]), ("aside", True, True))

    def test_the_input_banners_run_never_signals_a_process_and_says_nothing_of_one(self):
        # a close ends the input apps; only a tree with a resident app (the Search app, tools/voice.sh) has processes to end
        self.process(8321)
        self.running("com.webos.app.hdmi2")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["closed"]), (0, True, ["com.webos.app.hdmi2"]), out)
        self.assertNotIn("ended", out)
        rc, out = self.run_script("off")
        self.assertNotIn("ended", out)
        self.assertNotIn("pids", self.run_script("status")[1])
        self.assertTrue(self.alive(8321))
        self.assertFalse([c for c in self.sam_calls() if c.startswith("kill")])

    def test_the_boot_path_does_the_whole_of_it_again(self):
        # a cold power-on brings back the boot snapshot: the overlay is gone and so is the aside copy; LG's dump is where it was
        self.run_script("on")
        shutil.rmtree(os.path.join(self.criu, ASIDE))
        shutil.copytree(os.path.join(self.persisted, DUMP), os.path.join(self.criu, DUMP), symlinks=True)
        open(self.mounts, "w").close()
        self.running("com.webos.app.hdmi2")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["checkpointAside"], out["closed"]), (0, 1, ["com.webos.app.hdmi2"]), out)
        self.assertTrue(self.dump_aside() and not self.dump_in_place())


if __name__ == "__main__":
    unittest.main()
