"""Run with: cd tests && python3 -m unittest test_gfn_feeder   (needs zig for the host build of gfn/feeder/, skipped without it)

lumatv-gfn-feeder (gfn/feeder/), the 32-bit process that puts the GeForce NOW stream on the video plane, on the host: the real program
built for x86-64 against the NDL stand-in (gfn/feeder/tests/fake_ndl.c, which logs every call and flags the orders LG's library punishes)
and the small Wayland server of the saver's tests (vplane/tests/fake_wayland.py). Shown here: the call order, that a file is split into
pictures and fed at its rate with a rising PTS, the HDR and game-mode calls, Opus audio beside the video, what a full buffer does, the
engine's socket protocol end to end, and the facts of the ARM binary. What this cannot show is that LG's pipeline agrees: the TV steps in
docs/research/gfn-app-result.md measure that.
"""
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FEEDER = os.path.join(ROOT, "gfn", "feeder")
sys.path.insert(0, os.path.join(ROOT, "vplane", "tests"))
sys.path.insert(0, os.path.join(ROOT, "gfn", "tools"))

from fake_wayland import FakeCompositor  # noqa: E402

BIN = os.path.join(FEEDER, "build", "host", "lumatv-gfn-feeder")
ARM = os.path.join(FEEDER, "build", "arm", "lumatv-gfn-feeder")
NDL = os.path.join(FEEDER, "build", "host", "libNDL_directmedia.so.1")
STREAMS = os.path.join(ROOT, "gfn", "build", "streams")

FRAME = struct.Struct("<BBHI")
LOAD = struct.Struct("<8I15Iii")
MEDIA_HEAD = struct.Struct("<qB7x")
ACCEPT = struct.Struct("<qii")
DROP = struct.Struct("<qii")
EVENT = struct.Struct("<iiq")
STATUS = struct.Struct("<5Q4I2I2II2I2III")
HELLO = struct.Struct("<4I32s64s")
GF = dict(LOAD=1, VIDEO=2, AUDIO=3, FLUSH=4, UNLOAD=5, STATE=6, HDR=7, PING=8, LOADED=16, EVENT=17, ACCEPT=18, DROP=19, STATUS=20, ERROR=21, PONG=22, HELLO=23)


def build():
    if os.path.exists(BIN) and os.path.exists(NDL) and os.path.exists(ARM):
        newest = max(os.path.getmtime(os.path.join(FEEDER, d, f)) for d in ("src", "tests") for f in os.listdir(os.path.join(FEEDER, d)))
        if os.path.getmtime(BIN) >= newest:
            return True
    if not shutil.which("zig"):
        return False
    return subprocess.run(["sh", os.path.join(FEEDER, "build.sh")], capture_output=True).returncode == 0


HAVE = build()


def hevc_nal(t, first=False, size=40):
    body = bytes([0x80 if first else 0x00, 0x5a]) + bytes([t & 255]) * (size - 7)
    return b"\x00\x00\x00\x01" + bytes([t << 1, 1]) + body


def hevc_stream(pictures=60, aud=True, key_size=3000, other=150):
    """An Annex B stream the fake library accepts: headers and an IDR first, then TRAIL pictures of one slice; AUDs when asked."""
    out = b""
    for i in range(pictures):
        if aud:
            out += hevc_nal(35, size=7)
        if i % 30 == 0:
            out += hevc_nal(32) + hevc_nal(33) + hevc_nal(34) + hevc_nal(19, True, key_size)
        else:
            out += hevc_nal(1, True, other + i % 5)
    return out


def opus_file(path, packets=50):
    with open(path, "wb") as f:
        for i in range(packets):
            p = bytes([0xfc, i & 255]) + bytes(100 + i % 7)
            f.write(struct.pack("<H", len(p)) + p)


def lines(path):
    with open(path) as f:
        return [l for l in f.read().splitlines() if l.strip()]


