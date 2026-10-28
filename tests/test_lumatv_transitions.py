"""Run with: python3 -m unittest tests.test_lumatv_transitions

`lumatv transitions on|off|status` (tools/lumatv): smooth transitions, theme.motion.transitions. A switch and nothing more: it writes the owner's
theme.json through the theme tool and says whether LumaTV's compositor is the live one (LG's cuts, whatever the key says). The default is off and
the three places that say so agree: the shipped theme, the compositor's own default (`Settings::default` in transition.rs, for a theme without the
key) and the tool. The functions are the tool's own, run in a shell against a temporary state.
"""
import json
import os
import re
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")
TOOL = os.path.join(TOOLS, "lumatv")
THEME_TOOL = os.path.join(TOOLS, "theme.py")


def function(name):
    out, on = [], False
    with open(TOOL) as f:
        for line in f.read().splitlines(True):
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")


FUNCTIONS = ("json_escape", "emit_error", "transitions_state", "transitions_set", "transitions_json", "verb_transitions")


class Verb(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.theme_dir = os.path.join(self.tmp.name, "state", "theme")
        os.makedirs(self.theme_dir)

    def theme(self):
        with open(os.path.join(self.theme_dir, "theme.json")) as f:
            return json.load(f)

    def put(self, doc):
        with open(os.path.join(self.theme_dir, "theme.json"), "w") as f:
            json.dump(doc, f)

    def run_verb(self, word, live=False):
        script = (f'THEME_DIR="{self.theme_dir}"\nTHEME_TOOL="{THEME_TOOL}"\n'
                  'stage_theme() { [ -f "$THEME_DIR/theme.json" ] || echo "{}" > "$THEME_DIR/theme.json"; }\n'
                  + ("comp_live() { return 0; }\n" if live else "") + "".join(function(n) for n in FUNCTIONS) + f'verb_transitions "{word}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout.strip()

    def test_off_until_it_is_turned_on_and_it_says_whose_compositor_it_is(self):
        rc, out = self.run_verb("status")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "transitions": {"on": False, "compositor": "lg"}}))
        rc, out = self.run_verb("status", live=True)
        self.assertEqual(json.loads(out)["transitions"], {"on": False, "compositor": "own"})

    def test_on_and_off_write_the_owners_theme_and_change_nothing_else(self):
        self.put({"accent": "wallpaper", "motion": {"reduced": True}})
        rc, out = self.run_verb("on", live=True)
        self.assertEqual(rc, 0, out)
        self.assertEqual(json.loads(out)["transitions"], {"on": True, "compositor": "own"})
        self.assertEqual(self.theme(), {"accent": "wallpaper", "motion": {"reduced": True, "transitions": True}})
        rc, out = self.run_verb("off")
        self.assertEqual(json.loads(out)["transitions"]["on"], False)
        self.assertEqual(self.theme(), {"accent": "wallpaper", "motion": {"reduced": True, "transitions": False}})

    def test_on_with_no_theme_file_stages_one(self):
        rc, out = self.run_verb("on")
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.theme(), {"motion": {"transitions": True}})

    def test_a_word_it_does_not_know_is_an_error(self):
        rc, out = self.run_verb("sideways")
        self.assertEqual(rc, 1)
        self.assertFalse(json.loads(out)["ok"])


class Defaults(unittest.TestCase):
    def test_the_shipped_theme_and_the_compositor_agree_that_it_is_off(self):
        with open(os.path.join(ROOT, "payload", "theme", "theme.json")) as f:
            shipped = json.load(f)
        self.assertIs(shipped["motion"]["transitions"], False)
        with open(os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "transition.rs")) as f:
            rust = f.read()
        m = re.search(r"impl Default for Settings \{.*?Settings \{ enabled: (true|false)", rust, re.S)
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), "false")

    def test_the_settings_page_has_the_row_and_writes_the_same_key(self):
        with open(os.path.join(ROOT, "payload", "compositor", "lumatv", "settings", "PageLook.qml")) as f:
            qml = f.read()
        self.assertIn('label: "Smooth transitions"', qml)
        self.assertIn("motion: { transitions: v }", qml)

    def test_the_shell_binds_home_to_the_compositors_word(self):
        with open(os.path.join(ROOT, "compositor", "shell", "Shell.qml")) as f:
            qml = f.read()
        self.assertIn('Link.topics.transitions', qml)
        self.assertIn('property: "smoothLaunch"', qml)


if __name__ == "__main__":
    unittest.main()
