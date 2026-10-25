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
    def run_registry(self, flags=(), helper='{"running":false}', home="off", live=False):
        with tempfile.TemporaryDirectory() as d:
            paths = {k: os.path.join(d, k) for k in ("SYSTEMUI_ON", "WEBGLASS_ON", "NOTI_ON", "LOGO_ON", "PRELOAD_STATE", "INPUTCOMMON_ON")}
            for k in flags:
                open(paths[k], "w").close()
            script = "".join(f'{k}="{v}"\n' for k, v in paths.items())
            script += (f"homestub_json() {{ printf '{{\"state\":\"{home}\"}}'; }}\n"
                       f"helper_json() {{ printf '{helper}'; }}\n"
                       f"comp_live() {{ return {0 if live else 1}; }}\n" + function("surfaces_json") + "surfaces_json\n")
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            return {e["id"]: e for e in json.loads(r.stdout)}

    def test_every_switch_has_an_entry_that_names_its_way_back(self):
        reg = self.run_registry()
        self.assertEqual(sorted(reg), ["compositor", "helper", "home", "inputcommon", "logos", "notifications", "preload", "quicksettings", "settings-theme"])
        for e in reg.values():
            self.assertIsInstance(e["on"], bool)
            self.assertGreater(len(e["fallback"]), 20, e["id"])

    def test_nothing_on_reads_as_all_off(self):
        self.assertFalse(any(e["on"] for e in self.run_registry().values()))

    def test_each_entry_reads_its_own_flag(self):
        for flag, ident in (("SYSTEMUI_ON", "quicksettings"), ("WEBGLASS_ON", "settings-theme"), ("NOTI_ON", "notifications"),
                            ("LOGO_ON", "logos"), ("PRELOAD_STATE", "preload"), ("INPUTCOMMON_ON", "inputcommon")):
            reg = self.run_registry(flags=[flag])
            self.assertEqual([k for k, e in reg.items() if e["on"]], [ident], flag)

    def test_home_is_on_for_the_stub_and_a_trial_only(self):
        for state, on in (("stub", True), ("trial", True), ("lg", False), ("off", False)):
            reg = self.run_registry(home=state)
            self.assertEqual((reg["home"]["on"], reg["home"]["state"]), (on, state))

    def test_helper_and_compositor_come_from_their_own_probes(self):
        reg = self.run_registry(helper='{"running":true,"pid":9}', live=True)
        self.assertTrue(reg["helper"]["on"])
        self.assertTrue(reg["compositor"]["on"])


if __name__ == "__main__":
    unittest.main()
