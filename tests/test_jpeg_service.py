"""Run with: python3 -m unittest tests.test_jpeg_service   (needs `jpeg/build.sh` for the host build)

lumatv-jpeg (jpeg/service.c) on the host against the fake libdile_photo (jpeg/fake_dilephoto.c), which "decodes" every JPEG to
a synthetic picture in the layout of LG's contract (jpeg/dilejpeg.h). The picture is a fixed function of the position, so
the expected pixels are recomputed here independently. What matters: every chroma layout the block can write comes out as the
right NV12 and RGBA at every scale, odd sizes and padded or floored hardware sizes are cropped, what the block does not take is
refused before the library is called, a failing or surprising library is a status and never a crash, and the device is held
only while images are decoded.
"""
import io
import os
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import unittest

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
BUILD = os.path.join(os.path.dirname(HERE), "jpeg", "build", "host")
BIN = os.path.join(BUILD, "lumatv-jpeg")

REQ, RSP = 0x3151504A, 0x3152504A
NV12, RGBA, BGRA = 1, 2, 3
OK, E_REQUEST, E_UNSUPPORTED, E_NO_LIBRARY, E_OPEN, E_DECODE, E_OUTPUT, E_TOO_BIG = range(8)
LIMITED, SWAP_UV, HW_ONLY, CPU_ONLY = 1, 2, 4, 8
RFLAG_CPU = 1


def jpeg(w, h, sub="420", gray=False, progressive=False):
    img = Image.new("L" if gray else "RGB", (w, h), 128)
    buf = io.BytesIO()
    kw = {} if gray else {"subsampling": {"444": 0, "422": 1, "420": 2}[sub]}
    img.save(buf, "JPEG", progressive=progressive, **kw)
    return buf.getvalue()


def with_sof(data, marker=None, hv=None, precision=None):
    """The same file with its frame header changed (the fake and the service read only the header)."""
    b = bytearray(data)
    i = b.index(b"\xff\xc0") if b"\xff\xc0" in b else b.index(b"\xff\xc2")
    if marker is not None:
        b[i + 1] = marker
    if hv is not None:
        b[i + 11] = hv
    if precision is not None:
        b[i + 4] = precision
    return bytes(b)


def ceil_shift(v, s):
    return (v + (1 << s) - 1) >> s


def clamp(v):
    return 0 if v < 0 else 255 if v > 255 else int(v)


def pattern(w, h, hs, vs, gray, pad=False):
    """The fake library's picture on its own grid: Y over (w, h), chroma over the subsampled grid."""
    y = [[(3 * x + 5 * j + 7) & 255 for x in range(w)] for j in range(h)]
    if gray:
        return y, None, None
    cw, ch = (w + (1 << hs) - 1) >> hs, (h + (1 << vs) - 1) >> vs
    cb = [[(11 * x + 3 * j + 40) & 255 for x in range(cw)] for j in range(ch)]
    cr = [[(5 * x + 13 * j + 200) & 255 for x in range(cw)] for j in range(ch)]
    return y, cb, cr


def rgb_ref(y, cb, cr, studio=False, swap=False):
    if swap:
        cb, cr = cr, cb
    if studio:
        yy = 1.164383 * (y - 16)
        return (clamp(yy + 1.596027 * (cr - 128) + 0.5), clamp(yy - 0.391762 * (cb - 128) - 0.812968 * (cr - 128) + 0.5),
                clamp(yy + 2.017232 * (cb - 128) + 0.5))
    return (clamp(y + 1.402 * (cr - 128) + 0.5), clamp(y - 0.344136 * (cb - 128) - 0.714136 * (cr - 128) + 0.5),
            clamp(y + 1.772 * (cb - 128) + 0.5))


SAMPLING = {"444": (0, 0), "422": (1, 0), "224": (0, 1), "420": (1, 1)}


def sample_file(w, h, kind):
    if kind == "gray":
        return jpeg(w, h, gray=True)
    if kind == "224":
        return with_sof(jpeg(w, h, "420"), hv=0x12)
    return jpeg(w, h, kind)


