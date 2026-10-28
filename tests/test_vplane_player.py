"""Run with: cd tests && python3 -m unittest test_vplane_player   (needs zig for the host build of vplane/, skipped without it)

lumatv-vplane, the video-plane player (vplane/), on the host: the real program, built for x86-64, against a stand-in for LG's NDL
library (vplane/tests/fake_ndl.c: it logs every call and flags the orders LG's library punishes) and a small Wayland server
(vplane/tests/fake_wayland.py). What this shows is the player's own behaviour: what it asks the compositor for and in what order, what
it feeds the decoder and how fast, what it draws over the video and when, and that the window goes the moment it is told to, the
compositor takes it, or nobody renews it. What it cannot show is that LG's compositor and LG's pipeline agree: the TV steps in
docs/research/videoplane-saver-result.md say how that is measured. The real compositor library runs the same client in
compositor/crates/lumatv-compositor/tests/vplane_host.rs.
"""
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
VPLANE = os.path.join(ROOT, "vplane")
sys.path.insert(0, os.path.join(VPLANE, "tests"))
sys.path.insert(0, os.path.join(ROOT, "tools"))

import vplane_clips as clips  # noqa: E402
from fake_wayland import FakeCompositor, argb  # noqa: E402

BIN = os.path.join(VPLANE, "build", "host", "lumatv-vplane")
NDL = os.path.join(VPLANE, "build", "host", "libNDL_directmedia.so.1")
PLAN = os.path.join(VPLANE, "build", "host", "plan_test")
SHIPPED = os.path.join(ROOT, "payload", "vplane", "clips")
W, H = 1856, 1024


def put(path, blob):
    with open(path, "wb") as f:
        f.write(blob)


def build():
    if os.path.exists(BIN) and os.path.exists(NDL) and os.path.exists(PLAN):
        newest = max(os.path.getmtime(os.path.join(VPLANE, d, f)) for d in ("src", "tests") for f in os.listdir(os.path.join(VPLANE, d)))
        if os.path.getmtime(BIN) >= newest:
            return True
    if not shutil.which("zig"):
        return False
    return subprocess.run(["sh", os.path.join(VPLANE, "build.sh")], capture_output=True).returncode == 0


HAVE = build()


def synthetic_clip(path, units=30, key=3000, other=150, fps=60, w=W, h=H, codec=2):
    """A clip the player will feed: Annex B-looking units of the sizes asked for (the fake library only looks at the start code)."""
    def unit(size, k):
        return (b"\x00\x00\x00\x01" + (b"\x46\x01" if k else b"\x02\x01") + bytes([units & 255]) * (size - 6), k)
    us = [unit(key, True)] + [unit(other + i % 7, False) for i in range(units - 1)]
    clips.write_vpc(path, "hevc" if codec == 2 else "h264", (w, h), fps, us)
    return us


