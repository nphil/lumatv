"""Run with: python3 -m unittest tests.test_webapps

The shared web theme's app list: `webglass_apps` from tools/lumatv, extracted and run under sh against payload/webapps/apps.txt.
The ids it prints are the apps `lumatv webglass on` mounts the stylesheet on, so what matters is that comments and blank lines
never become an id and that the shipped list holds what the plan names: the Settings app, the Notification Centre and the
Enact sub-apps.
"""
import os
import re
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "lumatv")


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


def apps(src):
    r = subprocess.run(["sh", "-c", 'WEBGLASS_SRC="%s"\n%s webglass_apps' % (src, function("webglass_apps"))], capture_output=True, text=True)
    return r.returncode, r.stdout.split()


class WebglassApps(unittest.TestCase):
    def test_the_shipped_list_names_the_settings_app_and_its_sub_apps(self):
        rc, ids = apps(os.path.join(ROOT, "payload", "webapps"))
        self.assertEqual(rc, 0)
        for want in ("com.palm.app.settings", "com.webos.app.notificationcenter", "com.webos.app.connectionwizard", "com.webos.app.multiviewsettings"):
            self.assertIn(want, ids)
        self.assertEqual(len(ids), len(set(ids)), "an app is listed twice")
        for i in ids:
            self.assertRegex(i, r"^[a-z][a-z0-9-]*(\.[a-z0-9-]+)+$")

    def test_comments_and_blank_lines_are_not_ids_and_a_missing_list_is_empty(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "apps.txt"), "w") as f:
                f.write("# a comment\n\ncom.a.one\n   # an indented one\n\t\ncom.b.two\n")
            self.assertEqual(apps(d)[1], ["com.a.one", "com.b.two"])
            self.assertEqual(apps(os.path.join(d, "none"))[1], [])


if __name__ == "__main__":
    unittest.main()
