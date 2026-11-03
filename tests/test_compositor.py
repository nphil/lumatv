"""Run with: python3 -m unittest tests.test_compositor

tools/compositor.sh, the functions `lumatv` uses to put LumaTV's own compositor in LG's place, run
under sh with a fake mount table and fake `mount`, `umount` and `pgrep`: the boot mount's every
refusal, the switch on and off, the live test the guards use, and the status object. The firmware
check (SHA-256 of LG's binary and libraries against verified.list), the flag's session kind, the
bundle's integrity, the supervisor's own answers and the Home hold at boot are in the classes after
`Compositor`. The library runs under `set -u`, as it does inside tools/lumatv.
"""
import fcntl
import hashlib
import json
import os
import stat
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIB = os.path.join(ROOT, "tools", "compositor.sh")

class StorageGuard(unittest.TestCase):
    def test_reserve_peak_and_unreadable_capacity(self):
        with tempfile.TemporaryDirectory() as d:
            source = os.path.join(d, "incoming")
            os.mkdir(source)
            with open(os.path.join(source, "data"), "wb") as f:
                f.write(b"x" * 4096)
            fake = os.path.join(d, "df")
            with open(fake, "w") as f:
                f.write('#!/bin/sh\n[ "$FAIL_DF" != "$1" ] || exit 1\n'
                        'case "$1" in -Pk) echo "fs $BLOCK_TOTAL 1 $BLOCK_FREE 1% /";;'
                        ' -Pi) echo "fs $INODE_TOTAL 1 $INODE_FREE 1% /";; esac\n')
            os.chmod(fake, 0o755)
            env = dict(os.environ, PATH=d + os.pathsep + os.environ["PATH"],
                       BLOCK_TOTAL="1000000", BLOCK_FREE="200000",
                       INODE_TOTAL="100000", INODE_FREE="20000", FAIL_DF="")
            script = '. "$LIB"; storage_guard "$DEST" "$SOURCE"'
            def run(**kw):
                return subprocess.run(["sh", "-eu", "-c", script],
                                      env=dict(env, LIB=LIB, DEST=d, SOURCE=source, **kw)).returncode
            self.assertEqual(run(), 0)
            self.assertNotEqual(run(BLOCK_FREE="100000"), 0)
            self.assertNotEqual(run(INODE_FREE="10000"), 0)
            self.assertNotEqual(run(FAIL_DF="-Pi"), 0)
            self.assertNotEqual(run(BLOCK_TOTAL="bad"), 0)
            self.assertNotEqual(run(BLOCK_FREE="1000001"), 0)
            self.assertNotEqual(run(INODE_FREE="100001"), 0)
            self.assertNotEqual(run(BLOCK_TOTAL="999999999999999999999999999999"), 0)
            self.assertNotEqual(run(INODE_TOTAL="999999999999999999999999999999"), 0)
            self.assertNotEqual(run(BLOCK_TOTAL="0008"), 0)


