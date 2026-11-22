"""Run with: python3 -m unittest tests.test_gameapp

How a round carries LumaGame: game/build-ipk.sh packs a stand-in tree of its inputs (the real builds need the cross toolchain and the
pinned GameStream release, here a stand-in fetch), and tools/gameapp.sh, the functions `lumatv game` and boot use, runs under sh with a
fake `luna-send` that plays appInstallService and a fake `pgrep`. Shown here: the package's contents and content-named build, the
install, update and removal, and `adopt-gfn` copying the GeForce NOW app's sign-in into LumaGame's gfn engine directory.
"""
import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIB = os.path.join(ROOT, "tools", "gameapp.sh")
APP = "org.nphil.lumagame"

LUNA_SEND = r'''#!/bin/sh
echo "$*" >> "$FAKE/calls"
for last; do :; done
case "$*" in
  *appInstallService/dev/install*)
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
FETCH = '''#!/bin/sh
mkdir -p "$1/engines/gamestream"
echo '{"id":"gamestream"}' > "$1/engines/gamestream/engine.json"
echo stand-in > "$1/engines/gamestream/lumagame-gamestream"
'''


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


class Packed:
    """game/build-ipk.sh run on a tree that has its script and package files and stand-ins for the programs it packs."""

    def __init__(self, tmp):
        self.root = os.path.join(tmp, "repo")
        game = os.path.join(self.root, "game")
        os.makedirs(game)
        shutil.copy(os.path.join(ROOT, "game", "build-ipk.sh"), game)
        shutil.copytree(os.path.join(ROOT, "game", "package"), os.path.join(game, "package"))
        os.makedirs(os.path.join(game, "crates", "lumagame-engine-gfn"))
        shutil.copy(os.path.join(ROOT, "game", "crates", "lumagame-engine-gfn", "engine.json"), os.path.join(game, "crates", "lumagame-engine-gfn"))
        punktfunk = os.path.join(self.root, "game", "engines", "punktfunk")
        os.makedirs(punktfunk)
        for name in ("engine.json", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.txt"):
            shutil.copy(os.path.join(ROOT, "game", "engines", "punktfunk", name), punktfunk)
        steam = os.path.join(game, "engines", "steam")
        os.makedirs(steam)
        shutil.copy(os.path.join(ROOT, "game", "engines", "steam", "engine.json"), steam)
        write(os.path.join(game, "tools", "fetch-gamestream.sh"), FETCH, 0o755)
        self.set_core("core v1")
        for rel in ("gfn/feeder/build/arm/lumagame-main", "rustui/build/game/bin64/lumagame-ui",
                    "game/target/aarch64-unknown-linux-gnu/release/lumagame-engine-gfn", "gfn/build/OpenNOW/LICENSE",
                    "gfn/engine/target/aarch64-unknown-linux-gnu/release/opennow-streamer", "game/engines/punktfunk/bin/lumagame-engine-punktfunk",
                    "game/engines/steam/bin/lumagame-engine-steam",
                    "gfx64/build/out/heapshim.so", "gfx64/launch.sh", "gfx64/build/bundle/lib/ld-linux-aarch64.so.1"):
            write(os.path.join(self.root, rel), "stand-in " + rel + "\n")

    def set_core(self, text):
        write(os.path.join(self.root, "gfn/build/core/aarch64/opennow-core"), text)

    def build(self, dest):
        r = subprocess.run(["sh", os.path.join(self.root, "game", "build-ipk.sh")], capture_output=True, text=True, timeout=60)
        assert r.returncode == 0, r.stderr
        shutil.copy(os.path.join(self.root, "game", "build", APP + "_0.1.0_all.ipk"), dest)
        return dest


def control(ipk):
    out = subprocess.run("ar p %s control.tar.gz | tar -xzO control" % ipk, shell=True, capture_output=True, text=True, check=True).stdout
    return dict(line.split(": ", 1) for line in out.splitlines() if ": " in line)


class GameApp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.d = self.tmp.name
        self.packed = Packed(d)
        appdir = os.path.join(d, "apps", "org.nphil.lumatv")
        self.ipk = os.path.join(appdir, "payload", "game", APP + ".ipk")
        os.makedirs(os.path.dirname(self.ipk))
        self.packed.build(self.ipk)
        bin_dir = os.path.join(d, "bin")
        for name, body in (("luna-send", LUNA_SEND), ("pgrep", PGREP)):
            write(os.path.join(bin_dir, name), body, 0o755)
        self.bases = [os.path.join(d, "base1"), os.path.join(d, "base2")]
        self.dir = os.path.join(d, "apps", APP)
        self.env = dict(os.environ, PATH=bin_dir + os.pathsep + os.environ["PATH"], FAKE=d, APP=APP, APPDIR=appdir, SHORT_ID="gametest",
                        GAME_WAIT="3", LOGLINES=os.path.join(d, "loglines"), GAME_DATA_BASES=" ".join(self.bases),
                        GAME_GFN_LEGACY=os.path.join(d, "legacy"))

    def tearDown(self):
        self.tmp.cleanup()

    def run_lib(self, *calls):
        script = '. "%s"\nlog_line() { echo "$1" >> "$LOGLINES"; }\n' % LIB
        for c in calls:
            script += c + '\necho "rc=$?"\n'
        r = subprocess.run(["sh", "-c", script], env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.split()

    def status(self):
        return json.loads(subprocess.run(["sh", "-c", '. "%s"; game_status' % LIB], env=self.env, capture_output=True, text=True).stdout)

    def installs(self):
        try:
            with open(os.path.join(self.d, "calls")) as f:
                return [c for c in f.read().splitlines() if "dev/install" in c]
        except FileNotFoundError:
            return []

    def test_the_package_carries_every_engine_runnable_and_names_its_build_by_content(self):
        out = subprocess.run("ar p %s data.tar.gz | tar -tvz" % self.ipk, shell=True, capture_output=True, text=True, check=True).stdout
        modes = {line.split()[-1].split(APP + "/", 1)[-1]: line.split()[0] for line in out.splitlines()}
        for name in ("lumagame-main", "run64.sh", "bin64/lumagame-ui", "engines/gfn/bin/lumagame-engine-gfn", "engines/gfn/bin/opennow-core",
                     "engines/gfn/bin/opennow-streamer", "engines/punktfunk/bin/lumagame-engine-punktfunk", "engines/steam/bin/lumagame-engine-steam",
                     "gfx64/launch.sh"):
            self.assertIn("x", modes[name], name)
        for name in ("engines/gfn/engine.json", "engines/punktfunk/engine.json", "engines/steam/engine.json", "engines/gamestream/engine.json",
                     "gfx64/heapshim.so"):
            self.assertIn(name, modes)
        first = control(self.ipk)["Build-Id"]
        self.assertRegex(first, r"^[0-9a-f]{16}$")
        self.packed.set_core("core v2")
        self.assertNotEqual(control(self.packed.build(self.ipk))["Build-Id"], first)

    def test_boot_installs_nothing_the_owner_installs_updates_and_removes(self):
        self.assertEqual(self.run_lib("game_update"), ["rc=0"])
        self.assertEqual(self.installs(), [])
        self.assertEqual(self.run_lib("game_install", "game_install"), ["rc=0", "rc=0"])
        self.assertEqual(len(self.installs()), 1)
        shipped = control(self.ipk)["Build-Id"]
        self.assertEqual(self.status(), {"installed": True, "build": shipped, "shipped": shipped, "current": True, "running": False, "data": ""})
        self.packed.set_core("core v2")
        self.packed.build(self.ipk)
        open(os.path.join(self.d, "running"), "w").close()
        self.assertEqual(self.run_lib("game_update"), ["rc=0"])
        self.assertEqual(len(self.installs()), 1, "a running app is not updated")
        os.remove(os.path.join(self.d, "running"))
        self.assertEqual(self.run_lib("game_update"), ["rc=0"])
        self.assertTrue(self.status()["current"])
        self.assertEqual(self.run_lib("game_remove", "game_remove"), ["rc=0", "rc=0"])
        self.assertFalse(os.path.exists(self.dir))

    def gfn_sign_in(self, base):
        old = os.path.join(base, "org.nphil.lumatv.gfn")
        write(os.path.join(old, "config", "opennow", "session.json"), "token", 0o600)
        write(os.path.join(old, "data", "catalog.json"), "{}", 0o644)
        write(os.path.join(old, "prefs.json"), '{"tier":"4k"}', 0o600)
        return old

    def test_adopt_gfn_copies_the_sign_in_into_the_engine_directory_keeping_modes_and_originals(self):
        os.makedirs(os.path.join(self.bases[0], APP))
        new = os.path.join(self.bases[1], APP)
        os.makedirs(os.path.join(new, "cache"))
        old = self.gfn_sign_in(self.bases[1])
        self.assertEqual(self.run_lib("game_adopt_gfn"), ["rc=0"])
        dest = os.path.join(new, "engines", "gfn")
        with open(os.path.join(dest, "config", "opennow", "session.json")) as f:
            self.assertEqual(f.read(), "token")
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(dest, "config", "opennow", "session.json")).st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(dest, "data", "catalog.json")).st_mode), 0o644)
        self.assertEqual(stat.S_IMODE(os.stat(dest).st_mode), 0o700)
        engines = os.stat(os.path.join(new, "engines"))
        self.assertEqual((stat.S_IMODE(engines.st_mode), engines.st_uid), (0o755, os.stat(new).st_uid), "engines/ as the UI makes it")
        self.assertTrue(os.path.exists(os.path.join(dest, "prefs.json")))
        self.assertTrue(os.path.exists(os.path.join(old, "config", "opennow", "session.json")), "the original stays")
        self.assertEqual(os.listdir(os.path.join(new, "engines")), ["gfn"], "no staging directory is left")
        self.assertEqual(self.status()["data"], new)

    def test_adopt_gfn_needs_a_started_lumagame_and_never_writes_over_a_sign_in(self):
        self.gfn_sign_in(self.bases[0])
        self.assertEqual(self.run_lib("game_adopt_gfn"), ["rc=1"])
        new = os.path.join(self.bases[0], APP)
        os.makedirs(os.path.join(new, "cache"))
        write(os.path.join(new, "engines", "gfn", "config", "opennow", "session.json"), "newer", 0o600)
        self.assertEqual(self.run_lib("game_adopt_gfn"), ["rc=1"])
        with open(os.path.join(new, "engines", "gfn", "config", "opennow", "session.json")) as f:
            self.assertEqual(f.read(), "newer")


if __name__ == "__main__":
    unittest.main()
