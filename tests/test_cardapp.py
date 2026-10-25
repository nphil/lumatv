"""Run with: python3 -m unittest tests.test_cardapp

tools/cardapp.sh, the functions `lumatv` uses to put the card app on the set and take it off, run under sh with a fake `luna-send`
that plays appInstallService (it unpacks the ipk it is given under a fake apps directory, or removes the directory) and a fake
`pgrep`: the first install, the idempotent second, an update, an incomplete install put right, a failed install, the running app
closed first, the removal, and the status object. The ipk is built here the way tools/build-ipk.sh builds one.
"""
import json
import os
import stat
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIB = os.path.join(ROOT, "tools", "cardapp.sh")
APP = "org.nphil.lumatv.card"

# appInstallService as far as the library needs it: dev/install unpacks the ipk at "ipkUrl" under $FAKE/apps, dev/remove deletes the
# app's directory, closeByAppId ends the pretend process; every call is recorded.
LUNA_SEND = r'''#!/bin/sh
echo "$*" >> "$FAKE/calls"
for last; do :; done
case "$*" in
  *appInstallService/dev/install*)
    [ -e "$FAKE/install-does-nothing" ] && exit 0
    ipk=$(printf '%s' "$last" | sed -n 's/.*"ipkUrl":"\([^"]*\)".*/\1/p')
    mkdir -p "$FAKE/apps"
    ar p "$ipk" data.tar.gz | tar -xz -C "$FAKE/apps" --strip-components=4 ;;
  *appInstallService/dev/remove*) rm -rf "$FAKE/apps/$APP" ;;
  *closeByAppId*) rm -f "$FAKE/running" ;;
esac
exit 0
'''
PGREP = '''#!/bin/sh
[ -e "$FAKE/running" ] && echo 4242 && exit 0
exit 1
'''