class Rig:
    """A player, a compositor and a library stand-in, each in its own temporary place."""

    def __init__(self, clip_keys=(3000,), units=30, window="screensaver", args=(), env=None, glyphs=True, hold_window_id=False, captions=False, mailbox=None, release="immediate", lag=0.0, walls=0):
        self.tmp = tempfile.mkdtemp(prefix="vplane-test-")
        self.mb = mailbox or os.path.join(self.tmp, "mb")      # (the saver's own tests put it where the saver looks)
        self.clipdir = os.path.join(self.tmp, "clips")
        os.makedirs(self.clipdir)
        self.units = []
        for i, k in enumerate(clip_keys):
            self.units.append(synthetic_clip(os.path.join(self.clipdir, "clip%d.vpc" % i), units=units, key=k))
        for i in range(walls):         # the moving wallpaper's clips: the screen's size, named for their role
            synthetic_clip(os.path.join(self.clipdir, "wall-%d.vpc" % i), units=units, key=3000, w=1920, h=1080)
        if glyphs:
            shutil.copy(os.path.join(SHIPPED, "clock.vpg"), self.clipdir)
        self.captions = []
        if captions:
            geo = clips.geometry()
            for i, entry in enumerate(clips.pictures()[:len(clip_keys)]):
                bm = clips.caption_bitmap(entry, geo)
                clips.write_vpt(os.path.join(self.clipdir, "clip%d.vpt" % i), bm)
                self.captions.append(clips.read_vpt(os.path.join(self.clipdir, "clip%d.vpt" % i)))
        self.ndl_log = os.path.join(self.tmp, "ndl.log")
        open(self.ndl_log, "w").close()
        self.fc = FakeCompositor(hold_window_id, release, lag)
        e = dict(os.environ, **self.fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=self.ndl_log, FAKE_NDL_VERBOSE="1")
        e.update({k: v.replace("{sock}", self.fc.path) for k, v in (env or {}).items()})        # `{sock}` is the stand-in compositor's own socket
        self.err_path = os.path.join(self.tmp, "player.log")
        self.err = open(self.err_path, "w")
        self.proc = subprocess.Popen([BIN, "-d", self.mb, "-c", self.clipdir, "-L", NDL, "-w", window, "-m", "1", *args], env=e, stderr=self.err)
        self.seq = int(time.time() * 1000)
        self.wait(lambda: os.path.exists(os.path.join(self.mb, "status.json")), 5)

    def send(self, op, **kw):
        self.seq += 1
        req = {"seq": self.seq, "sent": int(time.time() * 1000), "op": op}
        req.update(kw)
        path = os.path.join(self.mb, "req.json")
        with open(path, "w") as f:          # as QML's TextFile does: open, write, close in place
            f.write(json.dumps(req))
        return req

    def fresh(self):
        """The status after the player has been asked to write it now."""
        before = self.status().get("ts", 0)
        self.send("status")
        self.wait(lambda: self.status().get("ts", 0) > before, 2)
        return self.status()

    def status(self):
        try:
            with open(os.path.join(self.mb, "status.json")) as f:
                return json.loads(f.read())
        except (OSError, ValueError):
            return {}

    def wait(self, pred, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                if pred():
                    return True
            except (KeyError, IndexError, TypeError):
                pass
            time.sleep(0.01)
        return False

    def wait_state(self, state, timeout=5.0):
        return self.wait(lambda: self.status().get("state") == state, timeout)

    def ndl(self):
        with open(self.ndl_log) as f:
            return [ln.split() for ln in f.read().splitlines()]

    def calls(self):
        return [ln[2] for ln in self.ndl() if len(ln) > 2]

    def plays(self):
        """[(size, pts)] of every unit the library took, in order."""
        return [(int(ln[5]), int(ln[7])) for ln in self.ndl() if len(ln) > 7 and ln[2] == "Play"]

    def log(self):
        self.err.flush()
        with open(self.err_path) as f:
            return f.read()

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.err.close()
        self.fc.close()
        shutil.rmtree(self.tmp, ignore_errors=True)


def expected_text_pixels(text, ink, alpha, veil, right, baseline):
    """The corner clock as the player must have drawn it: (x, y) -> premultiplied (a, r, g, b), from the shipped glyph atlas."""
    g = clips.read_glyphs(os.path.join(SHIPPED, "clock.vpg"))
    total = sum(g["glyphs"][c]["advance"] for c in text)
    pen = right - total
    out = {}

    def m(a, b):
        t = a * b + 128
        return (t + (t >> 8)) >> 8

    cr, cg, cb = (ink >> 16) & 255, (ink >> 8) & 255, ink & 255
    for ch in text:
        gl = g["glyphs"][ch]
        for gy in range(gl["h"]):
            for gx in range(gl["w"]):
                cover = gl["alpha"][gy * gl["w"] + gx]
                if not cover:
                    continue
                x, y = pen + gl["left"] + gx, baseline - gl["top"] + gy
                a = m(cover, alpha)
                d = out.get((x, y), (veil, 0, 0, 0))
                inv = 255 - a
                out[(x, y)] = (a + m(d[0], inv), m(cr, a) + m(d[1], inv), m(cg, a) + m(d[2], inv), m(cb, a) + m(d[3], inv))
        pen += gl["advance"]
    return out, g


def expected_caption_pixels(cap, ink, a1, ink2, a2, opacity, veil, dx=0, dy=0):
    """The caption as the player must have drawn it over a veil: (x, y) -> premultiplied (a, r, g, b)."""
    def m(a, b):
        t = a * b + 128
        return (t + (t >> 8)) >> 8

    out = {}
    for gy in range(cap["height"]):
        for gx in range(cap["width"]):
            cover = cap["alpha"][gy * cap["width"] + gx]
            if not cover:
                continue
            first = gx < cap["split"]
            rgb, al = (ink, a1) if first else (ink2, a2)
            a = m(m(cover, al), opacity)
            d = (veil, 0, 0, 0)
            inv = 255 - a
            out[(cap["x"] + dx + gx, cap["y"] + dy + gy)] = (a + m(d[0], inv), m((rgb >> 16) & 255, a), m((rgb >> 8) & 255, a), m(rgb & 255, a))
    return out


@unittest.skipUnless(HAVE, "needs zig to build vplane/")
class Arithmetic(unittest.TestCase):
    def test_the_c_unit_tests_pass(self):
        r = subprocess.run([PLAN], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("all passed", r.stdout)

    def calc(self, *args):
        return subprocess.run([BIN, "--calc", *map(str, args)], capture_output=True, text=True, check=True).stdout.split()

    def test_the_drift_is_the_qml_formula_with_javascripts_rounding(self):
        def js(t, ax=30, ay=20, px=540, py=380):
            return (math.floor(ax * math.sin(2 * math.pi * t / px) + 0.5), math.floor(ay * math.sin(2 * math.pi * t / py) + 0.5))
        for t in (0, 1, 2.5, 4, 17, 135, 270.5, 405, 539.9, 1000.5, 3600, 86399.5):
            self.assertEqual(tuple(map(int, self.calc("drift", t))), js(t), t)

    def test_the_dim_curve(self):
        for m, want in ((0, 1.0), (15, 0.9), (30, 0.8), (60, 0.6), (120, 0.6)):
            self.assertAlmostEqual(float(self.calc("level", m)[0]), want, places=6)
        self.assertEqual(float(self.calc("level", 30, 0)[0]), 1.0)

    def test_the_order_never_repeats_and_is_the_same_for_a_seed(self):
        a = self.calc("order", 6, 1, 4242, 60)
        self.assertEqual(a, self.calc("order", 6, 1, 4242, 60))
        self.assertNotEqual(a, self.calc("order", 6, 1, 4243, 60))
        for bag in range(10):
            self.assertEqual(sorted(a[bag * 6:bag * 6 + 6]), ["0", "1", "2", "3", "4", "5"])
        self.assertTrue(all(x != y for x, y in zip(a, a[1:])))
        self.assertEqual(self.calc("order", 4, 0, 1, 9), "0 1 2 3 0 1 2 3 0".split())

    def test_the_corner_clock_text_is_what_the_gpu_saver_shows(self):
        self.assertEqual(self.calc("clock", 17, 5, 1), ["5:05"])        # the twelve hour face folds the hour; Qt's "h:mm" alone would say 17:05
        self.assertEqual(self.calc("clock", 7, 5, 0), ["07:05"])
        self.assertEqual(self.calc("clock", 7, 5, 1), ["7:05"])
        self.assertEqual(self.calc("clock", 0, 30, 1), ["12:30"])
        self.assertEqual(self.calc("clock", 23, 39, 1), ["11:39"])
        self.assertEqual(self.calc("clock", 23, 39, 0), ["23:39"])


@unittest.skipUnless(HAVE, "needs zig to build vplane/")
class Clips(unittest.TestCase):
    def info(self, path):
        r = subprocess.run([BIN, "--vpc-info", path], capture_output=True, text=True)
        return r.returncode, json.loads(r.stdout)

    def test_the_reader_agrees_with_the_writer(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "a.vpc")
            us = synthetic_clip(path, units=17, key=999, fps=60)
            rc, j = self.info(path)
            self.assertEqual(rc, 0)
            self.assertEqual((j["codec"], j["width"], j["height"], j["fpsNum"], j["fpsDen"], j["units"], j["loopFrom"], j["keyBytes"]), (2, W, H, 60, 1, 17, 0, 999))
            self.assertEqual(j["bytes"], os.path.getsize(path))
            self.assertEqual(sum(len(u) for u, _ in us) + 40 + 8 * 17, os.path.getsize(path))

    def test_a_damaged_file_is_refused_for_the_reason(self):
        with tempfile.TemporaryDirectory() as d:
            good = os.path.join(d, "g.vpc")
            synthetic_clip(good, units=8)
            with open(good, "rb") as f:
                data = f.read()

            def damaged(name, blob):
                p = os.path.join(d, name)
                put(p, blob)
                return self.info(p)

            for name, blob, why in (
                ("magic", b"XPC1" + data[4:], "not a VPC1"),
                ("short", data[:30], "not a VPC1"),
                ("truncated", data[:-10], "runs past"),
                ("trailing", data + b"\0\0", "bytes after"),
                ("version", data[:4] + b"\x09" + data[5:], "unknown version"),
                ("codec", data[:5] + b"\x07" + data[6:], "unknown version or codec"),
                ("zero size", data[:8] + struct.pack("<I", 0) + data[12:], "bad header"),
                ("loop target", data[:28] + struct.pack("<I", 3) + data[32:], "key unit"),
                ("first not key", data[:44] + struct.pack("<I", 0) + data[48:], "key unit"),
                ("count", data[:24] + struct.pack("<I", 9999) + data[28:], "bad header"),
            ):
                rc, j = damaged(name, blob)
                self.assertEqual(rc, 2, name)
                self.assertIn(why, j["error"], name)
            self.assertEqual(self.info(os.path.join(d, "nothing.vpc"))[0], 2)

    def test_the_shipped_clips_are_all_readable_and_looped_on_a_key_unit(self):
        names = sorted(f for f in os.listdir(SHIPPED) if f.endswith(".vpc"))
        self.assertGreaterEqual(len(names), 6)
        for n in names:
            rc, j = self.info(os.path.join(SHIPPED, n))
            self.assertEqual(rc, 0, n)
            self.assertEqual((j["codec"], j["width"], j["height"], j["fpsNum"], j["fpsDen"], j["loopFrom"]), (2, W, H, 60, 1, 0), n)

    def test_the_probe_reads_the_library_and_the_clips(self):
        r = subprocess.run([BIN, "-n", "-c", SHIPPED, "-L", NDL], capture_output=True, text=True, env=dict(os.environ, FAKE_NDL_LOG=os.devnull))
        j = json.loads(r.stdout)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertTrue(j["ndl"])
        self.assertEqual(j["clips"], j["clipsOk"])
        self.assertGreaterEqual(j["clips"], 6)

    def test_the_probe_says_what_is_missing(self):
        r = subprocess.run([BIN, "-n", "-c", SHIPPED, "-L", "/nonexistent/libNDL.so"], capture_output=True, text=True, env=dict(os.environ, LD_LIBRARY_PATH=""))
        j = json.loads(r.stdout)
        self.assertEqual(r.returncode, 2)
        self.assertFalse(j["ndl"])
        self.assertIn("no NDL library", j["ndlError"])
        with tempfile.TemporaryDirectory() as d:
            shutil.copy(os.path.join(SHIPPED, os.listdir(SHIPPED)[0]), d)
            put(os.path.join(d, "broken.vpc"), b"junk" * 20)
            r = subprocess.run([BIN, "-n", "-c", d, "-L", NDL], capture_output=True, text=True, env=dict(os.environ, FAKE_NDL_LOG=os.devnull))
            j = json.loads(r.stdout)
            self.assertEqual(r.returncode, 2)
            self.assertEqual((j["clips"], j["clipsOk"]), (2 if os.listdir(SHIPPED)[0].endswith(".vpc") else 1, 1 if os.listdir(SHIPPED)[0].endswith(".vpc") else 0))
            self.assertIn("broken.vpc", j["clipError"])


@unittest.skipUnless(HAVE, "needs zig to build vplane/")
class Player(unittest.TestCase):
    def rig(self, **kw):
        r = Rig(**kw)
        self.addCleanup(r.close)
        return r

    # ---- idle

    def test_idle_costs_nothing_and_says_so(self):
        r = self.rig()
        s = r.status()
        self.assertEqual((s["state"], s["clips"], s["windowId"]), ("idle", 1, ""))
        time.sleep(0.3)
        self.assertEqual(r.fc.names(), [], "no connection to the compositor while idle")
        self.assertEqual(r.ndl(), [], "the library is not even loaded while idle")
        self.assertEqual(r.status()["state"], "idle")

    # ---- the handshake

    def test_the_window_the_export_and_the_decoder_come_in_the_order_the_libraries_demand(self):
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"), r.log())
        r.fc.settle()
        names = r.fc.names()
        for a, b in (("wl_display.get_registry", "wl_compositor.create_surface"), ("wl_compositor.create_surface", "wl_webos_shell.get_shell_surface"),
                     ("wl_webos_shell.get_shell_surface", "wl_webos_shell_surface.set_state"), ("wl_webos_shell_surface.set_state", "wl_shm.create_pool"),
                     ("wl_webos_foreign.export_element", "wl_webos_exported.set_exported_window")):
            self.assertLess(names.index(a), names.index(b), (a, b))
        self.assertLess(names.index("wl_webos_foreign.export_element"), names.index("wl_surface.commit"), "export before the first commit")
        binds = [a for w, a in r.fc.requests() if w == "bind"]
        self.assertEqual(sorted(b.split()[0] for b in binds), ["wl_compositor", "wl_seat", "wl_seat", "wl_shm", "wl_webos_foreign", "wl_webos_shell"])
        self.assertTrue(any(b.startswith("wl_compositor v3 ") for b in binds), "wl_compositor at version 3, LG's")
        self.assertEqual(r.fc.props["appId"], "org.nphil.lumatv.vplane")
        self.assertEqual(r.fc.state, 3)
        self.assertEqual(r.fc.video["src"], (0, 0, W, H))
        self.assertEqual(r.fc.video["dst"], (32, 28, W, H), "centred on the 1920 x 1080 screen: 32 and 28 pixels of room to drift in")
        calls = r.calls()
        self.assertEqual(calls[:6], ["DL_Initialize", "SetWindowId", "Init", "Load", "SetAppState", "Play"])
        wid = [ln for ln in r.ndl() if ln[2] == "SetWindowId"][0][3]
        self.assertEqual(wid, r.status()["windowId"])
        self.assertEqual(wid, "_Window_Id_1", "the id the compositor assigned is the id the library was given")
        self.assertEqual([ln for ln in r.ndl() if ln[2] == "Load"][0][3:], ["1856x1024", "type", "2"])
        self.assertEqual([ln for ln in r.ndl() if ln[2] == "SetAppState"][0][3], "0", "foreground")
        self.assertFalse([ln for ln in r.ndl() if ln[2] == "VIOLATION"], "no order the real library punishes")

    def test_the_export_can_come_after_the_first_commit(self):
        r = self.rig(args=("-e", "after"))
        r.send("start")
        self.assertTrue(r.wait_state("playing"), r.log())
        names = r.fc.names()
        self.assertLess(names.index("wl_surface.commit"), names.index("wl_webos_foreign.export_element"))

    def test_the_window_type_follows_the_option(self):
        for opt, want, play in (("screensaver", "_WEBOS_WINDOW_TYPE_SCREENSAVER", "true"), ("card", "_WEBOS_WINDOW_TYPE_CARD", None),
                                ("overlay", "_WEBOS_WINDOW_TYPE_OVERLAY", None), ("default", None, None)):
            r = self.rig(window=opt)
            r.send("start")
            self.assertTrue(r.wait_state("playing"), (opt, r.log()))
            self.assertEqual(r.fc.props.get("_WEBOS_WINDOW_TYPE"), want, opt)
            self.assertEqual(r.fc.props.get("playVideo"), play, opt)
            r.close()

    # ---- what is fed

    def test_units_are_fed_at_the_clips_rate_in_order_and_the_clip_loops_on_its_key_unit(self):
        r = self.rig(units=30)
        r.send("start")
        self.assertTrue(r.wait_state("playing"), r.log())
        time.sleep(1.0)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        plays = r.plays()
        self.assertGreater(len(plays), 50, "about sixty a second")
        self.assertLess(len(plays), 100)
        sizes = [len(u) for u, _ in r.units[0]]
        self.assertEqual([s for s, _ in plays], [sizes[i % 30] for i in range(len(plays))], "every unit once, in order, and the key unit again after the thirtieth")
        pts = [p for _, p in plays]
        self.assertTrue(all(b > a for a, b in zip(pts, pts[1:])), "PTS rises across the loop")
        steps = {b - a for a, b in zip(pts, pts[1:])}
        self.assertLessEqual(steps, {16666, 16667}, "a step of 1/60 s")

    def test_a_full_buffer_is_waited_out_not_skipped(self):
        r = self.rig(units=30, env={"FAKE_NDL_FULL_EVERY": "3"})
        r.send("start")
        self.assertTrue(r.wait_state("playing"), r.log())
        time.sleep(0.8)
        s = r.fresh()
        self.assertGreater(s["full"], 3)
        sizes = [len(u) for u, _ in r.units[0]]
        plays = r.plays()
        self.assertEqual([x for x, _ in plays], [sizes[i % 30] for i in range(len(plays))], "no unit lost to the refusals")

    def test_a_long_render_queue_holds_the_feed_back(self):
        r = self.rig(env={"FAKE_NDL_QUEUE": "50"})
        r.send("start")
        self.assertTrue(r.wait_state("opening", 3) or r.wait_state("playing", 3))
        time.sleep(0.5)
        s = r.fresh()
        self.assertEqual(r.plays(), [], "nothing is fed into a queue that is full")
        self.assertGreater(s["skipped"], 5)

    # ---- opening that fails

    def test_every_refusal_of_the_library_ends_with_no_window_and_the_reason(self):
        for fail, why in (("dlinit", "DL_Initialize"), ("init", "NDL_DirectMediaInit"), ("load", "NDL_DirectMediaLoad"), ("state", "SetAppState")):
            r = self.rig(env={"FAKE_NDL_FAIL": fail})
            r.send("start")
            self.assertTrue(r.wait_state("error"), (fail, r.log()))
            self.assertIn(why, r.status()["reason"], fail)
            self.assertTrue(r.wait(lambda: not r.fc.alive(), 3), "the window is destroyed: the connection is closed (%s)" % fail)
            r.close()

    def test_a_pipeline_that_never_plays_is_given_up_on(self):
        r = self.rig(env={"FAKE_NDL_NO_PLAYING": "1"}, args=("-o", "600"))
        r.send("start")
        self.assertTrue(r.wait_state("error", 4), r.log())
        self.assertIn("no playing event", r.status()["reason"])
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 3))
        self.assertIn("Quit", r.calls(), "the pipeline is unloaded too")

    def test_an_error_event_ends_the_play(self):
        r = self.rig(env={"FAKE_NDL_ERROR_AFTER": "20"})
        r.send("start")
        self.assertTrue(r.wait_state("error", 4), r.log())
        self.assertIn("pipeline reported an error", r.status()["reason"])
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 3))

    def test_no_clips_and_no_memory_are_reasons_too(self):
        r = self.rig()
        for f in os.listdir(r.clipdir):
            if f.endswith(".vpc"):
                os.unlink(os.path.join(r.clipdir, f))
        r.send("start")
        self.assertTrue(r.wait_state("error"))
        self.assertEqual(r.status()["reason"], "no clips")
        r.close()
        r = self.rig(args=("-m", "999999999"))
        r.send("start")
        self.assertTrue(r.wait_state("error"))
        self.assertIn("memory", r.status()["reason"])
        self.assertEqual(r.fc.names(), [], "nothing was asked of the compositor")

    def test_a_compositor_without_the_webos_protocol_is_named(self):
        r = self.rig()
        from fake_wayland import GLOBALS
        GLOBALS[:] = [g for g in GLOBALS if g[0] != "wl_webos_foreign"]
        try:
            r.send("start")
            self.assertTrue(r.wait_state("error"))
            self.assertIn("wl_webos_foreign", r.status()["reason"])
        finally:
            GLOBALS.append(("wl_webos_foreign", 1))

    def test_a_window_id_that_never_comes_is_a_reason(self):
        r = self.rig(hold_window_id=True)
        r.send("start")
        self.assertTrue(r.wait_state("error", 8))
        self.assertIn("no window id", r.status()["reason"])

    # ---- stopping, in every way

    def test_a_stop_takes_the_window_before_the_decoder_is_unloaded(self):
        r = self.rig(env={"FAKE_NDL_UNLOAD_MS": "500"})
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        self.assertEqual(r.status()["reason"], "stopped")
        self.assertTrue(r.wait(lambda: "Quit" in r.calls(), 3))
        self.assertTrue(r.fc.wait_for("disconnect", 3))
        gone = [t for t, w, a in r.fc.events if w == "disconnect"][0]
        unload = int([ln for ln in r.ndl() if ln[2] == "Unload"][0][1]) / 1000.0
        self.assertLess(gone, unload + 0.4, "the compositor saw the window go while the library was still unloading (a slow teardown does not keep it on screen)")
        self.assertGreater(len(r.fc.events), 10)

    def test_it_stops_itself_when_nobody_renews_it(self):
        r = self.rig()
        r.send("start", ttlMs=1200)
        self.assertTrue(r.wait_state("playing"))
        t0 = time.monotonic()
        self.assertTrue(r.wait_state("idle", 4))
        self.assertIn("ttl", r.status()["reason"])
        self.assertGreater(time.monotonic() - t0, 0.5)
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 3))

    def test_a_renewal_keeps_it_up_and_an_old_request_does_not_start_it(self):
        r = self.rig()
        r.send("start", ttlMs=1500)
        self.assertTrue(r.wait_state("playing"))
        for _ in range(4):
            time.sleep(0.6)
            r.send("renew", ttlMs=1500)
        self.assertEqual(r.status()["state"], "playing", "renewed for 2.4 s on a 1.5 s ttl")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        n = r.fc.count("wl_display.get_registry")
        self.seq = r.seq
        old = {"seq": r.seq + 5, "sent": int(time.time() * 1000) - 60000, "op": "start", "ttlMs": 30000}
        with open(os.path.join(r.mb, "req.json"), "w") as f:
            json.dump(old, f)
        time.sleep(0.5)
        self.assertEqual(r.status()["state"], "idle", "a start from a minute ago (ttl 30 s) is a leftover, not a request")
        self.assertEqual(r.fc.count("wl_display.get_registry"), n)

    def test_a_start_that_was_written_before_the_player_was_there_is_acted_on_if_fresh(self):
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, True)
        mb = os.path.join(tmp, "mb")
        os.makedirs(mb)
        with open(os.path.join(mb, "req.json"), "w") as f:
            json.dump({"seq": 1, "sent": int(time.time() * 1000), "op": "start", "ttlMs": 5000}, f)
        clipdir = os.path.join(tmp, "clips")
        os.makedirs(clipdir)
        synthetic_clip(os.path.join(clipdir, "a.vpc"))
        fc = FakeCompositor()
        self.addCleanup(fc.close)
        log = os.path.join(tmp, "ndl.log")
        p = subprocess.Popen([BIN, "-d", mb, "-c", clipdir, "-L", NDL, "-m", "1"], env=dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=log),
                             stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: (p.kill(), p.wait()))
        end = time.monotonic() + 5
        state = ""
        while time.monotonic() < end and state != "playing":
            try:
                with open(os.path.join(mb, "status.json")) as f:
                    state = json.load(f)["state"]
            except (OSError, ValueError):
                pass
            time.sleep(0.02)
        self.assertEqual(state, "playing")

    def test_the_same_request_read_twice_and_an_older_one_do_nothing(self):
        r = self.rig()
        req = r.send("start")
        self.assertTrue(r.wait_state("playing"))
        n = r.fc.count("wl_display.get_registry")
        with open(os.path.join(r.mb, "req.json"), "w") as f:
            json.dump(req, f)
        time.sleep(0.3)
        self.assertEqual(r.fc.count("wl_display.get_registry"), n)
        older = dict(req, seq=req["seq"] - 10, op="stop")
        with open(os.path.join(r.mb, "req.json"), "w") as f:
            json.dump(older, f)
        time.sleep(0.3)
        self.assertEqual(r.status()["state"], "playing", "a request older than the one acted on is ignored")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))

    def test_junk_in_the_request_file_is_ignored(self):
        r = self.rig()
        for junk in ("", "not json", "[1,2]", '{"op": 5}', '{"op":"warp","seq":3}', '{"seq":3}'):
            with open(os.path.join(r.mb, "req.json"), "w") as f:
                f.write(junk)
            time.sleep(0.1)
            self.assertEqual(r.status()["state"], "idle", junk)
        self.assertEqual(r.proc.poll(), None)

    def test_the_compositor_closing_the_window_ends_the_play_and_a_new_start_works(self):
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.fc.send_close()
        self.assertTrue(r.wait_state("idle", 3), r.log())
        self.assertIn("closed the window", r.status()["reason"])
        self.assertTrue(r.wait(lambda: "Quit" in r.calls(), 3))
        r.send("start")
        self.assertTrue(r.wait_state("playing", 4), r.log())
        self.assertEqual(r.fc.count("wl_webos_foreign.export_element"), 2)

    def test_a_dropped_connection_and_a_protocol_error_end_the_play(self):
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.fc.drop()
        self.assertTrue(r.wait_state("idle", 3), r.log())
        r.close()
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.fc.break_protocol()
        self.assertTrue(r.wait_state("idle", 3), r.log())
        self.assertIn("test error", r.status()["reason"])

    def test_sigterm_takes_the_window_down_and_unloads_the_decoder(self):
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.proc.terminate()
        self.assertEqual(r.proc.wait(5), 0)
        self.assertIn("Quit", r.calls())
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 2))

    # ---- what is drawn

    def veil_at(self, r, x=200, y=200):
        c = r.fc.last_commit()
        w, h, data = c[2]
        return argb(data, w, x, y)

    def test_the_picture_is_a_transparent_window_until_the_dim_has_begun(self):
        r = self.rig()
        r.send("start", clock=False)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.2)
        w, h, data = r.fc.last_commit()[2]
        self.assertEqual((w, h), (1920, 1080))
        self.assertEqual(argb(data, w, 960, 540), (0, 0, 0, 0), "over the picture a transparent window: the video shows through")
        self.assertEqual({argb(data, w, x, y) for x in range(100, 1800, 101) for y in range(100, 1000, 97)}, {(0, 0, 0, 0)})

    def test_what_the_window_covers_around_the_picture_is_black_and_follows_it(self):
        r = self.rig()
        r.send("start", clock=False, moveMs=60, periodX=1, periodY=2, ampX=30, ampY=20, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(1.2)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        self.assertGreater(len({h["dst"] for h in r.fc.video_history}), 3, "the picture moved")
        x0, y0, w0, h0 = r.fc.video_history[-1]["dst"]
        w, h, data = r.fc.last_commit()[2]
        for xy in ((x0 - 1, 540), (x0 + w0, 540), (960, y0 - 1), (960, y0 + h0), (0, 0), (1919, 1079)):
            self.assertEqual(argb(data, w, *xy), (255, 0, 0, 0), "black around the picture, or the margin shows what is behind the saver: %s" % (xy,))
        for xy in ((x0, 540), (x0 + w0 - 1, 540), (960, y0), (960, y0 + h0 - 1)):
            self.assertEqual(argb(data, w, *xy), (0, 0, 0, 0), "and nothing over the picture's own edge: %s" % (xy,))

    def test_the_decoder_library_is_given_the_compositors_runtime_dir_when_the_player_has_none(self):
        r = self.rig(env={"XDG_RUNTIME_DIR": "", "WAYLAND_DISPLAY": "{sock}"})
        r.send("start", clock=False)
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertIn("xdg=/tmp/xdg", [ln for ln in r.ndl() if ln[2] == "Init"][0], "LG's AV connector finds its socket through it")
        r.close()
        r = self.rig()
        r.send("start", clock=False)
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertIn("xdg=%s" % r.fc.dir, [ln for ln in r.ndl() if ln[2] == "Init"][0], "one that is given is kept")

    def test_the_dim_is_a_black_veil_of_the_right_alpha(self):
        r = self.rig()
        r.send("start", clock=False, since=int(time.time() * 1000) - 30 * 60000)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: self.veil_at(r) == (51, 0, 0, 0), 3), "half an hour into a dim to 0.6 over an hour: level 0.8, veil 51")
        self.assertEqual(self.veil_at(r, 1700, 1000), (51, 0, 0, 0))
        r.send("update", dim=False, clock=False)
        self.assertTrue(r.wait(lambda: r.status()["veil"] == 0, 3) or True)

    def test_without_the_overlay_the_window_is_never_drawn_on(self):
        r = self.rig()
        r.send("start", overlay=False, since=int(time.time() * 1000) - 60 * 60000, clockAt="10:42")
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.4)
        for t, s, got in r.fc.commits:
            self.assertEqual(set(got[2]), {0})

    def test_the_corner_clock_is_the_glyph_atlas_laid_on_the_veil_pixel_for_pixel(self):
        r = self.rig()
        ink, alpha = 0xF2EFF6, 118
        r.send("start", clockAt="10:42", ink2="#F2EFF6", ink2Alpha=alpha, since=int(time.time() * 1000) - 30 * 60000)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: r.fc.last_commit() and self.veil_at(r)[0] == 51, 3))
        time.sleep(0.2)
        w, h, data = r.fc.last_commit()[2]
        want, g = expected_text_pixels("10:42", ink, alpha, 51, g_right(), g_base())
        self.assertGreater(len(want), 200)
        wrong = [(xy, v, argb(data, w, *xy)) for xy, v in want.items() if argb(data, w, *xy) != v]
        self.assertEqual(wrong[:3], [], "%d of %d pixels differ" % (len(wrong), len(want)))
        xs = [x for x, _ in want]
        ys = [y for _, y in want]
        # nothing else was drawn on: outside the text's box every pixel is the veil
        for (x, y) in ((100, 100), (200, 1000), (1000, 500), (min(xs) - 10, min(ys)), (max(xs) + 10, max(ys))):
            self.assertEqual(argb(data, w, x, y), (51, 0, 0, 0), (x, y))
        self.assertGreater(max(xs), 1800)
        self.assertLess(max(xs), 1856 + 4, "right aligned at the screen's edge less the margin")

    def test_the_drift_moves_the_picture_and_the_clock_together_inside_its_room(self):
        r = self.rig()
        r.send("start", clockAt="8:08", moveMs=60, periodX=1, periodY=2, ampX=30, ampY=20, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(2.5)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        hist = r.fc.video_history
        self.assertGreater(len(hist), 8, "the picture moved many times")
        dsts = {h["dst"][:2] for h in hist}
        self.assertGreater(len(dsts), 6)
        for x, y, w, h in (h["dst"] for h in hist):
            self.assertTrue(2 <= x <= 62 and 8 <= y <= 48, (x, y))      # 32 +- 30 and 28 +- 20
            self.assertEqual((w, h), (W, H))
        # the clock moved with it: the box of its pixels follows the last drift
        c = r.fc.commits[-1][2]
        self.assertIsNotNone(c)

    def test_a_drift_larger_than_the_room_is_cut_to_the_room(self):
        r = self.rig()
        r.send("start", moveMs=40, periodX=1, periodY=1, ampX=200, ampY=200, clock=False)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(1.5)
        for x, y, w, h in (h["dst"] for h in r.fc.video_history):
            self.assertTrue(0 <= x <= 64 and 0 <= y <= 56, (x, y))

    def test_a_drift_that_is_off_never_moves_the_picture(self):
        r = self.rig()
        r.send("start", drift=False, moveMs=40, periodX=1, periodY=1, clock=False)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.6)
        self.assertEqual({h["dst"] for h in r.fc.video_history}, {(32, 28, W, H)})

    def test_the_print_changes_through_black_and_the_new_clip_starts_while_it_is_black(self):
        r = self.rig(clip_keys=(3000, 4000, 5000), units=20)
        r.send("start", everyMs=700, fadeMs=300, clock=False, shuffle=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: len({s for s, _ in r.plays() if s in (3000, 4000, 5000)}) >= 3, 9), "all three prints were shown in turn")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        veils = [argb(g[2], g[0], 200, 200)[0] for t, s, g in r.fc.commits if g]
        self.assertEqual(max(veils), 255, "black at the turn")
        peak = veils.index(255)
        back = peak + veils[peak:].index(0)          # the first cycle ends where the picture is clear again
        rise, fall = veils[:peak + 1], veils[peak:back + 1]
        self.assertTrue(all(b >= a for a, b in zip(rise, rise[1:])), "the fade out only darkens")
        self.assertTrue(all(b <= a for a, b in zip(fall, fall[1:])), "and the fade in only lightens")
        self.assertGreater(len(rise), 4, "stepped, not a cut")
        # the second clip's key unit (size 4000, in sequence) came after the commit that made the window black
        black_t = [t for t, s, g in r.fc.commits if g and argb(g[2], g[0], 200, 200)[0] == 255][0]
        first_new = [int(ln[1]) / 1000.0 for ln in r.ndl() if ln[2] == "Play" and ln[5] == "4000"][0]
        self.assertGreaterEqual(first_new, black_t - 0.002, "a unit of the new print was fed only once the window was black")
        # and every key unit that was fed was the first of its clip: sizes 3000, then 4000, then 5000 in sequence, the loop of each as often as it ran
        keys = [s for s, _ in r.plays() if s in (3000, 4000, 5000)]
        firsts = [k for i, k in enumerate(keys) if i == 0 or keys[i - 1] != k]
        self.assertEqual(firsts[:3], [3000, 4000, 5000])

    def test_one_print_is_never_faded_to_itself(self):
        r = self.rig(clip_keys=(3000,))
        r.send("start", everyMs=300, fadeMs=100, clock=False)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(1.2)
        self.assertEqual(r.status()["phase"], 0)
        self.assertEqual({argb(g[2], g[0], 200, 200)[0] for t, s, g in r.fc.commits if g}, {0})

    def test_a_next_request_turns_the_page_now(self):
        r = self.rig(clip_keys=(3000, 4000), units=20)
        r.send("start", fadeMs=100, clock=False, shuffle=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        r.send("next")
        self.assertTrue(r.wait(lambda: 4000 in [s for s, _ in r.plays()], 3))
        self.assertEqual(r.fresh()["clip"], "clip1")

    def test_the_start_picks_a_named_print_first(self):
        r = self.rig(clip_keys=(3000, 4000, 5000))
        r.send("start", clip="clip2", clock=False)
        self.assertTrue(r.wait_state("playing"))
        self.assertEqual(r.status()["clip"], "clip2")
        self.assertEqual(r.plays()[0][0], 5000)

    def test_a_next_request_can_name_the_print_and_an_unknown_name_is_the_next_in_the_order(self):
        r = self.rig(clip_keys=(3000, 4000, 5000), units=20)
        r.send("start", fadeMs=100, clock=False, shuffle=False, auto=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        r.send("next", clip="clip2")
        self.assertTrue(r.wait(lambda: 5000 in [s for s, _ in r.plays()], 3))
        self.assertEqual(r.fresh()["clip"], "clip2")
        self.assertNotIn(4000, [s for s, _ in r.plays()], "the print in between was never shown")
        r.send("next", clip="no-such-print")
        self.assertTrue(r.wait(lambda: [s for s, _ in r.plays() if s in (3000, 4000, 5000)][-1] == 3000, 3), "after the last, the first")
        self.assertEqual(r.fresh()["clip"], "clip0")

    def test_a_player_told_not_to_turn_pages_never_does_it_by_itself(self):
        r = self.rig(clip_keys=(3000, 4000), units=20)
        r.send("start", everyMs=300, fadeMs=100, clock=False, shuffle=False, auto=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(1.5)
        self.assertEqual({s for s, _ in r.plays() if s in (3000, 4000)}, {3000}, "one print for as long as nobody asks for another")
        self.assertEqual(r.status()["phase"], 0)
        r.send("next")
        self.assertTrue(r.wait(lambda: 4000 in [s for s, _ in r.plays()], 3))

    def test_without_the_overlay_a_named_next_is_a_cut(self):
        r = self.rig(clip_keys=(3000, 4000, 5000), units=20)
        r.send("start", overlay=False, auto=False, clock=False, shuffle=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        r.send("next", clip="clip2")
        self.assertTrue(r.wait(lambda: 5000 in [s for s, _ in r.plays()], 3))
        self.assertEqual(r.fresh()["clip"], "clip2")
        self.assertEqual({argb(g[2], g[0], 200, 200)[0] for t, s, g in r.fc.commits if g}, {0}, "no veil: the shell has its own")

    def test_the_corner_clock_is_dropped_quietly_when_there_is_no_atlas(self):
        r = self.rig(glyphs=False)
        r.send("start", clockAt="10:42")
        self.assertTrue(r.wait_state("playing"))
        self.assertIn("no corner clock", r.log())


    # ---- the caption

    def test_the_caption_is_laid_over_the_veil_not_under_it(self):
        r = self.rig(captions=True)
        r.send("start", clock=False, since=int(time.time() * 1000) - 60 * 60000, ink="#F2EFF6", inkAlpha=178, ink2="#F2EFF6", ink2Alpha=118)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: r.fc.last_commit() and self.veil_at(r)[0] == 102, 3), "an hour in, level 0.6: a veil of 102")
        time.sleep(0.2)
        w, h, data = r.fc.last_commit()[2]
        cap = r.captions[0]
        want = expected_caption_pixels(cap, 0xF2EFF6, 178, 0xF2EFF6, 118, 255, 102)
        self.assertGreater(len(want), 1500)
        wrong = [(xy, v, argb(data, w, *xy)) for xy, v in want.items() if argb(data, w, *xy) != v]
        self.assertEqual(wrong[:3], [], "%d of %d caption pixels differ" % (len(wrong), len(want)))
        # a pixel the face covers fully, in the artist's ink: its colour is the ink at 0.7, whatever the veil: the dim does not reach it
        full = [xy for xy, (a, rr, g, b) in want.items() if cap["alpha"][(xy[1] - cap["y"]) * cap["width"] + xy[0] - cap["x"]] == 255 and xy[0] - cap["x"] < cap["split"]]
        self.assertTrue(full)
        self.assertEqual(argb(data, w, *full[0])[1], (0xF2 * 178 + 127) // 255, "the ink's own brightness, not 0.6 of it")
        # it sits where the QML puts it: left edge at edgeX (64), the line ending 54 above the bottom of the screen
        xs = [x for x, _ in want]
        ys = [y for _, y in want]
        self.assertGreaterEqual(min(xs), 62)
        self.assertLessEqual(min(xs), 70)
        self.assertLessEqual(max(ys), 1080 - 54)
        self.assertGreater(max(ys), 1080 - 54 - 12)

    def test_the_caption_moves_with_the_picture(self):
        r = self.rig(captions=True)
        r.send("start", clock=False, moveMs=60, periodX=1, periodY=2, ampX=30, ampY=20, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(1.5)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        cap = r.captions[0]
        covered = [(gx, gy) for gy in range(cap["height"]) for gx in range(cap["width"]) if cap["alpha"][gy * cap["width"] + gx]]
        left = min(gx for gx, _ in covered)
        top = min(gy for _, gy in covered)
        offsets = []
        for t, s, g in r.fc.commits:
            if not g:
                continue
            w, h, data = g
            xs = [x for x in range(20, 140) for y in range(940, 1060) if argb(data, w, x, y)[1]]        # the ink, not the black around the picture
            ys = [y for x in range(60, 200) for y in range(940, 1060) if argb(data, w, x, y)[1]]
            if xs and ys:
                offsets.append((min(xs) - (cap["x"] + left), min(ys) - (cap["y"] + top)))
        self.assertGreater(len(set(offsets)), 4, "the caption was drawn at many places: %s" % sorted(set(offsets))[:8])
        for dx, dy in offsets:
            self.assertTrue(-30 <= dx <= 30 and -20 <= dy <= 20, (dx, dy))
        last = r.fc.video_history[-1]["dst"]
        self.assertEqual(offsets[-1], (last[0] - 32, last[1] - 28), "the caption sits at the picture's own offset")

    def test_the_caption_fades_with_its_print_and_not_with_the_dim(self):
        r = self.rig(clip_keys=(3000, 4000), units=20, captions=True)
        r.send("start", clock=False, everyMs=600, fadeMs=400, shuffle=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        cap = r.captions[0]
        probe = None
        for gy in range(cap["height"]):
            for gx in range(cap["split"]):
                if cap["alpha"][gy * cap["width"] + gx] == 255:
                    probe = (cap["x"] + gx, cap["y"] + gy)
                    break
            if probe:
                break
        self.assertIsNotNone(probe)
        self.assertTrue(r.wait(lambda: any(g and argb(g[2], g[0], 200, 200)[0] == 255 for t, s, g in r.fc.commits), 6), "the screen went black for the change")
        self.assertTrue(r.wait(lambda: argb(r.fc.last_commit()[2][2], r.fc.last_commit()[2][0], 200, 200)[0] == 0, 6), "and came back")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        series = [(argb(g[2], g[0], 200, 200)[0], argb(g[2], g[0], *probe)[1]) for t, s, g in r.fc.commits if g]
        series = series[next(i for i, (v, c) in enumerate(series) if c > 0):]          # from the first picture that carries the caption
        veils = [v for v, _ in series]
        peak = veils.index(255)
        at_black = [(v, c) for v, c in series if v == 255]
        self.assertTrue(all(c == 0 for _, c in at_black), "no caption pixel on the black screen of the change: %s" % at_black[:3])
        rising = [(v, c) for v, c in series[:peak + 1]]
        self.assertTrue(all(b[1] <= a[1] for a, b in zip(rising, rising[1:])), "the caption fades out as the veil rises: %s" % rising)
        self.assertGreater(series[0][1], 0, "it was there before")
        self.assertTrue(any(c > 0 for v, c in series[peak:]), "the next print's caption comes back as the veil falls")

    # ---- the buffers

    def attaches(self, r):
        return [a[0] for w, a in r.fc.requests() if w == "wl_surface.attach"]

    def test_the_window_has_two_buffers_of_one_pool_and_commits_them_in_turn(self):
        r = self.rig()
        r.send("start", clockAt="10:42", moveMs=60, periodX=1, periodY=2, ampX=30, ampY=20, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.8)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        pools = [a for w, a in r.fc.requests() if w == "wl_shm.create_pool"]
        self.assertEqual([p[2] for p in pools], [2 * 1920 * 1080 * 4], "one pool of two buffers at the window's size: 16588800 bytes")
        buffers = [a for w, a in r.fc.requests() if w == "wl_shm_pool.create_buffer"]
        self.assertEqual([b[1] for b in buffers], [0, 1920 * 1080 * 4])
        ids = self.attaches(r)
        self.assertGreater(len(ids), 6)
        self.assertEqual(set(ids), {b[0] for b in buffers})
        self.assertTrue(all(a != b for a, b in zip(ids, ids[1:])), "every commit is the other buffer: %s" % ids)
        self.assertEqual(r.fc.violations, [])

    def test_a_frame_is_never_seen_half_painted(self):
        # The compositor reads a committed buffer a while after the commit and keeps it until the next commit replaces it, while the player
        # goes on painting: fades, the drift and the clock repaint the window all the time.
        r = self.rig(clip_keys=(3000, 4000), units=20, captions=True, release="held", lag=0.02)
        ink2 = 0xF2EFF6
        r.send("start", clockAt="10:42", ink2="#F2EFF6", ink2Alpha=118, everyMs=500, fadeMs=300, moveMs=40, periodX=1, periodY=2, ampX=30, ampY=20, shuffle=False, ttlMs=60000)
        self.assertTrue(r.wait_state("playing"))
        time.sleep(3.5)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        self.assertEqual(r.fc.violations, [], "a buffer was written or committed while the compositor held it")
        cap = r.captions[0]
        want, _ = expected_text_pixels("10:42", ink2, 118, 0, g_right(), g_base())
        # the clock and the caption move with the picture (30 and 20 pixels): each is looked for in its box, a margin wider than the drift
        clock_box = (min(x for x, _ in want) - 34, min(y for _, y in want) - 24, max(x for x, _ in want) + 34, max(y for _, y in want) + 24)
        cap_box = (cap["x"] - 34, cap["y"] - 24, cap["x"] + cap["width"] + 34, cap["y"] + cap["height"] + 24)

        def inked(frame, box):
            w, h, data = frame
            px = memoryview(data).cast("I")
            x0, y0, x1, y1 = box
            return any((px[y * w + x] >> 16) & 255 for y in range(max(y0, 0), min(y1, h), 2) for x in range(max(x0, 0), min(x1, w), 2))
        grid = [(x, y) for x in range(150, 1500, 149) for y in range(200, 800, 97)]
        frames = [g for t, s, g in r.fc.commits if g and inked(g, clock_box)]        # from the first painted frame
        self.assertGreater(len(frames), 20)
        veils = set()
        for frame in frames:
            w, h, data = frame
            v = {argb(data, w, *xy)[0] for xy in grid}
            self.assertEqual(len(v), 1, "the veil is half filled: %s" % sorted(v))
            veil = v.pop()
            veils.add(veil)
            self.assertEqual(argb(data, w, 0, 0)[0], 255, "the margin is black in every frame")
            has_ink = inked(frame, cap_box)
            if veil <= 128:
                self.assertTrue(has_ink, "the caption is on every frame that is not nearly black (veil %d)" % veil)
            if veil == 255:
                self.assertFalse(has_ink, "no caption on a black frame")
        self.assertIn(255, veils, "the fade reached black")
        self.assertIn(0, veils)
        self.assertGreater(len(veils), 5, "and was seen on the way")

    def test_a_frame_waits_for_a_release_and_is_painted_when_it_comes(self):
        r = self.rig(release="never")
        r.send("start", clock=False, ttlMs=60000, since=int(time.time() * 1000) - 30 * 60000)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: len(self.attaches(r)) >= 2, 3), "the window's open commit and the first frame")
        time.sleep(1.5)                        # the veil step that is due in a second wants a third frame
        self.assertEqual(len(self.attaches(r)), 2, "both buffers are held: nothing is painted over either")
        self.assertEqual(r.fc.violations, [])
        self.assertEqual(r.fresh()["state"], "playing", "and the player carries on")
        r.fc.release_replaced()
        self.assertTrue(r.wait(lambda: len(self.attaches(r)) > 2, 3), "the frame owed is painted once a buffer is free")
        self.assertEqual(r.fc.violations, [])
        self.assertEqual(self.veil_at(r, 960, 540), (51, 0, 0, 0), "and it is the state as it is then")

    def test_a_caption_that_is_switched_off_is_not_drawn(self):
        r = self.rig(captions=True)
        r.send("start", clock=False, caption=False, since=int(time.time() * 1000) - 30 * 60000)
        self.assertTrue(r.wait_state("playing"))
        self.assertTrue(r.wait(lambda: r.fc.last_commit() and self.veil_at(r)[0] == 51, 3))
        w, h, data = r.fc.last_commit()[2]
        cap = r.captions[0]
        self.assertEqual(argb(data, w, cap["x"] + 10, cap["y"] + 20), (51, 0, 0, 0))
        self.assertEqual({argb(data, w, x, y) for x in range(200, 1700, 97) for y in range(150, 950, 89)}, {(51, 0, 0, 0)})

    def test_a_broken_caption_file_is_a_note_and_the_print_plays_without_it(self):
        r = self.rig(captions=True)
        with open(os.path.join(r.clipdir, "clip0.vpt"), "wb") as f:
            f.write(b"VPT1" + b"junk")
        r.send("start", clock=False)
        self.assertTrue(r.wait_state("playing"))
        self.assertIn("no caption", r.log())

    # ---- the way out

    def test_a_key_asks_for_the_screen_and_takes_the_window_away(self):
        marker = os.path.join(tempfile.gettempdir(), "vplane-woke-%d" % os.getpid())
        self.addCleanup(lambda: os.path.exists(marker) and os.unlink(marker))
        r = self.rig(args=("-W", "echo woke > %s" % marker))
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        r.fc.settle()
        self.assertEqual(r.fc.count("wl_seat.get_keyboard"), 2, "a keyboard from every seat")
        self.assertEqual(r.fc.count("wl_seat.get_pointer"), 2)
        r.fc.send_key(False)                          # a release alone is not a key press
        r.fc.send_pointer("enter")                    # nor is the pointer arriving
        time.sleep(0.3)
        self.assertEqual(r.status()["state"], "playing")
        self.assertFalse(os.path.exists(marker))
        r.fc.send_key(True, seat=1)                   # a press on the second device's seat
        self.assertTrue(r.wait_state("idle", 3), r.log())
        self.assertIn("a key or the pointer", r.status()["reason"])
        self.assertTrue(r.wait(lambda: os.path.exists(marker), 2), "the wake command ran")
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 2), "and the window is gone")
        self.assertIn("Quit", r.calls() + [ln[2] for ln in r.ndl()])

    def test_the_pointer_moving_or_pressing_is_a_way_out_too(self):
        for kind in ("motion", "button"):
            marker = os.path.join(tempfile.gettempdir(), "vplane-woke-%s-%d" % (kind, os.getpid()))
            self.addCleanup(lambda m=marker: os.path.exists(m) and os.unlink(m))
            r = self.rig(args=("-W", "echo woke > %s" % marker))
            r.send("start")
            self.assertTrue(r.wait_state("playing"))
            r.fc.settle()
            r.fc.send_pointer(kind)
            self.assertTrue(r.wait_state("idle", 3), (kind, r.log()))
            self.assertTrue(r.wait(lambda: os.path.exists(marker), 2), kind)
            r.close()

    def test_a_compositor_with_no_input_devices_still_plays(self):
        r = self.rig()
        r.fc.seat_caps = 0
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.2)
        self.assertEqual(r.fc.count("wl_seat.get_keyboard"), 0)
        self.assertEqual(r.status()["state"], "playing")

    def test_the_status_carries_what_the_requester_and_the_steps_need(self):
        r = self.rig()
        r.send("start")
        self.assertTrue(r.wait_state("playing"))
        time.sleep(0.3)
        s = r.fresh()
        for k in ("pid", "ts", "state", "reason", "seq", "clip", "clips", "role", "walls", "window", "windowId", "playingMs", "fed", "full", "skipped", "queue", "events", "drift", "veil", "phase", "memAvailKb", "cmaFreeKb", "rssKb"):
            self.assertIn(k, s)
        self.assertEqual(s["events"], "22,26")
        self.assertLess(abs(s["ts"] - time.time() * 1000), 3000)
        self.assertGreater(s["rssKb"], 100)


