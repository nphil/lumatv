"""Run with: cd tests && python3 -m unittest test_vplane_wall_clips   (needs Pillow 10.3+; the encoder tests need ffmpeg with 10-bit libx265, the
deep ones ffprobe and about a minute; the parity test needs bun)

tools/vplane_wall.py, the writer of Home's moving wallpaper clip for the video-plane player (vplane/), and the clip it wrote
(payload/vplane/clips/wall-silk.vpc, wall.json): that the warp is LumaHome's own (the shader's expressions are found in the QML, and the
numbers are read from theme.json), that the loop is periodic, that the stream is what the decoder needs (a key picture, then predicted copies, one
reference, no reordering, 10 bits), that the loop's seam is a fraction of a level, that the numbers the shell is given about the clip are what its
own sampler finds, and that the shipped set is small and consistent with its manifest.
"""
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "tools"))

try:
    import vplane_wall as vw
    import vplane_clips as vc
    from PIL import Image, ImageMath
    HAVE_PIL = hasattr(ImageMath, "lambda_eval")
except ImportError:
    HAVE_PIL = False

SHIPPED = os.path.join(ROOT, "payload", "vplane", "clips")
QML = os.path.join(ROOT, "payload", "compositor", "lumatv", "LumaHome.qml")
HOST_PLAYER = os.path.join(ROOT, "vplane", "build", "host", "lumatv-vplane")


def encoder_has_10_bits():
    if not shutil.which("ffmpeg"):
        return False
    r = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=s=64x64:d=0.1:r=10", "-pix_fmt", "yuv420p10le",
                        "-c:v", "libx265", "-x265-params", "log-level=error", "-f", "null", "-"], capture_output=True)
    return r.returncode == 0


HAVE_ENCODER = encoder_has_10_bits()


