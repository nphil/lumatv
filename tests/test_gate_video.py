"""Run with: python3 -m unittest tests.test_gate_video

The video check of the C4 gate (compositor/tools/tv-gate.sh `video`) and its picture reader (tv-pngstat.py) under sh against a fake set: the page
format and the OSD's constant alpha, the DISPLAY capture that must not be black, the compositor's cost with nothing over the video, and an overlay
over it. The compositor's `ctl` answers from files, LG's capture service is a script that writes a PNG, /proc and the fbdev files are directories.
"""
import json
import os
import struct
import subprocess
import sys
import unittest
import zlib

from tests.test_gate_lpc import Fixture, ROOT, write

TOOLS = os.path.join(ROOT, "compositor", "tools")


def png(path, w, h, pixel, ctype=2):
    """An 8-bit PNG; `pixel(x, y)` is an (r, g, b) tuple. Every row uses a different filter, to exercise the reader's unfiltering."""
    channels = 4 if ctype == 6 else 3
    rows, prev = [], bytes(w * channels)
    for y in range(h):
        line = bytearray()
        for x in range(w):
            line += bytes(pixel(x, y)) + (b"\xff" if ctype == 6 else b"")
        ftype = y % 5
        out = bytearray([ftype])
        for i, v in enumerate(line):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if ftype == 0:
                pred = 0
            elif ftype == 1:
                pred = a
            elif ftype == 2:
                pred = b
            elif ftype == 3:
                pred = (a + b) >> 1
            else:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
            out.append((v - pred) & 255)
        rows.append(bytes(out))
        prev = bytes(line)

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, ctype, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b""))


def stat(path, *box):
    r = subprocess.run([sys.executable, os.path.join(TOOLS, "tv-pngstat.py"), path, *map(str, box)], capture_output=True, text=True, timeout=30)
    return dict(kv.split("=", 1) for kv in r.stdout.split())


