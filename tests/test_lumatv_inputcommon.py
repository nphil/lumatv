"""Run with: python3 -m unittest tests.test_lumatv_inputcommon

How tools/lumatv drives the LG-process restyles of tools/inputcommon.sh and tools/keyboard_patch.py: the `inputcommon` verb
(a stand-in for the tool, so no mount happens), the switch's flag and where it is read (the registry, apply's boot re-run, revert),
and the keyboard patch in the shadow module's build, against the census copy of LG's view when it is on this workstation.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "lumatv")
KEYBOARD_TOOL = os.path.join(ROOT, "tools", "keyboard_patch.py")
THEME_SRC = os.path.join(ROOT, "payload", "theme")
VIEW = "/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/views/StarfishKeyboardView.qml"


def text():
    with open(TOOL) as f:
        return f.read()


def function(name):
    """One shell function of tools/lumatv, from its `name() {` line to the closing brace in column 0."""
    out, on = [], False
    with open(TOOL) as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")


FAKE_TOOL = """#!/bin/sh
echo "$*" >> "$FAKE/tool-calls"
case "$1" in
  on) if [ -e "$FAKE/on-fails" ]; then echo '{"ok":false,"error":"could not mount the patched copy; LG'"'"'s banners are in place"}'; exit 1; fi
      [ -e "$FAKE/silent" ] && exit 0
      echo '{"ok":true,"inputcommon":"on","snapshotsDropped":1}' ;;
  off) if [ -e "$FAKE/off-fails" ]; then echo '{"ok":false,"error":"could not unmount the overlay"}'; exit 1; fi
       echo '{"ok":true,"inputcommon":"off","snapshotsDropped":1}' ;;
  status) [ -e "$FAKE/silent" ] && exit 0
          m=false; [ -e "$FAKE/mounted" ] && m=true
          echo '{"ok":true,"qml":"/usr/palm/applications/com.webos.app.inputcommon/qml","staged":true,"mounted":'$m'}' ;;
