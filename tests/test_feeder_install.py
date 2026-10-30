"""Run with: cd tests && python3 -m unittest test_feeder_install

The camera feeder on the set (payload/feeder/feeder.sh, what `lumatv feeder` runs): the program is staged at the path the helper starts it from, its
LS2 role names exactly that path, the hub is asked to rescan only when the role changed, an update stages a new build and nothing else, a package that was
never installed is not touched by an update, remove takes it all away. And the verb's wiring in tools/lumatv: install and remove are the owner's own words,
apply only brings an installed feeder up to date, revert removes it. Run against a temporary state and role directory with a stand-in `ls-control`.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
SH = os.path.join(REPO, "payload", "feeder", "feeder.sh")
ROLE = os.path.join(REPO, "payload", "feeder", "role.json")
TOOL = os.path.join(REPO, "tools", "lumatv")
VPLANE_ROLE = os.path.join(REPO, "payload", "vplane", "role.json")


def tool_text():
    with open(TOOL) as f:
        return f.read()


def function(name):
    """One shell function of tools/lumatv, from its `name() {` line to the closing brace in column 0 (a one-line function is that line)."""
    out, on = [], False
    for line in tool_text().splitlines(True):
        if line.startswith(name + "() {"):
            if line.rstrip().endswith("}"):
                return line
            on = True
        if on:
            out.append(line)
            if line.rstrip() == "}":
                return "".join(out)
    raise AssertionError(name + " not found")


class Script(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="feeder-install-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = os.path.join(self.tmp, "state")
        self.ls2 = os.path.join(self.tmp, "ls2")
        self.bin = os.path.join(self.tmp, "bin")
        self.calls = os.path.join(self.tmp, "ls-control.calls")
        os.mkdir(self.bin)
        self.write(os.path.join(self.bin, "ls-control"), '#!/bin/sh\necho "$@" >> %s\n' % self.calls)
        self.package = self.write(os.path.join(self.tmp, "package-feeder"), "#!/bin/sh\necho one\n")
        self.env = dict(os.environ, LUMATV_LS2_DIR=self.ls2, PATH=self.bin + os.pathsep + os.environ["PATH"])

    def write(self, path, text):
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, 0o755)
        return path

    def run_sh(self, *args):
        return subprocess.run(["sh", SH, *args], env=self.env, capture_output=True, text=True, timeout=30)

    def install(self, package=None):
        return self.run_sh("install", package or self.package, ROLE, self.state)

    def rescans(self):
        try:
            with open(self.calls) as f:
                return f.read().split("\n")[:-1]
        except FileNotFoundError:
            return []

    def exe(self):
        return os.path.join(self.state, "bin", "lumatv-gfn-feeder")

    def role_path(self):
        return os.path.join(self.ls2, "roles.d", "lumatv-feeder.json")

    def manifest_path(self):
        return os.path.join(self.ls2, "manifests.d", "org.nphil.lumatv.feeder.json")

    def status(self):
        return json.loads(self.run_sh("status", self.state).stdout)

    def test_install_stages_the_program_where_the_helper_starts_it_and_writes_a_role_for_exactly_that_path(self):
        done = self.install()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(os.access(self.exe(), os.X_OK))
        with open(self.role_path()) as f:
            role = json.load(f)
        self.assertEqual(role["exeName"], self.exe(), "the role is for the staged path: the helper starts that one")
        with open(self.manifest_path()) as f:
            manifest = json.load(f)
        self.assertEqual((manifest["id"], manifest["roleFiles"]), ("org.nphil.lumatv.feeder", [self.role_path()]))
        self.assertEqual(self.rescans(), ["scan-volatile-dirs", "scan-services"])
        self.assertEqual(self.status(), {"installed": True, "role": True, "texture": False})

    def test_the_role_names_what_the_decoder_library_registers_as_the_players_does(self):
        with open(ROLE) as f:
            mine = json.load(f)
        with open(VPLANE_ROLE) as f:
            theirs = json.load(f)
        self.assertEqual((mine["allowedNames"], mine["permissions"]), (theirs["allowedNames"], theirs["permissions"]))

    def test_installing_again_asks_the_hub_for_nothing_and_a_new_build_is_staged(self):
        self.install()
        self.install()
        self.assertEqual(len(self.rescans()), 2, "one rescan, as the first install made: the role did not change")
        newer = self.write(os.path.join(self.tmp, "newer"), "#!/bin/sh\necho two\n")
        self.assertEqual(self.install(newer).returncode, 0)
        self.assertEqual(subprocess.run([self.exe()], capture_output=True, text=True).stdout.strip(), "two")
        self.assertFalse(os.path.exists(self.exe() + ".new"))

    def test_an_update_stages_a_new_build_over_an_installed_feeder_and_touches_nothing_else(self):
        self.install()
        before = self.rescans()
        newer = self.write(os.path.join(self.tmp, "newer"), "#!/bin/sh\necho two\n")
        done = self.run_sh("update", newer, self.state)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(subprocess.run([self.exe()], capture_output=True, text=True).stdout.strip(), "two")
        self.assertEqual(self.rescans(), before, "the role names the path, which did not change: no rescan")

    def test_an_update_of_a_feeder_nobody_installed_stages_nothing(self):
        done = self.run_sh("update", self.package, self.state)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(os.path.exists(self.exe()))
        self.assertFalse(os.path.exists(self.role_path()))
        self.assertEqual(self.status(), {"installed": False, "role": False, "texture": False})

    def test_remove_takes_the_program_the_role_and_the_manifest_away_and_rescans_once(self):
        self.install()
        before = len(self.rescans())
        self.assertEqual(self.run_sh("remove", self.state).returncode, 0)
        for path in (self.exe(), self.role_path(), self.manifest_path()):
            self.assertFalse(os.path.exists(path), path)
        self.assertEqual(len(self.rescans()), before + 2)
        self.run_sh("remove", self.state)
        self.assertEqual(len(self.rescans()), before + 2, "nothing was there: the hub is not asked again")

    def libs(self, where, version="1"):
        """The decoder libraries of the texture path in a package directory's `lib/`."""
        os.makedirs(os.path.join(where, "lib"), exist_ok=True)
        for name in ("libavcodec-lumatv.so.63", "libavutil-lumatv.so.61"):
            self.write(os.path.join(where, "lib", name), "lib %s %s\n" % (name, version))

    def staged_libs(self):
        d = os.path.join(self.state, "bin", "lib")
        return {n: open(os.path.join(d, n)).read() for n in sorted(os.listdir(d))} if os.path.isdir(d) else {}

    def test_the_decoder_libraries_beside_the_package_s_program_are_staged_beside_the_staged_one_and_reported(self):
        self.libs(self.tmp)                                    # the package: <tmp>/package-feeder and <tmp>/lib/*
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(sorted(self.staged_libs()), ["libavcodec-lumatv.so.63", "libavutil-lumatv.so.61"])
        self.assertEqual(self.status(), {"installed": True, "role": True, "texture": True})
        self.assertEqual(len(self.rescans()), 2, "the libraries need no role: the hub is asked once, for the program's")
        # an update with changed libraries stages them, an unchanged one writes nothing new, and no .new file is left
        before = os.stat(os.path.join(self.state, "bin", "lib", "libavutil-lumatv.so.61")).st_mtime_ns
        self.assertEqual(self.run_sh("update", self.package, self.state).returncode, 0)
        self.assertEqual(os.stat(os.path.join(self.state, "bin", "lib", "libavutil-lumatv.so.61")).st_mtime_ns, before)
        self.libs(self.tmp, "2")
        self.assertEqual(self.run_sh("update", self.package, self.state).returncode, 0)
        self.assertIn("lib libavutil-lumatv.so.61 2", self.staged_libs()["libavutil-lumatv.so.61"])
        self.assertEqual([n for n in os.listdir(os.path.join(self.state, "bin", "lib")) if n.endswith(".new")], [])
        # remove takes them away too
        self.run_sh("remove", self.state)
        self.assertFalse(os.path.exists(os.path.join(self.state, "bin", "lib")))

    def test_a_package_without_libraries_installs_a_feeder_that_has_no_texture_path(self):
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(self.staged_libs(), {})
        self.assertEqual(self.status()["texture"], False)

    def test_a_package_without_the_program_or_the_template_writes_nothing(self):
        done = self.run_sh("install", os.path.join(self.tmp, "missing"), ROLE, self.state)
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(os.path.exists(self.ls2))
        done = self.run_sh("install", self.package, os.path.join(self.tmp, "no-role.json"), self.state)
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(os.path.exists(self.exe()), "the program is not staged when the role cannot be written")

    def test_the_program_is_where_the_helper_starts_it_from(self):
        with open(os.path.join(REPO, "helper", "app", "src", "live", "mod.rs")) as f:
            helper = f.read()
        self.assertIn('data_dir.join("bin/lumatv-gfn-feeder")', helper)