class Pngstat(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "a.png")

    def tearDown(self):
        self.tmp.cleanup()

    def test_opaque_black_is_black_and_a_picture_is_not_in_every_filter_and_colour_type(self):
        for ctype in (2, 6):
            png(self.path, 96, 54, lambda x, y: (0, 0, 0), ctype)
            self.assertEqual(stat(self.path)["black"], "yes")
            png(self.path, 96, 54, lambda x, y: (200 * (x > 48), 120, 30 + 2 * y), ctype)
            s = stat(self.path)
            self.assertEqual((s["black"], s["w"], s["h"]), ("no", "96", "54"), s)
            self.assertGreater(float(s["std"]), 4)

    def test_a_dark_scene_with_detail_is_not_black_and_a_flat_dark_grey_floor_is(self):
        png(self.path, 96, 54, lambda x, y: (2 + 8 * ((x // 4 + y // 4) % 2),) * 3)
        self.assertEqual(stat(self.path)["black"], "no", "a spread over 4: a dark scene")
        png(self.path, 96, 54, lambda x, y: (3, 3, 3))
        self.assertEqual(stat(self.path)["black"], "yes")

    def test_the_region_is_the_middle_half_unless_given_and_black_around_a_picture_is_seen(self):
        lit = lambda x, y: (220, 180, 90) if 24 <= x < 72 and 14 <= y < 40 else (0, 0, 0)
        png(self.path, 96, 54, lit)
        self.assertEqual(stat(self.path)["black"], "no")
        self.assertEqual(stat(self.path, 0.0, 0.0, 0.2, 0.2)["black"], "yes", "the corner is black")

    def test_a_file_that_is_not_a_picture_is_black_with_the_reason(self):
        with open(self.path, "wb") as f:
            f.write(b"not a png")
        s = stat(self.path)
        self.assertEqual(s["black"], "yes")
        self.assertIn("error", s)
        self.assertEqual(stat(os.path.join(self.tmp.name, "missing.png"))["black"], "yes")


LUNA = r'''#!/bin/sh
# luna-send [-n 1] [-f] URI JSON: the capture service writes the prepared picture to the path asked; anything else answers true.
for last; do :; done
case "$*" in
*capture/executeOneShot*)
    p=$(echo "$last" | sed -n 's/.*"path":"\([^"]*\)".*/\1/p')
    if [ -f "$FAKE/capture.png" ]; then cp "$FAKE/capture.png" "$p"; echo '{"returnValue":true,"writtenBytes":4321}'; else echo '{"returnValue":false}'; fi ;;
*) echo '{"returnValue":true}' ;;
esac
'''
STATUS_OK = {"ok": True, "pageFormat": "AR24", "videoHolesOpaque": False, "stats": {"drawn": 100}}
SHELL = {"ok": True, "rust": {"connected": True, "kinds": ["osd"], "ui": {"kinds": ["osd", "quick"], "osd": True, "panels": {"quick": "open"}}}}


class Video(Fixture):
    def setUp(self):
        super().setUp()
        d = self.tmp.name
        for name in ("tv-rustjson.py", "tv-pngstat.py"):
            with open(os.path.join(TOOLS, name)) as f:
                write(os.path.join(self.dep, name.replace("tv-", "")), f.read(), 0o755)
        write(os.path.join(self.fakebin, "luna-send"), LUNA, 0o755)
        write(os.path.join(self.proc, "4242", "stat"), "4242 (lumatv-composit) S " + " ".join(["0"] * 10) + " 500 300 " + " ".join(["0"] * 30) + "\n")
        self.fbdev = os.path.join(d, "fbdev")
        write(os.path.join(self.fbdev, "fb0", "status"), " hdr.global_alpha_en: 0\n hdr.global_alpha: 0xff\n")
        write(os.path.join(self.fbdev, "fb0", "flip_cnt"), "1000\n")
        self.env.update(LUMATV_FBDEV=self.fbdev, LUMATV_MALI_UTIL=os.path.join(d, "none"), LUMATV_RUST_OPEN_WAIT="1", LUMATV_VIDEO_WAIT="2",
                        LUMATV_VIDEO_OVERLAYS="osd,quick")
        self.ctl_reply("status", STATUS_OK)
        self.ctl_reply("windows", {"ok": True, "windows": [], "video": [{"surface": 7}]})
        self.ctl_reply("shell", SHELL)
        self.ctl_reply("refresh", {"ok": True, "hz": 60, "intervals": {"one": 100, "two": 0, "three": 0, "more": 0, "worst_ms": 16.9}, "frames": {"late": 0}})
        self.picture(lambda x, y: (200 * (x > 48), 120, 30 + 2 * y))

    def picture(self, pixel):
        png(os.path.join(self.tmp.name, "capture.png"), 96, 54, pixel)

    def run_video(self, *args):
        return self.gate("video", "current", "10", *args).strip().splitlines()[-1]

    def test_a_video_with_ar24_pages_alpha_off_in_fb0_and_a_picture_in_the_capture_passes_with_its_numbers(self):
        out = self.run_video()
        self.assertTrue(out.startswith("GATE video-current: PASS"), out)
        for part in ("pages AR24 holes-opaque false fb0 global_alpha_en 0", "non-black in 3 of 3", "compositor CPU", "composed", "OSD flips", "overlays:", "osd: first frame", "quick: first frame"):
            self.assertIn(part, out)

    def test_xr24_pages_fail_it_whatever_the_picture(self):
        self.ctl_reply("status", dict(STATUS_OK, pageFormat="XR24", videoHolesOpaque=True))
        write(os.path.join(self.fbdev, "fb0", "status"), " hdr.global_alpha_en: 1\n")
        out = self.run_video()
        self.assertTrue(out.startswith("GATE video-current: FAIL"), out)
        self.assertIn("the pages are not AR24 with transparent holes", out)
        self.assertIn("fb0 hdr.global_alpha_en is '1', not 0", out)

    def test_a_black_display_capture_fails_it_even_with_ar24_pages(self):
        self.picture(lambda x, y: (0, 0, 0))
        out = self.run_video()
        self.assertTrue(out.startswith("GATE video-current: FAIL"), out)
        self.assertIn("the DISPLAY capture's middle is black in 3 of 3", out)

    def test_no_capture_at_all_is_black_too(self):
        os.remove(os.path.join(self.tmp.name, "capture.png"))
        self.assertIn("the video does not show", self.run_video())

    def test_a_compositor_that_works_under_a_video_with_nothing_over_it_fails_it_with_its_numbers(self):
        # 500+300 ticks at the start; the fake never moves, so the bar is crossed by a bar of zero
        self.env["LUMATV_VIDEO_MAX_CPU"] = "-1"
        out = self.run_video()
        self.assertIn("the compositor used 0.0 % of a core under a video with nothing over it (bar -1 %)", out)

    def test_an_overlay_that_hurts_the_video_fails_it_by_name(self):
        self.ctl_reply("refresh", {"ok": True, "hz": 60, "intervals": {"one": 100, "two": 3, "three": 1, "more": 0, "worst_ms": 50.0}, "frames": {"late": 4}})
        out = self.run_video()
        self.assertTrue(out.startswith("GATE video-current: FAIL"), out)
        self.assertIn("osd's open had 4 frame intervals over 20 ms", out)
        # at 120 Hz two refreshes are 16.7 ms: only the longer ones count
        self.ctl_reply("refresh", {"ok": True, "hz": 120, "intervals": {"one": 100, "two": 3, "three": 1, "more": 0, "worst_ms": 25.0}, "frames": {"late": 0}})
        out = self.run_video()
        self.assertIn("osd's open had 1 frame intervals over 20 ms", out)

    def test_no_video_element_is_a_failure_that_says_what_it_needs(self):
        self.ctl_reply("windows", {"ok": True, "windows": [], "video": []})
        out = self.run_video()
        self.assertTrue(out.startswith("GATE video-current: FAIL no video element came up within 2 s"), out)

    def test_an_overlay_the_shell_did_not_build_is_said_so_not_failed(self):
        self.env["LUMATV_VIDEO_OVERLAYS"] = "osd,drawer"
        out = self.run_video()
        self.assertIn("drawer: not built", out)
        self.assertTrue(out.startswith("GATE video-current: PASS"), out)


if __name__ == "__main__":
    unittest.main()
