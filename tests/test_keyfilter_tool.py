"""tools/keyfilter.py: where our two entries go in configd's key filter list, and that nothing is written when nothing changed.

The tool runs on the set and talks to configd through luna-send; here a stand-in luna-send keeps the list in a file and counts the writes.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOOL = os.path.join(ROOT, "tools", "keyfilter.py")
NAME = "com.webos.surfacemanager.keyFilters"
PATH = "/var/lib/lumatv/keyfilter/lumatv.js"

LG = [
    {"handler": "handleKeyBlockByApp", "file": "/usr/lib/qt5/qml/KeyFilters/preProcess.js"},
    {"handler": "handleLongPressKeys", "file": "/usr/lib/qt5/qml/KeyFilters/preProcess.js"},
    {"handler": "handleAvReset", "file": "/usr/lib/qt5/qml/KeyFilters/appLaunch.js"},
    {"handler": "handleSystemKeys", "file": "/usr/lib/qt5/qml/KeyFilters/systemUi.js"},
    {"handler": "handleHotkeys", "file": "/usr/lib/qt5/qml/KeyFilters/appLaunch.js"},
    {"handler": "handleLocalKeys", "file": "/usr/lib/qt5/qml/KeyFilters/appLaunch.js"},
]
HOME = {"handler": "handleLumaHome", "file": PATH}
QUICK = {"handler": "handleLumaQuick", "file": PATH}

# a luna-send that serves and records the list: getConfigs answers the state file, setConfigs replaces it and counts
FAKE = """#!/usr/bin/env python3
import json, sys
state = sys.argv[0] + ".state"
uri, payload = sys.argv[-2], json.loads(sys.argv[-1])
doc = json.load(open(state))
if uri.endswith("getConfigs"):
    print(json.dumps({"returnValue": True, "configs": {payload["configNames"][0]: doc["chain"]}}))
else:
    doc["chain"] = list(payload["configs"].values())[0]
    doc["writes"] += 1
    json.dump(doc, open(state, "w"))
    print(json.dumps({"returnValue": True}, separators=(",", ":")))
"""


class Tool(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.bin = os.path.join(self.dir.name, "luna-send")
        with open(self.bin, "w") as f:
            f.write(FAKE)
        os.chmod(self.bin, os.stat(self.bin).st_mode | stat.S_IEXEC)

    def start(self, chain):
        with open(self.bin + ".state", "w") as f:
            json.dump({"chain": chain, "writes": 0}, f)

    def state(self):
        with open(self.bin + ".state") as f:
            return json.load(f)

    def run_tool(self, mode):
        env = dict(os.environ, PATH=self.dir.name + os.pathsep + os.environ["PATH"])
        return subprocess.run([sys.executable, TOOL, mode, "handleLumaHome", PATH], env=env, capture_output=True, text=True).returncode

    def names(self):
        return [e["handler"] for e in self.state()["chain"]]

    def test_our_first_entry_goes_first_and_the_second_just_before_LGs_hotkeys(self):
        self.start(LG)
        self.assertEqual(self.run_tool("add"), 0)
        self.assertEqual(self.names(), ["handleLumaHome", "handleKeyBlockByApp", "handleLongPressKeys", "handleAvReset", "handleSystemKeys",
                                        "handleLumaQuick", "handleHotkeys", "handleLocalKeys"])
        for e in self.state()["chain"]:
            if e["handler"].startswith("handleLuma"):
                self.assertEqual(e["file"], PATH, "both are functions of the one script, so one loader object and one set of state")
        # every LG entry is still there, in its own order
        self.assertEqual([e for e in self.state()["chain"] if not e["handler"].startswith("handleLuma")], LG)

    def test_a_second_add_writes_nothing_because_every_write_reloads_LGs_chain(self):
        self.start(LG)
        self.run_tool("add")
        self.assertEqual(self.state()["writes"], 1)
        self.assertEqual(self.run_tool("add"), 0)
        self.assertEqual(self.state()["writes"], 1)

    def test_an_install_that_only_has_the_first_entry_gets_the_second(self):
        self.start([HOME] + LG)
        self.assertEqual(self.run_tool("add"), 0)
        self.assertEqual(self.names().count("handleLumaQuick"), 1)
        self.assertEqual(self.names().index("handleLumaQuick") + 1, self.names().index("handleHotkeys"))
        self.assertEqual(self.state()["writes"], 1)

    def test_a_stale_entry_moves_to_its_place_and_a_changed_path_is_written(self):
        self.start([QUICK, HOME] + LG)
        self.run_tool("add")
        self.assertEqual(self.names()[0], "handleLumaHome")
        self.assertEqual(self.names().index("handleLumaQuick") + 1, self.names().index("handleHotkeys"))
        self.assertEqual(self.names().count("handleLumaQuick"), 1)

    def test_remove_takes_both_and_leaves_LGs_chain_as_it_was(self):
        self.start(LG)
        self.run_tool("add")
        self.assertEqual(self.run_tool("remove"), 0)
        self.assertEqual(self.state()["chain"], LG)
        # nothing of ours left: nothing written
        writes = self.state()["writes"]
        self.assertEqual(self.run_tool("remove"), 0)
        self.assertEqual(self.state()["writes"], writes)

    def test_a_chain_without_handleHotkeys_gets_the_first_entry_only(self):
        chain = [e for e in LG if e["handler"] != "handleHotkeys"]
        self.start(chain)
        self.assertEqual(self.run_tool("add"), 0)
        self.assertEqual(self.names(), ["handleLumaHome"] + [e["handler"] for e in chain])


if __name__ == "__main__":
    unittest.main()
