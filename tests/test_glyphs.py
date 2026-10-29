"""Run with: python3 -m unittest tests.test_glyphs

assets/glyphs (generators/gen-glyphs.py): the remote-key and action glyphs hints are drawn with (Lucent, "Hints are glyphs, not sentences"). The
files are the generator's output, each is a 24-unit toolkit-neutral SVG with one accessible name, none draws a word (the Magic Remote has no OK
or Back lettering), and the index lists exactly the files there.
"""
import importlib.util
import json
import os
import unittest
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR = os.path.join(ROOT, "assets", "glyphs")
NS = "{http://www.w3.org/2000/svg}"

spec = importlib.util.spec_from_file_location("gen_glyphs", os.path.join(ROOT, "generators", "gen-glyphs.py"))
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def index():
    with open(os.path.join(DIR, "index.json"), encoding="utf-8") as f:
        return json.load(f)


class Glyphs(unittest.TestCase):
    def test_the_files_are_what_the_generator_writes(self):
        files, idx = gen.build()
        for name, text in files.items():
            with open(os.path.join(DIR, name), encoding="utf-8") as f:
                self.assertEqual(f.read(), text, name + " is stale: run generators/gen-glyphs.py")
        self.assertEqual(index()["glyphs"], idx["glyphs"])
        self.assertEqual(sorted(n for n in os.listdir(DIR) if n.endswith(".svg")), sorted(files))

    def test_every_glyph_is_a_24_unit_svg_with_one_name_and_no_words(self):
        for key, g in index()["glyphs"].items():
            root = ET.parse(os.path.join(DIR, g["file"])).getroot()
            self.assertEqual(root.get("viewBox"), "0 0 24 24", key)
            self.assertEqual(root.get("role"), "img", key)
            self.assertTrue(g["name"], key)
            self.assertEqual(root.get("aria-label"), g["name"], key)
            self.assertEqual(root.find(NS + "title").text, g["name"], key)
            self.assertEqual(list(root.iter(NS + "text")), [], key + ": a glyph draws no words")
            self.assertEqual(list(root.iter(NS + "image")), [], key)

    def test_the_set_covers_the_magic_remote_keys_hints_use(self):
        have = set(index()["glyphs"])
        for k in ("key-ok", "key-ok-hold", "key-up", "key-down", "key-left", "key-right", "key-back", "key-home", "key-menu", "key-mic",
                  "key-red", "key-green", "key-yellow", "key-blue", "expand", "collapse", "close", "hold"):
            self.assertIn(k, have)

    def test_a_colour_key_names_its_palette_tone_and_takes_no_colour_of_its_own(self):
        for k, tone in (("key-red", "danger"), ("key-green", "positive"), ("key-yellow", "warning"), ("key-blue", "info")):
            self.assertEqual(index()["glyphs"][k]["tone"], tone)
            with open(os.path.join(DIR, k + ".svg"), encoding="utf-8") as f:
                text = f.read()
            self.assertNotIn("#", text)
            self.assertIn('data-tone="%s"' % tone, text)

    def test_the_generator_is_repeatable(self):
        files, _ = gen.build()
        again, _ = gen.build()
        self.assertEqual(files, again)


if __name__ == "__main__":
    unittest.main()
