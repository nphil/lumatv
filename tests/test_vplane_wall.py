"""Run with: cd tests && python3 -m unittest test_vplane_wall   (needs Xvfb and PyQt5; the end-to-end case also needs zig for vplane/)

Home's moving wallpaper on the video plane (docs/research/videoplane-saver-result.md, "The moving wallpaper"): what LumaHome asks the player
(panels/../WallVideo.qml), when, and what it does with the answer. The QML host runs the real LumaHome; the player is a stand-in that speaks
the mailbox (a request file the shell writes, a status file the player writes) and can be told to play, refuse, say nothing or go quiet.

What must hold: with the owner's switch off (it is, by default) Home is what it was and nothing runs; with it on and no player or no clip Home
is what it was at once; with a player it asks for the clip only while Home is on screen under the conditions the clip is good for (the dark
material, the clip's wallpaper and look, no saver, LumaTV's compositor), draws no wallpaper of its own once the picture is on the plane and
the compositor says the hole is there, and whenever the player fails, goes quiet, or Home stops wanting it, the GPU wallpaper is drawn
again at once and the player is asked to stop after Home has had time to draw it.
"""
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

from test_plugins_surfaces import AWAKE, SAVER, Surfaces, js, probe as _probe, probes, publish, READY
from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SHIPPED = os.path.join(ROOT, "payload", "vplane", "clips")


def probe(at, label, body):
    return _probe(at, label, body, find="root")


class FakeWall:
    """The player's end of the mailbox, as far as Home can tell: it reads `req.json` and writes `status.json`, with the role it plays for."""

    def __init__(self, mailbox, mode="play", answer_ms=120):
        self.dir, self.mode, self.answer_ms = mailbox, mode, answer_ms
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir)
        self.requests, self.seen = [], None
        self.state, self.role, self.last_seq, self.reason = "idle", "", 0, ""
        self.running, self.quiet = True, False
        self.status("idle")
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def status(self, state, reason="", role=None):
        self.state, self.reason = state, reason
        if role is not None:
            self.role = role
        doc = {"v": 1, "pid": 1, "ts": int(time.time() * 1000), "state": state, "reason": reason, "seq": self.last_seq,
               "clip": "wall-silk" if state in ("opening", "playing") else "", "clips": 6, "walls": 1,
               "role": self.role if state in ("opening", "playing") else ""}
        tmp = os.path.join(self.dir, "status.json.tmp")
        with open(tmp, "w") as f:
            json.dump(doc, f)
        os.replace(tmp, os.path.join(self.dir, "status.json"))

    def loop(self):
        beat, answer_at = time.monotonic(), None
        while self.running:
            time.sleep(0.02)
            try:
                with open(os.path.join(self.dir, "req.json")) as f:
                    text = f.read()
            except OSError:
                text = ""
            if text and text != self.seen:
                self.seen = text
                try:
                    req = json.loads(text)
                except ValueError:
                    continue
                self.requests.append(req)
                role = req.get("role", "saver")
                if req["op"] == "start":
                    self.last_seq = req["seq"]
                    if self.mode in ("play", "quiet"):
                        self.role = role
                        answer_at = time.monotonic() + self.answer_ms / 1000
                    elif self.mode == "error":
                        self.status("error", "no pipeline")
                elif req["op"] == "stop" and (self.state not in ("opening", "playing") or role == self.role):
                    self.last_seq = req["seq"]
                    answer_at = None
                    self.status("idle", "stopped")
                elif req["op"] == "renew" and self.state == "playing" and not self.quiet and role == self.role:
                    self.status("playing")
            if answer_at and time.monotonic() >= answer_at:
                answer_at = None
                self.status("playing")
                if self.mode == "quiet":
                    self.quiet = True
            if self.state == "playing" and not self.quiet and time.monotonic() - beat > 0.5:
                beat = time.monotonic()
                self.status("playing")

    def ops(self):
        return [r["op"] for r in self.requests]

    def close(self):
        self.running = False
        self.thread.join(2)
        shutil.rmtree(self.dir, ignore_errors=True)


def manifest_dir():
    """A clips directory the way the set has it (`wall.json` is all Home reads of it), made from the shipped manifest."""
    d = tempfile.mkdtemp(prefix="wall-clips-")
    shutil.copy(os.path.join(SHIPPED, "wall.json"), d)
    return d


def configure(at, mailbox, clips, props=None):
    """JS: Home's wall controller reads the test's own mailbox and clips directory, with the timings a test shortens."""
    body = ("var w = root.videoWall; w.dirUrl = %s; w.clipsUrl = %s; w.loadManifest(); var c = %s; for (var k in c) w[k] = c[k];"
            % (json.dumps("file://" + mailbox + "/"), json.dumps("file://" + clips + "/"), json.dumps(props or {})))
    return js(at, body)


