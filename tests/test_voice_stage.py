"""Run with: python3 -m unittest tests.test_voice_stage

tools/voice.sh: LG's Search app's overlay, run by tools/inputcommon.sh with this app's names. What matters here is what differs from the input
banners': the tree (Main.qml and Style.qml), its own stage directory, the patch it runs, the dump it sets aside (voice, and never voiceweb or
voiceagent, or the input banners' own dump) and the answers it prints. The staging and the cold start themselves are tests.test_inputcommon_stage's.

`VoiceResident` is the part that makes the patched app show: SAM keeps the Search app alive, a close only hides it and it reads its QML once, so
`on` ends its process after the mount and `off` after the unmount (round 11: pid 8321 before and after the close). It runs on a stand-in tree
and patch with a fake /proc and a stand-in `kill`, and once on real processes.
"""
import json
import os
import shutil
import subprocess
import sys
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import test_inputcommon_stage as ics  # noqa: E402

ROOT = ics.ROOT
SCRIPT = os.path.join(ROOT, "tools", "voice.sh")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/voice/qml", "/tmp/tvsrc/apps/com.webos.app.voice/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/voice/qml")
VOICE = "com.webos.app.voice"
ASIDE = ".lumatv-aside-voice."
VOICE_BIN = "/usr/bin/com.webos.app.voice"
AGENT_BIN = "/usr/bin/com.webos.app.voiceagent"
# A stand-in for the patch of the stand-in tree of VoiceResident.
FAKE_VOICE_PATCH = """#!/usr/bin/env python3
import sys
open(sys.argv[1] + "/Style.qml", "a").write("// Luma: patched\\n")
print("Style.qml")
"""
# umount, finding the overlay busy for as long as process $FAKE_BUSY_PID is there (something holds a file of it)
FAKE_UMOUNT_BUSY = """#!/bin/sh
echo "umount $*" >> "$FAKE_LOG"
[ -d "$PROC_ROOT/$FAKE_BUSY_PID" ] && exit 1
grep -v " $1 overlay " "$PROC_MOUNTS" > "$PROC_MOUNTS.tmp"
mv "$PROC_MOUNTS.tmp" "$PROC_MOUNTS"
"""


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
        from test_voice_patch import PRINTED
        self.assertEqual(self.staged(), PRINTED)
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


