"""tools/ssgw.sh against stand-in mount and kill commands: xinetd's entry for port 3001 is bound only over LG's own entry with LG's nsl
and an installed ssgw-accept, the copy changes the program and nothing else, a mount that is not ours is never touched, a failure rolls
back, xinetd is told only when the entry changed, and the boot step binds it again only while it is switched on."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "ssgw.sh"

# /etc/xinetd.d/ssg_https of webOS 10.2.1, byte for byte.
STOCK = """service ssg_https
{
    disable         = no
    protocol        = tcp
    socket_type     = stream
    wait            = no
    user            = root
    server          = /usr/bin/nsl
    server_args     = 3001 com.webos.service.secondscreen.gateway
    port            = 3001
}

"""

# mount --bind SRC DST shows SRC itself at DST (same inode); umount DST brings back what was under it.
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
[ "${FAIL_UMOUNT:-0}" != 1 ] || exit 1
n=$(( $(wc -l < "$FAKE/mounts") - 1 ))
rm -f "$1"
mv "$FAKE/under.$n" "$1"
head -n "$n" "$FAKE/mounts" > "$FAKE/mounts.new" && mv "$FAKE/mounts.new" "$FAKE/mounts"
"""

KILL = """#!/bin/sh
echo "kill $*" >> "$FAKE/calls"
"""


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class SsgwTool(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ssgw-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = self.tmp / "state"
        self.gw = self.state / "ssgw"
        self.accept = self.gw / "bin" / "ssgw-accept"
        self.accept.parent.mkdir(parents=True)
        self.accept.write_text("#!/bin/sh\n")
        self.accept.chmod(0o755)
        self.entry = self.tmp / "ssg_https"
        self.entry.write_text(STOCK)
        self.nsl = self.tmp / "nsl"
        self.nsl.write_text("LG's nsl\n")
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        for name, text in (("mount", MOUNT), ("umount", UMOUNT), ("kill", KILL)):
            path = self.bin / name
            path.write_text(text)
            path.chmod(0o755)
        (self.tmp / "proc/4242").mkdir(parents=True)
        os.symlink("/usr/sbin/xinetd", self.tmp / "proc/4242/exe")
        (self.tmp / "xinetd.pid").write_text("4242\n")
        (self.tmp / "mounts").write_text("")
        (self.tmp / "calls").write_text("")
        self.env = dict(
            os.environ, FAKE=str(self.tmp), STATE=str(self.state), APPDIR=str(ROOT),
            SSGW_ENTRY=str(self.entry), SSGW_NSL=str(self.nsl), SSGW_NSL_SHA=sha(self.nsl),
            SSGW_XINETD_PIDFILE=str(self.tmp / "xinetd.pid"), SSGW_CONFIG=str(self.tmp / "config.json"),
            MOUNT=str(self.bin / "mount"), UMOUNT=str(self.bin / "umount"), KILL=str(self.bin / "kill"),
            PROC_MOUNTS=str(self.tmp / "mounts"), PROC_ROOT=str(self.tmp / "proc"), SSGW_LOCK_WAIT="0", LOGFILE=str(self.tmp / "log"))

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

    def reboot(self):
        """LG's entry is back on a new inode and nothing is mounted; the state directory stays."""
        self.entry.unlink()
        self.entry.write_text(STOCK)
        (self.tmp / "mounts").write_text("")

    def test_the_pinned_entry_is_lgs_as_it_is_on_the_set(self):
        # nothing overrides the tool's pin here: `stock` means STOCK has the pinned hash
        self.assertEqual(self.run_tool("status")[1]["entry"], "stock")

    def test_on_binds_a_copy_that_only_names_ssgw_accept_and_tells_xinetd_once(self):
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "ssgw": "bound", "xinetd": "told"}))
        self.assertEqual(len(self.mounts()), 1)
        self.assertTrue(os.path.samefile(self.entry, self.gw / "ssg_https"))
        self.assertEqual(self.entry.read_text(), STOCK.replace("= /usr/bin/nsl", "= %s" % self.accept))
        self.assertTrue((self.gw / "enabled").exists())
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "ssgw": "already"}))
        self.assertEqual(self.calls("kill "), ["kill -HUP 4242"])
        self.assertEqual(len(self.calls("mount ")), 1)
        status = self.run_tool("status")[1]
        self.assertEqual((status["enabled"], status["entry"], status["nsl"], status["accept"]), (True, "ours", "stock", True))

    def test_off_takes_lgs_entry_back_tells_xinetd_and_keeps_the_programs(self):
        self.run_tool("on")
        code, out = self.run_tool("off")
        self.assertEqual((code, out), (0, {"ok": True, "ssgw": "off", "was": "ours", "xinetd": "told"}))
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.entry.read_text(), STOCK)
        self.assertEqual(self.calls("kill "), ["kill -HUP 4242"] * 2)
        self.assertEqual(sorted(p.name for p in self.gw.iterdir()), ["bin"])
        self.assertEqual(self.run_tool("status")[1]["entry"], "stock")

    def test_off_with_nothing_to_undo_leaves_no_trace(self):
        code, out = self.run_tool("off")
        self.assertEqual((code, out), (0, {"ok": True, "ssgw": "off", "was": "stock"}))
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.tmp / "log").exists())

    def test_another_firmwares_entry_or_nsl_or_a_missing_accept_program_changes_nothing(self):
        cases = (
            ("entry", lambda: self.entry.write_text(STOCK + "# another firmware\n"), "not the entry"),
            ("nsl", lambda: self.nsl.write_text("another nsl\n"), "not the program"),
            ("accept", lambda: self.accept.unlink(), "not installed"),
        )
        for name, change, error in cases:
            with self.subTest(name):
                self.setUp()
                change()
                code, out = self.run_tool("on")
                self.assertEqual(code, 1)
                self.assertIn(error, out["error"])
                self.assertEqual(self.mounts(), [])
                self.assertFalse((self.gw / "enabled").exists())
                self.assertEqual(self.calls("kill "), [])

    def test_a_mount_that_is_not_ours_is_never_stacked_on_or_removed(self):
        (self.tmp / "mounts").write_text("/elsewhere %s none bind 0 0\n" % self.entry)
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("another mount", out["error"])
        code, out = self.run_tool("off")
        self.assertEqual(code, 1)
        self.assertIn("left alone", out["error"])
        self.assertEqual(self.calls("mount ") + self.calls("umount ") + self.calls("kill "), [])
        self.assertEqual(len(self.mounts()), 1)

    def test_a_failed_bind_rolls_back_and_xinetd_is_not_told(self):
        code, out = self.run_tool("on", FAIL_MOUNT="1")
        self.assertEqual(code, 1)
        self.assertIn("could not be bound", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.entry.read_text(), STOCK)
        self.assertEqual(sorted(p.name for p in self.gw.iterdir() if p.name != ".lock"), ["bin"])
        self.assertEqual(self.calls("kill "), [])

    def test_a_marker_that_cannot_be_written_takes_the_new_bind_back(self):
        (self.gw / "enabled.tmp").mkdir()
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("could not be written", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.entry.read_text(), STOCK)
        self.assertEqual(self.calls("kill "), [])

    def test_off_that_cannot_unmount_still_drops_the_boot_marker(self):
        self.run_tool("on")
        code, out = self.run_tool("off", FAIL_UMOUNT="1")
        self.assertEqual(code, 1)
        self.assertIn("could not be unmounted", out["error"])
        self.assertFalse((self.gw / "enabled").exists(), "a reboot must not bind it again")
        self.assertEqual(self.run_tool("status")[1]["entry"], "ours", "still bound and still reported")

    def test_off_that_cannot_get_the_lock_is_not_applied_and_says_so(self):
        self.run_tool("on")
        (self.gw / ".lock").mkdir()
        code, out = self.run_tool("off")
        self.assertEqual(code, 1)
        self.assertIn("off was not applied", out["error"])
        self.assertTrue((self.gw / "enabled").exists())
        self.assertEqual(len(self.mounts()), 1)

    def test_the_boot_step_binds_again_only_while_switched_on_and_never_fails(self):
        self.run_tool("on")
        self.reboot()
        code, out = self.run_tool("early")
        self.assertEqual((code, out), (0, {"ok": True, "applied": True, "ssgw": "bound"}))
        self.assertTrue(os.path.samefile(self.entry, self.gw / "ssg_https"))
        self.assertEqual(self.calls("kill "), ["kill -HUP 4242"] * 2)
        self.reboot()
        self.nsl.write_text("an updated nsl\n")
        code, out = self.run_tool("early")
        self.assertEqual((code, out["applied"]), (0, False))
        self.assertIn("not the program", out["why"])
        self.assertEqual(self.mounts(), [])
        self.run_tool("off")
        code, out = self.run_tool("early")
        self.assertEqual((code, out), (0, {"ok": True, "applied": False, "why": "not switched on"}))

    def test_xinetd_is_not_signalled_when_the_pid_is_not_xinetds(self):
        os.remove(self.tmp / "proc/4242/exe")
        os.symlink("/usr/bin/other", self.tmp / "proc/4242/exe")
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "ssgw": "bound", "xinetd": "not running"}))
        self.assertEqual(self.calls("kill "), [])


if __name__ == "__main__":
    unittest.main()