class Run:
    """One feeder run with its compositor and library stand-ins."""

    def __init__(self, args, env=None, compositor=True, timeout=30):
        self.tmp = tempfile.mkdtemp(prefix="gfn-feeder-test-")
        self.ndl_log = os.path.join(self.tmp, "ndl.log")
        self.stats = os.path.join(self.tmp, "stats.jsonl")
        self.err_path = os.path.join(self.tmp, "feeder.log")
        self.fc = FakeCompositor() if compositor else None
        e = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=self.ndl_log, FAKE_NDL_VERBOSE="1")
        e.pop("WAYLAND_DISPLAY", None)
        e.pop("WAYLAND_SOCKET", None)
        if self.fc:
            e.update(self.fc.env())
        e.update(env or {})
        self.env = e
        self.args = args
        self.timeout = timeout

    def go(self):
        with open(self.err_path, "w") as err:
            self.proc = subprocess.run([BIN, *self.args, "-L", NDL, "--stats", self.stats], env=self.env, stderr=err, timeout=self.timeout)
        return self

    def stats_lines(self):
        return [json.loads(l) for l in lines(self.stats)]

    def summary(self):
        return [l for l in self.stats_lines() if l.get("summary")][0]

    def ndl(self):
        return lines(self.ndl_log) if os.path.exists(self.ndl_log) else []

    def ndl_names(self):
        return [l.split(" ", 2)[2].split(" ")[0] for l in self.ndl()]

    def err(self):
        return open(self.err_path).read()

    def close(self):
        if self.fc:
            self.fc.close()
        shutil.rmtree(self.tmp, ignore_errors=True)


