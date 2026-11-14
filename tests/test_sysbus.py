"""tools/sysbus.sh against stand-in mount, kill, luna-send and module-builder commands: the launcher is bound only when it is LG's known
script, a mount that is not ours is never touched (equal bytes on another inode included), a failure rolls back, `off` survives a boot,
and a restart ends only the unified server's node and reports honestly whether the new one runs the right module."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "sysbus.sh"

STOCK = """#!/bin/sh
if [ $# -ge 1 ]; then
  echo "load $1"
  exit 0
fi
export NODE_PATH=/usr/lib/nodejs:/usr/lib/node_modules
echo "$NODE_PATH"
"""

FIX = """#!/bin/sh
case "$1" in
  build)
    [ "${FAIL_BUILD:-0}" != 1 ] || { echo "sysbus_fix: no" >&2; exit 1; }
    mkdir -p "$3" && echo node > "$3/webos-sysbus.node" && echo js > "$3/palmbus.js" && echo "{\\"v\\":${MANIFEST:-1}}" > "$3/manifest.json" ;;
  verify)
    [ -f "$2/webos-sysbus.node" ] && [ -f "$2/palmbus.js" ] && [ -f "$2/manifest.json" ] ;;
  *) exit 2 ;;
esac
"""

# mount --bind SRC DST shows SRC itself at DST (same inode); umount DST brings back what was under it. FAIL_MOUNT=copy is somebody
# else's mount that lands while ours fails: DST then holds the same bytes on another inode.
MOUNT = """#!/bin/sh
echo "mount $*" >> "$FAKE/calls"
[ "${FAIL_MOUNT:-0}" != 1 ] || exit 1
n=$(wc -l < "$FAKE/mounts")
mv "$3" "$FAKE/under.$n"
if [ "${FAIL_MOUNT:-0}" = copy ]; then
  cp "$2" "$3"
  echo "/elsewhere $3 none bind 0 0" >> "$FAKE/mounts"
  exit 1
fi
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

# TERM ends the process, unless NOEND (it ignores TERM) or REUSE (the pid is somebody else's process by the time it is looked at).
KILL = """#!/bin/sh
echo "kill $*" >> "$FAKE/calls"
if [ "${REUSE:-0}" = 1 ]; then
  sed -i 's/ 5$/ 777/' "$FAKE/proc/$2/stat"
elif [ "${NOEND:-0}" != 1 ]; then
  rm -rf "$FAKE/proc/$2"
fi
"""

# The question starts the server again: a process of the given pid and start time under the unified server's parent.
LUNA = """#!/bin/sh
echo "luna $*" >> "$FAKE/calls"
[ "${LUNA_FAIL:-0}" != 1 ] || exit 1
if [ -n "${SPAWN_PID:-}" ]; then
  d="$FAKE/proc/$SPAWN_PID"
  mkdir -p "$d" "$FAKE/proc/80"
  ln -s /usr/bin/node "$d/exe"
  printf '%s (ss.apiadapter) S 80 ZEROS %s\\n' "$SPAWN_PID" 900 > "$d/stat"
  printf '%s\\n' "${SPAWN_MAPS:-}" > "$d/maps"
  printf '/bin/sh\\0%s\\0' "$SYSBUS_LAUNCHER" > "$FAKE/proc/80/cmdline"
fi
if [ -n "${LUNA_REPLY:-}" ]; then echo "$LUNA_REPLY"; else echo '{"returnValue":true}'; fi
""".replace("ZEROS", " ".join(["0"] * 17))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def stat_line(pid, parent, start):
    # pid (comm) state ppid, 17 fields, then the start time: field 22 of /proc/PID/stat
    return "%d (ss.apiadapter) S %d %s %d\n" % (pid, parent, " ".join(["0"] * 17), start)


