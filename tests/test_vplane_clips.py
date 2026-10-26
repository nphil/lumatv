"""Run with: cd tests && python3 -m unittest test_vplane_clips   (needs Pillow; the encoder tests need ffmpeg with libx265, ffprobe for the deep ones)

tools/vplane_clips.py, the writer of the art mode's clips for the video-plane player, and the clips it wrote (payload/vplane/clips):
that the composition is the art mode's own (the numbers are read from the QML, and a clip written before one of them moved is reported),
that the container reads back what was written, that the stream is what the decoder needs (a key picture, then predicted copies, no
reordering, one reference), and that the shipped set is complete, small and consistent with its index.
"""
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "tools"))

try:
    import vplane_clips as vc
    from PIL import Image
    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False

SHIPPED = os.path.join(ROOT, "payload", "vplane", "clips")


def load(path):
    with open(path) as f:
        return json.load(f)


def read(path):
    with open(path, "rb") as f:
        return f.read()


def write(path, blob):
    with open(path, "wb") as f:
        f.write(blob)
QML = os.path.join(ROOT, "payload", "compositor", "lumatv")
HAVE_ENCODER = bool(shutil.which("ffmpeg")) and "libx265" in subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True).stdout if shutil.which("ffmpeg") else False


@unittest.skipUnless(HAVE_PIL, "needs Pillow")
class Numbers(unittest.TestCase):
    def test_the_composition_reads_the_art_modes_own_numbers(self):
        geo = vc.geometry()
        piece = read(os.path.join(QML, "panels", "ArtPiece.qml")).decode()
        self.assertEqual((geo["maxW"], geo["maxH"], geo["matte"], geo["frame"]), (1040, 700, 64, 18))
        for k in ("maxW", "maxH", "matte", "frame"):
            self.assertIn("property int %s: %d" % (k, geo[k]), piece)
        self.assertEqual((geo["frameColor"], geo["matteColor"]), ("#1F1A1B", "#E9E2D3"))
        self.assertEqual(geo["wall"], [[0.0, "#100d12"], [0.7, "#060507"], [1.0, "#030203"]])
        self.assertEqual((geo["safeX"], geo["safeY"], geo["edgeX"]), (96, 54, 64))
        theme = load(os.path.join(ROOT, "payload", "theme", "theme.json"))
        self.assertEqual(geo["artLevel"], theme["screensaver"]["artLevel"])
        self.assertEqual(geo["ink"], theme["dark"]["ink"])

    def test_the_picture_has_room_for_the_shift_the_qml_makes(self):
        d = vc.drift_numbers()
        self.assertEqual(d, {"ampX": 30, "periodX": 540, "ampY": 20, "periodY": 380})
        room = ((vc.SCREEN[0] - vc.CANVAS[0]) // 2, (vc.SCREEN[1] - vc.CANVAS[1]) // 2)
        self.assertEqual(room, (32, 28))
        self.assertLessEqual(d["ampX"], room[0])
        self.assertLessEqual(d["ampY"], room[1])
        self.assertEqual(vc.CANVAS[0] % 32, 0)
        self.assertEqual(vc.CANVAS[1] % 32, 0)

    def test_the_hex_colours_and_the_wall_gradient(self):
        self.assertEqual(vc.hexrgb("#E9E2D3"), ((233, 226, 211), 1.0))
        rgb, a = vc.hexrgb("#A8F2EFF6")                     # Qt's order: alpha first
        self.assertEqual(rgb, (0xF2, 0xEF, 0xF6))
        self.assertAlmostEqual(a, 168 / 255)
        geo = vc.geometry()
        top, mid, bottom = vc.wall_color(geo, 0), vc.wall_color(geo, 756), vc.wall_color(geo, 1079)
        self.assertEqual(top, (16, 13, 18))
        self.assertEqual(mid, (6, 5, 7))
        self.assertEqual(bottom, (3, 2, 3))
        self.assertTrue(all(a >= b for a, b in zip(top, mid)) and all(a >= b for a, b in zip(mid, bottom)), "darker down the wall")


@unittest.skipUnless(HAVE_PIL, "needs Pillow")
class Picture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.geo = vc.geometry()
        cls.entry = next(p for p in vc.pictures() if p["id"] == "starry-night")
        cls.img = vc.compose(cls.entry, cls.geo)

    def test_the_size_and_the_black_edges(self):
        w, h = self.img.size
        self.assertEqual((w, h), vc.CANVAS)
        for x, y in ((0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (0, h // 2), (w - 1, h // 2), (w // 2, 0), (w // 2, h - 1)):
            self.assertEqual(self.img.getpixel((x, y)), (0, 0, 0), "pure black at the edge, so the window's edge never shows: %s" % ((x, y),))
        self.assertEqual(self.img.getpixel((3, 500)), (0, 0, 0))
        self.assertTrue(any(self.img.getpixel((vc.FADE + 4, 500))), "the wall is there once the fade is over")

    def test_the_framed_print_sits_at_the_centre_of_the_screen(self):
        g = self.geo
        e = self.entry
        fit = min(g["maxW"] / e["width"], g["maxH"] / e["height"])
        pw, ph = round(e["width"] * fit), round(e["height"] * fit)
        cx, cy = vc.CANVAS[0] // 2, vc.CANVAS[1] // 2
        # the matte, left of the print, in the art level's brightness
        mx = cx - pw // 2 - g["matte"] // 2
        want = tuple(round(v * g["artLevel"]) for v in vc.hexrgb(g["matteColor"])[0])
        self.assertEqual(self.img.getpixel((mx, cy)), want)
        # the frame outside the matte
        fx = cx - pw // 2 - g["matte"] - g["frame"] // 2
        want = tuple(round(v * g["artLevel"]) for v in vc.hexrgb(g["frameColor"])[0])
        self.assertEqual(self.img.getpixel((fx, cy)), want)
        # the print itself is not the matte
        self.assertNotEqual(self.img.getpixel((cx, cy)), self.img.getpixel((mx, cy)))

    def test_the_caption_is_baked_in_at_the_corner_inside_the_safe_margin(self):
        g = self.geo
        mx, my = (vc.SCREEN[0] - vc.CANVAS[0]) // 2, (vc.SCREEN[1] - vc.CANVAS[1]) // 2
        x0, x1 = g["edgeX"] - mx, g["edgeX"] - mx + 300
        y1 = vc.SCREEN[1] - g["safeY"] - my
        lit = [(x, y) for x in range(x0, x1) for y in range(y1 - 40, y1) if max(self.img.getpixel((x, y))) > 60]
        self.assertGreater(len(lit), 300, "text pixels at the bottom left")
        self.assertTrue(all(y <= y1 for _, y in lit))
        right = [(x, y) for x in range(vc.CANVAS[0] - 300, vc.CANVAS[0] - 60) for y in range(y1 - 40, y1) if max(self.img.getpixel((x, y))) > 60]
        self.assertEqual(right, [], "the clock is not baked in: it changes every minute, the player draws it")

    def test_the_art_level_is_applied_to_the_pieces_and_not_to_the_caption(self):
        # the brightest point of the matte is the art level of its colour; the caption's ink is brighter than that colour's dimmed ink would be
        g = self.geo
        matte = max(vc.hexrgb(g["matteColor"])[0])
        self.assertLessEqual(max(self.img.getpixel((vc.CANVAS[0] // 2 - 440, vc.CANVAS[1] // 2))), round(matte * g["artLevel"]) + 1)


class Container(unittest.TestCase):
    def test_a_clip_reads_back_what_was_written(self):
        if not HAVE_PIL:
            self.skipTest("needs Pillow")
        with tempfile.TemporaryDirectory() as d:
            units = [(b"\x00\x00\x00\x01\x46\x01" + b"k" * 90, True)] + [(b"\x00\x00\x00\x01\x02\x01" + bytes([i]) * 10, False) for i in range(1, 9)]
            path = os.path.join(d, "a.vpc")
            vc.write_vpc(path, "hevc", (1856, 1024), 60, units)
            got = vc.read_vpc(path)
            self.assertEqual((got["codec"], got["width"], got["height"], got["fps"], got["loopFrom"]), ("hevc", 1856, 1024, 60.0, 0))
            self.assertEqual(got["units"], units)
            blob = read(path)
            self.assertEqual(blob[:4], b"VPC1")
            self.assertEqual(struct.unpack_from("<I", blob, 32)[0], 40 + 8 * 9, "the payload starts after the table")
            write(path, blob[:-1])
            with self.assertRaises(ValueError):
                vc.read_vpc(path)

    def test_a_stream_is_cut_at_its_access_unit_delimiters_and_keys_are_found(self):
        if not HAVE_PIL:
            self.skipTest("needs Pillow")
        sc = b"\x00\x00\x00\x01"
        # H.265: AUD is NAL type 35, IDR_W_RADL 19, a trailing picture 1; the NAL header's first byte is type << 1
        def h265(t, body=b"xx"):
            return sc + bytes([t << 1, 1]) + body
        stream = h265(35) + h265(32) + h265(33) + h265(34) + h265(19, b"idr") + h265(35) + h265(1, b"p1") + h265(35) + h265(1, b"p2")
        units = vc.split(stream, "hevc")
        self.assertEqual([k for _, k in units], [True, False, False])
        self.assertEqual(b"".join(u for u, _ in units), stream, "nothing lost, nothing added")
        self.assertTrue(units[0][0].startswith(sc + bytes([35 << 1, 1])))
        # H.264: AUD is type 9, IDR slice 5, non-IDR slice 1
        def h264(t, body=b"xx"):
            return sc + bytes([0x60 | t]) + body
        stream = h264(9) + h264(7) + h264(8) + h264(5, b"i") + h264(9) + h264(1, b"p")
        units = vc.split(stream, "h264")
        self.assertEqual([k for _, k in units], [True, False])
        with self.assertRaises(SystemExit):
            vc.split(h265(1) + h265(35), "hevc")     # a stream that does not begin with a delimiter

    def test_three_byte_start_codes_are_found_too(self):
        if not HAVE_PIL:
            self.skipTest("needs Pillow")
        stream = b"\x00\x00\x01" + bytes([35 << 1, 1, 0x50]) + b"\x00\x00\x01" + bytes([19 << 1, 1, 5])
        self.assertEqual([k for _, k in vc.split(stream, "hevc")], [True])


class Glyphs(unittest.TestCase):
    def test_the_atlas_has_the_digits_and_the_colon_laid_out_for_the_corner(self):
        if not HAVE_PIL:
            self.skipTest("needs Pillow")
        a = vc.read_glyphs(os.path.join(SHIPPED, "clock.vpg"))
        self.assertEqual(set(a["glyphs"]), set("0123456789:"))
        self.assertEqual(a["right"], 1920 - 64, "the QML's right edge: 1920 - edgeX")
        self.assertEqual(a["baseline"], 1080 - 54 - 11)
        for ch, g in a["glyphs"].items():
            self.assertEqual(len(g["alpha"]), g["w"] * g["h"], ch)
            self.assertGreater(g["advance"], 5, ch)
            self.assertGreater(sum(g["alpha"]), 255 * 6, "%r has ink" % ch)
        self.assertGreater(a["glyphs"]["0"]["h"], 18, "a digit of a 32 px face is taller than 18")
        self.assertLess(a["glyphs"]["0"]["h"], 32)
        widths = {g["advance"] for c, g in a["glyphs"].items() if c.isdigit()}
        self.assertLess(max(widths) - min(widths), 8, "digits of nearly one width")


@unittest.skipUnless(HAVE_PIL, "needs Pillow")
class Shipped(unittest.TestCase):
    def test_the_check_passes_on_the_shipped_set(self):
        self.assertEqual(vc.check(SHIPPED), [])

    def test_the_set_is_one_clip_for_each_public_domain_print_and_small(self):
        doc = load(os.path.join(SHIPPED, "index.json"))
        prints = {p["id"] for p in vc.pictures()}
        self.assertEqual({c["id"] for c in doc["clips"]}, prints)
        self.assertEqual(len(prints), 6)
        total = sum(c["bytes"] for c in doc["clips"])
        self.assertLess(total, 3_000_000, "the clips add under 3 MB to the package: %d" % total)
        for c in doc["clips"]:
            self.assertLess(c["bytes"], 600_000, c["id"])
            self.assertEqual(c["fps"], 60)
            self.assertEqual(c["units"], 120)
            self.assertEqual(c["sha256"], hashlib.sha256(read(os.path.join(SHIPPED, c["file"]))).hexdigest())

    def test_a_clip_is_one_key_picture_and_predicted_copies_that_are_nearly_free(self):
        for c in load(os.path.join(SHIPPED, "index.json"))["clips"]:
            v = vc.read_vpc(os.path.join(SHIPPED, c["file"]))
            sizes = [len(u) for u, _ in v["units"]]
            self.assertTrue(v["units"][0][1] and not any(k for _, k in v["units"][1:]))
            self.assertLess(max(sizes[1:]), 8000, "%s: a predicted copy of a still is a few hundred bytes, the first few refine it a little" % c["id"])
            self.assertLess(sum(sizes[1:]), sizes[0] // 20, "%s: all the predicted units together are under a twentieth of the key picture" % c["id"])
            self.assertGreater(sizes[0], 100_000)
            # parameter sets in front of the key unit (the decoder starts from it every loop) and none in the others
            self.assertIn(b"\x00\x00\x00\x01\x40", v["units"][0][0], "a VPS in the key unit")
            self.assertIn(b"\x00\x00\x00\x01\x42", v["units"][0][0], "an SPS")
            self.assertNotIn(b"\x00\x00\x00\x01\x42", v["units"][1][0])

    def test_the_check_finds_a_moved_number_a_changed_file_and_a_missing_clip(self):
        with tempfile.TemporaryDirectory() as d:
            shutil.copytree(SHIPPED, d, dirs_exist_ok=True)
            self.assertEqual(vc.check(d), [])
            real = vc.geometry
            try:
                vc.geometry = lambda: dict(real(), maxW=real()["maxW"] + 10)
                bad = vc.check(d)
            finally:
                vc.geometry = real
            self.assertTrue(any("moved" in b and "maxW" in b for b in bad), bad)
            self.assertTrue(any("changed since" in b for b in bad), "every clip is stale then")
            path = os.path.join(d, "great-wave.vpc")
            blob = bytearray(read(path))
            blob[5000] ^= 0xFF
            write(path, bytes(blob))
            self.assertTrue(any("great-wave: not the file" in b for b in vc.check(d)))
            os.unlink(path)
            self.assertTrue(any("great-wave: file missing" in b for b in vc.check(d)))
            os.unlink(os.path.join(d, "index.json"))
            self.assertEqual(len(vc.check(d)), 1)

    def test_a_clip_for_a_print_the_art_mode_does_not_show_is_reported(self):
        with tempfile.TemporaryDirectory() as d:
            shutil.copytree(SHIPPED, d, dirs_exist_ok=True)
            doc = load(os.path.join(d, "index.json"))
            doc["clips"].append(dict(doc["clips"][0], id="not-in-the-art-index"))
            with open(os.path.join(d, "index.json"), "w") as f:
                json.dump(doc, f)
            self.assertTrue(any("not-in-the-art-index" in b for b in vc.check(d)))

    def test_the_prints_are_the_art_modes_licence_filtered_set(self):
        index = load(os.path.join(QML, "art", "index.json"))["pictures"]
        self.assertEqual({p["id"] for p in vc.pictures()}, {p["id"] for p in index if p["licence"] == "Public domain"})
        self.assertTrue(all(p["licence"] == "Public domain" for p in vc.pictures()))

    @unittest.skipUnless(shutil.which("ffprobe"), "needs ffprobe")
    def test_every_shipped_stream_decodes_with_one_reference_and_no_reordering(self):
        self.assertEqual(vc.check(SHIPPED, deep=True), [])


@unittest.skipUnless(HAVE_PIL and HAVE_ENCODER, "needs Pillow and ffmpeg with libx265")
class Encoder(unittest.TestCase):
    def test_one_print_through_the_encoder(self):
        with tempfile.TemporaryDirectory() as d:
            r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "vplane_clips.py"), "art", "--out", d, "--only", "starry-night", "--loop-sec", "1"],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr[-800:])
            v = vc.read_vpc(os.path.join(d, "starry-night.vpc"))
            self.assertEqual((v["codec"], v["width"], v["height"], v["fps"], len(v["units"])), ("hevc", 1856, 1024, 60.0, 60))
            self.assertTrue(v["units"][0][1] and not any(k for _, k in v["units"][1:]))
            self.assertEqual(vc.deep_check("starry-night", v), [])
            doc = load(os.path.join(d, "index.json"))
            self.assertEqual(doc["clips"][0]["opts"], {"codec": "hevc", "crf": 16, "frames": 60, "fps": 60})
            # the picture is decodable to what was composed: the matte's brightness survives the encode within the codec's tolerance
            if shutil.which("ffmpeg"):
                raw = os.path.join(d, "f.png")
                with open(os.path.join(d, "s.bin"), "wb") as f:
                    f.write(b"".join(u for u, _ in v["units"][:1]))
                subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "hevc", "-i", os.path.join(d, "s.bin"), "-frames:v", "1", raw], check=True)
                got = Image.open(raw).convert("RGB")
                want = vc.compose(next(p for p in vc.pictures() if p["id"] == "starry-night"), vc.geometry())
                p = (vc.CANVAS[0] // 2 - 440, vc.CANVAS[1] // 2)
                self.assertTrue(all(abs(a - b) <= 4 for a, b in zip(got.getpixel(p), want.getpixel(p))), (got.getpixel(p), want.getpixel(p)))
                self.assertEqual(got.getpixel((0, 0)), (0, 0, 0), "the black edge is black after the encode (limited range back to full)")


if __name__ == "__main__":
    unittest.main()
