"""Run with: python3 -m unittest tests.test_audio_format   (needs node; skipped without it)

kit/AudioFormat.js turns what the Denon says about the signal it plays (`SYSDA`, the helper's `signal`) and its sound mode (`MS`, `soundMode`) into the
short words the volume bar shows, and into nothing when they say nothing: never a raw code. Read on the set (S760H, 2026-10-25): "PCM ZERO" with
"DOLBY AUDIO-DSUR" while nothing played. The other texts are the receiver's published naming, not yet seen on the set.
"""
import json
import os
import shutil
import subprocess
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JS = os.path.join(ROOT, "payload", "compositor", "lumatv", "kit", "AudioFormat.js")
NODE = shutil.which("node")
RUN = ("const fs=require('fs'); const src=fs.readFileSync(process.argv[1],'utf8').replace(/^\\.pragma.*$/m,''); "
       "const m=new Function(src+'; return {describe, logo}')(); console.log(JSON.stringify(JSON.parse(process.argv[2]).map(c=>m[process.argv[3]](c[0],c[1]))))")


def call(fn, cases):
    out = subprocess.run([NODE, "-e", RUN, JS, json.dumps(cases), fn], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def describe(cases):
    return call("describe", cases)


def logo(cases):
    return call("logo", cases)


@unittest.skipUnless(NODE, "needs node")
class AudioFormat(unittest.TestCase):
    def test_what_the_set_said_while_nothing_played_shows_nothing(self):
        self.assertEqual(describe([["DOLBY AUDIO-DSUR", "PCM ZERO"], ["DOLBY AUDIO-DSUR", "PCM ZERO             "]]), ["", ""])

    def test_the_formats_and_the_mode_word_after_them(self):
        got = describe([
            ["DOLBY AUDIO-DD+DSUR", "DOLBY D+ 5.1"], ["DOLBY ATMOS", "DOLBY ATMOS"], ["DOLBY AUDIO-DSUR", "DOLBY TRUEHD 7.1"],
            ["DTS:X", "DTS:X"], ["STEREO", "PCM 2.0"], ["MCH STEREO", "MULTI CH IN"], ["DIRECT", "DOLBY D 5.1"], ["STEREO", "DTS-HD MA 5.1"],
        ])
        self.assertEqual(got, ["Dolby Digital+ \u00b7 Surround", "Dolby Atmos", "Dolby TrueHD \u00b7 Surround", "DTS:X", "PCM 2.0", "Multi Ch In",
                               "Dolby Digital \u00b7 Direct", "DTS-HD \u00b7 Stereo"])

    def test_what_the_set_said_over_netflix_is_the_format_alone(self):
        # read on the set 2026-10-26: signal 'Dolby Atmos - DD+', soundMode 'DOLBY ATMOS'
        got = describe([["DOLBY ATMOS", "Dolby Atmos - DD+"], ["DOLBY ATMOS", "Dolby Atmos - TrueHD"], ["DOLBY ATMOS", "Dolby Atmos - Dolby TrueHD"],
                        ["DOLBY ATMOS", "DD+"], ["DOLBY ATMOS", "Dolby Digital+"], ["DOLBY ATMOS", "Dolby Atmos - DD+ "]])
        self.assertEqual(got, ["Dolby Atmos"] * 6)

    def test_other_denon_spellings_of_the_formats(self):
        got = describe([
            ["DTS:X", "DTS:X"], ["DTS NEURAL:X", "DTS:X MSTR"], ["STEREO", "DTS:X Master"],
            ["DOLBY SURROUND", "Dolby Digital+"], ["DOLBY DIGITAL+", "Dolby Digital +"], ["DOLBY SURROUND", "Dolby Digital Plus"],
            ["STEREO", "PCM 2.0ch"], ["STEREO", "PCM 2ch"], ["STEREO", "PCM 5.1ch"], ["STEREO", "LPCM 2.0ch"], ["DOLBY ATMOS", "PCM 2.0ch"],
            ["DOLBY SURROUND", "DTS-HD MSTR"], ["DTS SURROUND", "DTS"], ["DOLBY SURROUND", "Dolby TrueHD"],
        ])
        self.assertEqual(got, ["DTS:X", "DTS:X", "DTS:X", "Dolby Digital+ \u00b7 Surround", "Dolby Digital+", "Dolby Digital+ \u00b7 Surround",
                               "PCM 2.0", "PCM 2.0", "PCM 5.1", "PCM 2.0", "PCM 2.0", "DTS-HD \u00b7 Surround", "DTS", "Dolby TrueHD \u00b7 Surround"])

    def test_no_raw_text_of_a_carrier_or_code_shows(self):
        for out in describe([["DOLBY ATMOS", "Dolby Atmos - DD+"], ["DOLBY ATMOS", "Dolby Atmos - TrueHD"], ["STEREO", "PCM 2.0ch"], ["DTS:X", "DTS:X MSTR"]]):
            self.assertNotRegex(out, r" - |ch$|MSTR|\+\+")

    def test_what_it_cannot_name_is_not_shown_and_a_raw_code_never_is(self):
        self.assertEqual(describe([["DOLBY AUDIO-DD+DSUR", ""], ["", "XYZZY 13"], [None, None], ["STEREO", "13"], ["MS?", "SSINFAISSIG 13"]]), ["", "", "", "", ""])

    def test_the_mark_of_each_format_and_none_for_the_rest(self):
        got = logo([
            ["DOLBY ATMOS", "Dolby Atmos - DD+"], ["DOLBY ATMOS", "DD+"], ["DOLBY ATMOS", "Dolby Atmos - TrueHD"],
            ["DOLBY AUDIO-DD+DSUR", "DOLBY D+ 5.1"], ["DOLBY AUDIO-DSUR", "DOLBY TRUEHD 7.1"], ["DIRECT", "DOLBY D 5.1"],
            ["DTS:X", "DTS:X"], ["DTS NEURAL:X", "DTS:X MSTR"],
            ["DTS SURROUND", "DTS"], ["STEREO", "DTS-HD MA 5.1"], ["DOLBY SURROUND", "DTS-HD MSTR"],
        ])
        self.assertEqual(got, ["atmos", "atmos", "atmos", "dolby", "dolby", "dolby", "dtsx", "dtsx", "dts", "dts", "dts"])

    def test_pcm_stereo_multichannel_and_what_is_not_known_have_no_mark(self):
        got = logo([["STEREO", "PCM 2.0"], ["STEREO", "PCM 2.0ch"], ["DOLBY ATMOS", "PCM 2.0ch"], ["DOLBY AUDIO-DSUR", "PCM 5.1"], ["MCH STEREO", "MULTI CH IN"],
                    ["DOLBY AUDIO-DSUR", "PCM ZERO"], ["STEREO", ""], ["", "XYZZY 13"], [None, None]])
        self.assertEqual(got, [""] * 9)

    def test_the_marks_exist_as_files_the_size_the_index_says(self):
        from PIL import Image
        folder = os.path.join(ROOT, "payload", "compositor", "lumatv", "icons", "audio")
        with open(os.path.join(folder, "index.json")) as f:
            index = json.load(f)
        keys = {k for k in logo([["DOLBY ATMOS", "Dolby Atmos - DD+"], ["DOLBY AUDIO-DD+DSUR", "DOLBY D+ 5.1"], ["DTS:X", "DTS:X"], ["DTS SURROUND", "DTS"]])}
        self.assertEqual(sorted(index), sorted(keys))
        for key, size in index.items():
            image = Image.open(os.path.join(folder, key + ".png")).convert("RGBA")
            self.assertEqual(image.size, (size["width"], size["height"]), key)
            colours = {px[:3] for px in image.getdata() if px[3] > 0}
            self.assertEqual(colours, {(255, 255, 255)}, key + ": one colour, white, the alpha carries the shape")


if __name__ == "__main__":
    unittest.main()
