"""Run with: python3 -m unittest tests.test_gfnapp

How a round carries the GeForce NOW app: gfn/build-ipk.sh packs a stand-in tree of its inputs (the real builds need the cross toolchain),
and tools/gfnapp.sh, the functions `lumatv apply` and `lumatv gfn` use, runs under sh with a fake `luna-send` that plays appInstallService
(it unpacks the ipk it is given under a fake apps directory, or removes the directory) and a fake `pgrep`. Shown here: the package's build
id follows its content and not the version (the app on the set once stayed at the 2026-10-26 build under an unchanged 0.1.0), an
installed app that is stale is updated by apply, an app that is not installed is not installed by apply, a streaming app is not
overwritten, the explicit install and removal, an incomplete install put right, and the status object.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIB = os.path.join(ROOT, "tools", "gfnapp.sh")
APP = "org.nphil.lumatv.gfn"

LUNA_SEND = r'''#!/bin/sh
echo "$*" >> "$FAKE/calls"
for last; do :; done
case "$*" in
  *appInstallService/dev/install*)
    [ -e "$FAKE/install-does-nothing" ] && exit 0
    ipk=$(printf '%s' "$last" | sed -n 's/.*"ipkUrl":"\([^"]*\)".*/\1/p')
    mkdir -p "$FAKE/apps"
    rm -rf "$FAKE/apps/$APP"
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


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


class Packed:
    """gfn/build-ipk.sh run on a tree that has its script and package files and stand-ins for the programs it packs."""

    def __init__(self, tmp):
        self.root = os.path.join(tmp, "repo")
        gfn = os.path.join(self.root, "gfn")
        os.makedirs(gfn)
        shutil.copy(os.path.join(ROOT, "gfn", "build-ipk.sh"), gfn)
        shutil.copytree(os.path.join(ROOT, "gfn", "package"), os.path.join(gfn, "package"))
        self.set_binary("streamer", "engine v1")
        for rel in ("gfn/feeder/build/arm/lumatv-gfn-feeder", "gfn/engine/target/aarch64-unknown-linux-gnu/release/lumatv-gfn-latency",
                    "gfn/engine/target/aarch64-unknown-linux-gnu/release/liblumatv_gfn_sendtap.so", "gfn/build/core/aarch64/opennow-core",
                    "rustui/build/gfn/bin64/lumatv-gfn-app", "gfx64/build/out/heapshim.so", "gfx64/launch.sh",
                    "gfx64/build/bundle/lib/ld-linux-aarch64.so.1", "gfx64/build/bundle/lib/libmali.so", "payload/wallpaper/wall_1080.png"):
            write(os.path.join(self.root, rel), "stand-in " + rel + "\n")

    def set_binary(self, name, text):
        assert name == "streamer"
        write(os.path.join(self.root, "gfn/engine/target/aarch64-unknown-linux-gnu/release/opennow-streamer"), text)

    def build(self, dest):
        r = subprocess.run(["sh", os.path.join(self.root, "gfn", "build-ipk.sh")], capture_output=True, text=True, timeout=120)
        assert r.returncode == 0, r.stderr
        path = os.path.join(self.root, "gfn", "build", APP + "_0.1.0_all.ipk")
        shutil.copy(path, dest)
        return dest


def control(ipk):
    out = subprocess.run("ar p %s control.tar.gz | tar -xzO control" % ipk, shell=True, capture_output=True, text=True, check=True).stdout
    return dict(line.split(": ", 1) for line in out.splitlines() if ": " in line)


class GfnApp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.d = d
        self.packed = Packed(d)
        self.appdir = os.path.join(d, "apps", "org.nphil.lumatv")
        os.makedirs(os.path.join(self.appdir, "payload", "gfn"))
        self.ipk = os.path.join(self.appdir, "payload", "gfn", APP + ".ipk")
        self.packed.build(self.ipk)
        self.bin = os.path.join(d, "bin")
        for name, body in (("luna-send", LUNA_SEND), ("pgrep", PGREP)):
            write(os.path.join(self.bin, name), body, 0o755)
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], FAKE=d, APP=APP, APPDIR=self.appdir, APP_ID="org.nphil.lumatv",
                        SHORT_ID="gfntest", GFN_WAIT="3", LOGLINES=os.path.join(d, "loglines"), GFN_DIR=os.path.join(d, "apps", APP))
        self.dir = self.env["GFN_DIR"]

    def tearDown(self):
        self.tmp.cleanup()

    def run_lib(self, *calls):
        script = '. "%s"\nlog_line() { echo "$1" >> "$LOGLINES"; }\n' % LIB
        for c in calls:
            script += c + '\necho "rc=$?"\n'
        r = subprocess.run(["sh", "-c", script], env=self.env, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.split()

    def status(self):
        return json.loads(subprocess.run(["sh", "-c", '. "%s"; gfn_status' % LIB], env=self.env, capture_output=True, text=True).stdout)

    def installs(self):
        try:
            with open(os.path.join(self.d, "calls")) as f:
                return [c for c in f.read().splitlines() if "dev/install" in c]
        except FileNotFoundError:
            return []

    def log(self):
        try:
            with open(self.env["LOGLINES"]) as f:
                return f.read()
        except FileNotFoundError:
            return ""

    def repack(self, text):
        """A new engine build under the same version."""
        self.packed.set_binary("streamer", text)
        self.packed.build(self.ipk)

    def test_the_build_id_follows_the_content_and_not_the_version(self):
        first = control(self.ipk)
        self.assertEqual(first["Version"], "0.1.0")
        self.assertRegex(first["Build-Id"], r"^[0-9a-f]{16}$")
        self.packed.build(self.ipk)
        self.assertEqual(control(self.ipk)["Build-Id"], first["Build-Id"], "the same inputs give the same build")
        self.repack("engine v2")
        second = control(self.ipk)
        self.assertEqual(second["Version"], "0.1.0")
        self.assertNotEqual(second["Build-Id"], first["Build-Id"])

    def test_apply_does_not_put_the_app_on_a_set_that_has_none(self):
        self.assertEqual(self.run_lib("gfn_update"), ["rc=0"])
        self.assertEqual(self.installs(), [])
        self.assertFalse(os.path.exists(self.dir))
        self.assertFalse(self.status()["installed"])

    def test_the_explicit_install_then_the_second_is_a_no_op(self):
        self.assertEqual(self.run_lib("gfn_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 1)
        self.assertIn('"id":"%s"' % APP, self.installs()[0])
        self.assertIn('"ipkUrl":"/tmp/gfntest-gfn.ipk"', self.installs()[0])
        self.assertTrue(os.access(os.path.join(self.dir, "lumatv-gfn-feeder"), os.X_OK))
        self.assertTrue(os.access(os.path.join(self.dir, "bin64", "opennow-streamer"), os.X_OK))
        self.assertFalse(os.path.exists("/tmp/gfntest-gfn.ipk"), "the copy in /tmp is removed")
        shipped = control(self.ipk)["Build-Id"]
        self.assertEqual(self.status(), {"installed": True, "build": shipped, "shipped": shipped, "current": True, "running": False})
        self.assertEqual(self.run_lib("gfn_install", "gfn_update"), ["rc=0", "rc=0"])
        self.assertEqual(len(self.installs()), 1, "an app at the shipped build is left alone")

    def test_apply_brings_a_stale_installed_app_to_the_shipped_build(self):
        self.run_lib("gfn_install")
        self.repack("engine v2")
        self.assertFalse(self.status()["current"])
        self.assertEqual(self.run_lib("gfn_update"), ["rc=0"])
        self.assertEqual(len(self.installs()), 2)
        self.assertTrue(self.status()["current"])
        with open(os.path.join(self.dir, "bin64", "opennow-streamer.bin")) as f:
            self.assertEqual(f.read(), "engine v2")

    def test_an_app_from_before_build_ids_counts_as_stale(self):
        self.run_lib("gfn_install")
        os.remove(os.path.join(self.dir, "build-id"))
        self.assertEqual(self.status()["build"], "")
        self.assertEqual(self.run_lib("gfn_update"), ["rc=0"])
        self.assertEqual(len(self.installs()), 2)
        self.assertTrue(self.status()["current"])

    def test_apply_leaves_a_streaming_app_alone_and_the_explicit_install_closes_it(self):
        self.run_lib("gfn_install")
        self.repack("engine v2")
        open(os.path.join(self.d, "running"), "w").close()
        self.assertEqual(self.run_lib("gfn_update"), ["rc=0"])
        self.assertEqual(len(self.installs()), 1)
        self.assertIn("running, not updated", self.log())
        self.assertEqual(self.run_lib("gfn_install"), ["rc=0"])
        self.assertEqual(len(self.installs()), 2)
        self.assertFalse(os.path.exists(os.path.join(self.d, "running")), "SAM was asked to close it first")
        self.assertTrue(self.status()["current"])

    def test_an_incomplete_install_is_put_right(self):
        self.run_lib("gfn_install")
        os.remove(os.path.join(self.dir, "bin64", "lumatv-gfn-app"))
        status = self.status()
        self.assertFalse(status["installed"])
        self.assertFalse(status["current"])
        self.assertEqual(self.run_lib("gfn_update"), ["rc=0"])
        self.assertTrue(self.status()["current"])

    def test_an_install_that_never_lands_fails_and_says_so(self):
        open(os.path.join(self.d, "install-does-nothing"), "w").close()
        self.assertEqual(self.run_lib("gfn_install"), ["rc=1"])
        self.assertIn("gfn app install did not complete", self.log())
        self.assertFalse(os.path.exists("/tmp/gfntest-gfn.ipk"))

    def test_a_package_without_the_ipk_does_not_stop_apply_and_fails_the_explicit_install(self):
        os.remove(self.ipk)
        self.assertEqual(self.run_lib("gfn_update", "gfn_install"), ["rc=0", "rc=1"])
        self.assertIn("no ipk in the package", self.log())

    def test_remove_takes_the_app_away_and_is_idempotent(self):
        self.run_lib("gfn_install")
        self.assertEqual(self.run_lib("gfn_remove"), ["rc=0"])
        self.assertFalse(os.path.exists(self.dir))
        self.assertEqual(self.run_lib("gfn_remove"), ["rc=0"])
        self.assertFalse(self.status()["installed"])


class Mode(unittest.TestCase):
    def test_the_build_script_keeps_its_executable_programs(self):
        # The loader and programs are marked by the build; a lost mode would be a package that installs and cannot start.
        with tempfile.TemporaryDirectory() as d:
            ipk = Packed(d).build(os.path.join(d, "g.ipk"))
            out = subprocess.run("ar p %s data.tar.gz | tar -tvz" % ipk, shell=True, capture_output=True, text=True, check=True).stdout
            modes = {line.split()[-1]: line.split()[0] for line in out.splitlines()}
            for name in ("lumatv-gfn-feeder", "run64.sh", "bin64/opennow-streamer", "bin64/lumatv-gfn-app"):
                path = [k for k in modes if k.endswith("/" + name)]
                self.assertEqual(len(path), 1, name)
                self.assertIn("x", modes[path[0]], name)


if __name__ == "__main__":
    unittest.main()
