"""Run with: python3 -m unittest tests.test_lumatv_keys

How tools/lumatv drives the keyboard's keys (tools/maliit.sh): the `keys` verb with a stand-in for the tool (so no mount and no restart happens),
the switch's flag and where it is read (the registry, apply's boot re-mount, revert), and the rule that the verb itself never touches a unit:
the one restart (of maliit-server.service) is the tool's, tested in tests.test_maliit_stage.
"""
import json
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "lumatv")


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
  on) if [ -e "$FAKE/on-fails" ]; then echo '{"ok":false,"error":"MaliitServer could not load a QML file; the patched keys are unmounted"}'; exit 1; fi
      [ -e "$FAKE/silent" ] && exit 0
      echo '{"ok":true,"keys":"on","restarted":true,"unit":"active"}' ;;
  off) if [ -e "$FAKE/off-fails" ]; then echo '{"ok":false,"error":"could not unmount the overlay"}'; exit 1; fi
       echo '{"ok":true,"keys":"off","restarted":true,"unit":"active"}' ;;
  status) [ -e "$FAKE/silent" ] && exit 0
          m=false; [ -e "$FAKE/mounted" ] && m=true
          echo '{"ok":true,"qml":"/usr/lib/maliit/plugins/qml","staged":true,"mounted":'$m',"unit":"active"}' ;;
esac
"""


class KeysVerb(unittest.TestCase):
    """`lumatv keys`: the same switch as `inputcommon` and `voice`, for the keyboard's keys (tools/maliit.sh)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.tool = os.path.join(self.d, "maliit.sh")
        with open(self.tool, "w") as f:
            f.write(FAKE_TOOL)
        self.flag = os.path.join(self.d, ".keys-on")

    def tearDown(self):
        self.tmp.cleanup()

    def run_verb(self, arg, lock=True):
        script = (f'FAKE="{self.d}"; export FAKE\nKEYS_TOOL="{self.tool}"\nKEYS_ON="{self.flag}"\nLOGFILE="{self.d}/log"\n'
                  f'take_mutation_lock() {{ {"return 0" if lock else "return 1"}; }}\n'
                  f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + function("json_escape") + function("emit_error") + function("keys_set") + function("verb_keys")
                  + f'verb_keys "{arg}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout.strip()

    def mark(self, name):
        open(os.path.join(self.d, name), "w").close()

    def test_on_runs_the_tool_keeps_the_flag_and_prints_the_tools_object(self):
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)["keys"]), (0, "on"))
        self.assertTrue(os.path.exists(self.flag))
        with open(os.path.join(self.d, "loglines")) as f:
            self.assertTrue(f.read().startswith("keys on: {"))

    def test_off_removes_the_flag_and_a_refused_on_or_off_leaves_it_as_it_was(self):
        open(self.flag, "w").close()
        self.assertEqual(json.loads(self.run_verb("off")[1])["keys"], "off")
        self.assertFalse(os.path.exists(self.flag))
        self.mark("on-fails")
        rc, out = self.run_verb("on")
        doc = json.loads(out)
        self.assertEqual((rc, doc["ok"]), (1, False))
        self.assertIn("could not load a QML file", doc["error"])                    # the tool's own words, as it said them
        self.assertFalse(os.path.exists(self.flag))
        open(self.flag, "w").close()
        self.mark("off-fails")
        self.assertEqual(self.run_verb("off")[0], 1)
        self.assertTrue(os.path.exists(self.flag))

    def test_a_tool_that_says_nothing_is_an_error_and_changes_no_flag(self):
        self.mark("silent")
        rc, out = self.run_verb("on")
        self.assertEqual((rc, json.loads(out)), (1, {"ok": False, "error": "tools/maliit.sh did not answer"}))
        self.assertFalse(os.path.exists(self.flag))

    def test_status_is_the_tools_object_with_the_switch_in_front(self):
        rc, out = self.run_verb("status")
        doc = json.loads(out)
        self.assertEqual((rc, doc["on"], doc["mounted"], doc["unit"]), (0, False, False, "active"))
        open(self.flag, "w").close()
        self.assertTrue(json.loads(self.run_verb("status")[1])["on"])

    def test_a_mutation_in_progress_and_a_wrong_word_are_errors(self):
        self.assertEqual(json.loads(self.run_verb("on", lock=False)[1])["error"], "another apply or revert is still running")
        self.assertEqual(json.loads(self.run_verb("sideways")[1])["error"], "usage: keys on|off|status")

    def test_wiring_apply_remounts_revert_unmounts_the_verb_is_dispatched_documented_and_never_restarts_a_unit_itself(self):
        t = text()
        apply_text = function("verb_apply")
        self.assertIn('[ -f "$KEYS_ON" ] && { keys_mounted || keys_set on || log_line "apply: could not mount the patched keyboard keys over LG\'s"; }', apply_text)
        self.assertLess(apply_text.index('"$VOICE_ON" ] && { voice_mounted'), apply_text.index('"$KEYS_ON" ] && { keys_mounted'))
        self.assertLess(apply_text.index('"$KEYS_ON" ] && { keys_mounted'), apply_text.index("compositor_changed=1"))
        self.assertIn('[ -f "$KEYS_ON" ] && { keys_set off; rm -f "$KEYS_ON"; }', function("do_revert_internal"))
        self.assertRegex(t, r"\n  status\|apply\|[^\n]*\|keyboard\|voice\|keys\|[^\n]*\)\n")
        self.assertIn('keys) verb_keys "${2:-status}"; exit $? ;;', t)
        self.assertRegex(t, r"\n#   keys on\|off\|status +->")
        self.assertIn('KEYS_ON="$STATE/.keys-on"', t)
        self.assertIn('KEYS_TOOL="$APPDIR/tools/maliit.sh"', t)
        for name in ("keys_set", "keys_mounted", "verb_keys"):
            self.assertNotRegex(function(name), r"systemctl|pkill|killall|restart_surface|luna-send", name)

    def test_the_keyboard_window_switch_and_the_keys_switch_are_independent(self):
        """`keyboard on` rebuilds the shadow module and restarts the compositor; `keys on` does neither, and the other way round."""
        self.assertNotIn("KEYS_ON", function("verb_keyboard"))
        self.assertNotIn("KEYBOARD_ON", function("verb_keys"))
        self.assertNotIn('"$APPDIR/tools/lumatv" apply', function("verb_keys"))


if __name__ == "__main__":
    unittest.main()