SNAP = ("var w = s.videoWall; return {state: w.state, reason: w.reason, up: s.videoWallUp, wanted: s.videoWallWanted, gpu: s.gpuWall, timers: w.timersRunning, "
        "pick: !!s.wallPick.entry, why: s.wallPick.why, tint: s.wallGlassTint, tick: s.frameTick};")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class WallVideo(Surfaces):
    MINE = Surfaces.MINE + ("WallVideo.qml", "LumaHome.qml")
    VIDEO = {"wallpaperMotion": {"source": "video"}}
    PROPS = {"startMs": 1500, "renewMs": 700, "staleMs": 2500, "settleMs": 150, "endMs": 150, "retryMs": 700}
    ARGS = ("--prop", "videoWallBackend=own", "--prop", "videoWallHole=true")

    def setUp(self):
        super().setUp()
        self.mailbox = tempfile.mkdtemp(prefix="wall-mb-")
        self.clips = manifest_dir()
        self.addCleanup(shutil.rmtree, self.mailbox, True)
        self.addCleanup(shutil.rmtree, self.clips, True)

    def player(self, **kw):
        p = FakeWall(self.mailbox, **kw)
        self.addCleanup(p.close)
        return p

    def host(self, theme, events, wait=6500, args=None, allow=()):
        at = READY + 300
        rc, out = self.run_host(theme, *(args if args is not None else self.ARGS), *configure(at - 200, self.mailbox, self.clips, self.PROPS), *events(at), "--wait", str(at + wait))
        self.assertEqual(self.own_warnings(out, allow=allow), [], out[-3000:])
        return probes(out)

    def test_switch_off_nothing_runs_and_the_gpu_draws(self):
        p = self.player()
        got = self.host({}, lambda at: probe(at + 2500, "off", SNAP))
        self.assertEqual(got["off"]["state"], "off")
        self.assertEqual((got["off"]["wanted"], got["off"]["up"], got["off"]["gpu"], got["off"]["timers"]), (False, False, 1, False))
        self.assertEqual(p.requests, [], "nothing was asked of the player")

    def test_the_picture_comes_up_the_gpu_wallpaper_fades_and_the_tick_slows_to_the_clock(self):
        p = self.player()
        got = self.host(self.VIDEO, lambda at: [*probe(at + 150, "early", SNAP), *probe(at + 2600, "up", SNAP), *probe(at + 3000, "t1", "return s.frameTick;"),
                                                *probe(at + 5000, "t2", "return s.frameTick;")])
        self.assertEqual(p.ops()[0], "start")
        self.assertEqual(p.requests[0]["role"], "wall")
        self.assertEqual(p.requests[0]["clip"], "wall-silk")
        self.assertEqual(got["early"]["gpu"], 1, "the GPU wallpaper stays until the picture is up")
        self.assertEqual((got["up"]["state"], got["up"]["up"], got["up"]["gpu"]), ("playing", True, 0), got["up"])
        self.assertLessEqual(got["t2"] - got["t1"], 3, "the tick is the clock's: about once a second while the clip plays, not ten")
        self.assertIn("renew", p.ops(), "the player is renewed while it is wanted")

    def test_without_the_hole_the_wallpaper_is_not_taken_away(self):
        self.player()
        got = self.host(self.VIDEO, lambda at: probe(at + 2600, "nohole", SNAP), args=("--prop", "videoWallBackend=own"))
        self.assertEqual((got["nohole"]["state"], got["nohole"]["up"], got["nohole"]["gpu"]), ("playing", False, 1), got["nohole"])

    def test_under_lgs_compositor_it_is_not_offered(self):
        p = self.player()
        got = self.host(self.VIDEO, lambda at: probe(at + 2000, "lg", SNAP), args=())
        self.assertEqual((got["lg"]["wanted"], got["lg"]["state"], got["lg"]["gpu"]), (False, "off", 1))
        self.assertEqual(p.requests, [])

    def test_a_player_that_refuses_leaves_the_gpu_wallpaper_and_is_told_to_stop(self):
        p = self.player(mode="error")
        got = self.host(self.VIDEO, lambda at: probe(at + 1200, "failed", SNAP))
        self.assertEqual((got["failed"]["state"], got["failed"]["reason"], got["failed"]["up"], got["failed"]["gpu"]), ("failed", "no pipeline", False, 1))
        self.assertEqual(p.ops()[:2], ["start", "stop"])

    def test_no_player_is_off_and_asked_again(self):
        got = self.host(self.VIDEO, lambda at: probe(at + 600, "none", SNAP))
        self.assertEqual((got["none"]["state"], got["none"]["reason"], got["none"]["gpu"]), ("off", "no player", 1))

    def test_a_player_that_goes_quiet_is_given_up(self):
        p = self.player(mode="quiet")
        got = self.host(self.VIDEO, lambda at: [*probe(at + 1200, "up", SNAP), *probe(at + 4500, "quiet", SNAP)])
        self.assertEqual(got["up"]["state"], "playing")
        self.assertEqual((got["quiet"]["state"], got["quiet"]["reason"], got["quiet"]["up"], got["quiet"]["gpu"]), ("failed", "the player went quiet", False, 1), got["quiet"])
        self.assertIn("stop", p.ops())

    def test_a_look_the_clip_is_not_made_for_draws_the_gpu_wallpaper(self):
        p = self.player()
        change = js(READY + 300 + 2500, "root.patchTheme({wallpaperMotion: {amplitude: 9}});")
        got = self.host(self.VIDEO, lambda at: [*probe(at + 2000, "up", SNAP), *change, *probe(at + 3600, "changed", SNAP)])
        self.assertTrue(got["up"]["up"])
        self.assertEqual((got["changed"]["pick"], got["changed"]["up"], got["changed"]["gpu"]), (False, False, 1))
        self.assertIn("the look is not the clip's (amplitude)", got["changed"]["why"])
        self.assertEqual(p.ops()[-1], "stop", "the player was told to stop once the GPU wallpaper was back")

    def test_another_wallpaper_or_the_light_material_has_no_clip(self):
        self.player()
        other = self.host({"wallpaperMotion": {"source": "video"}, "wallpaper": "/media/x/other.png"}, lambda at: probe(at + 1500, "other", SNAP), allow=("other.png",))
        self.assertEqual((other["other"]["pick"], other["other"]["why"], other["other"]["state"]), (False, "no clip for this wallpaper", "off"))
        light = self.host({"wallpaperMotion": {"source": "video"}, "material": "light"}, lambda at: probe(at + 1500, "light", SNAP))
        self.assertEqual((light["light"]["wanted"], light["light"]["state"]), (False, "off"))

    def test_the_saver_ends_it_and_home_leaving_the_screen_ends_it(self):
        p = self.player()
        got = self.host(self.VIDEO, lambda at: [*probe(at + 2000, "up", SNAP), *publish(at + 2100, "screen", SAVER), *probe(at + 3200, "saver", SNAP),
                                                *publish(at + 3300, "screen", AWAKE), *probe(at + 5200, "back", SNAP)])
        self.assertTrue(got["up"]["up"])
        self.assertEqual((got["saver"]["wanted"], got["saver"]["up"], got["saver"]["gpu"], got["saver"]["state"]), (False, False, 1, "off"), got["saver"])
        self.assertIn("stop", p.ops())
        self.assertEqual(got["back"]["state"], "playing", "Home is back: the clip is asked for again")

    def test_hiding_home_stops_the_player_and_nothing_runs_while_it_is_hidden(self):
        p = self.player()
        got = self.host(self.VIDEO, lambda at: [*probe(at + 2000, "up", SNAP), *js(at + 2100, "root.hostActive = false;"), *probe(at + 3000, "hidden", SNAP)])
        self.assertTrue(got["up"]["up"])
        self.assertEqual((got["hidden"]["state"], got["hidden"]["up"], got["hidden"]["timers"]), ("off", False, False))
        self.assertEqual(p.ops()[-1], "stop")

    def test_the_tint_glass_is_flat_and_as_opaque_as_the_clip_says(self):
        self.player()
        theme = {"wallpaperMotion": {"source": "video", "glass": "tint"}}
        fill = ("var o = Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\nQtObject { property real fill: Lu.homeFill }', root); return {fill: o.fill, tint: s.wallGlassTint};")
        got = self.host(theme, lambda at: [*probe(at - 150, "before", fill), *probe(at + 3000, "after", fill)])
        with open(os.path.join(SHIPPED, "wall.json")) as f:
            want = json.load(f)["walls"][0]["glass"]["alpha"]
        self.assertEqual(got["before"], {"fill": 1, "tint": False})
        self.assertEqual(got["after"]["tint"], True)
        self.assertAlmostEqual(got["after"]["fill"], want, places=3)


