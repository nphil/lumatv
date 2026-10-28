"""Run with: python3 -m unittest tests.test_cover_splash_patch   (the census copy of LG's QML at /data/home/lgtv/uiinv is needed; skipped without it)

The three things that bring LG's own screens into LumaTV's look through the shadow module, and the switch that darkens the panel in standby:

* tools/cover_patch.py: LG's startup cover (CoverUI's MainView.qml) with LumaTV's picture and LG's sequencing,
* tools/splash_patch.py: LG's launch splash and launch spinner views,
* tools/lumatv: `cover`, `splash` (flags that build_shadow reads) and `standbyscreen` (the TV's own `general.lifeOnScreenMode`).

Every patch is anchored on LG's text and must fail, loudly and writing nothing, when LG's file is not what it expects; the tests run the tools
over LG's real files and over files that have lost an anchor. What the patched screens look like is tests/qmlhost/coverview.py and
splashview.py; whether the shadow module loads them is the set's (docs/research/surfaces-status.md lists the checks).
"""
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")
sys.path.insert(0, TOOLS)
sys.path.insert(0, HERE)

CENSUS = "/data/home/lgtv/uiinv/qml/qml"
COVER = os.path.join(CENSUS, "CoverUI")
VIEWS = os.path.join(CENSUS, "WebOSCompositor", "views")
MAIN = os.path.join("UserInterfaceLayer", "Containers", "MainView.qml")
TOOL = os.path.join(ROOT, "tools", "lumatv")

PAL = {"sheetTop": "#EE2A2738", "sheetBottom": "#EE1F1D2E", "row": "#59FFFFFF", "focusEdge": "#47FFFFFF", "hairline": "#1FFFFFFF", "ink": "#FFF2EFF6",
       "inkDim": "#A8F2EFF6", "accent": "#FFA79BF5", "scrim": "#6B120E1A"}


def function(name):
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


