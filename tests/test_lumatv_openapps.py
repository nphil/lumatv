"""Run with: python3 -m unittest tests.test_lumatv_openapps

`lumatv openapps status` and the `openApps` key of `lumatv status`: LG's list of apps that must be updated before they open, and the
setting that keeps it empty, as the files say it now, by the same rules as the helper's `openapps` topic (helper/app/src/openapps.rs).
Also `openapps_restore`, which `revert` calls.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "lumatv")

# What the TV's own file held on 2026-10-29 (compact, as LG writes it), and one with a third-party app that must be updated.
LG = '[{"nsu":"Y","status":"F","prohibit":"N","id":"com.webos.app.lifeonscreen","type":"S"}]'
TWO = LG[:-1] + ',{"nsu":"N","status":"N","prohibit":"Y","id":"dazn","type":"A"}]'


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


class Rig(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.info = os.path.join(self.d, "updateInfo")
        self.flag = os.path.join(self.d, "state", "openapps", "on")
        self.saved = os.path.join(self.d, "state", "openapps", "updateInfo.saved")
        self.proc = os.path.join(self.d, "proc")
        os.makedirs(os.path.dirname(self.flag))
        os.makedirs(self.proc)

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, path, text):
        with open(path, "w") as f:
            f.write(text)

    def read(self, path):
        with open(path) as f:
            return f.read()

    def process(self, pid, comm):
        os.makedirs(os.path.join(self.proc, str(pid)), exist_ok=True)
        self.put(os.path.join(self.proc, str(pid), "comm"), comm + "\n")

    def env(self):
        return (f'OA_INFO="{self.info}"\nOA_FLAG="{self.flag}"\nOA_SAVED="{self.saved}"\nOA_PROC="{self.proc}"\n'
                'log_line() { :; }\n')

    def status(self, arg="status"):
        script = (self.env() + function("json_escape") + function("emit_error") + function("openapps_running") + function("openapps_json")
                  + function("verb_openapps") + f'verb_openapps "{arg}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, json.loads(r.stdout)

    def restore(self):
        script = self.env() + function("openapps_running") + function("openapps_restore") + "openapps_restore\n"
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        self.assertEqual((r.returncode, r.stderr), (0, ""))


class Status(Rig):
    def test_off_says_how_many_apps_lg_would_ask_about(self):
        self.put(self.info, TWO)
        rc, doc = self.status()
        self.assertEqual(rc, 0)
        self.assertEqual(doc["openApps"], {"enabled": False, "entries": 2, "ids": ["com.webos.app.lifeonscreen", "dazn"], "state": "off", "saved": False})

    def test_on_with_an_empty_or_missing_list_is_holding(self):
        self.put(self.flag, "")
        self.assertEqual(self.status()[1]["openApps"]["state"], "holding")          # no file at all
        for text in ("[]", " [ ]\n", ""):
            self.put(self.info, text)
            u = self.status()[1]["openApps"]
            self.assertEqual((u["state"], u["entries"], u["ids"]), ("holding", 0, []), repr(text))

    def test_on_with_entries_is_clearing_or_waiting_while_lgs_updater_runs(self):
        self.put(self.flag, "")
        self.put(self.info, LG)
        self.process(100, "sam")
        self.assertEqual(self.status()[1]["openApps"]["state"], "clearing")
        self.process(200, "appinstalld")
        self.assertEqual(self.status()[1]["openApps"]["state"], "waiting")
        self.put(self.info, "[]")
        self.assertEqual(self.status()[1]["openApps"]["state"], "holding")           # a running updater matters only with a list to empty

    def test_a_file_that_is_not_an_array_is_unreadable_and_not_counted(self):
        self.put(self.flag, "")
        for text in ("{not json", "{}", "null"):
            self.put(self.info, text)
            u = self.status()[1]["openApps"]
            self.assertEqual((u["state"], u["entries"], u["ids"]), ("unreadable", 0, []), text)

    def test_only_the_first_eight_ids_are_listed_and_all_are_counted(self):
        self.put(self.info, "[" + ",".join('{"status":"N","id":"app%d","type":"A"}' % i for i in range(11)) + "]")
        u = self.status()[1]["openApps"]
        self.assertEqual((u["entries"], u["ids"]), (11, ["app%d" % i for i in range(8)]))

    def test_saved_shows_and_a_bad_argument_is_refused(self):
        self.put(self.saved, LG)
        self.assertTrue(self.status()[1]["openApps"]["saved"])
        rc, doc = self.status("on")
        self.assertEqual((rc, doc["ok"]), (1, False))


class Restore(Rig):
    def test_the_saved_list_goes_back_into_an_empty_file_and_the_setting_goes_off(self):
        self.put(self.flag, "")
        self.put(self.saved, TWO)
        self.put(self.info, "[]")
        self.restore()
        self.assertEqual(self.read(self.info), TWO)
        self.assertEqual((os.path.exists(self.flag), os.path.exists(self.saved)), (False, False))
        self.assertEqual(os.listdir(self.d).count("updateInfo.lumatv"), 0)

    def test_a_newer_list_from_lg_stays_and_the_saved_one_is_dropped(self):
        self.put(self.saved, TWO)
        self.put(self.info, LG)
        self.restore()
        self.assertEqual(self.read(self.info), LG)
        self.assertFalse(os.path.exists(self.saved))

    def test_a_running_updater_leaves_the_file_and_keeps_the_saved_list_for_the_helper(self):
        self.put(self.flag, "")
        self.put(self.saved, TWO)
        self.put(self.info, "[]")
        self.process(200, "appinstalld")
        self.restore()
        self.assertEqual(self.read(self.info), "[]")
        self.assertEqual((os.path.exists(self.flag), os.path.exists(self.saved)), (False, True))

    def test_nothing_saved_only_turns_the_setting_off(self):
        self.put(self.flag, "")
        self.put(self.info, LG)
        self.restore()
        self.assertEqual((self.read(self.info), os.path.exists(self.flag)), (LG, False))


class WholeTool(unittest.TestCase):
    """The real tools/lumatv run as a command in a copy of the app tree whose data directory is a temporary one: the verbs must find
    their functions."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.app = os.path.join(self.tmp.name, "app")
        os.makedirs(os.path.join(self.app, "tools"))
        for f in os.listdir(os.path.join(ROOT, "tools")):
            src = os.path.join(ROOT, "tools", f)
            if os.path.isfile(src):
                shutil.copy(src, os.path.join(self.app, "tools", f))
        shutil.copy(os.path.join(ROOT, "appinfo.json"), self.app)
        shutil.copy(os.path.join(ROOT, "brand.json"), self.app)
        tool = os.path.join(self.app, "tools", "lumatv")
        with open(tool) as f:
            text = f.read()
        self.assertEqual(text.count('\nSTATE="$BRAND_DATA_DIR"\n'), 1)
        self.data = os.path.join(self.tmp.name, "data")
        with open(tool, "w") as f:
            f.write(text.replace('\nSTATE="$BRAND_DATA_DIR"\n', '\nSTATE="%s"\n' % self.data))
        self.info = os.path.join(self.tmp.name, "updateInfo")
        with open(self.info, "w") as f:
            f.write(TWO)
        self.proc = os.path.join(self.tmp.name, "proc")
        os.makedirs(self.proc)

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, *args):
        env = dict(os.environ, OA_INFO=self.info, OA_PROC=self.proc)
        p = subprocess.run(["sh", os.path.join(self.app, "tools", "lumatv")] + list(args), env=env, capture_output=True, text=True, timeout=120)
        return p.returncode, p.stdout, p.stderr

    def test_the_verb_answers_and_the_flag_lives_under_the_data_dir(self):
        rc, out, err = self.run_tool("openapps", "status")
        self.assertEqual((rc, "not found" in err), (0, False), err)
        self.assertEqual(json.loads(out)["openApps"]["state"], "off")
        os.makedirs(os.path.join(self.data, "openapps"))
        open(os.path.join(self.data, "openapps", "on"), "w").close()
        u = json.loads(self.run_tool("openapps", "status")[1])["openApps"]
        self.assertEqual((u["enabled"], u["state"], u["entries"]), (True, "clearing", 2))

    def test_status_carries_the_key_and_prints_no_shell_error(self):
        rc, out, err = self.run_tool("status")
        self.assertNotIn("not found", err)
        self.assertEqual(json.loads(out)["openApps"]["entries"], 2)

    def test_a_bad_argument_is_refused(self):
        rc, out, err = self.run_tool("openapps", "on")
        self.assertEqual((rc, json.loads(out)["ok"]), (1, False))

    def test_revert_calls_the_restore(self):
        with open(TOOL) as f:
            text = f.read()
        i = text.index("do_revert_internal() {")
        self.assertIn("  helper_stop\n  openapps_restore\n", text[i:i + 1500])


if __name__ == "__main__":
    unittest.main()