@unittest.skipUnless(DISPLAY and HAVE_QT and shutil.which("ffmpeg"), "needs Xvfb, PyQt5 and ffmpeg")
class WallRender(unittest.TestCase):
    """Pictures of Home over the plane (tests/qmlhost/render_wall.py): the plane is a picture behind the QML, a frame decoded from the shipped clip,
    and for the glass the brightest every poster frame of the clip has at each point."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, os.path.join(HERE, "qmlhost"))
        sys.path.insert(0, os.path.join(ROOT, "tools"))
        import render_wall
        cls.rw = render_wall
        cls.dir = tempfile.mkdtemp(prefix="wall-shots-")
        cls.frame, cls.worst = os.path.join(cls.dir, "frame.png"), os.path.join(cls.dir, "worst.png")
        render_wall.decoded_frame(900, cls.frame)
        render_wall.worst_frame(cls.worst)
        env = dict(os.environ, DISPLAY=DISPLAY)
        os.environ["DISPLAY"] = DISPLAY
        cls.shots = {}
        for name, plane in (("gpu", cls.frame), ("backdrop", cls.frame), ("tint-worst", cls.worst)):
            theme, _ = render_wall.SCENES[name]
            cls.shots[name] = render_wall.render(os.path.join(cls.dir, name + ".png"), theme, bg=plane)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def px(self, path):
        from PIL import Image
        return Image.open(path).convert("RGB")

    OUTSIDE = ((8, 540), (1912, 540), (960, 8), (960, 1072), (20, 300))       # where Home draws nothing but the wallpaper: the margins of the screen

    def test_where_home_draws_no_wallpaper_the_plane_shows_through(self):
        plane, video, gpu = self.px(self.frame), self.px(self.shots["backdrop"]), self.px(self.shots["gpu"])
        for p in self.OUTSIDE:
            self.assertEqual(video.getpixel(p), plane.getpixel(p), p)
        self.assertTrue(any(gpu.getpixel(p) != plane.getpixel(p) for p in self.OUTSIDE), "with the GPU wallpaper those points are Home's own picture")

    def test_the_glass_over_the_plane_is_drawn_and_is_the_glass_of_the_gpu_wallpaper(self):
        video, gpu = self.px(self.shots["backdrop"]), self.px(self.shots["gpu"])
        # a pane's pixel: between the status bar's two ends (nothing but the pane's fill there), in the GPU's picture and over the plane
        for p in ((600, 85), (900, 85), (1200, 85)):
            self.assertNotEqual(video.getpixel(p), self.px(self.frame).getpixel(p), "the pane is drawn over the video")
            self.assertTrue(all(abs(a - b) <= 12 for a, b in zip(video.getpixel(p), gpu.getpixel(p))), (p, video.getpixel(p), gpu.getpixel(p)))

    def test_tint_glass_reads_over_the_brightest_the_clip_gets(self):
        """Lucent's distance contrast (7:1) for the ink over the pane's own fill, composited with the plane at its worst: every flat, dark pixel inside
        a card (its fill: no text, no icon) is measured."""
        import vplane_wall as vw
        import vplane_clips as vc
        ink, _ = vc.hexrgb(vw.read_theme()["dark"]["ink"])
        im = self.px(self.shots["tint-worst"])
        g = im.convert("L")
        cards = {"athome": (84, 398, 625, 590), "news": (688, 155, 1380, 590), "bar": (300, 70, 1650, 100)}
        for name, (x0, y0, x1, y1) in cards.items():
            worst, seen = 99.0, 0
            for y in range(y0, y1, 6):
                for x in range(x0, x1, 6):
                    near = [g.getpixel((x + dx, y + dy)) for dx in (-3, 0, 3) for dy in (-3, 0, 3)]
                    if max(near) - min(near) > 2 or vw.relative_luminance(im.getpixel((x, y))) > 0.2:
                        continue
                    worst = min(worst, vw.contrast(ink, im.getpixel((x, y))))
                    seen += 1
            self.assertGreater(seen, 200, name)
            self.assertGreaterEqual(worst, 7.0, "%s: the ink reads at the TV's distance over the worst plane" % name)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class WallTopic(unittest.TestCase):
    """The shell's end of the contract (compositor/shell/Shell.qml): this compositor has the wallpaper window under Home's, so Home is told the video
    wallpaper is available, and Home's hole is the compositor's `wall` topic, nothing else."""

    def test_the_wall_topic_is_homes_hole(self):
        from test_shell_client import HOME, ShellClient
        rc, out = ShellClient.host(
            self, "--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}",
            "--eval", "4000:JSON.stringify(home.state().wall)",
            "--push", "4500:wall:{\"hole\":true}", "--eval", "5500:JSON.stringify(home.state().wall)",
            "--push", "6000:wall:{\"hole\":false}", "--eval", "7000:JSON.stringify(home.state().wall)", wait=7500)
        got = ShellClient.evals(self, out)
        self.assertEqual(got, [{"backend": "own", "hole": False, "up": False}, {"backend": "own", "hole": True, "up": False},
                               {"backend": "own", "hole": False, "up": False}], out[-2500:])
        self.assertEqual(rc, 0, "\n".join(l for l in out.splitlines() if l.startswith("[warning]") or l.startswith("[critical]"))[-2000:])


if __name__ == "__main__":
    unittest.main()