def build_ipk(path, version, missing=()):
    """An ipk with the package's shape: usr/palm/applications/<id>/ with appinfo, the launcher, the app and the runtime."""
    with tempfile.TemporaryDirectory() as d:
        root = os.path.join(d, "data", "usr", "palm", "applications", APP)
        files = {"appinfo.json": json.dumps({"id": APP, "version": version, "type": "native", "main": "bin/lumatv-card"}),
                 "bin/lumatv-card": "#!/bin/sh\n", "bin64/lumatv-card-app": "#!/bin/sh\n", "gfx64/launch.sh": "#!/bin/sh\n", "gfx64/heapshim.so": "so"}
        for name, text in files.items():
            if name in missing:
                continue
            p = os.path.join(root, name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                f.write(text)
            if not name.endswith((".json", ".so")):
                os.chmod(p, 0o755)
        os.makedirs(os.path.join(d, "control"))
        with open(os.path.join(d, "control", "control"), "w") as f:
            f.write("Package: %s\nVersion: %s\nArchitecture: all\n" % (APP, version))
        with open(os.path.join(d, "debian-binary"), "w") as f:
            f.write("2.0\n")
        subprocess.run(["tar", "-czf", os.path.join(d, "data.tar.gz"), "-C", os.path.join(d, "data"), "."], check=True)
        subprocess.run(["tar", "-czf", os.path.join(d, "control.tar.gz"), "-C", os.path.join(d, "control"), "control"], check=True)
        if os.path.exists(path):
            os.remove(path)
        subprocess.run(["ar", "rc", path, os.path.join(d, "debian-binary"), os.path.join(d, "control.tar.gz"), os.path.join(d, "data.tar.gz")],
                       check=True)


class CardApp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.d = d
        self.appdir = os.path.join(d, "apps", "org.nphil.lumatv")
        os.makedirs(os.path.join(self.appdir, "payload", "card"))
        self.ipk = os.path.join(self.appdir, "payload", "card", APP + ".ipk")
        build_ipk(self.ipk, "0.1.0")
        self.bin = os.path.join(d, "bin")
        os.makedirs(self.bin)
        for name, body in (("luna-send", LUNA_SEND), ("pgrep", PGREP)):
            path = os.path.join(self.bin, name)
            with open(path, "w") as f:
                f.write(body)
            os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], FAKE=d, APP=APP, APPDIR=self.appdir,
                        APP_ID="org.nphil.lumatv", SHORT_ID="lumatv", CARD_WAIT="3", LOGLINES=os.path.join(d, "loglines"))
        self.env["CARD_DIR"] = os.path.join(d, "apps", APP)
        self.card_dir = self.env["CARD_DIR"]

    def tearDown(self):
        self.tmp.cleanup()

    def run_lib(self, *calls):
        script = '. "%s"\nlog_line() { echo "$1" >> "$LOGLINES"; }\n' % LIB
        for c in calls:
            script += c + '\necho "rc=$?"\n'
        r = subprocess.run(["sh", "-c", script], env=self.env, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.split()

    def calls(self):
        try:
            with open(os.path.join(self.d, "calls")) as f:
                return f.read().splitlines()
        except FileNotFoundError:
            return []

    def log(self):
        try:
            with open(self.env["LOGLINES"]) as f:
                return f.read()
        except FileNotFoundError:
            return ""

    def installs(self):
        return [c for c in self.calls() if "dev/install" in c]

    def test_the_first_install_puts_the_app_on_and_the_second_does_nothing(self):
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 1)
        self.assertIn('"id":"%s"' % APP, self.installs()[0])
        self.assertIn('"ipkUrl":"/tmp/lumatv-card.ipk"', self.installs()[0])
        self.assertTrue(os.access(os.path.join(self.card_dir, "bin", "lumatv-card"), os.X_OK))
        self.assertFalse(os.path.exists("/tmp/lumatv-card.ipk"), "the copy in /tmp is removed")
        self.assertIn("card app 0.1.0 installed", self.log())
        status = json.loads(subprocess.run(["sh", "-c", '. "%s"; card_status' % LIB], env=self.env, capture_output=True, text=True).stdout)
        self.assertEqual(status, {"installed": True, "version": "0.1.0", "shipped": "0.1.0", "running": False})
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 1, "an app that is there at the shipped version is left alone")

    def test_a_new_shipped_version_updates_and_an_incomplete_install_is_put_right(self):
        self.run_lib("card_install")
        build_ipk(self.ipk, "0.2.0")
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 2)
        with open(os.path.join(self.card_dir, "appinfo.json")) as f:
            self.assertEqual(json.load(f)["version"], "0.2.0")
        os.remove(os.path.join(self.card_dir, "bin64", "lumatv-card-app"))
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 3, "a missing 64-bit app means the install is done again")

    def test_an_install_that_does_not_complete_is_reported_and_leaves_nothing_in_tmp(self):
        open(os.path.join(self.d, "install-does-nothing"), "w").close()
        self.assertEqual(self.run_lib("card_install"), ["rc=1"])
        self.assertIn("card app install did not complete", self.log())
        self.assertFalse(os.path.exists("/tmp/lumatv-card.ipk"))

    def test_an_ipk_without_the_runtime_is_not_taken_for_installed(self):
        build_ipk(self.ipk, "0.1.0", missing=("gfx64/heapshim.so",))
        self.assertEqual(self.run_lib("card_install"), ["rc=1"])

    def test_a_running_app_is_closed_through_sam_before_the_install(self):
        self.run_lib("card_install")
        build_ipk(self.ipk, "0.2.0")
        open(os.path.join(self.d, "running"), "w").close()
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        calls = self.calls()
        close = next(i for i, c in enumerate(calls) if "closeByAppId" in c)
        last_install = max(i for i, c in enumerate(calls) if "dev/install" in c)
        self.assertLess(close, last_install)
        self.assertIn('"id":"%s"' % APP, calls[close])

    def test_remove_takes_the_app_off_and_an_absent_app_costs_no_call(self):
        self.assertEqual(self.run_lib("card_remove"), ["rc=0"])
        self.assertEqual(self.calls(), [])
        self.run_lib("card_install")
        self.assertEqual(self.run_lib("card_remove"), ["rc=0"])
        self.assertFalse(os.path.exists(self.card_dir))
        self.assertTrue(any("dev/remove" in c for c in self.calls()))
        self.assertIn("card app removed", self.log())

    def test_no_shipped_ipk_is_nothing_to_do(self):
        os.remove(self.ipk)
        self.assertEqual(self.run_lib("card_install"), ["rc=0"])
        self.assertEqual(self.calls(), [])


class Packaging(unittest.TestCase):
    """The ipk build names the card app's inputs and ships its installer; whether they exist depends on the checkout (they are built)."""

    def test_the_dry_run_lists_what_the_card_app_is_packed_from(self):
        r = subprocess.run(["sh", os.path.join(ROOT, "tools", "build-ipk.sh"), "--dry-run"], capture_output=True, text=True, cwd=ROOT, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        for want in ("card/appinfo.json", "card/splash.png", "rustui/build/card/bin/lumatv-card", "rustui/build/card/bin64/lumatv-card-app",
                     "gfx64/launch.sh", "gfx64/build/out/heapshim.so", "gfx64/build/bundle/lib/ld-linux-aarch64.so.1", "tools/cardapp.sh"):
            self.assertRegex(r.stdout, r"[\u2713\u2717] %s\b" % want.replace(".", r"\."), want)

    def test_the_card_apps_appinfo_is_a_native_app_that_registers_with_sam(self):
        with open(os.path.join(ROOT, "card", "appinfo.json")) as f:
            info = json.load(f)
        self.assertEqual(info["id"], "{{appId}}.card")
        self.assertEqual((info["type"], info["main"], info["nativeLifeCycleInterfaceVersion"]), ("native", "bin/lumatv-card", 2))
        self.assertTrue(info["visible"], "the switcher's list only keeps apps that have a launch point")
        self.assertEqual(info["splashBackground"], "splash.png")

    def test_the_script_is_valid_shell(self):
        for name in ("build-ipk.sh", "cardapp.sh"):
            self.assertEqual(subprocess.run(["sh", "-n", os.path.join(ROOT, "tools", name)]).returncode, 0, name)


if __name__ == "__main__":
    unittest.main()
