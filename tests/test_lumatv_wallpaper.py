"""Run with: python3 -m unittest tests.test_lumatv_wallpaper

`lumatv wallpaper video|gpu|status` (tools/lumatv): Home's wallpaper source, theme.wallpaperMotion.source. It is a switch and nothing more: it writes
the owner's theme.json through the theme tool, installs nothing and starts nothing, and its status names what the video wallpaper still needs
(LumaTV's compositor, the player installed and running, a clip) so a `video` that has not come up is not a mystery. The functions are the tool's own,
run in a shell against a temporary state, the real theme tool, a stand-in for the player's script and for the compositor's `comp_live`.
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
with open(os.path.join(ROOT, "brand.json")) as _f:
    BRAND = json.load(_f)


def text():
    with open(TOOL) as f:
        return f.read()


def function(name):
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


FUNCTIONS = ("json_escape", "emit_error", "vplane_json", "wallpaper_source", "wallpaper_set", "wallpaper_json", "verb_wallpaper")

FAKE_SH = """#!/bin/sh
# the player's script: status is what the test put in $FAKE/vplane-status, else the not-installed object
case "$1" in
  status) if [ -e "$FAKE/vplane-status" ]; then cat "$FAKE/vplane-status"; else echo '{"installed":false,"role":false,"clips":0,"walls":0,"running":false,"pid":0}'; fi ;;
