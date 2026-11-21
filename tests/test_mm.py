"""tools/mm.sh against stand-in mount, systemctl and kill commands: the shim is bound only over LG's own memchute, only when it would
serve and the bus would give it the name, a failure leaves LG's binary serving, `off` has the shim hand back in place, a give-up holds
for its boot, and the boot step binds again only while switched on."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "mm.sh"

# mount --bind SRC DST shows SRC itself at DST (same inode); umount [-l] DST brings back what was under it.
MOUNT = """#!/bin/sh
echo "mount $*" >> "$FAKE/calls"
[ "${FAIL_MOUNT:-0}" != 1 ] || exit 1
n=$(wc -l < "$FAKE/mounts")
mv "$3" "$FAKE/under.$n"
ln "$2" "$3"
echo "$2 $3 none bind 0 0" >> "$FAKE/mounts"
"""

UMOUNT = """#!/bin/sh
echo "umount $*" >> "$FAKE/calls"
for last; do :; done
n=$(( $(wc -l < "$FAKE/mounts") - 1 ))
[ "$n" -ge 0 ] || exit 1
rm -f "$last"
mv "$FAKE/under.$n" "$last"
head -n "$n" "$FAKE/mounts" > "$FAKE/mounts.new" && mv "$FAKE/mounts.new" "$FAKE/mounts"
"""

# The unit's process runs whatever the binary is when it starts.
SYSTEMCTL = """#!/bin/sh
case "$1" in
  show) echo "MainPID=4242" ;;
  restart) echo "systemctl $*" >> "$FAKE/calls"; ln -f "$BIN" "$FAKE/proc/4242/exe" ;;
esac
"""

# SIGUSR1 to the shim: it unmounts itself and execs LG's (unless the test says it hangs).
KILL = """#!/bin/sh
echo "kill $*" >> "$FAKE/calls"
[ "${SHIM_HANGS:-0}" = 1 ] && exit 0
"$FAKE/bin/umount" "$BIN"
ln -f "$BIN" "$FAKE/proc/4242/exe"
"""

SHIM = """#!/bin/sh
case "$3" in
  --would-run) [ -f "$2/enabled" ] && { echo run; exit 0; }; echo "no: LumaTV's memory manager is switched off"; exit 3 ;;
  --probe) exit "${PROBE:-0}" ;;
