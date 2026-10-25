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
          echo '{"ok":true,"qml":"/usr/palm/applications/com.webos.app.inputcommon/qml","staged":true,"mounted":false}' ;;
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

    def test_apply_mounts_again_while_the_switch_is_on_with_the_other_surfaces(self):
        apply_text = function("verb_apply")
        line = '[ -f "$INPUTCOMMON_ON" ] && { inputcommon_set on ||'
        self.assertIn(line, apply_text)
        self.assertLess(apply_text.index('[ -f "$NOTI_ON" ] && { noti_set on'), apply_text.index(line))
        self.assertLess(apply_text.index(line), apply_text.index("compositor_changed=1"))
        self.assertLess(apply_text.index('[ -f "$WEBGLASS_ON" ] && { webglass_set on'), apply_text.index(line))

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

    def build(self, tool=KEYBOARD_TOOL, theme=THEME_SRC):
        d = self.tmp.name
        script = (f'STOCK_QML_MODULE="{self.stock}"\nCOMPOSITOR_STAGE="{self.comp_stage}"\nLUMA_STAGE="{d}/luma"\nQML_MODULE=LumaHome\n'
                  f'SHADOW_ROOT="{self.shadow}"\nMODULE_RELPATH=views/fullscreen/StarfishFullscreenContainer.qml\n'
                  f'SHADOW_STOCK_FP="{d}/stock.fp"\nSHADOW_SELF_FP="{d}/self.fp"\nKEYBOARD_TOOL="{tool}"\nTHEME_SRC="{theme}"\nLOGFILE="{self.log}"\n'
                  f'log_line() {{ echo "$1" >> "{self.log}"; }}\n'
                  + function("stock_module_fp") + function("shadow_self_fp") + function("build_shadow") + "build_shadow; echo rc=$?\n")
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


if __name__ == "__main__":
    unittest.main()