MOUNT = '''#!/bin/sh
# mount --bind SRC DST: records DST in the fake mount table
[ "$1" = "--bind" ] || exit 1
[ -e "$FAKE/mount-fails" ] && exit 32
echo "1 2 0:3 / $3 rw - overlay x rw" >> "$FAKE/mountinfo"
echo "mount $2 $3" >> "$FAKE/calls"
'''
UMOUNT = '''#!/bin/sh
t=$1; [ "$t" = "-l" ] && t=$2
grep -vF " $t " "$FAKE/mountinfo" > "$FAKE/mountinfo.new"; mv "$FAKE/mountinfo.new" "$FAKE/mountinfo"
echo "umount $t" >> "$FAKE/calls"
'''
PGREP = '''#!/bin/sh
[ -e "$FAKE/compositor.up" ] && echo 4242 && exit 0
exit 1
'''
# The supervisor's three questions, answered from files in $FAKE; every call is recorded in $FAKE/sm-args.
SM = '''#!/bin/sh
echo "$*" >> "$FAKE/sm-args"
case "$*" in
  *--print-fingerprint*) echo abcdef0123456789 ;;
  *--would-run*|*--would-enable*)
    [ -e "$FAKE/sm-broken" ] && exit 127
    [ -e "$FAKE/sm-says" ] && { cat "$FAKE/sm-says"; exit 3; }
    echo run ;;
  *--home-held*) [ -e "$FAKE/home-held" ] && exit 0; exit 1 ;;
esac
'''
# A sha256sum that counts its files and hands them to the real one.
SHA256SUM = '''#!/bin/sh
for f in "$@"; do echo "$f" >> "$FAKE/hashed"; done
exec "$REAL_SHA256SUM" "$@"
'''
RELEASE = "Rockhopper release 10.2.1-5202 (ponytail-paparoa)\n"


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Fixture(unittest.TestCase):
    """An installed, verified set: LG's binary and two libraries as files, a bundle, the flag, the lists."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.comp = os.path.join(d, "comp")
        os.makedirs(os.path.join(self.comp, "bin"))
        os.makedirs(os.path.join(self.comp, "lib"))
        self.lg = os.path.join(d, "surface-manager")
        with open(self.lg, "w") as f:
            f.write("LG's binary")
        # what the firmware check covers besides LG's binary
        self.fw = os.path.join(d, "fw")
        os.makedirs(self.fw)
        self.release = os.path.join(self.fw, "starfish-release")
        self.libs = [os.path.join(self.fw, "libmali.so"), os.path.join(self.fw, "libwayland-webos-client.so")]
        with open(self.release, "w") as f:
            f.write(RELEASE)
        for i, p in enumerate(self.libs):
            with open(p, "wb") as f:
                f.write(b"library %d\n" % i * 100)
        self.bin = os.path.join(d, "bin")
        os.makedirs(self.bin)
        for name, body in (("mount", MOUNT), ("umount", UMOUNT), ("pgrep", PGREP)):
            path = os.path.join(self.bin, name)
            with open(path, "w") as f:
                f.write(body)
            os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        open(os.path.join(d, "mountinfo"), "w").close()
        with open(os.path.join(d, "boot_id"), "w") as f:
            f.write("BOOT-A\n")
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], FAKE=d, COMP=self.comp, COMP_LG=self.lg,
                        COMP_MOUNTINFO=os.path.join(d, "mountinfo"), COMP_LOCK=os.path.join(d, "recover.lock"),
                        COMP_RELEASE=self.release, COMP_FW_LIBS="\n".join(self.libs), COMP_BOOT_ID=os.path.join(d, "boot_id"))
        self.write_bin("lumatv-sm", SM)
        self.write_bin("lumatv-compositor", "the compositor\n")
        with open(os.path.join(self.comp, "lib", "ld-linux-aarch64.so.1"), "w") as f:
            f.write("loader\n")
        self.install()

    def tearDown(self):
        self.tmp.cleanup()

    def write_bin(self, name, text):
        p = os.path.join(self.comp, "bin", name)
        with open(p, "w") as f:
            f.write(text)
        os.chmod(p, 0o755)

    def state(self, name):
        return os.path.join(self.comp, name)

    def put(self, name, text=""):
        with open(self.state(name), "w") as f:
            f.write(text)

    def fake(self, name, text=""):
        with open(os.path.join(self.tmp.name, name), "w") as f:
            f.write(text)

    def install(self):
        """What compositor/tools/tv-cutover.sh install records, with the persistent flag on."""
        st = os.stat(self.lg)
        self.put("lg-binary", f"{st.st_size} {int(st.st_mtime)}\n")
        self.put("enabled")
        self.put("supervisor-api", "3\n")
        lines = []
        for sub in ("bin", "lib"):
            for name in sorted(os.listdir(self.state(sub))):
                with open(os.path.join(self.state(sub), name), "rb") as f:
                    data = f.read()
                lines.append(f"{len(data)} {sha(data) if sub == 'bin' else '-'} {sub}/{name}")
        self.put("bundle.manifest", "\n".join(lines) + "\n")
        rc, manifest = self.sh("comp_fw_manifest")
        self.put("manifest", manifest + "\n")
        rc, block = self.sh("comp_fw_block")
        self.put("verified.list", "# the builds the gate passed on\n" + block + "\n")
        for gone in ("fw-verified", "escaped", "running", "failures", "last-fallback"):
            if os.path.exists(self.state(gone)):
                os.remove(self.state(gone))

    def sh(self, script, env=None):
        prelude = ('set -u\n'
                   'log_line() { echo "log: $1" >> "$FAKE/calls"; }\n'
                   'restart_surface() { echo restart >> "$FAKE/calls"; [ ! -e "$FAKE/restart-fails" ]; }\n'
                   'recover_home() { echo recover >> "$FAKE/calls"; }\n'
                   'stock_ui_restore() { echo restore >> "$FAKE/calls"; }\n'
                   f'. {LIB}\n')
        r = subprocess.run(["sh", "-c", prelude + script], env=dict(self.env, **(env or {})), capture_output=True, text=True)
        return r.returncode, r.stdout.strip()

    def calls(self):
        p = os.path.join(self.tmp.name, "calls")
        return open(p).read().split("\n")[:-1] if os.path.exists(p) else []

    def mounted(self):
        return self.lg in open(os.path.join(self.tmp.name, "mountinfo")).read()

    def sm_calls(self):
        p = os.path.join(self.tmp.name, "sm-args")
        return open(p).read().split("\n")[:-1] if os.path.exists(p) else []


class Compositor(Fixture):

    def test_boot_mounts_when_everything_says_so(self):
        self.assertEqual(self.sh("comp_boot_mount"), (0, "mounted"))
        self.assertTrue(self.mounted())
        self.assertEqual(self.calls(), [f"mount {self.comp}/bin/lumatv-sm {self.lg}"])

    def test_boot_does_nothing_twice(self):
        self.sh("comp_boot_mount")
        self.assertEqual(self.sh("comp_boot_mount"), (0, "already"))
        self.assertEqual(len([c for c in self.calls() if c.startswith("mount")]), 1)

    def test_boot_refuses_without_the_flag_or_the_bundle(self):
        os.remove(os.path.join(self.comp, "enabled"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "off"))
        self.install()
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "off"))
        self.assertFalse(self.mounted())

    def test_boot_keeps_lgs_compositor_when_its_binary_changed(self):
        with open(self.lg, "w") as f:
            f.write("LG's binary, updated by a firmware update")
        self.assertEqual(self.sh("comp_boot_mount"), (0, "firmware"))
        self.assertFalse(self.mounted())
        # a recorded size and time that cannot be read count as changed
        os.remove(os.path.join(self.comp, "lg-binary"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "firmware"))

    def test_boot_keeps_lgs_compositor_when_stock_ui_is_held(self):
        self.put("stock-ui-held")
        self.assertEqual(self.sh("comp_boot_mount"), (0, "held"))
        self.assertFalse(self.mounted())

    def test_boot_never_fails_the_boot(self):
        open(os.path.join(self.tmp.name, "mount-fails"), "w").close()
        self.assertEqual(self.sh("comp_boot_mount"), (0, "error"))

    def test_live_needs_the_flag_the_mount_and_the_process(self):
        up = os.path.join(self.tmp.name, "compositor.up")
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")
        self.sh("comp_boot_mount")
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # mounted, no process yet
        open(up, "w").close()
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "live")
        os.remove(os.path.join(self.comp, "enabled"))
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # the flag is a builtin test: no pgrep
        self.install()
        os.remove(os.path.join(self.tmp.name, "mountinfo"))
        open(os.path.join(self.tmp.name, "mountinfo"), "w").close()
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # process but no mount

    def test_a_path_that_only_contains_the_name_is_not_the_mount(self):
        with open(os.path.join(self.tmp.name, "mountinfo"), "w") as f:
            f.write(f"1 2 0:3 / {self.lg}.old rw - overlay x rw\n1 2 0:3 / /mnt{self.lg} rw - overlay x rw\n")
        self.assertEqual(self.sh("comp_mounted && echo yes || echo no")[1], "no")

    def test_on_mounts_and_clears_the_failure_state(self):
        open(os.path.join(self.comp, "escaped"), "w").close()
        open(os.path.join(self.comp, "last-fallback"), "w").write("hold-home")
        open(os.path.join(self.comp, "failures"), "w").write("2\n")
        rc, out = self.sh("comp_switch on")
        self.assertEqual((rc, out), (0, ""))
        self.assertTrue(self.mounted())
        self.assertEqual(self.calls(), [f"mount {self.comp}/bin/lumatv-sm {self.lg}"])
        self.assertFalse(os.path.exists(os.path.join(self.comp, "escaped")))
        self.assertFalse(os.path.exists(os.path.join(self.comp, "last-fallback")))
        self.assertEqual(open(os.path.join(self.comp, "failures")).read().strip(), "0")

    def test_on_refuses_what_would_break_the_screen(self):
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm"))
        self.assertEqual(self.sh("comp_switch on")[0], 2)
        with open(os.path.join(self.comp, "bin", "lumatv-sm"), "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(os.path.join(self.comp, "bin", "lumatv-sm"), 0o755)
        with open(os.path.join(self.tmp.name, "recover.lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.sh("comp_switch on")[0], 3)
        with open(self.lg, "w") as f:
            f.write("a different binary")
        self.assertEqual(self.sh("comp_switch on")[0], 4)
        self.assertFalse(self.mounted())
        self.assertNotIn("restart", self.calls())

    def test_on_reports_mount_failure(self):
        self.fake("mount-fails")
        rc, out = self.sh("comp_switch on")
        self.assertEqual(rc, 1)
        self.assertEqual(out, "mount failed")
        self.assertNotIn("recover", self.calls())

    def test_off_removes_the_flag_and_mount_and_restores_stock_ui(self):
        self.sh("comp_switch on")
        rc, out = self.sh("comp_switch off")
        self.assertEqual(rc, 0)
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.comp, "enabled")))
        self.assertEqual(self.calls()[-2:], [f"umount {self.lg}", "restore"])
        self.assertTrue(os.path.exists(self.state("stock-ui-held")))

    def test_off_when_it_was_never_on_restarts_nothing(self):
        rc, out = self.sh("comp_switch off")
        self.assertEqual(rc, 0)
        self.assertNotIn("restart", self.calls())

    def test_release_unmounts_and_forgets_the_flag_without_restarting(self):
        self.sh("comp_boot_mount")
        self.sh("comp_release")
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.comp, "enabled")))
        self.assertNotIn("restart", self.calls())

    def test_a_recovery_in_progress_is_seen(self):
        self.assertEqual(self.sh("comp_recovering && echo yes || echo no")[1], "no")
        with open(os.path.join(self.tmp.name, "recover.lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.sh("comp_recovering && echo yes || echo no")[1], "yes")
        self.assertEqual(self.sh("comp_recovering && echo yes || echo no")[1], "no")

    def test_status_is_one_valid_json_object(self):
        logs = os.path.join(self.tmp.name, "central")
        os.mkdir(logs)
        self.env["LUMATV_LOG_DIR"] = logs
        open(os.path.join(logs, "current.log"), "w").write('<134>lumatv-sm: 12:00:00 said "hello"\\ there\n')
        open(os.path.join(self.comp, "last-fallback"), "w").write('a "quoted" reason\n')
        open(os.path.join(self.comp, "failures"), "w").write("1\n")
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["bundle"], j["enabled"], j["mounted"], j["live"], j["lgBinary"], j["failures"]), (True, True, False, False, "same", 1))
        self.assertEqual(j["smLog"], "12:00:00 said hello there")
        self.sh("comp_boot_mount")
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["mounted"], j["lgBinary"]), (True, "mounted"))
        with open(os.path.join(self.tmp.name, "mountinfo"), "w"):
            pass
        with open(self.lg, "w") as f:
            f.write("changed")
        self.assertEqual(json.loads(self.sh("comp_json")[1])["lgBinary"], "changed")

    def test_status_with_nothing_installed(self):
        for n in os.listdir(self.comp):
            p = os.path.join(self.comp, n)
            os.remove(p) if os.path.isfile(p) else None
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm")) if os.path.exists(os.path.join(self.comp, "bin", "lumatv-sm")) else None
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["bundle"], j["enabled"], j["live"], j["lgBinary"], j["failures"]), (False, False, False, "unknown", 0))


class Firmware(Fixture):
    """comp_fw_check: a persistent enable only on a build whose files hash as the gate saw them."""

    def check(self, arg=""):
        return self.sh("comp_fw_check %s" % arg)

    def hashed(self):
        p = os.path.join(self.tmp.name, "hashed")
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return f.read().split()

    def counting_sha256sum(self):
        import shutil
        path = os.path.join(self.bin, "sha256sum")
        with open(path, "w") as f:
            f.write(SHA256SUM)
        os.chmod(path, 0o755)
        self.env["REAL_SHA256SUM"] = shutil.which("sha256sum")

    def test_the_installed_build_is_verified(self):
        self.assertEqual(self.check("force"), (0, "verified 10.2.1-5202"))

    def test_a_changed_library_is_not_the_verified_build(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        rc, out = self.check("force")
        self.assertEqual(rc, 5)
        self.assertTrue(out.startswith("unverified: "), out)
        self.assertIn(self.libs[0], out)
        self.assertIn("is not the verified file", out)
        self.assertIn("closest entry 10.2.1-5202", out)

    def test_a_changed_compositor_binary_is_not_the_verified_build(self):
        with open(self.lg, "w") as f:
            f.write("LG's binary, updated")
        rc, out = self.check("force")
        self.assertEqual(rc, 5)
        self.assertIn(self.lg, out)

    def test_a_new_release_is_not_the_verified_build(self):
        with open(self.release, "w") as f:
            f.write("Rockhopper release 10.2.2-5300 (ponytail-paparoa)\n")
        rc, out = self.check("force")
        self.assertEqual(rc, 5)
        self.assertIn("the release is Rockhopper release 10.2.2-5300 (ponytail-paparoa), not Rockhopper release 10.2.1-5202", out)

    def test_a_file_that_cannot_be_read_is_unknown(self):
        os.remove(self.libs[1])
        rc, out = self.check("force")
        self.assertEqual(rc, 5)
        self.assertIn("%s cannot be read" % self.libs[1], out)

    def test_no_hash_tool_is_no_verification(self):
        rc, out = self.sh("comp_sha256() { :; }\ncomp_fw_check force")
        self.assertEqual(rc, 5)
        self.assertIn("cannot be read", out)

    def test_nothing_is_verified_without_a_list_or_with_an_empty_one(self):
        os.remove(self.state("verified.list"))
        self.assertEqual(self.check("force"), (5, "unverified: no verified.list is installed"))
        self.put("verified.list", "")
        self.assertEqual(self.check("force"), (5, "unverified: no verified.list is installed"))
        self.put("verified.list", "# the builds the gate passed on: none yet\n")
        self.assertEqual(self.check("force"), (5, "unverified: verified.list has no entry"))

    def test_any_entry_of_the_list_will_do(self):
        with open(self.state("verified.list")) as f:
            current = f.read()
        older = "build 10.1.0-1 2026-01-01\nrelease Rockhopper release 10.1.0-1 (x)\nfingerprint 1111111111111111\nfile %s 10 /usr/lib/x\nend\n" % ("0" * 64)
        self.put("verified.list", older + current)
        self.assertEqual(self.check("force"), (0, "verified 10.2.1-5202"))

    def test_an_entry_that_covers_other_files_is_another_build(self):
        with open(self.state("verified.list")) as f:
            text = f.read()
        self.put("verified.list", text.replace("\nend\n", "\nfile %s 5 /usr/lib/extra\nend\n" % ("0" * 64)))
        rc, out = self.check("force")
        self.assertEqual(rc, 5)
        self.assertIn("the entry covers other files", out)

    def test_a_verified_result_is_cached_until_a_file_changes(self):
        self.counting_sha256sum()
        self.assertEqual(self.check("force")[0], 0)
        self.assertEqual(len(self.hashed()), 4)                 # the release file, LG's binary and the two libraries
        os.remove(os.path.join(self.tmp.name, "hashed"))
        self.assertEqual(self.check(), (0, "verified 10.2.1-5202"))
        self.assertEqual(self.hashed(), [], "the cached result was hashed again")
        self.assertEqual(self.check("force")[0], 0)
        self.assertEqual(len(self.hashed()), 4)
        os.remove(os.path.join(self.tmp.name, "hashed"))
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        rc, out = self.check()
        self.assertEqual(rc, 5)
        self.assertEqual(len(self.hashed()), 4, "a changed library must be hashed, not trusted")
        self.assertFalse(os.path.exists(self.state("fw-verified")), "an unverified result must not stay cached")

    def test_the_mounted_set_is_checked_against_the_binary_install_recorded(self):
        self.assertEqual(self.sh("comp_boot_mount"), (0, "mounted"))
        with open(self.lg, "w") as f:
            f.write("the supervisor, as the mount shows it")      # LG's own file is hidden under it
        self.assertEqual(self.check("force"), (0, "verified 10.2.1-5202"))
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.assertEqual(self.check("force")[0], 5)

    def test_the_record_is_an_entry_the_check_accepts(self):
        rc, block = self.sh("comp_fw_block")
        lines = block.split("\n")
        self.assertTrue(lines[0].startswith("build 10.2.1-5202 "), lines[0])
        self.assertEqual(lines[1], "release Rockhopper release 10.2.1-5202 (ponytail-paparoa)")
        self.assertEqual(lines[2], "fingerprint abcdef0123456789")
        self.assertEqual(lines[-1], "end")
        files = [l.split() for l in lines if l.startswith("file ")]
        self.assertEqual([f[3] for f in files], [self.release, self.lg] + self.libs)
        for f in files:
            self.assertRegex(f[1], r"^[0-9a-f]{64}$")
            self.assertEqual(int(f[2]), os.path.getsize(f[3]))

    def test_the_state_object_reads_the_cache_and_never_hashes(self):
        self.counting_sha256sum()
        rc, out = self.sh("comp_json")
        self.assertEqual(json.loads(out)["firmware"], "unknown")
        self.assertEqual(self.hashed(), [])
        self.check("force")
        os.remove(os.path.join(self.tmp.name, "hashed"))
        self.assertEqual(json.loads(self.sh("comp_json")[1])["firmware"], "verified")
        self.assertEqual(self.hashed(), [])
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.assertEqual(json.loads(self.sh("comp_json")[1])["firmware"], "unknown")
        os.remove(self.state("verified.list"))
        self.assertEqual(json.loads(self.sh("comp_json")[1])["firmware"], "unverified")
        self.assertEqual(self.hashed(), [])

    def test_the_state_object_remembers_a_failed_check_until_a_file_changes(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.assertEqual(self.check("force")[0], 5)
        self.assertEqual(json.loads(self.sh("comp_json")[1])["firmware"], "unverified")
        with open(self.libs[0], "ab") as f:
            f.write(b"and more")
        self.assertEqual(json.loads(self.sh("comp_json")[1])["firmware"], "unknown")        # not what the last check saw


class BootDecision(Fixture):
    """comp_boot_mount: every reason the early hook leaves LG's compositor alone, and the one it mounts."""

    def boot(self, arg=""):
        return self.sh("comp_boot_mount %s" % arg)

    def logged(self):
        return [c for c in self.calls() if c.startswith("log: ")]

    def test_the_flag_kinds(self):
        self.assertEqual(self.sh("comp_flag")[1], "persistent")      # an empty file, what `touch enabled` made
        self.put("enabled", "persistent\n")
        self.assertEqual(self.sh("comp_flag")[1], "persistent")
        self.put("enabled", "session BOOT-A\n")
        self.assertEqual(self.sh("comp_flag")[1], "session")
        self.put("enabled", "session BOOT-OLD\n")
        self.assertEqual(self.sh("comp_flag")[1], "stale")
        self.put("enabled", "session\n")
        self.assertEqual(self.sh("comp_flag")[1], "stale")
        os.remove(self.state("enabled"))
        self.assertEqual(self.sh("comp_flag")[1], "off")

    def test_a_session_of_this_boot_mounts_on_a_build_the_gate_has_not_seen(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.put("enabled", "session BOOT-A\n")
        self.assertEqual(self.boot(), (0, "mounted"))

    def test_a_persistent_enable_on_such_a_build_stays_on_lgs_compositor_and_says_why(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.assertEqual(self.boot(), (0, "firmware"))
        self.assertFalse(self.mounted())
        self.assertEqual(len(self.logged()), 1)
        self.assertIn("compositor-boot: firmware: unverified: ", self.logged()[0])
        self.assertIn("is not the verified file", self.logged()[0])

    def test_a_session_of_an_earlier_boot_is_forgotten(self):
        self.put("enabled", "session BOOT-OLD\n")
        self.assertEqual(self.boot("check"), (0, "off"))
        self.assertTrue(os.path.exists(self.state("enabled")), "a check changes nothing")
        self.assertEqual(self.boot(), (0, "off"))
        self.assertFalse(os.path.exists(self.state("enabled")))
        self.assertFalse(self.mounted())

    def test_the_supervisor_has_the_last_word_before_the_mount(self):
        for word in ("escaped", "failures", "session", "firmware", "bundle", "off"):
            if os.path.exists(os.path.join(self.tmp.name, "calls")):
                os.remove(os.path.join(self.tmp.name, "calls"))
            self.fake("sm-says", "no %s: because of %s\n" % (word, word))
            self.assertEqual(self.boot(), (0, word), word)
            self.assertFalse(self.mounted(), word)
            self.assertEqual(self.logged(), ["log: compositor-boot: %s: because of %s" % (word, word)])

    def test_a_supervisor_that_cannot_answer_is_never_mounted(self):
        self.fake("sm-broken")
        self.assertEqual(self.boot(), (0, "broken"))
        self.assertIn("does not run here (exit 127)", self.logged()[0])
        self.assertFalse(self.mounted())

    def test_a_bundle_from_before_the_question_existed_is_not_asked_it(self):
        # an old binary takes --would-run for a start and would exec LG's compositor from the hook
        os.remove(self.state("supervisor-api"))
        self.assertEqual(self.boot(), (0, "broken"))
        self.assertIn("no supervisor-api", self.logged()[0])
        self.assertEqual([c for c in self.sm_calls() if "would-run" in c or "home-held" in c], [])

    def test_supervisor_handles_home_hold_without_hook_query(self):
        self.fake("home-held")
        self.assertEqual(self.boot("check"), (0, "would-mount"))
        self.assertEqual(self.boot(), (0, "mounted"))
        self.assertFalse(os.path.exists(self.state("escaped")))
        self.assertEqual([c for c in self.sm_calls() if "home-held" in c], [])

    def test_a_damaged_bundle_is_not_mounted(self):
        self.write_bin("lumatv-compositor", "the compositor, cut short\n"[:10])
        self.assertEqual(self.boot(), (0, "bundle"))
        self.assertIn("bin/lumatv-compositor is not the installed size", self.logged()[0])
        self.assertFalse(self.mounted())

    def test_a_bundle_without_the_record_of_its_install_is_not_mounted(self):
        os.remove(self.state("bundle.manifest"))
        self.assertEqual(self.boot(), (0, "bundle"))
        self.assertIn("no bundle.manifest", self.logged()[0])

    def test_stock_ui_hold_outranks_firmware_refusal(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        self.put("stock-ui-held")
        self.assertEqual(self.boot(), (0, "held"))
        self.assertFalse(self.mounted())

    def test_a_check_mounts_nothing_and_says_what_a_boot_would_do(self):
        self.assertEqual(self.boot("check"), (0, "would-mount"))
        self.assertFalse(self.mounted())
        self.assertEqual(self.calls(), [])
        self.assertTrue(os.path.exists(self.state("enabled")))

    def test_the_check_object_is_one_json_object_that_hashes_for_real(self):
        rc, out = self.sh("comp_check_json")
        j = json.loads(out)
        self.assertEqual((j["decision"], j["wouldMount"], j["flag"], j["build"], j["why"]), ("ok", True, "persistent", "10.2.1-5202", ""))
        with open(self.libs[0], "ab") as f:
            f.write(b"an update \"quoted\"")
        j = json.loads(self.sh("comp_check_json")[1])
        self.assertEqual((j["decision"], j["wouldMount"]), ("firmware", False))
        self.assertIn("is not the verified file", j["why"])

    def test_status_names_the_flag_and_an_unsettled_run(self):
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["flag"], j["unsettled"]), ("persistent", False))
        self.put("running", "BOOT-A 1\n")
        self.put("enabled", "session BOOT-A\n")
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["flag"], j["unsettled"], j["enabled"]), ("session", True, True))


