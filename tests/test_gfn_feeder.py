"""Run with: cd tests && python3 -m unittest test_gfn_feeder   (needs zig for the host build of gfn/feeder/, skipped without it)

lumatv-gfn-feeder (gfn/feeder/), the 32-bit process that puts the GeForce NOW stream on the video plane, on the host: the real program
built for x86-64 against the NDL stand-in (gfn/feeder/tests/fake_ndl.c, which logs every call and flags the orders LG's library punishes)
and the small Wayland server of the saver's tests (vplane/tests/fake_wayland.py). Shown here: the call order, that a file is split into
pictures and fed at its rate with a rising PTS, the HDR and game-mode calls, Opus audio beside the video, what a full buffer does, the
engine's socket protocol end to end, and the facts of the ARM binary. What this cannot show is that LG's pipeline agrees: the TV steps in
docs/research/gfn-app-result.md measure that.
"""
import array
import ctypes
import ctypes.util
import json
import math
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
DECODERS = os.path.join(FEEDER, "build", "host", "lib")
YUV_TEST = os.path.join(FEEDER, "build", "host", "yuv_test")

FRAME = struct.Struct("<BBHI")
LOAD = struct.Struct("<8I15Iii")
MEDIA_HEAD = struct.Struct("<qB7x")
ACCEPT = struct.Struct("<qii")
DROP = struct.Struct("<qii")
EVENT = struct.Struct("<iiq")
STATUS = struct.Struct("<5Q4I2I2II2I2III")
HELLO = struct.Struct("<4I32s64s")
GF = dict(LOAD=1, VIDEO=2, AUDIO=3, FLUSH=4, UNLOAD=5, STATE=6, HDR=7, PING=8, APP=9, TEXTURE=10, LOADED=16, EVENT=17, ACCEPT=18, DROP=19, STATUS=20, ERROR=21, PONG=22, HELLO=23)


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


def libopus():
    """The host's libopus through ctypes: the encoder that makes real packets and the reference decoder the feeder's is compared with."""
    name = ctypes.util.find_library("opus")
    if not name:
        return None
    lib = ctypes.CDLL(name)
    lib.opus_encoder_create.restype = ctypes.c_void_p
    lib.opus_encoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
    lib.opus_encode.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_int32]
    lib.opus_decoder_create.restype = ctypes.c_void_p
    lib.opus_decoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
    lib.opus_decode.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int32, ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
    return lib


OPUS = libopus()


def tone_packets(count, freq=440.0, rate=48000, channels=2, frame=960):
    """`count` real 20 ms Opus packets of a sine tone (left at full level, right at half), encoded by libopus."""
    err = ctypes.c_int(0)
    enc = OPUS.opus_encoder_create(rate, channels, 2049, ctypes.byref(err))
    assert err.value == 0
    packets = []
    for k in range(count):
        pcm = array.array("h")
        for i in range(frame):
            v = math.sin(2 * math.pi * freq * (k * frame + i) / rate)
            pcm.append(int(v * 12000))
            if channels == 2:
                pcm.append(int(v * 6000))
        out = ctypes.create_string_buffer(1500)
        n = OPUS.opus_encode(enc, pcm.tobytes(), frame, out, 1500)
        assert n > 0, n
        packets.append(out.raw[:n])
    return packets


def reference_pcm(packets, rate=48000, channels=2):
    """What libopus itself decodes the packets to, as S16 samples."""
    err = ctypes.c_int(0)
    dec = OPUS.opus_decoder_create(rate, channels, ctypes.byref(err))
    out = array.array("h")
    for p in packets:
        buf = ctypes.create_string_buffer(5760 * channels * 2)
        n = OPUS.opus_decode(dec, p, len(p), buf, 5760, 0)
        assert n > 0, n
        out.frombytes(buf.raw[:n * channels * 2])
    return out


def snr_db(reference, got):
    """Signal to error ratio of two S16 sample arrays of the same length."""
    assert len(reference) == len(got), (len(reference), len(got))
    sig = sum(float(a) * a for a in reference)
    err = sum(float(a - b) ** 2 for a, b in zip(reference, got))
    return 200.0 if err == 0 else 10 * math.log10(sig / err)