@unittest.skipUnless(os.path.exists(BIN), "needs jpeg/build.sh (the host build)")
class Service(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="jpeg-test-")
        self.log = os.path.join(self.tmp, "lib.log")
        self.procs, self.socks = [], []
        self.addCleanup(self.cleanup)

    def cleanup(self):
        for s in self.socks:
            s.close()
        for p in self.procs:
            if p.poll() is None:
                p.kill()
            p.wait()
            p.stderr.close()
        for n in os.listdir(self.tmp):
            os.unlink(os.path.join(self.tmp, n))
        os.rmdir(self.tmp)

    def start(self, *args, library=True, **env):
        mine, theirs = socket.socketpair()
        e = dict(os.environ, FAKE_JPEG_LOG=self.log, **env)
        if library:
            e["LD_LIBRARY_PATH"] = BUILD
        else:
            e.pop("LD_LIBRARY_PATH", None)
        p = subprocess.Popen([BIN, "-f", str(theirs.fileno()), *args], env=e, stderr=subprocess.PIPE, text=True, pass_fds=[theirs.fileno()])
        theirs.close()
        mine.settimeout(10)
        self.procs.append(p)
        self.socks.append(mine)
        return p, mine

    @staticmethod
    def request(sock, data, fmt=RGBA, shift=0, flags=0, max_pixels=0, op=1, rid=7, hw_only=True):
        """The hardware tests ask for the hardware alone, so a fallback cannot hide what they check."""
        flags |= HW_ONLY if hw_only else 0
        head = struct.pack("<8I", REQ, rid, op | fmt << 8 | shift << 16 | flags << 24, max_pixels, len(data), 0, 0, 0)
        sock.sendall(head + data)
        h = Service.read(sock, 64)
        words = struct.unpack("<16I", h)
        assert words[0] == RSP and words[1] == rid, words
        r = dict(zip("magic id status width height stride fmt len decode_us convert_us total_us hw_pf hw_w hw_h hw_bytes flags".split(), words))
        r["data"] = Service.read(sock, r["len"])
        return r

    @staticmethod
    def read(sock, n):
        out = b""
        while len(out) < n:
            chunk = sock.recv(n - len(out))
            if not chunk:
                raise EOFError("closed after %d of %d bytes" % (len(out), n))
            out += chunk
        return out

    def lib_calls(self):
        try:
            with open(self.log) as f:
                return f.read().split("\n")[:-1]
        except FileNotFoundError:
            return []

    def check_rgba(self, r, w, h, shift, kind, studio=False, swap=False, bgra=False, tol=1):
        ow, oh = ceil_shift(w, shift), ceil_shift(h, shift)
        self.assertEqual((r["status"], r["width"], r["height"], r["stride"], r["len"]), (OK, ow, oh, ow * 4, ow * oh * 4))
        hs, vs = SAMPLING.get(kind, (0, 0))
        gray = kind == "gray"
        y, cb, cr = pattern(r["hw_w"], r["hw_h"], hs, vs, gray)
        worst = 0
        for j in range(0, oh, max(1, oh // 23)):
            for i in range(0, ow, max(1, ow // 29)):
                px = r["data"][(j * ow + i) * 4:(j * ow + i) * 4 + 4]
                if gray:
                    yy = y[j][i]
                    want = (clamp(1.164383 * (yy - 16) + 0.5),) * 3 if studio else (yy,) * 3
                else:
                    want = rgb_ref(y[j][i], cb[j >> vs][i >> hs], cr[j >> vs][i >> hs], studio, swap)
                if bgra:
                    want = want[::-1]
                self.assertEqual(px[3], 255)
                worst = max(worst, max(abs(a - b) for a, b in zip(px[:3], want)))
        self.assertLessEqual(worst, tol, "pixel error %d at %dx%d shift %d %s" % (worst, w, h, shift, kind))

    def check_nv12(self, r, w, h, shift, kind):
        ow, oh = ceil_shift(w, shift), ceil_shift(h, shift)
        stride = (ow + 1) & ~1
        self.assertEqual((r["status"], r["width"], r["height"], r["stride"]), (OK, ow, oh, stride))
        self.assertEqual(r["len"], stride * (oh + (oh + 1) // 2))
        hs, vs = SAMPLING.get(kind, (0, 0))
        gray = kind == "gray"
        y, cb, cr = pattern(r["hw_w"], r["hw_h"], hs, vs, gray)
        d = r["data"]
        for j in range(oh):
            self.assertEqual(list(d[j * stride:j * stride + ow]), y[j][:ow])
        uv = d[stride * oh:]
        for cy in range((oh + 1) // 2):
            for cx in range((ow + 1) // 2):
                got = (uv[cy * stride + cx * 2], uv[cy * stride + cx * 2 + 1])
                if gray:
                    want = (128, 128)
                else:
                    sw, sh = (ow + (1 << hs) - 1) >> hs, (oh + (1 << vs) - 1) >> vs
                    xs = [x for x in range(cx * (2 >> hs), (cx + 1) * (2 >> hs)) if x < sw]
                    ys = [j for j in range(cy * (2 >> vs), (cy + 1) * (2 >> vs)) if j < sh]
                    n = len(xs) * len(ys)
                    want = tuple((sum(p[j][x] for j in ys for x in xs) + n // 2) // n for p in (cb, cr))
                self.assertEqual(got, want, "chroma at %d,%d" % (cx, cy))

    # -- the layouts --------------------------------------------------------------------------------------------------------

    def test_every_chroma_layout_at_every_scale_gives_the_right_rgba_and_nv12(self):
        for semi in ("0", "1"):
            _, s = self.start(FAKE_JPEG_SEMI=semi)
            for kind in ("444", "422", "224", "420", "gray"):
                for shift in range(4):
                    with self.subTest(semi=semi, kind=kind, shift=shift):
                        r = self.request(s, sample_file(301, 203, kind), RGBA, shift)
                        self.check_rgba(r, 301, 203, shift, kind)
                        r = self.request(s, sample_file(301, 203, kind), NV12, shift)
                        self.check_nv12(r, 301, 203, shift, kind)

    def test_even_and_tiny_sizes(self):
        _, s = self.start()
        for w, h in ((16, 16), (17, 33), (64, 48), (1920, 1080)):
            for kind in ("420", "444"):
                with self.subTest(w=w, h=h, kind=kind):
                    r = self.request(s, sample_file(w, h, kind), RGBA, 3 if w > 100 else 0)
                    self.check_rgba(r, w, h, 3 if w > 100 else 0, kind)

    def test_bgra_order_and_studio_range_and_swapped_chroma(self):
        _, s = self.start()
        f = sample_file(97, 61, "420")
        self.check_rgba(self.request(s, f, BGRA, 1), 97, 61, 1, "420", bgra=True)
        self.check_rgba(self.request(s, f, RGBA, 1, LIMITED), 97, 61, 1, "420", studio=True, tol=2)
        self.check_rgba(self.request(s, f, RGBA, 1, SWAP_UV), 97, 61, 1, "420", swap=True)
        self.check_rgba(self.request(s, sample_file(97, 61, "gray"), RGBA, 1, LIMITED), 97, 61, 1, "gray", studio=True, tol=2)

    def test_swapped_chroma_in_nv12_swaps_the_pairs(self):
        _, s = self.start()
        f = sample_file(64, 48, "420")
        a = self.request(s, f, NV12, 0)
        b = self.request(s, f, NV12, 0, SWAP_UV)
        n = a["stride"] * a["height"]
        self.assertEqual(a["data"][:n], b["data"][:n])
        for i in range(n, len(a["data"]), 2):
            self.assertEqual((a["data"][i], a["data"][i + 1]), (b["data"][i + 1], b["data"][i]))

    def test_the_buffers_row_and_plane_alignment_is_a_flag(self):
        _, s = self.start("-A", "64", "-B", "16", FAKE_JPEG_ALIGN="64", FAKE_JPEG_VALIGN="16")
        for kind in ("420", "444", "422"):
            r = self.request(s, sample_file(301, 203, kind), RGBA, 1)
            self.check_rgba(r, 301, 203, 1, kind)
        # the same buffer read with the wrong alignment is wrong, not a crash
        _, s = self.start(FAKE_JPEG_ALIGN="64", FAKE_JPEG_VALIGN="16")
        r = self.request(s, sample_file(301, 203, "420"), RGBA, 1)
        self.assertEqual(r["status"], OK)
        with self.assertRaises(AssertionError):
            self.check_rgba(r, 301, 203, 1, "420")

    def test_a_padded_or_floored_hardware_size_is_cropped_to_the_pictures(self):
        _, s = self.start(FAKE_JPEG_PAD="1")
        r = self.request(s, sample_file(301, 203, "420"), RGBA, 1)
        self.assertEqual((r["hw_w"], r["hw_h"]), (160, 112))
        self.check_rgba(r, 301, 203, 1, "420")
        self.check_nv12(self.request(s, sample_file(301, 203, "420"), NV12, 1), 301, 203, 1, "420")
        _, s = self.start(FAKE_JPEG_FLOOR="1")
        r = self.request(s, sample_file(301, 203, "420"), RGBA, 1)
        self.assertEqual((r["status"], r["width"], r["height"]), (OK, 150, 101))

    # -- what is refused ------------------------------------------------------------------------------------------------------

    def test_what_the_block_does_not_take_is_refused_before_the_library_is_called(self):
        _, s = self.start()
        base = sample_file(64, 64, "420")
        cases = {
            "arithmetic coded": with_sof(base, marker=0xC9),
            "lossless": with_sof(base, marker=0xC3),
            "12 bit": with_sof(base, precision=12),
            "4:1:1": with_sof(base, hv=0x41),
            "too small": sample_file(8, 8, "420"),
            "too small tall": sample_file(64, 15, "420"),
        }
        # the same files without the flag: libjpeg takes what is only too small, and decodes arithmetic coding; neither takes the rest
        for name, data in cases.items():
            with self.subTest(name):
                self.assertEqual(self.request(s, data)["status"], E_UNSUPPORTED)
        self.assertEqual(self.request(s, b"\x89PNG\r\n\x1a\n" + b"\0" * 64)["status"], E_REQUEST)
        self.assertEqual(self.request(s, base[:20])["status"], E_REQUEST)
        self.assertEqual(self.lib_calls(), [], "the library must not be touched for these")
        self.assertEqual(self.request(s, base)["status"], OK)
        for name in ("lossless", "12 bit", "4:1:1"):
            with self.subTest("neither takes " + name):
                self.assertEqual(self.request(s, cases[name], hw_only=False)["status"], E_UNSUPPORTED)

    def test_progressive_and_gray_files_are_taken(self):
        _, s = self.start()
        self.assertEqual(self.request(s, jpeg(64, 64, progressive=True))["status"], OK)
        self.assertEqual(self.request(s, jpeg(64, 64, gray=True, progressive=True))["status"], OK)

    def test_a_bad_request_is_answered_and_the_next_one_still_works(self):
        _, s = self.start()
        f = sample_file(64, 48, "420")
        for kw in ({"fmt": 9}, {"fmt": 0}, {"shift": 4}, {"op": 5}):
            with self.subTest(**kw):
                self.assertEqual(self.request(s, f, **kw)["status"], E_REQUEST)
        self.assertEqual(self.request(s, b"", RGBA)["status"], E_REQUEST)
        self.assertEqual(self.request(s, f)["status"], OK)

    def test_a_wrong_magic_ends_the_connection(self):
        p, s = self.start()
        s.sendall(struct.pack("<8I", 0x12345678, 1, 1 | 2 << 8, 0, 0, 0, 0, 0))
        r = struct.unpack("<16I", self.read(s, 64))
        self.assertEqual(r[2], E_REQUEST)
        self.assertEqual(p.wait(10), 0)

    def test_the_output_is_capped_by_max_pixels(self):
        _, s = self.start()
        f = sample_file(640, 480, "420")
        self.assertEqual(self.request(s, f, RGBA, 0, 0, 640 * 480 - 1)["status"], E_TOO_BIG)
        self.assertEqual(self.request(s, f, RGBA, 1, 0, 320 * 240)["status"], OK)
        self.assertEqual(self.lib_calls().count("open"), 1)

    def test_ping_answers_without_the_library(self):
        _, s = self.start(library=False)
        r = self.request(s, b"", RGBA, 0, 0, 0, op=2)
        self.assertEqual((r["status"], r["len"]), (OK, 0))

    def test_a_missing_library_a_failed_open_and_a_failed_decode_are_statuses(self):
        _, s = self.start(library=False)
        self.assertEqual(self.request(s, sample_file(64, 64, "420"))["status"], E_NO_LIBRARY)
        self.assertEqual(self.request(s, sample_file(64, 64, "420"))["status"], E_NO_LIBRARY)
        _, s = self.start(FAKE_JPEG_OPEN_FAIL="1")
        self.assertEqual(self.request(s, sample_file(64, 64, "420"))["status"], E_OPEN)
        _, s = self.start(FAKE_JPEG_DECODE_FAIL="1")
        self.assertEqual(self.request(s, sample_file(64, 64, "420"))["status"], E_DECODE)
        _, s = self.start(FAKE_JPEG_BAD_FORMAT="1")
        self.assertEqual(self.request(s, sample_file(64, 64, "420"))["status"], E_OUTPUT)

    def test_a_decode_that_overruns_its_buffer_is_a_status(self):
        # a layout that needs more than the library allocated: the fake refuses like a hardware that cannot write it; and a
        # service told a bigger alignment than the library used refuses a buffer that is too small for it
        _, s = self.start("-A", "4096", "-B", "4096")
        self.assertEqual(self.request(s, sample_file(301, 203, "420"))["status"], E_OUTPUT)

    # -- the device -----------------------------------------------------------------------------------------------------------

    def test_the_device_is_opened_once_for_a_burst_and_given_back_when_it_ends(self):
        p, s = self.start("-c", "300", "-x", "60000")
        f = sample_file(64, 48, "420")
        for _ in range(3):
            self.assertEqual(self.request(s, f)["status"], OK)
        self.assertEqual([c for c in self.lib_calls() if c in ("open", "close")], ["open"])
        time.sleep(0.9)
        self.assertEqual([c for c in self.lib_calls() if c in ("open", "close")], ["open", "close"])
        self.assertEqual(self.request(s, f)["status"], OK)
        self.assertEqual([c for c in self.lib_calls() if c in ("open", "close")], ["open", "close", "open"])
        s.close()
        self.assertEqual(p.wait(10), 0)
        self.assertEqual([c for c in self.lib_calls() if c in ("open", "close")][-1], "close")

    def test_nothing_is_opened_until_the_first_decode_and_the_process_ends_when_idle(self):
        p, s = self.start("-x", "400")
        time.sleep(0.1)
        self.assertEqual(self.lib_calls(), [])
        self.assertEqual(p.wait(10), 0)

    def test_a_closed_connection_ends_the_process_and_gives_the_device_back(self):
        p, s = self.start()
        self.assertEqual(self.request(s, sample_file(64, 48, "420"))["status"], OK)
        s.close()
        self.assertEqual(p.wait(10), 0)
        self.assertEqual(self.lib_calls()[-1], "close")

    def test_a_request_cut_off_in_its_body_ends_the_process_cleanly(self):
        p, s = self.start()
        f = sample_file(64, 48, "420")
        s.sendall(struct.pack("<8I", REQ, 1, 1 | RGBA << 8, 0, len(f), 0, 0, 0) + f[:50])
        s.shutdown(socket.SHUT_WR)
        self.assertEqual(p.wait(10), 0)

    def test_sigterm_gives_the_device_back(self):
        p, s = self.start()
        self.assertEqual(self.request(s, sample_file(64, 48, "420"))["status"], OK)
        p.terminate()
        self.assertEqual(p.wait(10), 0)
        self.assertEqual(self.lib_calls()[-1], "close")

    def test_the_dry_run_resolves_the_three_functions(self):
        out = subprocess.run([BIN, "-n"], env=dict(os.environ, LD_LIBRARY_PATH=BUILD), capture_output=True, text=True, timeout=10)
        self.assertEqual(out.returncode, 0)
        self.assertRegex(out.stdout, r"library: open=0x\w+ close=0x\w+ decode=0x\w+")

    def test_the_bench_mode_reports_and_dumps(self):
        f = os.path.join(self.tmp, "a.jpg")
        with open(f, "wb") as o:
            o.write(sample_file(640, 480, "420"))
        out = subprocess.run([BIN, "-b", f, "-N", "5", "-z", "1", "-o", self.tmp], env=dict(os.environ, LD_LIBRARY_PATH=BUILD),
                             capture_output=True, text=True, timeout=30)
        self.assertEqual(out.returncode, 0, out.stderr)
        for key in ("width=640", "out_width=320", "hw_pixfmt=13", "decode_us min=", "convert_us min=", "ok=5 of 5", "jpgdec_irq_before=", "cma_free_kb_min="):
            self.assertIn(key, out.stdout)
        self.assertEqual(os.path.getsize(os.path.join(self.tmp, "out-s1.rgba")), 320 * 240 * 4)
        self.assertGreater(os.path.getsize(os.path.join(self.tmp, "hw-s1.bin")), 320 * 240 * 3 // 2)

    # -- the CPU path (libjpeg.so.62) -----------------------------------------------------------------------------------------

    @staticmethod
    def photo(w, h, sub="420", progressive=False, gray=False):
        """Colour that changes slowly (so a wrong chroma plane shows) under a luma checkerboard (edges, which chroma does not follow)."""
        img = Image.new("L" if gray else "RGB", (w, h))
        px = img.load()
        for j in range(h):
            for i in range(w):
                d = 25 if (i // 16 + j // 16) % 2 else -25
                v = (min(255, max(0, 40 + i * 170 // max(1, w - 1) + d)), min(255, max(0, 40 + j * 170 // max(1, h - 1) + d)),
                     min(255, max(0, 60 + (i + j) * 120 // max(1, w + h - 2) + d)))
                px[i, j] = v[0] if gray else v
        buf = io.BytesIO()
        kw = {} if gray else {"subsampling": {"444": 0, "422": 1, "420": 2}[sub]}
        img.save(buf, "JPEG", quality=90, progressive=progressive, **kw)
        return buf.getvalue()

    def pil_rgba(self, data, shift, gray=False):
        im = Image.open(io.BytesIO(data))
        if shift:
            im.draft("RGB", (im.width >> shift, im.height >> shift))  # libjpeg's own scaled IDCT, the scale this size selects
        full = Image.open(io.BytesIO(data))
        self.assertEqual(im.size, (ceil_shift(full.width, shift), ceil_shift(full.height, shift)))
        return im.convert("RGB")

    def mean_error(self, r, ref, bgra=False):
        self.assertEqual((r["width"], r["height"]), ref.size)
        ow, oh = ref.size
        src = ref.tobytes()
        d = r["data"]
        total = n = 0
        for j in range(oh):
            for i in range(ow):
                px = d[(j * ow + i) * 4:(j * ow + i) * 4 + 3]
                if bgra:
                    px = px[::-1]
                for c in range(3):
                    total += abs(px[c] - src[(j * ow + i) * 3 + c])
                    n += 1
        return total / n

    def test_libjpeg_decodes_what_the_library_cannot_and_says_so(self):
        _, s = self.start(library=False)
        for kind, kw in (("420", {}), ("422", {}), ("444", {}), ("gray", {"gray": True}), ("420 progressive", {"progressive": True})):
            for shift in range(4):
                with self.subTest(kind=kind, shift=shift):
                    data = self.photo(203, 131, kind.split()[0] if kind != "gray" else "420", **kw)
                    r = self.request(s, data, RGBA, shift, hw_only=False)
                    self.assertEqual(r["status"], OK)
                    self.assertEqual(r["flags"], RFLAG_CPU | E_NO_LIBRARY << 8)
                    self.assertLess(self.mean_error(r, self.pil_rgba(data, shift)), 1.5)

    def test_every_hardware_failure_falls_back_to_the_same_picture(self):
        data = self.photo(150, 100)
        want = self.pil_rgba(data, 1)
        for env, why in (({"FAKE_JPEG_OPEN_FAIL": "1"}, E_OPEN), ({"FAKE_JPEG_DECODE_FAIL": "1"}, E_DECODE), ({"FAKE_JPEG_BAD_FORMAT": "1"}, E_OUTPUT)):
            with self.subTest(why=why):
                _, s = self.start(**env)
                r = self.request(s, data, RGBA, 1, hw_only=False)
                self.assertEqual((r["status"], r["flags"]), (OK, RFLAG_CPU | why << 8))
                self.assertLess(self.mean_error(r, want), 1.5)
        # a file the block does not take (under its 16 pixels) goes to libjpeg without touching the library
        _, s = self.start()
        os.path.exists(self.log) and os.unlink(self.log)
        small = self.photo(12, 9)
        r = self.request(s, small, RGBA, 0, hw_only=False)
        self.assertEqual((r["status"], r["width"], r["height"], r["flags"]), (OK, 12, 9, RFLAG_CPU | E_UNSUPPORTED << 8))
        self.assertEqual(self.lib_calls(), [])

    def test_the_hardware_is_used_when_it_works_and_the_flags_say_so(self):
        _, s = self.start()
        r = self.request(s, sample_file(64, 48, "420"), RGBA, 0, hw_only=False)
        self.assertEqual((r["status"], r["flags"]), (OK, 0))
        self.assertTrue(any(c.startswith("decode") for c in self.lib_calls()))

    def test_cpu_only_never_touches_the_library(self):
        _, s = self.start()
        data = self.photo(97, 61)
        r = self.request(s, data, BGRA, 0, CPU_ONLY, hw_only=False)
        self.assertEqual((r["status"], r["flags"]), (OK, RFLAG_CPU))
        self.assertLess(self.mean_error(r, self.pil_rgba(data, 0), bgra=True), 1.5)
        self.assertEqual(self.lib_calls(), [])

    def test_nv12_from_libjpeg_has_the_right_luma_and_a_plausible_chroma(self):
        _, s = self.start(library=False)
        for kind in ("420", "422", "444"):
            data = self.photo(97, 61, kind)
            r = self.request(s, data, NV12, 0, hw_only=False)
            self.assertEqual((r["status"], r["stride"], r["len"]), (OK, 98, 98 * (61 + 31)))
            ref = Image.open(io.BytesIO(data)).convert("YCbCr")
            y, cb, cr = [c.tobytes() for c in ref.split()]
            err = sum(abs(r["data"][j * 98 + i] - y[j * 97 + i]) for j in range(61) for i in range(97)) / (97 * 61)
            self.assertLess(err, 2.0, kind)
            uv = r["data"][98 * 61:]
            errc = 0
            for cy in range(31):
                for cx in range(49):
                    for k, plane in enumerate((cb, cr)):
                        x0, y0 = cx * 2, cy * 2
                        cells = [plane[j * 97 + i] for j in (y0, min(y0 + 1, 60)) for i in (x0, min(x0 + 1, 96))]
                        errc += abs(uv[cy * 98 + cx * 2 + k] - sum(cells) / 4)
            self.assertLess(errc / (31 * 49 * 2), 4.0, kind)

    def test_a_truncated_or_corrupt_file_is_a_picture_or_a_status_never_a_crash(self):
        p, s = self.start(library=False)
        data = self.photo(160, 120)
        r = self.request(s, data[:len(data) // 2], RGBA, 0, hw_only=False)
        self.assertEqual((r["status"], r["width"], r["height"]), (OK, 160, 120))
        import random
        rnd = random.Random(5)
        head = data[:data.index(b"\xff\xda") + 14]
        for i in range(20):
            junk = head + bytes(rnd.randrange(256) for _ in range(3000))
            r = self.request(s, junk, RGBA, i % 4, hw_only=False)
            self.assertIn(r["status"], (OK, E_DECODE, E_UNSUPPORTED, E_REQUEST))
        self.assertIsNone(p.poll())
        self.assertEqual(self.request(s, data, RGBA, 0, hw_only=False)["status"], OK)

    def test_max_pixels_holds_on_the_cpu_path_too(self):
        _, s = self.start(library=False)
        data = self.photo(640, 480)
        self.assertEqual(self.request(s, data, RGBA, 0, 0, 640 * 480 - 1, hw_only=False)["status"], E_TOO_BIG)
        self.assertEqual(self.request(s, data, RGBA, 1, 0, 320 * 240, hw_only=False)["status"], OK)

    def test_the_sets_own_binary_runs_the_same_decode_under_qemu_with_the_32_bit_layout(self):
        arm = os.path.join(os.path.dirname(BUILD), "arm")
        if not (shutil.which("qemu-arm") and os.path.isdir("/usr/arm-linux-gnueabi") and os.path.exists(os.path.join(arm, "libdile_photo.so.1"))):
            self.skipTest("needs qemu-arm, /usr/arm-linux-gnueabi and jpeg/build.sh")
        mine, theirs = socket.socketpair()
        p = subprocess.Popen(["qemu-arm", "-L", "/usr/arm-linux-gnueabi", "-cpu", "max", "-E", "LD_LIBRARY_PATH=" + arm, os.path.join(arm, "lumatv-jpeg"),
                              "-f", str(theirs.fileno())], pass_fds=[theirs.fileno()], stderr=subprocess.PIPE, text=True)
        theirs.close()
        mine.settimeout(30)
        self.procs.append(p)
        self.socks.append(mine)
        for kind in ("420", "444", "gray"):
            r = self.request(mine, sample_file(97, 61, kind), RGBA, 1)
            self.check_rgba(r, 97, 61, 1, kind)
        self.check_nv12(self.request(mine, sample_file(97, 61, "420"), NV12, 0), 97, 61, 0, "420")
        mine.close()
        self.assertEqual(p.wait(30), 0)

    def test_the_converters_agree_with_double_precision_and_the_neon_ones_with_the_scalar_ones(self):
        out = subprocess.run([os.path.join(BUILD, "convert_test"), "-n", "1500"], capture_output=True, text=True, timeout=120)
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("converters agree", out.stdout)
        arm = os.path.join(os.path.dirname(BUILD), "arm", "convert_test")
        if shutil.which("qemu-arm") and os.path.isdir("/usr/arm-linux-gnueabi") and os.path.exists(arm):
            out = subprocess.run(["qemu-arm", "-L", "/usr/arm-linux-gnueabi", "-cpu", "max", arm, "-n", "600"], capture_output=True, text=True, timeout=300)
            self.assertEqual(out.returncode, 0, out.stdout)
            self.assertIn("implementation chosen by the converters: neon", out.stdout)
            self.assertNotIn(" 0 against the NEON", out.stdout)

    def test_the_bench_times_the_cpu_baseline_and_the_hardware_alone(self):
        f = os.path.join(self.tmp, "b.jpg")
        with open(f, "wb") as o:
            o.write(self.photo(640, 480))
        env = dict(os.environ, LD_LIBRARY_PATH=BUILD)
        cpu = subprocess.run([BIN, "-b", f, "-N", "3", "-C", "-o", self.tmp], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(cpu.returncode, 0, cpu.stderr)
        for key in ("path=cpu", "used=cpu", "ok=3 of 3", "decode_us min="):
            self.assertIn(key, cpu.stdout)
        hw = subprocess.run([BIN, "-b", f, "-N", "3", "-H"], env=dict(os.environ), capture_output=True, text=True, timeout=30)
        self.assertEqual(hw.returncode, 2, "no library: every iteration is a status, none a silent fallback")
        self.assertIn("status=3", hw.stdout)


if __name__ == "__main__":
    unittest.main()