@unittest.skipUnless(os.path.isdir(COVER), "needs the census copy of LG's QML")
class CoverPatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.module = os.path.join(self.tmp.name, "CoverUI")
        shutil.copytree(COVER, self.module)

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, module=None):
        return subprocess.run([sys.executable, os.path.join(TOOLS, "cover_patch.py"), module or self.module, json.dumps(PAL), "LumaTV"], capture_output=True, text=True)

    def read(self, rel=MAIN):
        with open(os.path.join(self.module, rel)) as f:
            return f.read()

    def test_the_picture_is_lumatvs_and_lgs_sequencing_and_stolen_device_screen_are_untouched(self):
        before = self.read()
        r = self.run_tool()
        self.assertEqual(r.returncode, 0, r.stderr)
        after = self.read()
        self.assertTrue(after.startswith("// Luma: patched, see tools/cover_patch.py\n"))
        self.assertIn('color: "#06050B"', after)
        for kept in ("id: blockUI", "id: fade", "onForegroundAppReadyChanged", "onCoreBootDoneChanged", "interfaces.application.getFirstAppInformation()",
                     "interfaces.application.firstAppLaunchDone()", "onScreenSaverTimestampChanged", "rootWindow.close()"):
            self.assertIn(kept, before)
            self.assertIn(kept, after, kept + " is LG's sequencing and stays")
        # the close animation fades `logoText`, so it must still be there, once, as must the title the name goes in
        self.assertEqual(after.count("id: logoText"), 1)
        self.assertEqual(after.count("id: title"), 1)
        self.assertNotIn("font.pixelSize: styleSheet.getRValue(120)", after, "LG's 120 px name is gone")
        self.assertNotIn("stringSheet.loading", after, "and so is its Loading...")
        self.assertIn("Starting where you left off", after)
        for pic in ("cover-input.png", "cover-app.png"):
            self.assertTrue(os.path.isfile(os.path.join(self.module, os.path.dirname(MAIN), pic)), pic)

    def test_nothing_but_main_view_changes(self):
        def tree(root):
            out = {}
            for d, _, names in os.walk(root):
                for n in names:
                    p = os.path.join(d, n)
                    with open(p, "rb") as f:
                        out[os.path.relpath(p, root)] = f.read()
            return out
        before = tree(self.module)
        self.assertEqual(self.run_tool().returncode, 0)
        after = tree(self.module)
        changed = sorted(k for k in after if before.get(k) != after[k])
        self.assertEqual(changed, sorted([MAIN, os.path.join(os.path.dirname(MAIN), "cover-input.png"), os.path.join(os.path.dirname(MAIN), "cover-app.png")]))
        self.assertEqual(sorted(set(before) - set(after)), [])

    def test_a_file_that_is_already_patched_or_has_lost_an_anchor_is_refused_and_left_as_it_was(self):
        self.assertEqual(self.run_tool().returncode, 0)
        again = self.read()
        r = self.run_tool()
        self.assertEqual(r.returncode, 1)
        self.assertIn("already patched", r.stderr)
        self.assertEqual(self.read(), again)
        # an LG update that changes the first Column: the anchor is not there, nothing is written
        fresh = os.path.join(self.tmp.name, "fresh")
        shutil.copytree(COVER, fresh)
        path = os.path.join(fresh, MAIN)
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("spacing: 10\n        visible: !blockUI.visible", "spacing: 12\n        visible: !blockUI.visible", 1))
        r = self.run_tool(fresh)
        self.assertEqual(r.returncode, 1)
        self.assertIn("was not found once", r.stderr)
        with open(path) as f:
            self.assertEqual(f.read(), text.replace("spacing: 10\n        visible: !blockUI.visible", "spacing: 12\n        visible: !blockUI.visible", 1))
        self.assertFalse(os.path.exists(os.path.join(fresh, os.path.dirname(MAIN), "cover-input.png")), "no picture is left beside a file that was refused")

    def test_a_palette_without_a_colour_the_block_uses_is_refused(self):
        pal = dict(PAL)
        del pal["focusEdge"]
        r = subprocess.run([sys.executable, os.path.join(TOOLS, "cover_patch.py"), self.module, json.dumps(pal), "LumaTV"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("focusEdge", r.stderr)


@unittest.skipUnless(os.path.isdir(VIEWS), "needs the census copy of LG's QML")
class SplashPatch(unittest.TestCase):
    NAMES = ("StarfishImageSplash.qml", "StarfishSpinnerView.qml")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.views = os.path.join(self.tmp.name, "views")
        os.makedirs(self.views)
        for n in self.NAMES:
            shutil.copy(os.path.join(VIEWS, n), self.views)

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self):
        return subprocess.run([sys.executable, os.path.join(TOOLS, "splash_patch.py"), self.views, json.dumps(PAL)], capture_output=True, text=True)

    def read(self, name):
        with open(os.path.join(self.views, name)) as f:
            return f.read()

    def test_an_app_with_no_picture_gets_the_launch_screen_and_one_with_its_own_keeps_it(self):
        r = self.run_tool()
        self.assertEqual(r.returncode, 0, r.stderr)
        splash = self.read(self.NAMES[0])
        self.assertTrue(splash.startswith("// Luma: patched, see tools/splash_patch.py\n"))
        self.assertIn("root.lumaCard = !img;", splash, "the card is for an empty `img`, which is when LG draws its grey default")
        self.assertIn("visible: root.lumaCard && wiper.state === \"open\"", splash)
        # LG's own picture, its wiper and its timers are as they were
        for kept in ("Eos.Wiper {", "id: splashTimer", "function close(forced)", "theImage.sourceProxy = imgPath;", "wiper.open();"):
            self.assertIn(kept, splash, kept)
        self.assertEqual(splash.count("function open(appId, img)"), 1)
        self.assertIsNone(re.search(r"@@\w+@@", splash), "every palette name is filled in")

    def test_the_spinners_disc_and_gif_are_replaced_and_its_timers_and_input_block_are_kept(self):
        self.assertEqual(self.run_tool().returncode, 0)
        spin = self.read(self.NAMES[1])
        self.assertNotIn("AnimatedImage", spin)
        self.assertNotIn("movingImageSource", spin)
        self.assertEqual(spin.count("id: spinnerBackground"), 1)
        for kept in ("id: mouseCatcher", "id: waitTimer", "id: timeoutTimer", "id: runTimer", "function requestOpen(owner)", "root.closeView();"):
            self.assertIn(kept, spin, kept)
        self.assertIsNone(re.search(r"@@\w+@@", spin), "every palette name is filled in")

    def test_both_or_neither(self):
        # the spinner view has lost its anchor: the splash is not patched either
        path = os.path.join(self.views, self.NAMES[1])
        with open(path) as f:
            text = f.read()
        with open(path, "w") as f:
            f.write(text.replace("id: spinnerBackground", "id: spinnerDisc"))
        splash_before = self.read(self.NAMES[0])
        r = self.run_tool()
        self.assertEqual(r.returncode, 1)
        self.assertIn("disc was not found", r.stderr)
        self.assertEqual(self.read(self.NAMES[0]), splash_before)
        self.assertEqual(self.read(self.NAMES[1]), text.replace("id: spinnerBackground", "id: spinnerDisc"))

    def test_a_patched_file_is_not_patched_again(self):
        self.assertEqual(self.run_tool().returncode, 0)
        r = self.run_tool()
        self.assertEqual(r.returncode, 1)
        self.assertIn("already patched", r.stderr)


class Switches(unittest.TestCase):
    """The shell side: the flags build_shadow reads, the registry's way back, and `standbyscreen` against a make-believe settings service."""

    def test_the_two_shadow_switches_are_in_the_fingerprint_the_patch_list_and_revert(self):
        with open(TOOL) as f:
            text = f.read()
        fp = function("shadow_patch_fp")
        self.assertIn("COVER_ON", fp)
        self.assertIn("SPLASH_ON", fp)
        build = function("build_shadow")
        self.assertIn('python3 "$COVER_TOOL"', build)
        self.assertIn('python3 "$SPLASH_TOOL"', build)
        self.assertIn('rm -f "$COVER_ON" "$SPLASH_ON"', function("do_revert_internal"))
        listed = "\n".join(l for l in text.split("\n") if l.startswith("SHADOW_PATCH_FILES="))
        for f in ("$COVER_TOOL", "payload/cover/block.qml", "payload/cover/cover-input.png", "payload/cover/cover-app.png",
                  "$SPLASH_TOOL", "payload/splash/card.qml", "payload/splash/spinner.qml"):
            self.assertIn(f, listed, f + " is listed, so a change to it rebuilds the shadow")

    def run_standbyscreen(self, arg, start, refuse=False):
        """`verb_standbyscreen arg` with a luna-send that keeps `lifeOnScreenMode` in a file, starting at `start`. Returns (stdout, mode after, prev file)."""
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.makedirs(bindir)
            mode = os.path.join(d, "mode")
            if start is not None:
                with open(mode, "w") as f:
                    f.write(start)
            fake = os.path.join(bindir, "luna-send")
            write = 'echo \'{"returnValue":false}\'' if refuse else (
                'v=$(printf \'%s\' "$4" | sed -n \'s/.*"lifeOnScreenMode":"\\([^"]*\\)".*/\\1/p\'); printf \'%s\' "$v" > ' + mode + '; echo \'{"returnValue":true}\'')
            read = ('m=$(cat ' + mode + ' 2>/dev/null); if [ -n "$m" ]; then echo "{\\"returnValue\\":true,\\"settings\\":{\\"lifeOnScreenMode\\":\\"$m\\"}}"; '
                    'else echo \'{"returnValue":true,"settings":{}}\'; fi')
            with open(fake, "w") as f:
                f.write("#!/bin/sh\ncase \"$4\" in\n  *'\"settings\"'*) " + write + " ;;\n  *) " + read + " ;;\nesac\n")
            os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)
            script = (f'PATH="{bindir}:$PATH"\nSTATE="{d}/state"\nLOGFILE="{d}/log"\nSETTINGS_GET_URI=x\nSETTINGS_SET_URI=y\n'
                      'take_mutation_lock() { return 0; }\nlog_line() { :; }\n'
                      + "".join(function(n) for n in ("json_escape", "emit_error", "get_general_setting", "set_general_setting", "verb_standbyscreen"))
                      + 'STANDBYSCREEN_PREV="$STATE/standbyscreen.prev"\n' + f'verb_standbyscreen {arg}\n')
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            after = open(mode).read() if os.path.exists(mode) else None
            prev = os.path.join(d, "state", "standbyscreen.prev")
            return r.stdout.strip(), after, (open(prev).read().strip() if os.path.exists(prev) else None)

    def test_standbyscreen_off_darkens_the_panel_and_on_puts_back_what_was_there(self):
        out, after, prev = self.run_standbyscreen("off", "alwaysReady")
        self.assertEqual(json.loads(out), {"ok": True, "standbyscreen": "off", "was": "alwaysReady"})
        self.assertEqual((after, prev), ("off", "alwaysReady"), "the mode that was there is kept for `on`")
        out, after, _ = self.run_standbyscreen("status", "off")
        self.assertEqual(json.loads(out)["on"], True)
        out, after, _ = self.run_standbyscreen("status", "alwaysReady")
        self.assertEqual((json.loads(out)["on"], json.loads(out)["mode"]), (False, "alwaysReady"))

    def test_standbyscreen_off_twice_keeps_the_first_mode_and_on_without_one_is_the_settings_own_on(self):
        out, after, prev = self.run_standbyscreen("off", "off")
        self.assertEqual(json.loads(out)["standbyscreen"], "off")
        self.assertEqual(prev, None, "already off: there is nothing to put back, and nothing that was there is overwritten")
        out, after, _ = self.run_standbyscreen("on", "off")
        self.assertEqual((json.loads(out)["mode"], after), ("allEnabled", "allEnabled"), "Settings' own on (PageLook) when no mode was kept")

    def test_a_settings_service_that_refuses_is_an_error_and_nothing_is_claimed(self):
        out, after, _ = self.run_standbyscreen("off", "alwaysReady", refuse=True)
        self.assertFalse(json.loads(out)["ok"], out)
        self.assertEqual(after, "alwaysReady")


if __name__ == "__main__":
    unittest.main()