esac
"""


class Verb(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.tool = os.path.join(self.d, "inputcommon.sh")
        with open(self.tool, "w") as f:
            f.write(FAKE_TOOL)
        self.flag = os.path.join(self.d, ".inputcommon-on")

    def tearDown(self):
        self.tmp.cleanup()

    def run_verb(self, arg, lock=True):
        script = (f'FAKE="{self.d}"; export FAKE\nINPUTCOMMON_TOOL="{self.tool}"\nINPUTCOMMON_ON="{self.flag}"\nLOGFILE="{self.d}/log"\n'
                  f'take_mutation_lock() {{ {"return 0" if lock else "return 1"}; }}\n'
                  f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + function("json_escape") + function("emit_error") + function("inputcommon_set") + function("verb_inputcommon")
                  + f'verb_inputcommon "{arg}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout.strip()

    def calls(self):
        p = os.path.join(self.d, "tool-calls")
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return f.read().split("\n")[:-1]

    def loglines(self):
        p = os.path.join(self.d, "loglines")
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return f.read().split("\n")[:-1]

    def mark(self, name):
        open(os.path.join(self.d, name), "w").close()

    def test_on_runs_the_tool_keeps_the_flag_and_prints_the_tools_object(self):
        rc, out = self.run_verb("on")
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out), {"ok": True, "inputcommon": "on", "snapshotsDropped": 1})
        self.assertEqual(self.calls(), ["on"])
        self.assertTrue(os.path.exists(self.flag))
        self.assertEqual(len(self.loglines()), 1)
        self.assertTrue(self.loglines()[0].startswith("inputcommon on: {"))

    def test_off_removes_the_flag(self):
        open(self.flag, "w").close()
        rc, out = self.run_verb("off")
        self.assertEqual((rc, json.loads(out)["inputcommon"]), (0, "off"))
        self.assertFalse(os.path.exists(self.flag))

    def test_a_refused_on_says_why_and_leaves_the_flag_as_it_was(self):
        self.mark("on-fails")
        rc, out = self.run_verb("on")
        self.assertEqual(rc, 1)
        j = json.loads(out)
        self.assertFalse(j["ok"])
        self.assertIn("LG's banners are in place", j["error"])
        self.assertFalse(os.path.exists(self.flag))
        open(self.flag, "w").close()                                   # a second `on` that fails does not turn the switch off
        self.assertEqual(self.run_verb("on")[0], 1)
        self.assertTrue(os.path.exists(self.flag))

    def test_an_off_that_cannot_unmount_keeps_the_flag_so_that_apply_does_not_forget_the_mount(self):
        open(self.flag, "w").close()
        self.mark("off-fails")
        rc, out = self.run_verb("off")
        self.assertEqual(rc, 1)
        self.assertFalse(json.loads(out)["ok"])
        self.assertTrue(os.path.exists(self.flag))

    def test_a_tool_that_says_nothing_is_an_error_and_changes_no_flag(self):
        self.mark("silent")
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "tools/inputcommon.sh did not answer"}))
        self.assertFalse(os.path.exists(self.flag))
        rc, out = self.run_verb("status")
        self.assertEqual((rc, json.loads(out)["ok"]), (1, False))

    def test_status_is_the_tools_object_with_the_switch_in_front(self):
        rc, out = self.run_verb("status")
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out), {"ok": True, "on": False, "qml": "/usr/palm/applications/com.webos.app.inputcommon/qml", "staged": True, "mounted": False})
        open(self.flag, "w").close()
        self.assertTrue(json.loads(self.run_verb("status")[1])["on"])
        self.assertEqual(self.calls(), ["status", "status"])

    def test_a_mutation_in_progress_and_a_wrong_word_are_errors_that_touch_nothing(self):
        rc, out = self.run_verb("on", lock=False)
        self.assertEqual((rc, json.loads(out)["error"]), (1, "another apply or revert is still running"))
        rc, out = self.run_verb("sideways")
        self.assertEqual((rc, json.loads(out)["error"]), (1, "usage: inputcommon on|off|status"))
        self.assertEqual(self.calls(), [])


class Wiring(unittest.TestCase):
    """The places the switch is read: the registry, apply (the mount does not survive a reboot), revert, the dispatcher."""

    def apply_line(self):
        lines = [l.strip() for l in function("verb_apply").splitlines() if '"$INPUTCOMMON_ON"' in l]
        self.assertEqual(len(lines), 1)
        return lines[0]

    def test_apply_mounts_again_while_the_switch_is_on_with_the_other_surfaces(self):
        apply_text = function("verb_apply")
        line = self.apply_line()
        self.assertLess(apply_text.index('[ -f "$NOTI_ON" ] && { noti_set on'), apply_text.index(line))
        self.assertLess(apply_text.index(line), apply_text.index("compositor_changed=1"))
        self.assertLess(apply_text.index('[ -f "$WEBGLASS_ON" ] && { webglass_set on'), apply_text.index(line))

    def run_apply_line(self, flag=True, mounted=False, fails=False):
        with tempfile.TemporaryDirectory() as d:
            tool = os.path.join(d, "inputcommon.sh")
            with open(tool, "w") as f:
                f.write(FAKE_TOOL)
            for name, on in (("mounted", mounted), ("on-fails", fails), (".inputcommon-on", flag)):
                if on:
                    open(os.path.join(d, name), "w").close()
            script = (f'FAKE="{d}"; export FAKE\nINPUTCOMMON_TOOL="{tool}"\nINPUTCOMMON_ON="{d}/.inputcommon-on"\nLOGFILE="{d}/log"\n'
                      f'log_line() {{ echo "$1" >> "{d}/loglines"; }}\n'
                      + function("inputcommon_mounted") + function("inputcommon_set") + self.apply_line() + "\ntrue\n")
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            self.assertEqual(r.returncode, 0, r.stderr)
            calls = open(os.path.join(d, "tool-calls")).read().split("\n")[:-1] if os.path.exists(os.path.join(d, "tool-calls")) else []
            logs = open(os.path.join(d, "loglines")).read().split("\n")[:-1] if os.path.exists(os.path.join(d, "loglines")) else []
            return calls, logs

    def test_a_boot_with_the_switch_on_and_no_mount_makes_the_mount(self):
        calls, logs = self.run_apply_line()
        self.assertEqual(calls, ["status", "on"])
        self.assertEqual(len(logs), 1)

    def test_an_overlay_that_is_in_place_is_left_alone_so_inputcommons_snapshot_is_not_dropped_at_every_launch(self):
        self.assertEqual(self.run_apply_line(mounted=True), (["status"], []))

    def test_a_switch_that_is_off_asks_nothing(self):
        self.assertEqual(self.run_apply_line(flag=False), ([], []))

    def test_a_mount_that_fails_is_logged_and_apply_goes_on(self):
        calls, logs = self.run_apply_line(fails=True)
        self.assertEqual(calls, ["status", "on"])
        self.assertEqual(logs[-1], "apply: could not mount the patched input banners over LG's")

    def test_revert_puts_LGs_banners_back(self):
        self.assertIn('[ -f "$INPUTCOMMON_ON" ] && { inputcommon_set off; rm -f "$INPUTCOMMON_ON"; }', function("do_revert_internal"))

    def test_the_verb_is_dispatched_and_documented(self):
        t = text()
        self.assertRegex(t, r"\n  status\|apply\|[^\n]*\|webglass\|inputcommon\|[^\n]*\)\n")        # the verbs that migrate first
        self.assertIn('inputcommon) verb_inputcommon "${2:-status}"; exit $? ;;', t)
        self.assertRegex(t, r"\n#   inputcommon on\|off\|status ->")

    def test_the_flag_is_off_by_default_and_lives_in_the_data_directory(self):
        self.assertIn('INPUTCOMMON_ON="$STATE/.inputcommon-on"', text())
        self.assertIn('INPUTCOMMON_TOOL="$APPDIR/tools/inputcommon.sh"', text())

    def test_the_switch_never_restarts_a_service(self):
        # a mount over LG's QML and a snapshot dropped: nothing restarts, least of all SAM or the compositor's unit
        for name in ("inputcommon_set", "verb_inputcommon"):
            body = function(name)
            self.assertNotRegex(body, r"systemctl|restart|pkill|killall|restart_surface", name)
        self.assertNotIn("systemctl", open(os.path.join(ROOT, "tools", "inputcommon.sh")).read())


STOCK_MODULE = "WebOSCompositor"


@unittest.skipUnless(os.path.isfile(VIEW), "needs the census copy of LG's compositor module")
class ShadowKeyboard(unittest.TestCase):
    """build_shadow runs the keyboard patch on the shadow's copy of LG's view, and keeps LG's file when the patch refuses."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.stock = os.path.join(d, "stock", STOCK_MODULE)
        os.makedirs(os.path.join(self.stock, "views", "fullscreen"))
        shutil.copy(VIEW, os.path.join(self.stock, "views", "StarfishKeyboardView.qml"))
        with open(os.path.join(self.stock, "views", "fullscreen", "StarfishFullscreenContainer.qml"), "w") as f:
            f.write("// LG's container\n")
        self.comp_stage = os.path.join(d, "compositor.qml")
        with open(self.comp_stage, "w") as f:
            f.write("// ours\n")
        os.makedirs(os.path.join(d, "luma", "LumaHome"))
        with open(os.path.join(d, "luma", "LumaHome", "Home.qml"), "w") as f:
            f.write("// home\n")
        self.shadow = os.path.join(d, "qml")
        self.log = os.path.join(d, "log")
        self.stock_view = os.path.join(self.stock, "views", "StarfishKeyboardView.qml")
        self.shadow_view = os.path.join(self.shadow, STOCK_MODULE, "views", "StarfishKeyboardView.qml")

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, tool=KEYBOARD_TOOL, theme=THEME_SRC, on=True):
        d = self.tmp.name
        flag = os.path.join(d, ".keyboard-on")
        if on:
            open(flag, "w").close()
        elif os.path.exists(flag):
            os.remove(flag)
        script = (f'KEYBOARD_ON="{flag}"\n' + f'STOCK_QML_MODULE="{self.stock}"\nCOMPOSITOR_STAGE="{self.comp_stage}"\nLUMA_STAGE="{d}/luma"\nQML_MODULE=LumaHome\n'
                  f'SHADOW_ROOT="{self.shadow}"\nMODULE_RELPATH=views/fullscreen/StarfishFullscreenContainer.qml\n'
                  f'SHADOW_STOCK_FP="{d}/stock.fp"\nSHADOW_SELF_FP="{d}/self.fp"\nSHADOW_PATCH_FP="{d}/patch.fp"\nKEYBOARD_TOOL="{tool}"\nTHEME_SRC="{theme}"\nLOGFILE="{self.log}"\n'
                  'SHADOW_PATCH_FILES="$KEYBOARD_TOOL $THEME_SRC/theme.json"\n'
                  f'log_line() {{ echo "$1" >> "{self.log}"; }}\n'
                  + function("stock_module_fp") + function("shadow_self_fp") + function("shadow_patch_fp") + function("build_shadow") + "build_shadow; echo rc=$?\n")
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertIn("rc=0", r.stdout, r.stdout + r.stderr)
        return open(self.log).read() if os.path.exists(self.log) else ""

    def read(self, path):
        with open(path) as f:
            return f.read()

    def test_the_shadow_has_the_keyboard_in_the_themes_motion_and_lgs_own_stays_as_it_was(self):
        log = self.build()
        view = self.read(self.shadow_view)
        self.assertTrue(view.startswith("// Luma: patched"), view[:80])
        self.assertNotIn("Beziergon", view)
        self.assertEqual(self.read(self.stock_view), self.read(VIEW))             # LG's file under /usr is never written
        self.assertNotIn("keyboard patch", log)

    def test_a_view_that_lg_has_changed_is_left_as_lgs_and_the_build_goes_on(self):
        with open(self.stock_view, "w") as f:
            f.write("import QtQuick 2.4\nItem { /* a keyboard view with none of the anchors */ }\n")
        log = self.build()
        self.assertEqual(self.read(self.shadow_view), self.read(self.stock_view))
        self.assertIn("the keyboard patch was refused, LG's keyboard view stays", log)
        self.assertEqual(self.read(os.path.join(self.shadow, STOCK_MODULE, "views", "fullscreen", "StarfishFullscreenContainer.qml")), "// ours\n")

    def test_a_theme_without_usable_motion_leaves_lgs_view(self):
        bad = os.path.join(self.tmp.name, "theme")
        os.makedirs(bad)
        with open(os.path.join(bad, "theme.json"), "w") as f:
            json.dump({"motion": {"layerMs": 0, "cardEasing": "OutCubic"}}, f)
        log = self.build(theme=bad)
        self.assertEqual(self.read(self.shadow_view), self.read(VIEW))
        self.assertIn("the keyboard patch was refused", log)

    def test_without_the_tool_or_the_view_nothing_is_attempted(self):
        self.assertEqual(self.build(tool="/nonexistent/keyboard_patch.py"), "")
        self.assertEqual(self.read(self.shadow_view), self.read(VIEW))
        os.remove(self.stock_view)
        shutil.rmtree(self.shadow)
        self.assertEqual(self.build(), "")
        self.assertFalse(os.path.exists(self.shadow_view))

    def test_the_published_module_is_complete(self):
        self.build()
        for rel in ("views/StarfishKeyboardView.qml", "views/fullscreen/StarfishFullscreenContainer.qml", "LumaHome/Home.qml"):
            self.assertTrue(os.path.isfile(os.path.join(self.shadow, STOCK_MODULE, rel)), rel)
        self.assertFalse(os.path.exists(self.shadow + ".next"))
        self.assertTrue(re.fullmatch(r"[0-9a-f]{32}", self.read(os.path.join(self.tmp.name, "self.fp")).strip()))

    def test_with_the_switch_off_lgs_own_view_stays_and_the_build_says_nothing(self):
        log = self.build(on=False)
        self.assertEqual(self.read(self.shadow_view), self.read(VIEW))
        self.assertNotIn("keyboard patch", log)
        self.build(on=True)
        self.assertTrue(self.read(self.shadow_view).startswith("// Luma: patched"))


