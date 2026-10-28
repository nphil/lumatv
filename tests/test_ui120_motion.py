"""Run with: cd tests && python3 -m unittest test_ui120_motion   (needs Xvfb and PyQt5; skipped without them)

The per-frame motion of the 120 Hz refresh mode (payload/compositor/lumatv/kit/FrameMotion.qml, docs/research/ui120-result.md) on the host's
Qt with the basic render loop, the loop the shell client runs on the set: a motion steps once per rendered frame (its steps are the window's
frames), keeps the frames coming by itself until it ends, lands on its target at its duration and asks for nothing after; the panel slide
(panels/PanelMotion.qml) takes that path at 120 Hz and its NumberAnimation at 60. The host renders in software and has no refresh to pace
the frames, so the motions here are long and the cadence is the host's; the set's cadence is the gate's to measure.
"""
import os
import re
import subprocess
import sys
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")
THEME = '{"perf":{"syncResidents":true},"wallpaperMotion":{"enabled":false}}'
T0 = 7000                                        # Home has loaded and warmed

# An Item in Home's window with a FrameMotion on that window and a frame counter; parked on a var property of Home for the later reads.
PROBE = ('(function(){var o=Qt.createQmlObject("import QtQuick 2.4; import QtQuick.Window 2.2; import \\"kit\\"; import \\"panels\\"; '
         'Item { id: w; property alias fm: m; property alias pm: p; property int frames: 0; FrameMotion { id: m; window: w.Window.window } '
         'PanelMotion { id: p; window: w.Window.window } Connections { target: w.Window.window; onFrameSwapped: w.frames++ } }", root, '
         'root.moduleDirUrl + "probe.qml"); root.miniSpec = o; return o ? 1 : 0})()')


