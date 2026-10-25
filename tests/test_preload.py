"""Run with: python3 -m unittest discover -s tests -p 'test_*.py'

tools/preload.py against a fake luna-send that keeps configd's list and the live policy in files.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "preload.py")
PAYLOAD = os.path.join(ROOT, "payload", "preload.json")
KEY = "com.webos.service.preloadmanager.supportAppList"

FAKE = r'''#!/usr/bin/env python3
import json, os, sys
d = os.environ["FAKE_DIR"]
uri, payload = sys.argv[-2], json.loads(sys.argv[-1])
with open(os.path.join(d, "calls"), "a") as f:
    f.write(uri.rsplit("/", 1)[-1] + " " + json.dumps(payload) + "\n")
if os.path.exists(os.path.join(d, "fail")) and uri.endswith(open(os.path.join(d, "fail")).read().strip()):
    print(json.dumps({"returnValue": False}))
elif uri.endswith("getConfigs"):
    key = payload["configNames"][0]
    print(json.dumps({"returnValue": True, "configs": {key: json.load(open(os.path.join(d, "list.json")))}}))
elif uri.endswith("setConfigs"):
    key = list(payload["configs"])[0]
    json.dump(payload["configs"][key], open(os.path.join(d, "list.json"), "w"))
    print(json.dumps({"returnValue": True}))
elif uri.endswith("setPreloadPolicy"):
    a = payload["application"]
    live = json.load(open(os.path.join(d, "live.json"))) if os.path.exists(os.path.join(d, "live.json")) else {}
    live[a["id"]] = a["isEnabled"]
    json.dump(live, open(os.path.join(d, "live.json"), "w"))
    print(json.dumps({"returnValue": True}))
'''

LG = [
    {"id": "netflix", "isEnabled": True},
    {"id": "youtube.leanback.v4", "isEnabled": True},
    {"id": "com.webos.app.lgchannels", "isEnabled": True},
    {"id": "com.webos.app.browser", "isEnabled": True},
    {"id": "amazon", "isEnabled": True},
    {"id": "airplay", "isEnabled": False},
]


class Preload(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.bin = os.path.join(d, "bin")
        os.makedirs(self.bin)
        path = os.path.join(self.bin, "luna-send")
        with open(path, "w") as f:
            f.write(FAKE)
        os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], FAKE_DIR=d)
        self.orig = os.path.join(d, "data", "preload.orig")
        self.state = os.path.join(d, "data", "preload.json")
        with open(os.path.join(d, "list.json"), "w") as f:
            json.dump(LG, f)

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, verb):
        r = subprocess.run([sys.executable, TOOL, verb, PAYLOAD, self.orig, self.state], env=self.env, capture_output=True, text=True)
        return r.returncode, json.loads(r.stdout)

    def read(self, name):
        with open(os.path.join(self.tmp.name, name)) as f:
            return json.load(f)

    def enabled(self):
        return {e["id"]: e["isEnabled"] for e in self.read("list.json")}

    def test_on_drops_exactly_the_named_apps_and_tells_the_live_manager(self):
        rc, out = self.run_tool("on")
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.enabled(), {"netflix": True, "youtube.leanback.v4": False, "com.webos.app.lgchannels": False,
                                          "com.webos.app.browser": False, "amazon": False, "airplay": False})
        self.assertEqual(self.read("live.json"), {i: False for i in json.load(open(PAYLOAD))["off"]})
        self.assertEqual(self.read("data/preload.json"), {"off": json.load(open(PAYLOAD))["off"]})
        self.assertEqual(json.load(open(self.orig)), LG)

    def test_on_twice_keeps_the_first_backup_and_writes_nothing_new(self):
        self.run_tool("on")
        sets = open(os.path.join(self.tmp.name, "calls")).read().count("setConfigs")
        rc, out = self.run_tool("on")
        self.assertEqual(rc, 0)
        self.assertFalse(out["changed"])
        self.assertEqual(open(os.path.join(self.tmp.name, "calls")).read().count("setConfigs"), sets)
        self.assertEqual(json.load(open(self.orig)), LG)

    def test_off_restores_each_entry_as_lg_had_it_and_removes_the_state(self):
        self.run_tool("on")
        rc, out = self.run_tool("off")
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.read("list.json"), LG)
        self.assertEqual(self.read("live.json")["youtube.leanback.v4"], True)
        self.assertFalse(os.path.exists(self.state))

    def test_off_keeps_changes_made_to_other_entries_meanwhile(self):
        self.run_tool("on")
        lst = self.read("list.json")
        for e in lst:
            if e["id"] == "airplay":
                e["isEnabled"] = True
        json.dump(lst, open(os.path.join(self.tmp.name, "list.json"), "w"))
        self.run_tool("off")
        self.assertTrue(self.enabled()["airplay"])
        self.assertTrue(self.enabled()["youtube.leanback.v4"])

    def test_off_without_a_backup_fails_and_changes_nothing(self):
        rc, out = self.run_tool("off")
        self.assertEqual(rc, 1)
        self.assertEqual(self.read("list.json"), LG)

    def test_wake_resends_the_policy_for_the_recorded_apps_only(self):
        rc, out = self.run_tool("wake")
        self.assertEqual((rc, out["off"]), (0, []))
        self.run_tool("on")
        os.remove(os.path.join(self.tmp.name, "live.json"))
        rc, out = self.run_tool("wake")
        self.assertEqual(rc, 0)
        self.assertEqual(self.read("live.json"), {i: False for i in json.load(open(PAYLOAD))["off"]})

    def test_an_unreadable_list_fails_and_writes_no_state(self):
        with open(os.path.join(self.tmp.name, "fail"), "w") as f:
            f.write("getConfigs")
        rc, out = self.run_tool("on")
        self.assertEqual(rc, 1)
        self.assertFalse(os.path.exists(self.state))
        self.assertFalse(os.path.exists(self.orig))

    def test_a_refused_live_policy_is_reported(self):
        with open(os.path.join(self.tmp.name, "fail"), "w") as f:
            f.write("setPreloadPolicy")
        rc, out = self.run_tool("on")
        self.assertEqual(rc, 1)
        self.assertEqual(len(out["failed"]), len(json.load(open(PAYLOAD))["off"]))

    def test_status_reads_without_writing(self):
        rc, out = self.run_tool("status")
        self.assertEqual(rc, 0)
        self.assertFalse(out["on"])
        listed = {e["id"] for e in LG}
        self.assertEqual([o["enabled"] for o in out["off"]],
                         [True if i in listed else None for i in json.load(open(PAYLOAD))["off"]])
        self.assertFalse(os.path.exists(os.path.join(self.tmp.name, "data")))

    def test_the_shipped_list_keeps_only_netflix_and_home_warm(self):
        # LG's list on the set (2026-10-25, docs/research/memory-result.md): every entry but these two is off,
        # because the preload manager loads any enabled entry whenever memory allows.
        off = json.load(open(PAYLOAD))["off"]
        self.assertEqual(sorted(off), sorted([
            "youtube.leanback.v4", "com.webos.app.lgchannels", "com.webos.app.browser", "amazon",
            "com.webos.app.livetv", "com.webos.app.livemenu", "com.webos.app.voice", "amazon.alexapr",
            "com.webos.app.miracast", "com.webos.app.miracast-overlay",
            "com.webos.app.hdmi1", "com.webos.app.hdmi2", "com.webos.app.hdmi3", "com.webos.app.hdmi4",
            "vudu", "pooq", "globoplaywebos", "yandex.alice"]))
        self.assertNotIn("netflix", off)
        self.assertNotIn("com.webos.app.home", off)


if __name__ == "__main__":
    unittest.main()