@unittest.skipUnless(HAVE_PIL, "needs Pillow 10.3 or newer")
class Warp(unittest.TestCase):
    GPU_RATES = {"x": 0.70, "y": 0.50, "z": 0.40, "w": 0.90, "v": 0.56}   # LumaHome.qml: phase = t * (0.70, 0.50, 0.40, 0.90); the third field 0.8 of the first

    def test_the_shaders_expressions_are_the_ones_the_generator_evaluates(self):
        """The clip is the GPU wallpaper's picture: if the shader moves, the clip is stale and this says so."""
        with open(QML) as f:
            q = f.read()
        for expr in ("sin(uv.y * 6.0 * k + phase.x) * cos(uv.x * 4.0 * k - phase.y)", "sin((uv.x + uv.y) * 5.0 * k - phase.z)",
                     "sin(uv.x * 11.0 * k + phase.w) * sin(uv.y * 9.0 * k - phase.x * 0.8)",
                     "vec2(w1 + 0.5 * w2, 0.7 * w2 - 0.5 * w1) + params.w * 0.5 * vec2(w3, -w3)", "vUv = uv + params.x * disp",
                     "vLight = 1.0 + params.y * (0.3 * (w1 - 0.5 * w2) + params.w * 0.25 * w3)",
                     "y < 0.55 ? mix(scrimA, scrimB, y / 0.55) : mix(scrimB, scrimC, (y - 0.55) / 0.45)", "c = mix(c, sc.rgb, sc.a)",
                     "GridMesh { resolution: Qt.size(64, 36) }", "Qt.vector4d(t * 0.70, t * 0.50, t * 0.40, t * 0.90)",
                     "property real amp: (root.wallMotion.amplitude || 4) / 1920", "texture2D(src, vUv).rgb * vLight + vAdd",
                     "mediump float crest = clamp(0.5 + 0.5 * (w1 + 0.4 * w3), 0.0, 1.0)", "look.y * 0.35 * crest * crest * crest * crest * crest * crest",
                     "highp vec2 q = (uv - 0.5) * vec2(1.0, 0.5625)", "vLight *= 1.0 - look.x * 0.75 * smoothstep(0.12, 0.42, dot(q, q))",
                     "max(texture2DLod(glow, vUv, 0.0).rgb - 0.3, 0.0) * look.z", "vAdd = vec3(spec) + bl"):
            self.assertIn(expr, q, expr)
        self.assertEqual(vw.GRID, (64, 36))

    def test_the_numbers_are_the_themes_own(self):
        theme = vw.read_theme()
        look = vw.look_of(theme)
        self.assertEqual(look["motion"], {"amplitude": 4, "speed": 1, "depth": 0.25, "scale": 1, "detail": 0.35, "vignette": 0, "specular": 0, "bloom": 0,
                                          "saturation": 1, "contrast": 1})
        self.assertEqual(look["scrim"], [0.1, 0.18, 0.46])
        self.assertEqual((look["material"], look["wallpaper"]), ("dark", "wall_1080.png"))
        self.assertTrue(os.path.isfile(vw.wallpaper_path(look["wallpaper"])))
        with self.assertRaises(SystemExit):
            vw.wallpaper_path("../theme/theme.json")

    def test_the_phases_are_whole_turns_near_the_gpus_rates(self):
        loop = vw.LOOP_SEC
        for name, turns in vw.TURNS.items():
            self.assertIsInstance(turns, int)
            rate = 2 * math.pi * turns / loop
            self.assertLess(abs(rate - self.GPU_RATES[name]) / self.GPU_RATES[name], 0.3, (name, rate))
        self.assertGreater(len({vw.TURNS["x"], vw.TURNS["y"]}), 1, "the first field's two phases differ, or half of it would stand still")
        self.assertGreater(len({vw.TURNS["w"], vw.TURNS["v"]}), 1)

    def test_the_loop_is_periodic_to_the_last_digit(self):
        look = vw.look_of(vw.read_theme())["motion"]
        for iy in range(0, 37, 6):
            for ix in range(0, 65, 8):
                a = vw.vertex(ix / 64, iy / 36, 0.0, look)
                b = vw.vertex(ix / 64, iy / 36, 2 * math.pi, look)
                self.assertTrue(all(abs(x - y) < 1e-9 for x, y in zip(a, b)), (ix, iy, a, b))
        # and it moves: a quarter of the loop is not the start
        moved = max(abs(vw.vertex(ix / 64, iy / 36, 0.0, look)[2] - vw.vertex(ix / 64, iy / 36, math.pi / 2, look)[2]) for ix in range(0, 65, 8) for iy in range(0, 37, 6))
        self.assertGreater(moved, 0.01)

    def test_the_drift_and_the_light_are_within_what_the_gpu_does(self):
        look = vw.look_of(vw.read_theme())["motion"]
        worst_shift, worst_light = 0.0, 0.0
        for k in range(0, 600, 7):
            theta = vw.loop_angle(k, 600)
            for iy in range(0, 37, 3):
                for ix in range(0, 65, 4):
                    ux, uy, light, _ = vw.vertex(ix / 64, iy / 36, theta, look)
                    worst_shift = max(worst_shift, abs(ux - ix / 64) * 1920, abs(uy - iy / 36) * 1920)
                    worst_light = max(worst_light, abs(light - 1.0))
        self.assertLessEqual(worst_shift, 4 * 2.0, "a few pixels (the amplitude 4, the fields add to under 2)")
        self.assertLessEqual(worst_light, 0.25 * (0.3 * 1.5 + 0.35 * 0.25) + 1e-9)

    def test_vignette_specular_and_bloom_are_the_shaders_terms(self):
        base = dict(vw.LOOK_DEFAULTS)
        tuned = dict(base, vignette=0.9, specular=0.5, bloom=0.25)
        # the vignette is on the undisplaced position: nothing at the centre, the shader's smoothstep at a corner
        c0, c1 = vw.vertex(0.5, 0.5, 0.0, base)[2], vw.vertex(0.5, 0.5, 0.0, tuned)[2]
        self.assertAlmostEqual(c0, c1, places=12)
        d = 0.5 ** 2 + (0.5 * 0.5625) ** 2
        t = (d - 0.12) / (0.42 - 0.12)
        fall = 1.0 - 0.9 * 0.75 * t * t * (3.0 - 2.0 * t)
        self.assertAlmostEqual(vw.vertex(0.0, 0.0, 1.3, tuned)[2] / vw.vertex(0.0, 0.0, 1.3, base)[2], fall, places=12)
        # the lobe is the sixth power of the crest, scaled by the look
        w1, w2, w3 = vw.fields(0.3, 0.6, 2.0, base)
        crest = min(max(0.5 + 0.5 * (w1 + 0.4 * w3), 0.0), 1.0)
        self.assertAlmostEqual(vw.vertex(0.3, 0.6, 2.0, tuned)[3], 0.5 * 0.35 * crest ** 6, places=12)
        self.assertEqual(vw.vertex(0.3, 0.6, 2.0, base)[3], 0.0)
        # the bloom is what a glow texture has over 0.3, times the look; a dim one adds nothing
        bright = Image.new("RGB", (960, 540), (204, 204, 204))
        dim = Image.new("RGB", (960, 540), (51, 51, 51))
        self.assertEqual(vw.glow_at(bright, 0.3, 0.4, 0.25), tuple([(0.8 - 0.3) * 0.25] * 3))
        self.assertEqual(vw.glow_at(dim, 0.3, 0.4, 0.25), (0.0, 0.0, 0.0))

    def test_a_tuned_look_is_baked_and_the_default_looks_picture_has_no_additive_part(self):
        base = Image.open(vw.wallpaper_path("wall_1080.png")).convert("RGB")
        plain = {"motion": dict(vw.LOOK_DEFAULTS), "scrim": [0.1, 0.18, 0.46], "material": "dark", "wallpaper": "wall_1080.png"}
        self.assertIsNone(vw.grid(0.4, plain["motion"], plain["scrim"])[2], "no specular and no bloom: nothing is added")
        vignette = dict(plain, motion=dict(vw.LOOK_DEFAULTS, vignette=0.9))
        a, b = vw.render(base, 0.4, plain), vw.render(base, 0.4, vignette)
        self.assertLess(sum(b.getpixel((4, 4))), sum(a.getpixel((4, 4))), "the vignette darkens the corner")
        self.assertEqual(b.getpixel((960, 540)), a.getpixel((960, 540)), "and leaves the centre")
        bloom = vw.render(base, 0.4, dict(plain, motion=dict(vw.LOOK_DEFAULTS, bloom=0.25)))
        self.assertGreater(sum(bloom.getpixel((960, 540))), sum(a.getpixel((960, 540))), "the bloom adds the glow's light")

    def test_where_the_warp_pulls_the_edge_in_the_picture_repeats_its_edge_pixels_and_is_not_black(self):
        flat = Image.new("RGB", vw.SCREEN, (100, 100, 100))
        look = {"motion": dict(vw.LOOK_DEFAULTS, amplitude=12, depth=0), "scrim": [0.0, 0.0, 0.0], "material": "dark", "wallpaper": "wall_1080.png"}
        for theta in (0.0, 1.0, 2.5, 4.0):
            for band in vw.render_float(flat, theta, look):
                self.assertEqual(band.getextrema(), (100.0, 100.0), theta)

    def test_a_look_of_its_own_is_laid_over_the_themes_and_is_named_by_an_id_of_its_own(self):
        theme = vw.read_theme()
        self.assertEqual(vw.look_of(theme, {"amplitude": 12, "vignette": 0.9})["motion"]["amplitude"], 12)
        self.assertEqual(vw.look_of(theme, {"amplitude": 12})["motion"]["depth"], 0.25, "what is not given is the theme's")
        with self.assertRaises(SystemExit):
            vw.look_of(theme, {"amplitude": 12, "colour": 3})

    def test_with_nothing_to_apply_the_picture_is_the_wallpaper(self):
        base = Image.open(vw.wallpaper_path("wall_1080.png")).convert("RGB")
        flat = {"motion": dict(vw.LOOK_DEFAULTS, amplitude=0, depth=0), "scrim": [0.0, 0.0, 0.0], "material": "dark", "wallpaper": "wall_1080.png"}
        out = vw.render(base, 1.234, flat)
        from PIL import ImageChops
        d = ImageChops.difference(base, out).resize((192, 108), Image.BOX)
        mean = sum(sum(px) for px in d.getdata()) / (192 * 108 * 3)
        self.assertLess(mean, 0.6, "the mesh and the multiplier leave a picture alone")

    def test_the_scrim_is_the_qmls_black_gradient_over_the_picture(self):
        base = Image.open(vw.wallpaper_path("wall_1080.png")).convert("RGB")
        look = vw.look_of(vw.read_theme())
        flat = dict(look, motion=dict(look["motion"], amplitude=0, depth=0))
        out = vw.render(base, 0.0, flat)
        from PIL import ImageStat
        for y, alpha in ((4, 0.1), (594, 0.18), (1076, 0.46)):
            row = (0, y, 1920, y + 2)
            got = ImageStat.Stat(out.crop(row)).mean
            ref = ImageStat.Stat(base.crop(row)).mean
            for g, r in zip(got, ref):
                self.assertAlmostEqual(g / r, 1 - vw.scrim_alpha((y + 1) / 1080, look["scrim"]), delta=0.02, msg=(y, alpha))


