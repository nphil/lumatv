"""Run with: python3 -m unittest tests.test_jpeg_compare

jpeg/compare.py, the TV slot's reading of the hardware block's buffer, against buffers laid out the ways the block might lay them out
(made here from libjpeg's own planes): it has to name the row and plane alignment, the chroma order and the range, and say so when
nothing fits.
"""
import io
import os
import random
import re
import subprocess
import sys
import tempfile
import unittest

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
COMPARE = os.path.join(os.path.dirname(HERE), "jpeg", "compare.py")


def picture(w, h, sub=2):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for j in range(h):
        for i in range(w):
            d = 25 if (i // 16 + j // 16) % 2 else -25
            px[i, j] = (min(255, max(0, 40 + i * 170 // (w - 1) + d)), min(255, max(0, 40 + j * 170 // (h - 1) + d)), 90 + (i + j) % 40)
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=92, subsampling=sub)
    return buf.getvalue()


def up(v, a):
    return (v + a - 1) // a * a


def buffer(jpeg, hw, A, B, semi, swap, studio, hs=1, vs=1):
    """What a block that wrote planes with these alignments would hold: Y, then Cb and Cr, from libjpeg's YCbCr."""
    im = Image.open(io.BytesIO(jpeg))
    im.draft("YCbCr", im.size)
    y, cb, cr = [p.tobytes() for p in im.split()]
    w, h = im.size
    ys, yr = up(hw[0], A), up(hw[1], B)
    out = bytearray(ys * yr * 3)
    for j in range(h):
        for i in range(w):
            v = y[j * w + i]
            out[j * ys + i] = (16 + v * 219 // 255) if studio else v
    cw, crows = (ys + (1 << hs) - 1) >> hs, (yr + (1 << vs) - 1) >> vs
    first, second = (cr, cb) if swap else (cb, cr)
    base = ys * yr
    for cy in range((h + (1 << vs) - 1) >> vs):
        for cx in range((w + (1 << hs) - 1) >> hs):
            a, b = first[min(h - 1, cy << vs) * w + min(w - 1, cx << hs)], second[min(h - 1, cy << vs) * w + min(w - 1, cx << hs)]
            if semi:
                out[base + cy * cw * 2 + cx * 2] = a
                out[base + cy * cw * 2 + cx * 2 + 1] = b
            else:
                out[base + cy * cw + cx] = a
                out[base + cw * crows + cy * cw + cx] = b
    return bytes(out)


class Compare(unittest.TestCase):
    def run_compare(self, jpeg, dump, hw, pf, shift=0):
        with tempfile.TemporaryDirectory() as d:
            for name, data in (("a.jpg", jpeg), ("hw.bin", dump)):
                with open(os.path.join(d, name), "wb") as f:
                    f.write(data)
            with open(os.path.join(d, "bench.txt"), "w") as f:
                f.write("out_width=1 out_height=1 out_stride=4 hw_width=%d hw_height=%d hw_pixfmt=%d hw_bytes=%d used=hardware hw_status=0\n" % (hw[0], hw[1], pf, len(dump)))
            return subprocess.run([sys.executable, COMPARE, os.path.join(d, "a.jpg"), "--dump", os.path.join(d, "hw.bin"), "--report",
                                   os.path.join(d, "bench.txt"), "--shift", str(shift)], capture_output=True, text=True, timeout=120)

    def test_it_finds_the_alignment_the_chroma_order_and_the_range(self):
        jpeg = picture(100, 60)
        for A, B, semi, swap, studio in ((1, 1, True, 0, False), (64, 16, True, 0, False), (16, 16, False, 0, False), (32, 8, True, 1, False),
                                         (64, 16, True, 0, True), (128, 32, False, 1, True)):
            with self.subTest(A=A, B=B, semi=semi, swap=swap, studio=studio):
                hw = (100, 60)
                r = self.run_compare(jpeg, buffer(jpeg, hw, A, B, semi, swap, studio), hw, 13 if semi else 12)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                m = re.search(r"consistent: A in \[([\d, ]+)\] B in \[([\d, ]+)\]", r.stdout)
                self.assertIn(A, [int(x) for x in m.group(1).split(",")], r.stdout)
                self.assertIn(B, [int(x) for x in m.group(2).split(",")], r.stdout)
                self.assertEqual(('"-S"' in r.stdout.split("decoderArgs")[1], '"-L"' in r.stdout.split("decoderArgs")[1]), (bool(swap), studio))

    def test_a_padded_size_from_the_block_is_laid_out_from_the_padded_width(self):
        jpeg = picture(100, 60)
        hw = (112, 64)
        r = self.run_compare(jpeg, buffer(jpeg, hw, 16, 16, True, 0, False), hw, 13)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("lumatv-jpeg crops", r.stdout)
        m = re.search(r"consistent: A in \[([\d, ]+)\] B in \[([\d, ]+)\]", r.stdout)  # the block's own 112x64 already is a multiple of 16
        self.assertIn(16, [int(x) for x in m.group(1).split(",")])
        self.assertIn(16, [int(x) for x in m.group(2).split(",")])

    def test_a_size_that_hides_the_alignment_says_so_and_an_odd_one_does_not(self):
        jpeg = picture(128, 64)
        r = self.run_compare(jpeg, buffer(jpeg, (128, 64), 16, 16, True, 0, False), (128, 64), 13)
        self.assertIn("several", r.stdout)
        jpeg = picture(131, 67)
        r = self.run_compare(jpeg, buffer(jpeg, (131, 67), 16, 16, True, 0, False), (131, 67), 13)
        self.assertIn("consistent: A in [16] B in [16]", r.stdout)
        self.assertNotIn("several", r.stdout)

    def test_noise_matches_no_layout(self):
        jpeg = picture(100, 60)
        rnd = random.Random(3)
        r = self.run_compare(jpeg, bytes(rnd.randrange(256) for _ in range(100 * 60 * 3)), (100, 60), 13)
        self.assertEqual(r.returncode, 1)
        self.assertIn("NO LAYOUT MATCHES", r.stdout)

    def test_unreadable_inputs_are_exit_2(self):
        r = subprocess.run([sys.executable, COMPARE, "/nonexistent.jpg", "--dump", "/nonexistent", "--report", "/nonexistent"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


if __name__ == "__main__":
    unittest.main()