class VoiceResident(VoiceBase):
    def make_qml(self):
        os.makedirs(self.qml)
        for rel in ("Main.qml", "Style.qml"):
            with open(os.path.join(self.qml, rel), "w") as f:
                f.write("// LG's\n")

    def appdir(self):
        d = os.path.join(self.t, "app")
        os.makedirs(os.path.join(d, "tools"))
        os.symlink(os.path.join(ROOT, "tools", "inputcommon.sh"), os.path.join(d, "tools", "inputcommon.sh"))
        for name, body in (("tokens.py", ics.FAKE_TOKENS), ("voice_patch.py", FAKE_VOICE_PATCH)):
            with open(os.path.join(d, "tools", name), "w") as f:
                f.write(body)
        return d

    def setUp(self):
        super().setUp()
        # the Search app as it stands on the set once it has been opened: a process of its own that SAM lists, and forgets when it ends
        self.process(8321)
        self.running_as({"com.webos.app.home": 17483, VOICE: 8321})
        self.after_kill({"com.webos.app.home": 17483})

    def after_kill(self, apps):
        self.write("sam/running-after-kill", json.dumps({"running": [{"id": a, "processid": str(p)} for a, p in apps.items()], "returnValue": True}))

    def kills(self):
        return [c for c in self.sam_calls() if c.startswith("kill")]

    def log_text(self):
        return "\n".join(self.lines(os.path.join(self.state, "log")))

    def test_on_ends_the_process_after_the_mount_and_the_close_and_no_other(self):
        self.process(8400, AGENT_BIN, comm="com.webos.app.v")                # the same comm, another binary
        self.process(2513, "/usr/bin/com.webos.app.inputcommon")
        self.process(8500, "/usr/bin/flutter-client")                        # a process that only names the app in its arguments
        with open(os.path.join(self.proc, "8500", "cmdline"), "wb") as f:
            f.write(b"/usr/bin/flutter-client\0-i\0" + VOICE.encode() + b"\0")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["voice"], out["closed"], out["ended"]), (0, True, "on", [VOICE], [VOICE]), out)
        self.assertFalse(self.alive(8321))
        for pid in (8400, 2513, 8500):
            self.assertTrue(self.alive(pid), pid)
        self.assertEqual(self.kills(), ["kill TERM 8321"])
        sam = self.sam_calls()
        self.assertLess(sam.index('closeByAppId {"id":"%s"}' % VOICE), sam.index("kill TERM 8321"))
        names = [c.split()[0] for c in self.calls()]
        self.assertLess(names.index("mount"), names.index("kill"))
        self.assertIn("ended com.webos.app.voice (pids 8321)", self.log_text())

    def test_a_process_that_ignores_SIGTERM_is_killed(self):
        self.write("sam/ignore-term", "")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["ended"]), (0, True, [VOICE]), out)
        self.assertEqual(self.kills(), ["kill TERM 8321", "kill KILL 8321"])
        self.assertFalse(self.alive(8321))
        self.assertIn("ignored SIGTERM (pids 8321): SIGKILL", self.log_text())

    def test_a_process_that_survives_both_signals_is_reported_and_not_claimed(self):
        self.write("sam/immortal", "")
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["ended"]), (0, True, []), out)
        self.assertEqual(self.kills(), ["kill TERM 8321", "kill KILL 8321"])
        self.assertTrue(self.alive(8321))
        self.assertIn("pid 8321 of com.webos.app.voice is still running", self.log_text())
        self.assertTrue(self.run_script("status")[1]["mounted"])

    def test_off_unmounts_first_and_ends_the_process_so_that_the_next_start_reads_LGs_files(self):
        self.run_script("on")
        self.process(9001)                                                   # started since: it holds the patched files
        self.running_as({VOICE: 9001})
        self.after_kill({})
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["voice"], out["closed"], out["ended"]), (0, "off", [VOICE], [VOICE]), out)
        names = [c.split()[0] for c in self.calls()]
        self.assertLess(names.index("umount"), names.index("kill", names.index("umount")))
        self.assertFalse(self.alive(9001))
        self.assertFalse(self.run_script("status")[1]["mounted"])

    def test_off_on_a_set_that_never_used_the_switch_leaves_the_running_app_alone(self):
        rc, out = self.run_script("off")
        self.assertEqual((rc, out["ended"], out["closed"]), (0, [], []), out)
        self.assertTrue(self.alive(8321))
        self.assertEqual(self.sam_calls(), [])

    def test_a_busy_overlay_is_let_go_by_ending_the_app_that_holds_it_and_tried_again(self):
        self.run_script("on")
        self.process(9001)
        self.running_as({VOICE: 9001})
        self.after_kill({})
        with open(self.env["UMOUNT"], "w") as f:
            f.write(FAKE_UMOUNT_BUSY)
        rc, out = self.run_script("off", FAKE_BUSY_PID="9001")
        self.assertEqual((rc, out["ok"], out["ended"]), (0, True, [VOICE]), out)
        self.assertEqual([c.split()[0] for c in self.calls()], ["mount", "kill", "umount", "kill", "umount"])
        self.assertFalse(self.run_script("status")[1]["mounted"])

    def test_an_overlay_that_stays_busy_after_the_app_is_ended_stops_off(self):
        self.run_script("on")
        self.process(7777, "/usr/bin/some-other-holder", comm="holder")      # not the Search app: not ours to end
        with open(self.env["UMOUNT"], "w") as f:
            f.write(FAKE_UMOUNT_BUSY)
        rc, out = self.run_script("off", FAKE_BUSY_PID="7777")
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("could not unmount", out["error"])
        self.assertTrue(self.alive(7777))

    def test_the_app_on_screen_is_left_running(self):
        self.write("sam/foreground", VOICE)
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["closed"], out["ended"], out["onScreen"]), (0, [], [], VOICE), out)
        self.assertTrue(self.alive(8321))
        self.assertEqual(self.kills(), [])

    def test_a_process_SAM_does_not_list_is_ended_without_a_close(self):
        self.running_as({"com.webos.app.home": 17483})
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["closed"], out["ended"]), (0, [], [VOICE]), out)
        self.assertFalse([c for c in self.sam_calls() if c.startswith("closeByAppId")])
        self.assertFalse(self.alive(8321))

    def test_a_zombie_keeps_its_comm_but_is_neither_signalled_nor_waited_for(self):
        self.process(8600)
        open(os.path.join(self.proc, "8600", "cmdline"), "w").close()        # a zombie has no command line
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ended"], self.kills()), (0, [VOICE], ["kill TERM 8321"]), out)
        self.assertTrue(self.alive(8600))

    def test_with_no_process_there_is_nothing_to_end(self):
        shutil.rmtree(os.path.join(self.proc, "8321"))
        self.running_as({"com.webos.app.home": 17483})
        rc, out = self.run_script("on")
        self.assertEqual((rc, out["ok"], out["closed"], out["ended"]), (0, True, [], []), out)
        self.assertEqual(self.kills(), [])

    def test_the_app_gets_a_second_after_the_close_before_the_signal(self):
        t0 = time.time()
        rc, out = self.run_script("on", IC_STEP="1")
        self.assertEqual((rc, out["ended"]), (0, [VOICE]), out)
        self.assertGreaterEqual(time.time() - t0, 1.0)

    def test_SAM_still_listing_the_pid_is_waited_for_a_few_steps_and_then_let_go(self):
        os.unlink(os.path.join(self.t, "sam", "running-after-kill"))
        rc, out = self.run_script("on", IC_SAM_STEPS="2")
        self.assertEqual((rc, out["ok"], out["ended"]), (0, True, [VOICE]), out)
        # one read for the close, then a look before each of the two steps and one after
        self.assertEqual(len([c for c in self.sam_calls() if c.startswith("running")]), 4)
        self.assertIn("SAM still lists pids 8321 after 2 steps", self.log_text())

    def test_a_new_pid_for_the_app_means_SAM_has_forgotten_the_old_one(self):
        self.after_kill({VOICE: 9000})                                       # LG's preload manager started it again
        rc, out = self.run_script("on", IC_SAM_STEPS="2")
        self.assertEqual((rc, out["ended"]), (0, [VOICE]), out)
        self.assertEqual(len([c for c in self.sam_calls() if c.startswith("running")]), 2)
        self.assertNotIn("still lists", self.log_text())

    def test_status_lists_the_processes_of_the_app_and_only_those(self):
        self.process(8400, AGENT_BIN, comm="com.webos.app.v")
        self.assertEqual(self.run_script("status")[1]["pids"], [8321])
        self.run_script("on")
        self.assertEqual(self.run_script("status")[1]["pids"], [])

    def test_real_processes_get_the_same_signals_and_the_lookalike_is_left(self):
        # no stand-ins for /proc and kill: copies of sleep named like the Search binary and like a lookalike (the same first 15 characters,
        # so the same comm), started with their own path as argv[0], in a directory of their own so that nothing else on the machine matches
        d = os.path.join(self.t, "bin")
        os.makedirs(d)
        procs = {}
        try:
            for name in ("com.webos.app.voice", "com.webos.app.voiceagent"):
                path = os.path.join(d, name)
                shutil.copy(shutil.which("sleep"), path)
                procs[name] = subprocess.Popen([path, "60", "0"], stdin=subprocess.DEVNULL)
            binary = os.path.join(d, "com.webos.app.voice")
            env = dict(VOICE_BINARY=binary, PROC_ROOT="/proc", KILL="kill", IC_STEP="1")
            pid = procs["com.webos.app.voice"].pid
            self.assertEqual(self.run_script("status", **env)[1]["pids"], [pid])
            rc, out = self.run_script("on", **env)
            self.assertEqual((rc, out["ok"], out["ended"]), (0, True, [VOICE]), out)
            self.assertIsNotNone(procs["com.webos.app.voice"].wait(timeout=5))
            self.assertIsNone(procs["com.webos.app.voiceagent"].poll())
            self.assertEqual(self.run_script("status", **env)[1]["pids"], [])
        finally:
            for p in procs.values():
                p.kill()
                p.wait()


if __name__ == "__main__":
    unittest.main()