@unittest.skipUnless(HAVE_PIL, "needs Pillow 10.3 or newer")
class Numbers(unittest.TestCase):
    def test_the_tint_port_is_tint_js(self):
        """The manifest's tint and luma are Tint.js's own method: both run on the same bytes."""
        if not shutil.which("bun"):
            self.skipTest("needs bun")
        import random
        rng = random.Random(7)
        data = bytes(v for _ in range(32 * 18) for v in (rng.choice((12, 30, 70, 90, 120, 200)), rng.choice((10, 40, 80, 150)), rng.choice((20, 60, 100, 220)), 255 if rng.random() > 0.05 else 0))
        script = ("import { readFileSync } from 'node:fs';"
                  "const src = readFileSync(%r, 'utf8').replace(/^\\.pragma library$/m, '');"
                  "const T = new Function(src + '\\nreturn { dominant, luma };')();"
                  "const d = new Uint8Array(JSON.parse(process.env.TINT_DATA));"
                  "console.log(JSON.stringify({ dominant: T.dominant(d, 576), luma: T.luma(d, 576) }));") % os.path.join(ROOT, "payload", "compositor", "lumatv", "Tint.js")
        out = subprocess.run(["bun", "-e", script], capture_output=True, text=True, timeout=60, env=dict(os.environ, TINT_DATA=json.dumps(list(data))))
        self.assertEqual(out.returncode, 0, out.stderr)
        js = json.loads(out.stdout)
        py = vw.dominant_of(list(data), 576)
        self.assertEqual({k: js["dominant"][k] for k in ("r", "g", "b")}, {k: py[k] for k in ("r", "g", "b")})
        self.assertAlmostEqual(js["dominant"]["share"], py["share"], places=9)
        self.assertAlmostEqual(js["luma"], vw.luma_of(list(data), 576), places=9)

    def test_the_manifests_numbers_are_what_the_sampler_finds_in_the_wallpaper(self):
        w = json.load(open(os.path.join(SHIPPED, "wall.json")))["walls"][0]
        base = Image.open(vw.wallpaper_path(w["source"]["file"])).convert("RGB").resize(vw.SAMPLE, Image.BOX)
        luma, tint = vw.sample_of(base)
        self.assertAlmostEqual(w["tint"]["luma"], luma, delta=0.01, msg="the clip's pictures, scrim aside, are the wallpaper in slow motion")
        self.assertEqual(w["tint"]["tint"], vw.hexc(tint))
        self.assertLess(w["tint"]["lumaMax"] - w["tint"]["lumaMin"], 0.02, "the picture's brightness hardly moves: the glass material does not flip")
        self.assertEqual([p["tint"] for p in w["tint"]["posters"]].count(w["tint"]["tint"]), len(w["tint"]["posters"]))

    def test_the_glass_numbers_are_contrast_at_the_distance_rule(self):
        theme = vw.read_theme()
        g = json.load(open(os.path.join(SHIPPED, "wall.json")))["walls"][0]["glass"]
        self.assertGreaterEqual(g["alpha"], theme["dark"]["readingAlpha"])
        self.assertGreaterEqual(g["ratio"], 7.0)
        # recomputed from the numbers: the ink over the pane over the brightest the clip gets, at that alpha
        ink, _ = vc.hexrgb(theme["dark"]["ink"])
        pane, _ = vc.hexrgb(g["pane"])
        grey = vw.grey_of_luminance(g["worstLuminance"])
        self.assertAlmostEqual(vw.contrast(ink, vw.over(pane, g["alpha"], (grey, grey, grey))), g["ratio"], delta=0.05)
        # one step less opaque would not read (unless it is already the material's floor)
        if g["alpha"] - 0.02 >= theme["dark"]["readingAlpha"] - 1e-9:
            self.assertLess(vw.contrast(ink, vw.over(pane, g["alpha"] - 0.02, (grey, grey, grey))), 7.0)

    def test_the_least_alpha_is_found_and_a_darker_clip_needs_less(self):
        theme = vw.read_theme()
        bright = vw.glass_numbers(theme, 0.9, (120, 120, 140))
        dim = vw.glass_numbers(theme, 0.05, (30, 30, 40))
        self.assertGreater(bright["alpha"], dim["alpha"] - 1e-9)
        self.assertGreaterEqual(dim["alpha"], theme["dark"]["readingAlpha"], "never below the material's own reading alpha")
        self.assertGreaterEqual(bright["ratio"], 0)


