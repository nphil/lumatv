"""Run with: python3 -m unittest tests.test_surfaces_registry

`lumatv surfaces`: the registry function from tools/lumatv, extracted and run under sh with stubs for the
state it reads. One valid JSON array, one entry per switch, every entry names its way back.
"""
import json
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "tools", "lumatv")


def function(name):
    out, on = [], False
    with open(TOOL) as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")


class Registry(unittest.TestCase):
    def run_registry(self, flags=(), helper='{"running":false}', live=False, general=""):
        with tempfile.TemporaryDirectory() as d:
            paths = {"PRELOAD_STATE": os.path.join(d, "preload")}
            for k in flags:
                open(paths[k], "w").close()
            script = "".join(f'{k}="{v}"\n' for k, v in paths.items())
            script += (f"helper_json() {{ printf '{helper}'; }}\n"
                       f"comp_live() {{ return {0 if live else 1}; }}\n"
                       f"get_general_setting() {{ printf '%s' '{general}'; }}\n" + function("surfaces_json") + "surfaces_json\n")
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            return {e["id"]: e for e in json.loads(r.stdout)}

    def test_every_switch_has_an_entry_that_names_its_way_back(self):
        reg = self.run_registry()
        self.assertEqual(sorted(reg), ["compositor", "helper", "preload", "standbyscreen"])
        for e in reg.values():
            self.assertIsInstance(e["on"], bool)
            self.assertTrue(e["fallback"], e["id"])

    def test_nothing_on_reads_as_all_off(self):
        self.assertFalse(any(e["on"] for e in self.run_registry().values()))

    def test_each_entry_reads_its_own_flag(self):
        for flag, ident in (("PRELOAD_STATE", "preload"),):
            reg = self.run_registry(flags=[flag])
            self.assertEqual([k for k, e in reg.items() if e["on"]], [ident], flag)

    def test_standbyscreen_is_on_when_the_tv_says_always_ready_is_off(self):
        # read from the TV's own setting, not from a flag: the panel is dark in standby exactly when lifeOnScreenMode reads off
        for mode, on in (("off", True), ("alwaysReady", False), ("allEnabled", False), ("", False)):
            self.assertEqual(self.run_registry(general=mode)["standbyscreen"]["on"], on, mode)


    def test_helper_and_compositor_come_from_their_own_probes(self):
        reg = self.run_registry(helper='{"running":true,"pid":9}', live=True)
        self.assertTrue(reg["helper"]["on"])
        self.assertTrue(reg["compositor"]["on"])


if __name__ == "__main__":
    unittest.main()