def read(label, expr):
    return "(function(){var o=root.miniSpec; return %s + %s})()" % (repr(label + " "), expr)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class FrameMotionHost(unittest.TestCase):
    def host(self, hz, *steps, wait):
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen", QSG_RENDER_LOOP="basic", LIBGL_ALWAYS_SOFTWARE="1")
        cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--prop", "refreshHz=%d" % hz,
               "--theme", THEME, "--wait", str(wait), "--eval", "%d:%s" % (T0 - 1000, PROBE), *steps]
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        got = {}
        for m in re.finditer(r"\[eval\] \('(\S+) (.*)', (?:True|False)\)$", r.stdout, re.M):
            got[m.group(1)] = dict(kv.split("=") for kv in m.group(2).split())
        self.assertTrue(got, r.stdout[-3000:] + r.stderr[-1000:])
        return got

    def test_a_motion_steps_once_per_frame_keeps_the_frames_coming_and_stops_on_its_target(self):
        got = self.host(
            120,
            "--eval", "%d:%s" % (T0, "(function(){var o=root.miniSpec; o.frames=0; o.fm.start(100, 1500, 'linear'); return 'start value=' + o.fm.value + ' stepMs=' + o.fm.stepMs.toFixed(2)})()"),
            "--eval", "%d:%s" % (T0 + 500, read("mid", "'value=' + o.fm.value.toFixed(1) + ' steps=' + o.fm.steps + ' frames=' + o.frames + ' running=' + o.fm.running")),
            "--eval", "%d:%s" % (T0 + 1000, read("late", "'value=' + o.fm.value.toFixed(1) + ' steps=' + o.fm.steps + ' frames=' + o.frames")),
            "--eval", "%d:%s" % (T0 + 1700, read("done", "'value=' + o.fm.value.toFixed(1) + ' steps=' + o.fm.steps + ' frames=' + o.frames + ' running=' + o.fm.running")),
            "--eval", "%d:%s" % (T0 + 2700, read("after", "'steps=' + o.fm.steps + ' frames=' + o.frames")),
            wait=T0 + 3000)
        self.assertEqual(got["start"]["stepMs"], "8.33", "a step is one 120 Hz refresh of motion time")
        mid, late, done, after = got["mid"], got["late"], got["done"], got["after"]
        self.assertEqual(mid["running"], "true")
        self.assertGreater(int(mid["steps"]), 10, "frames keep coming while the motion runs: %s" % mid)
        self.assertEqual(mid["steps"], mid["frames"], "one step per frame")
        self.assertEqual(late["steps"], late["frames"])
        # linear over 1500 ms: the value follows the wall clock (the host's frames are not 8.33 ms apart, so the step count is resynced)
        self.assertAlmostEqual(float(mid["value"]), 33.3, delta=6, msg=str(mid))
        self.assertAlmostEqual(float(late["value"]), 66.7, delta=6, msg=str(late))
        self.assertEqual((done["value"], done["running"]), ("100.0", "false"), "on its target, done: %s" % done)
        # nothing of the motion asks for frames once it is done: the window's frames are the clock's and the host's own, a few a second
        self.assertEqual(after["steps"], done["steps"])
        self.assertLess(int(after["frames"]) - int(done["frames"]), 12, "no frame-driven loop left running: %s -> %s" % (done, after))

    def test_the_panel_slide_takes_the_per_frame_path_at_120_and_its_animation_at_60(self):
        slide = ["--eval", "%d:%s" % (T0, "(function(){var o=root.miniSpec; o.pm.presented = true; return 'open 1=1'})()"),
                 "--eval", "%d:%s" % (T0 + 40, read("early", "'progress=' + o.pm.progress.toFixed(2) + ' moving=' + o.pm.moving + ' perFrame=' + o.pm.perFrame")),
                 "--eval", "%d:%s" % (T0 + 400, read("shown", "'progress=' + o.pm.progress.toFixed(2) + ' moving=' + o.pm.moving + ' settled=' + o.pm.settled + ' steps=' + o.pm.frame.steps")),
                 "--eval", "%d:%s" % (T0 + 500, "(function(){var o=root.miniSpec; o.pm.presented = false; return 'close 1=1'})()"),
                 "--eval", "%d:%s" % (T0 + 900, read("gone", "'progress=' + o.pm.progress.toFixed(2) + ' moving=' + o.pm.moving + ' parked=' + o.pm.parked + ' steps=' + o.pm.frame.steps"))]
        fast = self.host(120, *slide, wait=T0 + 1200)
        self.assertEqual(fast["early"]["perFrame"], "true")
        self.assertEqual((fast["shown"]["progress"], fast["shown"]["moving"], fast["shown"]["settled"]), ("1.00", "false", "true"), str(fast["shown"]))
        self.assertGreater(int(fast["shown"]["steps"]), 0, "the slide in was stepped by frames")
        self.assertEqual((fast["gone"]["progress"], fast["gone"]["parked"]), ("0.00", "true"), str(fast["gone"]))
        self.assertGreater(int(fast["gone"]["steps"]), 0, "and the slide out")
        slow = self.host(60, *slide, wait=T0 + 1200)
        self.assertEqual(slow["early"]["perFrame"], "false")
        self.assertEqual((slow["shown"]["progress"], slow["shown"]["settled"], slow["shown"]["steps"]), ("1.00", "true", "0"), "the NumberAnimation, as before: %s" % slow["shown"])
        self.assertEqual((slow["gone"]["progress"], slow["gone"]["parked"], slow["gone"]["steps"]), ("0.00", "true", "0"), str(slow["gone"]))

    def test_homes_base_is_one_cached_texture_with_the_tick_held_while_a_panel_slides_at_120_and_the_slides_are_reported(self):
        """The quick panel through Home itself (`openLayer`): at 120 the item that holds the wallpaper, the widgets and the dock is an item
        layer from the panel's open to the end of its linger, the wallpaper tick holds while it slides, and each slide's steps land in `slides`
        (what the shell reports as its state); at 60 nothing is layered or held."""
        find = "(function(){var L=null;for(var i=0;i<root.children.length;i++){if(root.children[i].panelSliding!==undefined)L=root.children[i]}return L})()"
        steps = ["--eval", "%d:%s" % (T0 - 100, "'before cached=' + root.baseCached + ' tick=' + root.frameTick + ' slides=' + root.slides.length"),
                 "--eval", "%d:%s" % (T0, "(function(){root.openLayer('quick'); return 'open 1=1'})()"),
                 "--eval", "%d:%s" % (T0 + 60, "'opening cached=' + root.baseCached + ' sliding=' + %s.panelSliding + ' moving=' + %s.panelMoving + ' tick=' + root.frameTick" % (find, find)),
                 "--eval", "%d:%s" % (T0 + 100, "'mid moving=' + %s.panelMoving + ' tick=' + root.frameTick" % find),
                 "--eval", "%d:%s" % (T0 + 400, "'shown cached=' + root.baseCached + ' tick=' + root.frameTick + ' slides=' + root.slides.length"),
                 "--eval", "%d:%s" % (T0 + 500, "(function(){root.closeLayer(); return 'close 1=1'})()"),
                 "--eval", "%d:%s" % (T0 + 900, "(function(){var s=root.slides; var l=s.length?s[s.length-1]:{steps:0,ms:0,duration:0}; return 'closed cached=' + root.baseCached + ' sliding=' + %s.panelSliding + ' tick=' + root.frameTick + ' slides=' + s.length + ' steps=' + l.steps + ' ms=' + l.ms + ' for=' + l.duration})()" % find),
                 ]
        fast = self.host(120, *steps, wait=T0 + 1200)
        self.assertEqual(fast["before"]["cached"], "false")
        self.assertEqual((fast["opening"]["cached"], fast["opening"]["sliding"]), ("true", "true"), str(fast["opening"]))
        self.assertEqual((fast["opening"]["moving"], fast["mid"]["moving"]), ("true", "true"), "the slide is under way at both reads: %s %s" % (fast["opening"], fast["mid"]))
        self.assertEqual(fast["opening"]["tick"], fast["mid"]["tick"], "the wallpaper tick holds while the panel slides: %s %s" % (fast["opening"], fast["mid"]))
        self.assertEqual(fast["shown"]["slides"], "1", "the slide in was recorded")
        self.assertEqual((fast["closed"]["cached"], fast["closed"]["sliding"]), ("true", "true"), "parked within its linger, the texture is kept: %s" % fast["closed"])
        self.assertEqual((fast["closed"]["slides"], fast["closed"]["for"]), ("2", "110"), str(fast["closed"]))
        self.assertGreater(int(fast["closed"]["steps"]), 8, "the slide out stepped by frames: %s" % fast["closed"])
        self.assertGreater(int(fast["closed"]["tick"]), int(fast["shown"]["tick"]), "the tick runs again once the slide is over, the panel parked or at rest: %s %s" % (fast["shown"], fast["closed"]))
        slow = self.host(60, *steps, wait=T0 + 1200)
        self.assertEqual((slow["opening"]["cached"], slow["shown"]["cached"], slow["closed"]["cached"]), ("false", "false", "false"))
        self.assertGreater(int(slow["shown"]["tick"]), int(slow["opening"]["tick"]), "the tick never holds at 60: %s %s" % (slow["opening"], slow["shown"]))
        self.assertEqual((slow["shown"]["slides"], slow["closed"]["slides"]), ("0", "0"))