@unittest.skipUnless(HAVE_PIL, "needs Pillow 10.3 or newer")
class Shipped(unittest.TestCase):
    def manifest(self):
        with open(os.path.join(SHIPPED, "wall.json")) as f:
            return json.load(f)

    def test_the_shipped_set_checks_and_the_prints_are_not_disturbed(self):
        self.assertEqual(vw.check(SHIPPED), [])
        self.assertEqual(vc.check(SHIPPED), [], "the wallpaper's clip is not one of the prints")

    def test_a_clip_whose_look_or_picture_or_file_moved_is_reported(self):
        with tempfile.TemporaryDirectory() as d:
            for f in ("wall.json", "wall-silk.vpc"):
                shutil.copy(os.path.join(SHIPPED, f), d)
            doc = self.manifest()
            doc["walls"][0]["look"]["motion"]["amplitude"] = 9
            with open(os.path.join(d, "wall.json"), "w") as f:
                json.dump(doc, f)
            bad = vw.check(d)
            self.assertTrue(any("look moved" in b and "amplitude" in b for b in bad), bad)
            shutil.copy(os.path.join(SHIPPED, "wall.json"), d)
            with open(os.path.join(d, "wall-silk.vpc"), "ab") as f:
                f.write(b"x")
            self.assertTrue(any("not the file wall.json was written for" in b for b in vw.check(d)))
            os.remove(os.path.join(d, "wall-silk.vpc"))
            self.assertTrue(any("file missing" in b for b in vw.check(d)))
        with tempfile.TemporaryDirectory() as empty:
            self.assertEqual(vw.check(empty), ["%s: no wall.json" % empty])

    def test_the_container_is_the_players_and_the_loop_returns_to_the_key_unit(self):
        w = self.manifest()["walls"][0]
        v = vc.read_vpc(os.path.join(SHIPPED, w["file"]))
        self.assertEqual((v["codec"], v["width"], v["height"], v["fps"], v["loopFrom"]), ("hevc", 1920, 1080, 60, 0))
        self.assertEqual(len(v["units"]), 30 * 60)
        self.assertTrue(v["units"][0][1] and not any(k for _, k in v["units"][1:]))
        self.assertEqual(w["units"], len(v["units"]))

    def test_it_is_small(self):
        w = self.manifest()["walls"][0]
        self.assertLess(w["bytes"], 1_500_000, "a thirty second loop that is a few hundred bytes a picture")
        self.assertLess(w["keyBytes"], 40_000)
        total = sum(os.path.getsize(os.path.join(SHIPPED, f)) for f in os.listdir(SHIPPED))
        self.assertLess(total, 4_000_000)

    @unittest.skipUnless(os.path.exists(HOST_PLAYER), "needs the host build of the player (sh vplane/build.sh)")
    def test_the_player_reads_it(self):
        out = subprocess.run([HOST_PLAYER, "--vpc-info", os.path.join(SHIPPED, "wall-silk.vpc")], capture_output=True, text=True, timeout=30)
        info = json.loads(out.stdout)
        self.assertEqual((info["codec"], info["width"], info["height"], info["fpsNum"], info["fpsDen"], info["units"], info["loopFrom"]), (2, 1920, 1080, 60, 1, 1800, 0))

    @unittest.skipUnless(HAVE_ENCODER and shutil.which("ffprobe"), "needs ffmpeg with 10-bit libx265 and ffprobe")
    def test_the_stream_decodes_as_main10_one_reference_no_reordering_and_the_loop_has_no_seam(self):
        """Deep: ffprobe decodes the whole stream (the count, the size, the profile, no reordering) and then two plays of it, and the step across
        the loop's seam is a fraction of a level."""
        self.assertEqual(vw.check(SHIPPED, deep=True), [])
        w = self.manifest()["walls"][0]
        v = vc.read_vpc(os.path.join(SHIPPED, w["file"]))
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "s.bin")
            with open(path, "wb") as f:
                f.write(b"".join(u for u, _ in v["units"][:90]))
            out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=profile,pix_fmt,width,height", "-of", "json", "-f", "hevc", path],
                                 capture_output=True, text=True)
            s = json.loads(out.stdout)["streams"][0]
        self.assertEqual((s["profile"], s["pix_fmt"], s["width"], s["height"]), ("Main 10", "yuv420p10le", 1920, 1080))