esac
"""


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class MmTool(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="mm-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = self.tmp / "state"
        self.mm = self.state / "mm"
        self.shim = self.mm / "bin" / "lumatv-mm"
        self.shim.parent.mkdir(parents=True)
        self.shim.write_text(SHIM)
        self.shim.chmod(0o755)
        self.lg = self.tmp / "memchute"
        self.lg.write_text("#!/bin/sh\necho LG\n")
        self.lg.chmod(0o755)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        for name, text in (("mount", MOUNT), ("umount", UMOUNT), ("systemctl", SYSTEMCTL), ("kill", KILL)):
            path = self.bin / name
            path.write_text(text)
            path.chmod(0o755)
        (self.tmp / "proc/4242").mkdir(parents=True)
        os.link(self.lg, self.tmp / "proc/4242/exe")
        (self.tmp / "boot_id").write_text("boot-a\n")
        (self.tmp / "mounts").write_text("")
        (self.tmp / "calls").write_text("")
        self.env = dict(
            os.environ, FAKE=str(self.tmp), STATE=str(self.state), APPDIR=str(ROOT), BIN=str(self.lg),
            MM_BIN=str(self.lg), MM_BIN_SHA=sha(self.lg), BOOT_ID=str(self.tmp / "boot_id"),
            MOUNT=str(self.bin / "mount"), UMOUNT=str(self.bin / "umount"), SYSTEMCTL=str(self.bin / "systemctl"),
            KILL=str(self.bin / "kill"), PROC_MOUNTS=str(self.tmp / "mounts"), PROC_ROOT=str(self.tmp / "proc"),
            MM_LOCK_WAIT="0", MM_OFF_WAIT="0", LOGFILE=str(self.tmp / "log"))

    def run_tool(self, *args, **env):
        result = subprocess.run(["sh", str(TOOL), *args], env=dict(self.env, **env), capture_output=True, text=True, timeout=30)
        try:
            return result.returncode, json.loads(result.stdout)
        except ValueError:
            self.fail("not JSON: %r %r" % (result.stdout, result.stderr))

    def calls(self, prefix=""):
        return [c for c in (self.tmp / "calls").read_text().splitlines() if c.startswith(prefix)]

    def mounts(self):
        return (self.tmp / "mounts").read_text().splitlines()

    def status(self):
        return self.run_tool("status")[1]

    def reboot(self, boot="boot-b"):
        """LG's binary is back and serving, nothing is mounted; the state directory stays."""
        while self.mounts():
            subprocess.run([str(self.bin / "umount"), str(self.lg)], env=self.env, check=True)
        os.remove(self.tmp / "proc/4242/exe")
        os.link(self.lg, self.tmp / "proc/4242/exe")
        (self.tmp / "boot_id").write_text(boot + "\n")
        (self.tmp / "calls").write_text("")

    def test_on_binds_the_shim_probes_it_and_restarts_lgs_unit_once(self):
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "mm": "bound"}))
        self.assertTrue(os.path.samefile(self.lg, self.shim))
        self.assertTrue((self.mm / "enabled").exists())
        self.assertEqual(self.calls("systemctl "), ["systemctl restart memchute.service"])
        self.assertEqual(self.run_tool("on"), (0, {"ok": True, "mm": "already"}))
        self.assertEqual(len(self.calls("mount ")), 1)
        status = self.status()
        self.assertEqual((status["enabled"], status["bin"], status["serving"]), (True, "ours", "lumatv"))

    def test_a_shim_staged_while_one_is_bound_is_bound_in_its_place(self):
        self.run_tool("on")
        next_shim = self.mm / "bin" / "lumatv-mm.next"
        next_shim.write_text(SHIM + "# new\n")
        next_shim.chmod(0o755)
        self.assertEqual(self.run_tool("on", "--auto"), (0, {"ok": True, "mm": "bound"}))
        self.assertFalse(next_shim.exists())
        self.assertTrue(self.lg.read_text().endswith("# new\n"))
        self.assertEqual((len(self.mounts()), self.status()["serving"]), (1, "lumatv"))

    def test_off_has_the_shim_hand_back_in_place_and_leaves_lgs_binary(self):
        self.run_tool("on")
        code, out = self.run_tool("off")
        self.assertEqual((code, out), (0, {"ok": True, "mm": "off", "was": "ours", "lg": "handed"}))
        self.assertEqual(self.calls("kill "), ["kill -USR1 4242"])
        self.assertEqual(self.calls("systemctl "), ["systemctl restart memchute.service"], "no second restart")
        self.assertEqual(self.mounts(), [])
        self.assertFalse((self.mm / "enabled").exists())
        status = self.status()
        self.assertEqual((status["bin"], status["serving"]), ("stock", "lg"))

    def test_off_with_a_shim_that_does_not_hand_back_unmounts_and_restarts(self):
        self.run_tool("on")
        code, out = self.run_tool("off", SHIM_HANGS="1")
        self.assertEqual((code, out["lg"]), (0, "restarted"))
        self.assertEqual(self.calls("umount "), ["umount -l %s" % self.lg])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.status()["serving"], "lg")

    def test_off_with_nothing_to_undo_leaves_no_trace(self):
        self.assertEqual(self.run_tool("off"), (0, {"ok": True, "mm": "off", "was": "stock"}))
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.tmp / "log").exists())

    def test_another_binary_a_missing_or_broken_shim_or_a_foreign_mount_changes_nothing(self):
        cases = (
            ("binary", lambda: self.lg.write_text("another firmware's memchute\n"), "not the memory manager"),
            ("missing", lambda: self.shim.unlink(), "not installed"),
            ("broken", lambda: self.shim.write_text("#!/bin/sh\necho 'error while loading shared libraries'\nexit 127\n"), "cannot run here"),
            ("foreign", lambda: (self.tmp / "mounts").write_text("/elsewhere %s none bind 0 0\n" % self.lg), "another mount"),
        )
        for name, change, error in cases:
            with self.subTest(name):
                self.setUp()
                change()
                code, out = self.run_tool("on")
                self.assertEqual(code, 1)
                self.assertIn(error, out["error"])
                self.assertEqual(self.calls("mount ") + self.calls("systemctl "), [])
                self.assertFalse((self.mm / "enabled").exists())

    def test_a_probe_the_bus_refuses_takes_the_mount_back_before_lgs_unit_is_touched(self):
        code, out = self.run_tool("on", PROBE="2")
        self.assertEqual(code, 1)
        self.assertIn("would not give the shim the name", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.calls("systemctl "), [])
        self.assertFalse((self.mm / "enabled").exists())

    def test_a_give_up_holds_for_its_boot_and_switching_on_by_hand_clears_it(self):
        self.run_tool("on")
        self.reboot("boot-b")
        (self.mm / "gave-up").write_text("boot-b\n")
        (self.mm / "failures").write_text("boot-b 10 x\nboot-b 20 y\nboot-b 30 z\n")
        self.assertEqual(self.run_tool("on", "--auto")[1]["mm"], "gave up")
        self.assertEqual(self.run_tool("early"), (0, {"ok": True, "applied": False, "why": "gave up this boot"}))
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.run_tool("on"), (0, {"ok": True, "mm": "bound"}))
        self.assertFalse((self.mm / "gave-up").exists() or (self.mm / "failures").exists())

    def test_the_boot_step_and_the_helpers_retry_bind_again_only_while_switched_on(self):
        self.run_tool("on")
        self.reboot()
        self.assertEqual(self.run_tool("early"), (0, {"ok": True, "applied": True, "mm": "bound"}))
        self.assertEqual(self.status()["serving"], "lumatv")
        # Bound, but LG's serves after a fallback: the retry restarts the unit.
        os.remove(self.tmp / "proc/4242/exe")
        shutil.copy(self.lg, self.tmp / "proc/4242/exe")
        self.assertEqual(self.run_tool("on", "--auto"), (0, {"ok": True, "mm": "restarted"}))
        self.run_tool("off")
        self.reboot()
        self.assertEqual(self.run_tool("early"), (0, {"ok": True, "applied": False, "why": "not switched on"}))
        self.assertEqual(self.run_tool("on", "--auto"), (0, {"ok": True, "mm": "off"}))
        self.assertEqual(self.calls("mount "), [])
        # A failure at boot is a log line, never an exit status.
        self.run_tool("on")
        self.reboot()
        self.lg.write_text("an updated memchute\n")
        code, out = self.run_tool("early")
        self.assertEqual((code, out["applied"]), (0, False))
        self.assertIn("not the memory manager", out["why"])


if __name__ == "__main__":
    unittest.main()
