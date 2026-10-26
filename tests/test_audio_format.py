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
       "const m=new Function(src+'; return {describe}')(); console.log(JSON.stringify(JSON.parse(process.argv[2]).map(c=>m.describe(c[0],c[1]))))")


def describe(cases):
    out = subprocess.run([NODE, "-e", RUN, JS, json.dumps(cases)], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


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

    def test_what_it_cannot_name_is_not_shown_and_a_raw_code_never_is(self):
        self.assertEqual(describe([["DOLBY AUDIO-DD+DSUR", ""], ["", "XYZZY 13"], [None, None], ["STEREO", "13"], ["MS?", "SSINFAISSIG 13"]]), ["", "", "", "", ""])


if __name__ == "__main__":
    unittest.main()