KEYBOARD_APPLY = """#!/bin/sh
echo "$*" >> "$FAKE/apply-calls"
[ -e "$FAKE/silent" ] && exit 0
if [ -e "$FAKE/apply-fails" ]; then echo '{"ok":false,"error":"shadow module build failed"}'; exit 1; fi
echo '{"ok":true,"applied":true,"mounts":3,"took":4}'
"""


class KeyboardVerb(unittest.TestCase):
    """`keyboard on|off|status`: the flag build_shadow reads, and the one apply that makes the change reach the set."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        os.makedirs(os.path.join(self.d, "app", "tools"))
        with open(os.path.join(self.d, "app", "tools", "lumatv"), "w") as f:
            f.write(KEYBOARD_APPLY)
        self.flag = os.path.join(self.d, ".keyboard-on")
        self.shadow = os.path.join(self.d, "qml")
        self.view = os.path.join(self.shadow, "WebOSCompositor", "views", "StarfishKeyboardView.qml")

    def tearDown(self):
        self.tmp.cleanup()

    def run_verb(self, arg, module=True):
        if module:
            os.makedirs(os.path.dirname(self.view), exist_ok=True)
        script = (f'FAKE="{self.d}"; export FAKE\nAPPDIR="{self.d}/app"\nSTATE="{self.d}"\nKEYBOARD_ON="{self.flag}"\nSHADOW_ROOT="{self.shadow}"\nLOGFILE="{self.d}/log"\n'
                  f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + function("json_escape") + function("emit_error") + function("keyboard_patched") + function("verb_keyboard") + f'verb_keyboard "{arg}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout.strip()

    def applies(self):
        p = os.path.join(self.d, "apply-calls")
        return open(p).read().split("\n")[:-1] if os.path.exists(p) else []

    def mark(self, name):
        open(os.path.join(self.d, name), "w").close()

    def test_on_leaves_the_flag_and_applies_once(self):
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "keyboard": "on", "applied": True, "patched": False}))
        self.assertTrue(os.path.exists(self.flag))
        self.assertEqual(self.applies(), ["apply"])

    def test_off_takes_the_flag_away_and_applies_once(self):
        open(self.flag, "w").close()
        rc, out = self.run_verb("off")
        self.assertEqual((rc, json.loads(out)["keyboard"], json.loads(out)["applied"]), (0, "off", True))
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual(self.applies(), ["apply"])

    def test_a_set_that_was_never_applied_has_no_module_so_only_the_flag_changes(self):
        rc, out = self.run_verb("on", module=False)
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "keyboard": "on", "applied": False, "patched": False}))
        self.assertTrue(os.path.exists(self.flag))
        self.assertEqual(self.applies(), [])

    def test_an_apply_that_fails_puts_the_flag_back_and_says_why(self):
        self.mark("apply-fails")
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "shadow module build failed"}))
        self.assertFalse(os.path.exists(self.flag))
        open(self.flag, "w").close()
        self.assertEqual(self.run_verb("off")[0], 1)
        self.assertTrue(os.path.exists(self.flag))

    def test_an_apply_that_says_nothing_is_an_error_and_changes_no_flag(self):
        self.mark("silent")
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "apply did not answer"}))
        self.assertFalse(os.path.exists(self.flag))

    def test_status_reads_the_flag_and_the_module(self):
        self.assertEqual(json.loads(self.run_verb("status")[1]), {"ok": True, "on": False, "patched": False})
        with open(self.view, "w") as f:
            f.write("// Luma: patched, see tools/keyboard_patch.py\n")
        self.assertEqual(json.loads(self.run_verb("status")[1]), {"ok": True, "on": False, "patched": True})
        open(self.flag, "w").close()
        self.assertTrue(json.loads(self.run_verb("status")[1])["on"])
        self.assertEqual(self.applies(), [])

    def test_a_wrong_word_is_an_error_that_touches_nothing(self):
        rc, out = self.run_verb("sideways")
        self.assertEqual((rc, json.loads(out)["error"]), (1, "usage: keyboard on|off|status"))
        self.assertFalse(os.path.exists(self.flag))
        self.assertEqual(self.applies(), [])

    def test_the_fingerprint_names_the_switch_so_that_flipping_it_rebuilds_the_shadow(self):
        import hashlib
        a, b = os.path.join(self.d, "a.py"), os.path.join(self.d, "b.json")
        open(a, "w").write("print(1)\n")
        open(b, "w").write("{}\n")
        body = f"{a}\nprint(1)\n{b}\n{{}}\n"

        def fp(flag):
            script = f'SHADOW_PATCH_FILES="{a} {b}"\nKEYBOARD_ON="{flag}"\n' + function("shadow_patch_fp") + "shadow_patch_fp\n"
            return subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30).stdout.strip()
        self.assertEqual(fp(self.flag), hashlib.md5((body + "keyboard off\n").encode()).hexdigest())
        open(self.flag, "w").close()
        self.assertEqual(fp(self.flag), hashlib.md5((body + "keyboard on\n").encode()).hexdigest())

    def test_the_verb_is_dispatched_documented_and_in_the_registry(self):
        t = text()
        self.assertRegex(t, r"\n  status\|apply\|[^\n]*\|inputcommon\|keyboard\|[^\n]*\)\n")
        self.assertIn('keyboard) verb_keyboard "${2:-status}"; exit $? ;;', t)
        self.assertRegex(t, r"\n#   keyboard on\|off\|status ->")
        self.assertIn('{"id":"keyboard","on":%s', t)
        self.assertIn('KEYBOARD_ON="$STATE/.keyboard-on"', t)
        self.assertIn('rm -f "$KEYBOARD_ON"', function("do_revert_internal"))


def tool_lines(pattern):
    """The lines of tools/lumatv that match pattern at their start, in file order."""
    with open(TOOL) as f:
        return [l.rstrip("\n") for l in f if re.match(pattern, l)]


# The constants that name a script of tools/ and the lines that make up the shadow's patch list, as the tool defines them.
TOOL_CONSTANTS = tool_lines(r'[A-Z_]+_TOOL="\$APPDIR/tools/[^"]+"$')
PATCH_LIST = tool_lines(r"SHADOW_PATCH_FILES=")


class ShadowPatches(unittest.TestCase):
    """The shadow module's identity includes what its patches were made from. Without it a set that already has a shadow keeps the old
    one when an update changes only a patch script or the theme's motion: shadow_current() and apply's compositor_changed never
    looked at them, so the keyboard patch (and any later one) reached a set only when LumaHome's QML changed in the same update.

    Runs the tool's own functions and constants under sh against a small module and stand-in patch scripts, no census needed."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.d = d
        self.app = os.path.join(d, "app")
        self.files = {
            "stock/WebOSCompositor/views/StarfishKeyboardView.qml": "// LG's keyboard view\n",
            "stock/WebOSCompositor/views/fullscreen/StarfishFullscreenContainer.qml": "// LG's container\n",
            "compositor.qml": "// ours\n",
            "luma/LumaHome/Home.qml": "// home\n",
            "app/tools/keyboard_patch.py": "import sys\nopen(sys.argv[1], 'a').write('// patched v1\\n')\n",
            "app/tools/notification_patch.py": "# sub()\n",
            "app/tools/tokens.py": "# EASING\n",
            "app/theme/theme.json": '{"motion": {"layerMs": 300}}\n',
        }
        for rel, body in self.files.items():
            self.put(rel, body)
        self.put(".keyboard-on", "")                                # the switch on: the patch is made
        self.shadow = os.path.join(d, "qml")

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, rel, body):
        p = os.path.join(self.d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(body)

    def read(self, rel):
        with open(os.path.join(self.d, rel)) as f:
            return f.read()

    def sh(self, command, extra=""):
        d = self.d
        script = (f'APPDIR="{self.app}"\nSTATE="{d}/state"\nKEYBOARD_ON="{d}/.keyboard-on"\nTHEME_SRC="{self.app}/theme"\n' + "\n".join(TOOL_CONSTANTS) + "\n" + "\n".join(PATCH_LIST) + "\n"
                  f'STOCK_QML_MODULE="{d}/stock/WebOSCompositor"\nCOMPOSITOR_STAGE="{d}/compositor.qml"\nLUMA_STAGE="{d}/luma"\nQML_MODULE=LumaHome\n'
                  f'SHADOW_ROOT="{self.shadow}"\nMODULE_RELPATH=views/fullscreen/StarfishFullscreenContainer.qml\n'
                  'SHADOW_QML="$SHADOW_ROOT/WebOSCompositor/$MODULE_RELPATH"\n'
                  f'SHADOW_STOCK_FP="{d}/stock.fp"\nSHADOW_SELF_FP="{d}/self.fp"\nSHADOW_PATCH_FP="{d}/patch.fp"\nLOGFILE="{d}/log"\n'
                  f'log_line() {{ echo "$1" >> "{d}/log"; }}\n'
                  + "".join(function(n) for n in ("tree_fp", "stock_module_fp", "shadow_self_fp", "shadow_patch_fp", "shadow_patch_current", "build_shadow", "shadow_current"))
                  + extra + command + "\n")
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout.strip()

    def build(self):
        self.assertEqual(self.sh("build_shadow")[0], 0)

    def test_a_build_records_what_its_patches_were_made_from_and_is_then_current(self):
        self.build()
        rc, fp = self.sh("shadow_patch_fp")
        self.assertRegex(fp, r"^[0-9a-f]{32}$")
        self.assertEqual(self.read("patch.fp").strip(), fp)
        self.assertEqual(self.sh("shadow_current")[0], 0)
        self.assertEqual(self.sh("shadow_patch_current")[0], 0)
        with open(os.path.join(self.shadow, STOCK_MODULE, "views", "StarfishKeyboardView.qml")) as f:
            self.assertIn("// patched v1", f.read())

    def test_a_changed_patch_script_or_theme_makes_the_shadow_not_current_until_it_is_rebuilt(self):
        for rel in ("app/tools/keyboard_patch.py", "app/tools/notification_patch.py", "app/tools/tokens.py", "app/theme/theme.json"):
            self.build()
            self.assertEqual(self.sh("shadow_current")[0], 0, rel)
            self.put(rel, self.read(rel) + "\n# changed by an update\n")
            self.assertEqual(self.sh("shadow_patch_current")[0], 1, rel)
            self.assertEqual(self.sh("shadow_current")[0], 1, rel)
            self.build()
            self.assertEqual(self.sh("shadow_current")[0], 0, rel + ": a rebuild takes the change in")

    def test_the_rebuild_carries_the_new_patch_into_the_module(self):
        self.build()
        self.put("app/tools/keyboard_patch.py", self.files["app/tools/keyboard_patch.py"].replace("v1", "v2"))
        self.assertEqual(self.sh("shadow_current")[0], 1)
        self.build()
        with open(os.path.join(self.shadow, STOCK_MODULE, "views", "StarfishKeyboardView.qml")) as f:
            view = f.read()
        self.assertIn("// patched v2", view)
        self.assertNotIn("// patched v1", view)

    def test_a_shadow_built_before_there_was_a_fingerprint_is_rebuilt_once(self):
        self.build()
        os.remove(os.path.join(self.d, "patch.fp"))                 # what every shadow on a set today looks like
        self.assertEqual(self.sh("shadow_current")[0], 1)
        self.build()
        self.assertEqual(self.sh("shadow_current")[0], 0)

    def test_a_patch_script_that_goes_missing_is_a_change_too(self):
        self.build()
        os.remove(os.path.join(self.app, "tools", "keyboard_patch.py"))
        self.assertEqual(self.sh("shadow_current")[0], 1)
        self.build()                                                 # LG's view stays, the build goes on, and that is what was recorded
        self.assertEqual(self.sh("shadow_current")[0], 0)
        with open(os.path.join(self.shadow, STOCK_MODULE, "views", "StarfishKeyboardView.qml")) as f:
            self.assertEqual(f.read(), "// LG's keyboard view\n")

    def test_flipping_the_keyboard_switch_makes_the_shadow_not_current_until_it_is_rebuilt(self):
        self.build()
        os.remove(os.path.join(self.d, ".keyboard-on"))
        self.assertEqual(self.sh("shadow_current")[0], 1)
        self.build()
        self.assertEqual(self.sh("shadow_current")[0], 0)
        with open(os.path.join(self.shadow, STOCK_MODULE, "views", "StarfishKeyboardView.qml")) as f:
            self.assertEqual(f.read(), "// LG's keyboard view\n")

    def test_only_the_content_counts_not_the_time(self):
        self.build()
        os.utime(os.path.join(self.app, "tools", "keyboard_patch.py"), (1, 1))
        self.assertEqual(self.sh("shadow_current")[0], 0)

    def test_the_same_fingerprint_comes_back_from_the_same_files(self):
        a = self.sh("shadow_patch_fp")[1]
        self.assertEqual(self.sh("shadow_patch_fp")[1], a)
        self.put("app/theme/theme.json", "{}\n")
        self.assertNotEqual(self.sh("shadow_patch_fp")[1], a)

    def test_a_patch_only_update_changes_what_the_compositor_reads_so_apply_restarts_it_once(self):
        # apply's own condition, run as it is written: nothing staged differs, then only a patch does
        m = re.search(r"  compositor_changed=1\n  if .*?\n  fi\n", function("verb_apply"), re.S)
        self.assertIsNotNone(m, "the compositor_changed block of verb_apply is gone")
        for rel, body in (("stage/compositor.qml", "// ours\n"), ("stage/compositor.qml.next", "// ours\n"), ("wall.png", "png"), ("stage/wall.png", "png"),
                          ("stage/luma/Home.qml", "// home\n"), ("stage/luma.next/Home.qml", "// home\n")):
            self.put(rel, body)
        d = self.d
        setup = (f'COMPOSITOR_STAGE="{d}/stage/compositor.qml"\nWALL_SRC="{d}/wall.png"\nWALLPAPER_STAGE="{d}/stage/wall.png"\nLUMA_STAGE="{d}/stage/luma"\n'
                 'kf_changed=0\n')
        block = m.group(0) + 'echo "compositor_changed=$compositor_changed"\n'
        self.build()
        # SHADOW_ROOT & co. are the build's: run the block in the same shell setup
        def run(kf=0):
            rc, out = self.sh(block.replace('kf_changed', 'kf_changed'), extra=setup.replace("kf_changed=0", "kf_changed=%d" % kf) + 'LUMA_STAGE="%s/stage/luma"\n' % d)
            return out.split("compositor_changed=")[-1]
        self.assertEqual(run(), "0", "nothing changed: the compositor is left running")
        self.assertEqual(run(kf=1), "1", "the key filter changed: restart")
        self.put("app/tools/keyboard_patch.py", self.read("app/tools/keyboard_patch.py") + "# v2\n")
        self.assertEqual(run(), "1", "only a patch changed")
        os.remove(os.path.join(d, "patch.fp"))
        self.assertEqual(run(), "1", "a shadow with no fingerprint")

    def test_revert_forgets_the_fingerprint_with_the_other_two(self):
        self.assertIn('rm -f "$SHADOW_STOCK_FP" "$SHADOW_SELF_FP" "$SHADOW_PATCH_FP"', function("do_revert_internal"))


class ShadowPatchList(unittest.TestCase):
    """The list itself: every patch build_shadow runs is in it, and so is everything those patches import."""

    def listed(self):
        r = subprocess.run(["sh", "-c", f'APPDIR="{ROOT}"\nTHEME_SRC="{ROOT}/payload/theme"\n' + "\n".join(TOOL_CONSTANTS) + "\n" + "\n".join(PATCH_LIST)
                            + '\nfor f in $SHADOW_PATCH_FILES; do echo "$f"; done\n'], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.split()

    def test_the_list_is_one_plain_assignment_and_then_appends_so_that_a_later_patch_cannot_overwrite_an_earlier_one(self):
        self.assertGreaterEqual(len(PATCH_LIST), 1)
        self.assertRegex(PATCH_LIST[0], r'^SHADOW_PATCH_FILES="\$KEYBOARD_TOOL ')
        for line in PATCH_LIST[1:]:
            self.assertRegex(line, r'^SHADOW_PATCH_FILES="\$SHADOW_PATCH_FILES [^"]+"$')
        # every constant a line names is defined before that line
        with open(TOOL) as f:
            lines = f.read().split("\n")
        for n, line in enumerate(lines):
            if line.startswith("SHADOW_PATCH_FILES="):
                for name in re.findall(r"\$([A-Z_]+)", line):
                    if name == "SHADOW_PATCH_FILES":
                        continue
                    first = next(i for i, l in enumerate(lines) if re.match(r"%s=" % name, l))
                    self.assertLess(first, n, "%s is defined after the list that uses it" % name)

    def test_every_patch_build_shadow_runs_is_in_the_list(self):
        body = function("build_shadow")
        run = re.findall(r'python3 "\$([A-Z_]+)"', body)
        self.assertIn("KEYBOARD_TOOL", run)
        listed = "\n".join(PATCH_LIST)
        for name in run:
            self.assertIn("$" + name, listed, "build_shadow runs $%s and SHADOW_PATCH_FILES does not list it: an update that changes it would not rebuild the shadow" % name)

    def test_everything_a_listed_python_script_imports_is_listed_too(self):
        listed = self.listed()
        names = {os.path.basename(p) for p in listed}
        for path in listed:
            if not path.endswith(".py"):
                continue
            with open(path) as f:
                for line in f:
                    m = re.match(r"(?:from|import)\s+([A-Za-z0-9_]+)", line)
                    if m and os.path.exists(os.path.join(ROOT, "tools", m.group(1) + ".py")):
                        self.assertIn(m.group(1) + ".py", names, "%s imports %s.py, which the list leaves out" % (os.path.basename(path), m.group(1)))

    def test_the_keyboard_patch_its_modules_and_the_theme_are_listed_and_every_listed_file_exists(self):
        listed = self.listed()
        names = [os.path.basename(p) for p in listed]
        for want in ("keyboard_patch.py", "notification_patch.py", "tokens.py", "theme.json"):
            self.assertIn(want, names)
        for p in listed:
            self.assertTrue(os.path.isfile(p), p)
        self.assertIn("$THEME_SRC/theme.json", "\n".join(PATCH_LIST))


class BootBindRebuild(unittest.TestCase):
    """verb_boot_bind rebuilds a shadow that is not current (a changed patch makes it so) and restarts LG's compositor once, so that it
    loads the new module. LumaTV's own compositor draws none of the module: a restart under it only costs a boot's frames."""

    def run_bind(self, shadow_current, live):
        with tempfile.TemporaryDirectory() as d:
            for rel, body in (("stage/compositor.qml", "x"), ("stage/wall.png", "x"), ("stage/luma/LumaHome/Home.qml", "x"), ("assets/home.xml", "x"),
                              ("assets/i18n/en.json", "x"), ("verified", "h\n")):
                p = os.path.join(d, rel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                with open(p, "w") as f:
                    f.write(body)
            stubs = "\n".join([
                f'COMPOSITOR_STAGE="{d}/stage/compositor.qml"\nWALLPAPER_STAGE="{d}/stage/wall.png"\nLUMA_STAGE="{d}/stage/luma"\nQML_MODULE=LumaHome',
                f'ASSETS="{d}/assets"\nSTOCK_ASSETS="{d}/stock-assets"\nVERIFIED="{d}/verified"\nBOOT_PENDING="{d}/pending"\nENV_FILE="{d}/none.env"\nENV_EXTRA="{d}/none"',
                'DISPLAY_NAME=LumaTV\nSURFACE_SVC=surface-manager',
                'take_mutation_lock() { return 0; }', 'staged_hash() { echo h; }', 'log_line() { :; }', 'ntfy_notify() { :; }',
                'homestub_drop() { :; }', 'remove_env() { :; }', 'write_env() { echo write_env; }', 'keyfilter_set() { :; }',
                'is_mounted() { return 0; }', 'all_binds_live() { return 0; }', 'record_bound_now() { :; }', 'count_mounts() { echo 3; }',
                'surface_start_uptime() { return 0; }', 'panel_is_active() { return 0; }', 'compositor_has_shadow() { return 0; }',
                f'restart_surface() {{ echo restart_surface >> "{d}/calls"; }}', 'home_back_after_restart() { echo home_back; }',
                'build_shadow() { echo build_shadow; return 0; }',
                f'shadow_current() {{ return {0 if shadow_current else 1}; }}',
                f'comp_live() {{ return {0 if live else 1}; }}',
            ])
            r = subprocess.run(["sh", "-c", stubs + "\n" + function("verb_boot_bind") + "verb_boot_bind\n"], capture_output=True, text=True, timeout=30)
            calls = open(os.path.join(d, "calls")).read() if os.path.exists(os.path.join(d, "calls")) else ""
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout + calls

    def test_a_stale_shadow_is_rebuilt_and_lgs_compositor_restarted_once(self):
        out = self.run_bind(shadow_current=False, live=False)
        self.assertEqual(out.count("build_shadow"), 1, out)
        self.assertEqual(out.count("restart_surface"), 1, out)
        self.assertIn('"bound":true', out)

    def test_under_lumatvs_compositor_the_rebuild_costs_no_restart(self):
        out = self.run_bind(shadow_current=False, live=True)
        self.assertEqual(out.count("build_shadow"), 1, out)
        self.assertNotIn("restart_surface", out)
        self.assertNotIn("home_back", out)
        self.assertIn('"bound":true', out)

    def test_a_current_shadow_is_left_alone_either_way(self):
        for live in (False, True):
            out = self.run_bind(shadow_current=True, live=live)
            self.assertNotIn("build_shadow", out)
            self.assertNotIn("restart_surface", out)
            self.assertIn('"bound":true', out)


if __name__ == "__main__":
    unittest.main()