@unittest.skipUnless(HAVE, "zig is needed to build the feeder for the host")
class Play(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gfn-feeder-clip-")
        self.clip = os.path.join(self.tmp, "clip.hevc")
        with open(self.clip, "wb") as f:
            f.write(hevc_stream(60))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_call_order_and_every_picture_fed_at_the_clip_rate(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "-w", "card"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            names = r.ndl_names()
            for a, b in zip(["DL_Initialize", "SetWindowId", "Init", "Load", "SetAppState"], ["SetWindowId", "Init", "Load", "SetAppState", "Play"]):
                self.assertLess(names.index(a), names.index(b), names)
            self.assertNotIn("VIOLATION", " ".join(names))
            self.assertEqual(sum(1 for n in names if n == "Play"), 60)
            s = r.summary()
            self.assertEqual((s["units"], s["fed"], s["dropped"]), (60, 60, 0))
            self.assertGreaterEqual(s["seconds"], 0.9)
            self.assertLess(s["seconds"], 1.8)
            self.assertEqual((s["loaded"], s["playing"], s["error"]), (1, 1, 0))
            self.assertIn("22", s["events"])
            self.assertIn("26", s["events"])
            pts = [int(l.split(" pts ")[1].split()[0]) for l in r.ndl() if " Play #" in l]
            self.assertEqual(pts, sorted(pts))
            self.assertEqual(pts[1] - pts[0], 1000000000 // 60)
            # the window: card type, the app id, an exported element placed full screen, the video rectangle the clip's size
            self.assertIn("wl_webos_foreign.export_element", r.fc.names())
            self.assertIn("wl_webos_exported.set_exported_window", r.fc.names())
            self.assertIn("SetWindowId _Window_Id_", " ".join(r.ndl()))
            starts = [l for l in r.stats_lines() if l.get("start")]
            self.assertTrue(starts[0]["window"].startswith("_Window_Id_"))
            per_second = [l for l in r.stats_lines() if "t" in l]
            self.assertTrue(per_second and all("queue" in l and "cma_free_kb" in l for l in per_second))
        finally:
            r.close()

    def test_hdr_drop_threshold_and_pts_unit(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--hdr", "--drop-threshold", "2", "--pts-unit", "us"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            log = " ".join(r.ndl())
            self.assertIn("SetHDRInfo primaries 13250,34500 7500,3000 34000,16000 white 15635,16450 lum 10000000,1 cll 1000,400 vui 16,9,9", log)
            self.assertIn("SetFrameDropThreshold 2", log)
            names = r.ndl_names()
            self.assertLess(names.index("Load"), names.index("SetHDRInfo"))
            self.assertLess(names.index("Load"), names.index("SetFrameDropThreshold"))
            pts = [int(l.split(" pts ")[1].split()[0]) for l in r.ndl() if " Play #" in l]
            self.assertEqual(pts[1] - pts[0], 1000000 // 60)
        finally:
            r.close()

    def test_opus_audio_beside_the_video(self):
        pk = os.path.join(self.tmp, "tone.opuspkts")
        opus_file(pk, 50)
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--audio", pk]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            log = r.ndl()
            self.assertTrue(any("audio OPUS 2 ch 48.0 kHz" in l for l in log), log[:8])
            self.assertEqual(sum(1 for l in log if "AudioPlay #" in l), 50)
            s = r.summary()
            self.assertEqual((s["afed"], s["adropped"]), (50, 0))
            apts = [int(l.split(" pts ")[1].split()[0]) for l in log if "AudioPlay #" in l]
            self.assertEqual(apts[1] - apts[0], 20000000)
        finally:
            r.close()

    def test_a_full_buffer_is_retried_then_dropped(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60"], env={"FAKE_NDL_FULL_EVERY": "10"}).go()
        try:
            s = r.summary()
            self.assertEqual((s["fed"], s["dropped"]), (60, 0))
            self.assertEqual(s["retries"], 6)
            r2 = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--full-wait-ms", "3"], env={"FAKE_NDL_FULL_EVERY": "10", "FAKE_NDL_FULL_STICKY": "1"}).go()
            try:
                s2 = r2.summary()
                self.assertEqual(s2["dropped"], 6)
                self.assertEqual(s2["fed"], 54)
                secs = [l for l in r2.stats_lines() if "t" in l]
                self.assertTrue(any(l["wait_max_us"] >= 3000 for l in secs))
            finally:
                r2.close()
        finally:
            r.close()

    def test_a_pipeline_error_stops_the_run(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60"], env={"FAKE_NDL_ERROR_AFTER": "20"}).go()
        try:
            self.assertEqual(r.proc.returncode, 6)
            s = r.summary()
            self.assertEqual(s["error"], 1)
            self.assertLess(s["fed"], 60)
            self.assertIn("Unload", r.ndl_names())
            self.assertIn("Quit", r.ndl_names())
        finally:
            r.close()

    def test_a_given_window_id_means_no_compositor(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--window-id", "_Window_Id_77"], compositor=False).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            self.assertIn("SetWindowId _Window_Id_77", " ".join(r.ndl()))
            self.assertEqual(r.summary()["fed"], 60)
        finally:
            r.close()

    def test_loop_and_seconds(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--loop", "--seconds", "2"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            s = r.summary()
            self.assertGreaterEqual(s["fed"], 100)
            self.assertLessEqual(s["fed"], 125)
            self.assertNotIn("VIOLATION", " ".join(r.ndl()))
        finally:
            r.close()

    def test_burst_without_the_library_splits_and_counts(self):
        with open(self.clip, "wb") as f:
            f.write(hevc_stream(200, aud=False))
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--burst", "--no-ndl", "--no-window"], compositor=False).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            s = r.summary()
            self.assertEqual((s["units"], s["fed"]), (200, 200))
            self.assertLess(s["seconds"], 1.0)
        finally:
            r.close()

    def test_refusals_are_reported(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60"], env={"FAKE_NDL_FAIL": "load"}).go()
        try:
            self.assertEqual(r.proc.returncode, 4)
            self.assertIn("NDL_DirectMediaLoad: load refused", r.err())
        finally:
            r.close()


class Client:
    """The engine's end of the socket, in Python."""

    def __init__(self, path, timeout=10):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(timeout)
        end = time.time() + timeout
        while True:
            try:
                self.s.connect(path)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if time.time() > end:
                    raise
                time.sleep(0.05)
        self.buf = b""

    def send(self, t, payload=b""):
        self.s.sendall(FRAME.pack(t, 0, 0, len(payload)) + payload)

    def recv(self):
        while len(self.buf) < FRAME.size or len(self.buf) < FRAME.size + FRAME.unpack_from(self.buf)[3]:
            d = self.s.recv(65536)
            if not d:
                raise EOFError
            self.buf += d
        t, _, _, n = FRAME.unpack_from(self.buf)
        p = self.buf[FRAME.size:FRAME.size + n]
        self.buf = self.buf[FRAME.size + n:]
        return t, p

    def until(self, t, limit=50):
        got = []
        for _ in range(limit):
            m = self.recv()
            got.append(m)
            if m[0] == t:
                return m, got
        raise AssertionError("no message %d in %r" % (t, [g[0] for g in got]))

    def load(self, codec=2, w=320, h=240, fps=60, audio=1, hdr=1, drop=2, pts_unit=1):
        hdrv = (13250, 34500, 7500, 3000, 34000, 16000, 15635, 16450, 10000000, 1, 1000, 400, 16, 9, 9)
        self.send(GF["LOAD"], LOAD.pack(codec, w, h, fps, audio, 2, 48000, hdr, *hdrv, drop, pts_unit))

    def close(self):
        self.s.close()


@unittest.skipUnless(HAVE, "zig is needed to build the feeder for the host")
class Serve(unittest.TestCase):
    def test_the_engine_protocol_end_to_end(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-serve-")
        sock = os.path.join(tmp, "feed.sock")
        ndl_log = os.path.join(tmp, "ndl.log")
        fc = FakeCompositor()
        env = dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=ndl_log, FAKE_NDL_VERBOSE="1", FAKE_NDL_BUFFER_EVENTS="1")
        err = open(os.path.join(tmp, "feeder.log"), "w")
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "-L", NDL, "--stats", os.path.join(tmp, "stats.jsonl")], env=env, stderr=err, stdout=subprocess.DEVNULL)
        try:
            c = Client(sock)
            t, p = c.recv()
            self.assertEqual(t, GF["HELLO"])
            version, library, pid, window, wid, lib = HELLO.unpack(p)
            self.assertEqual((version, library, window), (1, 1, 1))
            self.assertEqual(pid, proc.pid)
            self.assertTrue(wid.rstrip(b"\0").decode().startswith("_Window_Id_"))
            self.assertEqual(lib.rstrip(b"\0"), b"libNDL_directmedia.so.1")
            c.load()
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
            self.assertTrue(p[4:].decode().startswith("_Window_Id_"))
            (t, p), _ = c.until(GF["EVENT"])
            self.assertEqual(EVENT.unpack_from(p)[0], 22)
            # 150 pictures, an IDR first, then one with a wrong PTS order is not sent (the engine's clock rises)
            stream = hevc_stream(150)
            units = []
            pos = 0
            while pos < len(stream):
                nxt = stream.find(b"\x00\x00\x00\x01\x46", pos + 1)
                units.append(stream[pos:nxt if nxt > 0 else len(stream)])
                pos = nxt if nxt > 0 else len(stream)
            self.assertEqual(len(units), 150)
            t0 = 1_000_000_000
            accepted = 0
            for i, u in enumerate(units):
                c.send(GF["VIDEO"], MEDIA_HEAD.pack(t0 + i * 16_666_667, 1 if i % 30 == 0 else 0) + u)
                if i % 3 == 0:
                    c.send(GF["AUDIO"], MEDIA_HEAD.pack(t0 + i * 16_666_667, 0) + bytes([0xfc, i & 255]) + bytes(80))
            c.send(GF["PING"], struct.pack("<q", 4242))
            seen = {}
            deadline = time.time() + 5
            while time.time() < deadline and (accepted < 150 or GF["STATUS"] not in seen or GF["PONG"] not in seen):
                t, p = c.recv()
                seen[t] = p
                if t == GF["ACCEPT"]:
                    accepted += 1
                    pts, queue, wait = ACCEPT.unpack(p)
                    self.assertGreaterEqual(queue, 0)
            self.assertEqual(accepted, 150)
            self.assertIn(GF["STATUS"], seen)
            st = STATUS.unpack_from(seen[GF["STATUS"]])
            self.assertEqual(st[0], 150)                      # video_fed
            self.assertEqual(st[2], 50)                       # audio_fed
            self.assertEqual(st[14], 1)                       # loaded
            self.assertEqual(struct.unpack_from("<qq", seen[GF["PONG"]])[0], 4242)
            evs = [EVENT.unpack_from(p)[0] for (t, p) in [(t, p) for t, p in seen.items()] if t == GF["EVENT"]]
            self.assertTrue(evs)
            c.send(GF["FLUSH"])
            c.send(GF["STATE"], b"\x00")
            c.send(GF["STATE"], b"\x01")
            c.send(GF["UNLOAD"])
            time.sleep(0.3)
            c.close()
            proc.wait(timeout=10)
            self.assertEqual(proc.returncode, 0)
            log = open(ndl_log).read()
            self.assertIn("FlushRenderBuffer", log)
            self.assertIn("SetAppState 1", log)
            self.assertIn("SetHDRInfo", log)
            self.assertIn("SetFrameDropThreshold 2", log)
            self.assertIn("audio OPUS 2 ch 48.0 kHz", log)
            self.assertIn("Unload", log)
            self.assertNotIn("VIOLATION", log)
            self.assertFalse(os.path.exists(sock))
        finally:
            if proc.poll() is None:
                proc.kill()
            err.close()
            fc.close()
            shutil.rmtree(tmp, ignore_errors=True)

    def test_a_refused_load_is_answered_not_fatal(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-serve-")
        sock = os.path.join(tmp, "feed.sock")
        env = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(tmp, "ndl.log"), FAKE_NDL_FAIL="load")
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "--window-id", "_Window_Id_5", "-L", NDL], env=env, stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
        try:
            c = Client(sock)
            self.assertEqual(c.recv()[0], GF["HELLO"])
            c.load()
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 0)
            self.assertIn(b"load refused", p)
            c.close()
            proc.wait(timeout=10)
            self.assertEqual(proc.returncode, 0)
        finally:
            if proc.poll() is None:
                proc.kill()
            shutil.rmtree(tmp, ignore_errors=True)


@unittest.skipUnless(HAVE, "zig is needed to build the feeder")
class Binary(unittest.TestCase):
    def test_arm_binary_is_32_bit_soft_float_with_libc_and_libdl_only(self):
        out = subprocess.run(["readelf", "-h", "-d", ARM], capture_output=True, text=True).stdout
        self.assertIn("ELF32", out)
        self.assertIn("Machine:                           ARM", out)
        needed = [l.split("[")[1].rstrip("]") for l in out.splitlines() if "NEEDED" in l]
        self.assertEqual(sorted(needed), ["libc.so.6", "libdl.so.2"])
        self.assertIn("Flags:                             0x5000200", out)

    def test_probe_reports_the_library_and_the_protocol_sizes(self):
        r = subprocess.run([BIN, "--probe", "-L", NDL], capture_output=True, text=True, env=dict(os.environ, FAKE_NDL_LOG=os.devnull))
        self.assertEqual(r.returncode, 0, r.stderr)
        j = json.loads(r.stdout)
        self.assertTrue(j["library"])
        self.assertEqual(j["features"], 31)
        self.assertEqual(j["sizes"], {"load": LOAD.size, "status": 104, "hello": HELLO.size})


@unittest.skipUnless(HAVE and os.path.exists(os.path.join(STREAMS, "manifest.json")), "the generated probe streams are needed (gfn/tools/gfn_streams.py gen)")
class GeneratedStreams(unittest.TestCase):
    def test_every_generated_stream_splits_into_its_pictures(self):
        m = json.load(open(os.path.join(STREAMS, "manifest.json")))
        for name, e in sorted(m["streams"].items()):
            if "error" in e or e["codec"] == "av1":
                continue
            path = os.path.join(STREAMS, e["file"])
            with self.subTest(name):
                r = Run(["--play", path, "--codec", e["codec"], "--size", "%dx%d" % (e["width"], e["height"]), "--fps", str(e["fps"]), "--burst",
                         "--no-ndl", "--no-window"], compositor=False, timeout=120).go()
                try:
                    self.assertEqual(r.proc.returncode, 0, r.err())
                    s = r.summary()
                    self.assertEqual(s["units"], e["fps"] * e["seconds"])
                    self.assertEqual(s["fed"], s["units"])
                finally:
                    r.close()


if __name__ == "__main__":
    unittest.main()