def write_packets(path, packets):
    with open(path, "wb") as f:
        for p in packets:
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

    def test_a_video_window_is_the_compositors_own_kind_and_draws_nothing(self):
        """`-w video`: the window type of LumaTV's own video windows (the compositor's `vwin`), without the saver's playVideo flag."""
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "-w", "video"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            self.assertEqual(r.fc.props["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_LUMATV_VIDEO")
            self.assertNotIn("playVideo", r.fc.props)
            self.assertIn("wl_webos_foreign.export_element", r.fc.names())
            self.assertIn("wl_webos_exported.set_exported_window", r.fc.names())
        finally:
            r.close()

    def test_the_decoder_can_be_opened_for_an_app_other_than_the_window_s_own(self):
        """`--ndl-app`: LG's video path connects a video to its sink only for an app on SAM's foreground list, so a video window that is no app of
        its own plays for the app in front; the window keeps its own id (the compositor binds a claim to it by that id)."""
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "-w", "video", "-a", "org.test.cameras", "--ndl-app", "com.webos.app.home"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            log = " ".join(r.ndl())
            self.assertIn("Init com.webos.app.home", log)
            self.assertNotIn("Init org.test.cameras", log)
            self.assertEqual(r.fc.props["appId"], "org.test.cameras")
        finally:
            r.close()

    def test_without_it_the_decoder_is_opened_for_the_window_s_own_app(self):
        r = Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "-w", "video", "-a", "org.test.cameras"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            self.assertIn("Init org.test.cameras", " ".join(r.ndl()))
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

    def test_the_engine_names_the_app_the_decoder_is_opened_for_before_a_load_and_may_take_it_back(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-app-")
        sock = os.path.join(tmp, "feed.sock")
        ndl_log = os.path.join(tmp, "ndl.log")
        fc = FakeCompositor()
        env = dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=ndl_log, FAKE_NDL_VERBOSE="1")
        err = open(os.path.join(tmp, "feeder.log"), "w")
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "-L", NDL, "-w", "video", "-a", "org.test.cameras", "--stats", os.path.join(tmp, "stats.jsonl")],
                                env=env, stderr=err, stdout=subprocess.DEVNULL)
        try:
            c = Client(sock)
            self.assertEqual(c.recv()[0], GF["HELLO"])
            c.send(GF["APP"], b"com.webos.app.home")
            c.load(audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
            c.send(GF["UNLOAD"])
            c.send(GF["APP"], b"")
            c.load(audio=0, hdr=0, drop=-1)
            c.until(GF["LOADED"])
            c.send(GF["UNLOAD"])
            time.sleep(0.3)
            c.close()
            proc.wait(timeout=10)
            inits = [l.split("Init ", 1)[1].strip() for l in open(ndl_log) if " Init " in l]
            self.assertEqual(inits, ["com.webos.app.home", "org.test.cameras"], "the first load for the app named, the second for the window's own after an empty name")
            self.assertIn("ndl app com.webos.app.home", open(os.path.join(tmp, "feeder.log")).read())
        finally:
            if proc.poll() is None:
                proc.kill()
            err.close()
            fc.close()
            shutil.rmtree(tmp, ignore_errors=True)

    def test_started_by_sam_the_feeder_starts_the_ui_takes_its_window_id_and_ends_with_it(self):
        """The installed app: a stand-in UI (a shell script in the app directory's bin64/) writes the window-id file, a load reads it, and
        the UI's end ends the feeder."""
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-sam-")
        try:
            home = os.path.join(tmp, "org.test.gfn")
            os.makedirs(os.path.join(home, "bin64"))
            os.makedirs(os.path.join(home, "gfx64"))
            exe = os.path.join(home, "lumatv-gfn-feeder")
            shutil.copy(BIN, exe)
            idfile = os.path.join(tmp, "window-id")
            sock = os.path.join(tmp, "feed.sock")
            done = os.path.join(tmp, "ui-done")
            with open(os.path.join(home, "gfx64", "launch.sh"), "w") as f:
                f.write("#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n")
            with open(os.path.join(home, "bin64", "lumatv-gfn-app"), "w") as f:
                f.write("#!/bin/sh\necho \"ui: $*\" >&2\nsleep 0.3\necho _Window_Id_31 > %s\nwhile [ ! -e %s ]; do sleep 0.05; done\nexit 0\n" % (idfile, done))
            os.chmod(os.path.join(home, "bin64", "lumatv-gfn-app"), 0o755)
            env = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(tmp, "ndl.log"), LUMATV_GFN_DATA_BASE=tmp)
            proc = subprocess.Popen([exe, "--serve", sock, "--ui-home", home, "--window-id-file", idfile, "-L", NDL], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            c = Client(sock)
            t, p = c.recv()
            self.assertEqual(t, GF["HELLO"])
            self.assertEqual(HELLO.unpack(p)[4].rstrip(b"\0"), b"_Window_Id_pending")
            c.load()
            (t, p), _ = c.until(GF["LOADED"])
            if struct.unpack_from("<i", p)[0] == 0:
                self.assertIn(b"no window id", p)
                time.sleep(0.5)
                c.load()
                (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
            self.assertEqual(p[4:], b"_Window_Id_31")
            self.assertIn("SetWindowId _Window_Id_31", open(os.path.join(tmp, "ndl.log")).read())
            open(done, "w").close()
            proc.wait(timeout=10)
            self.assertEqual(proc.returncode, 0)
            err = proc.stderr.read()
            self.assertIn("ui: --app-id org.test.gfn", err)
            self.assertIn("the UI ended", err)
        finally:
            if proc.poll() is None:
                proc.kill()
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


@unittest.skipUnless(HAVE and OPUS, "zig (the host build) and the host's libopus (the encoder and the reference decoder) are needed")
class PcmFallback(unittest.TestCase):
    """NDL refusing the Opus load: the feeder decodes the packets itself (libopus 1.5.2, fixed point, build/opus) and loads PCM."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gfn-feeder-pcm-")
        self.clip = os.path.join(self.tmp, "clip.hevc")
        with open(self.clip, "wb") as f:
            f.write(hevc_stream(120))
        self.packets = tone_packets(50)
        self.pk = os.path.join(self.tmp, "tone.opuspkts")
        write_packets(self.pk, self.packets)
        self.dump = os.path.join(self.tmp, "audio.pcm")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_play(self, extra=(), fail=None):
        env = {"FAKE_NDL_AUDIO_DUMP": self.dump}
        if fail:
            env["FAKE_NDL_FAIL"] = fail
        return Run(["--play", self.clip, "--size", "320x240", "--fps", "60", "--audio", self.pk, *extra], env=env).go()

    def played_pcm(self):
        pcm = array.array("h")
        with open(self.dump, "rb") as f:
            pcm.frombytes(f.read())
        return pcm

    def test_a_refused_opus_load_becomes_pcm_decoded_here_and_matches_libopus(self):
        r = self.run_play(fail="opus")
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            log = r.ndl()
            loads = [l for l in log if " Load " in l]
            self.assertEqual(len(loads), 2, loads)
            self.assertIn("audio OPUS 2 ch 48.0 kHz", loads[0])
            self.assertIn("audio PCM S16LE interleaved stereo rate 1", loads[1])
            self.assertIn("refused the Opus load", r.err())
            plays = [int(l.split(" size ")[1].split()[0]) for l in log if "AudioPlay #" in l]
            self.assertTrue(plays and all(n == 960 * 2 * 2 for n in plays), plays[:5])
            s = r.summary()
            self.assertEqual((s["afed"], s["adropped"], s["adecode_errors"], s["audio_route"]), (50, 0, 0, "pcm"))
            self.assertNotIn("VIOLATION", "\n".join(log))
            got = self.played_pcm()
            self.assertEqual(len(got), 50 * 960 * 2)
            snr = snr_db(reference_pcm(self.packets), got)
            self.assertGreater(snr, 40.0, "the fixed-point decode differs from libopus by more than rounding: %.1f dB" % snr)
            self.assertGreater(max(abs(v) for v in got), 8000, "the tone is there")
        finally:
            r.close()

    def test_pcm_mode_decodes_without_trying_opus_and_opus_mode_does_not_fall_back(self):
        r = self.run_play(extra=["--audio-mode", "pcm"])
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            loads = [l for l in r.ndl() if " Load " in l]
            self.assertEqual(len(loads), 1)
            self.assertIn("audio PCM S16LE", loads[0])
            self.assertEqual(r.summary()["audio_route"], "pcm")
            self.assertEqual(len(self.played_pcm()), 50 * 960 * 2)
        finally:
            r.close()
        os.remove(self.dump)
        r = self.run_play(extra=["--audio-mode", "opus"], fail="opus")
        try:
            self.assertEqual(r.proc.returncode, 4)
            self.assertIn("audio type 3 (Opus) is not supported", r.err())
            self.assertFalse(os.path.exists(self.dump))
        finally:
            r.close()

    def test_auto_keeps_opus_when_the_library_takes_it(self):
        r = self.run_play()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            loads = [l for l in r.ndl() if " Load " in l]
            self.assertEqual(len(loads), 1)
            self.assertIn("audio OPUS", loads[0])
            self.assertEqual(r.summary()["audio_route"], "opus")
            self.assertNotIn("refused the Opus load", r.err())
        finally:
            r.close()

    @unittest.skipUnless(shutil.which("qemu-arm"), "qemu-arm runs the ARM build of the decoder")
    def test_the_arm_build_of_the_decoder_gives_the_hosts_samples(self):
        """The decoder compiled for 32-bit ARM (the archive the set's feeder links) and run under qemu-arm, against libopus on the host."""
        src = os.path.join(FEEDER, "build", "opus", "src", "include")
        exe = os.path.join(self.tmp, "opus_decode")
        build = subprocess.run(["zig", "cc", "-target", "arm-linux-musleabi", "-O2", "-static", "-I" + os.path.join(FEEDER, "src"), "-I" + src,
                                os.path.join(FEEDER, "tests", "opus_decode.c"), os.path.join(FEEDER, "src", "gf_opus.c"),
                                os.path.join(FEEDER, "build", "opus", "arm", "libopus-dec.a"), "-o", exe], capture_output=True, text=True)
        self.assertEqual(build.returncode, 0, build.stderr)
        run = subprocess.run(["qemu-arm", exe, self.pk], capture_output=True, timeout=60)
        self.assertEqual(run.returncode, 0, run.stderr)
        got = array.array("h")
        got.frombytes(run.stdout)
        self.assertGreater(snr_db(reference_pcm(self.packets), got), 40.0)

    def test_a_packet_that_is_not_opus_is_counted_not_fed(self):
        bad = [self.packets[0], b"\xff" * 5, self.packets[1]]
        write_packets(self.pk, bad)
        r = self.run_play(extra=["--audio-mode", "pcm"])
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            s = r.summary()
            self.assertEqual((s["afed"], s["adropped"], s["adecode_errors"]), (2, 1, 1))
            self.assertIn("did not decode", r.err())
        finally:
            r.close()

    def test_the_engine_socket_gets_the_fallback_and_later_loads_go_straight_to_pcm(self):
        sock = os.path.join(self.tmp, "feed.sock")
        env = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(self.tmp, "ndl.log"), FAKE_NDL_FAIL="opus",
                   FAKE_NDL_AUDIO_DUMP=self.dump)
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "--window-id", "_Window_Id_7", "-L", NDL], env=env, stderr=subprocess.PIPE, stdout=subprocess.DEVNULL, text=True)
        try:
            c = Client(sock)
            self.assertEqual(c.recv()[0], GF["HELLO"])
            for round_ in range(2):
                c.load()
                (t, p), _ = c.until(GF["LOADED"])
                self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
                for i, pk in enumerate(self.packets[:10]):
                    c.send(GF["AUDIO"], MEDIA_HEAD.pack(1_000_000_000 + (round_ * 100 + i) * 20_000_000, 0) + pk)
                c.send(GF["PING"], struct.pack("<q", round_))
                c.until(GF["PONG"])
                c.send(GF["UNLOAD"])
                time.sleep(0.1)
            c.close()
            proc.wait(timeout=10)
            self.assertEqual(proc.returncode, 0)
            log = open(os.path.join(self.tmp, "ndl.log")).read()
            self.assertEqual(log.count("audio OPUS"), 1, "the second load does not ask for Opus again")
            self.assertEqual(log.count("audio PCM S16LE"), 2)
            self.assertNotIn("VIOLATION", log)
            self.assertEqual(len(self.played_pcm()), 20 * 960 * 2)
            self.assertEqual(proc.stderr.read().count("refused the Opus load"), 1)
        finally:
            if proc.poll() is None:
                proc.kill()