FUNCTIONS = ("json_escape", "emit_error", "feeder_json", "feeder_installed", "feeder_install", "feeder_update", "feeder_remove", "verb_feeder")


class Verb(Script):
    """`lumatv feeder ...` over the real script and a package laid out as the ipk lays it out."""

    def setUp(self):
        super().setUp()
        self.app = os.path.join(self.tmp, "app")
        pf = os.path.join(self.app, "payload", "feeder")
        os.makedirs(pf)
        shutil.copy(SH, os.path.join(pf, "feeder.sh"))
        shutil.copy(ROLE, os.path.join(pf, "role.json"))
        self.write(os.path.join(pf, "lumatv-gfn-feeder"), "#!/bin/sh\necho one\n")
        self.pf = pf

    def sh(self, body, lock=True):
        consts = [l for l in tool_text().split("\n") if re.match(r"FEEDER_[A-Z]+=", l)]
        script = (f'APPDIR="{self.app}"\nSTATE="{self.state}"\nLOGFILE="{self.tmp}/log"\n' + "\n".join(consts) + "\n"
                  + f'take_mutation_lock() {{ {"return 0" if lock else "return 1"}; }}\n'
                  + f'log_line() {{ echo "$1" >> "{self.tmp}/loglines"; }}\n'
                  + "".join(function(n) for n in FUNCTIONS) + body + "\n")
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=60, env=self.env)
        return r.returncode, r.stdout.strip()

    def test_install_status_and_remove_are_the_verbs_and_status_is_the_default(self):
        rc, out = self.sh("verb_feeder")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "feeder": {"installed": False, "role": False, "texture": False}}))
        rc, out = self.sh("verb_feeder install")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "feeder": {"installed": True, "role": True, "texture": False}}))
        self.assertTrue(os.path.exists(self.role_path()))
        rc, out = self.sh("verb_feeder remove")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "feeder": {"installed": False, "role": False, "texture": False}}))
        self.assertFalse(os.path.exists(self.role_path()))

    def test_install_and_remove_wait_for_the_lock_and_say_so_when_they_cannot_have_it(self):
        for word in ("install", "remove"):
            rc, out = self.sh("verb_feeder " + word, lock=False)
            self.assertEqual(rc, 1)
            self.assertFalse(json.loads(out)["ok"])
        self.assertFalse(os.path.exists(self.role_path()))

    def test_a_package_without_the_feeder_says_so_and_runs_nothing(self):
        os.remove(os.path.join(self.pf, "lumatv-gfn-feeder"))
        rc, out = self.sh("verb_feeder install")
        self.assertEqual(rc, 1)
        self.assertIn("no camera feeder", json.loads(out)["error"])
        self.assertEqual(self.sh("feeder_update")[0], 0, "an update of a package that has none is nothing")

    def test_apply_updates_an_installed_feeder_and_never_installs_one(self):
        line = [l.strip() for l in function("apply_run").splitlines() if "feeder_installed" in l]
        self.assertEqual(line, ["if feeder_installed; then feeder_update || true; fi"])
        self.assertNotIn("feeder_install ", function("apply_run"), "the role is the owner's to write")
        self.assertIn("feeder_remove", function("do_revert_internal"), "revert takes it away with its role")
        self.assertEqual(self.sh("feeder_installed")[0], 1)
        self.sh("verb_feeder install")
        self.write(os.path.join(self.pf, "lumatv-gfn-feeder"), "#!/bin/sh\necho two\n")
        rc, _ = self.sh("if feeder_installed; then feeder_update || true; fi")
        self.assertEqual(rc, 0)
        self.assertEqual(subprocess.run([self.exe()], capture_output=True, text=True).stdout.strip(), "two")


if __name__ == "__main__":
    unittest.main()