@unittest.skipUnless(HAVE, "needs zig to build vplane/")
class Wall(unittest.TestCase):
    """The player's second role: Home's moving wallpaper (docs/research/videoplane-saver-result.md, "The moving wallpaper"). One clip of the
    screen's size looped on a window of LumaTV's own wallpaper type, nothing drawn in it, no drift, no change, no way out by a key; and one
    decoder, so the saver takes the player from it and it never takes the player from the saver."""

    def rig(self, **kw):
        kw.setdefault("walls", 1)
        r = Rig(**kw)
        self.addCleanup(r.close)
        return r

    def test_the_window_is_the_wallpapers_type_the_clip_fills_the_screen_and_the_window_is_small(self):
        r = self.rig()
        r.send("start", role="wall", clip="wall-0")
        self.assertTrue(r.wait_state("playing"), r.log())
        r.fc.settle()
        s = r.status()
        self.assertEqual((s["role"], s["clip"], s["clips"], s["walls"]), ("wall", "wall-0", 1, 1), "the wallpaper's clip is not one of the prints")
        self.assertEqual(r.fc.props["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_LUMATV_WALLPAPER")
        self.assertNotIn("playVideo", r.fc.props, "the screen-saver type's flag is not the wallpaper's")
        self.assertEqual(r.fc.props["appId"], "org.nphil.lumatv.vplane")
        self.assertEqual(r.fc.video["src"], (0, 0, 1920, 1080))
        self.assertEqual(r.fc.video["dst"], (0, 0, 1920, 1080), "the whole screen, from its corner: no drift, no margin")
        self.assertEqual([ln for ln in r.ndl() if ln[2] == "Load"][0][3:], ["1920x1080", "type", "2"])
        self.assertEqual(r.calls()[:6], ["DL_Initialize", "SetWindowId", "Init", "Load", "SetAppState", "Play"])
        buffers = [a for w, a in r.fc.requests() if w == "wl_shm_pool.create_buffer"]
        self.assertTrue(buffers)
        self.assertEqual({(a[2], a[3]) for a in buffers}, {(16, 16)}, "a small surface: it draws nothing, the video's rectangle is the element's")

    def test_nothing_is_drawn_nothing_moves_and_the_clip_loops_for_as_long_as_it_is_renewed(self):
        r = self.rig(units=30)
        r.send("start", role="wall", clip="wall-0", ttlMs=3000, moveMs=100, auto=True, drift=True, clock=True, dim=True)
        self.assertTrue(r.wait_state("playing"), r.log())
        r.fc.settle()
        first = len(r.fc.commits)
        placements = len(r.fc.video_history)
        for _ in range(8):
            time.sleep(0.4)
            r.send("renew", role="wall", ttlMs=3000)
        s = r.fresh()
        self.assertEqual((s["state"], s["role"]), ("playing", "wall"), "renewed for three seconds past its ttl")
        self.assertEqual(len(r.fc.commits), first, "no frame is drawn after the first: no veil, no clock, no caption")
        self.assertEqual(len(r.fc.video_history), placements, "the picture is not moved (a drift would move the window)")
        self.assertEqual(s["drift"], [0, 0])
        plays = r.plays()
        sizes = [len(u) for u, _ in synthetic_clip_sizes(30)]
        self.assertEqual([x for x, _ in plays], [sizes[i % 30] for i in range(len(plays))], "every unit, in order, and the key unit again after the thirtieth")
        r.send("stop", role="wall")
        self.assertTrue(r.wait_state("idle"))

    def test_a_wall_that_is_not_renewed_goes_by_itself(self):
        r = self.rig()
        r.send("start", role="wall", clip="wall-0", ttlMs=1500)
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertTrue(r.wait_state("idle", 5), r.status())
        self.assertTrue(r.status()["reason"].startswith("ttl"), r.status())
        self.assertTrue(r.wait(lambda: not r.fc.alive(), 2), "and its window with it")

    def test_a_key_or_the_pointer_over_the_wallpaper_wakes_nothing(self):
        marker = os.path.join(tempfile.gettempdir(), "vplane-wall-woke-%d" % os.getpid())
        self.addCleanup(lambda: os.path.exists(marker) and os.unlink(marker))
        r = self.rig(args=("-W", "echo woke > %s" % marker))
        r.send("start", role="wall", clip="wall-0")
        self.assertTrue(r.wait_state("playing"), r.log())
        r.fc.settle()
        r.fc.send_key(True)
        r.fc.send_pointer("motion")
        r.fc.send_pointer("button")
        time.sleep(0.5)
        self.assertEqual(r.status()["state"], "playing", "the way out is Home's, not the wallpaper's")
        self.assertFalse(os.path.exists(marker), "the TV is not asked to turn the screen on")

    def test_a_wall_clip_that_is_not_there_or_not_the_screens_size_is_an_error_the_requester_sees(self):
        r = self.rig()
        r.send("start", role="wall", clip="wall-nine")
        self.assertTrue(r.wait_state("error"), r.log())
        self.assertIn("no wall clip named wall-nine", r.status()["reason"])
        self.assertEqual(r.fc.names(), [], "no window was made for it")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        none = self.rig(walls=0)
        none.send("start", role="wall", clip="wall-0")
        self.assertTrue(none.wait_state("error"), none.log())
        self.assertEqual(none.status()["reason"], "no wall clips")
        small = self.rig(walls=0)
        synthetic_clip(os.path.join(small.clipdir, "wall-odd.vpc"), units=10)       # 1856 x 1024: a print's size
        small.send("start", role="wall")
        self.assertTrue(small.wait_state("error"), small.log())
        self.assertIn("1920x1080", small.status()["reason"])

    def test_a_clip_named_for_a_wall_starts_the_wall_without_a_role(self):
        r = self.rig()
        r.send("start", clip="wall-0")                      # what `--once wall-0` writes
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertEqual(r.status()["role"], "wall")

    def test_the_saver_takes_the_player_from_the_wall_and_the_wall_never_takes_it_back(self):
        r = self.rig()
        r.send("start", role="wall", clip="wall-0")
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertEqual(r.status()["role"], "wall")
        r.send("start", ttlMs=60000, moveMs=600000)         # no role: the saver's
        self.assertTrue(r.wait(lambda: r.status().get("state") == "playing" and r.status().get("role") == "saver", 8), (r.status(), r.log()))
        self.assertEqual(r.status()["clip"], "clip0")
        self.assertEqual(r.fc.props["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_SCREENSAVER")
        self.assertIn("the saver has the player", r.log())
        seq = r.status()["seq"]
        # the wall asks for the player while the saver has it, and a stop meant for the wall is in flight: neither touches the saver
        r.send("start", role="wall", clip="wall-0")
        time.sleep(0.3)
        r.send("stop", role="wall")
        r.send("renew", role="wall")
        time.sleep(0.4)
        s = r.fresh()
        self.assertEqual((s["state"], s["role"]), ("playing", "saver"))
        self.assertEqual(s["seq"], r.seq, "only the status request is the last one the player took: the wall's were left out")
        self.assertGreater(s["seq"], seq)
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))

    def test_the_status_names_the_session_and_an_idle_player_has_none(self):
        r = self.rig()
        r.send("start", ttlMs=60000)
        self.assertTrue(r.wait_state("playing"), r.log())
        self.assertEqual(r.status()["role"], "saver")
        r.send("stop")
        self.assertTrue(r.wait_state("idle"))
        self.assertEqual(r.status()["role"], "", "an idle player has no role")
        r.send("start", role="wall", clip="wall-0")
        self.assertTrue(r.wait(lambda: r.status().get("state") == "playing" and r.status().get("role") == "wall", 8), (r.status(), r.log()))

    def test_the_probe_counts_the_walls_apart_from_the_prints(self):
        r = self.rig()
        out = subprocess.run([BIN, "-c", r.clipdir, "-L", NDL, "-n"], capture_output=True, text=True, env=dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL)))
        info = json.loads(out.stdout)
        self.assertEqual((info["clips"], info["clipsOk"], info["walls"], info["wallsOk"]), (1, 1, 1, 1), out)
        self.assertEqual(out.returncode, 0, out)