@unittest.skipUnless(HAVE, "zig is needed to build the feeder")
class Binary(unittest.TestCase):
    def test_arm_binary_is_32_bit_soft_float_with_libc_and_libdl_only(self):
        out = subprocess.run(["readelf", "-h", "-d", ARM], capture_output=True, text=True).stdout
        self.assertIn("ELF32", out)
        self.assertIn("Machine:                           ARM", out)
        needed = [l.split("[")[1].rstrip("]") for l in out.splitlines() if "NEEDED" in l]
        self.assertEqual(sorted(needed), ["libc.so.6", "libdl.so.2", "libpthread.so.0"])
        self.assertIn("Flags:                             0x5000200", out)

    def test_the_app_id_comes_from_the_installed_directory_and_no_arguments_means_serve(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-app-")
        try:
            appdir = os.path.join(tmp, "org.test.gfn")
            os.makedirs(appdir)
            exe = os.path.join(appdir, "lumatv-gfn-feeder")
            shutil.copy(BIN, exe)
            env = dict(os.environ, FAKE_NDL_LOG=os.devnull)
            env.pop("APPID", None)
            r = subprocess.run([exe, "--probe", "-L", NDL], capture_output=True, text=True, env=env)
            self.assertEqual(json.loads(r.stdout)["app_id"], "org.test.gfn")
            r = subprocess.run([exe, "--probe", "-L", NDL], capture_output=True, text=True, env=dict(env, APPID="com.other.app"))
            self.assertEqual(json.loads(r.stdout)["app_id"], "com.other.app")
            # SAM starts `main` with no arguments: the feeder serves the engine on the default socket from its own window
            fc = FakeCompositor()
            try:
                e = dict(env, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), LUMATV_GFN_LOG="-")
                p = subprocess.Popen([exe, "-L", NDL], env=e, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
                line = p.stdout.readline()
                self.assertIn('"serving":"/tmp/lumatv-gfn/feed.sock"', line)
                self.assertIn('"window":"_Window_Id_', line)
                p.terminate()
                p.wait(timeout=10)
            finally:
                fc.close()
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

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


SAM_APP = "org.test.sam"
HUB = os.path.join(os.path.dirname(NDL), "libluna-service2.so.3")
REGISTERED = '{"event":"registered","returnValue":true}'
RELAUNCH = '{"event":"relaunch","reason":"undefined","appId":"%s","parameters":{"launchType":"homeKey"},"returnValue":true}' % SAM_APP
CLOSE = '{"event":"close","reason":"memoryReclaim","returnValue":true}'


def launch_object(reason="undefined", params=None):
    return json.dumps({"@system_native_app": True, "reason": reason, "interfaceVersion": 2, "event": "launch",
                       "interfaceMethod": "registerApp", "appId": SAM_APP, "parameters": params or {}})


@unittest.skipUnless(HAVE and os.path.exists(HUB), "needs the feeder build and the stand-in hub (zig, the host's libglib-2.0.so.0)")
class Sam(unittest.TestCase):
    """The app's life with SAM (gf_sam.c): the launch argument, the registration with the hub, relaunch and close handed to the UI on its
    descriptor 3 (rustui/app/src/lifecycle.rs), the UI's end ending the app. The hub is the card launcher's stand-in (fakels2.c) playing
    a script; whether the set's hub accepts the registration from this executable is the TV's to say."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gfn-feeder-sam-")
        self.home = os.path.join(self.tmp, SAM_APP)
        os.makedirs(os.path.join(self.home, "bin64"))
        os.makedirs(os.path.join(self.home, "gfx64"))
        self.exe = os.path.join(self.home, "lumatv-gfn-feeder")
        shutil.copy(BIN, self.exe)
        os.makedirs(os.path.join(self.tmp, "data-base"))
        self.ui_log = os.path.join(self.tmp, "ui.log")
        self.hub_log = os.path.join(self.tmp, "hub.log")
        self.script = os.path.join(self.tmp, "script")
        with open(os.path.join(self.home, "gfx64", "launch.sh"), "w") as f:
            f.write("#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def ui(self, body):
        """The stand-in UI: logs its arguments, then `body`; descriptor 3 is the channel."""
        path = os.path.join(self.home, "bin64", "lumatv-gfn-app")
        with open(path, "w") as f:
            f.write("#!/bin/sh\necho \"args: $*\" >> %s\n%s\n" % (self.ui_log, body))
        os.chmod(path, 0o755)

    READ_UNTIL_CLOSE = 'while read -r line <&3; do echo "got: $line" >> %s; case "$line" in *\\"close\\"*) exit 0;; esac; done'

    def start(self, events=(), launch=None, env=None, extra=()):
        with open(self.script, "w") as f:
            for ms, payload in events:
                f.write("%d %s\n" % (ms, payload))
        e = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.devnull, FAKE_LS_LOG=self.hub_log, FAKE_SAM_EVENTS=self.script, LUMATV_GFN_LOG="-",
                 LUMATV_GFN_DATA_BASE=os.path.join(self.tmp, "data-base"))
        for k in ("APPID", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"):
            e.pop(k, None)
        e.update(env or {})
        args = [self.exe, "--serve", os.path.join(self.tmp, "feed.sock"), "--ui-home", self.home, "--window-id-file", os.path.join(self.tmp, "window-id"),
                "-L", NDL, *extra]
        if launch is not None:
            args.append(launch)
        return subprocess.Popen(args, env=e, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)

    def text(self, path):
        try:
            with open(path) as f:
                return f.read()
        except FileNotFoundError:
            return ""

    def finish(self, proc, timeout=15):
        try:
            _, err = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, err = proc.communicate()
            self.fail("the feeder did not end\n" + err + self.text(self.hub_log) + self.text(self.ui_log))
        return err

    def test_registers_under_the_launch_objects_id_and_hands_registered_relaunch_and_close_to_the_ui(self):
        self.ui(self.READ_UNTIL_CLOSE % self.ui_log)
        proc = self.start(events=[(50, REGISTERED), (300, RELAUNCH), (700, CLOSE)], launch=launch_object())
        err = self.finish(proc)
        self.assertEqual(proc.returncode, 0, err)
        hub = self.text(self.hub_log)
        self.assertIn("register %s" % SAM_APP, hub)
        self.assertIn('call luna://com.webos.applicationManager/registerApp {"appId":"%s","subscribe":true}' % SAM_APP, hub)
        self.assertIn("cancel", hub)
        self.assertIn("unregister", hub)
        ui = self.text(self.ui_log).splitlines()
        self.assertIn("--channel-fd 3", ui[0])
        self.assertNotIn("--hidden", ui[0])
        self.assertIn("--app-id %s" % SAM_APP, ui[0])
        got = [json.loads(l[5:]) for l in ui[1:] if l.startswith("got: ")]
        self.assertEqual(got, [{"kind": "sam", "registered": True},
                               {"kind": "relaunch", "reason": "undefined", "parameters": {"launchType": "homeKey"}},
                               {"kind": "close", "reason": "memoryReclaim"}])
        self.assertIn("the UI ended", err)

    def test_a_preload_starts_the_ui_hidden(self):
        self.ui(self.READ_UNTIL_CLOSE % self.ui_log)
        for launch in (launch_object("preload"), launch_object(params={"snapshot": True})):
            self.finish(self.start(events=[(300, CLOSE)], launch=launch))
        started = [l for l in self.text(self.ui_log).splitlines() if l.startswith("args: ")]
        self.assertEqual(len(started), 2)
        for l in started:
            self.assertIn("--hidden", l)

    def test_a_name_the_hub_refuses_is_told_to_the_ui_and_the_app_goes_on(self):
        self.ui('echo "got: $(head -n 1 <&3)" >> %s; sleep 30' % self.ui_log)
        proc = self.start(env={"FAKE_LS_REFUSE": "1"}, launch=launch_object())
        end = time.time() + 10
        while time.time() < end and '"registered":false' not in self.text(self.ui_log):
            time.sleep(0.05)
        self.assertIn('got: {"kind":"sam","registered":false}', self.text(self.ui_log))
        self.assertIsNone(proc.poll())
        proc.terminate()
        err = self.finish(proc)
        self.assertIn("LSRegister", err)

    def test_a_ui_that_ignores_close_is_ended_after_the_grace_and_the_feeder_follows(self):
        self.ui("trap '' TERM; sleep 60 <&3 & wait")
        proc = self.start(events=[(100, REGISTERED), (200, CLOSE)], launch=launch_object())
        began = time.time()
        err = self.finish(proc, timeout=20)
        self.assertGreater(time.time() - began, 3.5, err)
        self.assertLess(time.time() - began, 12, err)
        self.assertIn("did not end in time after SAM's close", err)

    def test_without_the_hubs_libraries_the_app_runs_without_sam(self):
        self.ui('sleep 30 <&3')
        proc = self.start(env={"LD_LIBRARY_PATH": ""}, launch=launch_object())
        time.sleep(1.0)
        self.assertIsNone(proc.poll())
        proc.terminate()
        err = self.finish(proc)
        self.assertIn("without SAM's events", err)

    def test_started_by_sam_with_no_environment_the_ui_gets_the_compositors_socket_and_its_neighbours_by_path_and_the_log_is_a_file(self):
        """SAM starts `main` with an empty environment (TV round 11: the UI could not find the compositor and the app was over in 200 ms)."""
        log = os.path.join(self.tmp, "feeder.log")
        self.ui('echo "env: $XDG_RUNTIME_DIR $WAYLAND_DISPLAY" >> %s; echo "stdout line"; echo "stderr line" >&2; %s' % (self.ui_log, self.READ_UNTIL_CLOSE % self.ui_log))
        base = os.path.join(self.tmp, "data-base")
        proc = self.start(events=[(300, CLOSE)], launch=launch_object(), env={"LUMATV_GFN_LOG": log})
        self.finish(proc)
        ui = self.text(self.ui_log)
        self.assertIn("env: /tmp/xdg wayland-0", ui)
        self.assertIn("--core %s/run64.sh --engine %s/bin64/opennow-streamer" % (self.home, self.home), ui)
        self.assertIn("--data %s/%s" % (base, SAM_APP), ui)
        self.assertTrue(os.path.isdir(os.path.join(base, SAM_APP)))
        said = self.text(log)
        self.assertIn("launched by SAM: reason undefined", said)
        self.assertIn("data directory %s/%s: writable" % (base, SAM_APP), said)
        self.assertIn("stdout line", said)
        self.assertIn("stderr line", said)
        self.assertIn("the UI ended", said)

    def test_a_json_argument_that_is_not_a_launch_object_is_refused(self):
        self.ui("exit 0")
        proc = self.start(launch="{}")
        err = self.finish(proc)
        self.assertEqual(proc.returncode, 2, err)
        self.assertIn("not a launch object", err)


def encoder(codec):
    """ffmpeg's encoder for the codec when this machine has ffmpeg with it (the texture tests make their clips with it)."""
    if not shutil.which("ffmpeg"):
        return None
    name = {"h264": "libx264", "h265": "libx265"}[codec]
    out = subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True).stdout
    return name if name in out else None


