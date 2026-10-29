"""Run with: cd tests && python3 -m unittest test_vplane_saver   (needs Xvfb and PyQt5; the end-to-end case also needs zig for vplane/)

The saver (panels/Screensaver.qml) and the video-plane player: what the saver asks, when, and what it does with the answer. The QML
host runs the real surface; the player is a stand-in that speaks the mailbox (vplane/ describes it: a request file the shell writes, a
status file the player writes) and can be told to play, to refuse, to say nothing or to go quiet; and once, the real player, built for
the host, with a stand-in for LG's decoder library and a small compositor, so the request the QML writes is the request the C reader
understands.

The behaviour that must hold: with the owner's switch off (it is, by default) the saver is what it was; with it on and no player on the TV
it is what it was at once; with a player it waits in black for the answer and then draws none of the art; and whenever the player refuses,
is silent or goes quiet the saver draws the art itself and asks the player to stop. Nothing runs while the saver is down.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from test_plugins_surfaces import AWAKE, SAVER, Surfaces, build, js, probe as _probe, probes, publish, BYNAME, BYPROP, READY
from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

SNAP = "var d = JSON.parse(s.snapshot()); return {up: d.up, timers: d.timers, video: d.video, current: d.art.current, shown: d.art.shown, loaded: d.art.loaded};"
# Home's own saver (Layers builds one) is the one under test: a second, built beside it, would answer the power state too and ask the player twice.
RES = "byProp(root, 'videoUsable')"


def probe(at, label, body):
    return _probe(at, label, body, find=RES)


def configure(at, cfg, mailbox=None):
    """JS: Home's own saver takes these settings (the timings a test shortens) before it comes up, and its player's mailbox is the test's own
    (/tmp/lumatv-vplane is the real player's on the set, and what every other run on this machine would share)."""
    pre = "s.videoPlayer.dirUrl = %s;" % json.dumps("file://" + mailbox + "/") if mailbox else ""
    return js(at, BYPROP + "var s = byProp(root, 'videoUsable'); " + pre + " var c = %s; for (var k in c) s[k] = c[k];" % json.dumps(cfg))


class FakePlayer:
    """The player's end of the mailbox, as far as the saver can tell: it reads `req.json` and writes `status.json`."""

    def __init__(self, mailbox, mode="play", answer_ms=120, ready=True):
        self.mode, self.answer_ms = mode, answer_ms
        self.dir = mailbox
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir)
        self.requests = []
        self.seen = None
        self.state, self.last_seq = "idle", 0
        self.running = True
        self.quiet = False
        if ready:
            self.status("idle")
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def status(self, state, reason=""):
        self.state = state
        doc = {"v": 1, "pid": 1, "ts": int(time.time() * 1000), "state": state, "reason": reason, "seq": self.last_seq, "clip": "great-wave"}
        tmp = os.path.join(self.dir, "status.json.tmp")
        with open(tmp, "w") as f:
            json.dump(doc, f)
        os.replace(tmp, os.path.join(self.dir, "status.json"))

    def loop(self):
        beat = time.monotonic()
        answer_at = None
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
                self.last_seq = req["seq"]
                if req["op"] == "start":
                    if self.mode == "play" or self.mode == "quiet":
                        answer_at = time.monotonic() + self.answer_ms / 1000
                    elif self.mode == "error":
                        self.status("error", "no pipeline")
                elif req["op"] == "stop":
                    answer_at = None
                    self.status("idle", "stopped")
                elif req["op"] == "renew" and self.state == "playing" and not self.quiet:
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


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class VideoSaver(Surfaces):
    MINE = Surfaces.MINE + ("SaverVideo.qml",)
    THEME = {"screensaver": {"style": "art", "video": True, "art": {"order": "sequence"}}}
    PROPS = {"videoStartMs": 1500, "videoRenewMs": 700, "videoStaleMs": 2500}

    def setUp(self):
        super().setUp()
        self.mailbox = tempfile.mkdtemp(prefix="vplane-saver-")
        self.addCleanup(shutil.rmtree, self.mailbox, True)

    def player(self, **kw):
        p = FakePlayer(self.mailbox, **kw)
        self.addCleanup(p.close)
        return p

    def run_saver(self, theme, player_events, props=None, wait=11000, awake_at=None):
        at = READY + 300
        args = [*publish(300, "screen", AWAKE), *configure(at - 200, dict(self.PROPS, **(props or {})), self.mailbox), *publish(at + 100, "screen", SAVER)]
        args += player_events(at)
        if awake_at:
            args += publish(at + awake_at, "screen", AWAKE)
        rc, out = self.run_host(theme, *args, "--wait", str(at + wait))
        self.assertEqual(self.own_warnings(out, allow=("no-such-print.jpg",)), [], out[-3000:])
        return probes(out), out

    # ---- the default is what it was

    def test_without_the_owners_switch_nothing_changes_and_nothing_is_asked(self):
        player = self.player()
        theme = {"screensaver": {"style": "art", "art": {"order": "sequence"}}}
        p, out = self.run_saver(theme, lambda at: [*probe(at + 1500, "up", SNAP)])
        self.assertEqual(p["up"]["video"]["usable"], False)
        self.assertEqual(p["up"]["video"]["state"], "off")
        self.assertEqual(p["up"]["shown"], "great-wave", "the GPU's art is drawn, as it was")
        self.assertEqual(player.requests, [], "the player is not asked")

    def test_a_preview_is_always_the_gpus(self):
        player = self.player()
        at = READY + 300
        # (the previewing surface and Home's own look at this test's player: /tmp/lumatv-vplane is the real one's, and what other runs share)
        mailbox = json.dumps("file://" + self.mailbox + "/")
        rc, out = self.run_host(self.THEME, *publish(300, "screen", AWAKE), *build(at, "Screensaver.qml", {"z": 95, "objectName": "under-test"}),
                                *configure(at - 200, {}, self.mailbox),
                                *js(at + 100, BYNAME + "byName(root, 'under-test').videoPlayer.dirUrl = %s;" % mailbox),
                                *js(at + 300, BYNAME + "byName(root, 'under-test').preview(0.2, 0.3, 'starry-night');"),
                                *_probe(at + 1500, "up", "var d = JSON.parse(s.snapshot()); return {up: d.up, forced: s.forced, usable: d.video.usable, state: d.video.state, spec: d.video.spec, shown: d.art.shown, style: s.style};"),
                                "--wait", str(at + 2500))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual((p["up"]["forced"], p["up"]["usable"]), (True, False), p)
        self.assertEqual(p["up"]["shown"], "starry-night")
        self.assertEqual((p["up"]["state"], p["up"]["spec"]), ("off", {}), "the preview asked for nothing")
        self.assertEqual(player.requests, [], "and the player was not asked by anything")

    # ---- no player

    def test_with_no_player_on_the_tv_the_gpu_draws_the_art_at_once(self):
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 700, "soon", SNAP)])
        v = p["soon"]["video"]
        self.assertEqual((v["usable"], v["state"], v["reason"]), (True, "off", "no player"))
        self.assertEqual(p["soon"]["shown"], "great-wave", "the art is up within a moment: no black wait for a player that is not there")
        self.assertFalse(os.path.exists(os.path.join(self.mailbox, "req.json")))

    def test_a_status_that_stopped_being_rewritten_is_no_player_either(self):
        player = self.player()
        with open(os.path.join(self.mailbox, "status.json")) as f:
            doc = json.load(f)
        doc["ts"] -= 10 * 60 * 1000
        with open(os.path.join(self.mailbox, "status.json"), "w") as f:
            json.dump(doc, f)
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 700, "soon", SNAP)])
        self.assertEqual((p["soon"]["video"]["state"], p["soon"]["shown"]), ("off", "great-wave"))
        self.assertEqual(player.requests, [])

    # ---- a player that plays

    def test_it_waits_in_black_for_the_player_and_then_the_gpu_draws_none_of_the_art(self):
        player = self.player()
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 150, "asked", SNAP), *probe(at + 1800, "playing", SNAP), *probe(at + 4500, "later", SNAP)],
                                awake_at=6000)
        self.assertIn(p["asked"]["video"]["state"], ("off", "starting"), "asking waits for the index of the prints")
        self.assertEqual(p["asked"]["shown"], "", "black while it waits: the GPU draws none of the art")
        self.assertEqual((p["playing"]["video"]["state"], p["playing"]["shown"]), ("playing", ""), "playing: none of the art on the GPU")
        self.assertFalse(p["playing"]["video"]["hole"], "the player's own window carries the dim, the caption and the clock")
        self.assertEqual(p["later"]["video"]["state"], "playing")
        ops = player.ops()
        self.assertEqual(ops[0], "start")
        self.assertGreaterEqual(ops.count("renew"), 3, "renewed while it is up: %s" % ops)
        self.assertEqual(ops[-1], "stop", "and stopped the moment the TV woke")
        self.assertEqual(ops.count("stop"), 1)

    def test_the_request_is_the_savers_own_numbers(self):
        player = self.player()
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 600, "up", SNAP)])
        req = player.requests[0]
        self.assertEqual(req["op"], "start")
        self.assertLess(abs(req["sent"] - time.time() * 1000), 15000)
        self.assertEqual(req["ttlMs"], 30000)
        want = {"style": "art", "everyMin": 15, "shuffle": False, "dim": True, "dimTo": 0.4, "dimOverMin": 20, "moveMs": 4000, "fadeMs": 700,
                "ampX": 30, "ampY": 20, "periodX": 540, "periodY": 380, "drift": True, "clock": True, "twelveHour": True, "caption": True,
                "overlay": True, "auto": False, "clip": "great-wave", "ink": "#f2eff6", "inkAlpha": 179, "ink2": "#f2eff6", "ink2Alpha": 118}
        for k, v in want.items():
            self.assertEqual(req[k], v, k)
        self.assertLess(abs(req["since"] - req["sent"]), 15000, "the dim counts from when the saver came up")
        # the spec the surface says it asked with is the one that was written
        self.assertEqual(p["up"]["video"]["spec"]["ink2Alpha"], 118)

    def test_in_the_players_own_window_the_surface_still_names_each_print(self):
        """The order is the surface's (its bag is kept across restarts); the player turns the page, through the dim it draws, when told."""
        player = self.player()
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 1800, "playing", SNAP), *probe(at + 4500, "later", SNAP)], props={"periodMs": 2500}, wait=6000)
        self.assertFalse(p["later"]["video"]["hole"])
        nexts = [r for r in player.requests if r["op"] == "next"]
        self.assertTrue(nexts, player.ops())
        self.assertEqual(nexts[0]["clip"], "starry-night")
        self.assertEqual(p["later"]["shown"], "", "the GPU holds no picture")

    def test_a_player_that_refuses_hands_the_art_back_to_the_gpu_and_is_told_to_stop(self):
        player = self.player(mode="error")
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 1500, "failed", SNAP)])
        v = p["failed"]["video"]
        self.assertEqual((v["state"], v["reason"]), ("failed", "no pipeline"))
        self.assertEqual(p["failed"]["shown"], "great-wave", "the GPU's art")
        self.assertEqual(player.ops()[:2], ["start", "stop"])

    def test_a_player_that_does_not_answer_is_given_the_time_and_then_the_gpu_has_it(self):
        player = self.player(mode="silent")
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 700, "waiting", SNAP), *probe(at + 2800, "failed", SNAP)])
        self.assertEqual((p["waiting"]["video"]["state"], p["waiting"]["shown"]), ("starting", ""))
        self.assertEqual((p["failed"]["video"]["state"], p["failed"]["video"]["reason"], p["failed"]["shown"]), ("failed", "the player did not answer", "great-wave"))
        self.assertEqual(player.ops()[-1], "stop")

    def test_a_player_that_goes_quiet_while_playing_hands_the_art_back(self):
        player = self.player(mode="quiet")
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 1300, "playing", SNAP), *probe(at + 6500, "quiet", SNAP)], wait=8000)
        self.assertEqual(p["playing"]["video"]["state"], "playing")
        v = p["quiet"]["video"]
        self.assertEqual((v["state"], v["reason"]), ("failed", "the player went quiet"))
        self.assertEqual(p["quiet"]["shown"], "great-wave")
        self.assertEqual(player.ops()[-1], "stop")

    def test_turning_the_switch_off_while_the_picture_is_up_gives_the_art_to_the_gpu(self):
        player = self.player()
        off = "root.applyTheme({screensaver: {style: 'art', video: false, art: {order: 'sequence'}}}, root.lastThemeText);"
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 1800, "playing", SNAP), *js(at + 2000, off), *probe(at + 2800, "after", SNAP)], wait=3600)
        self.assertEqual(p["playing"]["video"]["state"], "playing")
        a = p["after"]
        self.assertEqual((a["video"]["usable"], a["video"]["state"], a["shown"]), (False, "off", "great-wave"))
        self.assertEqual(player.ops()[-1], "stop")

    def test_nothing_runs_once_the_tv_is_awake(self):
        player = self.player()
        p, out = self.run_saver(self.THEME, lambda at: [*probe(at + 1500, "up", SNAP), *probe(at + 4200, "awake", SNAP)], awake_at=3000)
        self.assertTrue(p["up"]["timers"])
        a = p["awake"]
        self.assertEqual((a["up"], a["timers"], a["video"]["state"]), (False, False, "off"))
        n = len(player.requests)
        time.sleep(1.5)
        self.assertEqual(len(player.requests), n, "no request after the stop")

    # ---- the hole: this surface above the picture

    def test_in_shell_mode_the_surface_draws_over_the_picture_and_turns_the_page_itself(self):
        player = self.player()
        theme = {"screensaver": {"style": "art", "video": True, "videoOverlay": "shell", "art": {"order": "sequence"}}}
        p, out = self.run_saver(theme, lambda at: [*probe(at + 1800, "playing", SNAP), *probe(at + 5200, "turned", SNAP), *probe(at + 7200, "after", SNAP)],
                                props={"periodMs": 3000}, wait=9000)
        v = p["playing"]["video"]
        self.assertEqual((v["state"], v["hole"], v["caption"]), ("playing", True, "great-wave"), "the hole is open and the caption is this surface's, for the first print")
        req = player.requests[0]
        self.assertEqual((req["overlay"], req["auto"], req["clip"]), (False, False, "great-wave"), "the player draws nothing and turns no page: the surface names the print")
        nexts = [r for r in player.requests if r["op"] == "next"]
        self.assertTrue(nexts, player.ops())
        self.assertEqual(nexts[0]["clip"], "starry-night", "the next print, in the owner's order")
        self.assertEqual(p["after"]["video"]["caption"], "starry-night")
        self.assertAlmostEqual(p["after"]["video"]["vfade"], 0, places=2, msg="and the page is clear again")
        self.assertEqual(p["playing"]["current"], 0)

    def test_in_shell_mode_the_art_pieces_are_never_loaded(self):
        player = self.player()
        theme = {"screensaver": {"style": "art", "video": True, "videoOverlay": "shell", "art": {"order": "sequence"}}}
        p, out = self.run_saver(theme, lambda at: [*probe(at + 1800, "playing", SNAP)])
        self.assertEqual(p["playing"]["shown"], "", "the GPU holds no picture (the caption's words come from the index alone)")

    def test_in_shell_mode_a_failure_starts_the_gpus_art_from_the_first_print(self):
        player = self.player(mode="error")
        theme = {"screensaver": {"style": "art", "video": True, "videoOverlay": "shell", "art": {"order": "sequence"}}}
        p, out = self.run_saver(theme, lambda at: [*probe(at + 1800, "failed", SNAP)])
        self.assertEqual((p["failed"]["video"]["state"], p["failed"]["video"]["hole"], p["failed"]["shown"]), ("failed", False, "great-wave"))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class WithTheRealPlayer(Surfaces):
    """The request the QML writes, read by the C program."""

    MINE = Surfaces.MINE + ("SaverVideo.qml",)

    def test_the_savers_request_makes_the_player_open_its_window_dim_and_drift(self):
        from test_vplane_player import BIN, HAVE, NDL, Rig, argb
        if not HAVE:
            self.skipTest("needs zig to build vplane/")
        rig = Rig(clip_keys=(3000, 4000), units=20, captions=True)
        self.addCleanup(rig.close)
        at = READY + 300
        theme = {"screensaver": {"style": "art", "video": True, "dimOverMin": 1, "dimTo": 0.5, "shiftEverySec": 1, "art": {"order": "sequence"}}}
        rc, out = self.run_host(theme, *publish(300, "screen", AWAKE), *configure(at - 200, {"videoStartMs": 4000, "videoRenewMs": 1500}, rig.mb),
                                *publish(at + 100, "screen", SAVER), *probe(at + 3500, "up", SNAP), *publish(at + 5500, "screen", AWAKE), "--wait", str(at + 7500))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["up"]["video"]["state"], "playing", rig.log())
        self.assertEqual(p["up"]["shown"], "", "none of the art on the GPU")
        calls = rig.calls()
        self.assertEqual(calls[:6], ["DL_Initialize", "SetWindowId", "Init", "Load", "SetAppState", "Play"])
        self.assertEqual(rig.fc.props["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_SCREENSAVER")
        # `since`, `dimTo` and `dimOverMin` arrived: a few seconds into a one minute dim to 0.5 the veil has begun, and it is a veil, not black
        veils = [argb(g[2], g[0], 200, 200)[0] for t, s, g in rig.fc.commits if g]
        self.assertTrue(veils and 0 < max(veils) < 60, "the dim runs: %s" % veils[-5:])
        # the shift ran every second inside its room: the exported rectangle moved and stayed in 32 +- 30 by 28 +- 20
        dsts = {h["dst"][:2] for h in rig.fc.video_history}
        self.assertGreater(len(dsts), 2, dsts)
        for x, y in dsts:
            self.assertTrue(2 <= x <= 62 and 8 <= y <= 48, (x, y))
        # the corner clock is in the ink the surface asked for: ink2 at 0.46 of full (118 of 255)
        w, h, data = rig.fc.last_commit()[2]
        lit = [argb(data, w, x, y) for x in range(1700, 1850) for y in range(985, 1025)]
        ink = [c for c in lit if c[0] >= 100 and c[1] > 0]
        self.assertTrue(ink, "the clock's strokes")
        self.assertTrue(all(max(c[1:]) - min(c[1:]) <= 4 for c in ink), "ink2's neutral off-white, not a stray colour")
        # the TV woke: a stop, and the window went
        self.assertTrue(rig.wait(lambda: rig.status()["state"] == "idle", 4), rig.status())
        self.assertEqual(rig.status()["reason"], "stopped")
        self.assertTrue(rig.wait(lambda: not rig.fc.alive(), 3))


if __name__ == "__main__":
    unittest.main()
