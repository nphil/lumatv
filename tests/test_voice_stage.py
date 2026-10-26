"""Run with: python3 -m unittest tests.test_voice_stage

tools/voice.sh: LG's Search app's overlay, run by tools/inputcommon.sh with this app's names. What matters here is what differs from the input
banners': the tree (Main.qml and Style.qml), its own stage directory, the patch it runs, the dump it sets aside (voice, and never voiceweb or
voiceagent, or the input banners' own dump) and the answers it prints. The staging and the cold start themselves are tests.test_inputcommon_stage's.
"""
import json
import os
import shutil
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import test_inputcommon_stage as ics  # noqa: E402

ROOT = ics.ROOT
SCRIPT = os.path.join(ROOT, "tools", "voice.sh")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/voice/qml", "/tmp/tvsrc/apps/com.webos.app.voice/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/voice/qml")
VOICE = "com.webos.app.voice"
ASIDE = ".lumatv-aside-voice."


class VoiceBase(ics.Base):
    def setUp(self):
        super().setUp()
        env = dict(self.env)
        del env["INPUTCOMMON_QML"]
        env["VOICE_QML"] = self.qml
        self.env = env
        # the dumps beside the voice app's: the other apps whose names start the same, and the input banners'
        for d in (VOICE, "com.webos.app.voiceweb", "com.webos.app.voiceagent", "com.webos.app.inputcommon"):
            if not os.path.isdir(os.path.join(self.criu, d)):
                os.makedirs(os.path.join(self.criu, d))
                self.write(os.path.relpath(os.path.join(self.criu, d, "pgid"), self.t), "1\n")

    def run_script(self, verb, **env):
        r = subprocess.run(["sh", SCRIPT, verb], env=dict(self.env, **env), capture_output=True, text=True)
        return r.returncode, json.loads(r.stdout.strip().splitlines()[-1])

    def aside(self, name=VOICE):
        return os.path.isdir(os.path.join(self.criu, ASIDE + name))


@unittest.skipUnless(os.path.isdir(CENSUS), "needs the census copy of LG's Search tree")
class VoiceStaging(VoiceBase):
    def make_qml(self):
        shutil.copytree(CENSUS, self.qml)

    def staged(self):
        up = os.path.join(self.state, "voice", "upper")
        return sorted(os.path.relpath(os.path.join(b, n), up) for b, _, ns in os.walk(up) for n in ns) if os.path.isdir(up) else []

    def test_stage_keeps_only_the_files_the_patch_rewrote_in_a_directory_of_its_own_and_leaves_LGs_tree_alone(self):
        before = ics.tree_state(self.qml)
        rc, out = self.run_script("stage")
        self.assertEqual((rc, out["ok"], out["staged"]), (0, True, True))
        from test_voice_patch import TOUCHED
        self.assertEqual(self.staged(), TOUCHED)
        self.assertEqual(ics.tree_state(self.qml), before)
        self.assertFalse(os.path.exists(os.path.join(self.state, "inputcommon")))
        self.assertEqual(self.calls(), [])

    def test_on_mounts_the_overlay_named_voice_sets_only_the_voice_dump_aside_and_closes_the_app_when_it_is_idle(self):
        self.running(VOICE, "netflix")
        self.write("sam/foreground", "netflix")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["voice"], out["checkpointAside"], out["closed"]), (0, "on", 1, [VOICE]))
        self.assertTrue(any(c.startswith("mount -t overlay voice -o lowerdir=%s," % self.qml) for c in self.calls()), self.calls())
        self.assertTrue(self.aside())
        self.assertFalse(self.dump_in_place(VOICE))
        # the lookalikes, the input banners' and Home's dumps are where they were
        for name in ("com.webos.app.voiceweb", "com.webos.app.voiceagent", "com.webos.app.inputcommon", "com.webos.app.home"):
            self.assertTrue(self.dump_in_place(name), name)
        self.assertEqual(len(self.closes()), 1)
        self.assertEqual(self.sam_calls().count("closeByAppId {\"id\":\"%s\"}" % VOICE), 1)

    def test_the_app_on_screen_is_not_closed(self):
        self.running(VOICE)
        self.write("sam/foreground", VOICE)
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["closed"], out["onScreen"]), (0, [], VOICE))

    def test_off_puts_back_the_voice_dump_only_and_never_the_input_banners(self):
        self.run_script("on")
        # the input banners' own copy, set aside by their script, stays where it is
        os.rename(os.path.join(self.criu, "com.webos.app.inputcommon"), os.path.join(self.criu, ".lumatv-aside.com.webos.app.inputcommon"))
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["voice"], out["checkpointBack"]), (0, "off", 1))
        self.assertTrue(self.dump_in_place(VOICE))
        self.assertFalse(self.aside())
        self.assertTrue(os.path.isdir(os.path.join(self.criu, ".lumatv-aside.com.webos.app.inputcommon")))
        self.assertFalse(self.dump_in_place("com.webos.app.inputcommon"))

    def test_status_names_this_apps_stage_and_mount(self):
        self.run_script("stage")
        rc, out = self.run_script("status")
        self.assertEqual((rc, out["ok"], out["staged"], out["mounted"], out["qml"]), (0, True, True, False, self.qml))
        self.run_script("on")
        self.assertTrue(self.run_script("status")[1]["mounted"])

    def test_a_tree_that_is_not_the_search_app_is_not_found(self):
        os.unlink(os.path.join(self.qml, "Main.qml"))
        rc, out = self.run_script("stage")
        self.assertEqual(rc, 1)
        self.assertIn("LG's voice QML was not found (set VOICE_QML)", out["error"])

    def test_a_patch_that_refuses_leaves_everything_as_it_was(self):
        with open(os.path.join(self.qml, "Style.qml"), "a") as f:
            f.write("// Luma: patched\n")
        rc, out = self.run_script("on")
        self.assertEqual(rc, 1)
        self.assertFalse(out["ok"])
        self.assertEqual(self.calls(), [])
        self.assertTrue(self.dump_in_place(VOICE))

    def test_usage(self):
        rc, out = self.run_script("sideways")
        self.assertEqual((rc, out["error"]), (1, "usage: voice.sh on|off|stage|status"))


if __name__ == "__main__":
    unittest.main()