@unittest.skipUnless(HAVE_PIL and HAVE_ENCODER, "needs Pillow 10.3, and ffmpeg with 10-bit libx265")
class Encode(unittest.TestCase):
    def run_wall(self, *extra):
        d = tempfile.mkdtemp(prefix="wall-gen-")
        self.addCleanup(shutil.rmtree, d, True)
        r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "vplane_wall.py"), "wall", "--out", d, "--frames-only", "4", "--jobs", "2", *extra],
                           capture_output=True, text=True, timeout=600, env=dict(os.environ, VPLANE_QUIET="1"))
        self.assertEqual(r.returncode, 0, r.stderr[-2000:])
        return d

    def test_a_few_pictures_become_a_key_unit_and_predicted_copies_in_the_players_container(self):
        d = self.run_wall()
        w = json.load(open(os.path.join(d, "wall.json")))["walls"][0]
        v = vc.read_vpc(os.path.join(d, w["file"]))
        self.assertEqual(len(v["units"]), 4 * 3, "each picture shown three times: 20 a second at the stream's 60")
        self.assertTrue(v["units"][0][1] and not any(k for _, k in v["units"][1:]))
        self.assertEqual((v["width"], v["height"], v["fps"]), (1920, 1080, 60))
        self.assertEqual(w["opts"]["bits"], 10)
        self.assertEqual(w["opts"]["qp"], 18)
        deep = vc.deep_check("x", v)
        self.assertEqual(deep, [], "the stream decodes: its size and count, one reference, no reordering")
        # it is the loop's pictures only in part: the check says so (a loop of 1800 units is the contract), and says nothing else
        bad = vw.check(d)
        self.assertEqual([b for b in bad if "units for a loop" not in b], [])

    def test_regenerating_gives_the_same_bytes_and_reuse_keeps_the_file(self):
        a = self.run_wall()
        b = self.run_wall()
        with open(os.path.join(a, "wall-silk.vpc"), "rb") as f1, open(os.path.join(b, "wall-silk.vpc"), "rb") as f2:
            self.assertEqual(f1.read(), f2.read())
        before = os.stat(os.path.join(a, "wall-silk.vpc")).st_mtime_ns
        r = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "vplane_wall.py"), "wall", "--out", a, "--frames-only", "4", "--jobs", "2", "--reuse"],
                           capture_output=True, text=True, timeout=600)
        self.assertEqual(r.returncode, 0, r.stderr[-1000:])
        self.assertEqual(os.stat(os.path.join(a, "wall-silk.vpc")).st_mtime_ns, before, "the clip file was not written again")

    def test_the_eight_bit_clip_is_main_and_smaller_steps_are_not_claimed(self):
        d = self.run_wall("--bit-depth", "8", "--id", "eight")
        w = [x for x in json.load(open(os.path.join(d, "wall.json")))["walls"] if x["id"] == "eight"][0]
        self.assertEqual((w["opts"]["bits"], w["opts"]["qp"]), (8, 16))
        v = vc.read_vpc(os.path.join(d, w["file"]))
        with tempfile.TemporaryDirectory() as t:
            path = os.path.join(t, "s.bin")
            with open(path, "wb") as f:
                f.write(b"".join(u for u, _ in v["units"]))
            out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=profile,pix_fmt", "-of", "json", "-f", "hevc", path], capture_output=True, text=True)
        s = json.loads(out.stdout)["streams"][0]
        self.assertEqual((s["profile"], s["pix_fmt"]), ("Main", "yuv420p"))

    def test_a_look_the_clip_cannot_bake_is_refused(self):
        import argparse
        from unittest import mock
        theme = vw.read_theme()
        args = argparse.Namespace(out=tempfile.mkdtemp(), id="x", loop_sec=30, content_fps=20, bit_depth=10, qp=0, jobs=1, frames_only=1, reuse=False, look="")
        self.addCleanup(shutil.rmtree, args.out, True)
        bent = json.loads(json.dumps(theme))
        bent["wallpaperMotion"]["speed"] = 2                      # the pace is the clip's own: the only setting a clip cannot follow
        with mock.patch.object(vw, "read_theme", return_value=bent), self.assertRaises(SystemExit) as cm:
            vw.cmd_wall(args)
        self.assertIn("speed", str(cm.exception))
        for look, why in (('{"amplitude": 12}', "id of its own"), ("not json", "JSON"), ("{}", "at least one")):
            args.id = "silk" if why == "id of its own" else "x"
            args.look = look
            with self.assertRaises(SystemExit) as cm:
                vw.cmd_wall(args)
            self.assertIn(why, str(cm.exception), look)

    def test_a_wallpaper_that_is_not_the_packages_is_refused(self):
        with self.assertRaises(SystemExit):
            vw.wallpaper_path("not-there.png")


