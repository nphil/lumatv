"""Run with: python3 -m unittest tests.test_lumatv_lgextras

`lumatv lgextras off|on|status`: declutter's opt group `lgextras` in the extras' own terms. The real tools/lumatv runs in a copy of the
app tree whose declutter.sh is a stand-in that logs what it is asked and answers as the tool does.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

DECLUTTER = r'''#!/bin/sh
echo "$*" >> "$DIR/calls"
[ "$1" = "$FAIL" ] && { printf '{"ok":false,"error":"another run holds the lock"}\n'; exit 1; }
case "$1" in
  on) printf '{"ok":true,"on":"%s"}\n' "$3" ;;
  off) printf '{"ok":true,"off":"%s"}\n' "$3" ;;
  persist) printf '{"ok":true,"persist":true}\n' ;;
  status) cat "$DIR/status.json" ;;
esac
'''

STATUS = {"ok": True, "profile": "lean", "persist": True, "skip": False, "memAvailableKb": 512000,
          "groups": {"lgextras": {"on": True, "inTarget": "12/13", "activities": {
              "com.webos.service.homeconnect.start": "absent", "com.webos.service.billing.start": "unsatisfied",
              "com.webos.service.magicnum.getCountry": None}}}}


class LgExtras(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = tmp.name
        app = os.path.join(self.dir, "app")
        os.makedirs(os.path.join(app, "tools"))
        for f in os.listdir(os.path.join(ROOT, "tools")):
            if os.path.isfile(os.path.join(ROOT, "tools", f)):
                shutil.copy(os.path.join(ROOT, "tools", f), os.path.join(app, "tools", f))
        for f in ("appinfo.json", "brand.json"):
            shutil.copy(os.path.join(ROOT, f), app)
        # brand.sh insists on a data directory under /var/lib; the copy points at a temporary one.
        self.tool = os.path.join(app, "tools", "lumatv")
        with open(self.tool) as f:
            text = f.read()
        with open(self.tool, "w") as f:
            f.write(text.replace('\nSTATE="$BRAND_DATA_DIR"\n', '\nSTATE="%s"\n' % os.path.join(self.dir, "data")))
        with open(os.path.join(app, "tools", "declutter.sh"), "w") as f:
            f.write(DECLUTTER)
        with open(os.path.join(self.dir, "status.json"), "w") as f:
            json.dump(STATUS, f)

    def lgextras(self, *args, fail=""):
        p = subprocess.run(["sh", self.tool, "lgextras"] + list(args), env=dict(os.environ, DIR=self.dir, FAIL=fail),
                           capture_output=True, text=True, timeout=60)
        self.assertNotIn("not found", p.stderr)
        try:
            with open(os.path.join(self.dir, "calls")) as f:
                calls = f.read().splitlines()
            os.remove(os.path.join(self.dir, "calls"))
        except FileNotFoundError:
            calls = []
        return p.returncode, json.loads(p.stdout), calls

    def test_off_switches_the_group_on_and_arms_the_boot_hook_and_on_switches_it_off(self):
        self.assertEqual(self.lgextras("off"), (0, {"ok": True, "lgextras": "off"}, ["on --group lgextras", "persist on"]))
        self.assertEqual(self.lgextras("on"), (0, {"ok": True, "lgextras": "on"}, ["off --group lgextras"]))

    def test_status_speaks_of_the_extras_and_lists_each_activity(self):
        self.assertEqual(self.lgextras(), (0, {"ok": True, "lgextras": "off", "bootHook": True,
                                               "activities": STATUS["groups"]["lgextras"]["activities"]},
                                           ["status --group lgextras"]))

    def test_a_refusal_is_passed_on_and_a_bad_argument_is_refused(self):
        rc, out, calls = self.lgextras("off", fail="on")
        self.assertEqual((rc, out["ok"], calls), (1, False, ["on --group lgextras"]))
        rc, out, calls = self.lgextras("restore")
        self.assertEqual((rc, out["ok"], calls), (1, False, []))


if __name__ == "__main__":
    unittest.main()