def make_clip(path, codec="h264", size="1280x720", rate=30, seconds=3, gop=30):
    """A test-pattern clip as Annex B with an access unit delimiter before every picture (the tests split on them), no B pictures."""
    enc = encoder(codec)
    params = {"h264": ["-x264-params", "aud=1:bframes=0"], "h265": ["-x265-params", "aud=1:bframes=0:log-level=none"]}[codec]
    cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=%s:rate=%d" % (size, rate), "-t", str(seconds), "-c:v", enc,
           "-preset", "veryfast", "-g", str(gop), "-pix_fmt", "yuv420p", *params, "-f", "h264" if codec == "h264" else "hevc", path]
    subprocess.run(cmd, check=True)


def split_units(data, codec="h264"):
    """The access units of an Annex B stream, cut at its delimiters."""
    marker = b"\x00\x00\x00\x01\x09" if codec == "h264" else b"\x00\x00\x00\x01\x46\x01"
    cuts = []
    pos = data.find(marker)
    while pos >= 0:
        cuts.append(pos)
        pos = data.find(marker, pos + 1)
    return [data[a:b] for a, b in zip(cuts, cuts[1:] + [len(data)])]


def reference_frame(clip, index, size):
    """Frame `index` of the clip as FFmpeg decodes it and scales it with its area filter: BGRA bytes at `size` (WxH)."""
    w, h = size
    out = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", clip, "-vf", "select=eq(n\\,%d),scale=%d:%d:flags=area" % (index, w, h), "-frames:v", "1",
                          "-f", "rawvideo", "-pix_fmt", "bgra", "-"], capture_output=True, check=True).stdout
    return out


