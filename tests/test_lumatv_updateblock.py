"""Run with: python3 -m unittest tests.test_lumatv_updateblock

`lumatv updateblock status` and the `updateBlock` key of `lumatv status`: LG's system-update block as the files say it now (the Homebrew
Channel's flag file and what /etc/hosts maps), by the same rules as the helper's `updateblock` topic (helper/app/src/updateblock.rs).
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

# What the Homebrew Channel's startup.sh writes (services/startup.sh, "Block software update servers").
HBC = ("127.0.0.1 localhost\n\n# This file is dynamically regenerated on boot by webosbrew startup script\n"
       "127.0.0.1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n::1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n")


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


class UpdateBlock(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.flag = os.path.join(self.d, "webosbrew_block_updates")
        self.hosts = os.path.join(self.d, "hosts")

    def tearDown(self):
        self.tmp.cleanup()

    def status(self, arg="status", flag=None, hosts=None):
        if flag:
            open(self.flag, "w").close()
        if hosts is not None:
            with open(self.hosts, "w") as f:
                f.write(hosts)
        script = (f'UB_FLAG="{self.flag}"\nUB_HOSTS="{self.hosts}"\nUB_NAMES="snu.lge.com su.lge.com su-ssl.lge.com su-dev.lge.com"\n'
                  + function("json_escape") + function("emit_error") + function("updateblock_json") + function("verb_updateblock")
                  + f'verb_updateblock "{arg}"\n')
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, json.loads(r.stdout)

    def test_blocked_when_the_flag_and_all_four_names_are_there(self):
        rc, doc = self.status(flag=True, hosts=HBC)
        self.assertEqual(rc, 0)
        u = doc["updateBlock"]
        self.assertEqual((u["state"], u["flag"], u["live"], u["blocking"]), ("blocked", True, True, True))
        self.assertEqual(u["hosts"], {"snu.lge.com": True, "su.lge.com": True, "su-ssl.lge.com": True, "su-dev.lge.com": True})

    def test_armed_hosts_only_and_open(self):
        self.assertEqual(self.status(flag=True, hosts="127.0.0.1 localhost\n")[1]["updateBlock"]["state"], "armed")
        os.remove(self.flag)
        u = self.status(hosts=HBC)[1]["updateBlock"]
        self.assertEqual((u["state"], u["flag"], u["live"], u["blocking"]), ("hosts-only", False, True, True))
        u = self.status(hosts="127.0.0.1 localhost\n")[1]["updateBlock"]
        self.assertEqual((u["state"], u["blocking"]), ("open", False))

    def test_a_comment_another_address_or_another_name_does_not_block(self):
        hosts = "# 127.0.0.1 su.lge.com\n192.168.1.5 su.lge.com\n127.0.0.1 localhost # snu.lge.com\n127.0.0.1 su.lge.com.example\n"
        u = self.status(hosts=hosts)[1]["updateBlock"]
        self.assertEqual((u["state"], list(u["hosts"].values())), ("open", [False] * 4))

    def test_a_missing_hosts_file_is_open_and_a_bad_argument_is_refused(self):
        self.assertEqual(self.status()[1]["updateBlock"]["state"], "open")
        rc, doc = self.status("on")
        self.assertEqual((rc, doc["ok"]), (1, False))

    def test_lumatv_status_carries_it(self):
        with open(TOOL) as f:
            text = f.read()
        self.assertIn('"updateBlock":%s}', text)
        self.assertIn('"$compositor" "$updateblock"', text)
        self.assertIn("updateblock) verb_updateblock", text)


class WholeTool(unittest.TestCase):
    """The real tools/lumatv run as a command, in a copy of the app tree whose data directory is a temporary one: the verbs must find
    their functions (a paste into the middle of another function once made `updateblock_json: not found` and `verb_updateblock: not found`)."""

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
        # brand.sh insists on a data directory under /var/lib; the copy of the tool keeps every line but this one, which points at a temporary one
        tool = os.path.join(self.app, "tools", "lumatv")
        with open(tool) as f:
            text = f.read()
        self.assertEqual(text.count('\nSTATE="$BRAND_DATA_DIR"\n'), 1)
        with open(tool, "w") as f:
            f.write(text.replace('\nSTATE="$BRAND_DATA_DIR"\n', '\nSTATE="%s"\n' % os.path.join(self.tmp.name, "data")))
        self.flag = os.path.join(self.tmp.name, "flag")
        self.hosts = os.path.join(self.tmp.name, "hosts")
        with open(self.hosts, "w") as f:
            f.write(HBC)
        open(self.flag, "w").close()

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, *args):
        env = dict(os.environ, UB_FLAG=self.flag, UB_HOSTS=self.hosts)
        p = subprocess.run(["sh", os.path.join(self.app, "tools", "lumatv")] + list(args), env=env, capture_output=True, text=True, timeout=120)
        return p.returncode, p.stdout, p.stderr

    def test_the_verb_answers(self):
        rc, out, err = self.run_tool("updateblock", "status")
        self.assertEqual((rc, "not found" in err), (0, False), err)
        u = json.loads(out)["updateBlock"]
        self.assertEqual((u["state"], u["live"], u["flag"]), ("blocked", True, True))

    def test_status_carries_the_key_and_prints_no_shell_error(self):
        rc, out, err = self.run_tool("status")
        self.assertNotIn("not found", err)
        self.assertEqual(json.loads(out)["updateBlock"]["state"], "blocked")
        os.remove(self.flag)
        self.assertEqual(json.loads(self.run_tool("status")[1])["updateBlock"]["state"], "hosts-only")

    def test_a_bad_argument_is_refused(self):
        rc, out, err = self.run_tool("updateblock", "on")
        self.assertEqual((rc, json.loads(out)["ok"]), (1, False))


if __name__ == "__main__":
    unittest.main()
