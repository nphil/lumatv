"""Run with: cd tests && python3 -m unittest test_ha_plugin_package

The Home Assistant plugin's package on its way to the TV. helper/build.sh makes it (LumaShow's manifest as it is, the program built
here); tools/build-ipk.sh requires both files and puts them where `lumatv-helper plugin-enable` looks, `<app dir>/payload/plugins/
<id>` (helper/app/src/plugins/install.rs, `packaged`). Nothing here needs the built files: whether they exist depends on the checkout.
"""
import os
import re
import subprocess
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
IPK = os.path.join(ROOT, "tools", "build-ipk.sh")
BUILD = os.path.join(ROOT, "helper", "build.sh")


def text(path):
    with open(path) as f:
        return f.read()


class Package(unittest.TestCase):
    def test_the_dry_run_names_the_manifest_and_the_program(self):
        r = subprocess.run(["sh", IPK, "--dry-run"], capture_output=True, text=True, cwd=ROOT, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        for want in ("helper/dist/plugins/home-assistant/plugin.json", "helper/dist/plugins/home-assistant/bin/home-assistant"):
            self.assertRegex(r.stdout, r"[\u2713\u2717] %s\b" % re.escape(want), want)

    def test_the_ipk_puts_the_package_where_the_helper_looks_and_the_program_can_run(self):
        script = text(IPK)
        self.assertIn('"$DATA_ROOT/payload/plugins/home-assistant/', script)
        self.assertRegex(script, r'cp helper/dist/plugins/home-assistant/plugin\.json "\$DATA_ROOT/payload/plugins/home-assistant/"')
        self.assertRegex(script, r'cp helper/dist/plugins/home-assistant/bin/home-assistant "\$DATA_ROOT/payload/plugins/home-assistant/bin/"')
        self.assertRegex(script, r'chmod 755 "\$DATA_ROOT/payload/plugins/home-assistant/bin/home-assistant"')
        rust = text(os.path.join(ROOT, "helper", "app", "src", "plugins", "install.rs"))
        self.assertRegex(rust, r'\.join\("payload"\)\s*\.join\("plugins"\)\s*\.join\(id\)', "the helper looks in payload/plugins/<id> of the app")

    def test_a_missing_package_says_which_script_makes_it(self):
        script = text(IPK)
        self.assertRegex(script, r"\*helper/dist/lumatv-helper\*\|\*helper/dist/plugins/\*\)")
        self.assertIn("helper/build.sh", script)

    def test_the_build_makes_the_package_from_lumashows_manifest_unchanged(self):
        build = text(BUILD)
        self.assertIn("PLUGIN=lumatv-plugin-home-assistant", build)
        self.assertIn('-p lumatv-helper -p "$PLUGIN"', build, "the release build makes the helper and the plugin's program together")
        self.assertIn('PLUGIN_MANIFEST="$HERE/../../LumaShow/lumashow/plugins/home-assistant/plugin.json"', build)
        self.assertIn('cp -f "$PLUGIN_MANIFEST" "$pkg/plugin.json"', build, "copied, never rewritten: the approval is the hash of its text")
        self.assertIn('pkg="dist/plugins/home-assistant"', build)
        self.assertIn("bin/home-assistant", build)

    def test_the_scripts_are_valid_shell(self):
        for path in (IPK, BUILD):
            self.assertEqual(subprocess.run(["sh", "-n", path]).returncode, 0, path)


if __name__ == "__main__":
    unittest.main()