class SysbusTool(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="sysbus-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = self.tmp / "state"
        self.state.mkdir()
        self.sb = self.state / "sysbus"
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        self.src = self.tmp / "nodejs"
        self.src.mkdir()
        self.proc = self.tmp / "proc"
        self.proc.mkdir()
        self.launcher = self.tmp / "run-unified-service-server"
        self.launcher.write_text(STOCK)
        self.launcher.chmod(0o755)
        (self.tmp / "mounts").write_text("")
        (self.tmp / "calls").write_text("")
        for name, text in (("fix.sh", FIX), ("mount", MOUNT), ("umount", UMOUNT), ("kill", KILL), ("luna-send", LUNA)):
            path = self.bin / name
            path.write_text(text)
            path.chmod(0o755)
        self.env = dict(
            os.environ, FAKE=str(self.tmp), STATE=str(self.state), APPDIR=str(ROOT),
            SYSBUS_LAUNCHER=str(self.launcher), SYSBUS_LAUNCHER_SHA=sha(self.launcher), SYSBUS_SRC=str(self.src),
            SYSBUS_NODE="/usr/bin/node", SYSBUS_FIX_TOOL=str(self.bin / "fix.sh"), PYTHON="sh",
            MOUNT=str(self.bin / "mount"), UMOUNT=str(self.bin / "umount"), KILL=str(self.bin / "kill"),
            LUNA_SEND=str(self.bin / "luna-send"), PROC_MOUNTS=str(self.tmp / "mounts"), PROC_ROOT=str(self.proc),
            SYSBUS_LOCK_WAIT="1", SYSBUS_STOP_STEPS="2", SYSBUS_STOP_PAUSE="0", SYSBUS_START_STEPS="1", LOGFILE=str(self.tmp / "log"))

    def run_tool(self, *args, **env):
        result = subprocess.run(["sh", str(TOOL), *args], env=dict(self.env, **env), capture_output=True, text=True, timeout=30)
        try:
            return result.returncode, json.loads(result.stdout)
        except ValueError:
            self.fail("not JSON: %r %r" % (result.stdout, result.stderr))

    def calls(self):
        return (self.tmp / "calls").read_text().splitlines()

    def mounts(self):
        return (self.tmp / "mounts").read_text().splitlines()

    def reboot(self):
        """LG's script is back on a new inode and nothing is mounted; what was staged stays."""
        self.launcher.unlink()
        self.launcher.write_text(STOCK)
        self.launcher.chmod(0o755)
        (self.tmp / "mounts").write_text("")

    def server(self, pid, parent, start=5, maps="", argv=None):
        """A node process with the given parent argv (by default `/bin/sh <launcher>`, the unified server) and the maps it shows."""
        directory = self.proc / str(pid)
        directory.mkdir()
        os.symlink("/usr/bin/node", directory / "exe")
        (directory / "stat").write_text(stat_line(pid, parent, start))
        (directory / "maps").write_text(maps + "\n")
        parent_dir = self.proc / str(parent)
        parent_dir.mkdir(exist_ok=True)
        argv = argv or ["/bin/sh", str(self.launcher)]
        (parent_dir / "cmdline").write_bytes(b"".join(a.encode() + b"\0" for a in argv))

    def patched(self):
        return str(self.sb / "mod/webos-sysbus.node")

    def stock_module(self):
        return str(self.src / "webos-sysbus.node")

    def test_on_binds_the_wrapper_remembers_it_and_a_second_on_changes_nothing(self):
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "sysbus": "bound"}))
        self.assertEqual(len(self.mounts()), 1)
        self.assertTrue((self.sb / "enabled").exists())
        self.assertTrue(os.path.samefile(self.launcher, self.sb / "run-unified-service-server"))
        self.assertEqual((self.sb / "run-unified-service-server.orig").read_text(), STOCK)
        self.assertTrue((self.sb / "mod/webos-sysbus.node").exists())
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "sysbus": "already"}))
        self.assertEqual(sum(c.startswith("mount ") for c in self.calls()), 1)
        status = self.run_tool("status")[1]
        self.assertEqual((status["enabled"], status["launcher"], status["moduleOk"], status["loaded"]), (True, "ours", True, "none"))

    def test_off_unmounts_deletes_everything_staged_and_forgets_it(self):
        self.run_tool("on")
        code, out = self.run_tool("off")
        self.assertEqual((code, out), (0, {"ok": True, "sysbus": "off", "was": "ours"}))
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.launcher.read_text(), STOCK)
        self.assertEqual(sorted(p.name for p in self.sb.iterdir()), [])
        status = self.run_tool("status")[1]
        self.assertEqual((status["enabled"], status["launcher"]), (False, "stock"))

    def test_on_after_off_lays_the_module_again_into_the_empty_state_directory(self):
        self.run_tool("on")
        self.run_tool("off")
        self.assertEqual(list(self.sb.iterdir()), [])
        self.assertEqual(self.run_tool("status")[1]["moduleOk"], False)
        code, out = self.run_tool("on")
        self.assertEqual((code, out), (0, {"ok": True, "sysbus": "bound"}))
        status = self.run_tool("status")[1]
        self.assertEqual((status["enabled"], status["launcher"], status["moduleOk"]), (True, "ours", True))

    def test_off_with_nothing_to_undo_leaves_no_trace(self):
        code, out = self.run_tool("off")
        self.assertEqual((code, out), (0, {"ok": True, "sysbus": "off", "was": "stock"}))
        self.assertFalse(self.sb.exists())
        self.assertFalse((self.tmp / "log").exists())

    def test_a_launcher_that_is_not_the_known_script_is_left_alone(self):
        self.launcher.write_text(STOCK + "# another firmware\n")
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("not the script", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertFalse((self.sb / "enabled").exists())
        self.assertFalse((self.sb / "mod").exists())

    def test_a_mount_that_is_not_ours_is_never_stacked_on_or_removed(self):
        (self.tmp / "mounts").write_text("/elsewhere %s none bind 0 0\n" % self.launcher)
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("another mount", out["error"])
        self.assertFalse((self.sb / "enabled").exists())
        code, out = self.run_tool("off")
        self.assertEqual(code, 1)
        self.assertIn("another mount", out["error"])
        self.assertFalse(any(c.startswith(("mount ", "umount ")) for c in self.calls()))
        self.assertEqual(len(self.mounts()), 1)

    def test_somebody_elses_mount_showing_the_same_bytes_on_another_inode_is_not_ours(self):
        self.run_tool("on")
        wrapper = self.sb / "run-unified-service-server"
        self.reboot()
        self.launcher.write_bytes(wrapper.read_bytes())
        (self.tmp / "mounts").write_text("/elsewhere %s none bind 0 0\n" % self.launcher)
        self.assertEqual(self.run_tool("status")[1]["launcher"], "foreign")
        code, out = self.run_tool("off")
        self.assertEqual(code, 1)
        self.assertIn("left alone", out["error"])
        self.assertFalse((self.sb / "enabled").exists(), "the boot marker goes even so")
        self.assertEqual(sum(c.startswith("umount ") for c in self.calls()), 0)
        self.assertEqual(len(self.mounts()), 1)
        self.assertEqual(self.launcher.read_bytes(), wrapper.read_bytes())

    def test_a_failed_bind_rolls_everything_back(self):
        code, out = self.run_tool("on", FAIL_MOUNT="1")
        self.assertEqual(code, 1)
        self.assertIn("could not be bound", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(sum(c.startswith("umount ") for c in self.calls()), 0, "nothing was mounted, nothing is unmounted")
        self.assertEqual(self.launcher.read_text(), STOCK)
        self.assertEqual(sorted(p.name for p in self.sb.iterdir() if p.name != ".lock"), [])

    def test_a_failed_bind_does_not_take_off_a_mount_that_is_somebody_elses(self):
        # the only mount afterwards is not the wrapper's inode: a count of one is not proof it is ours
        code, out = self.run_tool("on", FAIL_MOUNT="copy")
        self.assertEqual(code, 1)
        self.assertIn("could not be bound", out["error"])
        self.assertIn("could not be taken back", out["error"])
        self.assertEqual(sum(c.startswith("umount ") for c in self.calls()), 0)
        self.assertEqual(len(self.mounts()), 1)
        self.assertFalse((self.sb / "enabled").exists())

    def test_a_module_that_cannot_be_built_changes_nothing(self):
        code, out = self.run_tool("on", FAIL_BUILD="1")
        self.assertEqual(code, 1)
        self.assertIn("not built", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertFalse((self.sb / "enabled").exists())

    def test_a_marker_that_cannot_be_written_takes_the_new_bind_back(self):
        (self.sb / "enabled.tmp").mkdir(parents=True)
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("could not be written", out["error"])
        self.assertEqual(self.mounts(), [])
        self.assertEqual(self.launcher.read_text(), STOCK)
        self.assertFalse((self.sb / "mod").exists())
        self.assertFalse((self.sb / "run-unified-service-server").exists())

    def test_a_marker_that_cannot_be_written_leaves_a_bind_that_was_already_there(self):
        self.run_tool("on")
        (self.sb / "enabled").unlink()
        (self.sb / "enabled.tmp").mkdir()
        code, out = self.run_tool("on")
        self.assertEqual(code, 1)
        self.assertIn("could not be written", out["error"])
        self.assertEqual(sum(c.startswith("umount ") for c in self.calls()), 0)
        self.assertEqual(self.run_tool("status")[1]["launcher"], "ours")

    def test_off_that_cannot_unmount_still_drops_the_boot_marker(self):
        self.run_tool("on")
        code, out = self.run_tool("off", FAIL_UMOUNT="1")
        self.assertEqual(code, 1)
        self.assertIn("could not be unmounted", out["error"])
        self.assertFalse((self.sb / "enabled").exists(), "a reboot must not lay it again")
        self.assertEqual(self.run_tool("early")[1]["applied"], False)
        self.assertEqual(self.run_tool("status")[1]["launcher"], "ours", "still bound and still reported")

    def test_off_that_cannot_get_the_lock_is_not_applied_and_says_so(self):
        self.run_tool("on")
        (self.sb / ".lock").mkdir()
        code, out = self.run_tool("off", SYSBUS_LOCK_WAIT="0")
        self.assertEqual(code, 1)
        self.assertIn("off was not applied", out["error"])
        self.assertTrue((self.sb / "enabled").exists(), "a run that holds the lock may still be writing it")
        self.assertEqual(len(self.mounts()), 1)

    def test_early_lays_it_again_only_while_switched_on_and_never_fails_the_boot(self):
        self.assertEqual(self.run_tool("early")[1], {"ok": True, "applied": False, "why": "not switched on"})
        self.run_tool("on")
        self.reboot()
        code, out = self.run_tool("early")
        self.assertEqual((code, out), (0, {"ok": True, "applied": True, "sysbus": "bound"}))
        self.assertEqual(len(self.mounts()), 1)
        self.assertEqual(self.run_tool("early")[1]["sysbus"], "already")
        self.reboot()
        code, out = self.run_tool("early", FAIL_MOUNT="1")
        self.assertEqual((code, out["ok"], out["applied"]), (0, True, False))
        self.assertEqual(self.launcher.read_text(), STOCK)
        self.assertIn("early: not applied", (self.tmp / "log").read_text())

    def test_the_rebuilt_module_replaces_the_old_only_when_it_differs(self):
        self.run_tool("on")
        marker = self.sb / "mod/keep"
        marker.write_text("x")
        self.run_tool("on")
        self.assertTrue(marker.exists(), "same manifest: the module in use stays")
        self.run_tool("on", MANIFEST="2")
        self.assertFalse(marker.exists())
        self.assertIn('"v":2', (self.sb / "mod/manifest.json").read_text())

    def test_the_wrapper_puts_the_module_first_and_falls_back_when_it_is_gone(self):
        self.run_tool("on")
        wrapper = self.sb / "run-unified-service-server"
        out = subprocess.run(["sh", str(wrapper)], capture_output=True, text=True, timeout=10).stdout.strip()
        self.assertEqual(out, "%s:/usr/lib/nodejs:/usr/lib/node_modules" % (self.sb / "mod"))
        self.assertEqual(subprocess.run(["sh", str(wrapper), "svc"], capture_output=True, text=True, timeout=10).stdout.strip(), "load svc")
        (self.sb / "mod/webos-sysbus.node").unlink()
        out = subprocess.run(["sh", str(wrapper)], capture_output=True, text=True, timeout=10).stdout.strip()
        self.assertEqual(out, "/usr/lib/nodejs:/usr/lib/node_modules", "a lost module leaves LG's search path")

    def test_only_a_node_whose_parent_is_exactly_sh_and_the_launcher_is_the_server(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.stock_module())
        self.server(101, 91, argv=["/bin/sh", str(self.launcher), "extra"])
        self.server(102, 92, argv=["/usr/bin/vim", str(self.launcher)])
        self.server(103, 93, argv=["/bin/sh", "-x", str(self.launcher)])
        self.server(104, 94, argv=["/bin/sh", "/usr/bin/other", str(self.launcher)])
        status = self.run_tool("status")[1]
        self.assertEqual([p["pid"] for p in status["processes"]], [100])

    def test_status_says_loaded_only_for_what_the_running_server_mapped(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.patched())
        status = self.run_tool("status")[1]
        self.assertEqual((status["launcher"], status["loaded"], status["restartNeeded"]), ("ours", "patched", False))
        self.run_tool("off")
        status = self.run_tool("status")[1]
        self.assertEqual((status["launcher"], status["loaded"], status["restartNeeded"]), ("stock", "patched", True))

    def test_two_copies_in_one_process_or_none_yet_need_a_restart(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.patched() + "\n" + self.stock_module())
        status = self.run_tool("status")[1]
        self.assertEqual((status["loaded"], status["restartNeeded"], status["processes"][0]["module"]), ("mixed", True, "mixed"))
        shutil.rmtree(self.proc / "100")
        self.server(100, 90, maps="")
        status = self.run_tool("status")[1]
        self.assertEqual((status["loaded"], status["restartNeeded"]), ("unknown", True))

    def test_restart_ends_exactly_the_unified_server_and_checks_the_new_process(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.stock_module())
        self.server(200, 91, argv=["/bin/sh", "/usr/bin/other-launcher"], maps=self.stock_module())
        code, out = self.run_tool("restart", SPAWN_PID="101", SPAWN_MAPS=self.patched())
        self.assertEqual(code, 0, out)
        self.assertEqual(out["restart"], {"ended": [100], "running": [101], "module": "patched"})
        self.assertEqual([c for c in self.calls() if c.startswith("kill ")], ["kill -TERM 100"])
        self.assertTrue((self.proc / "200").exists(), "another node host is not touched")
        self.assertTrue(any("luna://com.webos.service.apiadapter/system/getSystemInfo" in c for c in self.calls()))

    def test_restart_that_gets_no_answer_or_a_refusal_is_a_failure(self):
        self.run_tool("on")
        for env in ({"LUNA_FAIL": "1"}, {"LUNA_REPLY": '{"returnValue":false,"errorText":"no"}'}):
            with self.subTest(env=env):
                self.server(100, 90, maps=self.stock_module())
                code, out = self.run_tool("restart", SPAWN_PID="101", SPAWN_MAPS=self.patched(), **env)
                self.assertEqual(code, 1)
                self.assertFalse(out["ok"])
                self.assertIn("did not answer", out["error"])
                shutil.rmtree(self.proc / "101", True)

    def test_restart_with_no_new_process_is_a_failure(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.stock_module())
        code, out = self.run_tool("restart")
        self.assertEqual(code, 1)
        self.assertIn("no new unified server process", out["error"])

    def test_restart_whose_new_process_runs_the_wrong_module_is_a_failure(self):
        self.run_tool("on")
        cases = (("patched expected, LG's mapped", self.stock_module(), "expected patched"),
                 ("both mapped", self.patched() + "\n" + self.stock_module(), "module state mixed"),
                 ("neither mapped", "", "module state unknown"))
        for label, maps, why in cases:
            with self.subTest(label):
                self.server(100, 90, maps=self.stock_module())
                code, out = self.run_tool("restart", SPAWN_PID="101", SPAWN_MAPS=maps)
                self.assertEqual(code, 1)
                self.assertIn(why, out["error"])
                shutil.rmtree(self.proc / "101", True)
                shutil.rmtree(self.proc / "100", True)

    def test_off_with_restart_wants_LGs_module_in_the_new_process(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.patched())
        code, out = self.run_tool("off", "--restart", SPAWN_PID="101", SPAWN_MAPS=self.patched())
        self.assertEqual(code, 1)
        self.assertIn("expected stock", out["error"])
        self.assertEqual(self.launcher.read_text(), STOCK, "the launcher was taken off all the same")
        shutil.rmtree(self.proc / "101")
        self.run_tool("on")
        self.server(102, 90, start=6, maps=self.patched())
        code, out = self.run_tool("off", "--restart", SPAWN_PID="103", SPAWN_MAPS=self.stock_module())
        self.assertEqual(code, 0, out)
        self.assertEqual(out["restart"]["module"], "stock")
        calls = self.calls()
        self.assertLess(calls.index("umount " + str(self.launcher)), calls.index("kill -TERM 102"))

    def test_a_pid_that_became_another_process_is_not_signalled(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.patched())
        self.run_tool("restart", REUSE="1", SPAWN_PID="101", SPAWN_MAPS=self.patched())
        self.assertEqual([c for c in self.calls() if c.startswith("kill ")], ["kill -TERM 100"], "no KILL for a process that is not the pinned one")

    def test_a_server_that_ignores_term_is_killed_and_one_that_still_stands_is_a_failure(self):
        self.run_tool("on")
        self.server(100, 90, maps=self.stock_module())
        code, out = self.run_tool("restart", NOEND="1")
        self.assertEqual(code, 1)
        self.assertIn("pid 100 did not end", out["error"])
        self.assertEqual([c for c in self.calls() if c.startswith("kill ")], ["kill -TERM 100", "kill -KILL 100"])


if __name__ == "__main__":
    unittest.main()