esac
"""


class Wallpaper(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.d = self.tmp.name
        self.state = os.path.join(self.d, "state")
        self.theme_dir = os.path.join(self.state, "theme")
        os.makedirs(self.theme_dir)
        self.app = os.path.join(self.d, "app")
        os.makedirs(os.path.join(self.app, "payload", "vplane"))
        self.sh_path = os.path.join(self.app, "payload", "vplane", "vplane.sh")
        with open(self.sh_path, "w") as f:
            f.write(FAKE_SH)
        self.mailbox = os.path.join(self.d, "mailbox")
        os.makedirs(self.mailbox)
        self.env = dict(os.environ, FAKE=self.d)

    def theme(self):
        with open(os.path.join(self.theme_dir, "theme.json")) as f:
            return json.load(f)

    def put_theme(self, doc):
        with open(os.path.join(self.theme_dir, "theme.json"), "w") as f:
            json.dump(doc, f)

    def vplane(self, doc):
        with open(os.path.join(self.d, "vplane-status"), "w") as f:
            json.dump(doc, f, separators=(",", ":"))      # compact, as vplane.sh prints it (the tool matches on `"running":true`)

    def run_verb(self, word, live=False):
        script = (f'APPDIR="{self.app}"\nSTATE="{self.state}"\nTHEME_DIR="{self.theme_dir}"\nTHEME_TOOL="{THEME_TOOL}"\n'
                  f'VPLANE_SH="{self.sh_path}"\nVPLANE_MAILBOX="{self.mailbox}"\nLOGFILE="{self.d}/log"\n'
                  f'stage_theme() {{ [ -f "$THEME_DIR/theme.json" ] || echo "{{}}" > "$THEME_DIR/theme.json"; }}\n'
                  f'write_tokens_js() {{ echo tokens >> "{self.d}/calls"; return 0; }}\n'
                  f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + (f"comp_live() {{ return 0; }}\n" if live else "")
                  + "".join(function(n) for n in FUNCTIONS) + f'verb_wallpaper "{word}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60, env=self.env)
        return r.returncode, r.stdout.strip()

    def test_the_default_is_the_gpu_and_the_status_says_what_video_would_need(self):
        rc, out = self.run_verb("status")
        self.assertEqual(rc, 0, out)
        w = json.loads(out)["wallpaper"]
        self.assertEqual((w["source"], w["ready"], w["compositor"]), ("gpu", False, "lg"))
        self.assertEqual(w["missing"], ["LumaTV's compositor is not the live one", "the video-plane player is not installed (lumatv vplane install)",
                                        "the video-plane player is not running (lumatv vplane start)", "the package has no wallpaper clip"])
        self.assertEqual(w["vplane"], {"installed": False, "role": False, "clips": 0, "walls": 0, "running": False, "pid": 0})
        self.assertEqual((w["player"]["state"], w["player"]["role"]), ("", ""))

    def test_video_and_gpu_write_the_owners_theme_and_change_nothing_else(self):
        self.put_theme({"accent": "wallpaper", "wallpaperMotion": {"amplitude": 6}})
        rc, out = self.run_verb("video")
        self.assertEqual(rc, 0, out)
        self.assertEqual(json.loads(out)["wallpaper"]["source"], "video")
        self.assertEqual(self.theme(), {"accent": "wallpaper", "wallpaperMotion": {"amplitude": 6, "source": "video"}})
        rc, out = self.run_verb("gpu")
        self.assertEqual(json.loads(out)["wallpaper"]["source"], "gpu")
        self.assertEqual(self.theme(), {"accent": "wallpaper", "wallpaperMotion": {"amplitude": 6, "source": "gpu"}})
        with open(os.path.join(self.d, "calls")) as f:
            self.assertEqual(f.read().split(), ["tokens", "tokens"], "the tokens are written as `theme set` writes them")

    def test_video_is_only_a_switch_it_installs_and_starts_nothing(self):
        rc, out = self.run_verb("video")
        self.assertEqual(rc, 0, out)
        self.assertEqual(sorted(os.listdir(self.state)), ["theme"])
        self.assertEqual(os.listdir(self.mailbox), [])

    def test_ready_when_the_compositor_is_ours_and_the_player_is_installed_running_and_has_a_clip(self):
        self.vplane({"installed": True, "role": True, "clips": 6, "walls": 1, "running": True, "pid": 42})
        with open(os.path.join(self.mailbox, "status.json"), "w") as f:
            json.dump({"v": 1, "state": "playing", "role": "wall", "reason": ""}, f, separators=(",", ":"))
        rc, out = self.run_verb("video", live=True)
        w = json.loads(out)["wallpaper"]
        self.assertEqual((w["source"], w["ready"], w["missing"], w["compositor"]), ("video", True, [], "own"))
        self.assertEqual((w["player"]["state"], w["player"]["role"]), ("playing", "wall"))

    def test_each_missing_piece_is_named_alone(self):
        full = {"installed": True, "role": True, "clips": 6, "walls": 1, "running": True, "pid": 42}
        cases = (({"installed": False}, False, "the video-plane player is not installed (lumatv vplane install)"),
                 ({"role": False}, False, "the video-plane player is not installed (lumatv vplane install)"),
                 ({"running": False}, False, "the video-plane player is not running (lumatv vplane start)"),
                 ({"walls": 0}, False, "the package has no wallpaper clip"))
        for change, _, why in cases:
            self.vplane(dict(full, **change))
            w = json.loads(self.run_verb("status", live=True)[1])["wallpaper"]
            self.assertEqual((w["ready"], w["missing"]), (False, [why]), change)
        self.vplane(full)
        w = json.loads(self.run_verb("status", live=False)[1])["wallpaper"]
        self.assertEqual(w["missing"], ["LumaTV's compositor is not the live one"])

    def test_an_unknown_word_is_refused_and_writes_nothing(self):
        rc, out = self.run_verb("sideways")
        self.assertEqual(rc, 1)
        self.assertEqual(json.loads(out), {"ok": False, "error": "wallpaper: video, gpu or status"})
        self.assertFalse(os.path.exists(os.path.join(self.theme_dir, "theme.json")))

    def test_a_theme_that_says_something_else_is_the_gpu(self):
        self.put_theme({"wallpaperMotion": {"source": "hologram"}})
        self.assertEqual(json.loads(self.run_verb("status")[1])["wallpaper"]["source"], "gpu")


if __name__ == "__main__":
    unittest.main()