EVIDENCE = os.path.join(ROOT, "docs", "research", "compositor")
GPU_CAPTURE = os.path.join(EVIDENCE, "slot-2026-10-28-1300", "wallpaper", "home-gpu-look.png")          # LG's DISPLAY capture, the GPU wallpaper, the owner's look
PLANE_CAPTURE = os.path.join(EVIDENCE, "slot-2026-10-28-r14", "img", "wall-video-home.png")             # round 14: the video wallpaper's capture


@unittest.skipUnless(HAVE_PIL and shutil.which("ffmpeg") and os.path.exists(GPU_CAPTURE), "needs Pillow, numpy, ffmpeg and the evidence captures")
class Probe(unittest.TestCase):
    """`vplane_wall.py probe` on what the set captured. The GPU wallpaper that the TV drew in round 13 (the owner's look) is the baked clip's picture, which
    is the check of the bake against the real thing: the same structure (correlation 0.997 at 22.6 s of the loop, at the same place on the screen, so
    no scaling or shift) and the same light to within 2 to 7 percent. Round 14's capture is black wherever Home draws nothing, margin and gaps between
    its panes alike (so not a picture 1856 x 1024 in a window: the gaps are inside it): the plane shows nothing in it, or the capture cannot see it."""

    @classmethod
    def setUpClass(cls):
        try:
            import numpy  # noqa: F401
        except ImportError:
            raise unittest.SkipTest("needs numpy")

    def test_the_bake_is_the_gpu_wallpaper_the_tv_drew(self):
        r = vw.probe(GPU_CAPTURE, SHIPPED, "silk-tuned")
        self.assertEqual(r["verdict"], "ok", r)
        self.assertGreaterEqual(r["corr"], 0.99)
        self.assertTrue(22.0 <= r["t"] <= 23.2, r)
        self.assertTrue(all(0.95 <= x <= 1.10 for x in r["ratio"]), r)

    def test_the_clip_of_the_other_look_is_not_that_picture(self):
        self.assertLess(vw.probe(GPU_CAPTURE, SHIPPED, "silk")["corr"], vw.probe(GPU_CAPTURE, SHIPPED, "silk-tuned")["corr"])

    def test_round_14s_capture_is_black_where_the_plane_should_show(self):
        if not os.path.exists(PLANE_CAPTURE):
            self.skipTest("no round 14 capture")
        r = vw.probe(PLANE_CAPTURE, SHIPPED)
        self.assertEqual((r["verdict"], r["margin"]), ("black", 0.0), r)
        im = Image.open(PLANE_CAPTURE).convert("RGB")
        w, h = im.size
        black = lambda x, y: im.getpixel((x, y)) == (0, 0, 0)
        # the black is the whole of the screen that Home does not draw on: the margin, and the gaps between the panes (rows 60-64 and 310-314, the column
        # 478-484 of the 960 x 540 capture), which are inside any picture the margin could be the edge of
        self.assertTrue(all(black(x, y) for x in range(0, w, 20) for y in (3, 20)))
        self.assertTrue(all(black(x, y) for x in range(40, 920, 40) for y in (60, 62, 64, 310, 312, 314)))
        self.assertTrue(all(black(x, y) for x in (479, 481, 483) for y in range(80, 300, 20)))

    def test_a_darker_plane_is_a_colour_finding_and_not_a_geometry_one(self):
        with tempfile.TemporaryDirectory() as d:
            dark = os.path.join(d, "dark.png")
            Image.eval(Image.open(GPU_CAPTURE).convert("RGB"), lambda v: int(v * 0.8)).save(dark)
            r = vw.probe(dark, SHIPPED, "silk-tuned")
        self.assertEqual(r["verdict"], "colour", r)
        self.assertGreaterEqual(r["corr"], 0.98)
        self.assertTrue(all(0.78 <= x <= 0.92 for x in r["ratio"]), r)

if __name__ == "__main__":
    unittest.main()