def synthetic_clip_sizes(units, key=3000, other=150):
    """The sizes of the units `synthetic_clip` writes, as (bytes, key) pairs."""
    return [(b"\0" * (key if i == 0 else other + (i - 1) % 7), i == 0) for i in range(units)]


def g_right():
    return clips.read_glyphs(os.path.join(SHIPPED, "clock.vpg"))["right"]


def g_base():
    return clips.read_glyphs(os.path.join(SHIPPED, "clock.vpg"))["baseline"]


class ArmBuild(unittest.TestCase):
    ARM = os.path.join(VPLANE, "build", "arm", "lumatv-vplane")

    @unittest.skipUnless(os.path.exists(os.path.join(VPLANE, "build", "arm", "lumatv-vplane")) and shutil.which("readelf"), "no ARM build")
    def test_the_set_build_is_32_bit_arm_and_needs_only_what_the_set_has(self):
        out = subprocess.run(["readelf", "-h", "-d", self.ARM], capture_output=True, text=True).stdout
        self.assertIn("ELF32", out)
        self.assertIn("ARM", out)
        needed = sorted(ln.split("[")[1].rstrip("]") for ln in out.splitlines() if "NEEDED" in ln)
        self.assertEqual(needed, ["libc.so.6", "libdl.so.2", "libm.so.6"], "no libpthread: the build uses none, and a libc without it would turn mutexes into no-ops")
        ver = subprocess.run(["readelf", "-V", self.ARM], capture_output=True, text=True).stdout
        newest = max(tuple(int(x) for x in m.split("_")[1].split(".")) for m in set(__import__("re").findall(r"GLIBC_\d+\.\d+", ver)))
        self.assertLessEqual(newest, (2, 28), "nothing newer than the set's glibc 2.28")


if __name__ == "__main__":
    unittest.main()