class Switch(Fixture):
    """comp_switch on: the persistent enable of the user's command, which the firmware and the bundle must allow."""

    def test_on_refuses_a_build_the_gate_has_not_passed_on(self):
        with open(self.libs[0], "ab") as f:
            f.write(b"an update")
        rc, out = self.sh("comp_switch on")
        self.assertEqual(rc, 5)
        self.assertIn("unverified: ", out)
        self.assertIn(self.libs[0], out)
        self.assertFalse(self.mounted())
        self.assertNotIn("restart", self.calls())

    def test_on_checks_the_files_themselves_not_the_cache(self):
        self.assertEqual(self.sh("comp_fw_check force")[0], 0)            # the cache says verified
        st = os.stat(self.libs[0])
        with open(self.libs[0], "r+b") as f:                              # the same size and time, another content
            f.write(b"X")
        os.utime(self.libs[0], ns=(st.st_atime_ns, st.st_mtime_ns))
        self.assertEqual(self.sh("comp_fw_check")[0], 0, "the cache is stat-keyed by design")
        rc, out = self.sh("comp_switch on")
        self.assertEqual(rc, 5, out)

    def test_on_refuses_a_bundle_that_is_not_the_installed_one(self):
        self.write_bin("lumatv-compositor", "the compositoX\n")           # the size of the first one, not its content
        self.assertEqual(os.path.getsize(self.state("bin/lumatv-compositor")), len("the compositor\n"))
        rc, out = self.sh("comp_switch on")
        self.assertEqual(rc, 6)
        self.assertIn("bin/lumatv-compositor differs from the installed copy", out)
        self.assertFalse(self.mounted())

    def test_on_makes_the_persistent_flag(self):
        os.remove(self.state("enabled"))
        self.assertEqual(self.sh("comp_switch on"), (0, ""))
        with open(self.state("enabled")) as f:
            self.assertEqual(f.read().strip(), "persistent")
        self.assertEqual(self.sh("comp_flag")[1], "persistent")

    def test_on_clears_a_run_that_never_settled(self):
        self.put("running", "BOOT-A 1\n")
        self.assertEqual(self.sh("comp_switch on")[0], 0)
        self.assertFalse(os.path.exists(self.state("running")))




if __name__ == "__main__":
    unittest.main()