def psnr(a, b, step=7):
    """PSNR in dB of two BGRA pictures over their colour bytes, every `step`th pixel."""
    total = n = 0
    for i in range(0, len(a), 4 * step):
        total += (a[i] - b[i]) ** 2 + (a[i + 1] - b[i + 1]) ** 2 + (a[i + 2] - b[i + 2]) ** 2
        n += 3
    return 10 * math.log10(255 * 255 / (total / n + 1e-9))


TEXTURE_READY = HAVE and os.path.isdir(DECODERS) and any(f.startswith("libavcodec-lumatv.so.") for f in os.listdir(DECODERS)) and bool(encoder("h264"))


@unittest.skipUnless(HAVE and os.path.exists(YUV_TEST), "zig is needed to build the feeder for the host")
class Yuv(unittest.TestCase):
    def test_the_converter_is_right_and_says_what_it_costs(self):
        r = subprocess.run([YUV_TEST], capture_output=True, text=True, timeout=120)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("yuv_test: ok", r.stdout)


@unittest.skipUnless(TEXTURE_READY, "the texture path needs the host build of FFmpeg's decoders (gfn/feeder/build.sh) and ffmpeg with libx264")
class Texture(unittest.TestCase):
    """A camera's picture as a texture: the feeder decodes it and paints its window's buffers at the claim's size (docs/research/multiview-result.md)."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="gfn-feeder-texture-")
        cls.clip = os.path.join(cls.tmp, "t720.h264")
        make_clip(cls.clip, "h264", "1280x720", 30, 3, 30)
        cls.small = os.path.join(cls.tmp, "t360.h264")
        make_clip(cls.small, "h264", "640x360", 15, 3, 15)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def frames(self, fc, size):
        return [c for c in fc.commits if c[2] is not None and (c[2][0], c[2][1]) == size]

    def test_a_played_file_is_painted_into_the_window_at_the_asked_size_and_matches_ffmpeg(self):
        r = Run(["--play", self.clip, "--codec", "h264", "--size", "1280x720", "--fps", "4", "-w", "video", "-a", "org.test.cam", "--no-ndl", "--texture", "480x270",
                 "--swdec-lib", DECODERS, "--seconds", "3"]).go()
        # (the clip is 30 a second; it is played at 4 so that a loaded machine keeps up: what the rate costs is tests/swdec_bench.c's and the TV check's)
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            self.assertEqual(r.fc.props["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_LUMATV_VIDEO")
            self.assertEqual(r.fc.violations, [], "the buffers are the compositor's until it releases them")
            pictures = self.frames(r.fc, (480, 270))
            tex = [l["tex"] for l in r.stats_lines() if "tex" in l]
            self.assertTrue(tex, "one stats line a second")
            self.assertGreaterEqual(r.summary()["fed"], 10)
            shown = len(pictures) + tex[-1]["skipped"]
            self.assertTrue(r.summary()["fed"] - 2 <= shown <= r.summary()["fed"], "every unit but the decoder's first is a picture on the window or a counted skip: %d of %d" % (shown, r.summary()["fed"]))
            self.assertEqual((tex[-1]["src"], tex[-1]["out"], tex[-1]["errors"], tex[-1]["skipped"]), ("1280x720", "480x270", 0, 0))
            self.assertGreater(max(l["decode_us_avg"] for l in tex), 0)
            self.assertIn("cpu_pct", tex[-1])
            # the 8th picture against FFmpeg's decode of the 8th unit, scaled by FFmpeg's area filter
            got = pictures[8][2][2]
            ref = reference_frame(self.clip, 8, (480, 270))
            self.assertEqual(len(got), len(ref))
            psnr_db = psnr(got, ref)
            self.assertGreater(psnr_db, 22.0, "the picture is FFmpeg's, scaled by this feeder (PSNR %.1f dB)" % psnr_db)
        finally:
            r.close()

    def test_a_texture_session_over_the_engine_socket(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-texture-serve-")
        sock = os.path.join(tmp, "feed.sock")
        fc = FakeCompositor()
        env = dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(tmp, "ndl.log"))
        err = open(os.path.join(tmp, "feeder.log"), "w")
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "-w", "video", "-a", "org.test.cam", "-L", NDL, "--swdec-lib", DECODERS, "--stats", os.path.join(tmp, "stats.jsonl")],
                                env=env, stderr=err, stdout=subprocess.DEVNULL)
        c = None
        try:
            c = Client(sock)
            t, p = c.recv()
            self.assertEqual(t, GF["HELLO"])
            units = split_units(open(self.small, "rb").read())
            self.assertGreaterEqual(len(units), 40)
            c.send(GF["TEXTURE"], struct.pack("<II", 480, 270))
            c.load(codec=1, w=640, h=360, fps=15, audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
            t0 = 1_000_000_000
            seen = {}

            def send_paced(first, count):
                """Units at a camera's pace (a burst would find both buffers with the compositor, and the feeder skips what it cannot show)."""
                got = 0
                for i, u in enumerate(units[first:first + count]):
                    c.send(GF["VIDEO"], MEDIA_HEAD.pack(t0 + (first + i) * 66_666_667, 1 if first + i == 0 else 0) + u)
                    time.sleep(0.12)
                    c.s.settimeout(0.01)
                    try:
                        while True:
                            t, p = c.recv()
                            seen[t] = seen.get(t, 0) + 1
                            got += t == GF["ACCEPT"]
                    except socket.timeout:
                        pass
                    c.buf = c.buf if len(c.buf) < FRAME.size else c.buf
                c.s.settimeout(10)
                return got

            self.assertEqual(send_paced(0, 20) + 0, seen.get(GF["ACCEPT"], 0))
            self.assertEqual(seen.get(GF["ACCEPT"], 0), 20, seen)
            self.assertEqual(seen.get(GF["DROP"], 0), 0, seen)
            fc.settle()
            self.assertGreaterEqual(len(self.frames(fc, (480, 270))), 10)  # the feeder skips a picture while both buffers are with a slow compositor: a loaded host
            # the claim moved and changed size: the buffers follow, with no new load
            c.send(GF["TEXTURE"], struct.pack("<II", 320, 180))
            send_paced(20, 10)
            fc.settle()
            self.assertGreaterEqual(len(self.frames(fc, (320, 180))), 5)
            # a unit that is not a picture is dropped, not fatal, and the next key picture decodes again
            c.send(GF["VIDEO"], MEDIA_HEAD.pack(t0 + 40 * 66_666_667, 0) + bytes(range(256)) * 4)
            (t, p), _ = c.until(GF["DROP"])
            c.send(GF["VIDEO"], MEDIA_HEAD.pack(t0 + 45 * 66_666_667, 1) + units[30])
            self.assertEqual(c.until(GF["ACCEPT"])[0][0], GF["ACCEPT"])
            # the end of the session leaves the window as it was made: 16 x 16 and nothing in it
            before = len(fc.commits)
            c.send(GF["UNLOAD"])
            deadline = time.time() + 5
            while time.time() < deadline and not [x for x in fc.commits[before:] if x[2] is not None and (x[2][0], x[2][1]) == (16, 16)]:
                time.sleep(0.05)
            quiet = [x for x in fc.commits[before:] if x[2] is not None and (x[2][0], x[2][1]) == (16, 16)]
            self.assertTrue(quiet, "the window is a video window again")
            self.assertEqual(set(quiet[-1][2][2]), {0})
            self.assertEqual(fc.violations, [])
        finally:
            if c:
                c.close()
            proc.terminate()
            proc.wait(5)
            fc.close()
            shutil.rmtree(tmp, ignore_errors=True)

    def test_without_the_decoder_libraries_the_load_is_refused_with_the_reason(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-texture-nolib-")
        sock = os.path.join(tmp, "feed.sock")
        fc = FakeCompositor()
        env = dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(tmp, "ndl.log"))
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "-w", "video", "-a", "org.test.cam", "-L", NDL, "--swdec-lib", os.path.join(tmp, "none")], env=env,
                                stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
        try:
            c = Client(sock)
            c.recv()
            c.send(GF["TEXTURE"], struct.pack("<II", 480, 270))
            c.load(codec=1, w=640, h=360, fps=15, audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 0)
            self.assertIn("not installed", p[4:].decode())
            # and the same feeder still plays through NDL: texture mode off, a load goes to the library
            c.send(GF["TEXTURE"], struct.pack("<II", 0, 0))
            c.load(codec=2, w=320, h=240, fps=60, audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 1, p)
        finally:
            proc.terminate()
            proc.wait(5)
            fc.close()
            shutil.rmtree(tmp, ignore_errors=True)

    def test_a_stream_the_cpu_cannot_carry_is_refused_at_the_load_not_decoded_slowly(self):
        tmp = tempfile.mkdtemp(prefix="gfn-feeder-texture-big-")
        sock = os.path.join(tmp, "feed.sock")
        fc = FakeCompositor()
        env = dict(os.environ, **fc.env(), LD_LIBRARY_PATH=os.path.dirname(NDL), FAKE_NDL_LOG=os.path.join(tmp, "ndl.log"))
        proc = subprocess.Popen([BIN, "--serve", sock, "--once", "-w", "video", "-a", "org.test.cam", "-L", NDL, "--swdec-lib", DECODERS], env=env, stderr=subprocess.DEVNULL,
                                stdout=subprocess.DEVNULL)
        c = None
        try:
            c = Client(sock)
            c.recv()
            c.send(GF["TEXTURE"], struct.pack("<II", 640, 360))
            c.load(codec=1, w=3840, h=2160, fps=30, audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 0)
            self.assertIn("more than it takes", p[4:].decode())
            c.load(codec=1, w=1280, h=720, fps=120, audio=0, hdr=0, drop=-1)
            (t, p), _ = c.until(GF["LOADED"])
            self.assertEqual(struct.unpack_from("<i", p)[0], 0, "120 a second is the window's")
        finally:
            if c:
                c.close()
            proc.terminate()
            proc.wait(5)
            fc.close()
            shutil.rmtree(tmp, ignore_errors=True)

    @unittest.skipUnless(TEXTURE_READY and encoder("h265"), "ffmpeg with libx265")
    def test_h265_is_decoded_too(self):
        clip = os.path.join(self.tmp, "t360.hevc")
        make_clip(clip, "h265", "640x360", 15, 2, 15)
        r = Run(["--play", clip, "--codec", "h265", "--size", "640x360", "--fps", "5", "-w", "video", "-a", "org.test.cam", "--no-ndl", "--texture", "320x180",
                 "--swdec-lib", DECODERS, "--seconds", "3"]).go()
        try:
            self.assertEqual(r.proc.returncode, 0, r.err())
            self.assertGreaterEqual(len(self.frames(r.fc, (320, 180))), 10)
        finally:
            r.close()



if __name__ == "__main__":
    unittest.main()
